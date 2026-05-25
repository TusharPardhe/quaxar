//! Front load and exchange shell inside the reference implementation.
//!
//! This ports the exact current behavior around:
//!
//! - `tefINTERNAL` for a missing vault or share issuance,
//! - the current asset-versus-share branch selection,
//! - the current `tecINTERNAL` mapping for missing helper results,
//! - zero redeemed shares mapping to `tecPRECISION_LOSS` on the asset branch,
//! - unexpected amount-asset selection mapping to `tefINTERNAL`,
//! - and overflow from either branch mapping to `tecPATH_DRY`.

use protocol::Ter;

pub trait VaultWithdrawDoApplyExchangeVault {
    type Asset;
    type AccountId;
    type ShareId;

    fn asset(&self) -> &Self::Asset;
    fn account(&self) -> &Self::AccountId;
    fn owner(&self) -> &Self::AccountId;
    fn share_mpt_id(&self) -> &Self::ShareId;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VaultWithdrawDoApplyExchangeState<
    Vault,
    Issuance,
    Asset,
    AccountId,
    ShareId,
    ShareAmount,
    Amount,
> {
    pub vault: Vault,
    pub share_issuance: Issuance,
    pub vault_asset: Asset,
    pub vault_account: AccountId,
    pub share_id: ShareId,
    pub shares_redeemed: ShareAmount,
    pub assets_withdrawn: Amount,
}

#[allow(clippy::too_many_arguments)]
pub fn load_vault_withdraw_do_apply_exchange<
    Vault,
    Issuance,
    Amount,
    ShareAmount,
    Overflow,
    ReadVault,
    ReadIssuance,
    AmountIsVaultAsset,
    AmountIsShare,
    AmountToShareAmount,
    AssetsToSharesWithdraw,
    SharesToAssetsWithdraw,
    IsZeroShares,
>(
    amount: &Amount,
    read_vault: ReadVault,
    read_issuance: ReadIssuance,
    amount_is_vault_asset: AmountIsVaultAsset,
    amount_is_share: AmountIsShare,
    amount_to_share_amount: AmountToShareAmount,
    assets_to_shares_withdraw: AssetsToSharesWithdraw,
    shares_to_assets_withdraw: SharesToAssetsWithdraw,
    is_zero_shares: IsZeroShares,
) -> Result<
    VaultWithdrawDoApplyExchangeState<
        Vault,
        Issuance,
        Vault::Asset,
        Vault::AccountId,
        Vault::ShareId,
        ShareAmount,
        Amount,
    >,
    Ter,
>
where
    Vault: VaultWithdrawDoApplyExchangeVault,
    Vault::Asset: Clone,
    Vault::AccountId: Clone,
    Vault::ShareId: Clone,
    ReadVault: FnOnce() -> Option<Vault>,
    ReadIssuance: FnOnce(&Vault::ShareId) -> Option<Issuance>,
    AmountIsVaultAsset: FnOnce(&Amount, &Vault) -> bool,
    AmountIsShare: FnOnce(&Amount, &Vault::ShareId) -> bool,
    AmountToShareAmount: FnOnce(&Amount) -> ShareAmount,
    AssetsToSharesWithdraw:
        FnOnce(&Vault, &Issuance, &Amount) -> Result<Option<ShareAmount>, Overflow>,
    SharesToAssetsWithdraw:
        FnOnce(&Vault, &Issuance, &ShareAmount) -> Result<Option<Amount>, Overflow>,
    IsZeroShares: FnOnce(&ShareAmount) -> bool,
{
    let vault = read_vault().ok_or(Ter::TEF_INTERNAL)?;
    let share_id = vault.share_mpt_id().clone();
    let share_issuance = read_issuance(&share_id).ok_or(Ter::TEF_INTERNAL)?;
    let vault_asset = vault.asset().clone();
    let vault_account = vault.account().clone();

    let (shares_redeemed, assets_withdrawn) = if amount_is_vault_asset(amount, &vault) {
        let shares_redeemed = match assets_to_shares_withdraw(&vault, &share_issuance, amount) {
            Ok(Some(shares_redeemed)) => shares_redeemed,
            Ok(None) => return Err(Ter::TEC_INTERNAL),
            Err(_) => return Err(Ter::TEC_PATH_DRY),
        };

        if is_zero_shares(&shares_redeemed) {
            return Err(Ter::TEC_PRECISION_LOSS);
        }

        let assets_withdrawn =
            match shares_to_assets_withdraw(&vault, &share_issuance, &shares_redeemed) {
                Ok(Some(assets_withdrawn)) => assets_withdrawn,
                Ok(None) => return Err(Ter::TEC_INTERNAL),
                Err(_) => return Err(Ter::TEC_PATH_DRY),
            };

        (shares_redeemed, assets_withdrawn)
    } else if amount_is_share(amount, &share_id) {
        let shares_redeemed = amount_to_share_amount(amount);
        let assets_withdrawn =
            match shares_to_assets_withdraw(&vault, &share_issuance, &shares_redeemed) {
                Ok(Some(assets_withdrawn)) => assets_withdrawn,
                Ok(None) => return Err(Ter::TEC_INTERNAL),
                Err(_) => return Err(Ter::TEC_PATH_DRY),
            };

        (shares_redeemed, assets_withdrawn)
    } else {
        return Err(Ter::TEF_INTERNAL);
    };

    Ok(VaultWithdrawDoApplyExchangeState {
        vault,
        share_issuance,
        vault_asset,
        vault_account,
        share_id,
        shares_redeemed,
        assets_withdrawn,
    })
}

#[cfg(test)]
mod tests {
    use std::{cell::Cell, rc::Rc};

    use protocol::{Ter, trans_token};

    use super::{VaultWithdrawDoApplyExchangeVault, load_vault_withdraw_do_apply_exchange};

    #[derive(Debug, Clone, PartialEq, Eq)]
    struct TestVault {
        asset: &'static str,
        account: &'static str,
        owner: &'static str,
        share_mpt_id: &'static str,
    }

    impl VaultWithdrawDoApplyExchangeVault for TestVault {
        type Asset = &'static str;
        type AccountId = &'static str;
        type ShareId = &'static str;

        fn asset(&self) -> &Self::Asset {
            &self.asset
        }

        fn account(&self) -> &Self::AccountId {
            &self.account
        }

        fn owner(&self) -> &Self::AccountId {
            &self.owner
        }

        fn share_mpt_id(&self) -> &Self::ShareId {
            &self.share_mpt_id
        }
    }

    fn test_vault() -> TestVault {
        TestVault {
            asset: "USD",
            account: "vault-account",
            owner: "vault-owner",
            share_mpt_id: "share-id",
        }
    }

    #[test]
    fn vault_withdraw_do_apply_exchange_returns_tefinternal_when_vault_is_missing() {
        let issuance_read = Cell::new(false);

        let result = load_vault_withdraw_do_apply_exchange(
            &50_i64,
            || None::<TestVault>,
            |_| {
                issuance_read.set(true);
                Some("issuance")
            },
            |_, _| true,
            |_, _| false,
            |amount| *amount,
            |_, _, _| Ok::<_, ()>(Some(10_i64)),
            |_, _, _| Ok::<_, ()>(Some(25_i64)),
            |_| false,
        );

        assert_eq!(result, Err(Ter::TEF_INTERNAL));
        assert_eq!(trans_token(result.unwrap_err()), "tefINTERNAL");
        assert!(!issuance_read.get());
    }

    #[test]
    fn vault_withdraw_do_apply_exchange_returns_tefinternal_when_issuance_is_missing() {
        let result = load_vault_withdraw_do_apply_exchange(
            &50_i64,
            || Some(test_vault()),
            |_| None::<&'static str>,
            |_, _| true,
            |_, _| false,
            |amount| *amount,
            |_, _, _| Ok::<_, ()>(Some(10_i64)),
            |_, _, _| Ok::<_, ()>(Some(25_i64)),
            |_| false,
        );

        assert_eq!(result, Err(Ter::TEF_INTERNAL));
        assert_eq!(trans_token(result.unwrap_err()), "tefINTERNAL");
    }

    #[test]
    fn vault_withdraw_do_apply_exchange_returns_asset_branch_failures() {
        let missing = load_vault_withdraw_do_apply_exchange(
            &50_i64,
            || Some(test_vault()),
            |_| Some("issuance"),
            |_, _| true,
            |_, _| false,
            |amount| *amount,
            |_, _, _| Ok::<_, ()>(None::<i64>),
            |_, _, _| Ok::<_, ()>(Some(25_i64)),
            |_| false,
        );
        let zero = load_vault_withdraw_do_apply_exchange(
            &50_i64,
            || Some(test_vault()),
            |_| Some("issuance"),
            |_, _| true,
            |_, _| false,
            |amount| *amount,
            |_, _, _| Ok::<_, ()>(Some(0_i64)),
            |_, _, _| Ok::<_, ()>(Some(25_i64)),
            |shares| *shares == 0,
        );

        assert_eq!(missing, Err(Ter::TEC_INTERNAL));
        assert_eq!(zero, Err(Ter::TEC_PRECISION_LOSS));
        assert_eq!(trans_token(zero.unwrap_err()), "tecPRECISION_LOSS");
    }

    #[test]
    fn vault_withdraw_do_apply_exchange_returns_tefinternal_for_unexpected_asset() {
        let result = load_vault_withdraw_do_apply_exchange(
            &50_i64,
            || Some(test_vault()),
            |_| Some("issuance"),
            |_, _| false,
            |_, _| false,
            |amount| *amount,
            |_, _, _| Ok::<_, ()>(Some(10_i64)),
            |_, _, _| Ok::<_, ()>(Some(25_i64)),
            |_| false,
        );

        assert_eq!(result, Err(Ter::TEF_INTERNAL));
        assert_eq!(trans_token(result.unwrap_err()), "tefINTERNAL");
    }

    #[test]
    fn vault_withdraw_do_apply_exchange_maps_overflow_to_path_dry() {
        let asset_branch = load_vault_withdraw_do_apply_exchange(
            &50_i64,
            || Some(test_vault()),
            |_| Some("issuance"),
            |_, _| true,
            |_, _| false,
            |amount| *amount,
            |_, _, _| Err::<Option<i64>, &'static str>("overflow"),
            |_, _, _| Ok::<_, &'static str>(Some(25_i64)),
            |_| false,
        );
        let share_branch = load_vault_withdraw_do_apply_exchange(
            &50_i64,
            || Some(test_vault()),
            |_| Some("issuance"),
            |_, _| false,
            |_, _| true,
            |amount| *amount,
            |_, _, _| Ok::<_, &'static str>(Some(10_i64)),
            |_, _, _| Err::<Option<i64>, &'static str>("overflow"),
            |_| false,
        );

        assert_eq!(asset_branch, Err(Ter::TEC_PATH_DRY));
        assert_eq!(share_branch, Err(Ter::TEC_PATH_DRY));
        assert_eq!(trans_token(asset_branch.unwrap_err()), "tecPATH_DRY");
    }

    #[test]
    fn vault_withdraw_do_apply_exchange_runs_current_on_success() {
        let steps = Rc::new(std::cell::RefCell::new(Vec::new()));

        let result = load_vault_withdraw_do_apply_exchange(
            &50_i64,
            {
                let steps = Rc::clone(&steps);
                move || {
                    steps.borrow_mut().push("read_vault");
                    Some(test_vault())
                }
            },
            {
                let steps = Rc::clone(&steps);
                move |share_id| {
                    steps.borrow_mut().push("read_issuance");
                    assert_eq!(*share_id, "share-id");
                    Some("issuance")
                }
            },
            {
                let steps = Rc::clone(&steps);
                move |amount, vault| {
                    steps.borrow_mut().push("branch_asset");
                    assert_eq!(*amount, 50);
                    assert_eq!(vault.asset(), &"USD");
                    true
                }
            },
            {
                let steps = Rc::clone(&steps);
                move |_, _| {
                    steps.borrow_mut().push("branch_share");
                    false
                }
            },
            |amount| *amount,
            {
                let steps = Rc::clone(&steps);
                move |_, _, amount| {
                    steps.borrow_mut().push("assets_to_shares");
                    assert_eq!(*amount, 50);
                    Ok::<_, ()>(Some(10_i64))
                }
            },
            {
                let steps = Rc::clone(&steps);
                move |_, _, shares| {
                    steps.borrow_mut().push("shares_to_assets");
                    assert_eq!(*shares, 10);
                    Ok::<_, ()>(Some(25_i64))
                }
            },
            |_| false,
        )
        .unwrap();

        assert_eq!(result.vault_asset, "USD");
        assert_eq!(result.vault_account, "vault-account");
        assert_eq!(result.share_id, "share-id");
        assert_eq!(result.shares_redeemed, 10);
        assert_eq!(result.assets_withdrawn, 25);
        assert_eq!(
            steps.borrow().as_slice(),
            [
                "read_vault",
                "read_issuance",
                "branch_asset",
                "assets_to_shares",
                "shares_to_assets",
            ]
        );
    }
}
