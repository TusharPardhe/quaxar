use app::{
    APP_OPEN_LEDGER_DEFAULT_BASE_FEE_DROPS, AppOpenLedgerTxRecord, ApplicationRoot, ServiceRegistry,
};
use protocol::{STAmount, STTx, TxType, get_field_by_symbol};

fn payment_tx(sequence: u32, fill: u8) -> std::sync::Arc<STTx> {
    std::sync::Arc::new(STTx::new(TxType::PAYMENT, |tx| {
        let signing_pub_key = [fill; 33];
        tx.set_field_vl(get_field_by_symbol("sfSigningPubKey"), &signing_pub_key);
        tx.set_field_amount(
            get_field_by_symbol("sfFee"),
            STAmount::new_native(10, false),
        );
        tx.set_field_u32(get_field_by_symbol("sfSequence"), sequence);
    }))
}

#[test]
fn application_root_service_registry_owns_real_open_ledger_and_txq() {
    let root = ApplicationRoot::new(0).expect("root shell should build");
    let open_ledger = ServiceRegistry::get_open_ledger(&root);
    let open_ledger_const = ServiceRegistry::get_open_ledger_const(&root);
    let tx_q = ServiceRegistry::get_tx_q(&root);

    assert!(std::ptr::eq(open_ledger, open_ledger_const));
    assert!(open_ledger.empty::<AppOpenLedgerTxRecord>());
    assert_eq!(open_ledger.current().ledger_current_index, 0);
    assert_eq!(
        open_ledger.current().base_fee_drops,
        APP_OPEN_LEDGER_DEFAULT_BASE_FEE_DROPS
    );
    assert_eq!(tx_q.current_max_size(), None);

    let metrics = root.tx_q_metrics();
    assert_eq!(metrics.tx_in_ledger, 0);
    assert_eq!(metrics.tx_count, 0);
}

#[test]
fn application_root_txq_report_reads_current_open_ledger_snapshot() {
    let root = ApplicationRoot::new(0).expect("root shell should build");

    let changed = ServiceRegistry::get_open_ledger(&root).modify(|next| {
        next.ledger_current_index = 712;
        next.base_fee_drops = 17;
        next.push_transaction(payment_tx(1, 1));
        next.push_transaction(payment_tx(2, 2));
        true
    });
    assert!(changed);

    let snapshot = ServiceRegistry::get_open_ledger(&root).current();
    let expected_ids: Vec<_> = vec![payment_tx(1, 1), payment_tx(2, 2)]
        .iter()
        .map(|tx| tx.get_transaction_id())
        .collect();
    assert_eq!(snapshot.tx_ids(), expected_ids);

    let metrics = root.tx_q_metrics();
    assert_eq!(metrics.tx_in_ledger, 2);
    assert_eq!(metrics.tx_count, 0);
    assert_eq!(metrics.tx_q_max_size, None);

    let report = root.tx_q_rpc_report();
    assert_eq!(report.ledger_current_index, 712);
    assert_eq!(report.current_ledger_size, "2");
    assert_eq!(report.current_queue_size, "0");
    assert_eq!(report.max_queue_size, None);
    assert_eq!(report.drops.base_fee, "17");
}
