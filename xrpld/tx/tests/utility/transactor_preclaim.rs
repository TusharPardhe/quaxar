//! Integration tests that pin the shared `Transactor.cpp` preclaim helpers to
//! the current C++ behavior.

use protocol::{SeqProxy, Ter, trans_token};
use tx::{
    ApplyFlags, TransactorCheckFeeTx, TransactorCheckPermissionTx,
    TransactorCheckPriorTxAndLastLedgerTx, TransactorCheckSeqProxyTx, run_transactor_check_fee,
    run_transactor_check_permission, run_transactor_check_prior_tx_and_last_ledger,
    run_transactor_check_seq_proxy,
};

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
fn tx_transactor_check_permission_accepts_without_delegate() {
    let result = run_transactor_check_permission(
        &PermissionTx {
            account: "alice",
            delegate: None,
        },
        |_, _| -> Option<()> { panic!("missing delegate should skip ledger lookup") },
        |_, _| panic!("missing delegate should skip permission check"),
    );

    assert_eq!(result, Ter::TES_SUCCESS);
    assert_eq!(trans_token(result), "tesSUCCESS");
}

#[test]
fn tx_transactor_check_permission_returns_missing_delegate_error() {
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
fn tx_transactor_check_seq_proxy_keeps_sequence_error_order() {
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
    assert_eq!(trans_token(future), "terPRE_SEQ");
    assert_eq!(trans_token(past), "tefPAST_SEQ");
}

#[test]
fn tx_transactor_check_seq_proxy_keeps_ticket_error_order() {
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
    assert_eq!(trans_token(future), "terPRE_TICKET");
    assert_eq!(trans_token(missing_ticket), "tefNO_TICKET");
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
fn tx_transactor_check_prior_tx_and_last_ledger_keeps_current_guard_order() {
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
    assert_eq!(trans_token(wrong_prior), "tefWRONG_PRIOR");
    assert_eq!(trans_token(max_ledger), "tefMAX_LEDGER");
    assert_eq!(trans_token(already), "tefALREADY");
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
fn tx_transactor_check_fee_keeps_batch_and_open_ledger_rules() {
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
    assert_eq!(batch_non_zero, Ter::TEM_BAD_FEE);
    assert_eq!(open_too_small, Ter::TEL_INSUF_FEE_P);
    assert_eq!(trans_token(open_too_small), "telINSUF_FEE_P");
}

#[test]
fn tx_transactor_check_fee_keeps_closed_ledger_balance_mapping() {
    let closed_nonzero = run_transactor_check_fee(
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
    let closed_zero = run_transactor_check_fee(
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

    assert_eq!(closed_nonzero, Ter::TEC_INSUFF_FEE);
    assert_eq!(closed_zero, Ter::TER_INSUF_FEE_B);
    assert_eq!(trans_token(closed_nonzero), "tecINSUFF_FEE");
    assert_eq!(trans_token(closed_zero), "terINSUF_FEE_B");
}
