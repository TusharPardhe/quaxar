//! Deterministic
//! the reference implementation shell.
//!
//! This ports the exact current guard order for:
//!
//! - missing vault lookup,
//! - asset mismatch,
//! - transferability failure,
//! - vault share versus vault asset internal invariant failure,
//! - missing or locked share issuance,
//! - frozen vault-asset and share checks,
//! - private-vault domain authorization with `tecEXPIRED` suppression,
//! - source authorization,
//! - and final sufficient-funds rejection.

use protocol::{Ter, is_tes_success};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct VaultDepositPreclaimFacts {
    pub vault_exists: bool,
    pub deposited_asset_matches_vault_asset: bool,
    pub vault_share_matches_vault_asset: bool,
    pub issuance_exists: bool,
    pub issuance_locked: bool,
    pub vault_asset_is_issue: bool,
    pub vault_asset_frozen_for_account: bool,
    pub vault_share_frozen_for_account: bool,
    pub fix_cleanup_3_3_0_enabled: bool,
    pub vault_is_private: bool,
    pub submitter_is_owner: bool,
    pub domain_id_present: bool,
    pub account_holds_sufficient_assets: bool,
}

pub fn run_vault_deposit_preclaim<CanTransfer, ValidDomain, RequireAuth, CheckDepositFreeze>(
    facts: VaultDepositPreclaimFacts,
    can_transfer: CanTransfer,
    valid_domain: ValidDomain,
    require_auth: RequireAuth,
    check_deposit_freeze: CheckDepositFreeze,
) -> Ter
where
    CanTransfer: FnOnce() -> Ter,
    ValidDomain: FnOnce() -> Ter,
    RequireAuth: FnOnce() -> Ter,
    CheckDepositFreeze: FnOnce() -> Ter,
{
    if !facts.vault_exists {
        return Ter::TEC_NO_ENTRY;
    }

    if !facts.deposited_asset_matches_vault_asset {
        return Ter::TEC_WRONG_ASSET;
    }

    let ter = can_transfer();
    if !is_tes_success(ter) {
        return ter;
    }

    if facts.vault_share_matches_vault_asset {
        return Ter::TEF_INTERNAL;
    }

    if !facts.issuance_exists {
        return Ter::TEF_INTERNAL;
    }

    if facts.issuance_locked {
        return Ter::TEF_INTERNAL;
    }

    if facts.fix_cleanup_3_3_0_enabled {
        let ter = check_deposit_freeze();
        if !is_tes_success(ter) {
            return ter;
        }
    } else {
        if facts.vault_asset_frozen_for_account {
            return if facts.vault_asset_is_issue {
                Ter::TEC_FROZEN
            } else {
                Ter::TEC_LOCKED
            };
        }

        if facts.vault_share_frozen_for_account {
            return Ter::TEC_LOCKED;
        }
    }

    if facts.vault_is_private && !facts.submitter_is_owner {
        if facts.domain_id_present {
            let err = valid_domain();
            if !is_tes_success(err) && err != Ter::TEC_EXPIRED {
                return err;
            }
        } else {
            return Ter::TEC_NO_AUTH;
        }
    }

    let ter = require_auth();
    if !is_tes_success(ter) {
        return ter;
    }

    if !facts.account_holds_sufficient_assets {
        return Ter::TEC_INSUFFICIENT_FUNDS;
    }

    Ter::TES_SUCCESS
}

#[cfg(test)]
mod tests {
    use std::{cell::Cell, rc::Rc};

    use protocol::{Ter, trans_token};

    use super::{VaultDepositPreclaimFacts, run_vault_deposit_preclaim};

    #[test]
    fn vault_deposit_preclaim_rejects_missing_vault() {
        let can_transfer_called = Cell::new(false);

        let result = run_vault_deposit_preclaim(
            VaultDepositPreclaimFacts::default(),
            || {
                can_transfer_called.set(true);
                Ter::TES_SUCCESS
            },
            || Ter::TES_SUCCESS,
            || Ter::TES_SUCCESS,
            || Ter::TES_SUCCESS,
        );

        assert_eq!(result, Ter::TEC_NO_ENTRY);
        assert_eq!(trans_token(result), "tecNO_ENTRY");
        assert!(!can_transfer_called.get());
    }

    #[test]
    fn vault_deposit_preclaim_rejects_asset_mismatch_before_helper_calls() {
        let can_transfer_called = Cell::new(false);

        let result = run_vault_deposit_preclaim(
            VaultDepositPreclaimFacts {
                vault_exists: true,
                ..VaultDepositPreclaimFacts::default()
            },
            || {
                can_transfer_called.set(true);
                Ter::TES_SUCCESS
            },
            || Ter::TES_SUCCESS,
            || Ter::TES_SUCCESS,
            || Ter::TES_SUCCESS,
        );

        assert_eq!(result, Ter::TEC_WRONG_ASSET);
        assert_eq!(trans_token(result), "tecWRONG_ASSET");
        assert!(!can_transfer_called.get());
    }

    #[test]
    fn vault_deposit_preclaim_returns_transfer_failure_first() {
        let domain_checked = Cell::new(false);

        let result = run_vault_deposit_preclaim(
            VaultDepositPreclaimFacts {
                vault_exists: true,
                deposited_asset_matches_vault_asset: true,
                ..VaultDepositPreclaimFacts::default()
            },
            || Ter::TER_NO_RIPPLE,
            || {
                domain_checked.set(true);
                Ter::TES_SUCCESS
            },
            || Ter::TES_SUCCESS,
            || Ter::TES_SUCCESS,
        );

        assert_eq!(result, Ter::TER_NO_RIPPLE);
        assert_eq!(trans_token(result), "terNO_RIPPLE");
        assert!(!domain_checked.get());
    }

    #[test]
    fn vault_deposit_preclaim_rejects_vault_share_asset_overlap() {
        let result = run_vault_deposit_preclaim(
            VaultDepositPreclaimFacts {
                vault_exists: true,
                deposited_asset_matches_vault_asset: true,
                vault_share_matches_vault_asset: true,
                ..VaultDepositPreclaimFacts::default()
            },
            || Ter::TES_SUCCESS,
            || Ter::TES_SUCCESS,
            || Ter::TES_SUCCESS,
            || Ter::TES_SUCCESS,
        );

        assert_eq!(result, Ter::TEF_INTERNAL);
        assert_eq!(trans_token(result), "tefINTERNAL");
    }

    #[test]
    fn vault_deposit_preclaim_rejects_missing_or_locked_issuance() {
        let missing = run_vault_deposit_preclaim(
            VaultDepositPreclaimFacts {
                vault_exists: true,
                deposited_asset_matches_vault_asset: true,
                ..VaultDepositPreclaimFacts::default()
            },
            || Ter::TES_SUCCESS,
            || Ter::TES_SUCCESS,
            || Ter::TES_SUCCESS,
            || Ter::TES_SUCCESS,
        );
        let locked = run_vault_deposit_preclaim(
            VaultDepositPreclaimFacts {
                vault_exists: true,
                deposited_asset_matches_vault_asset: true,
                issuance_exists: true,
                issuance_locked: true,
                ..VaultDepositPreclaimFacts::default()
            },
            || Ter::TES_SUCCESS,
            || Ter::TES_SUCCESS,
            || Ter::TES_SUCCESS,
            || Ter::TES_SUCCESS,
        );

        assert_eq!(missing, Ter::TEF_INTERNAL);
        assert_eq!(locked, Ter::TEF_INTERNAL);
    }

    #[test]
    fn vault_deposit_preclaim_maps_frozen_asset_to_tecfrozen_or_teclocked() {
        let issue = run_vault_deposit_preclaim(
            VaultDepositPreclaimFacts {
                vault_exists: true,
                deposited_asset_matches_vault_asset: true,
                issuance_exists: true,
                vault_asset_is_issue: true,
                vault_asset_frozen_for_account: true,
                ..VaultDepositPreclaimFacts::default()
            },
            || Ter::TES_SUCCESS,
            || Ter::TES_SUCCESS,
            || Ter::TES_SUCCESS,
            || Ter::TES_SUCCESS,
        );
        let non_issue = run_vault_deposit_preclaim(
            VaultDepositPreclaimFacts {
                vault_exists: true,
                deposited_asset_matches_vault_asset: true,
                issuance_exists: true,
                vault_asset_frozen_for_account: true,
                ..VaultDepositPreclaimFacts::default()
            },
            || Ter::TES_SUCCESS,
            || Ter::TES_SUCCESS,
            || Ter::TES_SUCCESS,
            || Ter::TES_SUCCESS,
        );

        assert_eq!(issue, Ter::TEC_FROZEN);
        assert_eq!(non_issue, Ter::TEC_LOCKED);
    }

    #[test]
    fn vault_deposit_preclaim_rejects_frozen_shares() {
        let result = run_vault_deposit_preclaim(
            VaultDepositPreclaimFacts {
                vault_exists: true,
                deposited_asset_matches_vault_asset: true,
                issuance_exists: true,
                vault_share_frozen_for_account: true,
                ..VaultDepositPreclaimFacts::default()
            },
            || Ter::TES_SUCCESS,
            || Ter::TES_SUCCESS,
            || Ter::TES_SUCCESS,
            || Ter::TES_SUCCESS,
        );

        assert_eq!(result, Ter::TEC_LOCKED);
    }

    #[test]
    fn vault_deposit_preclaim_requires_domain_for_private_non_owner_vaults() {
        let valid_domain_called = Cell::new(false);

        let result = run_vault_deposit_preclaim(
            VaultDepositPreclaimFacts {
                vault_exists: true,
                deposited_asset_matches_vault_asset: true,
                issuance_exists: true,
                vault_is_private: true,
                ..VaultDepositPreclaimFacts::default()
            },
            || Ter::TES_SUCCESS,
            || {
                valid_domain_called.set(true);
                Ter::TES_SUCCESS
            },
            || Ter::TES_SUCCESS,
            || Ter::TES_SUCCESS,
        );

        assert_eq!(result, Ter::TEC_NO_AUTH);
        assert_eq!(trans_token(result), "tecNO_AUTH");
        assert!(!valid_domain_called.get());
    }

    #[test]
    fn vault_deposit_preclaim_suppresses_tecexpired_from_domain_check() {
        let require_auth_called = Cell::new(false);

        let result = run_vault_deposit_preclaim(
            VaultDepositPreclaimFacts {
                vault_exists: true,
                deposited_asset_matches_vault_asset: true,
                issuance_exists: true,
                vault_is_private: true,
                domain_id_present: true,
                account_holds_sufficient_assets: true,
                ..VaultDepositPreclaimFacts::default()
            },
            || Ter::TES_SUCCESS,
            || Ter::TEC_EXPIRED,
            || {
                require_auth_called.set(true);
                Ter::TES_SUCCESS
            },
            || Ter::TES_SUCCESS,
        );

        assert_eq!(result, Ter::TES_SUCCESS);
        assert!(require_auth_called.get());
    }

    #[test]
    fn vault_deposit_preclaim_returns_domain_or_auth_or_balance_failures() {
        let domain_failure = run_vault_deposit_preclaim(
            VaultDepositPreclaimFacts {
                vault_exists: true,
                deposited_asset_matches_vault_asset: true,
                issuance_exists: true,
                vault_is_private: true,
                domain_id_present: true,
                ..VaultDepositPreclaimFacts::default()
            },
            || Ter::TES_SUCCESS,
            || Ter::TEC_OBJECT_NOT_FOUND,
            || Ter::TES_SUCCESS,
            || Ter::TES_SUCCESS,
        );
        let auth_failure = run_vault_deposit_preclaim(
            VaultDepositPreclaimFacts {
                vault_exists: true,
                deposited_asset_matches_vault_asset: true,
                issuance_exists: true,
                account_holds_sufficient_assets: true,
                ..VaultDepositPreclaimFacts::default()
            },
            || Ter::TES_SUCCESS,
            || Ter::TES_SUCCESS,
            || Ter::TER_NO_ACCOUNT,
            || Ter::TES_SUCCESS,
        );
        let insufficient_funds = run_vault_deposit_preclaim(
            VaultDepositPreclaimFacts {
                vault_exists: true,
                deposited_asset_matches_vault_asset: true,
                issuance_exists: true,
                ..VaultDepositPreclaimFacts::default()
            },
            || Ter::TES_SUCCESS,
            || Ter::TES_SUCCESS,
            || Ter::TES_SUCCESS,
            || Ter::TES_SUCCESS,
        );

        assert_eq!(domain_failure, Ter::TEC_OBJECT_NOT_FOUND);
        assert_eq!(auth_failure, Ter::TER_NO_ACCOUNT);
        assert_eq!(insufficient_funds, Ter::TEC_INSUFFICIENT_FUNDS);
    }

    #[test]
    fn vault_deposit_preclaim_runs_helpers_in_current_on_success() {
        let seen = Rc::new(std::cell::RefCell::new(Vec::new()));

        let result = run_vault_deposit_preclaim(
            VaultDepositPreclaimFacts {
                vault_exists: true,
                deposited_asset_matches_vault_asset: true,
                issuance_exists: true,
                vault_is_private: true,
                domain_id_present: true,
                account_holds_sufficient_assets: true,
                ..VaultDepositPreclaimFacts::default()
            },
            {
                let seen = Rc::clone(&seen);
                move || {
                    seen.borrow_mut().push("transfer");
                    Ter::TES_SUCCESS
                }
            },
            {
                let seen = Rc::clone(&seen);
                move || {
                    seen.borrow_mut().push("domain");
                    Ter::TEC_EXPIRED
                }
            },
            {
                let seen = Rc::clone(&seen);
                move || {
                    seen.borrow_mut().push("auth");
                    Ter::TES_SUCCESS
                }
            },
            || Ter::TES_SUCCESS,
        );

        assert_eq!(result, Ter::TES_SUCCESS);
        assert_eq!(seen.borrow().as_slice(), ["transfer", "domain", "auth"]);
    }

    #[test]
    fn vault_deposit_preclaim_uses_unified_freeze_check_when_amendment_enabled() {
        let result = run_vault_deposit_preclaim(
            VaultDepositPreclaimFacts {
                vault_exists: true,
                deposited_asset_matches_vault_asset: true,
                issuance_exists: true,
                fix_cleanup_3_3_0_enabled: true,
                vault_asset_frozen_for_account: true,
                account_holds_sufficient_assets: true,
                ..VaultDepositPreclaimFacts::default()
            },
            || Ter::TES_SUCCESS,
            || Ter::TES_SUCCESS,
            || Ter::TES_SUCCESS,
            || Ter::TES_SUCCESS,
        );
        assert_eq!(result, Ter::TES_SUCCESS);

        let frozen = run_vault_deposit_preclaim(
            VaultDepositPreclaimFacts {
                vault_exists: true,
                deposited_asset_matches_vault_asset: true,
                issuance_exists: true,
                fix_cleanup_3_3_0_enabled: true,
                account_holds_sufficient_assets: true,
                ..VaultDepositPreclaimFacts::default()
            },
            || Ter::TES_SUCCESS,
            || Ter::TES_SUCCESS,
            || Ter::TES_SUCCESS,
            || Ter::TEC_FROZEN,
        );
        assert_eq!(frozen, Ter::TEC_FROZEN);
    }
}
