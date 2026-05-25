//! Integration tests that pin the system-family `invokePreflight` dispatch
//! shell to the current C++ ordering and transaction-type routing.

use std::cell::{Cell, RefCell};

use protocol::{Ter, TxType};
use tx::system_invoke_preflight::{
    run_system_invoke_preflight_for_txn_source_with_consequences,
    run_system_invoke_preflight_for_txn_type_with_consequences,
};
use tx::system_make_tx_consequences::run_system_make_tx_consequences_entrypoint_for_txn_type;
use tx::{
    HasTxnType, SystemTxnType, TxConsequences, classify_system_txn_type,
    run_change_invoke_preflight_for_txn_source, run_change_invoke_preflight_for_txn_type,
    run_system_invoke_preflight_for_txn_source, run_system_invoke_preflight_for_txn_type,
    run_with_system_txn_type_key, run_with_system_txn_type_source,
};

struct StubTx {
    txn_type: TxType,
}

impl HasTxnType for StubTx {
    fn txn_type(&self) -> TxType {
        self.txn_type
    }
}

#[test]
fn system_invoke_preflight_short_circuits_feature_gate_before_other_steps() {
    let trace = RefCell::new(Vec::new());

    let result = run_system_invoke_preflight_for_txn_type(
        TxType::BATCH,
        |system_type| {
            trace.borrow_mut().push(format!("feature:{system_type:?}"));
            false
        },
        |_| panic!("feature gate should skip extra-features"),
        |_| panic!("feature gate should skip flags mask"),
        |_| panic!("feature gate should skip preflight1"),
        |_| panic!("feature gate should skip selected preflight"),
        || panic!("feature gate should skip preflight2"),
        || panic!("feature gate should skip preflightSigValidated"),
    );

    assert_eq!(result, Ok(Ter::TEM_DISABLED));
    assert_eq!(trace.into_inner(), vec!["feature:Batch"]);
}

#[test]
fn system_invoke_preflight_preserves_current_cpp_step_order_for_batch() {
    let trace = RefCell::new(Vec::new());

    let result = run_system_invoke_preflight_for_txn_type(
        TxType::BATCH,
        |system_type| {
            trace.borrow_mut().push(format!("feature:{system_type:?}"));
            true
        },
        |system_type| {
            trace.borrow_mut().push(format!("extra:{system_type:?}"));
            true
        },
        |system_type| {
            trace.borrow_mut().push(format!("flags:{system_type:?}"));
            0x3ffc_ffff
        },
        |mask| {
            trace.borrow_mut().push(format!("preflight1:{mask:#x}"));
            assert_eq!(mask, 0x3ffc_ffff);
            Ter::TES_SUCCESS
        },
        |system_type| {
            trace.borrow_mut().push(format!("dispatch:{system_type:?}"));
            assert_eq!(system_type, SystemTxnType::Batch);
            Ter::TES_SUCCESS
        },
        || {
            trace.borrow_mut().push("preflight2".to_string());
            Ter::TES_SUCCESS
        },
        || {
            trace.borrow_mut().push("sigvalidated".to_string());
            Ter::TES_SUCCESS
        },
    );

    assert_eq!(result, Ok(Ter::TES_SUCCESS));
    assert_eq!(
        trace.into_inner(),
        vec![
            "feature:Batch",
            "extra:Batch",
            "flags:Batch",
            "preflight1:0x3ffcffff",
            "dispatch:Batch",
            "preflight2",
            "sigvalidated",
        ]
    );
}

#[test]
fn system_invoke_preflight_dispatches_to_ticket_create_and_ledger_state_fix() {
    let ticket_trace = RefCell::new(Vec::new());
    let ticket_result = run_system_invoke_preflight_for_txn_type(
        TxType::TICKET_CREATE,
        |_| panic!("ticket create should not consult a feature gate"),
        |system_type| {
            ticket_trace
                .borrow_mut()
                .push(format!("extra:{system_type:?}"));
            true
        },
        |system_type| {
            ticket_trace
                .borrow_mut()
                .push(format!("flags:{system_type:?}"));
            0x3fff_ffff
        },
        |mask| {
            ticket_trace
                .borrow_mut()
                .push(format!("preflight1:{mask:#x}"));
            assert_eq!(mask, 0x3fff_ffff);
            Ter::TES_SUCCESS
        },
        |system_type| {
            ticket_trace
                .borrow_mut()
                .push(format!("dispatch:{system_type:?}"));
            assert_eq!(system_type, SystemTxnType::TicketCreate);
            Ter::TES_SUCCESS
        },
        || {
            ticket_trace.borrow_mut().push("preflight2".to_string());
            Ter::TES_SUCCESS
        },
        || {
            ticket_trace.borrow_mut().push("sigvalidated".to_string());
            Ter::TES_SUCCESS
        },
    );

    let ledger_trace = RefCell::new(Vec::new());
    let ledger_result = run_system_invoke_preflight_for_txn_type(
        TxType::LEDGER_STATE_FIX,
        |system_type| {
            ledger_trace
                .borrow_mut()
                .push(format!("feature:{system_type:?}"));
            true
        },
        |system_type| {
            ledger_trace
                .borrow_mut()
                .push(format!("extra:{system_type:?}"));
            true
        },
        |system_type| {
            ledger_trace
                .borrow_mut()
                .push(format!("flags:{system_type:?}"));
            0x3fff_ffff
        },
        |mask| {
            ledger_trace
                .borrow_mut()
                .push(format!("preflight1:{mask:#x}"));
            assert_eq!(mask, 0x3fff_ffff);
            Ter::TES_SUCCESS
        },
        |system_type| {
            ledger_trace
                .borrow_mut()
                .push(format!("dispatch:{system_type:?}"));
            assert_eq!(system_type, SystemTxnType::LedgerStateFix);
            Ter::TES_SUCCESS
        },
        || {
            ledger_trace.borrow_mut().push("preflight2".to_string());
            Ter::TES_SUCCESS
        },
        || {
            ledger_trace.borrow_mut().push("sigvalidated".to_string());
            Ter::TES_SUCCESS
        },
    );

    assert_eq!(ticket_result, Ok(Ter::TES_SUCCESS));
    assert_eq!(
        ticket_trace.into_inner(),
        vec![
            "extra:TicketCreate",
            "flags:TicketCreate",
            "preflight1:0x3fffffff",
            "dispatch:TicketCreate",
            "preflight2",
            "sigvalidated",
        ]
    );

    assert_eq!(ledger_result, Ok(Ter::TES_SUCCESS));
    assert_eq!(
        ledger_trace.into_inner(),
        vec![
            "feature:LedgerStateFix",
            "extra:LedgerStateFix",
            "flags:LedgerStateFix",
            "preflight1:0x3fffffff",
            "dispatch:LedgerStateFix",
            "preflight2",
            "sigvalidated",
        ]
    );
}

#[test]
fn system_invoke_preflight_rejects_unknown_and_non_system_types() {
    assert_eq!(classify_system_txn_type(TxType::PAYMENT), None);
    assert_eq!(
        run_with_system_txn_type_key(TxType::PAYMENT, |_| Ter::TES_SUCCESS),
        Err(tx::UnknownTransactionType::new(TxType::PAYMENT))
    );
    assert_eq!(
        run_system_invoke_preflight_for_txn_type(
            TxType::PAYMENT,
            |_| panic!("unknown transaction type should not consult feature gates"),
            |_| panic!("unknown transaction type should not consult extra-features"),
            |_| panic!("unknown transaction type should not consult flags mask"),
            |_| panic!("unknown transaction type should not reach preflight1"),
            |_| panic!("unknown transaction type should not reach dispatch"),
            || panic!("unknown transaction type should not reach preflight2"),
            || panic!("unknown transaction type should not reach preflightSigValidated"),
        ),
        Err(tx::UnknownTransactionType::new(TxType::PAYMENT))
    );
}

#[test]
fn system_invoke_preflight_source_wrapper_uses_txn_type_from_source() {
    let tx = StubTx {
        txn_type: TxType::TICKET_CREATE,
    };
    let feature_called = Cell::new(false);

    let observed = run_system_invoke_preflight_for_txn_source(
        &tx,
        |_| panic!("ticket create should not consult a feature gate"),
        |system_type| {
            assert_eq!(system_type, SystemTxnType::TicketCreate);
            true
        },
        |system_type| {
            assert_eq!(system_type, SystemTxnType::TicketCreate);
            0x3fff_ffff
        },
        |mask| {
            assert_eq!(mask, 0x3fff_ffff);
            Ter::TES_SUCCESS
        },
        |system_type| {
            feature_called.set(true);
            assert_eq!(system_type, SystemTxnType::TicketCreate);
            Ter::TES_SUCCESS
        },
        || Ter::TES_SUCCESS,
        || Ter::TES_SUCCESS,
    );

    assert_eq!(observed, Ok(Ter::TES_SUCCESS));
    assert!(feature_called.get());

    let dispatch = run_with_system_txn_type_source(&tx, |system_type| system_type);
    assert_eq!(dispatch, Ok(SystemTxnType::TicketCreate));
}

#[test]
fn system_invoke_preflight_with_consequences_builds_success_consequences_only_on_success() {
    let trace = RefCell::new(Vec::new());
    let consequence_called = Cell::new(false);

    let observed = run_system_invoke_preflight_for_txn_type_with_consequences(
        TxType::TICKET_CREATE,
        |_| panic!("ticket create should not consult a feature gate"),
        |system_type| {
            trace.borrow_mut().push(format!("extra:{system_type:?}"));
            true
        },
        |system_type| {
            trace.borrow_mut().push(format!("flags:{system_type:?}"));
            0x3fff_ffff
        },
        |mask| {
            trace.borrow_mut().push(format!("preflight1:{mask:#x}"));
            assert_eq!(mask, 0x3fff_ffff);
            Ter::TES_SUCCESS
        },
        |system_type| {
            trace.borrow_mut().push(format!("dispatch:{system_type:?}"));
            assert_eq!(system_type, SystemTxnType::TicketCreate);
            Ter::TES_SUCCESS
        },
        || {
            trace.borrow_mut().push("preflight2".to_string());
            Ter::TES_SUCCESS
        },
        || {
            trace.borrow_mut().push("sigvalidated".to_string());
            Ter::TES_SUCCESS
        },
        |system_type| {
            consequence_called.set(true);
            assert_eq!(system_type, SystemTxnType::TicketCreate);
            run_system_make_tx_consequences_entrypoint_for_txn_type(
                TxType::TICKET_CREATE,
                12,
                protocol::SeqProxy::sequence(5),
                4,
            )
            .expect("ticket create should be known")
        },
    );

    assert_eq!(
        observed,
        Ok((
            Ter::TES_SUCCESS,
            run_system_make_tx_consequences_entrypoint_for_txn_type(
                TxType::TICKET_CREATE,
                12,
                protocol::SeqProxy::sequence(5),
                4,
            )
            .expect("ticket create should be known"),
        ))
    );
    assert!(consequence_called.get());
    assert_eq!(
        trace.into_inner(),
        vec![
            "extra:TicketCreate",
            "flags:TicketCreate",
            "preflight1:0x3fffffff",
            "dispatch:TicketCreate",
            "preflight2",
            "sigvalidated",
        ]
    );
}

#[test]
fn system_invoke_preflight_with_consequences_keeps_failure_consequences_without_success_builder() {
    let success_called = Cell::new(false);
    let trace = RefCell::new(Vec::new());

    let observed = run_system_invoke_preflight_for_txn_type_with_consequences(
        TxType::BATCH,
        |system_type| {
            trace.borrow_mut().push(format!("feature:{system_type:?}"));
            true
        },
        |system_type| {
            trace.borrow_mut().push(format!("extra:{system_type:?}"));
            true
        },
        |system_type| {
            trace.borrow_mut().push(format!("flags:{system_type:?}"));
            0x3ffc_ffff
        },
        |mask| {
            trace.borrow_mut().push(format!("preflight1:{mask:#x}"));
            assert_eq!(mask, 0x3ffc_ffff);
            Ter::TEM_INVALID_FLAG
        },
        |_system_type| panic!("failure should stop before selected dispatch"),
        || panic!("failure should stop before preflight2"),
        || panic!("failure should stop before sigvalidated"),
        |_system_type| {
            success_called.set(true);
            panic!("failure should not build success consequences");
        },
    );

    assert_eq!(
        observed,
        Ok((
            Ter::TEM_INVALID_FLAG,
            TxConsequences::from_preflight_result(Ter::TEM_INVALID_FLAG),
        ))
    );
    assert!(!success_called.get());
    assert_eq!(
        trace.into_inner(),
        vec![
            "feature:Batch",
            "extra:Batch",
            "flags:Batch",
            "preflight1:0x3ffcffff"
        ]
    );
}

#[test]
fn system_invoke_preflight_with_consequences_rejects_unknown_types() {
    let observed = run_system_invoke_preflight_for_txn_type_with_consequences(
        TxType::PAYMENT,
        |_| panic!("unknown type should not consult feature gates"),
        |_| panic!("unknown type should not consult extra-features"),
        |_| panic!("unknown type should not consult flags mask"),
        |_| panic!("unknown type should not reach preflight1"),
        |_| panic!("unknown type should not reach dispatch"),
        || panic!("unknown type should not reach preflight2"),
        || panic!("unknown type should not reach sigvalidated"),
        |_| panic!("unknown type should not build success consequences"),
    );

    assert_eq!(
        observed,
        Err(tx::UnknownTransactionType::new(TxType::PAYMENT))
    );
}

#[test]
fn system_invoke_preflight_source_with_consequences_uses_type_from_source() {
    let tx = StubTx {
        txn_type: TxType::TICKET_CREATE,
    };
    let consequence_called = Cell::new(false);

    let observed = run_system_invoke_preflight_for_txn_source_with_consequences(
        &tx,
        |_| panic!("ticket create should not consult a feature gate"),
        |system_type| {
            assert_eq!(system_type, SystemTxnType::TicketCreate);
            true
        },
        |system_type| {
            assert_eq!(system_type, SystemTxnType::TicketCreate);
            0x3fff_ffff
        },
        |mask| {
            assert_eq!(mask, 0x3fff_ffff);
            Ter::TES_SUCCESS
        },
        |system_type| {
            assert_eq!(system_type, SystemTxnType::TicketCreate);
            Ter::TES_SUCCESS
        },
        || Ter::TES_SUCCESS,
        || Ter::TES_SUCCESS,
        |system_type| {
            consequence_called.set(true);
            assert_eq!(system_type, SystemTxnType::TicketCreate);
            run_system_make_tx_consequences_entrypoint_for_txn_type(
                TxType::TICKET_CREATE,
                20,
                protocol::SeqProxy::ticket(11),
                2,
            )
            .expect("ticket create should be known")
        },
    );

    assert!(consequence_called.get());
    assert_eq!(
        observed,
        Ok((
            Ter::TES_SUCCESS,
            run_system_make_tx_consequences_entrypoint_for_txn_type(
                TxType::TICKET_CREATE,
                20,
                protocol::SeqProxy::ticket(11),
                2,
            )
            .expect("ticket create should be known"),
        ))
    );
}

#[test]
fn change_invoke_preflight_preserves_specialization_order() {
    let trace = RefCell::new(Vec::new());

    let result = run_change_invoke_preflight_for_txn_type(
        TxType::AMENDMENT,
        true,
        || {
            trace.borrow_mut().push("mask".to_string());
            0x4000_0000
        },
        |mask| {
            trace.borrow_mut().push(format!("preflight0:{mask:#x}"));
            assert_eq!(mask, 0x4000_0000);
            Ter::TES_SUCCESS
        },
        || {
            trace.borrow_mut().push("change-preflight".to_string());
            Ter::TES_SUCCESS
        },
    );

    assert_eq!(result, Ok(Ter::TES_SUCCESS));
    assert_eq!(
        trace.into_inner(),
        vec!["mask", "preflight0:0x40000000", "change-preflight"]
    );
}

#[test]
fn change_invoke_preflight_skips_the_flag_mask_helper_when_lending_protocol_is_disabled() {
    let mask_called = Cell::new(false);

    let result = run_change_invoke_preflight_for_txn_source(
        &TxType::UNL_MODIFY,
        false,
        || {
            mask_called.set(true);
            0x4000_0000
        },
        |mask| {
            assert_eq!(mask, 0);
            Ter::TES_SUCCESS
        },
        || Ter::TES_SUCCESS,
    );

    assert_eq!(result, Ok(Ter::TES_SUCCESS));
    assert!(!mask_called.get());
}
