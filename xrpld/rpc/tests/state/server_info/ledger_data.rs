use app::{ApplicationRoot, ApplicationRootOptions};
use basics::base_uint::{Uint160, Uint256};
use ledger::{Ledger, StateBatchOp};
use protocol::{
    AccountID, LedgerEntryType, STLedgerEntry, Serializer, StBase, account_keylet,
    get_field_by_symbol,
};
use rpc::{ApplicationServerInfo, LedgerDataSource, ledger_lookup};
use std::sync::Arc;

#[test]
fn application_server_info_resolve_ledger_data_decrements_marker() {
    let app =
        ApplicationRoot::with_options(ApplicationRootOptions::default()).expect("app should build");

    let mut ledger = Ledger::from_ledger_seq_and_close_time(512, 1000, false);

    // Create entries
    let mut entries = Vec::new();
    let mut mutations = Vec::new();

    for i in 1..=5u8 {
        let account = AccountID::from_slice(&[i; 20]).expect("valid account");
        let keylet = account_keylet(Uint160::from_slice(account.data()).expect("account width"));

        let mut sle = STLedgerEntry::new(keylet);
        sle.set_account_id(get_field_by_symbol("sfAccount"), account);

        let mut serializer = Serializer::new(1024);
        sle.add(&mut serializer);

        entries.push(sle);
        mutations.push((
            StateBatchOp::Insert,
            keylet.key,
            serializer.get_data().to_vec(),
        ));
    }

    ledger
        .apply_state_batch(&mutations)
        .expect("batch should apply");

    let ledger = Arc::new(ledger);
    app.on_validated_ledger(Arc::clone(&ledger));

    let source =
        ApplicationServerInfo::new(rpc::OwnedApplicationServerInfo::from_application_root(&app));
    let lookup_ledger = ledger_lookup::LedgerLookupLedger {
        hash: *ledger.header().hash.as_uint256(),
        seq: ledger.header().seq,
        open: false,
    };

    // Pagination query
    let resolved = source
        .resolve_ledger_data(&lookup_ledger, false, None, 2, LedgerEntryType::Any)
        .expect("should resolve");

    assert_eq!(resolved.entries.len(), 2);
    let marker = resolved.marker.expect("should have marker");

    let mut keys: Vec<Uint256> = entries.into_iter().map(|e| *e.key()).collect();
    keys.sort();

    let mut expected_marker = keys[2];
    expected_marker.decrement();
    assert_eq!(marker, expected_marker);
}
