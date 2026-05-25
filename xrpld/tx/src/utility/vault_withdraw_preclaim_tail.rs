//! Deterministic tail half of the reference implementation.
//!
//! This ports the exact current guard order for:
//!
//! - `requireAuth(...)` with the current weak-versus-strong auth-type choice,
//! - destination-account frozen asset rejection,
//! - submitter frozen share rejection,
//! - and final `tesSUCCESS`.

use protocol::{Ter, is_tes_success};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct VaultWithdrawPreclaimTailFacts {
    pub destination_is_submitter: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VaultWithdrawRequireAuthType {
    WeakAuth,
    StrongAuth,
}

pub fn run_vault_withdraw_preclaim_tail<RequireAuth, CheckDestinationFrozen, CheckSubmitterFrozen>(
    facts: VaultWithdrawPreclaimTailFacts,
    require_auth: RequireAuth,
    check_destination_frozen: CheckDestinationFrozen,
    check_submitter_share_frozen: CheckSubmitterFrozen,
) -> Ter
where
    RequireAuth: FnOnce(VaultWithdrawRequireAuthType) -> Ter,
    CheckDestinationFrozen: FnOnce() -> Ter,
    CheckSubmitterFrozen: FnOnce() -> Ter,
{
    let auth_type = if facts.destination_is_submitter {
        VaultWithdrawRequireAuthType::WeakAuth
    } else {
        VaultWithdrawRequireAuthType::StrongAuth
    };

    let ter = require_auth(auth_type);
    if !is_tes_success(ter) {
        return ter;
    }

    let ter = check_destination_frozen();
    if !is_tes_success(ter) {
        return ter;
    }

    let ter = check_submitter_share_frozen();
    if !is_tes_success(ter) {
        return ter;
    }

    Ter::TES_SUCCESS
}

#[cfg(test)]
mod tests {
    use std::{cell::Cell, rc::Rc};

    use protocol::{Ter, trans_token};

    use super::{
        VaultWithdrawPreclaimTailFacts, VaultWithdrawRequireAuthType,
        run_vault_withdraw_preclaim_tail,
    };

    #[test]
    fn vault_withdraw_preclaim_tail_uses_weakauth_when_destination_is_submitter() {
        let seen = Cell::new(None);

        let result = run_vault_withdraw_preclaim_tail(
            VaultWithdrawPreclaimTailFacts {
                destination_is_submitter: true,
            },
            |auth_type| {
                seen.set(Some(auth_type));
                Ter::TES_SUCCESS
            },
            || Ter::TES_SUCCESS,
            || Ter::TES_SUCCESS,
        );

        assert_eq!(result, Ter::TES_SUCCESS);
        assert_eq!(seen.get(), Some(VaultWithdrawRequireAuthType::WeakAuth));
    }

    #[test]
    fn vault_withdraw_preclaim_tail_uses_strongauth_when_destination_differs() {
        let seen = Cell::new(None);

        let result = run_vault_withdraw_preclaim_tail(
            VaultWithdrawPreclaimTailFacts::default(),
            |auth_type| {
                seen.set(Some(auth_type));
                Ter::TES_SUCCESS
            },
            || Ter::TES_SUCCESS,
            || Ter::TES_SUCCESS,
        );

        assert_eq!(result, Ter::TES_SUCCESS);
        assert_eq!(seen.get(), Some(VaultWithdrawRequireAuthType::StrongAuth));
    }

    #[test]
    fn vault_withdraw_preclaim_tail_returns_auth_failure_first() {
        let destination_checked = Cell::new(false);

        let result = run_vault_withdraw_preclaim_tail(
            VaultWithdrawPreclaimTailFacts::default(),
            |_| Ter::TEC_NO_AUTH,
            || {
                destination_checked.set(true);
                Ter::TES_SUCCESS
            },
            || Ter::TES_SUCCESS,
        );

        assert_eq!(result, Ter::TEC_NO_AUTH);
        assert_eq!(trans_token(result), "tecNO_AUTH");
        assert!(!destination_checked.get());
    }

    #[test]
    fn vault_withdraw_preclaim_tail_returns_destination_freeze_failure_before_share_check() {
        let share_checked = Cell::new(false);

        let result = run_vault_withdraw_preclaim_tail(
            VaultWithdrawPreclaimTailFacts::default(),
            |_| Ter::TES_SUCCESS,
            || Ter::TEC_FROZEN,
            || {
                share_checked.set(true);
                Ter::TES_SUCCESS
            },
        );

        assert_eq!(result, Ter::TEC_FROZEN);
        assert_eq!(trans_token(result), "tecFROZEN");
        assert!(!share_checked.get());
    }

    #[test]
    fn vault_withdraw_preclaim_tail_returns_submitter_share_freeze_failure() {
        let result = run_vault_withdraw_preclaim_tail(
            VaultWithdrawPreclaimTailFacts::default(),
            |_| Ter::TES_SUCCESS,
            || Ter::TES_SUCCESS,
            || Ter::TEC_LOCKED,
        );

        assert_eq!(result, Ter::TEC_LOCKED);
        assert_eq!(trans_token(result), "tecLOCKED");
    }

    #[test]
    fn vault_withdraw_preclaim_tail_runs_current_on_success() {
        let seen = Rc::new(std::cell::RefCell::new(Vec::new()));

        let result = run_vault_withdraw_preclaim_tail(
            VaultWithdrawPreclaimTailFacts::default(),
            {
                let seen = Rc::clone(&seen);
                move |_| {
                    seen.borrow_mut().push("auth");
                    Ter::TES_SUCCESS
                }
            },
            {
                let seen = Rc::clone(&seen);
                move || {
                    seen.borrow_mut().push("destination_frozen");
                    Ter::TES_SUCCESS
                }
            },
            {
                let seen = Rc::clone(&seen);
                move || {
                    seen.borrow_mut().push("share_frozen");
                    Ter::TES_SUCCESS
                }
            },
        );

        assert_eq!(result, Ter::TES_SUCCESS);
        assert_eq!(
            seen.borrow().as_slice(),
            ["auth", "destination_frozen", "share_frozen"]
        );
    }
}
