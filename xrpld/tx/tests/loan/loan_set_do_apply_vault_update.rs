//! Integration tests that pin the narrowed Rust
//! `LoanSet.cpp::doApply()` vault-balance update shell to the current C++
//! behavior.

use std::panic::{AssertUnwindSafe, catch_unwind};

use tx::{
    LOAN_SET_DO_APPLY_VAULT_UPDATE_ASSERT_MESSAGE, LoanSetDoApplyVaultUpdate,
    LoanSetDoApplyVaultUpdateSink, run_loan_set_do_apply_vault_update,
};

struct RecordingSink {
    assets_available: i64,
    assets_total: i64,
    steps: Vec<String>,
}

impl RecordingSink {
    fn new(assets_available: i64, assets_total: i64) -> Self {
        Self {
            assets_available,
            assets_total,
            steps: Vec::new(),
        }
    }
}

impl LoanSetDoApplyVaultUpdateSink for RecordingSink {
    type Amount = i64;

    fn subtract_assets_available(&mut self, value: Self::Amount) {
        self.assets_available -= value;
        self.steps.push(format!("assets_available-={value}"));
    }

    fn add_assets_total(&mut self, value: Self::Amount) {
        self.assets_total += value;
        self.steps.push(format!("assets_total+={value}"));
    }

    fn assets_available(&self) -> &Self::Amount {
        &self.assets_available
    }

    fn assets_total(&self) -> &Self::Amount {
        &self.assets_total
    }

    fn update_vault(&mut self) {
        self.steps.push("update_vault".to_string());
    }
}

#[test]
fn tx_loan_set_do_apply_vault_update_uses_current_cpp_mutation_order() {
    let mut sink = RecordingSink::new(1_000, 1_200);

    run_loan_set_do_apply_vault_update(
        &mut sink,
        LoanSetDoApplyVaultUpdate {
            principal_requested: 250,
            interest_due: 50,
        },
    );

    assert_eq!(sink.assets_available, 750);
    assert_eq!(sink.assets_total, 1_250);
    assert_eq!(
        sink.steps,
        vec!["assets_available-=250", "assets_total+=50", "update_vault",]
    );
}

#[test]
fn tx_loan_set_do_apply_vault_update_allows_equal_available_and_total() {
    let mut sink = RecordingSink::new(500, 400);

    run_loan_set_do_apply_vault_update(
        &mut sink,
        LoanSetDoApplyVaultUpdate {
            principal_requested: 200,
            interest_due: 100,
        },
    );

    assert_eq!(sink.assets_available, 300);
    assert_eq!(sink.assets_total, 500);
    assert_eq!(sink.steps.last(), Some(&"update_vault".to_string()));
}

#[test]
fn tx_loan_set_do_apply_vault_update_panics_before_update_when_invariant_breaks() {
    let mut sink = RecordingSink::new(1_000, 100);

    let panic = catch_unwind(AssertUnwindSafe(|| {
        run_loan_set_do_apply_vault_update(
            &mut sink,
            LoanSetDoApplyVaultUpdate {
                principal_requested: 50,
                interest_due: 25,
            },
        );
    }))
    .expect_err("invariant break should panic");

    let message = panic
        .downcast_ref::<String>()
        .map(String::as_str)
        .or_else(|| panic.downcast_ref::<&'static str>().copied())
        .expect("panic payload should be a string");

    assert!(message.contains(LOAN_SET_DO_APPLY_VAULT_UPDATE_ASSERT_MESSAGE));
    assert_eq!(sink.assets_available, 950);
    assert_eq!(sink.assets_total, 125);
    assert_eq!(
        sink.steps,
        vec!["assets_available-=50", "assets_total+=25",]
    );
}
