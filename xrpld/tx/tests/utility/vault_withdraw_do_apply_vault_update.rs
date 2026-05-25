//! Integration tests that pin the narrowed Rust
//! `VaultWithdraw.cpp::doApply()` guard and vault-update block to the current
//! C++ behavior.

use std::panic::{AssertUnwindSafe, catch_unwind};

use protocol::{Ter, trans_token};
use tx::{
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

    let result = run_vault_withdraw_do_apply_vault_update(&mut sink, &30_i64, &25_i64, |_| false);

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

    let result = run_vault_withdraw_do_apply_vault_update(&mut sink, &30_i64, &25_i64, |_| true);

    assert_eq!(result, Ter::TEC_INSUFFICIENT_FUNDS);
    assert_eq!(trans_token(result), "tecINSUFFICIENT_FUNDS");
    assert!(sink.steps.is_empty());
}

#[test]
fn vault_withdraw_do_apply_vault_update_uses_current_on_success() {
    let mut sink = RecordingSink::new(80, 100, 10);

    let result = run_vault_withdraw_do_apply_vault_update(&mut sink, &30_i64, &25_i64, |_| true);

    assert_eq!(result, Ter::TES_SUCCESS);
    assert_eq!(sink.assets_total, 75);
    assert_eq!(sink.assets_available, 55);
    assert_eq!(
        sink.steps,
        vec!["assets_total-=25", "assets_available-=25", "update_vault"]
    );
}
