//! Integration tests that pin the public system-family preflight composition
//! shell to the current C++ exception, unknown-type, and success-consequence
//! behavior.

use protocol::{Rules, SeqProxy, Ter, TxType};
use tx::system_invoke_preflight::{
    SystemTxnType, run_system_invoke_preflight_for_txn_type_with_consequences,
};
use tx::system_make_tx_consequences::run_system_make_tx_consequences_entrypoint_for_txn_type;
use tx::system_preflight_entrypoint::{
    run_system_preflight_for_txn_source, run_system_preflight_for_txn_source_with_consequences,
    run_system_preflight_for_txn_type, run_system_preflight_for_txn_type_with_consequences,
    run_system_preflight_with_context,
};
use tx::{ApplyFlags, HasTxnType, PreflightContext, TxConsequences, UNKNOWN_TRANSACTION_TYPE_TER};

fn empty_rules() -> Rules {
    Rules::new(std::iter::empty())
}

struct StubTx {
    txn_type: TxType,
}

impl HasTxnType for StubTx {
    fn txn_type(&self) -> TxType {
        self.txn_type
    }
}

#[test]
fn system_preflight_with_context_maps_exception_to_tefexception_and_uses_fallback() {
    let ctx = PreflightContext::<_, _, _, ()>::new(
        "registry",
        "tx",
        empty_rules(),
        ApplyFlags::NONE,
        "journal",
    );

    let observed = run_system_preflight_with_context(
        ctx,
        |_ctx| Err::<(Ter, TxConsequences), &str>("boom"),
        |_ctx| TxConsequences::with_sequences_consumed(12, SeqProxy::sequence(5), 3),
    );

    assert_eq!(observed.ter, Ter::TEF_EXCEPTION);
    assert_eq!(
        observed.consequences,
        TxConsequences::with_sequences_consumed(12, SeqProxy::sequence(5), 3)
    );
}

#[test]
fn system_preflight_for_txn_type_with_consequences_keeps_unknown_types_on_temunknown() {
    let ctx = PreflightContext::<_, _, _, ()>::new(
        "registry",
        "tx",
        empty_rules(),
        ApplyFlags::NONE,
        "journal",
    );

    let observed = run_system_preflight_for_txn_type_with_consequences(
        ctx,
        TxType::PAYMENT,
        |_ctx, _txn_type| -> Result<Ter, &str> {
            panic!("unknown type should not reach preflight")
        },
        |_ctx, _txn_type| -> TxConsequences {
            panic!("unknown type should not build success consequences")
        },
        |_ctx| TxConsequences::with_sequences_consumed(14, SeqProxy::sequence(7), 9),
    );

    assert_eq!(observed.ter, UNKNOWN_TRANSACTION_TYPE_TER);
    assert_eq!(
        observed.consequences,
        TxConsequences::from_preflight_result(UNKNOWN_TRANSACTION_TYPE_TER)
    );
}

#[test]
fn system_preflight_for_txn_source_with_consequences_composes_system_helpers() {
    let tx = StubTx {
        txn_type: TxType::TICKET_CREATE,
    };
    let ctx = PreflightContext::<_, _, _, ()>::new(
        "registry",
        tx,
        empty_rules(),
        ApplyFlags::NONE,
        "journal",
    );

    let observed = run_system_preflight_for_txn_source_with_consequences(
        ctx,
        |_ctx, txn_type| {
            assert_eq!(txn_type, TxType::TICKET_CREATE);

            let (ter, consequences) = run_system_invoke_preflight_for_txn_type_with_consequences(
                txn_type,
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
                    assert_eq!(system_type, SystemTxnType::TicketCreate);
                    run_system_make_tx_consequences_entrypoint_for_txn_type(
                        TxType::TICKET_CREATE,
                        20,
                        SeqProxy::ticket(11),
                        2,
                    )
                    .expect("ticket create should be known")
                },
            )
            .expect("ticket create should be known");

            assert_eq!(
                consequences,
                run_system_make_tx_consequences_entrypoint_for_txn_type(
                    TxType::TICKET_CREATE,
                    20,
                    SeqProxy::ticket(11),
                    2,
                )
                .expect("ticket create should be known")
            );
            assert_eq!(ter, Ter::TES_SUCCESS);
            Ok::<_, &str>(ter)
        },
        |_ctx, txn_type| {
            assert_eq!(txn_type, TxType::TICKET_CREATE);
            run_system_make_tx_consequences_entrypoint_for_txn_type(
                TxType::TICKET_CREATE,
                20,
                SeqProxy::ticket(11),
                2,
            )
            .expect("ticket create should be known")
        },
        |_ctx| panic!("successful path should not need fallback"),
    );

    assert_eq!(observed.ter, Ter::TES_SUCCESS);
    assert_eq!(
        observed.consequences,
        run_system_make_tx_consequences_entrypoint_for_txn_type(
            TxType::TICKET_CREATE,
            20,
            SeqProxy::ticket(11),
            2,
        )
        .expect("ticket create should be known")
    );
}

#[test]
fn system_public_preflight_routes_change_path_and_uses_normal_success_consequences() {
    let observed = run_system_preflight_for_txn_type(
        PreflightContext::<_, _, _, ()>::new(
            "registry",
            "tx",
            empty_rules(),
            ApplyFlags::NONE,
            "journal",
        ),
        TxType::AMENDMENT,
        15,
        SeqProxy::sequence(8),
        99,
        true,
        || 0x20,
        |mask| {
            assert_eq!(mask, 0x20);
            Ter::TES_SUCCESS
        },
        || Ter::TES_SUCCESS,
        |_| panic!("change path should not consult system feature gate"),
        |_| panic!("change path should not consult system extra-features"),
        |_| panic!("change path should not consult system flags"),
        |_| panic!("change path should not consult system preflight1"),
        |_| panic!("change path should not consult system typed preflight"),
        || panic!("change path should not consult system preflight2"),
        || panic!("change path should not consult sigvalidated"),
    );

    assert_eq!(observed.ter, Ter::TES_SUCCESS);
    assert_eq!(
        observed.consequences,
        TxConsequences::new(15, SeqProxy::sequence(8))
    );
}

#[test]
fn system_public_preflight_routes_ticket_create_and_preserves_batch_context() {
    let observed = run_system_preflight_for_txn_source(
        PreflightContext::new_batch(
            "registry",
            StubTx {
                txn_type: TxType::TICKET_CREATE,
            },
            "batch-1",
            empty_rules(),
            ApplyFlags::BATCH,
            "journal",
        ),
        17,
        SeqProxy::ticket(4),
        3,
        false,
        || panic!("ticket create should not use change flags"),
        |_| panic!("ticket create should not use preflight0"),
        || panic!("ticket create should not use change preflight"),
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
    );

    assert_eq!(observed.parent_batch_id, Some("batch-1"));
    assert_eq!(observed.flags, ApplyFlags::BATCH);
    assert_eq!(observed.ter, Ter::TES_SUCCESS);
    assert_eq!(
        observed.consequences,
        TxConsequences::with_sequences_consumed(17, SeqProxy::ticket(4), 3)
    );
}

#[test]
fn system_public_preflight_maps_non_system_type_to_temunknown() {
    let observed = run_system_preflight_for_txn_type(
        PreflightContext::<_, _, _, ()>::new(
            "registry",
            "tx",
            empty_rules(),
            ApplyFlags::NONE,
            "journal",
        ),
        TxType::PAYMENT,
        11,
        SeqProxy::sequence(5),
        2,
        false,
        || panic!("unknown type should not use change flags"),
        |_| panic!("unknown type should not use preflight0"),
        || panic!("unknown type should not use change preflight"),
        |_| panic!("unknown type should not use system feature gate"),
        |_| panic!("unknown type should not use system extra-features"),
        |_| panic!("unknown type should not use system flags"),
        |_| panic!("unknown type should not use system preflight1"),
        |_| panic!("unknown type should not use system typed preflight"),
        || panic!("unknown type should not use system preflight2"),
        || panic!("unknown type should not use sigvalidated"),
    );

    assert_eq!(observed.ter, UNKNOWN_TRANSACTION_TYPE_TER);
    assert_eq!(
        observed.consequences,
        TxConsequences::from_preflight_result(UNKNOWN_TRANSACTION_TYPE_TER)
    );
}
