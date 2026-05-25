//! Exchange-computation core inside the reference implementation.
//!
//! This ports the exact current behavior around:
//!
//! - mapping a missing `assetsToSharesDeposit(...)` result to `tecINTERNAL`,
//! - rejecting zero created shares with `tecPRECISION_LOSS`,
//! - mapping a missing `sharesToAssetsDeposit(...)` result to `tecINTERNAL`,
//! - rejecting computed assets that exceed the offered amount with
//!   `tecINTERNAL`,
//! - and mapping overflow from either helper to `tecPATH_DRY`.

use protocol::Ter;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VaultDepositDoApplyExchange<ShareAmount, AssetAmount> {
    pub shares_created: ShareAmount,
    pub assets_deposited: AssetAmount,
}

pub fn run_vault_deposit_do_apply_exchange<
    Amount,
    ShareAmount,
    AssetAmount,
    Overflow,
    AssetsToShares,
    SharesToAssets,
    IsZeroShares,
    AssetsExceedAmount,
>(
    amount: &Amount,
    assets_to_shares: AssetsToShares,
    shares_to_assets: SharesToAssets,
    is_zero_shares: IsZeroShares,
    assets_exceed_amount: AssetsExceedAmount,
) -> Result<VaultDepositDoApplyExchange<ShareAmount, AssetAmount>, Ter>
where
    AssetsToShares: FnOnce(&Amount) -> Result<Option<ShareAmount>, Overflow>,
    SharesToAssets: FnOnce(&ShareAmount) -> Result<Option<AssetAmount>, Overflow>,
    IsZeroShares: FnOnce(&ShareAmount) -> bool,
    AssetsExceedAmount: FnOnce(&AssetAmount, &Amount) -> bool,
{
    let shares_created = match assets_to_shares(amount) {
        Ok(Some(shares_created)) => shares_created,
        Ok(None) => return Err(Ter::TEC_INTERNAL),
        Err(_) => return Err(Ter::TEC_PATH_DRY),
    };

    if is_zero_shares(&shares_created) {
        return Err(Ter::TEC_PRECISION_LOSS);
    }

    let assets_deposited = match shares_to_assets(&shares_created) {
        Ok(Some(assets_deposited)) => assets_deposited,
        Ok(None) => return Err(Ter::TEC_INTERNAL),
        Err(_) => return Err(Ter::TEC_PATH_DRY),
    };

    if assets_exceed_amount(&assets_deposited, amount) {
        return Err(Ter::TEC_INTERNAL);
    }

    Ok(VaultDepositDoApplyExchange {
        shares_created,
        assets_deposited,
    })
}

#[cfg(test)]
mod tests {
    use std::{cell::Cell, rc::Rc};

    use protocol::{Ter, trans_token};

    use super::{VaultDepositDoApplyExchange, run_vault_deposit_do_apply_exchange};

    #[test]
    fn vault_deposit_do_apply_exchange_returns_internal_when_assets_to_shares_is_missing() {
        let shares_to_assets_called = Cell::new(false);

        let result = run_vault_deposit_do_apply_exchange(
            &50_i64,
            |_| Ok::<_, ()>(None::<i64>),
            |_| {
                shares_to_assets_called.set(true);
                Ok::<_, ()>(Some(50_i64))
            },
            |_| false,
            |_, _| false,
        );

        assert_eq!(result, Err(Ter::TEC_INTERNAL));
        assert_eq!(trans_token(result.unwrap_err()), "tecINTERNAL");
        assert!(!shares_to_assets_called.get());
    }

    #[test]
    fn vault_deposit_do_apply_exchange_rejects_zero_shares() {
        let shares_to_assets_called = Cell::new(false);

        let result = run_vault_deposit_do_apply_exchange(
            &50_i64,
            |_| Ok::<_, ()>(Some(0_i64)),
            |_| {
                shares_to_assets_called.set(true);
                Ok::<_, ()>(Some(50_i64))
            },
            |shares_created| *shares_created == 0,
            |_, _| false,
        );

        assert_eq!(result, Err(Ter::TEC_PRECISION_LOSS));
        assert_eq!(trans_token(result.unwrap_err()), "tecPRECISION_LOSS");
        assert!(!shares_to_assets_called.get());
    }

    #[test]
    fn vault_deposit_do_apply_exchange_returns_internal_when_shares_to_assets_is_missing() {
        let result = run_vault_deposit_do_apply_exchange(
            &50_i64,
            |_| Ok::<_, ()>(Some(10_i64)),
            |_| Ok::<_, ()>(None::<i64>),
            |_| false,
            |_, _| false,
        );

        assert_eq!(result, Err(Ter::TEC_INTERNAL));
        assert_eq!(trans_token(result.unwrap_err()), "tecINTERNAL");
    }

    #[test]
    fn vault_deposit_do_apply_exchange_returns_internal_when_assets_exceed_offer() {
        let result = run_vault_deposit_do_apply_exchange(
            &50_i64,
            |_| Ok::<_, ()>(Some(10_i64)),
            |_| Ok::<_, ()>(Some(60_i64)),
            |_| false,
            |assets_deposited, amount| *assets_deposited > *amount,
        );

        assert_eq!(result, Err(Ter::TEC_INTERNAL));
        assert_eq!(trans_token(result.unwrap_err()), "tecINTERNAL");
    }

    #[test]
    fn vault_deposit_do_apply_exchange_maps_assets_to_shares_overflow_to_path_dry() {
        let shares_to_assets_called = Cell::new(false);

        let result = run_vault_deposit_do_apply_exchange(
            &50_i64,
            |_| Err::<Option<i64>, &'static str>("overflow"),
            |_| {
                shares_to_assets_called.set(true);
                Ok::<_, &'static str>(Some(50_i64))
            },
            |_| false,
            |_, _| false,
        );

        assert_eq!(result, Err(Ter::TEC_PATH_DRY));
        assert_eq!(trans_token(result.unwrap_err()), "tecPATH_DRY");
        assert!(!shares_to_assets_called.get());
    }

    #[test]
    fn vault_deposit_do_apply_exchange_maps_shares_to_assets_overflow_to_path_dry() {
        let result = run_vault_deposit_do_apply_exchange(
            &50_i64,
            |_| Ok::<_, &'static str>(Some(10_i64)),
            |_| Err::<Option<i64>, &'static str>("overflow"),
            |_| false,
            |_, _| false,
        );

        assert_eq!(result, Err(Ter::TEC_PATH_DRY));
        assert_eq!(trans_token(result.unwrap_err()), "tecPATH_DRY");
    }

    #[test]
    fn vault_deposit_do_apply_exchange_runs_helpers_in_current_on_success() {
        let steps = Rc::new(std::cell::RefCell::new(Vec::new()));

        let result = run_vault_deposit_do_apply_exchange(
            &50_i64,
            {
                let steps = Rc::clone(&steps);
                move |amount| {
                    steps.borrow_mut().push("assets-to-shares");
                    assert_eq!(*amount, 50);
                    Ok::<_, ()>(Some(10_i64))
                }
            },
            {
                let steps = Rc::clone(&steps);
                move |shares_created| {
                    steps.borrow_mut().push("shares-to-assets");
                    assert_eq!(*shares_created, 10);
                    Ok::<_, ()>(Some(49_i64))
                }
            },
            |_| false,
            |assets_deposited, amount| *assets_deposited > *amount,
        );

        assert_eq!(
            result,
            Ok(VaultDepositDoApplyExchange {
                shares_created: 10_i64,
                assets_deposited: 49_i64,
            })
        );
        assert_eq!(
            steps.borrow().as_slice(),
            ["assets-to-shares", "shares-to-assets"]
        );
    }
}
