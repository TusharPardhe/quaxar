//! Integration tests that pin the narrowed Rust
//! `LoanSet.cpp::doApply()` broker-balance update shell to the current C++
//! behavior.

use protocol::{Ter, trans_token};
use tx::{
    LoanSetDoApplyBrokerUpdate, LoanSetDoApplyBrokerUpdateSink, run_loan_set_do_apply_broker_update,
};

struct RecordingSink {
    debt_total: i64,
    owner_count: u32,
    loan_sequence: u32,
    steps: Vec<String>,
}

impl RecordingSink {
    fn new(debt_total: i64, owner_count: u32, loan_sequence: u32) -> Self {
        Self {
            debt_total,
            owner_count,
            loan_sequence,
            steps: Vec::new(),
        }
    }
}

impl LoanSetDoApplyBrokerUpdateSink for RecordingSink {
    type DebtDelta = i64;
    type Asset = &'static str;
    type Scale = u32;

    fn adjust_debt_total(
        &mut self,
        delta: Self::DebtDelta,
        asset: Self::Asset,
        scale: Self::Scale,
    ) {
        self.debt_total += delta;
        self.steps.push(format!(
            "adjust_debt_total delta={delta} asset={asset} scale={scale}"
        ));
    }

    fn increment_owner_count(&mut self) {
        self.owner_count += 1;
        self.steps.push("increment_owner_count".to_string());
    }

    fn increment_loan_sequence(&mut self) -> u32 {
        self.loan_sequence = self.loan_sequence.wrapping_add(1);
        self.steps
            .push(format!("increment_loan_sequence={}", self.loan_sequence));
        self.loan_sequence
    }

    fn update_broker(&mut self) {
        self.steps.push("update_broker".to_string());
    }
}

#[test]
fn tx_loan_set_do_apply_broker_update_uses_current_on_success() {
    let mut sink = RecordingSink::new(500, 2, 7);

    let result = run_loan_set_do_apply_broker_update(
        &mut sink,
        LoanSetDoApplyBrokerUpdate {
            new_debt_delta: 304,
            vault_asset: "USD",
            vault_scale: 6,
        },
    );

    assert_eq!(result, Ter::TES_SUCCESS);
    assert_eq!(sink.debt_total, 804);
    assert_eq!(sink.owner_count, 3);
    assert_eq!(sink.loan_sequence, 8);
    assert_eq!(
        sink.steps,
        vec![
            "adjust_debt_total delta=304 asset=USD scale=6",
            "increment_owner_count",
            "increment_loan_sequence=8",
            "update_broker",
        ]
    );
}

#[test]
fn tx_loan_set_do_apply_broker_update_returns_max_sequence_reached_before_update() {
    let mut sink = RecordingSink::new(10, 0, u32::MAX);

    let result = run_loan_set_do_apply_broker_update(
        &mut sink,
        LoanSetDoApplyBrokerUpdate {
            new_debt_delta: 5,
            vault_asset: "USD",
            vault_scale: 6,
        },
    );

    assert_eq!(result, Ter::TEC_MAX_SEQUENCE_REACHED);
    assert_eq!(trans_token(result), "tecMAX_SEQUENCE_REACHED");
    assert_eq!(sink.debt_total, 15);
    assert_eq!(sink.owner_count, 1);
    assert_eq!(sink.loan_sequence, 0);
    assert_eq!(
        sink.steps,
        vec![
            "adjust_debt_total delta=5 asset=USD scale=6",
            "increment_owner_count",
            "increment_loan_sequence=0",
        ]
    );
}

#[test]
fn tx_loan_set_do_apply_broker_update_increments_owner_count_once() {
    let mut sink = RecordingSink::new(0, 41, 1);

    let result = run_loan_set_do_apply_broker_update(
        &mut sink,
        LoanSetDoApplyBrokerUpdate {
            new_debt_delta: 0,
            vault_asset: "XRP",
            vault_scale: 0,
        },
    );

    assert_eq!(result, Ter::TES_SUCCESS);
    assert_eq!(sink.owner_count, 42);
    assert_eq!(sink.steps[1], "increment_owner_count");
}
