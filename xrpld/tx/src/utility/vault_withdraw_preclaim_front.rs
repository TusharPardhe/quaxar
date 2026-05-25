//! Deterministic front half of the reference implementation.
//!
//! This ports the exact current guard order for:
//!
//! - missing vault lookup,
//! - amount asset mismatch,
//! - transferability failure,
//! - invalid withdrawal policy,
//! - the post-`fixSecurity3_1_3` share-denominated conversion branch,
//! - and the pre-amendment or asset-denominated direct `canWithdraw(...)`
//!   branch.

use protocol::{Ter, is_tes_success};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct VaultWithdrawPreclaimFrontFacts {
    pub vault_exists: bool,
    pub amount_asset_matches_vault_asset_or_share: bool,
    pub withdrawal_policy_is_first_come_first_serve: bool,
    pub fix_security_3_1_3_enabled: bool,
    pub amount_asset_is_vault_share: bool,
    pub share_issuance_exists: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VaultWithdrawShareBranchResult {
    Success,
    MissingConvertedAssets,
    Overflow,
    CanWithdrawFailure(Ter),
}

pub fn run_vault_withdraw_preclaim_front<CanTransfer, RunShareBranch, RunDirectBranch>(
    facts: VaultWithdrawPreclaimFrontFacts,
    can_transfer: CanTransfer,
    run_share_branch: RunShareBranch,
    run_direct_branch: RunDirectBranch,
) -> Ter
where
    CanTransfer: FnOnce() -> Ter,
    RunShareBranch: FnOnce() -> VaultWithdrawShareBranchResult,
    RunDirectBranch: FnOnce() -> Ter,
{
    if !facts.vault_exists {
        return Ter::TEC_NO_ENTRY;
    }

    if !facts.amount_asset_matches_vault_asset_or_share {
        return Ter::TEC_WRONG_ASSET;
    }

    let ter = can_transfer();
    if !is_tes_success(ter) {
        return ter;
    }

    if !facts.withdrawal_policy_is_first_come_first_serve {
        return Ter::TEF_INTERNAL;
    }

    if facts.fix_security_3_1_3_enabled && facts.amount_asset_is_vault_share {
        if !facts.share_issuance_exists {
            return Ter::TEF_INTERNAL;
        }

        return match run_share_branch() {
            VaultWithdrawShareBranchResult::Success => Ter::TES_SUCCESS,
            VaultWithdrawShareBranchResult::MissingConvertedAssets => Ter::TEF_INTERNAL,
            VaultWithdrawShareBranchResult::Overflow => Ter::TEC_PATH_DRY,
            VaultWithdrawShareBranchResult::CanWithdrawFailure(ter) => ter,
        };
    }

    let ter = run_direct_branch();
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
        VaultWithdrawPreclaimFrontFacts, VaultWithdrawShareBranchResult,
        run_vault_withdraw_preclaim_front,
    };

    #[test]
    fn vault_withdraw_preclaim_front_rejects_missing_vault() {
        let can_transfer_called = Cell::new(false);

        let result = run_vault_withdraw_preclaim_front(
            VaultWithdrawPreclaimFrontFacts::default(),
            || {
                can_transfer_called.set(true);
                Ter::TES_SUCCESS
            },
            || VaultWithdrawShareBranchResult::Success,
            || Ter::TES_SUCCESS,
        );

        assert_eq!(result, Ter::TEC_NO_ENTRY);
        assert_eq!(trans_token(result), "tecNO_ENTRY");
        assert!(!can_transfer_called.get());
    }

    #[test]
    fn vault_withdraw_preclaim_front_rejects_asset_mismatch() {
        let can_transfer_called = Cell::new(false);

        let result = run_vault_withdraw_preclaim_front(
            VaultWithdrawPreclaimFrontFacts {
                vault_exists: true,
                ..VaultWithdrawPreclaimFrontFacts::default()
            },
            || {
                can_transfer_called.set(true);
                Ter::TES_SUCCESS
            },
            || VaultWithdrawShareBranchResult::Success,
            || Ter::TES_SUCCESS,
        );

        assert_eq!(result, Ter::TEC_WRONG_ASSET);
        assert_eq!(trans_token(result), "tecWRONG_ASSET");
        assert!(!can_transfer_called.get());
    }

    #[test]
    fn vault_withdraw_preclaim_front_returns_transfer_failure_first() {
        let policy_checked = Cell::new(false);

        let result = run_vault_withdraw_preclaim_front(
            VaultWithdrawPreclaimFrontFacts {
                vault_exists: true,
                amount_asset_matches_vault_asset_or_share: true,
                ..VaultWithdrawPreclaimFrontFacts::default()
            },
            || Ter::TER_NO_RIPPLE,
            || {
                policy_checked.set(true);
                VaultWithdrawShareBranchResult::Success
            },
            || Ter::TES_SUCCESS,
        );

        assert_eq!(result, Ter::TER_NO_RIPPLE);
        assert_eq!(trans_token(result), "terNO_RIPPLE");
        assert!(!policy_checked.get());
    }

    #[test]
    fn vault_withdraw_preclaim_front_rejects_invalid_withdrawal_policy() {
        let share_called = Cell::new(false);

        let result = run_vault_withdraw_preclaim_front(
            VaultWithdrawPreclaimFrontFacts {
                vault_exists: true,
                amount_asset_matches_vault_asset_or_share: true,
                ..VaultWithdrawPreclaimFrontFacts::default()
            },
            || Ter::TES_SUCCESS,
            || {
                share_called.set(true);
                VaultWithdrawShareBranchResult::Success
            },
            || Ter::TES_SUCCESS,
        );

        assert_eq!(result, Ter::TEF_INTERNAL);
        assert_eq!(trans_token(result), "tefINTERNAL");
        assert!(!share_called.get());
    }

    #[test]
    fn vault_withdraw_preclaim_front_rejects_missing_share_issuance() {
        let share_called = Cell::new(false);

        let result = run_vault_withdraw_preclaim_front(
            VaultWithdrawPreclaimFrontFacts {
                vault_exists: true,
                amount_asset_matches_vault_asset_or_share: true,
                withdrawal_policy_is_first_come_first_serve: true,
                fix_security_3_1_3_enabled: true,
                amount_asset_is_vault_share: true,
                ..VaultWithdrawPreclaimFrontFacts::default()
            },
            || Ter::TES_SUCCESS,
            || {
                share_called.set(true);
                VaultWithdrawShareBranchResult::Success
            },
            || Ter::TES_SUCCESS,
        );

        assert_eq!(result, Ter::TEF_INTERNAL);
        assert!(!share_called.get());
    }

    #[test]
    fn vault_withdraw_preclaim_front_maps_share_branch_failures() {
        let missing_assets = run_vault_withdraw_preclaim_front(
            VaultWithdrawPreclaimFrontFacts {
                vault_exists: true,
                amount_asset_matches_vault_asset_or_share: true,
                withdrawal_policy_is_first_come_first_serve: true,
                fix_security_3_1_3_enabled: true,
                amount_asset_is_vault_share: true,
                share_issuance_exists: true,
            },
            || Ter::TES_SUCCESS,
            || VaultWithdrawShareBranchResult::MissingConvertedAssets,
            || Ter::TES_SUCCESS,
        );
        let overflow = run_vault_withdraw_preclaim_front(
            VaultWithdrawPreclaimFrontFacts {
                vault_exists: true,
                amount_asset_matches_vault_asset_or_share: true,
                withdrawal_policy_is_first_come_first_serve: true,
                fix_security_3_1_3_enabled: true,
                amount_asset_is_vault_share: true,
                share_issuance_exists: true,
            },
            || Ter::TES_SUCCESS,
            || VaultWithdrawShareBranchResult::Overflow,
            || Ter::TES_SUCCESS,
        );
        let can_withdraw_failure = run_vault_withdraw_preclaim_front(
            VaultWithdrawPreclaimFrontFacts {
                vault_exists: true,
                amount_asset_matches_vault_asset_or_share: true,
                withdrawal_policy_is_first_come_first_serve: true,
                fix_security_3_1_3_enabled: true,
                amount_asset_is_vault_share: true,
                share_issuance_exists: true,
            },
            || Ter::TES_SUCCESS,
            || VaultWithdrawShareBranchResult::CanWithdrawFailure(Ter::TEC_NO_PERMISSION),
            || Ter::TES_SUCCESS,
        );

        assert_eq!(missing_assets, Ter::TEF_INTERNAL);
        assert_eq!(overflow, Ter::TEC_PATH_DRY);
        assert_eq!(trans_token(overflow), "tecPATH_DRY");
        assert_eq!(can_withdraw_failure, Ter::TEC_NO_PERMISSION);
    }

    #[test]
    fn vault_withdraw_preclaim_front_uses_direct_branch_when_share_branch_is_inactive() {
        let direct_called = Cell::new(false);

        let result = run_vault_withdraw_preclaim_front(
            VaultWithdrawPreclaimFrontFacts {
                vault_exists: true,
                amount_asset_matches_vault_asset_or_share: true,
                withdrawal_policy_is_first_come_first_serve: true,
                fix_security_3_1_3_enabled: true,
                ..VaultWithdrawPreclaimFrontFacts::default()
            },
            || Ter::TES_SUCCESS,
            || VaultWithdrawShareBranchResult::CanWithdrawFailure(Ter::TEC_NO_PERMISSION),
            || {
                direct_called.set(true);
                Ter::TEC_NO_AUTH
            },
        );

        assert_eq!(result, Ter::TEC_NO_AUTH);
        assert!(direct_called.get());
    }

    #[test]
    fn vault_withdraw_preclaim_front_runs_share_branch_in_current_on_success() {
        let seen = Rc::new(std::cell::RefCell::new(Vec::new()));

        let result = run_vault_withdraw_preclaim_front(
            VaultWithdrawPreclaimFrontFacts {
                vault_exists: true,
                amount_asset_matches_vault_asset_or_share: true,
                withdrawal_policy_is_first_come_first_serve: true,
                fix_security_3_1_3_enabled: true,
                amount_asset_is_vault_share: true,
                share_issuance_exists: true,
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
                    seen.borrow_mut().push("share");
                    VaultWithdrawShareBranchResult::Success
                }
            },
            {
                let seen = Rc::clone(&seen);
                move || {
                    seen.borrow_mut().push("direct");
                    Ter::TES_SUCCESS
                }
            },
        );

        assert_eq!(result, Ter::TES_SUCCESS);
        assert_eq!(seen.borrow().as_slice(), ["transfer", "share"]);
    }

    #[test]
    fn vault_withdraw_preclaim_front_runs_direct_branch_in_current_on_success() {
        let seen = Rc::new(std::cell::RefCell::new(Vec::new()));

        let result = run_vault_withdraw_preclaim_front(
            VaultWithdrawPreclaimFrontFacts {
                vault_exists: true,
                amount_asset_matches_vault_asset_or_share: true,
                withdrawal_policy_is_first_come_first_serve: true,
                ..VaultWithdrawPreclaimFrontFacts::default()
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
                    seen.borrow_mut().push("share");
                    VaultWithdrawShareBranchResult::Success
                }
            },
            {
                let seen = Rc::clone(&seen);
                move || {
                    seen.borrow_mut().push("direct");
                    Ter::TES_SUCCESS
                }
            },
        );

        assert_eq!(result, Ter::TES_SUCCESS);
        assert_eq!(seen.borrow().as_slice(), ["transfer", "direct"]);
    }
}
