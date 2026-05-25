//! `assetsToClawback(...)` helper used by the reference implementation.
//!
//! This ports the exact current behavior around:
//!
//! - internal-error fallback for an unexpected amount-asset mismatch,
//! - zero-amount clawback using current share holdings directly,
//! - non-zero clawback conversion from assets to shares and back,
//! - clamping recovered assets to `AssetsAvailable`,
//! - truncating shares during the clamp retry,
//! - internal-error fallback for invalid post-clamp rounding,
//! - and overflow mapping to `tecPATH_DRY` on the non-zero conversion path.

use protocol::Ter;

pub trait VaultClawbackAssetsToClawbackVault {
    type Amount;

    fn assets_available(&self) -> &Self::Amount;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VaultClawbackAssetsToClawback<Amount, ShareAmount> {
    pub assets_recovered: Amount,
    pub shares_destroyed: ShareAmount,
}

#[allow(clippy::too_many_arguments)]
pub fn compute_vault_clawback_assets_to_clawback<
    Vault,
    ShareAmount,
    Overflow,
    AmountMatchesVaultAsset,
    IsZeroAmount,
    AccountHolds,
    SharesToAssetsFromHolds,
    AssetsToSharesWithdraw,
    AssetsToSharesWithdrawTruncated,
    SharesToAssetsWithdraw,
>(
    vault: &Vault,
    clawback_amount: &Vault::Amount,
    amount_matches_vault_asset: AmountMatchesVaultAsset,
    is_zero_amount: IsZeroAmount,
    account_holds: AccountHolds,
    shares_to_assets_from_holds: SharesToAssetsFromHolds,
    assets_to_shares_withdraw: AssetsToSharesWithdraw,
    assets_to_shares_withdraw_truncated: AssetsToSharesWithdrawTruncated,
    shares_to_assets_withdraw: SharesToAssetsWithdraw,
) -> Result<VaultClawbackAssetsToClawback<Vault::Amount, ShareAmount>, Ter>
where
    Vault: VaultClawbackAssetsToClawbackVault,
    Vault::Amount: Clone + PartialOrd,
    AmountMatchesVaultAsset: FnOnce(&Vault::Amount, &Vault) -> bool,
    IsZeroAmount: FnOnce(&Vault::Amount) -> bool,
    AccountHolds: FnOnce() -> ShareAmount,
    SharesToAssetsFromHolds: FnOnce(&ShareAmount) -> Option<Vault::Amount>,
    AssetsToSharesWithdraw: FnMut(&Vault::Amount) -> Result<Option<ShareAmount>, Overflow>,
    AssetsToSharesWithdrawTruncated: FnMut(&Vault::Amount) -> Result<Option<ShareAmount>, Overflow>,
    SharesToAssetsWithdraw: FnMut(&ShareAmount) -> Result<Option<Vault::Amount>, Overflow>,
{
    if !amount_matches_vault_asset(clawback_amount, vault) {
        return Err(Ter::TEC_INTERNAL);
    }

    let assets_available = vault.assets_available().clone();

    if is_zero_amount(clawback_amount) {
        let shares_destroyed = account_holds();
        let assets_recovered =
            shares_to_assets_from_holds(&shares_destroyed).ok_or(Ter::TEC_INTERNAL)?;

        return Ok(VaultClawbackAssetsToClawback {
            assets_recovered,
            shares_destroyed,
        });
    }

    let mut assets_to_shares_withdraw = assets_to_shares_withdraw;
    let mut assets_to_shares_withdraw_truncated = assets_to_shares_withdraw_truncated;
    let mut shares_to_assets_withdraw = shares_to_assets_withdraw;

    let mut shares_destroyed = match assets_to_shares_withdraw(clawback_amount) {
        Ok(Some(shares_destroyed)) => shares_destroyed,
        Ok(None) => return Err(Ter::TEC_INTERNAL),
        Err(_) => return Err(Ter::TEC_PATH_DRY),
    };

    let mut assets_recovered = match shares_to_assets_withdraw(&shares_destroyed) {
        Ok(Some(assets_recovered)) => assets_recovered,
        Ok(None) => return Err(Ter::TEC_INTERNAL),
        Err(_) => return Err(Ter::TEC_PATH_DRY),
    };

    if assets_recovered > assets_available {
        let clamped_assets = assets_available.clone();
        shares_destroyed = match assets_to_shares_withdraw_truncated(&clamped_assets) {
            Ok(Some(shares_destroyed)) => shares_destroyed,
            Ok(None) => return Err(Ter::TEC_INTERNAL),
            Err(_) => return Err(Ter::TEC_PATH_DRY),
        };

        assets_recovered = match shares_to_assets_withdraw(&shares_destroyed) {
            Ok(Some(assets_recovered)) => assets_recovered,
            Ok(None) => return Err(Ter::TEC_INTERNAL),
            Err(_) => return Err(Ter::TEC_PATH_DRY),
        };

        if assets_recovered > assets_available {
            return Err(Ter::TEC_INTERNAL);
        }
    }

    Ok(VaultClawbackAssetsToClawback {
        assets_recovered,
        shares_destroyed,
    })
}

#[cfg(test)]
mod tests {
    use std::{cell::Cell, rc::Rc};

    use protocol::{Ter, trans_token};

    use super::{
        VaultClawbackAssetsToClawback, VaultClawbackAssetsToClawbackVault,
        compute_vault_clawback_assets_to_clawback,
    };

    #[derive(Debug, Clone, PartialEq, Eq)]
    struct TestVault {
        assets_available: i64,
    }

    impl VaultClawbackAssetsToClawbackVault for TestVault {
        type Amount = i64;

        fn assets_available(&self) -> &Self::Amount {
            &self.assets_available
        }
    }

    #[test]
    fn vault_clawback_assets_to_clawback_rejects_amount_asset_mismatch() {
        let holds_called = Cell::new(false);

        let result = compute_vault_clawback_assets_to_clawback(
            &TestVault {
                assets_available: 5,
            },
            &10_i64,
            |_, _| false,
            |_| false,
            || {
                holds_called.set(true);
                9_i64
            },
            |_| Some(1_i64),
            |_| Ok::<_, ()>(Some(1_i64)),
            |_| Ok::<_, ()>(Some(1_i64)),
            |_| Ok::<_, ()>(Some(1_i64)),
        );

        assert_eq!(result, Err(Ter::TEC_INTERNAL));
        assert_eq!(trans_token(result.unwrap_err()), "tecINTERNAL");
        assert!(!holds_called.get());
    }

    #[test]
    fn vault_clawback_assets_to_clawback_uses_holder_shares_directly_for_zero_amount() {
        let steps = Rc::new(std::cell::RefCell::new(Vec::new()));

        let result = compute_vault_clawback_assets_to_clawback(
            &TestVault {
                assets_available: 5,
            },
            &0_i64,
            |_, _| true,
            |amount| *amount == 0,
            {
                let steps = Rc::clone(&steps);
                move || {
                    steps.borrow_mut().push("account_holds".to_string());
                    9_i64
                }
            },
            {
                let steps = Rc::clone(&steps);
                move |shares| {
                    steps
                        .borrow_mut()
                        .push(format!("shares_to_assets_from_holds:{shares}"));
                    Some(4_i64)
                }
            },
            |_| Ok::<_, ()>(Some(1_i64)),
            |_| Ok::<_, ()>(Some(1_i64)),
            |_| Ok::<_, ()>(Some(1_i64)),
        );

        assert_eq!(
            result,
            Ok(VaultClawbackAssetsToClawback {
                assets_recovered: 4_i64,
                shares_destroyed: 9_i64,
            })
        );
        assert_eq!(
            steps.borrow().as_slice(),
            ["account_holds", "shares_to_assets_from_holds:9"]
        );
    }

    #[test]
    fn vault_clawback_assets_to_clawback_returns_internal_when_zero_amount_assets_are_missing() {
        let result = compute_vault_clawback_assets_to_clawback(
            &TestVault {
                assets_available: 5,
            },
            &0_i64,
            |_, _| true,
            |amount| *amount == 0,
            || 9_i64,
            |_| None::<i64>,
            |_| Ok::<_, ()>(Some(1_i64)),
            |_| Ok::<_, ()>(Some(1_i64)),
            |_| Ok::<_, ()>(Some(1_i64)),
        );

        assert_eq!(result, Err(Ter::TEC_INTERNAL));
        assert_eq!(trans_token(result.unwrap_err()), "tecINTERNAL");
    }

    #[test]
    fn vault_clawback_assets_to_clawback_runs_nonzero_conversion_order() {
        let steps = Rc::new(std::cell::RefCell::new(Vec::new()));

        let result = compute_vault_clawback_assets_to_clawback(
            &TestVault {
                assets_available: 10,
            },
            &4_i64,
            |_, _| true,
            |_| false,
            || 9_i64,
            |_| Some(1_i64),
            {
                let steps = Rc::clone(&steps);
                move |amount| {
                    steps
                        .borrow_mut()
                        .push(format!("assets_to_shares:{amount}"));
                    Ok::<_, ()>(Some(6_i64))
                }
            },
            {
                let steps = Rc::clone(&steps);
                move |amount| {
                    steps
                        .borrow_mut()
                        .push(format!("assets_to_shares_truncated:{amount}"));
                    Ok::<_, ()>(Some(5_i64))
                }
            },
            {
                let steps = Rc::clone(&steps);
                move |shares| {
                    steps
                        .borrow_mut()
                        .push(format!("shares_to_assets:{shares}"));
                    Ok::<_, ()>(Some(4_i64))
                }
            },
        );

        assert_eq!(
            result,
            Ok(VaultClawbackAssetsToClawback {
                assets_recovered: 4_i64,
                shares_destroyed: 6_i64,
            })
        );
        assert_eq!(
            steps.borrow().as_slice(),
            ["assets_to_shares:4", "shares_to_assets:6"]
        );
    }

    #[test]
    fn vault_clawback_assets_to_clawback_clamps_and_retries_with_truncated_shares() {
        let steps = Rc::new(std::cell::RefCell::new(Vec::new()));

        let result = compute_vault_clawback_assets_to_clawback(
            &TestVault {
                assets_available: 5,
            },
            &9_i64,
            |_, _| true,
            |_| false,
            || 9_i64,
            |_| Some(1_i64),
            {
                let steps = Rc::clone(&steps);
                move |amount| {
                    steps
                        .borrow_mut()
                        .push(format!("assets_to_shares:{amount}"));
                    Ok::<_, ()>(Some(8_i64))
                }
            },
            {
                let steps = Rc::clone(&steps);
                move |amount| {
                    steps
                        .borrow_mut()
                        .push(format!("assets_to_shares_truncated:{amount}"));
                    Ok::<_, ()>(Some(5_i64))
                }
            },
            {
                let steps = Rc::clone(&steps);
                let call = Cell::new(0_u32);
                move |shares| {
                    let next = call.get() + 1;
                    call.set(next);
                    steps
                        .borrow_mut()
                        .push(format!("shares_to_assets:{shares}"));
                    if next == 1 {
                        Ok::<_, ()>(Some(7_i64))
                    } else {
                        Ok::<_, ()>(Some(5_i64))
                    }
                }
            },
        );

        assert_eq!(
            result,
            Ok(VaultClawbackAssetsToClawback {
                assets_recovered: 5_i64,
                shares_destroyed: 5_i64,
            })
        );
        assert_eq!(
            steps.borrow().as_slice(),
            [
                "assets_to_shares:9",
                "shares_to_assets:8",
                "assets_to_shares_truncated:5",
                "shares_to_assets:5",
            ]
        );
    }

    #[test]
    fn vault_clawback_assets_to_clawback_rejects_invalid_post_clamp_rounding() {
        let result = compute_vault_clawback_assets_to_clawback(
            &TestVault {
                assets_available: 5,
            },
            &9_i64,
            |_, _| true,
            |_| false,
            || 9_i64,
            |_| Some(1_i64),
            |_| Ok::<_, ()>(Some(8_i64)),
            |_| Ok::<_, ()>(Some(5_i64)),
            {
                let call = Cell::new(0_u32);
                move |_| {
                    let next = call.get() + 1;
                    call.set(next);
                    if next == 1 {
                        Ok::<_, ()>(Some(7_i64))
                    } else {
                        Ok::<_, ()>(Some(6_i64))
                    }
                }
            },
        );

        assert_eq!(result, Err(Ter::TEC_INTERNAL));
        assert_eq!(trans_token(result.unwrap_err()), "tecINTERNAL");
    }

    #[test]
    fn vault_clawback_assets_to_clawback_maps_nonzero_overflow_to_path_dry() {
        let shares_to_assets_called = Cell::new(false);

        let result = compute_vault_clawback_assets_to_clawback(
            &TestVault {
                assets_available: 5,
            },
            &9_i64,
            |_, _| true,
            |_| false,
            || 9_i64,
            |_| Some(1_i64),
            |_| Err::<Option<i64>, &'static str>("overflow"),
            |_| Ok::<_, &'static str>(Some(5_i64)),
            |_| {
                shares_to_assets_called.set(true);
                Ok::<_, &'static str>(Some(4_i64))
            },
        );

        assert_eq!(result, Err(Ter::TEC_PATH_DRY));
        assert_eq!(trans_token(result.unwrap_err()), "tecPATH_DRY");
        assert!(!shares_to_assets_called.get());
    }
}
