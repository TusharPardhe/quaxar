//! Guard and vault-update block inside the reference implementation.
//!
//! This ports the exact current behavior around:
//!
//! - rejecting insufficient held shares with `tecINSUFFICIENT_FUNDS`,
//! - asserting the current unrealized-loss invariant before the asset-available
//!   guard,
//! - rejecting insufficient vault assets with `tecINSUFFICIENT_FUNDS`,
//! - subtracting withdrawn assets from `AssetsTotal` first and
//!   `AssetsAvailable` second,
//! - and updating the vault after those mutations.

use std::ops::Sub;

use protocol::Ter;

pub const VAULT_WITHDRAW_DO_APPLY_VAULT_UPDATE_ASSERT_MESSAGE: &str =
    "xrpl::VaultWithdraw::doApply : loss and assets do balance";

pub trait VaultWithdrawDoApplyVaultUpdateSink {
    type Amount;

    fn assets_available(&self) -> &Self::Amount;
    fn assets_total(&self) -> &Self::Amount;
    fn loss_unrealized(&self) -> &Self::Amount;
    fn subtract_assets_total(&mut self, value: Self::Amount);
    fn subtract_assets_available(&mut self, value: Self::Amount);
    fn update_vault(&mut self);
}

pub fn run_vault_withdraw_do_apply_vault_update<Sink, ShareAmount, HasEnoughShares>(
    sink: &mut Sink,
    shares_redeemed: &ShareAmount,
    assets_withdrawn: &Sink::Amount,
    has_enough_shares: HasEnoughShares,
) -> Ter
where
    Sink: VaultWithdrawDoApplyVaultUpdateSink,
    Sink::Amount: Clone + PartialOrd + Sub<Output = Sink::Amount>,
    HasEnoughShares: FnOnce(&ShareAmount) -> bool,
{
    if !has_enough_shares(shares_redeemed) {
        return Ter::TEC_INSUFFICIENT_FUNDS;
    }

    let assets_available = sink.assets_available().clone();
    let assets_total = sink.assets_total().clone();
    let loss_unrealized = sink.loss_unrealized().clone();
    assert!(
        loss_unrealized <= (assets_total.clone() - assets_available.clone()),
        "{VAULT_WITHDRAW_DO_APPLY_VAULT_UPDATE_ASSERT_MESSAGE}"
    );

    if assets_available < *assets_withdrawn {
        return Ter::TEC_INSUFFICIENT_FUNDS;
    }

    sink.subtract_assets_total(assets_withdrawn.clone());
    sink.subtract_assets_available(assets_withdrawn.clone());
    sink.update_vault();
    Ter::TES_SUCCESS
}

#[cfg(test)]
mod tests {
    use std::panic::{AssertUnwindSafe, catch_unwind};

    use protocol::{Ter, trans_token};

    use super::{
        VAULT_WITHDRAW_DO_APPLY_VAULT_UPDATE_ASSERT_MESSAGE, VaultWithdrawDoApplyVaultUpdateSink,
        run_vault_withdraw_do_apply_vault_update,
    };

    struct RecordingSink {
        assets_available: i64,
        assets_total: i64,
        loss_unrealized: i64,
        steps: Vec<String>,
    }

    impl RecordingSink {
        fn new(assets_available: i64, assets_total: i64, loss_unrealized: i64) -> Self {
            Self {
                assets_available,
                assets_total,
                loss_unrealized,
                steps: Vec::new(),
            }
        }
    }

    impl VaultWithdrawDoApplyVaultUpdateSink for RecordingSink {
        type Amount = i64;

        fn assets_available(&self) -> &Self::Amount {
            &self.assets_available
        }

        fn assets_total(&self) -> &Self::Amount {
            &self.assets_total
        }

        fn loss_unrealized(&self) -> &Self::Amount {
            &self.loss_unrealized
        }

        fn subtract_assets_total(&mut self, value: Self::Amount) {
            self.assets_total -= value;
            self.steps.push(format!("assets_total-={value}"));
        }

        fn subtract_assets_available(&mut self, value: Self::Amount) {
            self.assets_available -= value;
            self.steps.push(format!("assets_available-={value}"));
        }

        fn update_vault(&mut self) {
            self.steps.push("update_vault".to_string());
        }
    }

    #[test]
    fn vault_withdraw_do_apply_vault_update_rejects_insufficient_shares_first() {
        let mut sink = RecordingSink::new(80, 100, 10);

        let result =
            run_vault_withdraw_do_apply_vault_update(&mut sink, &30_i64, &25_i64, |_| false);

        assert_eq!(result, Ter::TEC_INSUFFICIENT_FUNDS);
        assert_eq!(trans_token(result), "tecINSUFFICIENT_FUNDS");
        assert!(sink.steps.is_empty());
    }

    #[test]
    fn vault_withdraw_do_apply_vault_update_panics_on_loss_invariant_before_asset_guard() {
        let mut sink = RecordingSink::new(80, 100, 25);

        let panic = catch_unwind(AssertUnwindSafe(|| {
            run_vault_withdraw_do_apply_vault_update(&mut sink, &30_i64, &25_i64, |_| true);
        }))
        .expect_err("invalid loss invariant should panic");

        let message = panic
            .downcast_ref::<String>()
            .map(String::as_str)
            .or_else(|| panic.downcast_ref::<&'static str>().copied())
            .expect("panic payload should be a string");

        assert!(message.contains(VAULT_WITHDRAW_DO_APPLY_VAULT_UPDATE_ASSERT_MESSAGE));
        assert!(sink.steps.is_empty());
    }

    #[test]
    fn vault_withdraw_do_apply_vault_update_rejects_insufficient_vault_assets_before_update() {
        let mut sink = RecordingSink::new(20, 100, 10);

        let result =
            run_vault_withdraw_do_apply_vault_update(&mut sink, &30_i64, &25_i64, |_| true);

        assert_eq!(result, Ter::TEC_INSUFFICIENT_FUNDS);
        assert_eq!(trans_token(result), "tecINSUFFICIENT_FUNDS");
        assert!(sink.steps.is_empty());
    }

    #[test]
    fn vault_withdraw_do_apply_vault_update_uses_current_on_success() {
        let mut sink = RecordingSink::new(80, 100, 10);

        let result =
            run_vault_withdraw_do_apply_vault_update(&mut sink, &30_i64, &25_i64, |_| true);

        assert_eq!(result, Ter::TES_SUCCESS);
        assert_eq!(sink.assets_total, 75);
        assert_eq!(sink.assets_available, 55);
        assert_eq!(
            sink.steps,
            vec!["assets_total-=25", "assets_available-=25", "update_vault"]
        );
    }
}
