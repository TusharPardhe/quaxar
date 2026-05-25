//! Current Rust helper mirroring the pre-transfer balance-snapshot reads
//! inside the LoanPay transactor.
//!
//! This module preserves the current deterministic sampling order around:
//!
//! - the earlier vault pseudo-account balance read that happens before the
//!   post-payment shaping helpers, and
//! - the later borrower, vault, and broker balance reads that happen after
//!   the vault update and before the transfer helper runs.

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoanPayPreTransferSnapshotFacts<AccountId, Asset, Amount> {
    pub account: AccountId,
    pub vault_pseudo_account: AccountId,
    pub broker_payee: AccountId,
    pub asset: Asset,
    pub pseudo_account_balance_before: Amount,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoanPayPreTransferSnapshotResult<Amount> {
    pub pseudo_account_balance_before: Amount,
    pub borrower_balance_before: Amount,
    pub vault_balance_before: Amount,
    pub broker_balance_before: Amount,
    pub borrower_is_vault_pseudo: bool,
    pub borrower_is_broker_payee: bool,
}

pub trait LoanPayPreTransferSnapshotSink {
    type AccountId;
    type Asset;
    type Amount;

    fn sample_balance(&mut self, account: &Self::AccountId, asset: &Self::Asset) -> Self::Amount;
}

pub fn compute_loan_pay_pre_transfer_snapshot<Sink>(
    sink: &mut Sink,
    facts: LoanPayPreTransferSnapshotFacts<Sink::AccountId, Sink::Asset, Sink::Amount>,
) -> LoanPayPreTransferSnapshotResult<Sink::Amount>
where
    Sink: LoanPayPreTransferSnapshotSink,
    Sink::AccountId: Clone + PartialEq,
    Sink::Amount: Clone,
{
    let borrower_balance_before = sink.sample_balance(&facts.account, &facts.asset);
    let vault_balance_before = sink.sample_balance(&facts.vault_pseudo_account, &facts.asset);
    let broker_balance_before = sink.sample_balance(&facts.broker_payee, &facts.asset);

    LoanPayPreTransferSnapshotResult {
        pseudo_account_balance_before: facts.pseudo_account_balance_before,
        borrower_balance_before,
        vault_balance_before,
        broker_balance_before,
        borrower_is_vault_pseudo: facts.account == facts.vault_pseudo_account,
        borrower_is_broker_payee: facts.account == facts.broker_payee,
    }
}

#[cfg(test)]
mod tests {
    use std::{cell::RefCell, rc::Rc};

    use super::{
        LoanPayPreTransferSnapshotFacts, LoanPayPreTransferSnapshotSink,
        compute_loan_pay_pre_transfer_snapshot,
    };

    #[derive(Debug, Clone, PartialEq, Eq)]
    struct TestSink {
        balances: std::collections::HashMap<&'static str, i64>,
        steps: Rc<RefCell<Vec<&'static str>>>,
    }

    impl LoanPayPreTransferSnapshotSink for TestSink {
        type AccountId = &'static str;
        type Asset = &'static str;
        type Amount = i64;

        fn sample_balance(
            &mut self,
            account: &Self::AccountId,
            _asset: &Self::Asset,
        ) -> Self::Amount {
            self.steps.borrow_mut().push(match *account {
                "vault-pseudo" => "sample_pseudo",
                "borrower" => "sample_borrower",
                "vault" => "sample_vault",
                "broker" => "sample_broker",
                _ => "sample_other",
            });
            *self.balances.get(account).unwrap_or(&0)
        }
    }

    #[test]
    fn loan_pay_pre_transfer_snapshot_samples_balances_in() {
        let mut sink = TestSink {
            balances: std::collections::HashMap::from([
                ("vault-pseudo", 10),
                ("borrower", 80),
                ("vault", 30),
                ("broker", 15),
            ]),
            steps: Rc::new(RefCell::new(Vec::new())),
        };

        let result = compute_loan_pay_pre_transfer_snapshot(
            &mut sink,
            LoanPayPreTransferSnapshotFacts {
                account: "borrower",
                vault_pseudo_account: "vault-pseudo",
                broker_payee: "broker",
                asset: "USD",
                pseudo_account_balance_before: 10,
            },
        );

        assert_eq!(
            sink.steps.borrow().as_slice(),
            &["sample_borrower", "sample_pseudo", "sample_broker"]
        );
        assert_eq!(result.pseudo_account_balance_before, 10);
        assert_eq!(result.borrower_balance_before, 80);
        assert_eq!(result.vault_balance_before, 10);
        assert_eq!(result.broker_balance_before, 15);
        assert!(!result.borrower_is_vault_pseudo);
        assert!(!result.borrower_is_broker_payee);
    }

    #[test]
    fn loan_pay_pre_transfer_snapshot_tracks_alias_flags() {
        let mut sink = TestSink {
            balances: std::collections::HashMap::from([("borrower", 80), ("vault", 30)]),
            steps: Rc::new(RefCell::new(Vec::new())),
        };

        let result = compute_loan_pay_pre_transfer_snapshot(
            &mut sink,
            LoanPayPreTransferSnapshotFacts {
                account: "borrower",
                vault_pseudo_account: "borrower",
                broker_payee: "borrower",
                asset: "USD",
                pseudo_account_balance_before: 80,
            },
        );

        assert!(result.borrower_is_vault_pseudo);
        assert!(result.borrower_is_broker_payee);
        assert_eq!(result.pseudo_account_balance_before, 80);
        assert_eq!(result.borrower_balance_before, 80);
        assert_eq!(result.vault_balance_before, 80);
        assert_eq!(result.broker_balance_before, 80);
    }
}
