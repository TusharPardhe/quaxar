//! Immutable `ReadView` preclaim helpers for TrustSet, Clawback, and MPToken.
//!
//! This module owns only these token transaction types. It derives typed facts
//! from immutable ledger reads, never creates a sandbox or invokes apply code,
//! and returns `None` for every unowned transaction type.

use std::sync::Arc;

use basics::base_uint::{Uint160, Uint192};
use ledger::ReadView;
use protocol::{
    AccountID, Asset, STLedgerEntry, STTx, Ter, TxType, feature_amm, feature_deep_freeze,
    feature_single_asset_vault, get_field_by_symbol, lsfAllowTrustLineClawback, lsfMPTCanClawback,
    lsfMPTCanLock, lsfMPTCanTransfer, lsfMPTLocked, lsfMPTRequireAuth, lsfNoFreeze, lsfRequireAuth,
    tfMPTUnauthorize,
};

use crate::{
    ClawbackIssuePreclaimFacts, ClawbackMptPreclaimFacts, ClawbackPreclaimAssetFacts,
    ClawbackPreclaimFacts, ClawbackTrustlineBalanceSign, MPTokenAuthorizePreclaimFacts,
    MPTokenIssuanceDestroyPreclaimFacts, MPTokenIssuanceSetPreclaimFacts, TrustSetPreclaimFacts,
    run_clawback_preclaim, run_mp_token_authorize_preclaim, run_mp_token_issuance_destroy_preclaim,
    run_mp_token_issuance_set_preclaim, run_trust_set_preclaim_with_facts,
};

fn sf(name: &str) -> &'static protocol::SField {
    get_field_by_symbol(name)
}

fn read_error() -> Ter {
    Ter::TEF_BAD_LEDGER
}

fn account_keylet(account: AccountID) -> protocol::Keylet {
    protocol::account_keylet(Uint160::from_void(account.data()))
}

fn read<V: ReadView>(
    view: &V,
    keylet: protocol::Keylet,
) -> Result<Option<Arc<STLedgerEntry>>, Ter> {
    view.read(keylet).map_err(|_| read_error())
}

fn account<V: ReadView>(view: &V, id: AccountID) -> Result<Option<Arc<STLedgerEntry>>, Ter> {
    read(view, account_keylet(id))
}

fn mptoken<V: ReadView>(
    view: &V,
    id: Uint192,
    holder: AccountID,
) -> Result<Option<Arc<STLedgerEntry>>, Ter> {
    read(
        view,
        protocol::mptoken_keylet_from_mptid(id, Uint160::from_void(holder.data())),
    )
}

fn mpt_issuance<V: ReadView>(view: &V, id: Uint192) -> Result<Option<Arc<STLedgerEntry>>, Ter> {
    read(view, protocol::mpt_issuance_keylet_from_mptid(id))
}

fn pseudo_account(sle: &STLedgerEntry) -> bool {
    ["sfAMMID", "sfVaultID", "sfLoanBrokerID"]
        .iter()
        .any(|field| sle.is_field_present(sf(field)))
}

fn account_holds_positive<V: ReadView>(
    view: &V,
    account_id: AccountID,
    asset: Asset,
) -> Result<bool, Ter> {
    match asset {
        Asset::Issue(issue) if issue.native() || issue.account == account_id => Ok(false),
        Asset::Issue(issue) => {
            let Some(line) = read(
                view,
                protocol::line(account_id, issue.account, issue.currency),
            )?
            else {
                return Ok(false);
            };
            let mut balance = line.get_field_amount(sf("sfBalance"));
            if account_id > issue.account {
                balance.negate();
            }
            Ok(balance.signum() > 0)
        }
        Asset::MPTIssue(issue) if issue.issuer() == account_id => Ok(false),
        Asset::MPTIssue(issue) => Ok(mptoken(view, issue.mpt_id(), account_id)?
            .is_some_and(|token| token.get_field_u64(sf("sfMPTAmount")) > 0)),
    }
}

fn preclaim_trust_set<V: ReadView>(view: &V, tx: &STTx) -> Result<Ter, Ter> {
    let account_id = tx.get_account_id(sf("sfAccount"));
    let source = account(view, account_id)?;
    if source.is_none() {
        return Ok(run_trust_set_preclaim_with_facts(TrustSetPreclaimFacts {
            account_exists: false,
            tx_flags: tx.get_flags(),
            account_requires_auth: false,
            destination_is_source: false,
            destination_exists: false,
            amm_or_single_asset_vault_enabled: false,
            destination_account_flags: 0,
            fix_disallow_incoming_v1_enabled: false,
            trustline_exists: false,
            destination_is_pseudo_account: false,
            pseudo_destination_is_amm: false,
            pseudo_destination_is_vault_or_loan_broker: false,
            amm_ledger_entry_exists: false,
            amm_lp_token_balance_non_zero: false,
            amm_lp_token_currency_matches_limit: false,
            deep_freeze_enabled: false,
            account_no_freeze: false,
            high_account_side: false,
            current_trustline_flags: 0,
        }));
    }

    let source = source.expect("checked above");
    let limit = tx.get_field_amount(sf("sfLimitAmount"));
    let issue = limit.issue();
    let destination = issue.account;
    let destination_sle = account(view, destination)?;
    let line = read(
        view,
        protocol::line(account_id, destination, issue.currency),
    )?;
    let destination_is_amm = destination_sle
        .as_ref()
        .is_some_and(|sle| sle.is_field_present(sf("sfAMMID")));
    let amm = if destination_is_amm && line.is_none() {
        let id = destination_sle
            .as_ref()
            .expect("AMM designator requires destination")
            .get_field_h256(sf("sfAMMID"));
        read(view, protocol::amm_keylet(id))?
    } else {
        None
    };

    Ok(run_trust_set_preclaim_with_facts(TrustSetPreclaimFacts {
        account_exists: true,
        tx_flags: tx.get_flags(),
        account_requires_auth: source.is_flag(lsfRequireAuth),
        destination_is_source: account_id == destination,
        destination_exists: destination_sle.is_some(),
        amm_or_single_asset_vault_enabled: view.rules().enabled(&feature_amm())
            || view.rules().enabled(&feature_single_asset_vault()),
        destination_account_flags: destination_sle.as_ref().map_or(0, |sle| sle.get_flags()),
        fix_disallow_incoming_v1_enabled: view
            .rules()
            .enabled(&protocol::feature_id("fixDisallowIncomingV1")),
        trustline_exists: line.is_some(),
        destination_is_pseudo_account: destination_sle
            .as_ref()
            .is_some_and(|sle| pseudo_account(sle)),
        pseudo_destination_is_amm: destination_is_amm,
        pseudo_destination_is_vault_or_loan_broker: destination_sle.as_ref().is_some_and(|sle| {
            sle.is_field_present(sf("sfVaultID")) || sle.is_field_present(sf("sfLoanBrokerID"))
        }),
        amm_ledger_entry_exists: amm.is_some(),
        amm_lp_token_balance_non_zero: amm
            .as_ref()
            .is_some_and(|sle| sle.get_field_amount(sf("sfLPTokenBalance")).signum() != 0),
        amm_lp_token_currency_matches_limit: amm.as_ref().is_some_and(|sle| {
            sle.get_field_amount(sf("sfLPTokenBalance"))
                .issue()
                .currency
                == issue.currency
        }),
        deep_freeze_enabled: view.rules().enabled(&feature_deep_freeze()),
        account_no_freeze: source.is_flag(lsfNoFreeze),
        high_account_side: account_id > destination,
        current_trustline_flags: line.as_ref().map_or(0, |sle| sle.get_flags()),
    }))
}

fn preclaim_clawback<V: ReadView>(view: &V, tx: &STTx) -> Result<Ter, Ter> {
    let issuer = tx.get_account_id(sf("sfAccount"));
    let amount = tx.get_field_amount(sf("sfAmount"));
    let holder = match amount.asset() {
        Asset::Issue(issue) => issue.account,
        Asset::MPTIssue(_) => tx.get_account_id(sf("sfHolder")),
    };
    let issuer_sle = account(view, issuer)?;
    let holder_sle = account(view, holder)?;
    if issuer_sle.is_none() || holder_sle.is_none() {
        return Ok(Ter::TER_NO_ACCOUNT);
    }
    let holder_sle = holder_sle.expect("checked above");
    let asset = match amount.asset() {
        Asset::Issue(issue) => {
            let line = read(view, protocol::line(holder, issuer, issue.currency))?;
            let balance_sign = line
                .as_ref()
                .map_or(ClawbackTrustlineBalanceSign::Zero, |sle| {
                    match sle.get_field_amount(sf("sfBalance")).signum() {
                        positive if positive > 0 => ClawbackTrustlineBalanceSign::Positive,
                        negative if negative < 0 => ClawbackTrustlineBalanceSign::Negative,
                        _ => ClawbackTrustlineBalanceSign::Zero,
                    }
                });
            ClawbackPreclaimAssetFacts::Issue(ClawbackIssuePreclaimFacts {
                allow_trustline_clawback: issuer_sle
                    .as_ref()
                    .is_some_and(|sle| sle.is_flag(lsfAllowTrustLineClawback)),
                issuer_no_freeze: issuer_sle
                    .as_ref()
                    .is_some_and(|sle| sle.is_flag(lsfNoFreeze)),
                ripple_state_exists: line.is_some(),
                trustline_balance_sign: balance_sign,
                issuer_holder_ordering: issuer.cmp(&holder),
                account_holds_positive: account_holds_positive(view, holder, Asset::Issue(issue))?,
            })
        }
        Asset::MPTIssue(issue) => {
            let issuance = mpt_issuance(view, issue.mpt_id())?;
            let token = mptoken(view, issue.mpt_id(), holder)?;
            ClawbackPreclaimAssetFacts::Mpt(ClawbackMptPreclaimFacts {
                issuance_exists: issuance.is_some(),
                issuance_can_clawback: issuance
                    .as_ref()
                    .is_some_and(|sle| sle.is_flag(lsfMPTCanClawback)),
                issuance_issuer_matches: issuance
                    .as_ref()
                    .is_some_and(|sle| sle.get_account_id(sf("sfIssuer")) == issuer),
                holder_token_exists: token.is_some(),
                account_holds_positive: token
                    .as_ref()
                    .is_some_and(|sle| sle.get_field_u64(sf("sfMPTAmount")) > 0),
            })
        }
    };
    Ok(run_clawback_preclaim(ClawbackPreclaimFacts {
        issuer_exists: true,
        holder_exists: true,
        single_asset_vault_enabled: view.rules().enabled(&feature_single_asset_vault()),
        holder_is_pseudo_account: pseudo_account(&holder_sle),
        holder_is_amm_account: holder_sle.is_field_present(sf("sfAMMID")),
        asset,
    }))
}

fn preclaim_mpt_authorize<V: ReadView>(view: &V, tx: &STTx) -> Result<Ter, Ter> {
    let account_id = tx.get_account_id(sf("sfAccount"));
    let issuance_id = tx.get_field_h192(sf("sfMPTokenIssuanceID"));
    let holder = tx
        .is_field_present(sf("sfHolder"))
        .then(|| tx.get_account_id(sf("sfHolder")));

    if let Some(holder) = holder {
        let holder_sle = account(view, holder)?;
        if holder_sle.is_none() {
            return Ok(Ter::TEC_NO_DST);
        }
        let issuance = mpt_issuance(view, issuance_id)?;
        let holder_token = mptoken(view, issuance_id, holder)?;
        return Ok(run_mp_token_authorize_preclaim(
            MPTokenAuthorizePreclaimFacts {
                holder_present: true,
                account_token_exists: false,
                tx_flags: tx.get_flags(),
                token_balance_is_zero: true,
                token_locked_amount_is_zero: true,
                issuance_exists: issuance.is_some(),
                single_asset_vault_enabled: view.rules().enabled(&feature_single_asset_vault()),
                token_locked: false,
                account_is_issuer: issuance
                    .as_ref()
                    .is_some_and(|sle| sle.get_account_id(sf("sfIssuer")) == account_id),
                holder_account_exists: true,
                issuance_requires_auth: issuance
                    .as_ref()
                    .is_some_and(|sle| sle.is_flag(lsfMPTRequireAuth)),
                holder_token_exists: holder_token.is_some(),
                holder_is_pseudo_account: holder_sle.is_some_and(|sle| pseudo_account(&sle)),
            },
        ));
    }

    // rippled reads the holding first. In the unauthorize success case it
    // deliberately does not read the issuance, including when it was deleted.
    let token = mptoken(view, issuance_id, account_id)?;
    let unauthorize = (tx.get_flags() & tfMPTUnauthorize) != 0;
    if unauthorize {
        let Some(token) = token else {
            return Ok(Ter::TEC_OBJECT_NOT_FOUND);
        };
        let balance = token.get_field_u64(sf("sfMPTAmount"));
        let locked = if token.is_field_present(sf("sfLockedAmount")) {
            token.get_field_u64(sf("sfLockedAmount"))
        } else {
            0
        };
        if balance != 0 || locked != 0 {
            let issuance = mpt_issuance(view, issuance_id)?;
            return Ok(run_mp_token_authorize_preclaim(
                MPTokenAuthorizePreclaimFacts {
                    holder_present: false,
                    account_token_exists: true,
                    tx_flags: tx.get_flags(),
                    token_balance_is_zero: balance == 0,
                    token_locked_amount_is_zero: locked == 0,
                    issuance_exists: issuance.is_some(),
                    single_asset_vault_enabled: view.rules().enabled(&feature_single_asset_vault()),
                    token_locked: token.is_flag(lsfMPTLocked),
                    account_is_issuer: false,
                    holder_account_exists: false,
                    issuance_requires_auth: false,
                    holder_token_exists: false,
                    holder_is_pseudo_account: false,
                },
            ));
        }
        if view.rules().enabled(&feature_single_asset_vault()) && token.is_flag(lsfMPTLocked) {
            return Ok(Ter::TEC_NO_PERMISSION);
        }
        return Ok(Ter::TES_SUCCESS);
    }

    let issuance = mpt_issuance(view, issuance_id)?;
    Ok(run_mp_token_authorize_preclaim(
        MPTokenAuthorizePreclaimFacts {
            holder_present: false,
            account_token_exists: token.is_some(),
            tx_flags: tx.get_flags(),
            token_balance_is_zero: true,
            token_locked_amount_is_zero: true,
            issuance_exists: issuance.is_some(),
            single_asset_vault_enabled: view.rules().enabled(&feature_single_asset_vault()),
            token_locked: false,
            account_is_issuer: issuance
                .as_ref()
                .is_some_and(|sle| sle.get_account_id(sf("sfIssuer")) == account_id),
            holder_account_exists: false,
            issuance_requires_auth: false,
            holder_token_exists: false,
            holder_is_pseudo_account: false,
        },
    ))
}

fn preclaim_mpt_destroy<V: ReadView>(view: &V, tx: &STTx) -> Result<Ter, Ter> {
    let issuance = mpt_issuance(view, tx.get_field_h192(sf("sfMPTokenIssuanceID")))?;
    let account_id = tx.get_account_id(sf("sfAccount"));
    Ok(run_mp_token_issuance_destroy_preclaim(
        MPTokenIssuanceDestroyPreclaimFacts {
            issuance_exists: issuance.is_some(),
            issuer_matches: issuance
                .as_ref()
                .is_some_and(|sle| sle.get_account_id(sf("sfIssuer")) == account_id),
            outstanding_amount_is_zero: issuance
                .as_ref()
                .is_some_and(|sle| sle.get_field_u64(sf("sfOutstandingAmount")) == 0),
            locked_amount_is_zero: issuance.as_ref().is_some_and(|sle| {
                !sle.is_field_present(sf("sfLockedAmount"))
                    || sle.get_field_u64(sf("sfLockedAmount")) == 0
            }),
        },
    ))
}

fn preclaim_mpt_set<V: ReadView>(view: &V, tx: &STTx) -> Result<Ter, Ter> {
    let issuance_id = tx.get_field_h192(sf("sfMPTokenIssuanceID"));
    let issuance = mpt_issuance(view, issuance_id)?;
    if issuance.is_none() {
        return Ok(Ter::TEC_OBJECT_NOT_FOUND);
    }
    let issuance = issuance.expect("checked above");
    let holder = tx
        .is_field_present(sf("sfHolder"))
        .then(|| tx.get_account_id(sf("sfHolder")));
    let holder_sle = holder.map(|id| account(view, id)).transpose()?;
    let holder_token = holder
        .map(|id| mptoken(view, issuance_id, id))
        .transpose()?;
    let domain = tx
        .is_field_present(sf("sfDomainID"))
        .then(|| tx.get_field_h256(sf("sfDomainID")));
    let domain_sle = domain
        .filter(|id| !id.is_zero())
        .map(|id| read(view, protocol::permissioned_domain_keylet_from_id(id)))
        .transpose()?;
    Ok(run_mp_token_issuance_set_preclaim(
        MPTokenIssuanceSetPreclaimFacts {
            issuance_exists: true,
            issuance_can_lock: issuance.is_flag(lsfMPTCanLock),
            single_asset_vault_enabled: view.rules().enabled(&feature_single_asset_vault()),
            dynamic_mpt_enabled: view.rules().enabled(&protocol::feature_id("DynamicMPT")),
            tx_flags: tx.get_flags(),
            issuer_matches: issuance.get_account_id(sf("sfIssuer"))
                == tx.get_account_id(sf("sfAccount")),
            holder_present: holder.is_some(),
            holder_account_exists: holder_sle.is_some_and(|entry| entry.is_some()),
            holder_token_exists: holder_token.is_some_and(|entry| entry.is_some()),
            domain_id_present: domain.is_some(),
            domain_id_is_zero: domain.is_some_and(|id| id.is_zero()),
            issuance_requires_auth: issuance.is_flag(lsfMPTRequireAuth),
            domain_exists: domain_sle.is_some_and(|entry| entry.is_some()),
            issuance_domain_present: issuance.is_field_present(sf("sfDomainID")),
            current_mutable_flags: issuance.get_field_u32(sf("sfMutableFlags")),
            mutable_flags: tx
                .is_field_present(sf("sfMutableFlags"))
                .then(|| tx.get_field_u32(sf("sfMutableFlags"))),
            metadata_present: tx.is_field_present(sf("sfMPTokenMetadata")),
            transfer_fee: tx
                .is_field_present(sf("sfTransferFee"))
                .then(|| tx.get_field_u16(sf("sfTransferFee"))),
            issuance_can_transfer: issuance.is_flag(lsfMPTCanTransfer),
        },
    ))
}

/// Runs the complete immutable preclaim for the owned token families.
///
/// `None` identifies an unowned transaction type. It is intentionally not a
/// success fallback; an explicit match arm is required for every owned type.
pub fn run_token_read_view_preclaim<V: ReadView>(
    view: &V,
    tx: &STTx,
    txn_type: TxType,
) -> Option<Ter> {
    let result = match txn_type {
        TxType::TRUST_SET => preclaim_trust_set(view, tx),
        TxType::CLAWBACK => preclaim_clawback(view, tx),
        TxType::MPTOKEN_AUTHORIZE => preclaim_mpt_authorize(view, tx),
        // MPTokenIssuanceCreate inherits Transactor::preclaim in rippled.
        // Keeping that audited no-op explicit avoids a permissive default.
        TxType::MPTOKEN_ISSUANCE_CREATE => return Some(Ter::TES_SUCCESS),
        TxType::MPTOKEN_ISSUANCE_DESTROY => preclaim_mpt_destroy(view, tx),
        TxType::MPTOKEN_ISSUANCE_SET => preclaim_mpt_set(view, tx),
        _ => return None,
    };
    Some(result.unwrap_or_else(|ter| ter))
}

#[cfg(test)]
mod tests {
    use std::{collections::BTreeMap, sync::Arc};

    use basics::base_uint::Uint256;
    use ledger::{Fees, LedgerHeader, ReadView, ReadViewTx, Rules, ViewError};
    use protocol::{AccountID, Keylet, STAmount, STLedgerEntry, STTx, Ter, TxType, XRPAmount};

    use super::{run_token_read_view_preclaim, sf};

    #[derive(Debug, Default)]
    struct View {
        entries: BTreeMap<Uint256, Arc<STLedgerEntry>>,
    }

    impl ReadView for View {
        fn open(&self) -> bool {
            false
        }
        fn header(&self) -> LedgerHeader {
            LedgerHeader::default()
        }
        fn fees(&self) -> Fees {
            Fees::default()
        }
        fn rules(&self) -> Rules {
            Rules::default()
        }
        fn exists(&self, keylet: Keylet) -> Result<bool, ViewError> {
            Ok(self.entries.contains_key(&keylet.key))
        }
        fn succ(&self, _: Uint256, _: Option<Uint256>) -> Result<Option<Uint256>, ViewError> {
            Ok(None)
        }
        fn read(&self, keylet: Keylet) -> Result<Option<Arc<STLedgerEntry>>, ViewError> {
            Ok(self.entries.get(&keylet.key).cloned())
        }
        fn sles(&self) -> Result<Vec<Arc<STLedgerEntry>>, ViewError> {
            Ok(self.entries.values().cloned().collect())
        }
        fn tx_exists(&self, _: Uint256) -> Result<bool, ViewError> {
            Ok(false)
        }
        fn tx_read(&self, _: Uint256) -> Result<Option<ReadViewTx>, ViewError> {
            Ok(None)
        }
        fn txs(&self) -> Result<Vec<ReadViewTx>, ViewError> {
            Ok(Vec::new())
        }
    }

    fn account(fill: u8) -> AccountID {
        AccountID::from_array([fill; 20])
    }

    #[test]
    fn token_helper_has_no_unowned_success_default() {
        let view = View::default();
        let tx = STTx::new(TxType::PAYMENT, |_| {});
        assert_eq!(
            run_token_read_view_preclaim(&view, &tx, TxType::PAYMENT),
            None
        );
    }

    #[test]
    fn trust_set_and_clawback_preserve_their_first_account_failures() {
        let view = View::default();
        let trust = STTx::new(TxType::TRUST_SET, |tx| {
            tx.set_account_id(sf("sfAccount"), account(1));
            tx.set_field_amount(
                sf("sfLimitAmount"),
                STAmount::from_xrp_amount(XRPAmount::from_drops(1)),
            );
        });
        let clawback = STTx::new(TxType::CLAWBACK, |tx| {
            tx.set_account_id(sf("sfAccount"), account(1));
            tx.set_field_amount(
                sf("sfAmount"),
                STAmount::from_xrp_amount(XRPAmount::from_drops(1)),
            );
        });
        assert_eq!(
            run_token_read_view_preclaim(&view, &trust, TxType::TRUST_SET),
            Some(Ter::TER_NO_ACCOUNT)
        );
        assert_eq!(
            run_token_read_view_preclaim(&view, &clawback, TxType::CLAWBACK),
            Some(Ter::TER_NO_ACCOUNT)
        );
        assert!(view.entries.is_empty(), "ReadView preclaim must not mutate");
    }

    #[test]
    fn mpt_owned_types_route_without_an_apply_or_default_path() {
        let view = View::default();
        let create = STTx::new(TxType::MPTOKEN_ISSUANCE_CREATE, |_| {});
        let destroy = STTx::new(TxType::MPTOKEN_ISSUANCE_DESTROY, |tx| {
            tx.set_field_h192(
                sf("sfMPTokenIssuanceID"),
                basics::base_uint::Uint192::zero(),
            );
            tx.set_account_id(sf("sfAccount"), account(1));
        });
        assert_eq!(
            run_token_read_view_preclaim(&view, &create, TxType::MPTOKEN_ISSUANCE_CREATE),
            Some(Ter::TES_SUCCESS)
        );
        assert_eq!(
            run_token_read_view_preclaim(&view, &destroy, TxType::MPTOKEN_ISSUANCE_DESTROY),
            Some(Ter::TEC_OBJECT_NOT_FOUND)
        );
        assert!(view.entries.is_empty(), "all MPT paths are immutable");
    }
}
