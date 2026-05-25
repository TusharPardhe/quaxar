//! Current Rust helpers mirroring the shared the reference implementation preclaim
//! helpers that run before transaction-specific `preclaim(...)`.
//!
//! This module preserves the current deterministic helper behavior around:
//!
//! - delegated permission lookup and passthrough,
//! - sequence versus ticket checks,
//! - prior-tx, last-ledger, and duplicate-tx rejection,
//! - and fee validation before the transaction-specific preclaim step.

use protocol::{NotTec, SeqProxy, Ter};

use crate::{ApplyFlags, any_apply_flags};

pub trait TransactorCheckPermissionTx {
    type AccountId: Clone;

    fn account_id(&self) -> Self::AccountId;
    fn delegate(&self) -> Option<Self::AccountId>;
}

pub fn run_transactor_check_permission<Tx, DelegateState, ReadDelegate, CheckTxPermission>(
    tx: &Tx,
    read_delegate: ReadDelegate,
    check_tx_permission: CheckTxPermission,
) -> NotTec
where
    Tx: TransactorCheckPermissionTx,
    ReadDelegate: FnOnce(&Tx::AccountId, &Tx::AccountId) -> Option<DelegateState>,
    CheckTxPermission: FnOnce(DelegateState, &Tx) -> NotTec,
{
    let Some(delegate) = tx.delegate() else {
        return Ter::TES_SUCCESS;
    };

    let account = tx.account_id();
    let Some(delegate_state) = read_delegate(&account, &delegate) else {
        return Ter::TER_NO_DELEGATE_PERMISSION;
    };

    check_tx_permission(delegate_state, tx)
}

pub trait TransactorCheckSeqProxyTx {
    type AccountId: Clone;

    fn account_id(&self) -> Self::AccountId;
    fn seq_proxy(&self) -> SeqProxy;
    fn ticket_sequence_present(&self) -> bool;
}

pub fn run_transactor_check_seq_proxy<
    Tx,
    AccountState,
    ReadAccount,
    AccountSequence,
    TicketExists,
>(
    tx: &Tx,
    mut read_account: ReadAccount,
    mut account_sequence: AccountSequence,
    mut ticket_exists: TicketExists,
) -> NotTec
where
    Tx: TransactorCheckSeqProxyTx,
    ReadAccount: FnMut(&Tx::AccountId) -> Option<AccountState>,
    AccountSequence: FnMut(&AccountState) -> u32,
    TicketExists: FnMut(&Tx::AccountId, SeqProxy) -> bool,
{
    let account = tx.account_id();
    let Some(account_state) = read_account(&account) else {
        return Ter::TER_NO_ACCOUNT;
    };

    let tx_seq_proxy = tx.seq_proxy();
    let account_seq = SeqProxy::sequence(account_sequence(&account_state));

    if tx_seq_proxy.is_seq() {
        if tx.ticket_sequence_present() {
            return Ter::TEM_SEQ_AND_TICKET;
        }

        if tx_seq_proxy != account_seq {
            if account_seq < tx_seq_proxy {
                return Ter::TER_PRE_SEQ;
            }

            return Ter::TEF_PAST_SEQ;
        }
    } else if tx_seq_proxy.is_ticket() {
        if account_seq.value() <= tx_seq_proxy.value() {
            return Ter::TER_PRE_TICKET;
        }

        if !ticket_exists(&account, tx_seq_proxy) {
            return Ter::TEF_NO_TICKET;
        }
    }

    Ter::TES_SUCCESS
}

pub trait TransactorCheckPriorTxAndLastLedgerTx {
    type AccountId: Clone;
    type TxId: Eq;

    fn account_id(&self) -> Self::AccountId;
    fn account_txn_id(&self) -> Option<Self::TxId>;
    fn last_ledger_sequence(&self) -> Option<u32>;
    fn transaction_id(&self) -> Self::TxId;
}

pub fn run_transactor_check_prior_tx_and_last_ledger<
    Tx,
    AccountState,
    ReadAccount,
    AccountTxnId,
    TxExists,
>(
    current_ledger_seq: u32,
    tx: &Tx,
    mut read_account: ReadAccount,
    mut account_txn_id: AccountTxnId,
    mut tx_exists: TxExists,
) -> NotTec
where
    Tx: TransactorCheckPriorTxAndLastLedgerTx,
    ReadAccount: FnMut(&Tx::AccountId) -> Option<AccountState>,
    AccountTxnId: FnMut(&AccountState) -> Tx::TxId,
    TxExists: FnMut(&Tx::TxId) -> bool,
{
    let account = tx.account_id();
    let Some(account_state) = read_account(&account) else {
        return Ter::TER_NO_ACCOUNT;
    };

    if let Some(prior_tx) = tx.account_txn_id()
        && account_txn_id(&account_state) != prior_tx
    {
        return Ter::TEF_WRONG_PRIOR;
    }

    if let Some(last_ledger_seq) = tx.last_ledger_sequence()
        && current_ledger_seq > last_ledger_seq
    {
        return Ter::TEF_MAX_LEDGER;
    }

    let tx_id = tx.transaction_id();
    if tx_exists(&tx_id) {
        return Ter::TEF_ALREADY;
    }

    Ter::TES_SUCCESS
}

pub trait TransactorCheckFeeTx {
    type AccountId: Clone;
    type Amount: Copy + Ord;

    fn fee_is_native(&self) -> bool;
    fn fee_paid(&self) -> Self::Amount;
    fn fee_payer(&self) -> Self::AccountId;
}

#[allow(clippy::too_many_arguments)]
pub fn run_transactor_check_fee<
    Tx,
    AccountState,
    IsLegalAmount,
    MinimumFee,
    ReadAccount,
    AccountBalance,
>(
    flags: ApplyFlags,
    ledger_open: bool,
    tx: &Tx,
    base_fee: Tx::Amount,
    zero: Tx::Amount,
    mut is_legal_amount: IsLegalAmount,
    minimum_fee: MinimumFee,
    mut read_account: ReadAccount,
    mut account_balance: AccountBalance,
) -> Ter
where
    Tx: TransactorCheckFeeTx,
    IsLegalAmount: FnMut(Tx::Amount) -> bool,
    MinimumFee: FnOnce(Tx::Amount) -> Tx::Amount,
    ReadAccount: FnMut(&Tx::AccountId) -> Option<AccountState>,
    AccountBalance: FnMut(&AccountState) -> Tx::Amount,
{
    if !tx.fee_is_native() {
        return Ter::TEM_BAD_FEE;
    }

    let fee_paid = tx.fee_paid();

    if any_apply_flags(flags & ApplyFlags::BATCH) {
        return if fee_paid == zero {
            Ter::TES_SUCCESS
        } else {
            Ter::TEM_BAD_FEE
        };
    }

    if !is_legal_amount(fee_paid) || fee_paid < zero {
        return Ter::TEM_BAD_FEE;
    }

    if ledger_open {
        let fee_due = minimum_fee(base_fee);
        if fee_paid < fee_due {
            return Ter::TEL_INSUF_FEE_P;
        }
    }

    if fee_paid == zero {
        return Ter::TES_SUCCESS;
    }

    let fee_payer = tx.fee_payer();
    let Some(account_state) = read_account(&fee_payer) else {
        return Ter::TER_NO_ACCOUNT;
    };

    let balance = account_balance(&account_state);
    if balance < fee_paid {
        if balance > zero && !ledger_open {
            return Ter::TEC_INSUFF_FEE;
        }

        return Ter::TER_INSUF_FEE_B;
    }

    Ter::TES_SUCCESS
}

#[cfg(test)]
mod tests {
    use protocol::{SeqProxy, Ter, trans_token};

    use super::{
        TransactorCheckFeeTx, TransactorCheckPermissionTx, TransactorCheckPriorTxAndLastLedgerTx,
        TransactorCheckSeqProxyTx, run_transactor_check_fee, run_transactor_check_permission,
        run_transactor_check_prior_tx_and_last_ledger, run_transactor_check_seq_proxy,
    };
    use crate::ApplyFlags;

    struct PermissionTx {
        account: &'static str,
        delegate: Option<&'static str>,
    }

    impl TransactorCheckPermissionTx for PermissionTx {
        type AccountId = &'static str;

        fn account_id(&self) -> Self::AccountId {
            self.account
        }

        fn delegate(&self) -> Option<Self::AccountId> {
            self.delegate
        }
    }

    #[test]
    fn transactor_check_permission_accepts_without_delegate() {
        let result = run_transactor_check_permission(
            &PermissionTx {
                account: "alice",
                delegate: None,
            },
            |_, _| -> Option<()> { panic!("no delegate should skip lookup") },
            |_, _| panic!("no delegate should skip permission check"),
        );

        assert_eq!(result, Ter::TES_SUCCESS);
    }

    #[test]
    fn transactor_check_permission_returns_missing_delegate_error() {
        let result = run_transactor_check_permission(
            &PermissionTx {
                account: "alice",
                delegate: Some("bob"),
            },
            |account, delegate| {
                assert_eq!(*account, "alice");
                assert_eq!(*delegate, "bob");
                None::<()>
            },
            |_, _| panic!("missing delegate entry should skip permission decode"),
        );

        assert_eq!(result, Ter::TER_NO_DELEGATE_PERMISSION);
        assert_eq!(trans_token(result), "terNO_DELEGATE_PERMISSION");
    }

    #[test]
    fn transactor_check_permission_returns_permission_result_unchanged() {
        let result = run_transactor_check_permission(
            &PermissionTx {
                account: "alice",
                delegate: Some("bob"),
            },
            |_, _| Some("delegate-state"),
            |delegate_state, _| {
                assert_eq!(delegate_state, "delegate-state");
                Ter::TEM_DISABLED
            },
        );

        assert_eq!(result, Ter::TEM_DISABLED);
    }

    struct SeqProxyTx {
        account: &'static str,
        seq_proxy: SeqProxy,
        ticket_sequence_present: bool,
    }

    impl TransactorCheckSeqProxyTx for SeqProxyTx {
        type AccountId = &'static str;

        fn account_id(&self) -> Self::AccountId {
            self.account
        }

        fn seq_proxy(&self) -> SeqProxy {
            self.seq_proxy
        }

        fn ticket_sequence_present(&self) -> bool {
            self.ticket_sequence_present
        }
    }

    #[test]
    fn transactor_check_seq_proxy_rejects_missing_account() {
        let result = run_transactor_check_seq_proxy(
            &SeqProxyTx {
                account: "alice",
                seq_proxy: SeqProxy::sequence(1),
                ticket_sequence_present: false,
            },
            |_| None::<u32>,
            |account_state| *account_state,
            |_, _| true,
        );

        assert_eq!(result, Ter::TER_NO_ACCOUNT);
    }

    #[test]
    fn transactor_check_seq_proxy_rejects_seq_and_ticket_combo() {
        let result = run_transactor_check_seq_proxy(
            &SeqProxyTx {
                account: "alice",
                seq_proxy: SeqProxy::sequence(7),
                ticket_sequence_present: true,
            },
            |_| Some(7_u32),
            |account_state| *account_state,
            |_, _| true,
        );

        assert_eq!(result, Ter::TEM_SEQ_AND_TICKET);
    }

    #[test]
    fn transactor_check_seq_proxy_preserves_sequence_error_order() {
        let future = run_transactor_check_seq_proxy(
            &SeqProxyTx {
                account: "alice",
                seq_proxy: SeqProxy::sequence(8),
                ticket_sequence_present: false,
            },
            |_| Some(7_u32),
            |account_state| *account_state,
            |_, _| true,
        );
        let past = run_transactor_check_seq_proxy(
            &SeqProxyTx {
                account: "alice",
                seq_proxy: SeqProxy::sequence(6),
                ticket_sequence_present: false,
            },
            |_| Some(7_u32),
            |account_state| *account_state,
            |_, _| true,
        );

        assert_eq!(future, Ter::TER_PRE_SEQ);
        assert_eq!(past, Ter::TEF_PAST_SEQ);
    }

    #[test]
    fn transactor_check_seq_proxy_preserves_ticket_error_order() {
        let future = run_transactor_check_seq_proxy(
            &SeqProxyTx {
                account: "alice",
                seq_proxy: SeqProxy::ticket(7),
                ticket_sequence_present: false,
            },
            |_| Some(7_u32),
            |account_state| *account_state,
            |_, _| true,
        );
        let missing_ticket = run_transactor_check_seq_proxy(
            &SeqProxyTx {
                account: "alice",
                seq_proxy: SeqProxy::ticket(7),
                ticket_sequence_present: false,
            },
            |_| Some(8_u32),
            |account_state| *account_state,
            |_, _| false,
        );

        assert_eq!(future, Ter::TER_PRE_TICKET);
        assert_eq!(missing_ticket, Ter::TEF_NO_TICKET);
    }

    #[test]
    fn transactor_check_seq_proxy_accepts_current_sequence_and_existing_ticket() {
        let sequence = run_transactor_check_seq_proxy(
            &SeqProxyTx {
                account: "alice",
                seq_proxy: SeqProxy::sequence(7),
                ticket_sequence_present: false,
            },
            |_| Some(7_u32),
            |account_state| *account_state,
            |_, _| panic!("sequence path should skip ticket lookup"),
        );
        let ticket = run_transactor_check_seq_proxy(
            &SeqProxyTx {
                account: "alice",
                seq_proxy: SeqProxy::ticket(7),
                ticket_sequence_present: false,
            },
            |_| Some(8_u32),
            |account_state| *account_state,
            |account, seq_proxy| {
                assert_eq!(*account, "alice");
                assert_eq!(seq_proxy, SeqProxy::ticket(7));
                true
            },
        );

        assert_eq!(sequence, Ter::TES_SUCCESS);
        assert_eq!(ticket, Ter::TES_SUCCESS);
    }

    struct PriorTx {
        account: &'static str,
        account_txn_id: Option<u32>,
        last_ledger_sequence: Option<u32>,
        transaction_id: u32,
    }

    impl TransactorCheckPriorTxAndLastLedgerTx for PriorTx {
        type AccountId = &'static str;
        type TxId = u32;

        fn account_id(&self) -> Self::AccountId {
            self.account
        }

        fn account_txn_id(&self) -> Option<Self::TxId> {
            self.account_txn_id
        }

        fn last_ledger_sequence(&self) -> Option<u32> {
            self.last_ledger_sequence
        }

        fn transaction_id(&self) -> Self::TxId {
            self.transaction_id
        }
    }

    #[test]
    fn transactor_check_prior_tx_and_last_ledger_rejects_missing_account() {
        let result = run_transactor_check_prior_tx_and_last_ledger(
            10,
            &PriorTx {
                account: "alice",
                account_txn_id: None,
                last_ledger_sequence: None,
                transaction_id: 55,
            },
            |_| None::<u32>,
            |state| *state,
            |_| false,
        );

        assert_eq!(result, Ter::TER_NO_ACCOUNT);
    }

    #[test]
    fn transactor_check_prior_tx_and_last_ledger_preserves_guard_order() {
        let wrong_prior = run_transactor_check_prior_tx_and_last_ledger(
            10,
            &PriorTx {
                account: "alice",
                account_txn_id: Some(99),
                last_ledger_sequence: Some(8),
                transaction_id: 55,
            },
            |_| Some(11_u32),
            |state| *state,
            |_| true,
        );
        let max_ledger = run_transactor_check_prior_tx_and_last_ledger(
            10,
            &PriorTx {
                account: "alice",
                account_txn_id: Some(11),
                last_ledger_sequence: Some(8),
                transaction_id: 55,
            },
            |_| Some(11_u32),
            |state| *state,
            |_| true,
        );
        let already = run_transactor_check_prior_tx_and_last_ledger(
            10,
            &PriorTx {
                account: "alice",
                account_txn_id: Some(11),
                last_ledger_sequence: Some(10),
                transaction_id: 55,
            },
            |_| Some(11_u32),
            |state| *state,
            |tx_id| *tx_id == 55,
        );

        assert_eq!(wrong_prior, Ter::TEF_WRONG_PRIOR);
        assert_eq!(max_ledger, Ter::TEF_MAX_LEDGER);
        assert_eq!(already, Ter::TEF_ALREADY);
    }

    #[test]
    fn transactor_check_prior_tx_and_last_ledger_accepts_current_state() {
        let result = run_transactor_check_prior_tx_and_last_ledger(
            10,
            &PriorTx {
                account: "alice",
                account_txn_id: Some(11),
                last_ledger_sequence: Some(10),
                transaction_id: 55,
            },
            |_| Some(11_u32),
            |state| *state,
            |_| false,
        );

        assert_eq!(result, Ter::TES_SUCCESS);
    }

    struct FeeTx {
        fee_is_native: bool,
        fee_paid: i64,
        fee_payer: &'static str,
    }

    impl TransactorCheckFeeTx for FeeTx {
        type AccountId = &'static str;
        type Amount = i64;

        fn fee_is_native(&self) -> bool {
            self.fee_is_native
        }

        fn fee_paid(&self) -> Self::Amount {
            self.fee_paid
        }

        fn fee_payer(&self) -> Self::AccountId {
            self.fee_payer
        }
    }

    #[test]
    fn transactor_check_fee_rejects_non_native_batch_and_invalid_amounts() {
        let non_native = run_transactor_check_fee(
            ApplyFlags::NONE,
            true,
            &FeeTx {
                fee_is_native: false,
                fee_paid: 10,
                fee_payer: "alice",
            },
            10,
            0,
            |_| true,
            |_| 10,
            |_| Some(100_i64),
            |balance| *balance,
        );
        let batch_non_zero = run_transactor_check_fee(
            ApplyFlags::BATCH,
            true,
            &FeeTx {
                fee_is_native: true,
                fee_paid: 1,
                fee_payer: "alice",
            },
            10,
            0,
            |_| true,
            |_| 10,
            |_| Some(100_i64),
            |balance| *balance,
        );
        let illegal = run_transactor_check_fee(
            ApplyFlags::NONE,
            true,
            &FeeTx {
                fee_is_native: true,
                fee_paid: -1,
                fee_payer: "alice",
            },
            10,
            0,
            |_| false,
            |_| 10,
            |_| Some(100_i64),
            |balance| *balance,
        );

        assert_eq!(non_native, Ter::TEM_BAD_FEE);
        assert_eq!(batch_non_zero, Ter::TEM_BAD_FEE);
        assert_eq!(illegal, Ter::TEM_BAD_FEE);
    }

    #[test]
    fn transactor_check_fee_handles_batch_zero_and_open_ledger_minimum() {
        let batch_zero = run_transactor_check_fee(
            ApplyFlags::BATCH,
            true,
            &FeeTx {
                fee_is_native: true,
                fee_paid: 0,
                fee_payer: "alice",
            },
            10,
            0,
            |_| true,
            |_| 20,
            |_| Some(100_i64),
            |balance| *balance,
        );
        let open_too_small = run_transactor_check_fee(
            ApplyFlags::NONE,
            true,
            &FeeTx {
                fee_is_native: true,
                fee_paid: 19,
                fee_payer: "alice",
            },
            10,
            0,
            |_| true,
            |_| 20,
            |_| Some(100_i64),
            |balance| *balance,
        );

        assert_eq!(batch_zero, Ter::TES_SUCCESS);
        assert_eq!(open_too_small, Ter::TEL_INSUF_FEE_P);
    }

    #[test]
    fn transactor_check_fee_preserves_balance_outcomes() {
        let zero_fee = run_transactor_check_fee(
            ApplyFlags::NONE,
            false,
            &FeeTx {
                fee_is_native: true,
                fee_paid: 0,
                fee_payer: "alice",
            },
            10,
            0,
            |_| true,
            |_| 20,
            |_| Some(100_i64),
            |balance| *balance,
        );
        let missing_fee_payer = run_transactor_check_fee(
            ApplyFlags::NONE,
            false,
            &FeeTx {
                fee_is_native: true,
                fee_paid: 20,
                fee_payer: "alice",
            },
            10,
            0,
            |_| true,
            |_| 20,
            |_| None::<i64>,
            |balance| *balance,
        );
        let closed_insufficient_nonzero = run_transactor_check_fee(
            ApplyFlags::NONE,
            false,
            &FeeTx {
                fee_is_native: true,
                fee_paid: 20,
                fee_payer: "alice",
            },
            10,
            0,
            |_| true,
            |_| 20,
            |_| Some(10_i64),
            |balance| *balance,
        );
        let insufficient_zero = run_transactor_check_fee(
            ApplyFlags::NONE,
            false,
            &FeeTx {
                fee_is_native: true,
                fee_paid: 20,
                fee_payer: "alice",
            },
            10,
            0,
            |_| true,
            |_| 20,
            |_| Some(0_i64),
            |balance| *balance,
        );
        let success = run_transactor_check_fee(
            ApplyFlags::NONE,
            false,
            &FeeTx {
                fee_is_native: true,
                fee_paid: 20,
                fee_payer: "alice",
            },
            10,
            0,
            |_| true,
            |_| 20,
            |_| Some(20_i64),
            |balance| *balance,
        );

        assert_eq!(zero_fee, Ter::TES_SUCCESS);
        assert_eq!(missing_fee_payer, Ter::TER_NO_ACCOUNT);
        assert_eq!(closed_insufficient_nonzero, Ter::TEC_INSUFF_FEE);
        assert_eq!(insufficient_zero, Ter::TER_INSUF_FEE_B);
        assert_eq!(success, Ter::TES_SUCCESS);
    }
}
