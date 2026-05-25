//! Integration tests that pin the narrowed Rust `DelegateUtils.cpp` helpers to
//! the current C++ behavior.

use std::collections::BTreeSet;

use protocol::{Ter, trans_token};
use tx::{
    load_granular_permissions, permission_to_tx_type, run_check_tx_permission,
    tx_to_permission_type,
};

#[test]
fn delegate_utils_check_tx_permission() {
    let allowed = run_check_tx_permission(Some(&[1, tx_to_permission_type(8), 65_540]), 8);
    let denied = run_check_tx_permission(Some(&[1, 10, 65_540]), 8);
    let missing = run_check_tx_permission(None, 8);

    assert_eq!(allowed, Ter::TES_SUCCESS);
    assert_eq!(denied, Ter::TER_NO_DELEGATE_PERMISSION);
    assert_eq!(missing, Ter::TER_NO_DELEGATE_PERMISSION);
    assert_eq!(trans_token(denied), "terNO_DELEGATE_PERMISSION");
}

#[test]
fn delegate_utils_load_granular_permission_filtering() {
    let loaded =
        load_granular_permissions(
            Some(&[65_540, 65_541, 3]),
            7_u16,
            |permission| match permission {
                65_540 => Some(7),
                65_541 => Some(8),
                _ => None,
            },
        );

    assert_eq!(loaded, BTreeSet::from([65_540]));
    assert_eq!(permission_to_tx_type(8), 7);
}
