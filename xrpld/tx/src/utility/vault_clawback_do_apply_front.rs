//! Front the reference implementation shell.
//!
//! This ports the exact current behavior around:
//!
//! - `tefINTERNAL` for a missing vault or share issuance,
//! - the current `clawbackAmount(...)` amount selection,
//! - the current loss invariant assert before branch work,
//! - the owner-share branch versus issuer-asset branch choice,
//! - zero destroyed shares mapping to `tecPRECISION_LOSS`,
//! - and the current `AssetsTotal` / `AssetsAvailable` subtraction plus vault
//!   update order before the later transfer tail.

use std::ops::Sub;

use protocol::Ter;

use crate::{
    VaultClawbackAmountVault, VaultClawbackAssetsToClawback, VaultClawbackAssetsToClawbackVault,
    select_vault_clawback_amount,
};

pub const VAULT_CLAWBACK_DO_APPLY_FRONT_ASSERT_MESSAGE: &str =
    "xrpl::VaultClawback::doApply : loss and assets do balance";

pub trait VaultClawbackDoApplyFrontVault:
    VaultClawbackAmountVault + VaultClawbackAssetsToClawbackVault
{
    fn account(&self) -> &<Self as VaultClawbackAmountVault>::AccountId;
    fn assets_total(&self) -> &<Self as VaultClawbackAssetsToClawbackVault>::Amount;
    fn loss_unrealized(&self) -> &<Self as VaultClawbackAssetsToClawbackVault>::Amount;
    fn subtract_assets_total(
        &mut self,
        value: <Self as VaultClawbackAssetsToClawbackVault>::Amount,
    );
    fn subtract_assets_available(
        &mut self,
        value: <Self as VaultClawbackAssetsToClawbackVault>::Amount,
    );
    fn update_vault(&mut self);
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VaultClawbackDoApplyFrontState<Vault, Asset, AccountId, ShareId, Amount, ShareAmount> {
    pub vault: Vault,
    pub vault_asset: Asset,
    pub vault_account: AccountId,
    pub share_id: ShareId,
    pub holder: AccountId,
    pub assets_recovered: Amount,
    pub shares_destroyed: ShareAmount,
}

#[allow(clippy::too_many_arguments)]
pub fn load_vault_clawback_do_apply_front<
    Vault,
    Issuance,
    ShareAmount,
    ReadVault,
    ReadIssuance,
    MakeShareAmount,
    MakeImplicitAssetAmount,
    MakeZeroRecoveredAmount,
    AmountIsShare,
    AccountHolds,
    ComputeAssetsToClawback,
    IsZeroShares,
>(
    account: &<Vault as VaultClawbackAmountVault>::AccountId,
    holder: <Vault as VaultClawbackAmountVault>::AccountId,
    maybe_amount: Option<<Vault as VaultClawbackAssetsToClawbackVault>::Amount>,
    read_vault: ReadVault,
    read_issuance: ReadIssuance,
    make_share_amount: MakeShareAmount,
    make_implicit_asset_amount: MakeImplicitAssetAmount,
    make_zero_recovered_amount: MakeZeroRecoveredAmount,
    amount_is_share: AmountIsShare,
    account_holds: AccountHolds,
    compute_assets_to_clawback: ComputeAssetsToClawback,
    is_zero_shares: IsZeroShares,
) -> Result<
    VaultClawbackDoApplyFrontState<
        Vault,
        <Vault as VaultClawbackAmountVault>::Asset,
        <Vault as VaultClawbackAmountVault>::AccountId,
        <Vault as VaultClawbackAmountVault>::ShareId,
        <Vault as VaultClawbackAssetsToClawbackVault>::Amount,
        ShareAmount,
    >,
    Ter,
>
where
    Vault: VaultClawbackDoApplyFrontVault,
    <Vault as VaultClawbackAmountVault>::Asset: Clone,
    <Vault as VaultClawbackAmountVault>::AccountId: Clone + PartialEq,
    <Vault as VaultClawbackAmountVault>::ShareId: Clone,
    <Vault as VaultClawbackAssetsToClawbackVault>::Amount:
        Clone + PartialOrd + Sub<Output = <Vault as VaultClawbackAssetsToClawbackVault>::Amount>,
    ReadVault: FnOnce() -> Option<Vault>,
    ReadIssuance: FnOnce(&<Vault as VaultClawbackAmountVault>::ShareId) -> Option<Issuance>,
    MakeShareAmount: FnOnce(
        <Vault as VaultClawbackAmountVault>::ShareId,
    ) -> <Vault as VaultClawbackAssetsToClawbackVault>::Amount,
    MakeImplicitAssetAmount: FnOnce(
        <Vault as VaultClawbackAmountVault>::Asset,
    ) -> <Vault as VaultClawbackAssetsToClawbackVault>::Amount,
    MakeZeroRecoveredAmount: FnOnce(
        &<Vault as VaultClawbackAmountVault>::Asset,
    ) -> <Vault as VaultClawbackAssetsToClawbackVault>::Amount,
    AmountIsShare: FnOnce(
        &<Vault as VaultClawbackAssetsToClawbackVault>::Amount,
        &<Vault as VaultClawbackAmountVault>::ShareId,
    ) -> bool,
    AccountHolds: FnOnce(
        &<Vault as VaultClawbackAmountVault>::ShareId,
        &<Vault as VaultClawbackAmountVault>::AccountId,
    ) -> ShareAmount,
    ComputeAssetsToClawback: FnOnce(
        &Vault,
        &Issuance,
        &<Vault as VaultClawbackAmountVault>::AccountId,
        &<Vault as VaultClawbackAssetsToClawbackVault>::Amount,
    ) -> Result<
        VaultClawbackAssetsToClawback<
            <Vault as VaultClawbackAssetsToClawbackVault>::Amount,
            ShareAmount,
        >,
        Ter,
    >,
    IsZeroShares: FnOnce(&ShareAmount) -> bool,
{
    let mut vault = read_vault().ok_or(Ter::TEF_INTERNAL)?;
    let share_id = vault.share_mpt_id().clone();
    let share_issuance = read_issuance(&share_id).ok_or(Ter::TEF_INTERNAL)?;
    let vault_asset = vault.asset().clone();
    let vault_account = vault.account().clone();

    let amount =
        select_vault_clawback_amount(&vault, maybe_amount, account, make_share_amount, |_| {
            make_implicit_asset_amount(vault_asset.clone())
        });

    let assets_available = vault.assets_available().clone();
    let assets_total = vault.assets_total().clone();
    let loss_unrealized = vault.loss_unrealized().clone();
    assert!(
        loss_unrealized <= (assets_total.clone() - assets_available.clone()),
        "{VAULT_CLAWBACK_DO_APPLY_FRONT_ASSERT_MESSAGE}"
    );

    let zero_assets_recovered = make_zero_recovered_amount(&vault_asset);

    let (assets_recovered, shares_destroyed) =
        if account == vault.owner() && amount_is_share(&amount, &share_id) {
            let shares_destroyed = account_holds(&share_id, &holder);
            (zero_assets_recovered, shares_destroyed)
        } else {
            let clawback = compute_assets_to_clawback(&vault, &share_issuance, &holder, &amount)?;
            (clawback.assets_recovered, clawback.shares_destroyed)
        };

    if is_zero_shares(&shares_destroyed) {
        return Err(Ter::TEC_PRECISION_LOSS);
    }

    vault.subtract_assets_total(assets_recovered.clone());
    vault.subtract_assets_available(assets_recovered.clone());
    vault.update_vault();

    Ok(VaultClawbackDoApplyFrontState {
        vault,
        vault_asset,
        vault_account,
        share_id,
        holder,
        assets_recovered,
        shares_destroyed,
    })
}

#[cfg(test)]
mod tests {
    use std::{
        cell::{Cell, RefCell},
        panic::{AssertUnwindSafe, catch_unwind},
        rc::Rc,
    };

    use protocol::{Ter, trans_token};

    use super::{
        VAULT_CLAWBACK_DO_APPLY_FRONT_ASSERT_MESSAGE, VaultClawbackDoApplyFrontState,
        VaultClawbackDoApplyFrontVault, load_vault_clawback_do_apply_front,
    };
    use crate::{
        VaultClawbackAmountVault, VaultClawbackAssetsToClawback, VaultClawbackAssetsToClawbackVault,
    };

    #[derive(Clone, Debug, PartialEq, Eq)]
    struct TestVault {
        asset: &'static str,
        owner: &'static str,
        account: &'static str,
        share_mpt_id: &'static str,
        assets_available: i64,
        assets_total: i64,
        loss_unrealized: i64,
        steps: Rc<RefCell<Vec<String>>>,
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

    impl VaultClawbackAssetsToClawbackVault for TestVault {
        type Amount = i64;

        fn assets_available(&self) -> &Self::Amount {
            &self.assets_available
        }
    }

    impl VaultClawbackDoApplyFrontVault for TestVault {
        fn account(&self) -> &<Self as VaultClawbackAmountVault>::AccountId {
            &self.account
        }

        fn assets_total(&self) -> &<Self as VaultClawbackAssetsToClawbackVault>::Amount {
            &self.assets_total
        }

        fn loss_unrealized(&self) -> &<Self as VaultClawbackAssetsToClawbackVault>::Amount {
            &self.loss_unrealized
        }

        fn subtract_assets_total(
            &mut self,
            value: <Self as VaultClawbackAssetsToClawbackVault>::Amount,
        ) {
            self.assets_total -= value;
            self.steps
                .borrow_mut()
                .push(format!("assets_total-={value}"));
        }

        fn subtract_assets_available(
            &mut self,
            value: <Self as VaultClawbackAssetsToClawbackVault>::Amount,
        ) {
            self.assets_available -= value;
            self.steps
                .borrow_mut()
                .push(format!("assets_available-={value}"));
        }

        fn update_vault(&mut self) {
            self.steps.borrow_mut().push("update_vault".to_string());
        }
    }

    fn build_vault(steps: Rc<RefCell<Vec<String>>>) -> TestVault {
        TestVault {
            asset: "USD",
            owner: "vault-owner",
            account: "vault-account",
            share_mpt_id: "share-id",
            assets_available: 80,
            assets_total: 100,
            loss_unrealized: 10,
            steps,
        }
    }

    #[test]
    fn vault_clawback_do_apply_front_returns_tefinternal_when_vault_is_missing() {
        let issuance_read = Cell::new(false);

        let result = load_vault_clawback_do_apply_front(
            &"issuer",
            "holder",
            None::<i64>,
            || None::<TestVault>,
            |_| {
                issuance_read.set(true);
                Some("issuance")
            },
            |_| 0_i64,
            |_| 0_i64,
            |_| 0_i64,
            |_, _| false,
            |_, _| 0_i64,
            |_, _, _, _| {
                Ok(VaultClawbackAssetsToClawback {
                    assets_recovered: 1_i64,
                    shares_destroyed: 1_i64,
                })
            },
            |_| false,
        );

        assert_eq!(result, Err(Ter::TEF_INTERNAL));
        assert_eq!(trans_token(result.unwrap_err()), "tefINTERNAL");
        assert!(!issuance_read.get());
    }

    #[test]
    fn vault_clawback_do_apply_front_returns_tefinternal_when_issuance_is_missing() {
        let result = load_vault_clawback_do_apply_front(
            &"issuer",
            "holder",
            None::<i64>,
            || Some(build_vault(Rc::new(RefCell::new(Vec::new())))),
            |_| None::<&'static str>,
            |_| 0_i64,
            |_| 0_i64,
            |_| 0_i64,
            |_, _| false,
            |_, _| 0_i64,
            |_, _, _, _| {
                Ok(VaultClawbackAssetsToClawback {
                    assets_recovered: 1_i64,
                    shares_destroyed: 1_i64,
                })
            },
            |_| false,
        );

        assert_eq!(result, Err(Ter::TEF_INTERNAL));
        assert_eq!(trans_token(result.unwrap_err()), "tefINTERNAL");
    }

    #[test]
    fn vault_clawback_do_apply_front_panics_on_loss_invariant_before_branch_work() {
        let holds_called = Cell::new(false);
        let assets_called = Cell::new(false);

        let panic = catch_unwind(AssertUnwindSafe(|| {
            load_vault_clawback_do_apply_front(
                &"issuer",
                "holder",
                Some(4_i64),
                || {
                    Some(TestVault {
                        loss_unrealized: 25,
                        ..build_vault(Rc::new(RefCell::new(Vec::new())))
                    })
                },
                |_| Some("issuance"),
                |_| 0_i64,
                |_| 0_i64,
                |_| 0_i64,
                |_, _| false,
                |_, _| {
                    holds_called.set(true);
                    0_i64
                },
                |_, _, _, _| {
                    assets_called.set(true);
                    Ok(VaultClawbackAssetsToClawback {
                        assets_recovered: 1_i64,
                        shares_destroyed: 1_i64,
                    })
                },
                |_| false,
            )
        }))
        .expect_err("invalid loss invariant should panic");

        let message = panic
            .downcast_ref::<String>()
            .map(String::as_str)
            .or_else(|| panic.downcast_ref::<&'static str>().copied())
            .expect("panic payload should be a string");

        assert!(message.contains(VAULT_CLAWBACK_DO_APPLY_FRONT_ASSERT_MESSAGE));
        assert!(!holds_called.get());
        assert!(!assets_called.get());
    }

    #[test]
    fn vault_clawback_do_apply_front_returns_asset_branch_failure_before_update() {
        let zero_amount_built = Cell::new(false);

        let result = load_vault_clawback_do_apply_front(
            &"issuer",
            "holder",
            Some(4_i64),
            || Some(build_vault(Rc::new(RefCell::new(Vec::new())))),
            |_| Some("issuance"),
            |_| 0_i64,
            |_| 0_i64,
            |_| {
                zero_amount_built.set(true);
                0_i64
            },
            |_, _| false,
            |_, _| 0_i64,
            |_, _, _, _| Err(Ter::TEC_PATH_DRY),
            |_| false,
        );

        assert_eq!(result, Err(Ter::TEC_PATH_DRY));
        assert_eq!(trans_token(result.unwrap_err()), "tecPATH_DRY");
        assert!(zero_amount_built.get());
    }

    #[test]
    fn vault_clawback_do_apply_front_runs_owner_share_branch_in_current() {
        let steps = Rc::new(RefCell::new(Vec::new()));
        let assets_called = Cell::new(false);

        let result = load_vault_clawback_do_apply_front(
            &"vault-owner",
            "holder",
            None::<i64>,
            {
                let steps = Rc::clone(&steps);
                move || {
                    steps.borrow_mut().push("read_vault".to_string());
                    Some(build_vault(Rc::clone(&steps)))
                }
            },
            {
                let steps = Rc::clone(&steps);
                move |_| {
                    steps.borrow_mut().push("read_issuance".to_string());
                    Some("issuance")
                }
            },
            {
                let steps = Rc::clone(&steps);
                move |share_id| {
                    steps.borrow_mut().push(format!("make_share:{share_id}"));
                    0_i64
                }
            },
            |_| 0_i64,
            {
                let steps = Rc::clone(&steps);
                move |_| {
                    steps.borrow_mut().push("make_zero_recovered".to_string());
                    0_i64
                }
            },
            |amount, _| *amount == 0,
            {
                let steps = Rc::clone(&steps);
                move |share_id, holder| {
                    steps
                        .borrow_mut()
                        .push(format!("account_holds:{share_id}:{holder}"));
                    9_i64
                }
            },
            |_, _, _, _| {
                assets_called.set(true);
                Ok(VaultClawbackAssetsToClawback {
                    assets_recovered: 1_i64,
                    shares_destroyed: 1_i64,
                })
            },
            |_| false,
        );

        assert_eq!(
            result,
            Ok(VaultClawbackDoApplyFrontState {
                vault: TestVault {
                    assets_available: 80,
                    assets_total: 100,
                    loss_unrealized: 10,
                    asset: "USD",
                    owner: "vault-owner",
                    account: "vault-account",
                    share_mpt_id: "share-id",
                    steps: Rc::clone(&steps),
                },
                vault_asset: "USD",
                vault_account: "vault-account",
                share_id: "share-id",
                holder: "holder",
                assets_recovered: 0_i64,
                shares_destroyed: 9_i64,
            })
        );
        assert!(!assets_called.get());
        assert_eq!(
            steps.borrow().as_slice(),
            [
                "read_vault",
                "read_issuance",
                "make_share:share-id",
                "make_zero_recovered",
                "account_holds:share-id:holder",
                "assets_total-=0",
                "assets_available-=0",
                "update_vault",
            ]
        );
    }

    #[test]
    fn vault_clawback_do_apply_front_returns_precision_loss_for_zero_destroyed_shares() {
        let result = load_vault_clawback_do_apply_front(
            &"issuer",
            "holder",
            Some(4_i64),
            || Some(build_vault(Rc::new(RefCell::new(Vec::new())))),
            |_| Some("issuance"),
            |_| 0_i64,
            |_| 0_i64,
            |_| 0_i64,
            |_, _| false,
            |_, _| 0_i64,
            |_, _, _, _| {
                Ok(VaultClawbackAssetsToClawback {
                    assets_recovered: 4_i64,
                    shares_destroyed: 0_i64,
                })
            },
            |shares| *shares == 0,
        );

        assert_eq!(result, Err(Ter::TEC_PRECISION_LOSS));
        assert_eq!(trans_token(result.unwrap_err()), "tecPRECISION_LOSS");
    }
}
