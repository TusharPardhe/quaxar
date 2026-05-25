use std::sync::Arc;

use ledger::{Ledger, LedgerHeader, LedgerHolder};

fn immutable_ledger() -> Arc<Ledger> {
    let mut ledger = Ledger::new(
        LedgerHeader {
            seq: 91,
            ..LedgerHeader::default()
        },
        true,
    );
    ledger.set_immutable(true);
    Arc::new(ledger)
}

fn mutable_ledger() -> Arc<Ledger> {
    Arc::new(Ledger::new(
        LedgerHeader {
            seq: 92,
            ..LedgerHeader::default()
        },
        true,
    ))
}

#[test]
fn ledger_holder_starts_empty_and_returns_none() {
    let holder = LedgerHolder::new();

    assert!(holder.empty());
    assert!(holder.get().is_none());
}

#[test]
fn ledger_holder_sets_and_returns_the_same_immutable_ledger() {
    let holder = LedgerHolder::new();
    let ledger = immutable_ledger();

    holder.set(Some(Arc::clone(&ledger)));

    assert!(!holder.empty());
    let held = holder
        .get()
        .expect("holder should return the stored ledger");
    assert!(held.is_immutable());
    assert_eq!(held.header().seq, ledger.header().seq);
    assert!(Arc::ptr_eq(&held, &ledger));
}

#[test]
#[should_panic(expected = "LedgerHolder::set with nullptr")]
fn ledger_holder_rejects_null_ledgers() {
    let holder = LedgerHolder::new();

    holder.set(None);
}

#[test]
#[should_panic(expected = "LedgerHolder::set with mutable Ledger")]
fn ledger_holder_rejects_mutable_ledgers() {
    let holder = LedgerHolder::new();

    holder.set(Some(mutable_ledger()));
}
