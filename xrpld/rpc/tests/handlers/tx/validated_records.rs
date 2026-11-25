//! `validated` for ledgers the validated ledger's skip lists cannot name.

use super::*;

fn accepted(seq: u32) -> Arc<Ledger> {
    let mut ledger = funded_parent_ledger(seq, account(0x61), 1);
    ledger.set_accepted(800 + seq, LEDGER_DEFAULT_TIME_RESOLUTION, true);
    Arc::new(ledger)
}

/// More than 256 ledgers back, rippled's `walkHashBySeq` reads the covering
/// flag ledger's skip list. For a ledger loaded from disk that list is often
/// not in memory, and quaxar then answered `validated: false` for every
/// ledger older than 256 (mainnet capture, 2026-11-25). Quaxar's validated
/// ledger records (ledger history, ledger database) answer instead. Modelled
/// by a validated ledger whose state has no skip list.
#[test]
fn validated_falls_back_to_validated_ledger_records() {
    let mut app = ApplicationRoot::with_options(app::ApplicationRootOptions {
        standalone: true,
        ..app::ApplicationRootOptions::default()
    })
    .expect("standalone root should build");
    let _ = app.attach_default_network_ops_runtime();
    let older = accepted(5);
    let reference = accepted(9);
    assert!(app.on_validated_ledger(Arc::clone(&older)));
    assert!(app.on_validated_ledger(Arc::clone(&reference)));
    assert_eq!(
        reference.hash_of_seq(5, &ledger::NullLedgerJournal),
        None,
        "fixture: the validated ledger cannot name ledger 5"
    );

    let source = rpc::ApplicationServerInfo::new(&app);
    let lookup = |hash: Uint256| rpc::LedgerLookupLedger {
        hash,
        seq: 5,
        open: false,
    };
    assert!(rpc::LedgerLookupSource::is_validated(
        &source,
        &lookup(*older.header().hash.as_uint256())
    ));
    assert!(
        !rpc::LedgerLookupSource::is_validated(&source, &lookup(Uint256::from_array([0x5A; 32]))),
        "another ledger at a validated sequence is not validated"
    );
}
