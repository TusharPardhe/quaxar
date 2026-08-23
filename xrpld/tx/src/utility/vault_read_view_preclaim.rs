//! Immutable `ReadView` preclaim helpers for the Vault transaction family.
//!
//! This module owns only Vault transaction types. It performs ledger reads only,
//! deliberately creates no sandbox, and returns `None` for every unowned type.
//! Read failures are ledger failures (`tefBAD_LEDGER`), never success.

use basics::base_uint::{Uint160, Uint192};
use ledger::{ReadView, RelativeDistanceAmount};
use protocol::{
    AccountID, Asset, MPTIssue, STAmount, STLedgerEntry, STTx, Ter, TxType, get_field_by_symbol,
    lsfAccepted, lsfAllowTrustLineClawback, lsfGlobalFreeze, lsfHighAuth, lsfHighFreeze,
    lsfLowAuth, lsfLowFreeze, lsfMPTCanClawback, lsfMPTLocked, lsfMPTRequireAuth, lsfNoFreeze,
    lsfRequireAuth,
};

use crate::{
    VaultClawbackPreclaimFacts, VaultClawbackSelectedAmountAssetKind, VaultClawbackVaultAssetKind,
    VaultCreatePreclaimFacts, VaultDeletePreclaimFacts, VaultDepositPreclaimFacts,
    VaultSetPreclaimFacts, VaultWithdrawPreclaimFrontFacts, VaultWithdrawPreclaimTailFacts,
    VaultWithdrawRequireAuthType, VaultWithdrawShareBranchResult, run_vault_clawback_preclaim,
    run_vault_create_preclaim, run_vault_delete_preclaim, run_vault_deposit_preclaim,
    run_vault_set_preclaim, run_vault_withdraw_preclaim,
};

fn sf(name: &str) -> &'static protocol::SField {
    get_field_by_symbol(name)
}

fn account_keylet(account: AccountID) -> protocol::Keylet {
    protocol::account_keylet(Uint160::from_void(account.data()))
}

fn read_error() -> Ter {
    Ter::TEF_BAD_LEDGER
}

fn read_account<V: ReadView>(
    view: &V,
    account: AccountID,
) -> Result<Option<std::sync::Arc<STLedgerEntry>>, Ter> {
    view.read(account_keylet(account)).map_err(|_| read_error())
}

fn read_vault<V: ReadView>(
    view: &V,
    vault_id: basics::base_uint::Uint256,
) -> Result<Option<std::sync::Arc<STLedgerEntry>>, Ter> {
    view.read(protocol::vault_keylet_from_key(vault_id))
        .map_err(|_| read_error())
}

fn read_issuance<V: ReadView>(
    view: &V,
    id: Uint192,
) -> Result<Option<std::sync::Arc<STLedgerEntry>>, Ter> {
    view.read(protocol::mpt_issuance_keylet_from_mptid(id))
        .map_err(|_| read_error())
}

fn asset_frozen<V: ReadView>(view: &V, account: AccountID, asset: Asset) -> Result<Ter, Ter> {
    match asset {
        Asset::Issue(issue) if issue.native() || issue.account == account => Ok(Ter::TES_SUCCESS),
        Asset::Issue(issue) => {
            let issuer = read_account(view, issue.account)?;
            let global = issuer.is_some_and(|sle| sle.is_flag(lsfGlobalFreeze));
            let line = view
                .read(protocol::line(account, issue.account, issue.currency))
                .map_err(|_| read_error())?;
            let individual = line.is_some_and(|sle| {
                sle.is_flag(if account > issue.account {
                    lsfHighFreeze
                } else {
                    lsfLowFreeze
                })
            });
            Ok(if global || individual {
                Ter::TEC_FROZEN
            } else {
                Ter::TES_SUCCESS
            })
        }
        Asset::MPTIssue(issue) => Ok(
            if ledger::mptoken_helpers::is_frozen_mpt(view, &account, &issue)
                .map_err(|_| read_error())?
            {
                Ter::TEC_LOCKED
            } else {
                Ter::TES_SUCCESS
            },
        ),
    }
}

fn require_auth<V: ReadView>(
    view: &V,
    account: AccountID,
    asset: Asset,
    strong: bool,
) -> Result<Ter, Ter> {
    match asset {
        Asset::Issue(issue) if issue.native() || issue.account == account => Ok(Ter::TES_SUCCESS),
        Asset::Issue(issue) => {
            let line = view
                .read(protocol::line(account, issue.account, issue.currency))
                .map_err(|_| read_error())?;
            if line.is_none() && strong {
                return Ok(Ter::TEC_NO_LINE);
            }
            let issuer_requires_auth =
                read_account(view, issue.account)?.is_some_and(|sle| sle.is_flag(lsfRequireAuth));
            if !issuer_requires_auth {
                return Ok(Ter::TES_SUCCESS);
            }
            let Some(line) = line else {
                return Ok(Ter::TEC_NO_LINE);
            };
            Ok(
                if line.is_flag(if account > issue.account {
                    lsfLowAuth
                } else {
                    lsfHighAuth
                }) {
                    Ter::TES_SUCCESS
                } else {
                    Ter::TEC_NO_AUTH
                },
            )
        }
        Asset::MPTIssue(issue) => ledger::mptoken_helpers::require_auth_mpt_with_type(
            view,
            &issue,
            &account,
            if strong {
                ledger::mptoken_helpers::MPTAuthType::Strong
            } else {
                ledger::mptoken_helpers::MPTAuthType::Weak
            },
        )
        .map_err(|_| read_error()),
    }
}

fn can_transfer<V: ReadView>(
    view: &V,
    asset: Asset,
    from: AccountID,
    to: AccountID,
    waive_mpt_can_transfer: bool,
) -> Result<Ter, Ter> {
    match asset {
        Asset::MPTIssue(issue) if !waive_mpt_can_transfer => {
            ledger::mptoken_helpers::can_transfer_mpt(view, &issue, &from, &to)
                .map_err(|_| read_error())
        }
        // Issued-currency transferability is enforced by the trust-line and
        // authorization/freeze reads below, exactly as the family preclaims do.
        _ => Ok(Ter::TES_SUCCESS),
    }
}

fn asset_holds<V: ReadView>(
    view: &V,
    account: AccountID,
    requested: &STAmount,
) -> Result<STAmount, Ter> {
    match requested.asset() {
        Asset::Issue(issue) if issue.native() => {
            let Some(root) = read_account(view, account)? else {
                return Ok(requested.zeroed());
            };
            Ok(root.get_field_amount(sf("sfBalance")))
        }
        Asset::Issue(issue) if issue.account == account => Ok(requested.clone()),
        Asset::Issue(issue) => {
            let Some(line) = view
                .read(protocol::line(account, issue.account, issue.currency))
                .map_err(|_| read_error())?
            else {
                return Ok(requested.zeroed());
            };
            let mut amount = line.get_field_amount(sf("sfBalance"));
            if account > issue.account {
                amount.negate();
            }
            amount.set_issuer(issue.account);
            Ok(amount)
        }
        Asset::MPTIssue(issue) if issue.issuer() == account => Ok(requested.clone()),
        Asset::MPTIssue(issue) => {
            let Some(token) = view
                .read(protocol::mptoken_keylet_from_mptid(
                    issue.mpt_id(),
                    Uint160::from_void(account.data()),
                ))
                .map_err(|_| read_error())?
            else {
                return Ok(requested.zeroed());
            };
            Ok(STAmount::from_mpt_amount(
                sf("sfAmount"),
                protocol::MPTAmount::from_value(token.get_field_u64(sf("sfMPTAmount")) as i64),
                issue,
            ))
        }
    }
}

fn valid_domain<V: ReadView>(
    view: &V,
    account: AccountID,
    domain_id: basics::base_uint::Uint256,
) -> Result<Ter, Ter> {
    let Some(domain) = view
        .read(protocol::permissioned_domain_keylet_from_id(domain_id))
        .map_err(|_| read_error())?
    else {
        return Ok(Ter::TEC_OBJECT_NOT_FOUND);
    };
    let mut expired = false;
    for accepted in domain.get_field_array(sf("sfAcceptedCredentials")).iter() {
        let issuer = accepted.get_account_id(sf("sfIssuer"));
        let credential_type = accepted.get_field_vl(sf("sfCredentialType"));
        let keylet = protocol::credential_keylet(
            Uint160::from_void(account.data()),
            Uint160::from_void(issuer.data()),
            &credential_type,
        );
        let Some(credential) = view.read(keylet).map_err(|_| read_error())? else {
            continue;
        };
        let is_expired = credential.is_field_present(sf("sfExpiration"))
            && view.parent_close_time().as_seconds()
                >= credential.get_field_u32(sf("sfExpiration"));
        if is_expired {
            expired = true;
            continue;
        }
        if credential.is_flag(lsfAccepted) {
            return Ok(Ter::TES_SUCCESS);
        }
    }
    Ok(if expired {
        Ter::TEC_EXPIRED
    } else {
        Ter::TEC_NO_PERMISSION
    })
}

fn preclaim_create<V: ReadView>(view: &V, tx: &STTx) -> Result<Ter, Ter> {
    let account = tx.get_account_id(sf("sfAccount"));
    let asset = tx.get_field_issue(sf("sfAsset")).asset();
    let domain = tx
        .is_field_present(sf("sfDomainID"))
        .then(|| tx.get_field_h256(sf("sfDomainID")));
    let sequence = tx.get_seq_value();
    Ok(run_vault_create_preclaim(
        VaultCreatePreclaimFacts {
            asset_is_native: asset.native(),
            asset_is_issue: matches!(asset, Asset::Issue(_)),
            domain_id_present: domain.is_some(),
        },
        || ledger::can_add_holding(view, &asset),
        || {
            !asset.native()
                && read_account(view, asset.issuer())
                    .map(|entry| {
                        entry.is_some_and(|sle| {
                            sle.is_field_present(sf("sfVaultID"))
                                || sle.is_field_present(sf("sfLoanBrokerID"))
                                || sle.is_field_present(sf("sfAMMID"))
                        })
                    })
                    .unwrap_or(true)
        },
        || {
            asset_frozen(view, account, asset)
                .map(|ter| ter != Ter::TES_SUCCESS)
                .unwrap_or(true)
        },
        || {
            domain
                .map(|id| {
                    view.read(protocol::permissioned_domain_keylet_from_id(id))
                        .map(|entry| entry.is_some())
                        .unwrap_or(false)
                })
                .unwrap_or(true)
        },
        || {
            ledger::pseudo_account_address(
                view,
                protocol::vault_keylet(Uint160::from_void(account.data()), sequence).key,
            )
            .is_zero()
        },
    ))
}

fn preclaim_delete<V: ReadView>(view: &V, tx: &STTx) -> Result<Ter, Ter> {
    let account = tx.get_account_id(sf("sfAccount"));
    let Some(vault) = read_vault(view, tx.get_field_h256(sf("sfVaultID")))? else {
        return Ok(run_vault_delete_preclaim(
            VaultDeletePreclaimFacts::default(),
        ));
    };
    let issuance = read_issuance(view, vault.get_field_h192(sf("sfShareMPTID")))?;
    Ok(run_vault_delete_preclaim(VaultDeletePreclaimFacts {
        vault_exists: true,
        submitter_is_owner: vault.get_account_id(sf("sfOwner")) == account,
        assets_available_is_zero: vault.get_field_number(sf("sfAssetsAvailable")).value()
            == basics::number::NumberParts::zero(),
        assets_total_is_zero: vault.get_field_number(sf("sfAssetsTotal")).value()
            == basics::number::NumberParts::zero(),
        issuance_exists: issuance.is_some(),
        issuance_issuer_matches_pseudo: issuance.as_ref().is_some_and(|sle| {
            sle.get_account_id(sf("sfIssuer")) == vault.get_account_id(sf("sfAccount"))
        }),
        outstanding_amount_is_zero: issuance
            .as_ref()
            .is_some_and(|sle| sle.get_field_u64(sf("sfOutstandingAmount")) == 0),
    }))
}

fn preclaim_set<V: ReadView>(view: &V, tx: &STTx) -> Result<Ter, Ter> {
    let account = tx.get_account_id(sf("sfAccount"));
    let Some(vault) = read_vault(view, tx.get_field_h256(sf("sfVaultID")))? else {
        return Ok(run_vault_set_preclaim(VaultSetPreclaimFacts::default()));
    };
    let issuance = read_issuance(view, vault.get_field_h192(sf("sfShareMPTID")))?;
    let domain = tx
        .is_field_present(sf("sfDomainID"))
        .then(|| tx.get_field_h256(sf("sfDomainID")));
    Ok(run_vault_set_preclaim(VaultSetPreclaimFacts {
        vault_exists: true,
        submitter_is_owner: vault.get_account_id(sf("sfOwner")) == account,
        issuance_exists: issuance.is_some(),
        domain_id_present: domain.is_some(),
        domain_id_is_zero: domain.is_some_and(|id| id.is_zero()),
        vault_is_private: vault.is_flag(protocol::lsfVaultPrivate),
        domain_exists: domain
            .filter(|id| !id.is_zero())
            .map(|id| {
                view.read(protocol::permissioned_domain_keylet_from_id(id))
                    .map(|entry| entry.is_some())
                    .unwrap_or(false)
            })
            .unwrap_or(false),
        issuance_requires_auth: issuance
            .as_ref()
            .is_some_and(|sle| sle.is_flag(lsfMPTRequireAuth)),
    }))
}

fn preclaim_deposit<V: ReadView>(view: &V, tx: &STTx) -> Result<Ter, Ter> {
    let account = tx.get_account_id(sf("sfAccount"));
    let Some(vault) = read_vault(view, tx.get_field_h256(sf("sfVaultID")))? else {
        return Ok(run_vault_deposit_preclaim(
            VaultDepositPreclaimFacts::default(),
            || Ter::TES_SUCCESS,
            || Ter::TES_SUCCESS,
            || Ter::TES_SUCCESS,
            || Ter::TES_SUCCESS,
        ));
    };
    let amount = tx.get_field_amount(sf("sfAmount"));
    let asset = vault.get_field_issue(sf("sfAsset")).asset();
    let share = MPTIssue::new(vault.get_field_h192(sf("sfShareMPTID")));
    let issuance = read_issuance(view, share.mpt_id())?;
    let private = vault.is_flag(protocol::lsfVaultPrivate);
    let domain = issuance.as_ref().and_then(|sle| {
        sle.is_field_present(sf("sfDomainID"))
            .then(|| sle.get_field_h256(sf("sfDomainID")))
    });
    let frozen_asset = asset_frozen(view, account, asset)?;
    let frozen_share = asset_frozen(view, account, Asset::MPTIssue(share))?;
    let holdings = asset_holds(view, account, &amount)?;
    Ok(run_vault_deposit_preclaim(
        VaultDepositPreclaimFacts {
            vault_exists: true,
            deposited_asset_matches_vault_asset: amount.asset() == asset,
            vault_share_matches_vault_asset: Asset::MPTIssue(share) == amount.asset(),
            issuance_exists: issuance.is_some(),
            issuance_locked: issuance
                .as_ref()
                .is_some_and(|sle| sle.is_flag(lsfMPTLocked)),
            vault_asset_is_issue: matches!(asset, Asset::Issue(_)),
            vault_asset_frozen_for_account: frozen_asset != Ter::TES_SUCCESS,
            vault_share_frozen_for_account: frozen_share != Ter::TES_SUCCESS,
            fix_cleanup_3_3_0_enabled: false,
            vault_is_private: private,
            submitter_is_owner: account == vault.get_account_id(sf("sfOwner")),
            domain_id_present: domain.is_some(),
            account_holds_sufficient_assets: holdings >= amount,
        },
        || {
            can_transfer(
                view,
                asset,
                account,
                vault.get_account_id(sf("sfAccount")),
                false,
            )
            .unwrap_or(Ter::TEF_BAD_LEDGER)
        },
        || {
            domain
                .map(|id| valid_domain(view, account, id).unwrap_or(Ter::TEF_BAD_LEDGER))
                .unwrap_or(Ter::TES_SUCCESS)
        },
        || require_auth(view, account, asset, false).unwrap_or(Ter::TEF_BAD_LEDGER),
        || frozen_asset,
    ))
}

fn preclaim_withdraw<V: ReadView>(view: &V, tx: &STTx) -> Result<Ter, Ter> {
    let account = tx.get_account_id(sf("sfAccount"));
    let destination = if tx.is_field_present(sf("sfDestination")) {
        tx.get_account_id(sf("sfDestination"))
    } else {
        account
    };
    let Some(vault) = read_vault(view, tx.get_field_h256(sf("sfVaultID")))? else {
        return Ok(run_vault_withdraw_preclaim(
            VaultWithdrawPreclaimFrontFacts::default(),
            VaultWithdrawPreclaimTailFacts::default(),
            || Ter::TES_SUCCESS,
            || VaultWithdrawShareBranchResult::Success,
            || Ter::TES_SUCCESS,
            |_| Ter::TES_SUCCESS,
            || Ter::TES_SUCCESS,
            || Ter::TES_SUCCESS,
            || Ter::TES_SUCCESS,
        ));
    };
    let amount = tx.get_field_amount(sf("sfAmount"));
    let asset = vault.get_field_issue(sf("sfAsset")).asset();
    let share = MPTIssue::new(vault.get_field_h192(sf("sfShareMPTID")));
    let issuance = read_issuance(view, share.mpt_id())?;
    let available = vault.get_field_number(sf("sfAssetsAvailable")).value();
    let direct = if amount.asset() == asset && amount.as_number() <= available {
        Ter::TES_SUCCESS
    } else {
        Ter::TEC_INSUFFICIENT_FUNDS
    };
    let share_branch = if let Some(issuance) = issuance.as_ref() {
        match ledger::vault_helpers::shares_to_assets_withdraw(
            &vault,
            issuance,
            &amount,
            ledger::vault_helpers::WaiveUnrealizedLoss::No,
        ) {
            Some(converted) if converted.as_number() <= available => {
                VaultWithdrawShareBranchResult::Success
            }
            Some(_) => {
                VaultWithdrawShareBranchResult::CanWithdrawFailure(Ter::TEC_INSUFFICIENT_FUNDS)
            }
            None => VaultWithdrawShareBranchResult::MissingConvertedAssets,
        }
    } else {
        VaultWithdrawShareBranchResult::MissingConvertedAssets
    };
    Ok(run_vault_withdraw_preclaim(
        VaultWithdrawPreclaimFrontFacts {
            vault_exists: true,
            amount_asset_matches_vault_asset_or_share: amount.asset() == asset
                || amount.asset() == Asset::MPTIssue(share),
            withdrawal_policy_is_first_come_first_serve: vault
                .get_field_u8(sf("sfWithdrawalPolicy"))
                == protocol::VAULT_STRATEGY_FIRST_COME_FIRST_SERVE,
            fix_cleanup_3_1_3_enabled: view
                .rules()
                .enabled(&protocol::feature_id("fixCleanup3_1_3")),
            amount_asset_is_vault_share: amount.asset() == Asset::MPTIssue(share),
            share_issuance_exists: issuance.is_some(),
        },
        VaultWithdrawPreclaimTailFacts {
            destination_is_submitter: destination == account,
            fix_cleanup_3_3_0_enabled: false,
        },
        || {
            can_transfer(
                view,
                asset,
                vault.get_account_id(sf("sfAccount")),
                destination,
                view.rules()
                    .enabled(&protocol::feature_id("fixCleanup3_2_0")),
            )
            .unwrap_or(Ter::TEF_BAD_LEDGER)
        },
        || share_branch,
        || direct,
        |auth| {
            require_auth(
                view,
                destination,
                asset,
                matches!(auth, VaultWithdrawRequireAuthType::StrongAuth),
            )
            .unwrap_or(Ter::TEF_BAD_LEDGER)
        },
        || asset_frozen(view, destination, asset).unwrap_or(Ter::TEF_BAD_LEDGER),
        || asset_frozen(view, account, Asset::MPTIssue(share)).unwrap_or(Ter::TEF_BAD_LEDGER),
        || asset_frozen(view, destination, asset).unwrap_or(Ter::TEF_BAD_LEDGER),
    ))
}

fn preclaim_clawback<V: ReadView>(view: &V, tx: &STTx) -> Result<Ter, Ter> {
    let account = tx.get_account_id(sf("sfAccount"));
    let holder = tx.get_account_id(sf("sfHolder"));
    let Some(vault) = read_vault(view, tx.get_field_h256(sf("sfVaultID")))? else {
        return Ok(run_vault_clawback_preclaim(
            VaultClawbackPreclaimFacts::default(),
        ));
    };
    let asset = vault.get_field_issue(sf("sfAsset")).asset();
    let share = MPTIssue::new(vault.get_field_h192(sf("sfShareMPTID")));
    let issuance = read_issuance(view, share.mpt_id())?;
    let amount = if tx.is_field_present(sf("sfAmount")) {
        tx.get_field_amount(sf("sfAmount"))
    } else {
        STAmount::new_with_asset(sf("sfAmount"), Asset::MPTIssue(share), 0, 0, false)
    };
    let kind = if amount.asset() == Asset::MPTIssue(share) {
        VaultClawbackSelectedAmountAssetKind::Share
    } else if amount.asset() == asset {
        VaultClawbackSelectedAmountAssetKind::VaultAsset
    } else {
        VaultClawbackSelectedAmountAssetKind::Other
    };
    let issuer = asset.issuer();
    let issuer_sle = if matches!(asset, Asset::Issue(_)) {
        read_account(view, account)?
    } else {
        None
    };
    let mpt = match asset {
        Asset::MPTIssue(issue) => read_issuance(view, issue.mpt_id())?,
        _ => None,
    };
    let shares_held = asset_holds(
        view,
        holder,
        &STAmount::from_mpt_amount(
            sf("sfAmount"),
            protocol::MPTAmount::from_value(amount.mpt().value()),
            share,
        ),
    )?;
    Ok(run_vault_clawback_preclaim(VaultClawbackPreclaimFacts {
        vault_exists: true,
        share_issuance_exists: issuance.is_some(),
        amount_present: tx.is_field_present(sf("sfAmount")),
        vault_asset_is_native: asset.native(),
        vault_asset_issuer_is_owner: !asset.native()
            && issuer == vault.get_account_id(sf("sfOwner")),
        selected_amount_asset_kind: kind,
        submitter_is_owner: account == vault.get_account_id(sf("sfOwner")),
        vault_shares_total_is_zero: issuance
            .as_ref()
            .is_none_or(|sle| sle.get_field_u64(sf("sfOutstandingAmount")) == 0),
        vault_assets_total_is_zero: vault.get_field_number(sf("sfAssetsTotal")).value()
            == basics::number::NumberParts::zero(),
        vault_assets_available_is_zero: vault.get_field_number(sf("sfAssetsAvailable")).value()
            == basics::number::NumberParts::zero(),
        selected_amount_is_zero: amount.signum() == 0,
        selected_amount_matches_shares_held: amount == shares_held,
        submitter_is_vault_asset_issuer: account == issuer,
        submitter_is_holder: account == holder,
        vault_asset_kind: if matches!(asset, Asset::MPTIssue(_)) {
            VaultClawbackVaultAssetKind::Mpt
        } else {
            VaultClawbackVaultAssetKind::Issue
        },
        mpt_vault_asset_issuance_exists: mpt.is_some(),
        mpt_vault_asset_can_clawback: mpt
            .as_ref()
            .is_some_and(|sle| sle.is_flag(lsfMPTCanClawback)),
        issuer_account_exists: issuer_sle.is_some(),
        issuer_allows_trustline_clawback: issuer_sle
            .as_ref()
            .is_some_and(|sle| sle.is_flag(lsfAllowTrustLineClawback)),
        issuer_has_no_freeze: issuer_sle
            .as_ref()
            .is_some_and(|sle| sle.is_flag(lsfNoFreeze)),
    }))
}

/// Evaluates the Vault typed preclaim tail against an immutable ledger view.
///
/// `None` means that `txn_type` is not a Vault transaction. This is never a
/// success fallback; callers must explicitly continue to another family.
pub fn run_vault_read_view_preclaim<V: ReadView>(
    view: &V,
    tx: &STTx,
    txn_type: TxType,
) -> Option<Ter> {
    let result = match txn_type {
        TxType::VAULT_CREATE => preclaim_create(view, tx),
        TxType::VAULT_SET => preclaim_set(view, tx),
        TxType::VAULT_DELETE => preclaim_delete(view, tx),
        TxType::VAULT_DEPOSIT => preclaim_deposit(view, tx),
        TxType::VAULT_WITHDRAW => preclaim_withdraw(view, tx),
        TxType::VAULT_CLAWBACK => preclaim_clawback(view, tx),
        _ => return None,
    };
    Some(result.unwrap_or_else(|ter| ter))
}

#[cfg(test)]
mod tests {
    use std::{collections::BTreeMap, sync::Arc};

    use basics::base_uint::Uint256;
    use ledger::{Fees, LedgerHeader, ReadView, ReadViewTx, Rules, ViewError};
    use protocol::{AccountID, Asset, LedgerEntryType, STIssue, STLedgerEntry, STTx, Ter, TxType};

    use super::{run_vault_read_view_preclaim, sf};

    #[derive(Debug, Default)]
    struct View {
        entries: BTreeMap<Uint256, Arc<STLedgerEntry>>,
    }
    impl View {
        fn insert(&mut self, entry: STLedgerEntry) {
            self.entries.insert(*entry.key(), Arc::new(entry));
        }
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
        fn exists(&self, keylet: protocol::Keylet) -> Result<bool, ViewError> {
            Ok(self.entries.contains_key(&keylet.key))
        }
        fn succ(&self, _: Uint256, _: Option<Uint256>) -> Result<Option<Uint256>, ViewError> {
            Ok(None)
        }
        fn read(&self, keylet: protocol::Keylet) -> Result<Option<Arc<STLedgerEntry>>, ViewError> {
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
    fn account(n: u8) -> AccountID {
        AccountID::from_array([n; 20])
    }
    fn vault(id: Uint256, owner: AccountID, pseudo: AccountID) -> STLedgerEntry {
        let mut entry = STLedgerEntry::from_type_and_key(LedgerEntryType::Vault, id);
        entry.set_account_id(sf("sfOwner"), owner);
        entry.set_account_id(sf("sfAccount"), pseudo);
        entry.set_field_issue(
            sf("sfAsset"),
            STIssue::new_with_asset(sf("sfAsset"), Asset::Issue(protocol::xrp_issue())),
        );
        entry.set_field_h192(sf("sfShareMPTID"), basics::base_uint::Uint192::zero());
        entry.set_field_number(sf("sfAssetsAvailable"), protocol::STNumber::default());
        entry.set_field_number(sf("sfAssetsTotal"), protocol::STNumber::default());
        entry
    }
    #[test]
    fn vault_helper_returns_none_for_unowned_type() {
        let tx = STTx::new(TxType::PAYMENT, |_| {});
        assert_eq!(
            run_vault_read_view_preclaim(&View::default(), &tx, TxType::PAYMENT),
            None
        );
    }
    #[test]
    fn vault_delete_reads_missing_vault_without_mutating_view() {
        let id = Uint256::from_u64(1);
        let tx = STTx::new(TxType::VAULT_DELETE, |tx| {
            tx.set_field_h256(sf("sfVaultID"), id);
            tx.set_account_id(sf("sfAccount"), account(1));
        });
        let view = View::default();
        assert_eq!(
            run_vault_read_view_preclaim(&view, &tx, TxType::VAULT_DELETE),
            Some(Ter::TEC_NO_ENTRY)
        );
        assert!(view.entries.is_empty());
    }
    #[test]
    fn vault_set_rejects_non_owner_before_missing_share_issuance() {
        let id = Uint256::from_u64(2);
        let mut view = View::default();
        view.insert(vault(id, account(1), account(2)));
        let tx = STTx::new(TxType::VAULT_SET, |tx| {
            tx.set_field_h256(sf("sfVaultID"), id);
            tx.set_account_id(sf("sfAccount"), account(3));
        });
        assert_eq!(
            run_vault_read_view_preclaim(&view, &tx, TxType::VAULT_SET),
            Some(Ter::TEC_NO_PERMISSION)
        );
        assert_eq!(view.entries.len(), 1, "ReadView helper must not mutate");
    }
}
