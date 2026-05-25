//! Integration tests that pin the narrowed Rust shared lending feature gate to
//! the current C++ behavior.

use std::cell::Cell;

use tx::run_check_lending_protocol_dependencies;

#[test]
fn lending_protocol_dependencies_short_circuit_when_single_asset_vault_is_disabled() {
    let vault_helper_called = Cell::new(false);

    let result = run_check_lending_protocol_dependencies(false, || {
        vault_helper_called.set(true);
        true
    });

    assert!(!result);
    assert!(!vault_helper_called.get());
}

#[test]
fn lending_protocol_dependencies_return_vault_helper_result() {
    assert!(run_check_lending_protocol_dependencies(true, || true));
    assert!(!run_check_lending_protocol_dependencies(true, || false));
}
