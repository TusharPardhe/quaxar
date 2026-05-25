//! Integration tests that pin the system-family `run_system_apply(...)` shell
//! to the current C++ transaction-type routing and unknown fallback.

use protocol::{Rules, Ter, TxType};
use tx::{ApplyFlags, ApplyResult, PreclaimResult, run_system_apply_for_txn_type};

#[test]
fn system_apply_routes_system_types() {
    let rules = Rules::default();
    let preclaim_result = PreclaimResult {
        ter: Ter::TES_SUCCESS,
        tx: "tx".to_string(),
        journal: "journal".to_string(),
        ledger_seq: 7,
        parent_batch_id: None::<u32>,
        flags: ApplyFlags::NONE,
        likely_to_claim_fee: true,
    };

    let change = run_system_apply_for_txn_type(
        preclaim_result.clone(),
        TxType::AMENDMENT,
        &rules,
        "registry",
        7,
        "base",
        "view",
        |_, _, _| 10_u64,
        || 0_u64,
        |_, _| Ok::<ApplyResult, String>(ApplyResult::new(Ter::TES_SUCCESS, true, false)),
    );
    assert_eq!(change, ApplyResult::new(Ter::TES_SUCCESS, true, false));

    let batch = run_system_apply_for_txn_type(
        preclaim_result.clone(),
        TxType::BATCH,
        &rules,
        "registry",
        7,
        "base",
        "view",
        |_, _, _| 10_u64,
        || 0_u64,
        |_, _| Ok::<ApplyResult, String>(ApplyResult::new(Ter::TES_SUCCESS, true, true)),
    );
    assert_eq!(batch, ApplyResult::new(Ter::TES_SUCCESS, true, true));
}

#[test]
fn system_apply_maps_non_system_types_to_temunknown() {
    let rules = Rules::default();
    let preclaim_result = PreclaimResult {
        ter: Ter::TES_SUCCESS,
        tx: "tx".to_string(),
        journal: "journal".to_string(),
        ledger_seq: 7,
        parent_batch_id: None::<u32>,
        flags: ApplyFlags::NONE,
        likely_to_claim_fee: true,
    };

    let result = run_system_apply_for_txn_type(
        preclaim_result,
        TxType::ESCROW_CREATE,
        &rules,
        "registry",
        7,
        "base",
        "view",
        |_, _, _| 10_u64,
        || 0_u64,
        |_, _| -> Result<ApplyResult, String> { panic!("non-system tx should not reach dispatch") },
    );

    assert_eq!(result.ter, Ter::TEM_UNKNOWN);
}
