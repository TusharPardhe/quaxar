//! Transaction simulation tests — proves the node can run transactions against
//! current state and return exact results WITHOUT applying changes.
//!
//! This is the XRPL equivalent of Ethereum's eth_call / Solana's simulateTransaction.

use app::apply_submit_transactor_shell;
use ledger::ApplyViewImpl;
use ledger::{Ledger, LedgerHeader, StateBatchOp};
use protocol::{AccountID, STAmount, STTx, Ter, TxType, account_keylet, get_field_by_symbol};
use std::sync::Arc;

fn account(fill: u8) -> AccountID {
    AccountID::from_hex(&format!("{fill:02x}").repeat(20)).unwrap()
}

fn raw_id(id: AccountID) -> basics::base_uint::Uint160 {
    basics::base_uint::Uint160::from_slice(id.data()).unwrap()
}

fn make_funded_ledger(accounts: &[(AccountID, u64)]) -> Ledger {
    let mut ledger = Ledger::new(
        LedgerHeader {
            seq: 100,
            drops: accounts.iter().map(|(_, b)| b).sum(),
            close_time: 800_000_000,
            close_time_resolution: 10,
            ..LedgerHeader::default()
        },
        false,
    );

    let mut ops = Vec::new();
    for (acct, balance) in accounts {
        let key = account_keylet(raw_id(*acct));
        let mut sle = protocol::STLedgerEntry::new(key);
        sle.set_account_id(get_field_by_symbol("sfAccount"), *acct);
        sle.set_field_amount(
            get_field_by_symbol("sfBalance"),
            STAmount::new_native(*balance, false),
        );
        sle.set_field_u32(get_field_by_symbol("sfSequence"), 1);
        sle.set_field_u32(get_field_by_symbol("sfOwnerCount"), 0);
        ops.push((
            StateBatchOp::Insert,
            key.key,
            sle.get_serializer().data().to_vec(),
        ));
    }
    ledger.apply_state_batch(&ops).expect("fund accounts");
    ledger
}

/// Test: Simulate a valid payment — returns tesSUCCESS without changing state.
#[test]
fn simulate_valid_payment_returns_success() {
    let alice = account(0x11);
    let bob = account(0x22);
    let ledger = make_funded_ledger(&[(alice, 10_000_000_000), (bob, 10_000_000_000)]);

    let tx = STTx::new(TxType::PAYMENT, |tx| {
        tx.set_account_id(get_field_by_symbol("sfAccount"), alice);
        tx.set_account_id(get_field_by_symbol("sfDestination"), bob);
        tx.set_field_amount(
            get_field_by_symbol("sfAmount"),
            STAmount::new_native(1_000_000, false),
        );
        tx.set_field_amount(
            get_field_by_symbol("sfFee"),
            STAmount::new_native(10, false),
        );
        tx.set_field_u32(get_field_by_symbol("sfSequence"), 1);
    });

    // Simulate: create disposable view, run transactor, check result
    let mut view = ApplyViewImpl::new(Arc::new(ledger.clone()), tx::ApplyFlags::NONE);
    let result = apply_submit_transactor_shell(&mut view, &tx, TxType::PAYMENT);

    assert_eq!(
        result,
        Ter::TES_SUCCESS,
        "Valid payment should simulate as tesSUCCESS"
    );

    // Verify state was NOT changed on the original ledger
    let alice_sle = ledger.read(account_keylet(raw_id(alice))).unwrap().unwrap();
    let balance = alice_sle.get_field_amount(get_field_by_symbol("sfBalance"));
    assert_eq!(
        balance.xrp().drops(),
        10_000_000_000i64,
        "Original ledger must be unchanged"
    );
}

/// Test: Simulate underfunded payment — returns tecUNFUNDED_PAYMENT.
#[test]
fn simulate_underfunded_payment_returns_error() {
    let alice = account(0x33);
    let bob = account(0x44);
    let ledger = make_funded_ledger(&[(alice, 1_000), (bob, 10_000_000_000)]);

    let tx = STTx::new(TxType::PAYMENT, |tx| {
        tx.set_account_id(get_field_by_symbol("sfAccount"), alice);
        tx.set_account_id(get_field_by_symbol("sfDestination"), bob);
        tx.set_field_amount(
            get_field_by_symbol("sfAmount"),
            STAmount::new_native(999_999_999, false),
        );
        tx.set_field_amount(
            get_field_by_symbol("sfFee"),
            STAmount::new_native(10, false),
        );
        tx.set_field_u32(get_field_by_symbol("sfSequence"), 1);
    });

    let mut view = ApplyViewImpl::new(Arc::new(ledger), tx::ApplyFlags::NONE);
    let result = apply_submit_transactor_shell(&mut view, &tx, TxType::PAYMENT);

    // Should fail with insufficient funds
    assert_ne!(
        result,
        Ter::TES_SUCCESS,
        "Underfunded payment must not succeed"
    );
}

/// Test: Simulate payment to non-existent account — creates account when no reserve set.
#[test]
fn simulate_payment_to_new_account_succeeds_without_reserve() {
    let alice = account(0x55);
    let bob = account(0x66); // Not in ledger
    let ledger = make_funded_ledger(&[(alice, 10_000_000_000)]);

    let tx = STTx::new(TxType::PAYMENT, |tx| {
        tx.set_account_id(get_field_by_symbol("sfAccount"), alice);
        tx.set_account_id(get_field_by_symbol("sfDestination"), bob);
        tx.set_field_amount(
            get_field_by_symbol("sfAmount"),
            STAmount::new_native(1_000_000, false),
        );
        tx.set_field_amount(
            get_field_by_symbol("sfFee"),
            STAmount::new_native(10, false),
        );
        tx.set_field_u32(get_field_by_symbol("sfSequence"), 1);
    });

    let mut view = ApplyViewImpl::new(Arc::new(ledger), tx::ApplyFlags::NONE);
    let result = apply_submit_transactor_shell(&mut view, &tx, TxType::PAYMENT);

    // Without reserve configured, payment to new account succeeds (creates it)
    assert_eq!(result, Ter::TES_SUCCESS);
}

/// Test: Simulate with bad fee — returns temBAD_FEE.
#[test]
fn simulate_bad_fee_returns_tem() {
    let alice = account(0x77);
    let ledger = make_funded_ledger(&[(alice, 10_000_000_000)]);

    let tx = STTx::new(TxType::PAYMENT, |tx| {
        tx.set_account_id(get_field_by_symbol("sfAccount"), alice);
        tx.set_account_id(get_field_by_symbol("sfDestination"), account(0x88));
        tx.set_field_amount(
            get_field_by_symbol("sfAmount"),
            STAmount::new_native(1_000, false),
        );
        // Negative fee
        tx.set_field_amount(get_field_by_symbol("sfFee"), STAmount::new_native(10, true));
        tx.set_field_u32(get_field_by_symbol("sfSequence"), 1);
    });

    let mut view = ApplyViewImpl::new(Arc::new(ledger), tx::ApplyFlags::NONE);
    let result = apply_submit_transactor_shell(&mut view, &tx, TxType::PAYMENT);

    assert_eq!(result, Ter::TEM_BAD_FEE);
}

/// Test: Simulate with zero account — returns temBAD_SRC_ACCOUNT.
#[test]
fn simulate_zero_account_returns_tem() {
    let ledger = make_funded_ledger(&[(account(0xAA), 10_000_000_000)]);

    let tx = STTx::new(TxType::PAYMENT, |tx| {
        tx.set_account_id(get_field_by_symbol("sfAccount"), AccountID::default()); // all zeros
        tx.set_account_id(get_field_by_symbol("sfDestination"), account(0xBB));
        tx.set_field_amount(
            get_field_by_symbol("sfAmount"),
            STAmount::new_native(1_000, false),
        );
        tx.set_field_amount(
            get_field_by_symbol("sfFee"),
            STAmount::new_native(10, false),
        );
        tx.set_field_u32(get_field_by_symbol("sfSequence"), 1);
    });

    let mut view = ApplyViewImpl::new(Arc::new(ledger), tx::ApplyFlags::NONE);
    let result = apply_submit_transactor_shell(&mut view, &tx, TxType::PAYMENT);

    assert_eq!(result, Ter::TEM_BAD_SRC_ACCOUNT);
}

/// Test: Multiple simulations on same ledger don't interfere.
#[test]
fn multiple_simulations_are_independent() {
    let alice = account(0xCC);
    let bob = account(0xDD);
    let ledger = make_funded_ledger(&[(alice, 10_000_000_000), (bob, 10_000_000_000)]);

    // Simulate payment 1
    let tx1 = STTx::new(TxType::PAYMENT, |tx| {
        tx.set_account_id(get_field_by_symbol("sfAccount"), alice);
        tx.set_account_id(get_field_by_symbol("sfDestination"), bob);
        tx.set_field_amount(
            get_field_by_symbol("sfAmount"),
            STAmount::new_native(5_000_000_000, false),
        );
        tx.set_field_amount(
            get_field_by_symbol("sfFee"),
            STAmount::new_native(10, false),
        );
        tx.set_field_u32(get_field_by_symbol("sfSequence"), 1);
    });

    let mut view1 = ApplyViewImpl::new(Arc::new(ledger.clone()), tx::ApplyFlags::NONE);
    let r1 = apply_submit_transactor_shell(&mut view1, &tx1, TxType::PAYMENT);

    // Simulate payment 2 (same ledger — should see original balances, not tx1's changes)
    let tx2 = STTx::new(TxType::PAYMENT, |tx| {
        tx.set_account_id(get_field_by_symbol("sfAccount"), alice);
        tx.set_account_id(get_field_by_symbol("sfDestination"), bob);
        tx.set_field_amount(
            get_field_by_symbol("sfAmount"),
            STAmount::new_native(5_000_000_000, false),
        );
        tx.set_field_amount(
            get_field_by_symbol("sfFee"),
            STAmount::new_native(10, false),
        );
        tx.set_field_u32(get_field_by_symbol("sfSequence"), 1);
    });

    let mut view2 = ApplyViewImpl::new(Arc::new(ledger), tx::ApplyFlags::NONE);
    let r2 = apply_submit_transactor_shell(&mut view2, &tx2, TxType::PAYMENT);

    // Both should succeed — they each see the full 10B balance
    assert_eq!(r1, Ter::TES_SUCCESS);
    assert_eq!(r2, Ter::TES_SUCCESS);
}
