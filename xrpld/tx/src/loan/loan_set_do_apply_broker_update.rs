//! Current Rust helper mirroring the post-vault-update
//! the LoanSet transactor broker-balance update shell.
//!
//! This module preserves the deterministic behavior around:
//!
//! - adjusting `DebtTotal` from `newDebtDelta`, `vaultAsset`, and `vaultScale`,
//! - incrementing the broker owner count for the new outstanding loan,
//! - incrementing `LoanSequence`,
//! - returning `tecMAX_SEQUENCE_REACHED` on rollover before `view.update`, and
//! - updating the broker only when the sequence did not roll over.

use protocol::Ter;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoanSetDoApplyBrokerUpdate<DebtDelta, Asset, Scale> {
    pub new_debt_delta: DebtDelta,
    pub vault_asset: Asset,
    pub vault_scale: Scale,
}

pub trait LoanSetDoApplyBrokerUpdateSink {
    type DebtDelta;
    type Asset;
    type Scale;

    fn adjust_debt_total(&mut self, delta: Self::DebtDelta, asset: Self::Asset, scale: Self::Scale);
    fn increment_owner_count(&mut self);
    fn increment_loan_sequence(&mut self) -> u32;
    fn update_broker(&mut self);
}

pub fn run_loan_set_do_apply_broker_update<Sink>(
    sink: &mut Sink,
    update: LoanSetDoApplyBrokerUpdate<Sink::DebtDelta, Sink::Asset, Sink::Scale>,
) -> Ter
where
    Sink: LoanSetDoApplyBrokerUpdateSink,
{
    sink.adjust_debt_total(
        update.new_debt_delta,
        update.vault_asset,
        update.vault_scale,
    );
    sink.increment_owner_count();

    if sink.increment_loan_sequence() == 0 {
        return Ter::TEC_MAX_SEQUENCE_REACHED;
    }

    sink.update_broker();
    Ter::TES_SUCCESS
}

#[cfg(test)]
mod tests {
    use protocol::trans_token;

    use super::{
        LoanSetDoApplyBrokerUpdate, LoanSetDoApplyBrokerUpdateSink,
        run_loan_set_do_apply_broker_update,
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
    fn loan_set_do_apply_broker_update_uses_current_on_success() {
        let mut sink = RecordingSink::new(500, 2, 7);

        let result = run_loan_set_do_apply_broker_update(
            &mut sink,
            LoanSetDoApplyBrokerUpdate {
                new_debt_delta: 304,
                vault_asset: "USD",
                vault_scale: 6,
            },
        );

        assert_eq!(result, protocol::Ter::TES_SUCCESS);
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
    fn loan_set_do_apply_broker_update_returns_max_sequence_reached_before_update() {
        let mut sink = RecordingSink::new(10, 0, u32::MAX);

        let result = run_loan_set_do_apply_broker_update(
            &mut sink,
            LoanSetDoApplyBrokerUpdate {
                new_debt_delta: 5,
                vault_asset: "USD",
                vault_scale: 6,
            },
        );

        assert_eq!(result, protocol::Ter::TEC_MAX_SEQUENCE_REACHED);
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
    fn loan_set_do_apply_broker_update_increments_owner_count_once() {
        let mut sink = RecordingSink::new(0, 41, 1);

        let result = run_loan_set_do_apply_broker_update(
            &mut sink,
            LoanSetDoApplyBrokerUpdate {
                new_debt_delta: 0,
                vault_asset: "XRP",
                vault_scale: 0,
            },
        );

        assert_eq!(result, protocol::Ter::TES_SUCCESS);
        assert_eq!(sink.owner_count, 42);
        assert_eq!(sink.steps[1], "increment_owner_count");
    }
}
