//! Vault-total update block inside the reference implementation.
//!
//! This ports the exact current behavior around:
//!
//! - asserting that the created shares and deposited assets are distinct,
//! - adding the deposited assets to `AssetsTotal`,
//! - adding the same deposited assets to `AssetsAvailable`,
//! - updating the vault before the limit check,
//! - and returning `tecLIMIT_EXCEEDED` only after that update when the
//!   post-update total exceeds a non-zero maximum.

use protocol::Ter;

pub const VAULT_DEPOSIT_DO_APPLY_VAULT_UPDATE_ASSERT_MESSAGE: &str =
    "xrpl::VaultDeposit::doApply : assets are not shares";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VaultDepositDoApplyVaultUpdate<Amount> {
    pub assets_deposited: Amount,
    pub maximum: Option<Amount>,
    pub shares_and_assets_are_distinct: bool,
}

pub trait VaultDepositDoApplyVaultUpdateSink {
    type Amount;

    fn add_assets_total(&mut self, value: Self::Amount);
    fn add_assets_available(&mut self, value: Self::Amount);
    fn assets_total(&self) -> &Self::Amount;
    fn update_vault(&mut self);
}

pub fn run_vault_deposit_do_apply_vault_update<Sink>(
    sink: &mut Sink,
    update: VaultDepositDoApplyVaultUpdate<Sink::Amount>,
) -> Ter
where
    Sink: VaultDepositDoApplyVaultUpdateSink,
    Sink::Amount: Clone + PartialOrd,
{
    assert!(
        update.shares_and_assets_are_distinct,
        "{VAULT_DEPOSIT_DO_APPLY_VAULT_UPDATE_ASSERT_MESSAGE}"
    );

    sink.add_assets_total(update.assets_deposited.clone());
    sink.add_assets_available(update.assets_deposited);
    sink.update_vault();

    if let Some(maximum) = update.maximum.as_ref()
        && sink.assets_total() > maximum
    {
        return Ter::TEC_LIMIT_EXCEEDED;
    }

    Ter::TES_SUCCESS
}

#[cfg(test)]
mod tests {
    use std::panic::{AssertUnwindSafe, catch_unwind};

    use protocol::{Ter, trans_token};

    use super::{
        VAULT_DEPOSIT_DO_APPLY_VAULT_UPDATE_ASSERT_MESSAGE, VaultDepositDoApplyVaultUpdate,
        VaultDepositDoApplyVaultUpdateSink, run_vault_deposit_do_apply_vault_update,
    };

    struct RecordingSink {
        assets_total: i64,
        assets_available: i64,
        steps: Vec<String>,
    }

    impl RecordingSink {
        fn new(assets_total: i64, assets_available: i64) -> Self {
            Self {
                assets_total,
                assets_available,
                steps: Vec::new(),
            }
        }
    }

    impl VaultDepositDoApplyVaultUpdateSink for RecordingSink {
        type Amount = i64;

        fn add_assets_total(&mut self, value: Self::Amount) {
            self.assets_total += value;
            self.steps.push(format!("assets_total+={value}"));
        }

        fn add_assets_available(&mut self, value: Self::Amount) {
            self.assets_available += value;
            self.steps.push(format!("assets_available+={value}"));
        }

        fn assets_total(&self) -> &Self::Amount {
            &self.assets_total
        }

        fn update_vault(&mut self) {
            self.steps.push("update_vault".to_string());
        }
    }

    #[test]
    fn vault_deposit_do_apply_vault_update_uses_current_on_success() {
        let mut sink = RecordingSink::new(100, 25);

        let result = run_vault_deposit_do_apply_vault_update(
            &mut sink,
            VaultDepositDoApplyVaultUpdate {
                assets_deposited: 30,
                maximum: Some(150),
                shares_and_assets_are_distinct: true,
            },
        );

        assert_eq!(result, Ter::TES_SUCCESS);
        assert_eq!(sink.assets_total, 130);
        assert_eq!(sink.assets_available, 55);
        assert_eq!(
            sink.steps,
            vec!["assets_total+=30", "assets_available+=30", "update_vault",]
        );
    }

    #[test]
    fn vault_deposit_do_apply_vault_update_skips_limit_when_maximum_is_zero() {
        let mut sink = RecordingSink::new(100, 25);

        let result = run_vault_deposit_do_apply_vault_update(
            &mut sink,
            VaultDepositDoApplyVaultUpdate {
                assets_deposited: 60,
                maximum: None,
                shares_and_assets_are_distinct: true,
            },
        );

        assert_eq!(result, Ter::TES_SUCCESS);
        assert_eq!(sink.assets_total, 160);
        assert_eq!(sink.steps.last(), Some(&"update_vault".to_string()));
    }

    #[test]
    fn vault_deposit_do_apply_vault_update_allows_equal_maximum() {
        let mut sink = RecordingSink::new(100, 25);

        let result = run_vault_deposit_do_apply_vault_update(
            &mut sink,
            VaultDepositDoApplyVaultUpdate {
                assets_deposited: 50,
                maximum: Some(150),
                shares_and_assets_are_distinct: true,
            },
        );

        assert_eq!(result, Ter::TES_SUCCESS);
        assert_eq!(sink.assets_total, 150);
    }

    #[test]
    fn vault_deposit_do_apply_vault_update_returns_limit_exceeded_after_update() {
        let mut sink = RecordingSink::new(100, 25);

        let result = run_vault_deposit_do_apply_vault_update(
            &mut sink,
            VaultDepositDoApplyVaultUpdate {
                assets_deposited: 60,
                maximum: Some(150),
                shares_and_assets_are_distinct: true,
            },
        );

        assert_eq!(result, Ter::TEC_LIMIT_EXCEEDED);
        assert_eq!(trans_token(result), "tecLIMIT_EXCEEDED");
        assert_eq!(sink.assets_total, 160);
        assert_eq!(sink.assets_available, 85);
        assert_eq!(
            sink.steps,
            vec!["assets_total+=60", "assets_available+=60", "update_vault",]
        );
    }

    #[test]
    fn vault_deposit_do_apply_vault_update_panics_before_mutation_when_assets_are_shares() {
        let mut sink = RecordingSink::new(100, 25);

        let panic = catch_unwind(AssertUnwindSafe(|| {
            run_vault_deposit_do_apply_vault_update(
                &mut sink,
                VaultDepositDoApplyVaultUpdate {
                    assets_deposited: 60,
                    maximum: Some(150),
                    shares_and_assets_are_distinct: false,
                },
            );
        }))
        .expect_err("matching share and asset types should panic");

        let message = panic
            .downcast_ref::<String>()
            .map(String::as_str)
            .or_else(|| panic.downcast_ref::<&'static str>().copied())
            .expect("panic payload should be a string");

        assert!(message.contains(VAULT_DEPOSIT_DO_APPLY_VAULT_UPDATE_ASSERT_MESSAGE));
        assert_eq!(sink.assets_total, 100);
        assert_eq!(sink.assets_available, 25);
        assert!(sink.steps.is_empty());
    }
}
