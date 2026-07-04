use std::collections::{BTreeMap, BTreeSet};

use basics::base_uint::Uint256;
use ledger::{ApplyView, FlowSandbox, ReadView, flow_sandbox::Action};
use protocol::{AccountID, LedgerEntryType, STAmount, STTx, Ter, XRPAmount, get_field_by_symbol};

mod amm;
mod clawback;
mod common;
mod directory;
mod entry;
mod lending;
mod mpt;
mod object_deletion;
mod permissioned_dex;
mod permissioned_domain;
mod vault;

use amm::*;
use clawback::*;
use common::sf;
use directory::*;
use entry::*;
use lending::*;
use mpt::*;
use object_deletion::*;
use permissioned_dex::*;
use permissioned_domain::*;
use vault::*;

pub fn check_invariants_for_tx<V: ApplyView>(
    sandbox: &FlowSandbox<V>,
    tx: &STTx,
    result: Ter,
    fee: XRPAmount,
) -> Ter {
    let txn_type = tx.get_txn_type();
    let tx_domain = tx
        .is_field_present(sf("sfDomainID"))
        .then(|| tx.get_field_h256(sf("sfDomainID")));
    let tx_account = tx
        .is_field_present(sf("sfAccount"))
        .then(|| tx.get_account_id(sf("sfAccount")));
    let tx_destination = tx
        .is_field_present(sf("sfDestination"))
        .then(|| tx.get_account_id(sf("sfDestination")));
    let tx_holder = tx
        .is_field_present(sf("sfHolder"))
        .then(|| tx.get_account_id(sf("sfHolder")));
    let tx_amount = tx
        .is_field_present(sf("sfAmount"))
        .then(|| tx.get_field_amount(sf("sfAmount")));
    let tx_has_holder = tx.is_field_present(sf("sfHolder"));
    let cross_currency_payment = payment_is_cross_currency(tx);
    check_invariants_inner(
        sandbox,
        txn_type,
        tx_domain,
        tx_account,
        tx_destination,
        tx_holder,
        tx_amount,
        tx_has_holder,
        cross_currency_payment,
        result,
        fee,
    )
}

pub fn check_invariants<V: ApplyView>(
    sandbox: &FlowSandbox<V>,
    txn_type: protocol::TxType,
    result: Ter,
    fee: XRPAmount,
) -> Ter {
    check_invariants_inner(
        sandbox, txn_type, None, None, None, None, None, false, false, result, fee,
    )
}

fn payment_is_cross_currency(tx: &STTx) -> bool {
    if tx.get_txn_type() != protocol::TxType::PAYMENT || !tx.is_field_present(sf("sfAmount")) {
        return false;
    }

    let amount = tx.get_field_amount(sf("sfAmount"));
    let send_max = if tx.is_field_present(sf("sfSendMax")) {
        tx.get_field_amount(sf("sfSendMax"))
    } else {
        amount.clone()
    };
    send_max.asset() != amount.asset()
}

fn check_invariants_inner<V: ApplyView>(
    sandbox: &FlowSandbox<V>,
    txn_type: protocol::TxType,
    tx_domain: Option<Uint256>,
    tx_account: Option<AccountID>,
    tx_destination: Option<AccountID>,
    tx_holder: Option<AccountID>,
    tx_amount: Option<STAmount>,
    tx_has_holder: bool,
    cross_currency_payment: bool,
    result: Ter,
    fee: XRPAmount,
) -> Ter {
    let mut xrp_balance_change: i64 = 0;
    let fix_cleanup_3_1_3 = sandbox
        .rules()
        .enabled(&protocol::feature_id("fixCleanup3_1_3"));
    let fix_cleanup_3_2_0 = sandbox
        .rules()
        .enabled(&protocol::feature_id("fixCleanup3_2_0"));
    let amm_invariant_enabled =
        fix_cleanup_3_2_0 || sandbox.rules().enabled(&protocol::fix_ammv1_3());
    let single_asset_vault_enabled = sandbox
        .rules()
        .enabled(&protocol::feature_id("SingleAssetVault"));
    let vault_invariant_enabled = fix_cleanup_3_2_0 || single_asset_vault_enabled;
    let lending_protocol_enabled = sandbox
        .rules()
        .enabled(&protocol::feature_id("LendingProtocol"));
    let mptokens_v2_enabled = sandbox.rules().enabled(&protocol::feature_id("MPTokensV2"));
    let mpt_transfer_invariant_enabled = fix_cleanup_3_2_0 || mptokens_v2_enabled;
    let permissioned_dex_invariant_enabled = fix_cleanup_3_2_0
        || sandbox
            .rules()
            .enabled(&protocol::feature_id("PermissionedDEX"));
    let mut directory_roots = BTreeSet::new();
    let mut mpt_accounting = BTreeMap::new();
    let mut mpt_transfers = BTreeMap::new();
    let mut mpt_issuance_lifecycle = MptIssuanceLifecycle::default();
    let mut permissioned_domain = PermissionedDomainState::default();
    let mut permissioned_dex = PermissionedDexState::default();
    let mut amm = AmmState::default();
    let mut vault = VaultState::default();
    let mut lending = LendingState::default();
    let mut clawback = ClawbackState::default();
    let mut object_deletion = ObjectDeletionState::default();
    let fix_cleanup_3_3_0 = sandbox
        .rules()
        .enabled(&protocol::fix_cleanup_3_3_0());

    for (index, entry) in sandbox.items() {
        let is_delete = entry.action == Action::Erase;
        let after = if is_delete { None } else { Some(&entry.sle) };
        let before = sandbox
            .peek_parent(protocol::Keylet::new(
                after
                    .map(|a| a.get_type())
                    .unwrap_or_else(|| entry.sle.get_type()),
                *index,
            ))
            .ok()
            .flatten();

        let before_sle = before.as_deref();
        let after_sle = after.map(|s| &**s);

        // 4. LedgerEntryTypesMatch
        if let (Some(b), Some(a)) = (before_sle, after_sle) {
            if b.get_type() != a.get_type() {
                return Ter::TEC_INVARIANT_FAILED;
            }
        }

        // 2. AccountRootsNotDeleted
        if is_delete {
            let sle_to_delete = before_sle.unwrap_or(&*entry.sle);
            if sle_to_delete.get_type() == LedgerEntryType::AccountRoot {
                if txn_type != protocol::TxType::ACCOUNT_DELETE
                    && txn_type != protocol::TxType::VAULT_DELETE
                    && txn_type != protocol::TxType::LOAN_BROKER_DELETE
                    && txn_type != protocol::TxType::AMM_DELETE
                    && txn_type != protocol::TxType::AMM_WITHDRAW
                    && txn_type != protocol::TxType::AMM_CLAWBACK
                {
                    return Ter::TEC_INVARIANT_FAILED;
                }
            }
        }

        let sle_type = after_sle
            .map(|s| s.get_type())
            .unwrap_or_else(|| before_sle.unwrap_or(&*entry.sle).get_type());

        if amm_invariant_enabled {
            record_amm_state(&mut amm, is_delete, before_sle, after_sle);
        }
        if vault_invariant_enabled {
            record_vault_state(&mut vault, is_delete, before_sle, after_sle);
        }
        if lending_protocol_enabled {
            record_lending_state(sandbox, &mut lending, after_sle);
        }
        if fix_cleanup_3_1_3 || txn_type == protocol::TxType::PERMISSIONED_DOMAIN_SET {
            record_permissioned_domain_state(
                &mut permissioned_domain,
                is_delete,
                before_sle,
                after_sle,
            );
        }

        if mpt_transfer_invariant_enabled {
            if let Some(b) = before_sle {
                record_mpt_accounting(&mut mpt_accounting, b, true);
                record_mpt_transfer(&mut mpt_transfers, b, true);
            }
            if let Some(a) = after_sle {
                record_mpt_accounting(&mut mpt_accounting, a, false);
                record_mpt_transfer(&mut mpt_transfers, a, false);
                if fix_cleanup_3_2_0 && protocol::has_invalid_amount(&a.clone_as_object()) {
                    return Ter::TEC_INVARIANT_FAILED;
                }
            }
        }

        if permissioned_dex_invariant_enabled {
            record_permissioned_dex(&mut permissioned_dex, is_delete, before_sle, after_sle);
        }
        record_clawback_state(&mut clawback, before_sle);

        if fix_cleanup_3_3_0 {
            record_object_deletion_state(&mut object_deletion, is_delete, before_sle);
        }

        if fix_cleanup_3_2_0 {
            let deleted_sle = before_sle.unwrap_or(&entry.sle);
            record_mpt_issuance_lifecycle(
                sandbox,
                txn_type,
                &mut mpt_issuance_lifecycle,
                is_delete,
                before_sle,
                after_sle,
                deleted_sle,
            );
            if !maybe_record_directory_root(&mut directory_roots, is_delete, before_sle, after_sle)
            {
                return Ter::TEC_INVARIANT_FAILED;
            }
        }

        match sle_type {
            LedgerEntryType::AccountRoot => {
                // 8. XRPBalanceChecks
                if let Some(a) = after_sle {
                    let balance_field = get_field_by_symbol("sfBalance");
                    if a.is_field_present(balance_field) {
                        let bal = a.get_field_amount(balance_field);
                        if bal.negative() || bal.xrp().drops() > protocol::INITIAL_XRP.drops() {
                            return Ter::TEC_INVARIANT_FAILED;
                        }
                    }
                }

                // 7. ValidNewAccountRoot
                // when DeletableAccounts is enabled (always on testnet/mainnet).
                if entry.action == Action::Insert {
                    if let Some(a) = after_sle {
                        let seq = a.get_field_u32(get_field_by_symbol("sfSequence"));
                        let expected_seq = sandbox.header().seq;
                        if seq != expected_seq && seq != 0 {
                            return Ter::TEC_INVARIANT_FAILED;
                        }
                    }
                }

                // 1. XRPNotCreated (AccountRoot)
                let bal_before = before_sle
                    .map(|b| {
                        b.get_field_amount(get_field_by_symbol("sfBalance"))
                            .xrp()
                            .drops() as i64
                    })
                    .unwrap_or(0);
                let bal_after = after_sle
                    .map(|a| {
                        a.get_field_amount(get_field_by_symbol("sfBalance"))
                            .xrp()
                            .drops() as i64
                    })
                    .unwrap_or(0);
                xrp_balance_change += bal_after - bal_before;
            }
            LedgerEntryType::Escrow => {
                // 6. NoZeroEscrow
                if let Some(a) = after_sle {
                    let amt = a.get_field_amount(get_field_by_symbol("sfAmount"));
                    if amt.signum() <= 0 {
                        return Ter::TEC_INVARIANT_FAILED;
                    }
                }

                // 1. XRPNotCreated (Escrow). Token escrows are covered by
                // token-specific accounting; only native amounts affect XRP.
                let bal_before = before_sle
                    .map(|b| b.get_field_amount(get_field_by_symbol("sfAmount")))
                    .filter(|amount| amount.native())
                    .map(|amount| amount.xrp().drops())
                    .unwrap_or(0);
                let bal_after = after_sle
                    .map(|a| a.get_field_amount(get_field_by_symbol("sfAmount")))
                    .filter(|amount| amount.native())
                    .map(|amount| amount.xrp().drops())
                    .unwrap_or(0);
                xrp_balance_change += bal_after - bal_before;
            }
            LedgerEntryType::PayChannel => {
                // 1. XRPNotCreated (PayChannel)
                let bal_before = before_sle
                    .map(|b| {
                        b.get_field_amount(get_field_by_symbol("sfAmount"))
                            .xrp()
                            .drops() as i64
                    })
                    .unwrap_or(0);
                let bal_after = after_sle
                    .map(|a| {
                        a.get_field_amount(get_field_by_symbol("sfAmount"))
                            .xrp()
                            .drops() as i64
                    })
                    .unwrap_or(0);
                xrp_balance_change += bal_after - bal_before;
            }
            LedgerEntryType::Offer => {
                // 5. NoBadOffers
                if let Some(a) = after_sle {
                    let gets = a.get_field_amount(get_field_by_symbol("sfTakerGets"));
                    let pays = a.get_field_amount(get_field_by_symbol("sfTakerPays"));
                    if gets.negative()
                        || gets.mantissa() == 0
                        || pays.negative()
                        || pays.mantissa() == 0
                    {
                        return Ter::TEC_INVARIANT_FAILED;
                    }
                }
            }
            LedgerEntryType::DirectoryNode => {}
            LedgerEntryType::RippleState => {
                if let Some(a) = after_sle
                    && !validate_ripple_state_entry(a)
                {
                    return Ter::TEC_INVARIANT_FAILED;
                }
            }
            LedgerEntryType::MPTokenIssuance | LedgerEntryType::MPToken => {
                if fix_cleanup_3_2_0
                    && let Some(a) = after_sle
                    && !validate_mpt_entry(a)
                {
                    return Ter::TEC_INVARIANT_FAILED;
                }
            }
            LedgerEntryType::Vault => {
                if vault_invariant_enabled
                    && let Some(a) = after_sle
                    && !validate_vault_entry(a)
                {
                    return Ter::TEC_INVARIANT_FAILED;
                }
            }
            LedgerEntryType::AMM => {
                if amm_invariant_enabled
                    && amm_invariant_result_applies(result)
                    && let Some(a) = after_sle
                    && !validate_amm_entry(a)
                {
                    return Ter::TEC_INVARIANT_FAILED;
                }
            }
            LedgerEntryType::Loan => {
                if lending_protocol_enabled
                    && let Some(a) = after_sle
                    && !validate_loan_entry(before_sle, a)
                {
                    return Ter::TEC_INVARIANT_FAILED;
                }
            }
            LedgerEntryType::LoanBroker => {
                if lending_protocol_enabled
                    && let Some(a) = after_sle
                    && !validate_loan_broker_entry(
                        sandbox,
                        txn_type,
                        fix_cleanup_3_1_3,
                        before_sle,
                        a,
                    )
                {
                    return Ter::TEC_INVARIANT_FAILED;
                }
            }
            _ => {}
        }
    }

    if (fix_cleanup_3_1_3 || txn_type == protocol::TxType::PERMISSIONED_DOMAIN_SET)
        && !validates_permissioned_domain(txn_type, result, fix_cleanup_3_1_3, &permissioned_domain)
    {
        return Ter::TEC_INVARIANT_FAILED;
    }

    if permissioned_dex_invariant_enabled {
        if !validates_permissioned_dex(
            sandbox,
            txn_type,
            result,
            tx_domain,
            fix_cleanup_3_1_3,
            fix_cleanup_3_2_0,
            &permissioned_dex,
        ) {
            return Ter::TEC_INVARIANT_FAILED;
        }
    }

    if !validates_clawback(
        sandbox,
        txn_type,
        result,
        tx_account,
        tx_holder,
        tx_amount.as_ref(),
        mptokens_v2_enabled,
        &clawback,
    ) {
        return Ter::TEC_INVARIANT_FAILED;
    }

    if fix_cleanup_3_2_0 {
        if !validates_mpt_issuance_lifecycle(&mpt_issuance_lifecycle) {
            return Ter::TEC_INVARIANT_FAILED;
        }
        if !validates_mpt_lifecycle_counts(
            txn_type,
            result,
            tx_has_holder,
            single_asset_vault_enabled,
            lending_protocol_enabled,
            mptokens_v2_enabled,
            &mpt_issuance_lifecycle,
        ) {
            return Ter::TEC_INVARIANT_FAILED;
        }
        for root_index in directory_roots {
            if !matches!(
                sandbox.read(protocol::Keylet::new(
                    LedgerEntryType::DirectoryNode,
                    root_index
                )),
                Ok(Some(_))
            ) {
                return Ter::TEC_INVARIANT_FAILED;
            }
        }
    }

    if mpt_transfer_invariant_enabled {
        if !validates_mpt_accounting(&mpt_accounting, mptokens_v2_enabled) {
            return Ter::TEC_INVARIANT_FAILED;
        }
        if !validates_mpt_transfers(
            sandbox,
            txn_type,
            cross_currency_payment,
            fix_cleanup_3_2_0,
            mptokens_v2_enabled,
            &mpt_transfers,
        ) {
            return Ter::TEC_INVARIANT_FAILED;
        }
    }

    if amm_invariant_enabled && !validates_amm_state(sandbox, txn_type, result, &amm) {
        return Ter::TEC_INVARIANT_FAILED;
    }

    if vault_invariant_enabled
        && !validates_vault_state(
            sandbox,
            txn_type,
            tx_account,
            tx_destination,
            tx_holder,
            tx_amount.as_ref(),
            fix_cleanup_3_2_0,
            result,
            fee,
            &vault,
        )
    {
        return Ter::TEC_INVARIANT_FAILED;
    }

    if fix_cleanup_3_3_0 && !validates_object_deletion(sandbox, &object_deletion) {
        return Ter::TEC_INVARIANT_FAILED;
    }

    if lending_protocol_enabled {
        for broker_id in lending.broker_refs {
            if !matches!(
                sandbox.read(protocol::loan_broker_keylet_from_key(broker_id)),
                Ok(Some(_))
            ) {
                return Ter::TEC_INVARIANT_FAILED;
            }
        }
    }

    // 1. XRPNotCreated (finalize)
    // Since our sandbox does not contain the fee deduction (it's applied to the parent view),
    // the net XRP change inside the sandbox MUST be <= 0.
    if xrp_balance_change > 0 {
        return Ter::TEC_INVARIANT_FAILED;
    }

    // 3. TransactionFeeCheck
    if fee.drops() < 0 || fee.drops() > protocol::INITIAL_XRP.drops() {
        return Ter::TEC_INVARIANT_FAILED;
    }

    result
}

#[cfg(test)]
mod tests {
    use super::vault::{
        VaultAssetDelta, VaultSnapshot, compute_vault_min_scale, rounded_vault_delta,
    };
    use basics::{
        base_uint::Uint256,
        number::{NumberParts as RuntimeNumber, get_mantissa_scale},
    };
    use protocol::{AccountID, Asset, Issue};

    fn account(byte: u8) -> AccountID {
        AccountID::from_array([byte; 20])
    }

    fn usd_asset() -> Asset {
        Asset::Issue(Issue {
            currency: protocol::currency_from_string("USD"),
            account: account(0xA1),
        })
    }

    fn vault_snapshot_with_scale(scale: Option<i32>) -> VaultSnapshot {
        VaultSnapshot {
            key: Uint256::from_u64(1),
            asset: usd_asset(),
            pseudo_id: account(0xA2),
            share_mpt_id: protocol::MPTIssue::new(protocol::make_mpt_id(1, account(0xA2))).mpt_id(),
            scale,
            assets_total: RuntimeNumber::from_i64(1),
            assets_available: RuntimeNumber::from_i64(1),
            loss_unrealized: RuntimeNumber::zero(),
        }
    }

    #[test]
    fn vault_invariant_min_scale_prefers_explicit_vault_scale_after_cleanup_3_2_0() {
        let before = vault_snapshot_with_scale(Some(-2));
        let after = vault_snapshot_with_scale(Some(-2));
        let delta = VaultAssetDelta {
            delta: RuntimeNumber::try_from_external_parts(12345, -4, get_mantissa_scale())
                .expect("valid delta"),
            scale: Some(-4),
        };

        assert_eq!(compute_vault_min_scale(&before, &after, delta, true), -2);
        assert_eq!(
            rounded_vault_delta(after.asset, delta, -2),
            RuntimeNumber::try_from_external_parts(123, -2, get_mantissa_scale())
                .expect("vault-scale rounded delta")
        );
    }

    #[test]
    fn vault_invariant_min_scale_preserves_legacy_coarsest_scale_before_cleanup_3_2_0() {
        let before = vault_snapshot_with_scale(Some(-2));
        let mut after = vault_snapshot_with_scale(Some(-2));
        after.assets_total =
            RuntimeNumber::try_from_external_parts(10001, -4, get_mantissa_scale())
                .expect("valid total");
        after.assets_available = after.assets_total;
        let delta = VaultAssetDelta {
            delta: RuntimeNumber::try_from_external_parts(1, -4, get_mantissa_scale())
                .expect("valid delta"),
            scale: Some(-4),
        };

        assert_eq!(compute_vault_min_scale(&before, &after, delta, false), -4);
    }
}
