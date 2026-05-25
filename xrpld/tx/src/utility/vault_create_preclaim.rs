//! Deterministic
//! the reference implementation shell.
//!
//! This ports the exact helper-call order around:
//!
//! - `canAddHolding(...)`,
//! - pseudo-account issuer rejection for non-native assets,
//! - frozen-asset mapping to `tecFROZEN` versus `tecLOCKED`,
//! - optional permissioned-domain existence checks,
//! - and the final pseudo-account-address collision guard.

use protocol::{Ter, is_tes_success};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct VaultCreatePreclaimFacts {
    pub asset_is_native: bool,
    pub asset_is_issue: bool,
    pub domain_id_present: bool,
}

pub fn run_vault_create_preclaim<
    CanAddHolding,
    IsPseudoAccount,
    IsFrozen,
    DomainExists,
    PseudoAccountAddressIsZero,
>(
    facts: VaultCreatePreclaimFacts,
    can_add_holding: CanAddHolding,
    is_pseudo_account: IsPseudoAccount,
    is_frozen: IsFrozen,
    domain_exists: DomainExists,
    pseudo_account_address_is_zero: PseudoAccountAddressIsZero,
) -> Ter
where
    CanAddHolding: FnOnce() -> Ter,
    IsPseudoAccount: FnOnce() -> bool,
    IsFrozen: FnOnce() -> bool,
    DomainExists: FnOnce() -> bool,
    PseudoAccountAddressIsZero: FnOnce() -> bool,
{
    let ter = can_add_holding();
    if !is_tes_success(ter) {
        return ter;
    }

    if !facts.asset_is_native && is_pseudo_account() {
        return Ter::TEC_WRONG_ASSET;
    }

    if is_frozen() {
        return if facts.asset_is_issue {
            Ter::TEC_FROZEN
        } else {
            Ter::TEC_LOCKED
        };
    }

    if facts.domain_id_present && !domain_exists() {
        return Ter::TEC_OBJECT_NOT_FOUND;
    }

    if pseudo_account_address_is_zero() {
        return Ter::TER_ADDRESS_COLLISION;
    }

    Ter::TES_SUCCESS
}

#[cfg(test)]
mod tests {
    use std::{cell::Cell, rc::Rc};

    use protocol::{Ter, trans_token};

    use super::{VaultCreatePreclaimFacts, run_vault_create_preclaim};

    #[test]
    fn vault_create_preclaim_returns_can_add_holding_failure_first() {
        let pseudo_checked = Cell::new(false);

        let result = run_vault_create_preclaim(
            VaultCreatePreclaimFacts::default(),
            || Ter::TER_NO_RIPPLE,
            || {
                pseudo_checked.set(true);
                false
            },
            || false,
            || true,
            || false,
        );

        assert_eq!(result, Ter::TER_NO_RIPPLE);
        assert_eq!(trans_token(result), "terNO_RIPPLE");
        assert!(!pseudo_checked.get());
    }

    #[test]
    fn vault_create_preclaim_skips_pseudo_account_check_for_native_assets() {
        let pseudo_checked = Cell::new(false);

        let result = run_vault_create_preclaim(
            VaultCreatePreclaimFacts {
                asset_is_native: true,
                ..VaultCreatePreclaimFacts::default()
            },
            || Ter::TES_SUCCESS,
            || {
                pseudo_checked.set(true);
                true
            },
            || false,
            || true,
            || false,
        );

        assert_eq!(result, Ter::TES_SUCCESS);
        assert!(!pseudo_checked.get());
    }

    #[test]
    fn vault_create_preclaim_rejects_pseudo_account_issuer_for_non_native_assets() {
        let result = run_vault_create_preclaim(
            VaultCreatePreclaimFacts::default(),
            || Ter::TES_SUCCESS,
            || true,
            || false,
            || true,
            || false,
        );

        assert_eq!(result, Ter::TEC_WRONG_ASSET);
        assert_eq!(trans_token(result), "tecWRONG_ASSET");
    }

    #[test]
    fn vault_create_preclaim_maps_frozen_issue_to_tecfrozen() {
        let result = run_vault_create_preclaim(
            VaultCreatePreclaimFacts {
                asset_is_issue: true,
                ..VaultCreatePreclaimFacts::default()
            },
            || Ter::TES_SUCCESS,
            || false,
            || true,
            || true,
            || false,
        );

        assert_eq!(result, Ter::TEC_FROZEN);
    }

    #[test]
    fn vault_create_preclaim_maps_frozen_non_issue_to_teclocked() {
        let result = run_vault_create_preclaim(
            VaultCreatePreclaimFacts::default(),
            || Ter::TES_SUCCESS,
            || false,
            || true,
            || true,
            || false,
        );

        assert_eq!(result, Ter::TEC_LOCKED);
    }

    #[test]
    fn vault_create_preclaim_requires_existing_domain() {
        let result = run_vault_create_preclaim(
            VaultCreatePreclaimFacts {
                domain_id_present: true,
                ..VaultCreatePreclaimFacts::default()
            },
            || Ter::TES_SUCCESS,
            || false,
            || false,
            || false,
            || false,
        );

        assert_eq!(result, Ter::TEC_OBJECT_NOT_FOUND);
        assert_eq!(trans_token(result), "tecOBJECT_NOT_FOUND");
    }

    #[test]
    fn vault_create_preclaim_returns_address_collision() {
        let result = run_vault_create_preclaim(
            VaultCreatePreclaimFacts::default(),
            || Ter::TES_SUCCESS,
            || false,
            || false,
            || true,
            || true,
        );

        assert_eq!(result, Ter::TER_ADDRESS_COLLISION);
        assert_eq!(trans_token(result), "terADDRESS_COLLISION");
    }

    #[test]
    fn vault_create_preclaim_runs_helpers_in_current_on_success() {
        let seen = Rc::new(std::cell::RefCell::new(Vec::new()));

        let result = run_vault_create_preclaim(
            VaultCreatePreclaimFacts {
                domain_id_present: true,
                ..VaultCreatePreclaimFacts::default()
            },
            {
                let seen = Rc::clone(&seen);
                move || {
                    seen.borrow_mut().push("holding");
                    Ter::TES_SUCCESS
                }
            },
            {
                let seen = Rc::clone(&seen);
                move || {
                    seen.borrow_mut().push("pseudo");
                    false
                }
            },
            {
                let seen = Rc::clone(&seen);
                move || {
                    seen.borrow_mut().push("frozen");
                    false
                }
            },
            {
                let seen = Rc::clone(&seen);
                move || {
                    seen.borrow_mut().push("domain");
                    true
                }
            },
            {
                let seen = Rc::clone(&seen);
                move || {
                    seen.borrow_mut().push("address");
                    false
                }
            },
        );

        assert_eq!(result, Ter::TES_SUCCESS);
        assert_eq!(
            seen.borrow().as_slice(),
            ["holding", "pseudo", "frozen", "domain", "address"]
        );
    }
}
