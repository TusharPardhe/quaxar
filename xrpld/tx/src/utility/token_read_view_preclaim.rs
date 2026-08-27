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
    MPTokenAuthorizePreclaimFacts, MPTokenIssuanceDestroyPreclaimFacts,
    MPTokenIssuanceSetPreclaimFacts, TrustSetPreclaimFacts, run_mp_token_authorize_preclaim,
    run_mp_token_issuance_destroy_preclaim, run_mp_token_issuance_set_preclaim,
    run_trust_set_preclaim_with_facts,
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
    ledger::is_pseudo_account(sle)
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
            balance.set_issuer(issue.account);
            Ok(view
                .balance_hook_iou(account_id, issue.account, balance)
                .signum()
                > 0)
        }
        Asset::MPTIssue(issue) if issue.issuer() == account_id => Ok(false),
        Asset::MPTIssue(issue) => {
            let Some(token) = mptoken(view, issue.mpt_id(), account_id)? else {
                return Ok(false);
            };
            let amount = i64::try_from(token.get_field_u64(sf("sfMPTAmount")))
                .map_err(|_| Ter::TEF_BAD_LEDGER)?;
            if view.rules().enabled(&protocol::feature_id("MPTokensV2")) {
                Ok(view.balance_hook_mpt(account_id, issue, amount).signum() > 0)
            } else {
                Ok(amount > 0)
            }
        }
    }
}

fn preclaim_trust_set<V: ReadView>(view: &V, tx: &STTx) -> Result<Ter, Ter> {
    let account_id = tx.get_account_id(sf("sfAccount"));
    let source = account(view, account_id)?;
    let Some(source) = source else {
        return Ok(run_trust_set_preclaim_with_facts(TrustSetPreclaimFacts {
            account_exists: false,
            tx_flags: tx.get_flags(),
            account_requires_auth: false,
            destination_is_source: false,
            destination_exists: false,
            amm_or_single_asset_vault_enabled: false,
            destination_account_flags: 0,
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
    };
    // TrustSet::preclaim checks the source's RequireAuth contract before it
    // derives or reads anything for the destination.
    if (tx.get_flags() & protocol::tfSetfAuth) != 0 && !source.is_flag(lsfRequireAuth) {
        return Ok(Ter::TEF_NO_AUTH_REQUIRED);
    }
    let limit = tx.get_field_amount(sf("sfLimitAmount"));
    let issue = limit.issue();
    let destination = issue.account;
    if account_id == destination {
        return Ok(Ter::TEM_DST_IS_SRC);
    }
    let destination_sle = account(view, destination)?;
    let amm_or_single_asset_vault_enabled =
        view.rules().enabled(&feature_amm()) || view.rules().enabled(&feature_single_asset_vault());
    if amm_or_single_asset_vault_enabled && destination_sle.is_none() {
        return Ok(Ter::TEC_NO_DST);
    }
    let pseudo = destination_sle
        .as_ref()
        .is_some_and(|sle| pseudo_account(sle));
    let deep_freeze_enabled = view.rules().enabled(&feature_deep_freeze());
    let destination_disallows = destination_sle
        .as_ref()
        .is_some_and(|sle| (sle.get_flags() & protocol::lsfDisallowIncomingTrustline) != 0);
    let mut line = None;
    // The disallow-incoming check is the first branch that can touch the
    // trust line after reading the destination.
    if destination_disallows {
        line = read(
            view,
            protocol::line(account_id, destination, issue.currency),
        )?;
        if line.is_none() {
            return Ok(Ter::TEC_NO_PERMISSION);
        }
    }
    let destination_is_amm = destination_sle
        .as_ref()
        .is_some_and(|sle| sle.is_field_present(sf("sfAMMID")));
    let destination_is_vault_or_loan = destination_sle.as_ref().is_some_and(|sle| {
        sle.is_field_present(sf("sfVaultID")) || sle.is_field_present(sf("sfLoanBrokerID"))
    });
    if pseudo && !destination_is_amm && !destination_is_vault_or_loan {
        return Ok(Ter::TEC_PSEUDO_ACCOUNT);
    }

    if deep_freeze_enabled {
        let flags = tx.get_flags();
        let set_freeze = (flags & protocol::tfSetFreeze) != 0;
        let set_deep_freeze = (flags & protocol::tfSetDeepFreeze) != 0;
        let clear_freeze = (flags & protocol::tfClearFreeze) != 0;
        let clear_deep_freeze = (flags & protocol::tfClearDeepFreeze) != 0;
        if (source.is_flag(lsfNoFreeze) && (set_freeze || set_deep_freeze))
            || ((set_freeze || set_deep_freeze) && (clear_freeze || clear_deep_freeze))
        {
            return Ok(Ter::TEC_NO_PERMISSION);
        }
    }

    // Only a recognized pseudo-account branch or the final DeepFreeze state
    // computation needs the line after the earlier canonical exits.
    if line.is_none() && (pseudo || deep_freeze_enabled) {
        line = read(
            view,
            protocol::line(account_id, destination, issue.currency),
        )?;
    }
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
        amm_or_single_asset_vault_enabled,
        destination_account_flags: destination_sle.as_ref().map_or(0, |sle| sle.get_flags()),
        trustline_exists: line.is_some(),
        destination_is_pseudo_account: pseudo,
        pseudo_destination_is_amm: destination_is_amm,
        pseudo_destination_is_vault_or_loan_broker: destination_is_vault_or_loan,
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
        deep_freeze_enabled,
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
    if issuer_sle.is_none() {
        return Ok(Ter::TER_NO_ACCOUNT);
    }
    let Some(holder_sle) = holder_sle else {
        return Ok(Ter::TER_NO_ACCOUNT);
    };
    // Pinned Clawback::preclaim rejects pseudo/AMM holders before it reads the
    // trust line or MPT issuance. Preserve that ordering so an unrelated
    // storage fault cannot replace the canonical holder TER.
    if view.rules().enabled(&feature_single_asset_vault()) && pseudo_account(&holder_sle) {
        return Ok(Ter::TEC_PSEUDO_ACCOUNT);
    }
    if holder_sle.is_field_present(sf("sfAMMID")) {
        return Ok(Ter::TEC_AMM_ACCOUNT);
    }

    match amount.asset() {
        Asset::Issue(issue) => {
            let issuer_sle = issuer_sle.as_ref().expect("issuer existence checked above");
            // The issuer permission check precedes the trust-line read in
            // rippled's preclaimHelper<Issue>.
            if !issuer_sle.is_flag(lsfAllowTrustLineClawback) || issuer_sle.is_flag(lsfNoFreeze) {
                return Ok(Ter::TEC_NO_PERMISSION);
            }
            let line = read(view, protocol::line(holder, issuer, issue.currency))?;
            let Some(line) = line else {
                return Ok(Ter::TEC_NO_LINE);
            };
            let balance_sign = line.get_field_amount(sf("sfBalance")).signum();
            if (balance_sign > 0 && issuer < holder) || (balance_sign < 0 && issuer > holder) {
                return Ok(Ter::TEC_NO_PERMISSION);
            }
            // Clawback deliberately overloads Amount.Issue.issuer with the
            // holder. Reconstruct the canonical issuer before accountHolds.
            if !account_holds_positive(
                view,
                holder,
                Asset::Issue(protocol::Issue::new(issue.currency, issuer)),
            )? {
                return Ok(Ter::TEC_INSUFFICIENT_FUNDS);
            }
        }
        Asset::MPTIssue(issue) => {
            let issuance = mpt_issuance(view, issue.mpt_id())?;
            let Some(issuance) = issuance else {
                return Ok(Ter::TEC_OBJECT_NOT_FOUND);
            };
            if !issuance.is_flag(lsfMPTCanClawback)
                || issuance.get_account_id(sf("sfIssuer")) != issuer
            {
                return Ok(Ter::TEC_NO_PERMISSION);
            }
            let token = mptoken(view, issue.mpt_id(), holder)?;
            let Some(_token) = token else {
                return Ok(Ter::TEC_OBJECT_NOT_FOUND);
            };
            if !account_holds_positive(view, holder, Asset::MPTIssue(issue))? {
                return Ok(Ter::TEC_INSUFFICIENT_FUNDS);
            }
        }
    }
    Ok(Ter::TES_SUCCESS)
}

fn preclaim_mpt_authorize<V: ReadView>(view: &V, tx: &STTx) -> Result<Ter, Ter> {
    let account_id = tx.get_account_id(sf("sfAccount"));
    let issuance_id = tx.get_field_h192(sf("sfMPTokenIssuanceID"));
    let holder = tx
        .is_field_present(sf("sfHolder"))
        .then(|| tx.get_account_id(sf("sfHolder")));

    if let Some(holder) = holder {
        let holder_sle = account(view, holder)?;
        let Some(holder_sle) = holder_sle else {
            return Ok(Ter::TEC_NO_DST);
        };
        let Some(issuance) = mpt_issuance(view, issuance_id)? else {
            return Ok(Ter::TEC_OBJECT_NOT_FOUND);
        };
        if issuance.get_account_id(sf("sfIssuer")) != account_id {
            return Ok(Ter::TEC_NO_PERMISSION);
        }
        if !issuance.is_flag(lsfMPTRequireAuth) {
            return Ok(Ter::TEC_NO_AUTH);
        }
        let holder_token = mptoken(view, issuance_id, holder)?;
        if holder_token.is_none() {
            return Ok(Ter::TEC_OBJECT_NOT_FOUND);
        }
        return Ok(if pseudo_account(&holder_sle) {
            Ter::TEC_NO_PERMISSION
        } else {
            Ter::TES_SUCCESS
        });
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
                    confidential_transfer_enabled: false,
                    confidential_outstanding_nonzero: false,
                    token_has_confidential_balance: false,
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
        if view
            .rules()
            .enabled(&protocol::feature_confidential_transfer())
        {
            let issuance = mpt_issuance(view, issuance_id)?;
            if issuance.is_some_and(|issuance| {
                issuance.is_field_present(sf("sfConfidentialOutstandingAmount"))
                    && issuance.get_field_u64(sf("sfConfidentialOutstandingAmount")) != 0
                    && (token.is_field_present(sf("sfConfidentialBalanceInbox"))
                        || token.is_field_present(sf("sfConfidentialBalanceSpending")))
            }) {
                return Ok(Ter::TEC_HAS_OBLIGATIONS);
            }
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
            confidential_transfer_enabled: false,
            confidential_outstanding_nonzero: false,
            token_has_confidential_balance: false,
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
    let Some(issuance) = issuance else {
        return Ok(Ter::TEC_OBJECT_NOT_FOUND);
    };
    if !issuance.is_flag(lsfMPTCanLock) {
        if !view.rules().enabled(&feature_single_asset_vault())
            && !view.rules().enabled(&protocol::feature_id("DynamicMPT"))
        {
            return Ok(Ter::TEC_NO_PERMISSION);
        }
        if (tx.get_flags() & (protocol::tfMPTLock | protocol::tfMPTUnlock)) != 0 {
            return Ok(Ter::TEC_NO_PERMISSION);
        }
    }
    if issuance.get_account_id(sf("sfIssuer")) != tx.get_account_id(sf("sfAccount")) {
        return Ok(Ter::TEC_NO_PERMISSION);
    }
    let holder = tx
        .is_field_present(sf("sfHolder"))
        .then(|| tx.get_account_id(sf("sfHolder")));
    // MPTokenIssuanceSet::preclaim reads and validates the holder AccountRoot
    // before touching its MPToken.  Preserve that short-circuit so a corrupt
    // token read cannot replace the canonical tecNO_DST result.
    let (holder_account_exists, holder_token_exists) = if let Some(holder) = holder {
        if account(view, holder)?.is_none() {
            return Ok(Ter::TEC_NO_DST);
        }
        (true, mptoken(view, issuance_id, holder)?.is_some())
    } else {
        (false, false)
    };
    let domain = tx
        .is_field_present(sf("sfDomainID"))
        .then(|| tx.get_field_h256(sf("sfDomainID")));
    if domain.is_some() && !issuance.is_flag(lsfMPTRequireAuth) {
        return Ok(Ter::TEC_NO_PERMISSION);
    }
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
            holder_account_exists,
            holder_token_exists,
            domain_id_present: domain.is_some(),
            domain_id_is_zero: domain.is_some_and(|id| id.is_zero()),
            issuance_requires_auth: issuance.is_flag(lsfMPTRequireAuth),
            domain_exists: domain_sle.is_some_and(|entry| entry.is_some()),
            issuance_domain_present: issuance.is_field_present(sf("sfDomainID")),
            current_mutable_flags: issuance.get_field_u32(sf("sfImmutableFlags")),
            mutable_flags: tx
                .is_field_present(sf("sfImmutableFlags"))
                .then(|| tx.get_field_u32(sf("sfImmutableFlags"))),
            metadata_present: tx.is_field_present(sf("sfMPTokenMetadata")),
            transfer_fee: tx
                .is_field_present(sf("sfTransferFee"))
                .then(|| tx.get_field_u16(sf("sfTransferFee"))),
            issuance_can_transfer: issuance.is_flag(lsfMPTCanTransfer),
            issuance_has_confidential_balance: issuance
                .is_flag(protocol::lsfMPTCanHoldConfidentialBalance),
            issuance_transfer_fee_nonzero: issuance.is_field_present(sf("sfTransferFee"))
                && issuance.get_field_u16(sf("sfTransferFee")) > 0,
            issuer_encryption_key_present: issuance.is_field_present(sf("sfIssuerEncryptionKey")),
            auditor_encryption_key_present: issuance.is_field_present(sf("sfAuditorEncryptionKey")),
            tx_has_issuer_encryption_key: tx.is_field_present(sf("sfIssuerEncryptionKey")),
            tx_has_auditor_encryption_key: tx.is_field_present(sf("sfAuditorEncryptionKey")),
            confidential_outstanding_nonzero: issuance
                .is_field_present(sf("sfConfidentialOutstandingAmount"))
                && issuance.get_field_u64(sf("sfConfidentialOutstandingAmount")) > 0,
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
    use std::{
        collections::{BTreeMap, BTreeSet},
        sync::Arc,
    };

    use basics::base_uint::{Uint160, Uint256};
    use ledger::{Fees, LedgerHeader, ReadView, ReadViewTx, Rules, ViewError};
    use protocol::{
        AccountID, IOUAmount, Issue, Keylet, LedgerEntryType, STAmount, STLedgerEntry, STTx, Ter,
        TxType, XRPAmount, account_keylet, currency_from_string, feature_single_asset_vault, line,
        lsfAllowTrustLineClawback, lsfMPTCanLock, lsfMPTRequireAuth, sf_generic,
    };

    use super::{run_token_read_view_preclaim, sf};

    #[derive(Debug, Default)]
    struct View {
        entries: BTreeMap<Uint256, Arc<STLedgerEntry>>,
        fail_reads: BTreeSet<Uint256>,
        rules: Rules,
        zero_iou_balance_hook: bool,
        zero_mpt_balance_hook: bool,
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
            self.rules.clone()
        }
        fn exists(&self, keylet: Keylet) -> Result<bool, ViewError> {
            Ok(self.entries.contains_key(&keylet.key))
        }
        fn succ(&self, _: Uint256, _: Option<Uint256>) -> Result<Option<Uint256>, ViewError> {
            Ok(None)
        }
        fn read(&self, keylet: Keylet) -> Result<Option<Arc<STLedgerEntry>>, ViewError> {
            if self.fail_reads.contains(&keylet.key) {
                return Err(ViewError::Conversion("fault-injected token read".into()));
            }
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
        fn balance_hook_iou(
            &self,
            _account: AccountID,
            _issuer: AccountID,
            amount: STAmount,
        ) -> STAmount {
            if self.zero_iou_balance_hook {
                let issue = amount.issue();
                STAmount::from_iou_amount(sf_generic(), IOUAmount::new(), issue)
            } else {
                amount
            }
        }
        fn balance_hook_mpt(
            &self,
            _account: AccountID,
            issue: protocol::MPTIssue,
            amount: i64,
        ) -> STAmount {
            STAmount::from_mpt_amount(
                sf_generic(),
                protocol::MPTAmount::from_value(if self.zero_mpt_balance_hook {
                    0
                } else {
                    amount
                }),
                issue,
            )
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
    fn trust_set_preserves_pinned_short_circuit_read_order() {
        let source = account(1);
        let destination = account(2);
        let currency = currency_from_string("USD");
        let mut view = View::default();
        for id in [source, destination] {
            let keylet = account_keylet(Uint160::from_void(id.data()));
            let mut root =
                STLedgerEntry::from_type_and_key(LedgerEntryType::AccountRoot, keylet.key);
            root.set_account_id(sf("sfAccount"), id);
            root.set_field_u32(sf("sfFlags"), 0);
            view.entries.insert(keylet.key, Arc::new(root));
        }
        let destination_key = account_keylet(Uint160::from_void(destination.data())).key;
        view.fail_reads.insert(destination_key);
        let limit = STAmount::from_iou_amount(
            sf("sfLimitAmount"),
            IOUAmount::from_parts(1, 0).expect("valid amount"),
            Issue::new(currency, destination),
        );
        let auth = STTx::new(TxType::TRUST_SET, |tx| {
            tx.set_account_id(sf("sfAccount"), source);
            tx.set_field_amount(sf("sfLimitAmount"), limit.clone());
            tx.set_field_u32(sf("sfFlags"), protocol::tfSetfAuth);
        });
        assert_eq!(
            run_token_read_view_preclaim(&view, &auth, TxType::TRUST_SET),
            Some(Ter::TEF_NO_AUTH_REQUIRED),
            "RequireAuth rejection precedes the destination AccountRoot read"
        );

        view.fail_reads.remove(&destination_key);
        view.fail_reads
            .insert(line(source, destination, currency).key);
        let ordinary = STTx::new(TxType::TRUST_SET, |tx| {
            tx.set_account_id(sf("sfAccount"), source);
            tx.set_field_amount(sf("sfLimitAmount"), limit.clone());
        });
        assert_eq!(
            run_token_read_view_preclaim(&view, &ordinary, TxType::TRUST_SET),
            Some(Ter::TES_SUCCESS),
            "ordinary TrustSet does not read the trust line before apply"
        );

        let source_key = account_keylet(Uint160::from_void(source.data())).key;
        let mut missing_destination = View {
            rules: Rules::new([protocol::feature_amm()]),
            ..View::default()
        };
        missing_destination.entries.insert(
            source_key,
            view.entries
                .get(&source_key)
                .expect("source fixture")
                .clone(),
        );
        missing_destination
            .fail_reads
            .insert(line(source, destination, currency).key);
        assert_eq!(
            run_token_read_view_preclaim(&missing_destination, &ordinary, TxType::TRUST_SET),
            Some(Ter::TEC_NO_DST),
            "missing destination precedes every trust-line read once AMM is active"
        );

        view.rules = Rules::new([protocol::feature_deep_freeze()]);
        let conflicting_freeze = STTx::new(TxType::TRUST_SET, |tx| {
            tx.set_account_id(sf("sfAccount"), source);
            tx.set_field_amount(sf("sfLimitAmount"), limit);
            tx.set_field_u32(
                sf("sfFlags"),
                protocol::tfSetFreeze | protocol::tfClearFreeze,
            );
        });
        assert_eq!(
            run_token_read_view_preclaim(&view, &conflicting_freeze, TxType::TRUST_SET),
            Some(Ter::TEC_NO_PERMISSION),
            "contradictory freeze flags precede the final trust-line state read"
        );
    }

    #[test]
    fn iou_clawback_reconstructs_the_real_issuer_for_account_holds() {
        let issuer = account(1);
        let holder = account(2);
        let currency = currency_from_string("USD");
        let mut view = View::default();

        for (account, flags) in [(issuer, lsfAllowTrustLineClawback), (holder, 0)] {
            let keylet = account_keylet(Uint160::from_void(account.data()));
            let mut root =
                STLedgerEntry::from_type_and_key(LedgerEntryType::AccountRoot, keylet.key);
            root.set_account_id(sf("sfAccount"), account);
            root.set_field_u32(sf("sfFlags"), flags);
            view.entries.insert(keylet.key, Arc::new(root));
        }

        // issuer < holder, so a negative low-side balance means the holder
        // owns positive issuer-issued USD. The transaction wire amount uses
        // the holder in Issue.issuer by Clawback definition.
        let line_keylet = line(issuer, holder, currency);
        let iou = |value, account| {
            STAmount::from_iou_amount(
                sf_generic(),
                IOUAmount::from_parts(value, 0).expect("valid IOU amount"),
                Issue::new(currency, account),
            )
        };
        let mut trust =
            STLedgerEntry::from_type_and_key(LedgerEntryType::RippleState, line_keylet.key);
        trust.set_field_amount(sf("sfBalance"), iou(-50, protocol::no_account()));
        trust.set_field_amount(sf("sfLowLimit"), iou(0, issuer));
        trust.set_field_amount(sf("sfHighLimit"), iou(0, holder));
        view.entries.insert(line_keylet.key, Arc::new(trust));

        let clawback = STTx::new(TxType::CLAWBACK, |tx| {
            tx.set_account_id(sf("sfAccount"), issuer);
            tx.set_field_amount(sf("sfAmount"), iou(10, holder));
        });
        assert_eq!(
            run_token_read_view_preclaim(&view, &clawback, TxType::CLAWBACK),
            Some(Ter::TES_SUCCESS),
            "accountHolds must use the transaction source as the IOU issuer"
        );
    }

    #[test]
    fn clawback_rejects_holder_and_issuer_permissions_before_asset_storage_reads() {
        let issuer = account(1);
        let holder = account(2);
        let currency = currency_from_string("USD");
        let mut view = View::default();

        for id in [issuer, holder] {
            let keylet = account_keylet(Uint160::from_void(id.data()));
            let mut root =
                STLedgerEntry::from_type_and_key(LedgerEntryType::AccountRoot, keylet.key);
            root.set_account_id(sf("sfAccount"), id);
            root.set_field_u32(sf("sfFlags"), 0);
            view.entries.insert(keylet.key, Arc::new(root));
        }
        let line_keylet = line(issuer, holder, currency);
        view.fail_reads.insert(line_keylet.key);
        let amount = STAmount::from_iou_amount(
            sf("sfAmount"),
            IOUAmount::from_parts(1, 0).expect("valid amount"),
            Issue::new(currency, holder),
        );
        let clawback = STTx::new(TxType::CLAWBACK, |tx| {
            tx.set_account_id(sf("sfAccount"), issuer);
            tx.set_field_amount(sf("sfAmount"), amount);
        });

        assert_eq!(
            run_token_read_view_preclaim(&view, &clawback, TxType::CLAWBACK),
            Some(Ter::TEC_NO_PERMISSION),
            "issuer clawback permission precedes the trust-line read"
        );

        let issuer_keylet = account_keylet(Uint160::from_void(issuer.data()));
        let mut issuer_root =
            STLedgerEntry::from_type_and_key(LedgerEntryType::AccountRoot, issuer_keylet.key);
        issuer_root.set_account_id(sf("sfAccount"), issuer);
        issuer_root.set_field_u32(sf("sfFlags"), lsfAllowTrustLineClawback);
        view.entries
            .insert(issuer_keylet.key, Arc::new(issuer_root));

        let holder_keylet = account_keylet(Uint160::from_void(holder.data()));
        let mut holder_root =
            STLedgerEntry::from_type_and_key(LedgerEntryType::AccountRoot, holder_keylet.key);
        holder_root.set_account_id(sf("sfAccount"), holder);
        holder_root.set_field_u32(sf("sfFlags"), 0);
        holder_root.set_field_h256(sf("sfVaultID"), Uint256::from_u64(7));
        view.entries
            .insert(holder_keylet.key, Arc::new(holder_root));
        view.rules = Rules::new([feature_single_asset_vault()]);

        assert_eq!(
            run_token_read_view_preclaim(&view, &clawback, TxType::CLAWBACK),
            Some(Ter::TEC_PSEUDO_ACCOUNT),
            "pseudo-account rejection precedes every asset-specific read"
        );
    }

    #[test]
    fn clawback_uses_canonical_balance_hooks_for_iou_and_mpt_funds() {
        let issuer = account(1);
        let holder = account(2);
        let currency = currency_from_string("USD");
        let mut view = View::default();

        for (id, flags) in [(issuer, lsfAllowTrustLineClawback), (holder, 0)] {
            let keylet = account_keylet(Uint160::from_void(id.data()));
            let mut root =
                STLedgerEntry::from_type_and_key(LedgerEntryType::AccountRoot, keylet.key);
            root.set_account_id(sf("sfAccount"), id);
            root.set_field_u32(sf("sfFlags"), flags);
            view.entries.insert(keylet.key, Arc::new(root));
        }

        let iou = |value, account| {
            STAmount::from_iou_amount(
                sf_generic(),
                IOUAmount::from_parts(value, 0).expect("valid IOU amount"),
                Issue::new(currency, account),
            )
        };
        let line_keylet = line(issuer, holder, currency);
        let mut trust =
            STLedgerEntry::from_type_and_key(LedgerEntryType::RippleState, line_keylet.key);
        trust.set_field_amount(sf("sfBalance"), iou(-5, protocol::no_account()));
        trust.set_field_amount(sf("sfLowLimit"), iou(0, issuer));
        trust.set_field_amount(sf("sfHighLimit"), iou(0, holder));
        view.entries.insert(line_keylet.key, Arc::new(trust));
        view.zero_iou_balance_hook = true;
        let iou_clawback = STTx::new(TxType::CLAWBACK, |tx| {
            tx.set_account_id(sf("sfAccount"), issuer);
            tx.set_field_amount(sf("sfAmount"), iou(1, holder));
        });
        assert_eq!(
            run_token_read_view_preclaim(&view, &iou_clawback, TxType::CLAWBACK),
            Some(Ter::TEC_INSUFFICIENT_FUNDS),
            "IOU accountHolds must observe deferred-credit balanceHookIOU"
        );

        view.zero_iou_balance_hook = false;
        view.zero_mpt_balance_hook = true;
        view.rules = Rules::new([protocol::feature_id("MPTokensV2")]);
        let issue = protocol::MPTIssue::new(protocol::make_mpt_id(7, issuer));
        let issuance_keylet = protocol::mpt_issuance_keylet_from_mptid(issue.mpt_id());
        let mut issuance =
            STLedgerEntry::from_type_and_key(LedgerEntryType::MPTokenIssuance, issuance_keylet.key);
        issuance.set_account_id(sf("sfIssuer"), issuer);
        issuance.set_field_u32(sf("sfFlags"), protocol::lsfMPTCanClawback);
        view.entries.insert(issuance_keylet.key, Arc::new(issuance));
        let token_keylet =
            protocol::mptoken_keylet_from_mptid(issue.mpt_id(), Uint160::from_void(holder.data()));
        let mut token =
            STLedgerEntry::from_type_and_key(LedgerEntryType::MPToken, token_keylet.key);
        token.set_account_id(sf("sfAccount"), holder);
        token.set_field_h192(sf("sfMPTokenIssuanceID"), issue.mpt_id());
        token.set_field_u64(sf("sfMPTAmount"), 5);
        view.entries.insert(token_keylet.key, Arc::new(token));
        let mpt_clawback = STTx::new(TxType::CLAWBACK, |tx| {
            tx.set_account_id(sf("sfAccount"), issuer);
            tx.set_account_id(sf("sfHolder"), holder);
            tx.set_field_amount(
                sf("sfAmount"),
                STAmount::from_mpt_amount(
                    sf("sfAmount"),
                    protocol::MPTAmount::from_value(1),
                    issue,
                ),
            );
        });
        assert_eq!(
            run_token_read_view_preclaim(&view, &mpt_clawback, TxType::CLAWBACK),
            Some(Ter::TEC_INSUFFICIENT_FUNDS),
            "MPTokensV2 accountHolds must observe deferred-credit balanceHookMPT"
        );
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

    #[test]
    fn mpt_set_issuance_permissions_precede_holder_storage_reads() {
        let issuance_id = basics::base_uint::Uint192::from_u64(7);
        let issuer = account(1);
        let other = account(2);
        let holder = account(3);
        let issuance_keylet = protocol::mpt_issuance_keylet_from_mptid(issuance_id);
        let mut issuance =
            STLedgerEntry::from_type_and_key(LedgerEntryType::MPTokenIssuance, issuance_keylet.key);
        issuance.set_account_id(sf("sfIssuer"), issuer);
        issuance.set_field_u32(sf("sfFlags"), lsfMPTCanLock);
        issuance.set_field_u32(sf("sfImmutableFlags"), 0);

        let mut view = View::default();
        view.entries.insert(issuance_keylet.key, Arc::new(issuance));
        view.fail_reads
            .insert(protocol::account_keylet(Uint160::from_void(holder.data())).key);
        let tx = STTx::new(TxType::MPTOKEN_ISSUANCE_SET, |tx| {
            tx.set_account_id(sf("sfAccount"), other);
            tx.set_account_id(sf("sfHolder"), holder);
            tx.set_field_h192(sf("sfMPTokenIssuanceID"), issuance_id);
        });

        assert_eq!(
            run_token_read_view_preclaim(&view, &tx, TxType::MPTOKEN_ISSUANCE_SET),
            Some(Ter::TEC_NO_PERMISSION),
            "issuer mismatch must short-circuit before the holder AccountRoot read"
        );

        let mut missing_holder_view = View::default();
        let issuance_keylet = protocol::mpt_issuance_keylet_from_mptid(issuance_id);
        let mut issuance =
            STLedgerEntry::from_type_and_key(LedgerEntryType::MPTokenIssuance, issuance_keylet.key);
        issuance.set_account_id(sf("sfIssuer"), issuer);
        issuance.set_field_u32(sf("sfFlags"), lsfMPTCanLock);
        issuance.set_field_u32(sf("sfImmutableFlags"), 0);
        missing_holder_view
            .entries
            .insert(issuance_keylet.key, Arc::new(issuance));
        missing_holder_view.fail_reads.insert(
            protocol::mptoken_keylet_from_mptid(issuance_id, Uint160::from_void(holder.data())).key,
        );
        let missing_holder_tx = STTx::new(TxType::MPTOKEN_ISSUANCE_SET, |tx| {
            tx.set_account_id(sf("sfAccount"), issuer);
            tx.set_account_id(sf("sfHolder"), holder);
            tx.set_field_h192(sf("sfMPTokenIssuanceID"), issuance_id);
        });
        assert_eq!(
            run_token_read_view_preclaim(
                &missing_holder_view,
                &missing_holder_tx,
                TxType::MPTOKEN_ISSUANCE_SET,
            ),
            Some(Ter::TEC_NO_DST),
            "missing holder AccountRoot must precede the holder MPToken read"
        );
    }

    #[test]
    fn mpt_authorize_issuer_permissions_precede_holder_token_storage_reads() {
        let issuance_id = basics::base_uint::Uint192::from_u64(8);
        let issuer = account(1);
        let submitter = account(2);
        let holder = account(3);
        let issuance_keylet = protocol::mpt_issuance_keylet_from_mptid(issuance_id);
        let mut issuance =
            STLedgerEntry::from_type_and_key(LedgerEntryType::MPTokenIssuance, issuance_keylet.key);
        issuance.set_account_id(sf("sfIssuer"), issuer);
        issuance.set_field_u32(sf("sfFlags"), lsfMPTRequireAuth);
        let holder_keylet = protocol::account_keylet(Uint160::from_void(holder.data()));
        let mut holder_root =
            STLedgerEntry::from_type_and_key(LedgerEntryType::AccountRoot, holder_keylet.key);
        holder_root.set_account_id(sf("sfAccount"), holder);
        holder_root.set_field_u32(sf("sfFlags"), 0);

        let mut view = View::default();
        view.entries.insert(issuance_keylet.key, Arc::new(issuance));
        view.entries
            .insert(holder_keylet.key, Arc::new(holder_root));
        view.fail_reads.insert(
            protocol::mptoken_keylet_from_mptid(issuance_id, Uint160::from_void(holder.data())).key,
        );
        let tx = STTx::new(TxType::MPTOKEN_AUTHORIZE, |tx| {
            tx.set_account_id(sf("sfAccount"), submitter);
            tx.set_account_id(sf("sfHolder"), holder);
            tx.set_field_h192(sf("sfMPTokenIssuanceID"), issuance_id);
        });

        assert_eq!(
            run_token_read_view_preclaim(&view, &tx, TxType::MPTOKEN_AUTHORIZE),
            Some(Ter::TEC_NO_PERMISSION),
            "issuer mismatch must precede the holder MPToken existence read"
        );
    }

    #[test]
    fn mpt_unauthorize_confidential_issuance_storage_failure_is_hard() {
        let issuance_id = basics::base_uint::Uint192::from_u64(9);
        let holder = account(3);
        let token_keylet =
            protocol::mptoken_keylet_from_mptid(issuance_id, Uint160::from_void(holder.data()));
        let mut token =
            STLedgerEntry::from_type_and_key(LedgerEntryType::MPToken, token_keylet.key);
        token.set_account_id(sf("sfAccount"), holder);
        token.set_field_h192(sf("sfMPTokenIssuanceID"), issuance_id);
        token.set_field_u64(sf("sfMPTAmount"), 0);
        token.set_field_u32(sf("sfFlags"), 0);
        token.set_stbase(protocol::STBlob::from_buffer(
            sf("sfConfidentialBalanceInbox"),
            basics::buffer::Buffer::from(&[1_u8][..]),
        ));

        let issuance_keylet = protocol::mpt_issuance_keylet_from_mptid(issuance_id);
        let mut view = View {
            rules: Rules::new([protocol::feature_confidential_transfer()]),
            ..View::default()
        };
        view.entries.insert(token_keylet.key, Arc::new(token));
        view.fail_reads.insert(issuance_keylet.key);
        let tx = STTx::new(TxType::MPTOKEN_AUTHORIZE, |tx| {
            tx.set_account_id(sf("sfAccount"), holder);
            tx.set_field_h192(sf("sfMPTokenIssuanceID"), issuance_id);
            tx.set_field_u32(sf("sfFlags"), protocol::tfMPTUnauthorize);
        });

        assert_eq!(
            run_token_read_view_preclaim(&view, &tx, TxType::MPTOKEN_AUTHORIZE),
            Some(Ter::TEF_BAD_LEDGER)
        );
    }
}
