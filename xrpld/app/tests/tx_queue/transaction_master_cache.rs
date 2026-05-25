use app::{SharedTransaction, Transaction, TransactionMaster};
use basics::tagged_cache::ManualClock;
use protocol::{STAmount, STTx, TxType, get_field_by_symbol};
use std::sync::{Arc, Mutex};

fn account(fill: u8) -> protocol::AccountID {
    protocol::AccountID::from_array([fill; 20])
}

fn payment_tx(sequence: u32, fill: u8) -> STTx {
    STTx::new(TxType::PAYMENT, |tx| {
        tx.set_account_id(get_field_by_symbol("sfAccount"), account(fill));
        tx.set_account_id(
            get_field_by_symbol("sfDestination"),
            account(fill.wrapping_add(1)),
        );
        tx.set_field_amount(
            get_field_by_symbol("sfAmount"),
            STAmount::new_native(1_000_000, false),
        );
        tx.set_field_amount(
            get_field_by_symbol("sfFee"),
            STAmount::new_native(10, false),
        );
        tx.set_field_u32(get_field_by_symbol("sfSequence"), sequence);
    })
}

fn shared_transaction(tx: STTx) -> SharedTransaction {
    Arc::new(Mutex::new(Transaction::new(Arc::new(tx))))
}

#[test]
fn transaction_master_exposes_cache_keys_and_preserves_snapshot_semantics() {
    let clock = Arc::new(ManualClock::new(0));
    let master = TransactionMaster::new_with_clock(Arc::clone(&clock));

    let first_tx = payment_tx(5, 0x11);
    let second_tx = payment_tx(6, 0x21);
    let mut first = shared_transaction(first_tx.clone());
    let mut second = shared_transaction(second_tx.clone());

    master.canonicalize(&mut first);
    master.canonicalize(&mut second);

    let mut keys = master.cache_keys();
    keys.sort_unstable();
    assert_eq!(
        keys,
        vec![
            first_tx.get_transaction_id(),
            second_tx.get_transaction_id()
        ]
    );
    keys.clear();
    assert_eq!(master.cache_keys().len(), 2);

    drop(first);
    drop(second);
    clock.advance_seconds(30 * 60 + 1);
    master.sweep();
    assert!(master.cache_keys().is_empty());
}
