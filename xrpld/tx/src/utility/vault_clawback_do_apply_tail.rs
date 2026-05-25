//! Transfer tail inside the reference implementation.
//!
//! This ports the exact current behavior around:
//!
//! - returning the share transfer failure unchanged,
//! - only trying `removeEmptyHolding(...)` for non-owner holders,
//! - ignoring `tecHAS_OBLIGATIONS` from that cleanup attempt,
//! - returning any other cleanup failure unchanged,
//! - only transferring recovered assets when the recovered amount is positive,
//! - mapping a post-transfer negative vault-asset balance to `tefINTERNAL`,
//! - associating the vault asset after the optional recovered-asset branch,
//! - and then returning `tesSUCCESS`.

use protocol::{Ter, is_tes_success};

use crate::{VaultClawbackAmountVault, VaultClawbackDoApplyFrontState};

pub fn run_vault_clawback_do_apply_tail<
    Vault,
    Asset,
    AccountId,
    ShareId,
    Amount,
    ShareAmount,
    SendSharesToVault,
    RemoveEmptyHolding,
    AssetsRecoveredIsPositive,
    SendRecoveredAssetsToIssuer,
    VaultAssetsBalanceIsNegative,
    AssociateAsset,
>(
    account: &AccountId,
    state: &VaultClawbackDoApplyFrontState<Vault, Asset, AccountId, ShareId, Amount, ShareAmount>,
    send_shares_to_vault: SendSharesToVault,
    remove_empty_holding: RemoveEmptyHolding,
    assets_recovered_is_positive: AssetsRecoveredIsPositive,
    send_recovered_assets_to_issuer: SendRecoveredAssetsToIssuer,
    vault_assets_balance_is_negative: VaultAssetsBalanceIsNegative,
    associate_asset: AssociateAsset,
) -> Ter
where
    Vault: VaultClawbackAmountVault<AccountId = AccountId>,
    AccountId: PartialEq,
    SendSharesToVault: FnOnce(&AccountId, &AccountId, &ShareAmount) -> Ter,
    RemoveEmptyHolding: FnOnce(&AccountId, &ShareAmount) -> Ter,
    AssetsRecoveredIsPositive: FnOnce(&Amount) -> bool,
    SendRecoveredAssetsToIssuer: FnOnce(&AccountId, &AccountId, &Amount) -> Ter,
    VaultAssetsBalanceIsNegative: FnOnce(&AccountId, &Asset) -> bool,
    AssociateAsset: FnOnce(&Vault, &Asset),
{
    let ter = send_shares_to_vault(&state.holder, &state.vault_account, &state.shares_destroyed);
    if !is_tes_success(ter) {
        return ter;
    }

    if state.holder != *state.vault.owner() {
        let ter = remove_empty_holding(&state.holder, &state.shares_destroyed);
        if !is_tes_success(ter) && ter != Ter::TEC_HAS_OBLIGATIONS {
            return ter;
        }
    }

    if assets_recovered_is_positive(&state.assets_recovered) {
        let ter =
            send_recovered_assets_to_issuer(&state.vault_account, account, &state.assets_recovered);
        if !is_tes_success(ter) {
            return ter;
        }

        if vault_assets_balance_is_negative(&state.vault_account, &state.vault_asset) {
            return Ter::TEF_INTERNAL;
        }
    }

    associate_asset(&state.vault, &state.vault_asset);
    Ter::TES_SUCCESS
}

#[cfg(test)]
mod tests {
    use std::{cell::Cell, rc::Rc};

    use protocol::{Ter, trans_token};

    use super::run_vault_clawback_do_apply_tail;
    use crate::{VaultClawbackAmountVault, VaultClawbackDoApplyFrontState};

    #[derive(Debug, Clone, PartialEq, Eq)]
    struct TestVault {
        asset: &'static str,
        owner: &'static str,
        share_mpt_id: &'static str,
    }

    impl VaultClawbackAmountVault for TestVault {
        type Asset = &'static str;
        type AccountId = &'static str;
        type ShareId = &'static str;

        fn asset(&self) -> &Self::Asset {
            &self.asset
        }

        fn owner(&self) -> &Self::AccountId {
            &self.owner
        }

        fn share_mpt_id(&self) -> &Self::ShareId {
            &self.share_mpt_id
        }
    }

    fn test_state(
        holder: &'static str,
        assets_recovered: i64,
    ) -> VaultClawbackDoApplyFrontState<TestVault, &'static str, &'static str, &'static str, i64, i64>
    {
        VaultClawbackDoApplyFrontState {
            vault: TestVault {
                asset: "USD",
                owner: "vault-owner",
                share_mpt_id: "share-id",
            },
            vault_asset: "USD",
            vault_account: "vault-account",
            share_id: "share-id",
            holder,
            assets_recovered,
            shares_destroyed: 9,
        }
    }

    #[test]
    fn vault_clawback_do_apply_tail_returns_share_transfer_failure_unchanged() {
        let touched = Cell::new(false);

        let result = run_vault_clawback_do_apply_tail(
            &"issuer",
            &test_state("holder", 4),
            |_, _, _| Ter::TER_NO_ACCOUNT,
            |_, _| {
                touched.set(true);
                Ter::TES_SUCCESS
            },
            |_| {
                touched.set(true);
                true
            },
            |_, _, _| {
                touched.set(true);
                Ter::TES_SUCCESS
            },
            |_, _| {
                touched.set(true);
                false
            },
            |_, _| touched.set(true),
        );

        assert_eq!(result, Ter::TER_NO_ACCOUNT);
        assert_eq!(trans_token(result), "terNO_ACCOUNT");
        assert!(!touched.get());
    }

    #[test]
    fn vault_clawback_do_apply_tail_returns_cleanup_failure_unchanged() {
        let touched = Cell::new(false);

        let result = run_vault_clawback_do_apply_tail(
            &"issuer",
            &test_state("holder", 0),
            |_, _, _| Ter::TES_SUCCESS,
            |_, _| Ter::TEC_NO_PERMISSION,
            |_| {
                touched.set(true);
                false
            },
            |_, _, _| {
                touched.set(true);
                Ter::TES_SUCCESS
            },
            |_, _| {
                touched.set(true);
                false
            },
            |_, _| touched.set(true),
        );

        assert_eq!(result, Ter::TEC_NO_PERMISSION);
        assert_eq!(trans_token(result), "tecNO_PERMISSION");
        assert!(!touched.get());
    }

    #[test]
    fn vault_clawback_do_apply_tail_skips_cleanup_for_owner() {
        let remove_called = Cell::new(false);
        let associated = Cell::new(false);

        let result = run_vault_clawback_do_apply_tail(
            &"issuer",
            &test_state("vault-owner", 0),
            |holder, vault_account, shares_destroyed| {
                assert_eq!(*holder, "vault-owner");
                assert_eq!(*vault_account, "vault-account");
                assert_eq!(*shares_destroyed, 9);
                Ter::TES_SUCCESS
            },
            |_, _| {
                remove_called.set(true);
                Ter::TES_SUCCESS
            },
            |_| false,
            |_, _, _| unreachable!("zero recovered should skip asset transfer"),
            |_, _| unreachable!("zero recovered should skip balance check"),
            |_, asset| {
                associated.set(true);
                assert_eq!(*asset, "USD");
            },
        );

        assert_eq!(result, Ter::TES_SUCCESS);
        assert!(!remove_called.get());
        assert!(associated.get());
    }

    #[test]
    fn vault_clawback_do_apply_tail_returns_asset_transfer_failure_unchanged() {
        let balance_checked = Cell::new(false);
        let associated = Cell::new(false);

        let result = run_vault_clawback_do_apply_tail(
            &"issuer",
            &test_state("holder", 4),
            |_, _, _| Ter::TES_SUCCESS,
            |_, _| Ter::TEC_HAS_OBLIGATIONS,
            |_| true,
            |from, to, amount| {
                assert_eq!(*from, "vault-account");
                assert_eq!(*to, "issuer");
                assert_eq!(*amount, 4);
                Ter::TER_RETRY
            },
            |_, _| {
                balance_checked.set(true);
                false
            },
            |_, _| associated.set(true),
        );

        assert_eq!(result, Ter::TER_RETRY);
        assert_eq!(trans_token(result), "terRETRY");
        assert!(!balance_checked.get());
        assert!(!associated.get());
    }

    #[test]
    fn vault_clawback_do_apply_tail_returns_tefinternal_on_negative_vault_balance() {
        let associated = Cell::new(false);

        let result = run_vault_clawback_do_apply_tail(
            &"issuer",
            &test_state("holder", 4),
            |_, _, _| Ter::TES_SUCCESS,
            |_, _| Ter::TEC_HAS_OBLIGATIONS,
            |_| true,
            |_, _, _| Ter::TES_SUCCESS,
            |vault_account, asset| {
                assert_eq!(*vault_account, "vault-account");
                assert_eq!(*asset, "USD");
                true
            },
            |_, _| associated.set(true),
        );

        assert_eq!(result, Ter::TEF_INTERNAL);
        assert_eq!(trans_token(result), "tefINTERNAL");
        assert!(!associated.get());
    }

    #[test]
    fn vault_clawback_do_apply_tail_runs_full_success_path_in_current() {
        let steps = Rc::new(std::cell::RefCell::new(Vec::new()));

        let result = run_vault_clawback_do_apply_tail(
            &"issuer",
            &test_state("holder", 4),
            {
                let steps = Rc::clone(&steps);
                move |holder, vault_account, shares_destroyed| {
                    steps.borrow_mut().push(format!(
                        "send_shares:{holder}:{vault_account}:{shares_destroyed}"
                    ));
                    Ter::TES_SUCCESS
                }
            },
            {
                let steps = Rc::clone(&steps);
                move |holder, shares_destroyed| {
                    steps
                        .borrow_mut()
                        .push(format!("remove_empty:{holder}:{shares_destroyed}"));
                    Ter::TEC_HAS_OBLIGATIONS
                }
            },
            {
                let steps = Rc::clone(&steps);
                move |assets_recovered| {
                    steps
                        .borrow_mut()
                        .push(format!("is_positive:{assets_recovered}"));
                    *assets_recovered > 0
                }
            },
            {
                let steps = Rc::clone(&steps);
                move |from, to, amount| {
                    steps
                        .borrow_mut()
                        .push(format!("send_assets:{from}:{to}:{amount}"));
                    Ter::TES_SUCCESS
                }
            },
            {
                let steps = Rc::clone(&steps);
                move |vault_account, asset| {
                    steps
                        .borrow_mut()
                        .push(format!("check_negative:{vault_account}:{asset}"));
                    false
                }
            },
            {
                let steps = Rc::clone(&steps);
                move |vault, asset| {
                    steps
                        .borrow_mut()
                        .push(format!("associate:{}:{asset}", vault.owner()));
                }
            },
        );

        assert_eq!(result, Ter::TES_SUCCESS);
        assert_eq!(
            steps.borrow().as_slice(),
            [
                "send_shares:holder:vault-account:9",
                "remove_empty:holder:9",
                "is_positive:4",
                "send_assets:vault-account:issuer:4",
                "check_negative:vault-account:USD",
                "associate:vault-owner:USD",
            ]
        );
    }
}
