//! Concrete application adapter for rippled `applySteps.cpp::invokePreclaim`.
//!
//! The typed family preclaim tail remains owned by its family dispatchers. This
//! module owns the shared ledger-backed checks and invokes that tail only after
//! the exact generic `invokePreclaim` gate has succeeded.

use basics::base_uint::{Uint160, Uint256};
use ledger::ReadView;
use protocol::{
    AccountID, ApplyFlags, NotTec, PublicKey, STLedgerEntry, STObject, STTx, Ter, calc_account_id,
    feature_batch, feature_lending_protocol, get_field_by_symbol, is_tes_success, lsfDisableMaster,
};
use std::cell::Cell;
use tx::{
    TransactorCheckFeeTx, TransactorCheckPermissionTx, TransactorCheckPriorTxAndLastLedgerTx,
    TransactorCheckSeqProxyTx, TransactorMultiSignAccountSigner, TransactorMultiSignSignerList,
    TransactorMultiSignTxSigner, TransactorSignMultiSignObject, TransactorSignObject,
    TransactorSignTx, TransactorSingleSignAccountState, run_batch_check_sign,
    run_check_tx_permission, run_transactor_check_permission,
    run_transactor_check_prior_tx_and_last_ledger, run_transactor_check_seq_proxy,
    run_transactor_invoke_preclaim, run_transactor_preclaim_check_sign,
};

fn check_sponsor<V: ReadView>(view: &V, tx: &STTx) -> NotTec {
    if !tx.is_field_present(sf("sfSponsor")) {
        return Ter::TES_SUCCESS;
    }

    let sponsor = tx.get_account_id(sf("sfSponsor"));
    let sponsor_flags = tx.get_field_u32(sf("sfSponsorFlags"));
    if tx.is_field_present(sf("sfDelegate")) && ledger::is_reserve_sponsored(sponsor_flags) {
        return Ter::TEM_INVALID;
    }
    match read_account_result(view, &sponsor) {
        Ok(Some(_)) => {}
        Ok(None) => return Ter::TER_NO_ACCOUNT,
        Err(_) => return Ter::TEF_BAD_LEDGER,
    }
    if tx.is_field_present(sf("sfSponsorSignature")) {
        return Ter::TES_SUCCESS;
    }

    let keylet = protocol::sponsorship_keylet(
        Uint160::from_void(sponsor.data()),
        Uint160::from_void(tx.get_initiator().data()),
    );
    let sponsorship = match view.read(keylet) {
        Ok(Some(sponsorship)) => sponsorship,
        Ok(None) => return Ter::TER_NO_PERMISSION,
        Err(_) => return Ter::TEF_BAD_LEDGER,
    };
    let flags = sponsorship.get_field_u32(sf("sfFlags"));
    if ledger::is_fee_sponsored(sponsor_flags)
        && flags & protocol::LSF_SPONSORSHIP_REQUIRE_SIGN_FOR_FEE != 0
    {
        return Ter::TER_NO_PERMISSION;
    }
    if ledger::is_reserve_sponsored(sponsor_flags)
        && flags & protocol::LSF_SPONSORSHIP_REQUIRE_SIGN_FOR_RESERVE != 0
    {
        return Ter::TER_NO_PERMISSION;
    }
    Ter::TES_SUCCESS
}

fn check_fee<V: ReadView>(
    view: &V,
    tx: &STTx,
    flags: ApplyFlags,
    base_fee: i64,
    minimum_fee: impl FnOnce(i64) -> i64,
) -> Ter {
    let fee = tx.get_field_amount(sf("sfFee"));
    if !fee.native() || fee.negative() || !fee.is_legal_net() {
        return Ter::TEM_BAD_FEE;
    }
    let paid = fee.xrp().drops();
    if (flags & ApplyFlags::BATCH) == ApplyFlags::BATCH {
        return if paid == 0 {
            Ter::TES_SUCCESS
        } else {
            Ter::TEM_BAD_FEE
        };
    }
    if view.open() && paid < minimum_fee(base_fee) {
        return Ter::TEL_INSUF_FEE_P;
    }
    if paid == 0 {
        return Ter::TES_SUCCESS;
    }

    let fee_sponsored = tx.is_field_present(sf("sfSponsor"))
        && ledger::is_fee_sponsored(tx.get_field_u32(sf("sfSponsorFlags")));
    let max_spendable = if fee_sponsored {
        let sponsor = tx.get_account_id(sf("sfSponsor"));
        let sponsorship_keylet = protocol::sponsorship_keylet(
            Uint160::from_void(sponsor.data()),
            Uint160::from_void(tx.get_initiator().data()),
        );
        let sponsorship = match view.read(sponsorship_keylet) {
            Ok(sponsorship) => sponsorship,
            Err(_) => return Ter::TEF_BAD_LEDGER,
        };
        if let Some(sponsorship) = sponsorship {
            let balance = if sponsorship.is_field_present(sf("sfFeeAmount")) {
                sponsorship
                    .get_field_amount(sf("sfFeeAmount"))
                    .xrp()
                    .drops()
            } else {
                0
            };
            if sponsorship.is_field_present(sf("sfMaxFee")) {
                balance.min(sponsorship.get_field_amount(sf("sfMaxFee")).xrp().drops())
            } else {
                balance
            }
        } else {
            let account = match read_account_result(view, &sponsor) {
                Ok(Some(account)) => account,
                Ok(None) => return Ter::TER_NO_ACCOUNT,
                Err(_) => return Ter::TEF_BAD_LEDGER,
            };
            let balance = account.get_field_amount(sf("sfBalance")).xrp().drops();
            let reserve = ledger::effective_account_reserve(view.fees(), &account, 0, 0) as i64;
            balance.saturating_sub(reserve).max(0)
        }
    } else {
        let payer = tx.get_initiator();
        let account = match read_account_result(view, &payer) {
            Ok(Some(account)) => account,
            Ok(None) => return Ter::TER_NO_ACCOUNT,
            Err(_) => return Ter::TEF_BAD_LEDGER,
        };
        account.get_field_amount(sf("sfBalance")).xrp().drops()
    };

    if max_spendable < paid {
        if max_spendable > 0 && !view.open() {
            Ter::TEC_INSUFF_FEE
        } else {
            Ter::TER_INSUF_FEE_B
        }
    } else {
        Ter::TES_SUCCESS
    }
}

fn sf(name: &str) -> &'static protocol::SField {
    get_field_by_symbol(name)
}

fn account_keylet(account: AccountID) -> protocol::Keylet {
    protocol::account_keylet(Uint160::from_void(account.data()))
}

fn read_account<V: ReadView>(
    view: &V,
    account: &AccountID,
    read_failed: &Cell<bool>,
) -> Option<std::sync::Arc<STLedgerEntry>> {
    match view.read(account_keylet(*account)) {
        Ok(account) => account,
        Err(_) => {
            read_failed.set(true);
            None
        }
    }
}

fn read_account_result<V: ReadView>(
    view: &V,
    account: &AccountID,
) -> Result<Option<std::sync::Arc<STLedgerEntry>>, ledger::ViewError> {
    view.read(account_keylet(*account))
}

fn is_pseudo_account(account: Option<&LedgerSignAccountState>) -> bool {
    account.is_some_and(|account| account.is_pseudo)
}

/// Runs the concrete ledger-backed form of rippled `invokePreclaim`.
///
/// `calculate_base_fee` is deliberately lazy: the shared shell must not
/// calculate a fee until sequence, prior-transaction, permission, and signer
/// checks have all succeeded. `typed_preclaim_tail` is likewise deferred until
/// every generic guard succeeds. A zero account preserves rippled's pseudo
/// transaction exception by skipping the complete generic block.
#[allow(clippy::too_many_arguments)]
#[cfg_attr(not(test), allow(dead_code))] // shared preclaim shell; dormant until a production preclaim caller lands
pub(crate) fn invoke_preclaim<V, CalculateBaseFee, MinimumFee, TypedPreclaimTail>(
    view: &V,
    tx: &STTx,
    current_ledger_seq: u32,
    flags: ApplyFlags,
    calculate_base_fee: CalculateBaseFee,
    minimum_fee: MinimumFee,
    typed_preclaim_tail: TypedPreclaimTail,
) -> Ter
where
    V: ReadView,
    CalculateBaseFee: FnOnce() -> i64,
    MinimumFee: FnOnce(i64) -> i64,
    TypedPreclaimTail: FnOnce() -> Ter,
{
    invoke_preclaim_with_parent_batch_id(
        view,
        tx,
        current_ledger_seq,
        flags,
        None,
        || Ter::TES_SUCCESS,
        || Ok(calculate_base_fee()),
        minimum_fee,
        typed_preclaim_tail,
    )
}

/// Shared `invokePreclaim` with the `parentBatchId` carried by
/// `applySteps.cpp::preclaim` into `PreclaimContext`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn invoke_preclaim_with_parent_batch_id<
    V,
    CalculateBaseFee,
    MinimumFee,
    TypedPreclaimTail,
>(
    view: &V,
    tx: &STTx,
    current_ledger_seq: u32,
    flags: ApplyFlags,
    parent_batch_id: Option<Uint256>,
    check_batch_sign: impl FnOnce() -> NotTec,
    calculate_base_fee: CalculateBaseFee,
    minimum_fee: MinimumFee,
    typed_preclaim_tail: TypedPreclaimTail,
) -> Ter
where
    V: ReadView,
    CalculateBaseFee: FnOnce() -> Result<i64, Ter>,
    MinimumFee: FnOnce(i64) -> i64,
    TypedPreclaimTail: FnOnce() -> Ter,
{
    debug_assert!(
        parent_batch_id.is_none() || (flags & ApplyFlags::BATCH) == ApplyFlags::BATCH,
        "rippled applySteps.cpp::preclaim requires TapBatch whenever parentBatchId is present"
    );

    let adapted = LedgerPreclaimTx { tx };
    let account_is_zero = tx.get_account_id(sf("sfAccount")).is_zero();
    if !account_is_zero && read_account_result(view, &tx.get_account_id(sf("sfAccount"))).is_err() {
        return Ter::TEF_BAD_LEDGER;
    }

    let base_fee_failure = Cell::new(None);
    run_transactor_invoke_preclaim(
        account_is_zero,
        || {
            let read_failed = Cell::new(false);
            let result = run_transactor_check_seq_proxy(
                &adapted,
                |account| {
                    read_account(view, account, &read_failed).map(LedgerSignAccountState::from)
                },
                |account| account.sequence,
                |account, seq_proxy| match view.exists(protocol::ticket_keylet_from_seq_proxy(
                    Uint160::from_void(account.data()),
                    seq_proxy,
                )) {
                    Ok(exists) => exists,
                    Err(_) => {
                        read_failed.set(true);
                        false
                    }
                },
            );
            if read_failed.get() {
                Ter::TEF_BAD_LEDGER
            } else {
                result
            }
        },
        || {
            let read_failed = Cell::new(false);
            let result = run_transactor_check_prior_tx_and_last_ledger(
                current_ledger_seq,
                &adapted,
                |account| read_account(view, account, &read_failed),
                |account| {
                    if account.is_field_present(sf("sfAccountTxnID")) {
                        account.get_field_h256(sf("sfAccountTxnID"))
                    } else {
                        Uint256::zero()
                    }
                },
                |tx_id| match view.tx_exists(*tx_id) {
                    Ok(exists) => exists,
                    Err(_) => {
                        read_failed.set(true);
                        false
                    }
                },
            );
            if read_failed.get() {
                Ter::TEF_BAD_LEDGER
            } else {
                result
            }
        },
        || {
            let sponsor = check_sponsor(view, tx);
            if !protocol::is_tes_success(sponsor) {
                return sponsor;
            }
            typed_check_permission(view, tx, &adapted)
        },
        || {
            let check_standard_sign = || {
                if tx.get_txn_type() == protocol::TxType::LOAN_SET {
                    check_loan_set_counterparty_sign(view, tx, flags, || {
                        check_ledger_signer_authorization(view, &adapted, flags, parent_batch_id)
                    })
                } else {
                    check_ledger_signer_authorization(view, &adapted, flags, parent_batch_id)
                }
            };

            if tx.get_txn_type() == protocol::TxType::BATCH {
                // Parity: ../rippled/src/libxrpl/tx/transactors/system/Batch.cpp::
                // Batch::checkSign calls Transactor::checkSign before
                // Transactor::checkBatchSign, and applySteps.cpp runs both
                // before calculateBaseFee/checkFee.
                run_batch_check_sign(check_standard_sign, check_batch_sign)
            } else {
                check_standard_sign()
            }
        },
        || match calculate_base_fee() {
            Ok(base_fee) => base_fee,
            Err(ter) => {
                base_fee_failure.set(Some(ter));
                0
            }
        },
        |base_fee| {
            if let Some(ter) = base_fee_failure.get() {
                ter
            } else {
                check_fee(view, tx, flags, base_fee, minimum_fee)
            }
        },
        typed_preclaim_tail,
    )
}

#[derive(Clone, Copy)]
struct LedgerPreclaimTx<'a> {
    tx: &'a STTx,
}

pub(crate) fn scale_fee_load(
    fee: i64,
    mut fee_factor: u32,
    remote_fee_factor: u32,
    load_base: u32,
    unlimited: bool,
) -> i64 {
    if fee == 0 {
        return fee;
    }

    if unlimited
        && fee_factor > remote_fee_factor
        && u64::from(fee_factor) < 4_u64.saturating_mul(u64::from(remote_fee_factor))
    {
        fee_factor = remote_fee_factor;
    }

    let denominator = u128::from(load_base);
    if denominator == 0 {
        return i64::MAX;
    }
    let Some(scaled) = u128::try_from(fee)
        .ok()
        .and_then(|fee| fee.checked_mul(u128::from(fee_factor)))
        .map(|numerator| numerator / denominator)
    else {
        return i64::MAX;
    };

    i64::try_from(scaled).unwrap_or(i64::MAX)
}

fn read_delegate_permissions<V: ReadView>(
    view: &V,
    account: AccountID,
    delegate: AccountID,
) -> Result<Option<LedgerDelegatePermissions>, ledger::ViewError> {
    view.read(protocol::delegate_keylet(
        Uint160::from_void(account.data()),
        Uint160::from_void(delegate.data()),
    ))
    .map(|entry| entry.map(LedgerDelegatePermissions::from_entry))
}

fn tx_permission_result(delegate: &LedgerDelegatePermissions, tx: &STTx) -> NotTec {
    run_check_tx_permission(Some(&delegate.permissions), u16::from(tx.get_txn_type()))
}

fn typed_check_permission<V: ReadView>(
    view: &V,
    tx: &STTx,
    adapted: &LedgerPreclaimTx<'_>,
) -> NotTec {
    let account = tx.get_account_id(sf("sfAccount"));
    let delegate = tx
        .is_field_present(sf("sfDelegate"))
        .then(|| tx.get_account_id(sf("sfDelegate")));
    let delegate_permissions = match delegate {
        Some(delegate) => match read_delegate_permissions(view, account, delegate) {
            Ok(permissions) => permissions,
            Err(_) => return Ter::TEF_BAD_LEDGER,
        },
        None => None,
    };

    // Match `Transactor::checkPermission` before transaction-specific
    // `checkGranularSemantics`: transaction-level permission short-circuits;
    // otherwise all held granular permissions for this transaction contribute
    // a union of permitted flags and fields.
    if delegate.is_some() {
        let Some(permissions) = delegate_permissions.as_ref() else {
            return Ter::TER_NO_DELEGATE_PERMISSION;
        };
        if is_tes_success(tx_permission_result(permissions, tx)) {
            return Ter::TES_SUCCESS;
        }
        let registry = protocol::Permission::get_instance();
        let held = permissions
            .permissions
            .iter()
            .filter_map(|value| registry.get_granular_type(*value))
            .filter(|permission| {
                registry.get_granular_tx_type(*permission) == Some(tx.get_txn_type())
            })
            .collect::<Vec<_>>();
        if held.is_empty() || !registry.check_granular_sandbox(tx, held) {
            return Ter::TER_NO_DELEGATE_PERMISSION;
        }
    }

    match tx.get_txn_type() {
        protocol::TxType::ACCOUNT_SET => tx::run_account_set_check_permission(
            adapted,
            |_account, _delegate| delegate_permissions.clone(),
            |permissions, permission| permissions.permissions.contains(&(permission as u32)),
        ),
        protocol::TxType::PAYMENT => {
            let amount = tx.get_field_amount(sf("sfAmount"));
            let asset = amount.asset();
            let issuer = asset.issuer();
            let destination = tx.get_account_id(sf("sfDestination"));
            // Payment::checkGranularSemantics determines IOU mint/burn from
            // the source/destination trust-line orientation, not from the
            // sfAmount issuer alias alone.  In particular, keylet::trustLine
            // is built from both endpoints even when sfAmount names the
            // source as issuer.
            let (trustline_exists, account_is_holder, dest_limit_positive) = if delegate.is_some() {
                match asset {
                    protocol::Asset::Issue(issue)
                        if !issue.native()
                            && (issue.account == account || issue.account == destination) =>
                    {
                        let line =
                            match view.read(protocol::line(account, destination, issue.currency)) {
                                Ok(line) => line,
                                Err(_) => return Ter::TEF_BAD_LEDGER,
                            };
                        match line {
                            Some(line) => {
                                let account_is_low = account < destination;
                                let dest_limit = line.get_field_amount(if account_is_low {
                                    sf("sfHighLimit")
                                } else {
                                    sf("sfLowLimit")
                                });
                                let raw_balance = line.get_field_amount(sf("sfBalance"));
                                (
                                    true,
                                    Some(if account_is_low {
                                        raw_balance.signum() > 0
                                    } else {
                                        raw_balance.signum() < 0
                                    }),
                                    Some(dest_limit.signum() > 0),
                                )
                            }
                            None => (false, None, None),
                        }
                    }
                    _ => (false, None, None),
                }
            } else {
                (false, None, None)
            };
            tx::run_payment_check_permission(tx::PaymentCheckPermissionFacts {
                delegate_present: delegate.is_some(),
                delegate_entry_exists: delegate_permissions.is_some(),
                check_tx_permission_result: delegate_permissions
                    .as_ref()
                    .map_or(Ter::TER_NO_DELEGATE_PERMISSION, |permissions| {
                        tx_permission_result(permissions, tx)
                    }),
                send_max_present: tx.is_field_present(sf("sfSendMax")),
                send_max_asset_matches_amount: !tx.is_field_present(sf("sfSendMax"))
                    || tx.get_field_amount(sf("sfSendMax")).asset() == asset,
                paths_present: tx.is_field_present(sf("sfPaths")),
                payment_mint_permission: delegate_permissions.as_ref().is_some_and(|permissions| {
                    permissions
                        .permissions
                        .contains(&(protocol::GranularPermissionType::PaymentMint as u32))
                }),
                payment_burn_permission: delegate_permissions.as_ref().is_some_and(|permissions| {
                    permissions
                        .permissions
                        .contains(&(protocol::GranularPermissionType::PaymentBurn as u32))
                }),
                amount_is_xrp: amount.native(),
                is_mpt: matches!(asset, protocol::Asset::MPTIssue(_)),
                amount_issuer_is_source: !amount.native() && issuer == account,
                amount_issuer_is_destination: !amount.native() && issuer == destination,
                trustline_exists,
                account_is_holder,
                dest_limit_positive,
            })
        }
        protocol::TxType::TRUST_SET => {
            let limit = tx.get_field_amount(sf("sfLimitAmount"));
            let issuer = limit.issue().account;
            let trustline = match view.read(protocol::line(account, issuer, limit.issue().currency))
            {
                Ok(line) => line,
                Err(_) => return Ter::TEF_BAD_LEDGER,
            };
            let current_limit_equals_proposed_limit = trustline.as_ref().is_some_and(|line| {
                let current = line.get_field_amount(if account > issuer {
                    sf("sfHighLimit")
                } else {
                    sf("sfLowLimit")
                });
                let mut proposed = limit.clone();
                proposed.set_issuer(account);
                current == proposed
            });
            tx::run_trust_set_check_permission(tx::TrustSetCheckPermissionFacts {
                delegate_present: delegate.is_some(),
                delegate_entry_exists: delegate_permissions.is_some(),
                check_tx_permission_result: delegate_permissions
                    .as_ref()
                    .map_or(Ter::TER_NO_DELEGATE_PERMISSION, |permissions| {
                        tx_permission_result(permissions, tx)
                    }),
                tx_flags: tx.get_flags(),
                quality_in_present: tx.is_field_present(sf("sfQualityIn")),
                quality_out_present: tx.is_field_present(sf("sfQualityOut")),
                trustline_exists: trustline.is_some(),
                granular_trustline_authorize: delegate_permissions.as_ref().is_some_and(
                    |permissions| {
                        permissions.permissions.contains(
                            &(protocol::GranularPermissionType::TrustlineAuthorize as u32),
                        )
                    },
                ),
                granular_trustline_freeze: delegate_permissions.as_ref().is_some_and(
                    |permissions| {
                        permissions
                            .permissions
                            .contains(&(protocol::GranularPermissionType::TrustlineFreeze as u32))
                    },
                ),
                granular_trustline_unfreeze: delegate_permissions.as_ref().is_some_and(
                    |permissions| {
                        permissions
                            .permissions
                            .contains(&(protocol::GranularPermissionType::TrustlineUnfreeze as u32))
                    },
                ),
                current_limit_equals_proposed_limit,
            })
        }
        protocol::TxType::MPTOKEN_ISSUANCE_SET => {
            let mut granular_permissions = std::collections::BTreeSet::new();
            if delegate_permissions.as_ref().is_some_and(|permissions| {
                permissions
                    .permissions
                    .contains(&(protocol::GranularPermissionType::MPTokenIssuanceLock as u32))
            }) {
                granular_permissions.insert(tx::MPTokenIssuanceSetGranularPermission::Lock);
            }
            if delegate_permissions.as_ref().is_some_and(|permissions| {
                permissions
                    .permissions
                    .contains(&(protocol::GranularPermissionType::MPTokenIssuanceUnlock as u32))
            }) {
                granular_permissions.insert(tx::MPTokenIssuanceSetGranularPermission::Unlock);
            }
            tx::run_mp_token_issuance_set_check_permission(tx::MPTokenIssuanceSetPermissionFacts {
                delegate_present: delegate.is_some(),
                delegate_entry_exists: delegate_permissions.is_some(),
                broad_permission_granted: delegate_permissions.as_ref().is_some_and(
                    |permissions| is_tes_success(tx_permission_result(permissions, tx)),
                ),
                tx_flags: tx.get_flags(),
                granular_permissions,
            })
        }
        _ => run_transactor_check_permission(
            adapted,
            |_account, _delegate| delegate_permissions.clone(),
            |permissions, delegated_tx| tx_permission_result(&permissions, delegated_tx.tx),
        ),
    }
}

impl tx::AccountSetPermissionTx for LedgerPreclaimTx<'_> {
    type AccountId = AccountID;

    fn account_id(&self) -> Self::AccountId {
        self.tx.get_account_id(sf("sfAccount"))
    }

    fn delegate(&self) -> Option<Self::AccountId> {
        self.tx
            .is_field_present(sf("sfDelegate"))
            .then(|| self.tx.get_account_id(sf("sfDelegate")))
    }

    fn set_flag(&self) -> u32 {
        self.tx.get_field_u32(sf("sfSetFlag"))
    }

    fn clear_flag(&self) -> u32 {
        self.tx.get_field_u32(sf("sfClearFlag"))
    }

    fn flags(&self) -> u32 {
        self.tx.get_flags()
    }

    fn email_hash_present(&self) -> bool {
        self.tx.is_field_present(sf("sfEmailHash"))
    }

    fn wallet_locator_present(&self) -> bool {
        self.tx.is_field_present(sf("sfWalletLocator"))
    }

    fn nftoken_minter_present(&self) -> bool {
        self.tx.is_field_present(sf("sfNFTokenMinter"))
    }

    fn message_key_present(&self) -> bool {
        self.tx.is_field_present(sf("sfMessageKey"))
    }

    fn domain_present(&self) -> bool {
        self.tx.is_field_present(sf("sfDomain"))
    }

    fn transfer_rate_present(&self) -> bool {
        self.tx.is_field_present(sf("sfTransferRate"))
    }

    fn tick_size_present(&self) -> bool {
        self.tx.is_field_present(sf("sfTickSize"))
    }
}

impl TransactorCheckSeqProxyTx for LedgerPreclaimTx<'_> {
    type AccountId = AccountID;

    fn account_id(&self) -> Self::AccountId {
        self.tx.get_account_id(sf("sfAccount"))
    }

    fn seq_proxy(&self) -> protocol::SeqProxy {
        self.tx.get_seq_proxy()
    }

    fn ticket_sequence_present(&self) -> bool {
        self.tx.is_field_present(sf("sfTicketSequence"))
    }
}

impl TransactorCheckPriorTxAndLastLedgerTx for LedgerPreclaimTx<'_> {
    type AccountId = AccountID;
    type TxId = Uint256;

    fn account_id(&self) -> Self::AccountId {
        self.tx.get_account_id(sf("sfAccount"))
    }

    fn account_txn_id(&self) -> Option<Self::TxId> {
        self.tx
            .is_field_present(sf("sfAccountTxnID"))
            .then(|| self.tx.get_field_h256(sf("sfAccountTxnID")))
    }

    fn last_ledger_sequence(&self) -> Option<u32> {
        self.tx
            .is_field_present(sf("sfLastLedgerSequence"))
            .then(|| self.tx.get_field_u32(sf("sfLastLedgerSequence")))
    }

    fn transaction_id(&self) -> Self::TxId {
        self.tx.get_transaction_id()
    }
}

impl TransactorCheckPermissionTx for LedgerPreclaimTx<'_> {
    type AccountId = AccountID;

    fn account_id(&self) -> Self::AccountId {
        self.tx.get_account_id(sf("sfAccount"))
    }

    fn delegate(&self) -> Option<Self::AccountId> {
        self.tx
            .is_field_present(sf("sfDelegate"))
            .then(|| self.tx.get_account_id(sf("sfDelegate")))
    }
}

impl TransactorCheckFeeTx for LedgerPreclaimTx<'_> {
    type AccountId = AccountID;
    type Amount = i64;

    fn fee_is_native(&self) -> bool {
        !self.tx.is_field_present(sf("sfFee")) || self.tx.get_field_amount(sf("sfFee")).native()
    }

    fn fee_paid(&self) -> Self::Amount {
        if self.tx.is_field_present(sf("sfFee")) {
            self.tx.get_field_amount(sf("sfFee")).xrp().drops()
        } else {
            0
        }
    }

    fn fee_payer(&self) -> Self::AccountId {
        self.tx.get_fee_payer_id()
    }
}

impl TransactorSignTx for LedgerPreclaimTx<'_> {
    type AccountId = AccountID;

    fn has_delegate(&self) -> bool {
        self.tx.is_field_present(sf("sfDelegate"))
    }

    fn delegate_account_id(&self) -> Self::AccountId {
        self.tx.get_account_id(sf("sfDelegate"))
    }

    fn account_id(&self) -> Self::AccountId {
        self.tx.get_account_id(sf("sfAccount"))
    }
}

#[derive(Clone)]
struct LedgerDelegatePermissions {
    permissions: Vec<u32>,
}

impl LedgerDelegatePermissions {
    fn from_entry(entry: std::sync::Arc<STLedgerEntry>) -> Self {
        Self {
            permissions: entry
                .get_field_array(sf("sfPermissions"))
                .iter()
                .map(|permission| permission.get_field_u32(sf("sfPermissionValue")))
                .collect(),
        }
    }
}

#[derive(Clone)]
struct LedgerSignAccountState {
    sequence: u32,
    balance: i64,
    regular_key: Option<AccountID>,
    master_disabled: bool,
    is_pseudo: bool,
}

impl From<std::sync::Arc<STLedgerEntry>> for LedgerSignAccountState {
    fn from(entry: std::sync::Arc<STLedgerEntry>) -> Self {
        let regular_key = entry
            .is_field_present(sf("sfRegularKey"))
            .then(|| entry.get_account_id(sf("sfRegularKey")));
        Self {
            sequence: entry.get_field_u32(sf("sfSequence")),
            balance: entry.get_field_amount(sf("sfBalance")).xrp().drops(),
            regular_key,
            master_disabled: entry.get_field_u32(sf("sfFlags")) & lsfDisableMaster != 0,
            is_pseudo: ["sfAMMID", "sfVaultID", "sfLoanBrokerID"]
                .iter()
                .any(|field| entry.is_field_present(sf(field))),
        }
    }
}

impl TransactorSingleSignAccountState<AccountID> for LedgerSignAccountState {
    fn regular_key(&self) -> Option<&AccountID> {
        self.regular_key.as_ref()
    }

    fn is_master_disabled(&self) -> bool {
        self.master_disabled
    }
}

#[derive(Clone)]
struct LedgerAccountSigner {
    account: AccountID,
    weight: u32,
}

impl TransactorMultiSignAccountSigner<AccountID> for LedgerAccountSigner {
    fn account_id(&self) -> &AccountID {
        &self.account
    }

    fn weight(&self) -> u32 {
        self.weight
    }
}

#[derive(Clone)]
struct LedgerSignerList {
    signer_list_id_present: bool,
    signer_list_id: u32,
    quorum: u32,
    entries: Vec<LedgerAccountSigner>,
}

impl From<std::sync::Arc<STLedgerEntry>> for LedgerSignerList {
    fn from(entry: std::sync::Arc<STLedgerEntry>) -> Self {
        Self {
            signer_list_id_present: entry.is_field_present(sf("sfSignerListID")),
            signer_list_id: entry.get_field_u32(sf("sfSignerListID")),
            quorum: entry.get_field_u32(sf("sfSignerQuorum")),
            entries: entry
                .get_field_array(sf("sfSignerEntries"))
                .iter()
                .map(|signer| LedgerAccountSigner {
                    account: signer.get_account_id(sf("sfAccount")),
                    weight: u32::from(signer.get_field_u16(sf("sfSignerWeight"))),
                })
                .collect(),
        }
    }
}

impl TransactorMultiSignSignerList<LedgerAccountSigner> for LedgerSignerList {
    type Entries = Vec<LedgerAccountSigner>;

    fn signer_list_id_present(&self) -> bool {
        self.signer_list_id_present
    }

    fn signer_list_id(&self) -> u32 {
        self.signer_list_id
    }

    fn signer_quorum(&self) -> u32 {
        self.quorum
    }

    fn signer_entries(self) -> Result<Self::Entries, NotTec> {
        Ok(self.entries)
    }
}

#[derive(Clone)]
struct LedgerTxSigner {
    object: STObject,
}

impl TransactorMultiSignTxSigner<AccountID> for LedgerTxSigner {
    fn account_id(&self) -> AccountID {
        self.object.get_account_id(sf("sfAccount"))
    }

    fn signing_pub_key_is_empty(&self) -> bool {
        self.object.get_field_vl(sf("sfSigningPubKey")).is_empty()
    }
}

struct LedgerSignatureObject<'a> {
    tx: &'a STTx,
}

impl TransactorSignObject for LedgerSignatureObject<'_> {
    fn signing_pub_key_is_empty(&self) -> bool {
        self.tx.get_field_vl(sf("sfSigningPubKey")).is_empty()
    }

    fn has_signers(&self) -> bool {
        self.tx.is_field_present(sf("sfSigners"))
    }

    fn has_txn_signature(&self) -> bool {
        self.tx.is_field_present(sf("sfTxnSignature"))
    }
}

impl TransactorSignMultiSignObject<AccountID> for LedgerSignatureObject<'_> {
    type TxSigner = LedgerTxSigner;
    type TxSigners = Vec<LedgerTxSigner>;

    fn tx_signers(&self) -> Self::TxSigners {
        self.tx
            .get_field_array(sf("sfSigners"))
            .iter()
            .cloned()
            .map(|object| LedgerTxSigner { object })
            .collect()
    }
}

struct LedgerLoanSetSignTx<'a> {
    tx: &'a STTx,
    counterparty_signature: STObject,
}

impl tx::LoanSetSignTx for LedgerLoanSetSignTx<'_> {
    type AccountId = AccountID;
    type CounterpartySignature = STObject;

    fn counterparty(&self) -> Option<Self::AccountId> {
        self.tx
            .is_field_present(sf("sfCounterparty"))
            .then(|| self.tx.get_account_id(sf("sfCounterparty")))
    }

    fn has_counterparty_signature(&self) -> bool {
        self.tx.is_field_present(sf("sfCounterpartySignature"))
    }

    fn counterparty_signature(&self) -> &Self::CounterpartySignature {
        &self.counterparty_signature
    }
}

struct LedgerCounterpartySignTx {
    account: AccountID,
}

impl TransactorSignTx for LedgerCounterpartySignTx {
    type AccountId = AccountID;

    fn has_delegate(&self) -> bool {
        false
    }

    fn delegate_account_id(&self) -> Self::AccountId {
        self.account
    }

    fn account_id(&self) -> Self::AccountId {
        self.account
    }
}

struct LedgerCounterpartySignatureObject<'a> {
    object: &'a STObject,
}

impl TransactorSignObject for LedgerCounterpartySignatureObject<'_> {
    fn signing_pub_key_is_empty(&self) -> bool {
        self.object.get_field_vl(sf("sfSigningPubKey")).is_empty()
    }

    fn has_signers(&self) -> bool {
        self.object.is_field_present(sf("sfSigners"))
    }

    fn has_txn_signature(&self) -> bool {
        self.object.is_field_present(sf("sfTxnSignature"))
    }
}

impl TransactorSignMultiSignObject<AccountID> for LedgerCounterpartySignatureObject<'_> {
    type TxSigner = LedgerTxSigner;
    type TxSigners = Vec<LedgerTxSigner>;

    fn tx_signers(&self) -> Self::TxSigners {
        self.object
            .get_field_array(sf("sfSigners"))
            .iter()
            .cloned()
            .map(|object| LedgerTxSigner { object })
            .collect()
    }
}

/// `../rippled/src/libxrpl/tx/transactors/lending/LoanSet.cpp::LoanSet::checkSign`: runs the `CounterpartySignature` authorization part.
fn check_loan_set_counterparty_sign<V: ReadView>(
    view: &V,
    tx: &STTx,
    flags: ApplyFlags,
    check_primary_sign: impl FnOnce() -> NotTec,
) -> NotTec {
    let adapted = LedgerLoanSetSignTx {
        tx,
        counterparty_signature: tx.get_field_object(sf("sfCounterpartySignature")),
    };
    let read_failed = Cell::new(false);
    let result = tx::run_loan_set_check_sign(
        &adapted,
        || match view.read(protocol::loan_broker_keylet_from_key(
            tx.get_field_h256(sf("sfLoanBrokerID")),
        )) {
            Ok(broker) => broker.map(|broker| broker.get_account_id(sf("sfOwner"))),
            Err(_) => {
                read_failed.set(true);
                None
            }
        },
        check_primary_sign,
        |counter_signer, signature| {
            check_ledger_counterparty_signer_authorization(view, counter_signer, signature, flags)
        },
    );
    if read_failed.get() {
        Ter::TEF_BAD_LEDGER
    } else {
        result
    }
}

fn check_ledger_counterparty_signer_authorization<V: ReadView>(
    view: &V,
    counter_signer: AccountID,
    signature: &STObject,
    flags: ApplyFlags,
) -> NotTec {
    let signer_tx = LedgerCounterpartySignTx {
        account: counter_signer,
    };
    let signature = LedgerCounterpartySignatureObject { object: signature };
    let read_failed = Cell::new(false);
    let result = tx::run_transactor_preclaim_check_sign(
        flags,
        false,
        view.rules().enabled(&feature_batch()),
        view.rules().enabled(&feature_lending_protocol()),
        &signer_tx,
        &signature,
        |account| read_account(view, account, &read_failed).map(LedgerSignAccountState::from),
        |account| match view.read(protocol::signers_keylet(Uint160::from_void(account.data()))) {
            Ok(signers) => signers.map(LedgerSignerList::from),
            Err(_) => {
                read_failed.set(true);
                None
            }
        },
        is_pseudo_account,
        |signature| {
            PublicKey::from_slice(&signature.object.get_field_vl(sf("sfSigningPubKey"))).is_ok()
        },
        |signature| {
            let key = PublicKey::from_slice(&signature.object.get_field_vl(sf("sfSigningPubKey")))
                .expect("public-key type was checked before account derivation");
            calc_account_id(key.as_bytes())
        },
        |signer| PublicKey::from_slice(&signer.object.get_field_vl(sf("sfSigningPubKey"))).is_ok(),
        |signer| {
            let key = PublicKey::from_slice(&signer.object.get_field_vl(sf("sfSigningPubKey")))
                .expect("public-key type was checked before account derivation");
            calc_account_id(key.as_bytes())
        },
    );
    if read_failed.get() {
        Ter::TEF_BAD_LEDGER
    } else {
        result
    }
}

fn check_ledger_signer_authorization<V: ReadView>(
    view: &V,
    tx: &LedgerPreclaimTx<'_>,
    flags: ApplyFlags,
    parent_batch_id: Option<Uint256>,
) -> NotTec {
    // The shared helper selects sfDelegate over sfAccount for delegated
    // transactions and the regular account otherwise, exactly like
    // Transactor::checkSign(PreclaimContext). `Batch::checkSign` then runs
    // its BatchSigners tail before `invokePreclaim` reaches checkFee.
    let signature = LedgerSignatureObject { tx: tx.tx };
    let read_failed = Cell::new(false);
    let result = run_transactor_preclaim_check_sign(
        flags,
        parent_batch_id.is_some(),
        view.rules().enabled(&feature_batch()),
        view.rules().enabled(&feature_lending_protocol()),
        tx,
        &signature,
        |account| read_account(view, account, &read_failed).map(LedgerSignAccountState::from),
        |account| match view.read(protocol::signers_keylet(Uint160::from_void(account.data()))) {
            Ok(signers) => signers.map(LedgerSignerList::from),
            Err(_) => {
                read_failed.set(true);
                None
            }
        },
        is_pseudo_account,
        |signature| {
            PublicKey::from_slice(&signature.tx.get_field_vl(sf("sfSigningPubKey"))).is_ok()
        },
        |signature| {
            let key = PublicKey::from_slice(&signature.tx.get_field_vl(sf("sfSigningPubKey")))
                .expect("public-key type was checked before account derivation");
            calc_account_id(key.as_bytes())
        },
        |signer| PublicKey::from_slice(&signer.object.get_field_vl(sf("sfSigningPubKey"))).is_ok(),
        |signer| {
            let key = PublicKey::from_slice(&signer.object.get_field_vl(sf("sfSigningPubKey")))
                .expect("public-key type was checked before account derivation");
            calc_account_id(key.as_bytes())
        },
    );
    if read_failed.get() {
        Ter::TEF_BAD_LEDGER
    } else {
        result
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use basics::base_uint::{Uint160, Uint256};
    use ledger::{ApplyView, Ledger, ReadView, ReadViewTx, ViewError};
    use protocol::{
        AccountID, ApplyFlags, BatchTransactionFlags, Currency, INNER_BATCH_TRANSACTION_FLAG,
        IOUAmount, Issue, LedgerEntryType, STAmount, STArray, STLedgerEntry, STObject, STTx,
        StBase, Ter, TxType, get_field_by_symbol, sf_generic,
    };

    use super::{
        LedgerPreclaimTx, check_fee, check_sponsor, invoke_preclaim, scale_fee_load,
        typed_check_permission,
    };
    use tx::TransactorCheckFeeTx;

    #[derive(Debug)]
    struct FaultReadView {
        base: Arc<Ledger>,
    }

    impl ReadView for FaultReadView {
        fn open(&self) -> bool {
            ReadView::open(self.base.as_ref())
        }
        fn header(&self) -> ledger::LedgerHeader {
            ReadView::header(self.base.as_ref())
        }
        fn fees(&self) -> ledger::Fees {
            ReadView::fees(self.base.as_ref())
        }
        fn rules(&self) -> protocol::Rules {
            ReadView::rules(self.base.as_ref())
        }
        fn exists(&self, key: protocol::Keylet) -> Result<bool, ViewError> {
            ReadView::exists(self.base.as_ref(), key)
        }
        fn succ(&self, key: Uint256, last: Option<Uint256>) -> Result<Option<Uint256>, ViewError> {
            ReadView::succ(self.base.as_ref(), key, last)
        }
        fn read(&self, _key: protocol::Keylet) -> Result<Option<Arc<STLedgerEntry>>, ViewError> {
            Err(ViewError::Conversion("injected read failure".into()))
        }
        fn sles(&self) -> Result<Vec<Arc<STLedgerEntry>>, ViewError> {
            ReadView::sles(self.base.as_ref())
        }
        fn tx_exists(&self, key: Uint256) -> Result<bool, ViewError> {
            ReadView::tx_exists(self.base.as_ref(), key)
        }
        fn tx_read(&self, key: Uint256) -> Result<Option<ReadViewTx>, ViewError> {
            ReadView::tx_read(self.base.as_ref(), key)
        }
        fn txs(&self) -> Result<Vec<ReadViewTx>, ViewError> {
            ReadView::txs(self.base.as_ref())
        }
    }

    #[test]
    fn shared_sponsor_fee_and_preclaim_reads_fail_hard() {
        let account = AccountID::from_array([0xE1; 20]);
        let sponsor = AccountID::from_array([0xE2; 20]);
        let tx = STTx::new(TxType::PAYMENT, |tx| {
            tx.set_account_id(get_field_by_symbol("sfAccount"), account);
            tx.set_account_id(get_field_by_symbol("sfSponsor"), sponsor);
            tx.set_field_u32(get_field_by_symbol("sfSponsorFlags"), 1);
            tx.set_field_amount(
                get_field_by_symbol("sfFee"),
                STAmount::new_native(10, false),
            );
        });
        let view = FaultReadView {
            base: Arc::new(Ledger::from_ledger_seq_and_close_time(1, 0, false)),
        };
        assert_eq!(check_sponsor(&view, &tx), Ter::TEF_BAD_LEDGER);
        assert_eq!(
            check_fee(&view, &tx, ApplyFlags::NONE, 10, |fee| fee),
            Ter::TEF_BAD_LEDGER
        );
        assert_eq!(
            invoke_preclaim(
                &view,
                &tx,
                1,
                ApplyFlags::NONE,
                || 10,
                |fee| fee,
                || Ter::TES_SUCCESS,
            ),
            Ter::TEF_BAD_LEDGER
        );
    }

    #[test]
    fn specialized_and_recursive_batch_base_fee_reads_fail_hard() {
        let account = AccountID::from_array([0xE3; 20]);
        let direct = STTx::new(TxType::REGULAR_KEY_SET, |tx| {
            tx.set_account_id(get_field_by_symbol("sfAccount"), account);
            tx.set_field_amount(
                get_field_by_symbol("sfFee"),
                STAmount::new_native(10, false),
            );
            tx.set_field_u32(get_field_by_symbol("sfSequence"), 1);
        });
        let view = FaultReadView {
            base: Arc::new(Ledger::from_ledger_seq_and_close_time(1, 0, false)),
        };
        assert_eq!(
            crate::state::application_root::calculate_sttx_base_fee(&view, &direct),
            Err(Ter::TEF_BAD_LEDGER)
        );

        let inner = STTx::new(TxType::REGULAR_KEY_SET, |tx| {
            tx.set_account_id(get_field_by_symbol("sfAccount"), account);
            tx.set_field_amount(get_field_by_symbol("sfFee"), STAmount::new_native(0, false));
            tx.set_field_u32(get_field_by_symbol("sfSequence"), 2);
            tx.set_field_u32(get_field_by_symbol("sfFlags"), INNER_BATCH_TRANSACTION_FLAG);
            tx.set_field_vl(get_field_by_symbol("sfSigningPubKey"), &[]);
        });
        let mut raw = STArray::new(get_field_by_symbol("sfRawTransactions"));
        let mut raw_inner = inner.clone_as_object();
        raw_inner.set_fname(get_field_by_symbol("sfRawTransaction"));
        raw.push_back(raw_inner);
        let batch = STTx::new(TxType::BATCH, |tx| {
            tx.set_account_id(get_field_by_symbol("sfAccount"), account);
            tx.set_field_amount(
                get_field_by_symbol("sfFee"),
                STAmount::new_native(10, false),
            );
            tx.set_field_u32(get_field_by_symbol("sfSequence"), 1);
            tx.set_field_u32(
                get_field_by_symbol("sfFlags"),
                BatchTransactionFlags::ALL_OR_NOTHING.bits(),
            );
            tx.set_field_array(get_field_by_symbol("sfRawTransactions"), raw);
        });
        assert_eq!(
            crate::state::application_root::calculate_sttx_base_fee(&view, &batch),
            Err(Ter::TEF_BAD_LEDGER)
        );
    }

    fn insert_delegate(
        view: &mut ledger::Sandbox<ledger::Ledger>,
        account: AccountID,
        delegate: AccountID,
        permissions: &[u32],
    ) {
        let keylet = protocol::delegate_keylet(
            Uint160::from_void(account.data()),
            Uint160::from_void(delegate.data()),
        );
        let mut permission_entries = STArray::new(get_field_by_symbol("sfPermissions"));
        for permission in permissions {
            let mut entry = STObject::make_inner_object(get_field_by_symbol("sfPermission"));
            entry.set_field_u32(get_field_by_symbol("sfPermissionValue"), *permission);
            permission_entries.push_back(entry);
        }
        let mut entry = STLedgerEntry::from_type_and_key(LedgerEntryType::Delegate, keylet.key);
        entry.set_account_id(get_field_by_symbol("sfAccount"), account);
        entry.set_account_id(get_field_by_symbol("sfAuthorize"), delegate);
        entry.set_field_array(get_field_by_symbol("sfPermissions"), permission_entries);
        view.insert(Arc::new(entry))
            .expect("delegate entry should insert into the preclaim view");
    }

    fn insert_account(
        view: &mut ledger::Sandbox<ledger::Ledger>,
        account: AccountID,
        balance: i64,
        owner_count: u32,
    ) {
        let keylet = protocol::account_keylet(Uint160::from_void(account.data()));
        let mut entry = STLedgerEntry::from_type_and_key(LedgerEntryType::AccountRoot, keylet.key);
        entry.set_account_id(get_field_by_symbol("sfAccount"), account);
        entry.set_field_u32(get_field_by_symbol("sfSequence"), 1);
        entry.set_field_u32(get_field_by_symbol("sfOwnerCount"), owner_count);
        entry.set_field_amount(
            get_field_by_symbol("sfBalance"),
            STAmount::new_native(balance.try_into().expect("nonnegative balance"), false),
        );
        view.insert(Arc::new(entry)).expect("account insert");
    }

    fn sponsored_payment(source: AccountID, sponsor: AccountID, fee: i64, co_signed: bool) -> STTx {
        STTx::new(TxType::PAYMENT, |tx| {
            tx.set_account_id(get_field_by_symbol("sfAccount"), source);
            tx.set_account_id(
                get_field_by_symbol("sfDestination"),
                AccountID::from_array([0xEE; 20]),
            );
            tx.set_field_u32(get_field_by_symbol("sfSequence"), 1);
            tx.set_field_amount(
                get_field_by_symbol("sfFee"),
                STAmount::new_native(fee.try_into().expect("nonnegative fee"), false),
            );
            tx.set_field_amount(
                get_field_by_symbol("sfAmount"),
                STAmount::new_native(1, false),
            );
            tx.set_account_id(get_field_by_symbol("sfSponsor"), sponsor);
            tx.set_field_u32(get_field_by_symbol("sfSponsorFlags"), 1);
            if co_signed {
                tx.set_field_object(
                    get_field_by_symbol("sfSponsorSignature"),
                    STObject::make_inner_object(get_field_by_symbol("sfSponsorSignature")),
                );
            }
        })
    }

    #[test]
    fn sponsor_preclaim_uses_prefunded_keylet_cap_and_cosigned_reserve() {
        let source = AccountID::from_array([0x11; 20]);
        let sponsor = AccountID::from_array([0x22; 20]);
        let mut view = ledger::Sandbox::new(
            Arc::new(Ledger::from_ledger_seq_and_close_time(1, 0, false)),
            ApplyFlags::NONE,
        );
        insert_account(&mut view, source, 1_000_000_000, 0);
        insert_account(&mut view, sponsor, 1_000_000_000, 0);

        let keylet = protocol::sponsorship_keylet(
            Uint160::from_void(sponsor.data()),
            Uint160::from_void(source.data()),
        );
        let mut sponsorship =
            STLedgerEntry::from_type_and_key(LedgerEntryType::Sponsorship, keylet.key);
        sponsorship.set_field_amount(
            get_field_by_symbol("sfFeeAmount"),
            STAmount::new_native(50, false),
        );
        sponsorship.set_field_amount(
            get_field_by_symbol("sfMaxFee"),
            STAmount::new_native(20, false),
        );
        sponsorship.set_field_u32(get_field_by_symbol("sfFlags"), 0);
        view.insert(Arc::new(sponsorship))
            .expect("sponsorship insert");

        let capped = sponsored_payment(source, sponsor, 25, false);
        assert_eq!(check_sponsor(&view, &capped), Ter::TES_SUCCESS);
        assert_eq!(
            check_fee(&view, &capped, ApplyFlags::NONE, 10, |fee| fee),
            Ter::TEC_INSUFF_FEE
        );
        let within_cap = sponsored_payment(source, sponsor, 15, false);
        assert_eq!(
            check_fee(&view, &within_cap, ApplyFlags::NONE, 10, |fee| fee),
            Ter::TES_SUCCESS
        );

        // Removing the prefund selects the co-signed AccountRoot route. Its
        // reserve, not its full balance, is spendable.
        let prefund = view.peek(keylet).expect("peek").expect("sponsorship");
        view.erase(prefund).expect("erase sponsorship");
        let reserve = view.fees().account_reserve(0) as i64;
        let sponsor_keylet = protocol::account_keylet(Uint160::from_void(sponsor.data()));
        let sponsor_root = view.peek(sponsor_keylet).expect("peek").expect("sponsor");
        let mut sponsor_root =
            STLedgerEntry::from_stobject(sponsor_root.clone_as_object(), *sponsor_root.key());
        sponsor_root.set_field_amount(
            get_field_by_symbol("sfBalance"),
            STAmount::new_native(
                (reserve + 10).try_into().expect("nonnegative reserve"),
                false,
            ),
        );
        view.update(Arc::new(sponsor_root)).expect("update sponsor");
        let cosigned = sponsored_payment(source, sponsor, 11, true);
        assert_eq!(check_sponsor(&view, &cosigned), Ter::TES_SUCCESS);
        assert_eq!(
            check_fee(&view, &cosigned, ApplyFlags::NONE, 10, |fee| fee),
            Ter::TEC_INSUFF_FEE
        );
    }

    #[test]
    fn shared_preclaim_dispatches_account_payment_trust_and_mpt_permissions_polymorphically() {
        // ../rippled/src/libxrpl/tx/transactors/account/AccountSet.cpp::AccountSet::checkPermission (lines 173-217); ../rippled/src/libxrpl/tx/transactors/payment/Payment.cpp::Payment::checkPermission (lines 276-312); ../rippled/src/libxrpl/tx/transactors/token/TrustSet.cpp::TrustSet::checkPermission (lines 128-184); ../rippled/src/libxrpl/tx/transactors/token/MPTokenIssuanceSet.cpp::MPTokenIssuanceSet::checkPermission (lines 143-172).
        let account = AccountID::from_array([0xA1; 20]);
        let broad_delegate = AccountID::from_array([0xA2; 20]);
        let granular_delegate = AccountID::from_array([0xA3; 20]);
        let mut view = ledger::Sandbox::new(
            Arc::new(ledger::Ledger::from_ledger_seq_and_close_time(1, 0, false)),
            ApplyFlags::NONE,
        );
        insert_delegate(
            &mut view,
            account,
            broad_delegate,
            &[
                protocol::Permission::tx_to_permission_type(TxType::ACCOUNT_SET),
                protocol::Permission::tx_to_permission_type(TxType::PAYMENT),
                protocol::Permission::tx_to_permission_type(TxType::TRUST_SET),
            ],
        );
        insert_delegate(
            &mut view,
            account,
            granular_delegate,
            &[protocol::GranularPermissionType::MPTokenIssuanceLock as u32],
        );

        let account_set = STTx::new(TxType::ACCOUNT_SET, |tx| {
            tx.set_account_id(get_field_by_symbol("sfAccount"), account);
            tx.set_account_id(get_field_by_symbol("sfDelegate"), broad_delegate);
            tx.set_field_vl(get_field_by_symbol("sfDomain"), b"example.org");
        });
        let payment = STTx::new(TxType::PAYMENT, |tx| {
            tx.set_account_id(get_field_by_symbol("sfAccount"), account);
            tx.set_account_id(get_field_by_symbol("sfDelegate"), broad_delegate);
            tx.set_account_id(
                get_field_by_symbol("sfDestination"),
                AccountID::from_array([0xB1; 20]),
            );
            tx.set_field_amount(
                get_field_by_symbol("sfAmount"),
                STAmount::new_native(1, false),
            );
        });
        let trust_set = STTx::new(TxType::TRUST_SET, |tx| {
            tx.set_account_id(get_field_by_symbol("sfAccount"), account);
            tx.set_account_id(get_field_by_symbol("sfDelegate"), broad_delegate);
            tx.set_field_amount(
                get_field_by_symbol("sfLimitAmount"),
                STAmount::new_native(1, false),
            );
        });
        let mpt_set = STTx::new(TxType::MPTOKEN_ISSUANCE_SET, |tx| {
            tx.set_account_id(get_field_by_symbol("sfAccount"), account);
            tx.set_account_id(get_field_by_symbol("sfDelegate"), granular_delegate);
            tx.set_field_u32(get_field_by_symbol("sfFlags"), protocol::tfMPTLock);
        });

        assert_eq!(
            typed_check_permission(&view, &account_set, &LedgerPreclaimTx { tx: &account_set }),
            Ter::TES_SUCCESS,
            "the generic rippled permission hierarchy short-circuits on a stored tx-level permission"
        );
        assert_eq!(
            typed_check_permission(&view, &payment, &LedgerPreclaimTx { tx: &payment }),
            Ter::TES_SUCCESS
        );
        assert_eq!(
            typed_check_permission(&view, &trust_set, &LedgerPreclaimTx { tx: &trust_set }),
            Ter::TES_SUCCESS
        );
        assert_eq!(
            typed_check_permission(&view, &mpt_set, &LedgerPreclaimTx { tx: &mpt_set }),
            Ter::TES_SUCCESS,
            "MPTokenIssuanceSet must accept its lock granular permission"
        );
    }

    #[test]
    fn delegated_iou_payment_mint_uses_endpoint_trustline_orientation() {
        // Payment.cpp::checkGranularSemantics reads keylet::trustLine(source,
        // destination, currency), then derives mint/burn from sfBalance and
        // the destination-side limit. sfAmount may name either endpoint as
        // issuer, so reading line(source, sfAmount.issuer) is not equivalent.
        let issuer = AccountID::from_array([0x41; 20]);
        let destination = AccountID::from_array([0x61; 20]);
        let delegate = AccountID::from_array([0x51; 20]);
        let currency = Currency::from_array([0x71; 20]);
        let mut view = ledger::Sandbox::new(
            Arc::new(ledger::Ledger::from_ledger_seq_and_close_time(1, 0, false)),
            ApplyFlags::NONE,
        );
        insert_delegate(
            &mut view,
            issuer,
            delegate,
            &[protocol::GranularPermissionType::PaymentMint as u32],
        );
        let iou = |mantissa, account| {
            STAmount::from_iou_amount(
                sf_generic(),
                IOUAmount::from_parts(mantissa, 0).expect("valid test IOU"),
                Issue::new(currency, account),
            )
        };
        let line_keylet = protocol::line(issuer, destination, currency);
        let mut line =
            STLedgerEntry::from_type_and_key(LedgerEntryType::RippleState, line_keylet.key);
        line.set_field_amount(
            get_field_by_symbol("sfBalance"),
            iou(0, protocol::no_account()),
        );
        line.set_field_amount(get_field_by_symbol("sfLowLimit"), iou(0, issuer));
        line.set_field_amount(get_field_by_symbol("sfHighLimit"), iou(1_000, destination));
        view.insert(Arc::new(line)).expect("insert trust line");

        let payment = STTx::new(TxType::PAYMENT, |tx| {
            tx.set_account_id(get_field_by_symbol("sfAccount"), issuer);
            tx.set_account_id(get_field_by_symbol("sfDelegate"), delegate);
            tx.set_account_id(get_field_by_symbol("sfDestination"), destination);
            tx.set_field_amount(get_field_by_symbol("sfAmount"), iou(10, issuer));
        });
        assert_eq!(
            typed_check_permission(&view, &payment, &LedgerPreclaimTx { tx: &payment }),
            Ter::TES_SUCCESS
        );
    }

    #[test]
    fn shared_preclaim_scales_minimum_fee_and_honors_unlimited_load_window() {
        // ../rippled/src/libxrpl/tx/Transactor.cpp::Transactor::minimumFee (lines 354-362) and ../rippled/src/libxrpl/server/LoadFeeTrack.cpp::scaleFeeLoad (lines 62-85).
        assert_eq!(scale_fee_load(10, 512, 256, 256, false), 20);
        assert_eq!(scale_fee_load(10, 512, 256, 256, true), 10);
        assert_eq!(scale_fee_load(10, 1_024, 256, 256, true), 40);
    }

    use super::check_loan_set_counterparty_sign;

    #[test]
    fn delegated_preclaim_uses_delegate_as_fee_payer_without_rechecking_sfaccount() {
        // ../rippled/src/libxrpl/tx/Transactor.cpp::Transactor::checkSign
        // selects sfDelegate over sfAccount, and
        // ../rippled/src/libxrpl/protocol/STTx.cpp::STTx::getFeePayer keeps
        // that selected delegate responsible for fee-claiming tec outcomes.
        let account = AccountID::from_array([0xD1; 20]);
        let delegate = AccountID::from_array([0xD2; 20]);
        let sponsor = AccountID::from_array([0xD3; 20]);
        let delegated = STTx::new(TxType::PAYMENT, |tx| {
            tx.set_account_id(get_field_by_symbol("sfAccount"), account);
            tx.set_account_id(get_field_by_symbol("sfDelegate"), delegate);
            tx.set_field_amount(
                get_field_by_symbol("sfFee"),
                STAmount::new_native(10, false),
            );
        });
        assert_eq!(LedgerPreclaimTx { tx: &delegated }.fee_payer(), delegate);

        let sponsored = STTx::new(TxType::PAYMENT, |tx| {
            tx.set_account_id(get_field_by_symbol("sfAccount"), account);
            tx.set_account_id(get_field_by_symbol("sfDelegate"), delegate);
            tx.set_account_id(get_field_by_symbol("sfSponsor"), sponsor);
            tx.set_field_u32(get_field_by_symbol("sfSponsorFlags"), 1);
            tx.set_field_amount(
                get_field_by_symbol("sfFee"),
                STAmount::new_native(10, false),
            );
        });
        assert_eq!(LedgerPreclaimTx { tx: &sponsored }.fee_payer(), sponsor);

        let reserve_sponsored = STTx::new(TxType::PAYMENT, |tx| {
            tx.set_account_id(get_field_by_symbol("sfAccount"), account);
            tx.set_account_id(get_field_by_symbol("sfDelegate"), delegate);
            tx.set_account_id(get_field_by_symbol("sfSponsor"), sponsor);
            tx.set_field_u32(get_field_by_symbol("sfSponsorFlags"), 2);
            tx.set_field_amount(
                get_field_by_symbol("sfFee"),
                STAmount::new_native(10, false),
            );
        });
        assert_eq!(
            LedgerPreclaimTx {
                tx: &reserve_sponsored
            }
            .fee_payer(),
            delegate
        );
    }

    #[test]
    fn loan_set_check_sign_requires_a_counter_signer_before_optional_signature_success() {
        // ../rippled/src/libxrpl/tx/transactors/lending/LoanSet.cpp::LoanSet::checkSign: resolves the counter-signer before accepting an absent CounterpartySignature.
        let account = AccountID::from_array([0xC5; 20]);
        let tx = STTx::new(TxType::LOAN_SET, |tx| {
            tx.set_account_id(get_field_by_symbol("sfAccount"), account);
        });
        let view = Ledger::from_ledger_seq_and_close_time(1, 0, false);

        assert_eq!(
            check_loan_set_counterparty_sign(&view, &tx, ApplyFlags::NONE, || Ter::TES_SUCCESS),
            Ter::TEM_BAD_SIGNER
        );
    }
}
