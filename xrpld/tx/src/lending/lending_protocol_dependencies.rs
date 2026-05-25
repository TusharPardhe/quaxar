//! Shared lending feature gate in the reference implementation.
//!
//! This ports the exact deterministic short-circuit shape of
//! `checkLendingProtocolDependencies(...)`.

pub fn run_check_lending_protocol_dependencies(
    single_asset_vault_enabled: bool,
    check_vault_create_extra_features: impl FnOnce() -> bool,
) -> bool {
    single_asset_vault_enabled && check_vault_create_extra_features()
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use super::run_check_lending_protocol_dependencies;

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
}
