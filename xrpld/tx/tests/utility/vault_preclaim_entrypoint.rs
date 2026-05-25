//! Integration tests that pin the public vault-family preclaim composition
//! shell to the current C++ reflight, exception, and unknown-type behavior.

use std::cell::RefCell;

use basics::base_uint::Uint256;
use protocol::{Rules, Ter, TxType};
use tx::vault_preclaim_entrypoint::{
    run_vault_preclaim_for_txn_source, run_vault_preclaim_for_txn_type,
};
use tx::{ApplyFlags, HasTxnType, PreflightResult, TxConsequences};

fn rules(seed: u8) -> Rules {
    Rules::from_ledger(
        [protocol::feature_single_asset_vault()],
        Uint256::from_array([seed; 32]),
        std::iter::empty(),
    )
}

#[derive(Clone)]
struct StubTx {
    txn_type: TxType,
}

impl HasTxnType for StubTx {
    fn txn_type(&self) -> TxType {
        self.txn_type
    }
}

#[test]
fn vault_preclaim_reruns_preflight_and_routes_vault_types() {
    let old_rules = rules(0x11);
    let new_rules = rules(0x22);
    let trace = RefCell::new(Vec::new());

    let preflight_result = PreflightResult::new(
        StubTx {
            txn_type: TxType::VAULT_CREATE,
        },
        None::<()>,
        old_rules,
        TxConsequences::new(12, protocol::SeqProxy::sequence(5)),
        ApplyFlags::NONE,
        "journal",
        Ter::TES_SUCCESS,
    );

    let result = run_vault_preclaim_for_txn_source(
        preflight_result,
        "registry",
        "view",
        &new_rules,
        9,
        |preflight_result, current_rules| {
            trace.borrow_mut().push("rerun_preflight");
            assert_ne!(preflight_result.rules, *current_rules);
            PreflightResult::new(
                StubTx {
                    txn_type: TxType::VAULT_CREATE,
                },
                None::<()>,
                current_rules.clone(),
                TxConsequences::new(12, protocol::SeqProxy::sequence(5)),
                ApplyFlags::NONE,
                "journal",
                Ter::TES_SUCCESS,
            )
        },
        |ctx, txn_type| {
            trace.borrow_mut().push("dispatch_preclaim");
            assert_eq!(ctx.preflight_result, Ter::TES_SUCCESS);
            assert_eq!(txn_type, TxType::VAULT_CREATE);
            Ok::<Ter, &str>(Ter::TES_SUCCESS)
        },
    );

    assert_eq!(result.ter, Ter::TES_SUCCESS);
    assert_eq!(result.ledger_seq, 9);
    assert_eq!(
        trace.into_inner(),
        vec!["rerun_preflight", "dispatch_preclaim"]
    );
}

#[test]
fn vault_preclaim_maps_non_vault_types_to_temunknown() {
    let preflight_result = PreflightResult::new(
        StubTx {
            txn_type: TxType::PAYMENT,
        },
        None::<()>,
        rules(0x33),
        TxConsequences::new(12, protocol::SeqProxy::sequence(5)),
        ApplyFlags::NONE,
        "journal",
        Ter::TES_SUCCESS,
    );

    let result = run_vault_preclaim_for_txn_type(
        preflight_result,
        TxType::PAYMENT,
        "registry",
        "view",
        &rules(0x33),
        9,
        |_preflight_result, _current_rules| panic!("rules match should skip reflight"),
        |_ctx, _txn_type| -> Result<Ter, &str> { panic!("non-vault tx should not reach dispatch") },
    );

    assert_eq!(result.ter, protocol::Ter::TEM_UNKNOWN);
}

#[test]
fn vault_preclaim_maps_dispatch_errors_to_tefexception() {
    let preflight_result = PreflightResult::new(
        StubTx {
            txn_type: TxType::VAULT_WITHDRAW,
        },
        None::<()>,
        rules(0x44),
        TxConsequences::new(12, protocol::SeqProxy::ticket(7)),
        ApplyFlags::NONE,
        "journal",
        Ter::TES_SUCCESS,
    );

    let result = run_vault_preclaim_for_txn_type(
        preflight_result,
        TxType::VAULT_WITHDRAW,
        "registry",
        "view",
        &rules(0x44),
        9,
        |_preflight_result, _current_rules| panic!("rules match should skip reflight"),
        |_ctx, _txn_type| Err::<Ter, &str>("boom"),
    );

    assert_eq!(result.ter, Ter::TEF_EXCEPTION);
}
