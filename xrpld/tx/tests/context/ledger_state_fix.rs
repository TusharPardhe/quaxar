//! Integration tests that pin the narrowed Rust `LedgerStateFix.cpp`
//! transactor shell to the current C++ behavior.

use std::cell::Cell;

use protocol::{Ter, trans_token};
use tx::{
    LedgerStateFixType, run_ledger_state_fix_do_apply, run_ledger_state_fix_preclaim,
    run_ledger_state_fix_preflight,
};

#[test]
fn ledger_state_fix_preflight_requires_owner_for_nft_page_link() {
    assert_eq!(
        run_ledger_state_fix_preflight(LedgerStateFixType::NfTokenPageLink, false),
        Ter::TEM_INVALID
    );
    assert_eq!(
        run_ledger_state_fix_preflight(LedgerStateFixType::NfTokenPageLink, true),
        Ter::TES_SUCCESS
    );
}

#[test]
fn ledger_state_fix_preflight_rejects_unknown_fix_type_codes() {
    let zero = run_ledger_state_fix_preflight(LedgerStateFixType::from(0), true);
    let two_hundred = run_ledger_state_fix_preflight(LedgerStateFixType::from(200), true);

    assert_eq!(zero, Ter::TEF_INVALID_LEDGER_FIX_TYPE);
    assert_eq!(two_hundred, Ter::TEF_INVALID_LEDGER_FIX_TYPE);
    assert_eq!(trans_token(zero), "tefINVALID_LEDGER_FIX_TYPE");
}

#[test]
fn ledger_state_fix_preclaim_requires_owner_account() {
    let missing = run_ledger_state_fix_preclaim(LedgerStateFixType::NfTokenPageLink, false);
    let present = run_ledger_state_fix_preclaim(LedgerStateFixType::NfTokenPageLink, true);

    assert_eq!(missing, Ter::TEC_OBJECT_NOT_FOUND);
    assert_eq!(present, Ter::TES_SUCCESS);
}

#[test]
fn ledger_state_fix_do_apply_maps_repair_result() {
    let called = Cell::new(false);
    let success = run_ledger_state_fix_do_apply(LedgerStateFixType::NfTokenPageLink, || {
        called.set(true);
        true
    });
    let failure = run_ledger_state_fix_do_apply(LedgerStateFixType::NfTokenPageLink, || false);

    assert!(called.get());
    assert_eq!(success, Ter::TES_SUCCESS);
    assert_eq!(failure, Ter::TEC_FAILED_PROCESSING);
    assert_eq!(trans_token(failure), "tecFAILED_PROCESSING");
}
