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

/// Mirrors `ApplyContext::failInvariantCheck`: a broken invariant while
/// recovering a prior invariant failure is a hard failure and must not enter
/// the ledger as a fee-claim transaction.
fn invariant_failure_result(result: Ter) -> Ter {
    if matches!(
        result,
        Ter::TEC_INVARIANT_FAILED | Ter::TEF_INVARIANT_FAILED
    ) {
        Ter::TEF_INVARIANT_FAILED
    } else {
        Ter::TEC_INVARIANT_FAILED
    }
}

fn map_invariant_result(result: Ter, checked: Result<Ter, ()>) -> Ter {
    match checked {
        Ok(result) => result,
        Err(()) => invariant_failure_result(result),
    }
}

pub fn check_invariants_for_tx<V: ApplyView + ?Sized>(
    sandbox: &FlowSandbox<V>,
    tx: &STTx,
    result: Ter,
    fee: XRPAmount,
) -> Ter {
    check_invariants_for_tx_with_expected_xrp_delta(sandbox, tx, result, fee, None)
}

/// Production invariant entry point. `expected_xrp_delta` describes the
/// mutation scope being checked: handler/cleanup sandboxes exclude the fee and
/// therefore expect zero; the outer transaction sandbox includes the charged
/// fee and expects its negative. The public compatibility wrapper above keeps
/// synthetic invariant tests able to exercise one invariant in isolation.
pub(crate) fn check_invariants_for_tx_with_expected_xrp_delta<V: ApplyView + ?Sized>(
    sandbox: &FlowSandbox<V>,
    tx: &STTx,
    result: Ter,
    fee: XRPAmount,
    expected_xrp_delta: Option<i64>,
) -> Ter {
    let fee_field = sf("sfFee");
    if tx.is_field_present(fee_field) && fee.drops() > tx.get_field_amount(fee_field).xrp().drops()
    {
        return invariant_failure_result(result);
    }
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
    map_invariant_result(
        result,
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
            expected_xrp_delta,
        ),
    )
}

pub fn check_invariants<V: ApplyView + ?Sized>(
    sandbox: &FlowSandbox<V>,
    txn_type: protocol::TxType,
    result: Ter,
    fee: XRPAmount,
) -> Ter {
    map_invariant_result(
        result,
        check_invariants_inner(
            sandbox, txn_type, None, None, None, None, None, false, false, result, fee, None,
        ),
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

fn pay_channel_held_drops(amount: STAmount, balance: STAmount) -> i64 {
    amount.xrp().drops() - balance.xrp().drops()
}

fn check_invariants_inner<V: ApplyView + ?Sized>(
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
    expected_xrp_delta: Option<i64>,
) -> Result<Ter, ()> {
    let mut xrp_balance_change: i64 = 0;
    let mut has_xrp_trust_line = false;
    let mut deep_freeze_violation = false;
    let mut mpt_issuance_locked_violation = false;
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
    let fix_cleanup_3_3_0 = sandbox.rules().enabled(&protocol::fix_cleanup_3_3_0());

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
                return Err(());
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
                    return Err(());
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
                    return Err(());
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

        if fix_cleanup_3_2_0 || mptokens_v2_enabled {
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
        }

        if fix_cleanup_3_2_0 {
            if !maybe_record_directory_root(&mut directory_roots, is_delete, before_sle, after_sle)
            {
                return Err(());
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
                            return Err(());
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
                            return Err(());
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
                        return Err(());
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
                // 1. XRPNotCreated (PayChannel).  A channel's XRP still held
                // in escrow is `sfAmount - sfBalance`, not `sfAmount`.
                // PaymentChannelClaim advances sfBalance while crediting the
                // destination by the same delta; counting sfAmount alone
                // therefore reports that credit as newly-created XRP and
                // incorrectly converts a valid claim to tecINVARIANT_FAILED.
                // This mirrors rippled XRPNotCreated::visitEntry exactly,
                // including ignoring the after-value for a deleted channel
                // (closeChannel refunds the remaining held balance).
                let bal_before = before_sle
                    .map(|b| {
                        pay_channel_held_drops(
                            b.get_field_amount(get_field_by_symbol("sfAmount")),
                            b.get_field_amount(get_field_by_symbol("sfBalance")),
                        )
                    })
                    .unwrap_or(0);
                let bal_after = (!is_delete)
                    .then_some(after_sle)
                    .flatten()
                    .map(|a| {
                        pay_channel_held_drops(
                            a.get_field_amount(get_field_by_symbol("sfAmount")),
                            a.get_field_amount(get_field_by_symbol("sfBalance")),
                        )
                    })
                    .unwrap_or(0);
                xrp_balance_change += bal_after - bal_before;
            }
            LedgerEntryType::Sponsorship => {
                // A prefunded sponsorship escrows XRP in sfFeeAmount. Match
                // rippled XRPNotCreated accounting so creating, consuming, or
                // deleting that object cannot appear to mint or lose XRP.
                let fee_amount = get_field_by_symbol("sfFeeAmount");
                let bal_before = before_sle
                    .filter(|sle| sle.is_field_present(fee_amount))
                    .map(|sle| sle.get_field_amount(fee_amount).xrp().drops())
                    .unwrap_or(0);
                let bal_after = (!is_delete)
                    .then_some(after_sle)
                    .flatten()
                    .filter(|sle| sle.is_field_present(fee_amount))
                    .map(|sle| sle.get_field_amount(fee_amount).xrp().drops())
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
                        return Err(());
                    }
                }
            }
            LedgerEntryType::DirectoryNode => {}
            LedgerEntryType::RippleState => {
                if let Some(a) = after_sle {
                    has_xrp_trust_line = accumulate_invariant_violation(
                        has_xrp_trust_line,
                        is_xrp_trust_line(a),
                        fix_cleanup_3_1_3,
                    );
                    deep_freeze_violation = accumulate_invariant_violation(
                        deep_freeze_violation,
                        has_deep_freeze_without_freeze(a),
                        fix_cleanup_3_1_3,
                    );
                }
            }
            LedgerEntryType::MPTokenIssuance | LedgerEntryType::MPToken => {
                if let Some(a) = after_sle {
                    if a.get_type() == LedgerEntryType::MPTokenIssuance
                        && a.is_field_present(sf("sfLockedAmount"))
                    {
                        mpt_issuance_locked_violation = accumulate_invariant_violation(
                            mpt_issuance_locked_violation,
                            a.get_field_u64(sf("sfOutstandingAmount"))
                                < a.get_field_u64(sf("sfLockedAmount")),
                            fix_cleanup_3_1_3,
                        );
                    }
                    if fix_cleanup_3_2_0 && !validate_mpt_entry(a) {
                        return Err(());
                    }
                }
            }
            LedgerEntryType::Vault => {}
            LedgerEntryType::AMM => {
                if amm_invariant_enabled
                    && amm_invariant_result_applies(result)
                    && let Some(a) = after_sle
                    && !validate_amm_entry(a)
                {
                    return Err(());
                }
            }
            LedgerEntryType::Loan => {
                if lending_protocol_enabled
                    && let Some(a) = after_sle
                    && !validate_loan_entry(before_sle, a)
                {
                    return Err(());
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
                    return Err(());
                }
            }
            _ => {}
        }
    }

    if has_xrp_trust_line || deep_freeze_violation || mpt_issuance_locked_violation {
        return Err(());
    }

    if (fix_cleanup_3_1_3 || txn_type == protocol::TxType::PERMISSIONED_DOMAIN_SET)
        && !validates_permissioned_domain(txn_type, result, fix_cleanup_3_1_3, &permissioned_domain)
    {
        return Err(());
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
            return Err(());
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
        return Err(());
    }

    if fix_cleanup_3_2_0 || mptokens_v2_enabled {
        if !validates_mpt_issuance_lifecycle(&mpt_issuance_lifecycle) {
            return Err(());
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
            return Err(());
        }
    }

    if fix_cleanup_3_2_0 {
        for root_index in directory_roots {
            if !matches!(
                sandbox.read(protocol::Keylet::new(
                    LedgerEntryType::DirectoryNode,
                    root_index
                )),
                Ok(Some(_))
            ) {
                return Err(());
            }
        }
    }

    if mpt_transfer_invariant_enabled {
        if !validates_mpt_accounting(&mpt_accounting, mptokens_v2_enabled) {
            return Err(());
        }
        if !validates_mpt_transfers(
            sandbox,
            txn_type,
            cross_currency_payment,
            fix_cleanup_3_2_0,
            mptokens_v2_enabled,
            &mpt_transfers,
        ) {
            return Err(());
        }
    }

    if amm_invariant_enabled && !validates_amm_state(sandbox, txn_type, result, &amm) {
        return Err(());
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
            &vault,
        )
    {
        return Err(());
    }

    if fix_cleanup_3_3_0 && !validates_object_deletion(sandbox, &object_deletion) {
        return Err(());
    }

    if lending_protocol_enabled {
        for broker_id in lending.broker_refs {
            if !matches!(
                sandbox.read(protocol::loan_broker_keylet_from_key(broker_id)),
                Ok(Some(_))
            ) {
                return Err(());
            }
        }
    }

    // 1. XRPNotCreated (finalize). Production callers supply the exact delta
    // appropriate to the sandbox scope. This is the two-sandbox equivalent of
    // rippled's `-drops_ == fee`: handler/cleanup state must conserve XRP,
    // while the outer transaction delta must destroy exactly the charged fee.
    if let Some(expected) = expected_xrp_delta {
        if xrp_balance_change != expected {
            return Err(());
        }
    } else if xrp_balance_change > 0 {
        return Err(());
    }

    // 3. TransactionFeeCheck
    if fee.drops() < 0 || fee.drops() >= protocol::INITIAL_XRP.drops() {
        return Err(());
    }

    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::vault::{
        VaultAssetDelta, VaultSnapshot, VaultState, add_vault_asset_delta, compute_vault_min_scale,
        rounded_vault_delta, vault_transaction_account_asset_delta,
    };
    use super::{check_invariants_for_tx_with_expected_xrp_delta, pay_channel_held_drops};
    use basics::{
        base_uint::Uint256,
        number::{NumberParts as RuntimeNumber, get_mantissa_scale},
    };
    use ledger::{ApplyView, FlowSandbox, Ledger, LedgerHeader, Sandbox};
    use protocol::{
        AccountID, ApplyFlags, Asset, Issue, STAmount, STLedgerEntry, Ter, TxType, XRPAmount,
        account_keylet, get_field_by_symbol, pay_channel_keylet_from_key,
    };
    use std::sync::Arc;

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
    fn vault_invariant_uses_post_fee_xrp_depositor_delta() {
        let depositor = account(0xA3);
        let asset = Asset::Issue(protocol::xrp_issue());
        let mut state = VaultState::default();
        add_vault_asset_delta(
            &mut state,
            depositor,
            asset,
            RuntimeNumber::from_i64(-1_000_000),
            None,
        );

        let delta = vault_transaction_account_asset_delta(&state, depositor, asset)
            .expect("the XRP deposit transfer must retain its nonzero delta");
        assert_eq!(delta.delta, RuntimeNumber::from_i64(-1_000_000));
    }

    #[test]
    fn pay_channel_invariant_counts_only_unpaid_xrp() {
        let amount = STAmount::from_xrp_amount(XRPAmount::from_drops(500_000));
        let before_balance = STAmount::from_xrp_amount(XRPAmount::from_drops(0));
        let after_balance = STAmount::from_xrp_amount(XRPAmount::from_drops(25_000));

        let before = pay_channel_held_drops(amount.clone(), before_balance);
        let after = pay_channel_held_drops(amount, after_balance);

        assert_eq!(before, 500_000);
        assert_eq!(after, 475_000);
        assert_eq!(after - before, -25_000);
    }

    fn insert_account<V: ApplyView>(view: &mut V, id: AccountID, balance: i64) {
        let keylet = account_keylet(
            basics::base_uint::Uint160::from_slice(id.data()).expect("account width"),
        );
        let mut sle = STLedgerEntry::new(keylet);
        sle.set_account_id(get_field_by_symbol("sfAccount"), id);
        sle.set_field_amount(
            get_field_by_symbol("sfBalance"),
            STAmount::from_xrp_amount(XRPAmount::from_drops(balance)),
        );
        sle.set_field_u32(get_field_by_symbol("sfSequence"), 1);
        sle.set_field_u32(get_field_by_symbol("sfOwnerCount"), 0);
        view.insert(Arc::new(sle)).expect("insert account");
    }

    fn insert_pay_channel<V: ApplyView>(
        view: &mut V,
        key: Uint256,
        source: AccountID,
        destination: AccountID,
        amount: i64,
        balance: i64,
    ) {
        let mut sle = STLedgerEntry::new(pay_channel_keylet_from_key(key));
        sle.set_account_id(get_field_by_symbol("sfAccount"), source);
        sle.set_account_id(get_field_by_symbol("sfDestination"), destination);
        sle.set_field_amount(
            get_field_by_symbol("sfAmount"),
            STAmount::from_xrp_amount(XRPAmount::from_drops(amount)),
        );
        sle.set_field_amount(
            get_field_by_symbol("sfBalance"),
            STAmount::from_xrp_amount(XRPAmount::from_drops(balance)),
        );
        view.insert(Arc::new(sle)).expect("insert payment channel");
    }

    #[test]
    fn pay_channel_claim_destination_credit_preserves_xrp_invariant() {
        let source = account(0xC1);
        let destination = account(0xC2);
        let channel = Uint256::from_u64(0xCAFE);
        let base = Arc::new(Ledger::new(LedgerHeader::default(), false));
        let mut parent = Sandbox::new(base, ApplyFlags::NONE);
        insert_account(&mut parent, destination, 100_000_000);
        insert_pay_channel(&mut parent, channel, source, destination, 500_000, 0);

        let mut claim = FlowSandbox::new(&mut parent);
        let destination_keylet = account_keylet(
            basics::base_uint::Uint160::from_slice(destination.data()).expect("account width"),
        );
        let destination_sle = claim
            .peek(destination_keylet)
            .expect("read destination")
            .expect("destination exists");
        let mut destination_object = destination_sle.clone_as_object();
        destination_object.set_field_amount(
            get_field_by_symbol("sfBalance"),
            STAmount::from_xrp_amount(XRPAmount::from_drops(100_025_000)),
        );
        claim
            .update(Arc::new(STLedgerEntry::from_stobject(
                destination_object,
                destination_keylet.key,
            )))
            .expect("credit destination");

        let channel_keylet = pay_channel_keylet_from_key(channel);
        let channel_sle = claim
            .peek(channel_keylet)
            .expect("read channel")
            .expect("channel exists");
        let mut channel_object = channel_sle.clone_as_object();
        channel_object.set_field_amount(
            get_field_by_symbol("sfBalance"),
            STAmount::from_xrp_amount(XRPAmount::from_drops(25_000)),
        );
        claim
            .update(Arc::new(STLedgerEntry::from_stobject(
                channel_object,
                channel_keylet.key,
            )))
            .expect("advance channel balance");

        let tx = protocol::STTx::new(TxType::PAYCHAN_CLAIM, |tx| {
            tx.set_field_amount(
                get_field_by_symbol("sfFee"),
                STAmount::from_xrp_amount(XRPAmount::from_drops(0)),
            );
        });
        assert_eq!(
            check_invariants_for_tx_with_expected_xrp_delta(
                &claim,
                &tx,
                Ter::TES_SUCCESS,
                XRPAmount::from_drops(0),
                Some(0),
            ),
            Ter::TES_SUCCESS
        );
    }

    #[test]
    fn pay_channel_close_refund_preserves_xrp_invariant() {
        let source = account(0xD1);
        let destination = account(0xD2);
        let channel = Uint256::from_u64(0xD00D);
        let base = Arc::new(Ledger::new(LedgerHeader::default(), false));
        let mut parent = Sandbox::new(base, ApplyFlags::NONE);
        insert_account(&mut parent, source, 1_000_000);
        insert_pay_channel(&mut parent, channel, source, destination, 500_000, 25_000);

        let mut close = FlowSandbox::new(&mut parent);
        let source_keylet = account_keylet(
            basics::base_uint::Uint160::from_slice(source.data()).expect("account width"),
        );
        let source_sle = close
            .peek(source_keylet)
            .expect("read source")
            .expect("source exists");
        let mut source_object = source_sle.clone_as_object();
        source_object.set_field_amount(
            get_field_by_symbol("sfBalance"),
            STAmount::from_xrp_amount(XRPAmount::from_drops(1_475_000)),
        );
        close
            .update(Arc::new(STLedgerEntry::from_stobject(
                source_object,
                source_keylet.key,
            )))
            .expect("refund source");
        let channel_sle = close
            .peek(pay_channel_keylet_from_key(channel))
            .expect("read channel")
            .expect("channel exists");
        close.erase(channel_sle).expect("erase channel");

        let tx = protocol::STTx::new(TxType::PAYCHAN_CLAIM, |tx| {
            tx.set_field_amount(
                get_field_by_symbol("sfFee"),
                STAmount::from_xrp_amount(XRPAmount::from_drops(0)),
            );
        });
        assert_eq!(
            check_invariants_for_tx_with_expected_xrp_delta(
                &close,
                &tx,
                Ter::TES_SUCCESS,
                XRPAmount::from_drops(0),
                Some(0),
            ),
            Ter::TES_SUCCESS
        );
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
