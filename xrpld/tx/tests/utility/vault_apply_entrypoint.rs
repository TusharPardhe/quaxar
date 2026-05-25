//! Integration tests that pin the public vault-family apply composition shell
//! to the current C++ stale-ledger, exception, and unknown-type behavior.

use std::cell::RefCell;

use basics::base_uint::Uint256;
use protocol::{Rules, Ter, TxType};
use tx::vault_apply_entrypoint::{run_vault_apply_for_txn_source, run_vault_apply_for_txn_type};
use tx::{ApplyFlags, ApplyResult, HasTxnType, PreclaimResult};

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
fn vault_apply_routes_vault_types() {
    let trace = RefCell::new(Vec::new());
    let preclaim_result = PreclaimResult::new(
        9,
        StubTx {
            txn_type: TxType::VAULT_DELETE,
        },
        None::<()>,
        ApplyFlags::NONE,
        "journal",
        Ter::TES_SUCCESS,
    );

    let result = run_vault_apply_for_txn_source(
        preclaim_result,
        &rules(0x11),
        "registry",
        9,
        "base",
        "view",
        |_base, _tx, txn_type| {
            assert_eq!(txn_type, TxType::VAULT_DELETE);
            20_u64
        },
        || 0_u64,
        |ctx, txn_type| {
            trace.borrow_mut().push("dispatch_apply");
            assert_eq!(ctx.preclaim_result, Ter::TES_SUCCESS);
            assert_eq!(txn_type, TxType::VAULT_DELETE);
            Ok::<ApplyResult, &str>(ApplyResult::new(Ter::TES_SUCCESS, true, false))
        },
    );

    assert_eq!(result, ApplyResult::new(Ter::TES_SUCCESS, true, false));
    assert_eq!(trace.into_inner(), vec!["dispatch_apply"]);
}

#[test]
fn vault_apply_maps_non_vault_types_to_temunknown() {
    let trace = RefCell::new(Vec::new());
    let preclaim_result = PreclaimResult::new(
        9,
        StubTx {
            txn_type: TxType::PAYMENT,
        },
        None::<()>,
        ApplyFlags::NONE,
        "journal",
        Ter::TES_SUCCESS,
    );

    let result = run_vault_apply_for_txn_type(
        preclaim_result,
        TxType::PAYMENT,
        &rules(0x22),
        "registry",
        9,
        "base",
        "view",
        |_base, _tx, txn_type| {
            trace.borrow_mut().push("base-fee");
            assert_eq!(txn_type, TxType::PAYMENT);
            0_u64
        },
        || 0_u64,
        |_ctx, _txn_type| -> Result<ApplyResult, &str> {
            panic!("non-vault tx should not reach dispatch")
        },
    );

    assert_eq!(
        result,
        ApplyResult::new(protocol::Ter::TEM_UNKNOWN, false, false)
    );
    assert_eq!(trace.into_inner(), vec!["base-fee"]);
}

#[test]
fn vault_apply_maps_dispatch_errors_to_tefexception() {
    let preclaim_result = PreclaimResult::new(
        9,
        StubTx {
            txn_type: TxType::VAULT_CLAWBACK,
        },
        None::<()>,
        ApplyFlags::NONE,
        "journal",
        Ter::TES_SUCCESS,
    );

    let result = run_vault_apply_for_txn_type(
        preclaim_result,
        TxType::VAULT_CLAWBACK,
        &rules(0x33),
        "registry",
        9,
        "base",
        "view",
        |_base, _tx, txn_type| {
            assert_eq!(txn_type, TxType::VAULT_CLAWBACK);
            20_u64
        },
        || 0_u64,
        |_ctx, _txn_type| Err::<ApplyResult, &str>("boom"),
    );

    assert_eq!(result, ApplyResult::new(Ter::TEF_EXCEPTION, false, false));
}
