//! Static `VaultSet` transactor feature gate in the reference implementation.
//!
//! This ports the exact deterministic behavior around:
//!
//! - accepting all transactions without `sfDomainID`, and
//! - requiring `featurePermissionedDomains` only when `sfDomainID` is present.

pub fn run_vault_set_check_extra_features(
    domain_id_present: bool,
    permissioned_domains_enabled: impl FnOnce() -> bool,
) -> bool {
    !domain_id_present || permissioned_domains_enabled()
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use super::run_vault_set_check_extra_features;

    #[test]
    fn vault_set_check_extra_features_skips_domain_gate_without_domain() {
        let checked = Cell::new(false);

        let result = run_vault_set_check_extra_features(false, || {
            checked.set(true);
            false
        });

        assert!(result);
        assert!(!checked.get());
    }

    #[test]
    fn vault_set_check_extra_features_requires_permissioned_domains_for_domain_updates() {
        assert!(run_vault_set_check_extra_features(true, || true));
        assert!(!run_vault_set_check_extra_features(true, || false));
    }
}
