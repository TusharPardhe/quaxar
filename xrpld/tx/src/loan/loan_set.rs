//! Consolidated owner-facing `LoanSet` family surface above the already-landed
//! metadata, preflight, sign, preclaim, and apply helper slices.
//!
//! This gives callers one family-level API that composes the current
//! `LoanSet` helper graph in the same staged order as the reference transactor.

use std::{
    fmt::Display,
    ops::{Add, Mul, Sub},
};

use protocol::{NotTec, Ter};

use crate::{
    LoanSetBaseFeeTx, LoanSetDoApplyLedgerStateBroker, LoanSetDoApplyLedgerStateVault,
    LoanSetDoApplyLoadedGuardedTransferBroker, LoanSetDoApplyLoadedPreGuardedTransferBroker,
    LoanSetDoApplyLoadedPreGuardedTransferVault,
    LoanSetDoApplyLoadedTransferAndPostTransferAccountState,
    LoanSetDoApplyLoadedTransferAndPostTransferTx, LoanSetDoApplyPreGuardedTransferProperties,
    LoanSetDoApplyPreGuardedTransferState, LoanSetPreclaimBrokerTx, LoanSetPreclaimLoadedBroker,
    LoanSetPreclaimLoadedVault, LoanSetPreclaimPermissionTx, LoanSetPreclaimRepresentabilityTx,
    LoanSetPreflightTx, LoanSetRepresentabilityField, LoanSetScheduleGuardInputs, LoanSetSignTx,
    run_loan_set_calculate_base_fee, run_loan_set_check_sign, run_loan_set_do_apply,
    run_loan_set_invoke_preclaim, run_loan_set_invoke_preflight, run_loan_set_preclaim,
    run_loan_set_preflight,
};

#[allow(clippy::too_many_arguments)]
pub fn run_loan_set_family_preflight<Tx>(
    tx: &Tx,
    lending_protocol_enabled: bool,
    single_asset_vault_enabled: bool,
    batch_enabled: bool,
    check_vault_create_extra_features: impl FnOnce() -> bool,
    run_preflight1: impl FnOnce(u32) -> NotTec,
    check_signing_key: impl FnOnce(&Tx::CounterpartySignature) -> NotTec,
    validate_data_length: impl FnOnce() -> bool,
    validate_loan_service_fee: impl FnOnce() -> bool,
    validate_late_payment_fee: impl FnOnce() -> bool,
    validate_close_payment_fee: impl FnOnce() -> bool,
    validate_principal_requested: impl FnOnce() -> bool,
    validate_loan_origination_fee: impl FnOnce() -> bool,
    validate_interest_rate: impl FnOnce() -> bool,
    validate_overpayment_fee: impl FnOnce() -> bool,
    validate_late_interest_rate: impl FnOnce() -> bool,
    validate_close_interest_rate: impl FnOnce() -> bool,
    validate_overpayment_interest_rate: impl FnOnce() -> bool,
    validate_payment_total: impl FnOnce() -> bool,
    validate_payment_interval: impl FnOnce() -> bool,
    validate_grace_period: impl FnOnce() -> bool,
    check_simulate_keys: impl FnOnce(&Tx::CounterpartySignature) -> NotTec,
    check_broker_id: impl FnOnce() -> bool,
    run_preflight2: impl FnOnce() -> NotTec,
) -> NotTec
where
    Tx: LoanSetPreflightTx,
{
    run_loan_set_invoke_preflight(
        lending_protocol_enabled,
        single_asset_vault_enabled,
        check_vault_create_extra_features,
        run_preflight1,
        || {
            run_loan_set_preflight(
                tx,
                batch_enabled,
                check_signing_key,
                validate_data_length,
                validate_loan_service_fee,
                validate_late_payment_fee,
                validate_close_payment_fee,
                validate_principal_requested,
                validate_loan_origination_fee,
                validate_interest_rate,
                validate_overpayment_fee,
                validate_late_interest_rate,
                validate_close_interest_rate,
                validate_overpayment_interest_rate,
                validate_payment_total,
                validate_payment_interval,
                validate_grace_period,
                check_simulate_keys,
                check_broker_id,
            )
        },
        run_preflight2,
    )
}

#[allow(clippy::too_many_arguments)]
pub fn run_loan_set_family_preclaim<
    Tx,
    AccountId,
    Broker,
    Borrower,
    Vault,
    Fee,
    ReadBrokerOwner,
    CheckSign,
    CheckCounterpartySign,
    ReadBroker,
    ReadBorrower,
    ReadVault,
    CanRepresent,
    CheckCanAddHolding,
    CheckFrozen,
    CheckDeepFrozen,
>(
    tx: &Tx,
    account_is_zero: bool,
    schedule_inputs: LoanSetScheduleGuardInputs,
    check_seq_proxy: impl FnOnce() -> NotTec,
    check_prior_tx_and_last_ledger: impl FnOnce() -> NotTec,
    check_permission: impl FnOnce() -> NotTec,
    read_broker_owner: ReadBrokerOwner,
    check_sign: CheckSign,
    check_counterparty_sign: CheckCounterpartySign,
    calculate_base_fee: impl FnOnce() -> Fee,
    check_fee: impl FnOnce(Fee) -> Ter,
    read_broker: ReadBroker,
    read_borrower: ReadBorrower,
    read_vault: ReadVault,
    can_represent: CanRepresent,
    check_can_add_holding: CheckCanAddHolding,
    check_frozen: CheckFrozen,
    check_deep_frozen: CheckDeepFrozen,
) -> Ter
where
    Tx: LoanSetSignTx<AccountId = AccountId>
        + LoanSetPreclaimBrokerTx
        + LoanSetPreclaimPermissionTx<AccountId = AccountId>
        + LoanSetPreclaimRepresentabilityTx,
    AccountId: Clone + PartialEq + Eq,
    Broker: LoanSetPreclaimLoadedBroker<AccountId = AccountId>,
    Vault: LoanSetPreclaimLoadedVault<AccountId = AccountId>,
    ReadBrokerOwner: FnMut() -> Option<AccountId>,
    CheckSign: FnOnce() -> NotTec,
    CheckCounterpartySign: FnOnce(AccountId, &Tx::CounterpartySignature) -> NotTec,
    ReadBroker: FnOnce(&Tx::BrokerId) -> Option<Broker>,
    ReadBorrower: FnOnce(&AccountId) -> Option<Borrower>,
    ReadVault: FnOnce(&Broker::VaultId) -> Option<Vault>,
    CanRepresent: FnMut(LoanSetRepresentabilityField, &Tx::Value) -> bool,
    CheckCanAddHolding: FnOnce(&Vault::Asset) -> Ter,
    CheckFrozen: FnMut(&AccountId, &Vault::Asset) -> Ter,
    CheckDeepFrozen: FnMut(&AccountId, &Vault::Asset) -> Ter,
{
    run_loan_set_invoke_preclaim(
        account_is_zero,
        check_seq_proxy,
        check_prior_tx_and_last_ledger,
        check_permission,
        || run_loan_set_check_sign(tx, read_broker_owner, check_sign, check_counterparty_sign),
        calculate_base_fee,
        check_fee,
        || {
            run_loan_set_preclaim(
                tx,
                schedule_inputs,
                read_broker,
                read_borrower,
                read_vault,
                can_represent,
                check_can_add_holding,
                check_frozen,
                check_deep_frozen,
            )
        },
    )
}

pub fn run_loan_set_family_calculate_base_fee<Tx, Fee>(
    tx: &Tx,
    normal_cost: Fee,
    base_fee: Fee,
) -> Fee
where
    Tx: LoanSetBaseFeeTx,
    Fee: Copy + Add<Output = Fee> + Mul<u64, Output = Fee>,
{
    run_loan_set_calculate_base_fee(tx, normal_cost, base_fee)
}

#[allow(clippy::too_many_arguments)]
pub fn run_loan_set_family_do_apply<
    Tx,
    Broker,
    AccountState,
    Vault,
    Asset,
    Amount,
    Balance,
    InterestRate,
    OwnerCount,
    Properties,
    State,
    ReadBroker,
    ReadVault,
    ReadAccount,
    ComputeVaultScale,
    ComputeLoanProperties,
    ConstructLoanState,
    CanRepresent,
    CheckLoanGuards,
    ComputeRequiredCover,
    IncrementBorrowerOwnerCount,
    ComputeAccountReserve,
    AddBorrowerHolding,
    CheckBorrowerAuth,
    AddOwnerHolding,
    CheckOwnerAuth,
    AccountSendMulti,
    RunPostTransfer,
>(
    tx: &Tx,
    pre_fee_balance: &Balance,
    read_broker: ReadBroker,
    read_vault: ReadVault,
    read_account: ReadAccount,
    default_interest_rate: InterestRate,
    default_payment_interval: u32,
    default_payment_total: u32,
    zero: &Amount,
    compute_vault_scale: ComputeVaultScale,
    compute_loan_properties: ComputeLoanProperties,
    construct_loan_state: ConstructLoanState,
    can_represent: CanRepresent,
    check_loan_guards: CheckLoanGuards,
    compute_required_cover: ComputeRequiredCover,
    increment_borrower_owner_count: IncrementBorrowerOwnerCount,
    compute_account_reserve: ComputeAccountReserve,
    add_borrower_holding: AddBorrowerHolding,
    check_borrower_auth: CheckBorrowerAuth,
    add_owner_holding: AddOwnerHolding,
    check_owner_auth: CheckOwnerAuth,
    account_send_multi: AccountSendMulti,
    run_post_transfer: RunPostTransfer,
) -> Ter
where
    Tx: LoanSetDoApplyLoadedTransferAndPostTransferTx<Amount = Amount, InterestRate = InterestRate>,
    Tx::AccountId: Clone + Eq,
    Broker: LoanSetDoApplyLedgerStateBroker<AccountId = Tx::AccountId>
        + LoanSetDoApplyLoadedGuardedTransferBroker<Amount = Amount>
        + LoanSetDoApplyLoadedPreGuardedTransferBroker,
    Vault: LoanSetDoApplyLedgerStateVault<AccountId = Tx::AccountId, Asset = Asset>
        + LoanSetDoApplyLoadedPreGuardedTransferVault<Amount = Amount>,
    AccountState: LoanSetDoApplyLoadedTransferAndPostTransferAccountState<Balance = Balance>,
    Asset: Clone,
    Amount: Clone + Display + PartialEq + PartialOrd + Add<Output = Amount> + Sub<Output = Amount>,
    Balance: PartialOrd,
    InterestRate: Copy + PartialEq,
    OwnerCount: Copy,
    Properties: LoanSetDoApplyPreGuardedTransferProperties<Amount = Amount>,
    State: LoanSetDoApplyPreGuardedTransferState<Amount = Amount>,
    ReadBroker: FnOnce(&Tx::BrokerId) -> Option<Broker>,
    ReadVault: FnOnce(&<Broker as LoanSetDoApplyLedgerStateBroker>::VaultId) -> Option<Vault>,
    ReadAccount: FnMut(&Tx::AccountId) -> Option<AccountState>,
    ComputeVaultScale: FnOnce(&Vault) -> i32,
    ComputeLoanProperties: FnOnce(
        &Asset,
        &Amount,
        InterestRate,
        u32,
        u32,
        <Broker as LoanSetDoApplyLoadedPreGuardedTransferBroker>::ManagementFeeRate,
        i32,
    ) -> Properties,
    ConstructLoanState: FnOnce(&Amount, &Amount, &Amount) -> State,
    CanRepresent: FnMut(LoanSetRepresentabilityField, &Tx::Value) -> bool,
    CheckLoanGuards: FnOnce(&Asset, &Amount, bool, u32, &Properties) -> Ter,
    ComputeRequiredCover:
        FnOnce(&Amount, <Broker as LoanSetDoApplyLoadedGuardedTransferBroker>::CoverRate) -> Amount,
    IncrementBorrowerOwnerCount: FnOnce() -> OwnerCount,
    ComputeAccountReserve: FnOnce(OwnerCount) -> Balance,
    AddBorrowerHolding: FnOnce() -> Ter,
    CheckBorrowerAuth: FnOnce() -> Ter,
    AddOwnerHolding: FnOnce() -> Ter,
    CheckOwnerAuth: FnOnce() -> Ter,
    AccountSendMulti: FnOnce() -> Ter,
    RunPostTransfer: FnOnce() -> Ter,
{
    run_loan_set_do_apply(
        tx,
        pre_fee_balance,
        read_broker,
        read_vault,
        read_account,
        default_interest_rate,
        default_payment_interval,
        default_payment_total,
        zero,
        compute_vault_scale,
        compute_loan_properties,
        construct_loan_state,
        can_represent,
        check_loan_guards,
        compute_required_cover,
        increment_borrower_owner_count,
        compute_account_reserve,
        add_borrower_holding,
        check_borrower_auth,
        add_owner_holding,
        check_owner_auth,
        account_send_multi,
        run_post_transfer,
    )
}
