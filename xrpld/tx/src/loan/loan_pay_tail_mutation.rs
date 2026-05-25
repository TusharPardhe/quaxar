//! Current Rust helper mirroring the post-payment mutation block inside
//! the LoanPay transactor.
//!
//! This module preserves the deterministic ordering around:
//!
//! - `view.update(vaultSle)`,
//! - vault `assetsAvailable` / `assetsTotal` mutation,
//! - the `assetsAvailable <= assetsTotal` guard,
//! - the fallback first-loss cover increment, and
//! - `associateAsset(...)` ordering for loan, broker, and vault objects.

use protocol::Ter;

use crate::{LoanPayDoApplyBroker, LoanPayDoApplyLoan, LoanPayDoApplyVault};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoanPayTailMutationFacts<Asset, Amount> {
    pub asset: Asset,
    pub payment_value_change: Amount,
    pub total_paid_to_vault_rounded: Amount,
    pub total_paid_to_broker: Amount,
    pub send_broker_fee_to_owner: bool,
}

pub trait LoanPayTailMutationSink {
    type Vault: LoanPayDoApplyVault;

    fn update_vault(&mut self, vault: &Self::Vault);
}

pub fn run_loan_pay_tail_mutation<Sink, Loan, Broker, Vault>(
    sink: &mut Sink,
    loan: &mut Loan,
    broker: &mut Broker,
    vault: &mut Vault,
    facts: LoanPayTailMutationFacts<Loan::Asset, Vault::Amount>,
) -> Ter
where
    Sink: LoanPayTailMutationSink<Vault = Vault>,
    Loan: LoanPayDoApplyLoan,
    Broker: LoanPayDoApplyBroker<Asset = Loan::Asset, Amount = Vault::Amount>,
    Vault: LoanPayDoApplyVault<Asset = Loan::Asset>,
    Loan::Asset: Clone,
    Vault::Amount: Clone,
{
    sink.update_vault(vault);

    vault.add_assets_available(facts.total_paid_to_vault_rounded.clone());
    vault.add_assets_total(facts.payment_value_change);

    if vault.assets_available_exceeds_total() {
        return Ter::TEC_INTERNAL;
    }

    if !facts.send_broker_fee_to_owner {
        broker.add_cover_available(facts.total_paid_to_broker);
    }

    loan.associate_asset(&facts.asset);
    broker.associate_asset(&facts.asset);
    vault.associate_asset(&facts.asset);

    Ter::TES_SUCCESS
}

#[cfg(test)]
mod tests {
    use std::{cell::RefCell, rc::Rc};

    use protocol::Ter;

    use super::{LoanPayTailMutationFacts, LoanPayTailMutationSink, run_loan_pay_tail_mutation};
    use crate::{LoanPayDoApplyBroker, LoanPayDoApplyLoan, LoanPayDoApplyVault};

    #[derive(Debug, Clone, PartialEq, Eq)]
    struct TestLoan {
        broker_id: &'static str,
        steps: Rc<RefCell<Vec<&'static str>>>,
    }

    impl LoanPayDoApplyLoan for TestLoan {
        type BrokerId = &'static str;
        type Asset = &'static str;

        fn broker_id(&self) -> &Self::BrokerId {
            &self.broker_id
        }

        fn scale(&self) -> i32 {
            0
        }

        fn is_impaired(&self) -> bool {
            false
        }

        fn associate_asset(&mut self, _asset: &Self::Asset) {
            self.steps.borrow_mut().push("associate_loan_asset");
        }
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    struct TestBroker {
        owner: &'static str,
        pseudo_account: &'static str,
        vault_id: &'static str,
        debt_total: i64,
        cover_available: i64,
        steps: Rc<RefCell<Vec<&'static str>>>,
    }

    impl LoanPayDoApplyBroker for TestBroker {
        type AccountId = &'static str;
        type VaultId = &'static str;
        type Amount = i64;
        type Asset = &'static str;

        fn owner(&self) -> &Self::AccountId {
            &self.owner
        }

        fn pseudo_account(&self) -> &Self::AccountId {
            &self.pseudo_account
        }

        fn vault_id(&self) -> &Self::VaultId {
            &self.vault_id
        }

        fn cover_available(&self) -> &Self::Amount {
            &self.cover_available
        }

        fn debt_total(&self) -> &Self::Amount {
            &self.debt_total
        }

        fn cover_rate_minimum(&self) -> u32 {
            0
        }

        fn add_cover_available(&mut self, amount: Self::Amount) {
            self.steps.borrow_mut().push("add_cover_available");
            self.cover_available += amount;
        }

        fn adjust_debt_total(&mut self, amount: Self::Amount) {
            self.debt_total -= amount;
        }

        fn associate_asset(&mut self, _asset: &Self::Asset) {
            self.steps.borrow_mut().push("associate_broker_asset");
        }
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    struct TestVault {
        pseudo_account: &'static str,
        asset: &'static str,
        assets_available: i64,
        assets_total: i64,
        steps: Rc<RefCell<Vec<&'static str>>>,
    }

    impl LoanPayDoApplyVault for TestVault {
        type AccountId = &'static str;
        type Asset = &'static str;
        type Amount = i64;

        fn pseudo_account(&self) -> &Self::AccountId {
            &self.pseudo_account
        }

        fn asset(&self) -> &Self::Asset {
            &self.asset
        }

        fn assets_available(&self) -> &Self::Amount {
            &self.assets_available
        }

        fn assets_total(&self) -> &Self::Amount {
            &self.assets_total
        }

        fn add_assets_available(&mut self, amount: Self::Amount) {
            self.steps.borrow_mut().push("add_assets_available");
            self.assets_available += amount;
        }

        fn add_assets_total(&mut self, amount: Self::Amount) {
            self.steps.borrow_mut().push("add_assets_total");
            self.assets_total += amount;
        }

        fn assets_available_exceeds_total(&self) -> bool {
            self.assets_available > self.assets_total
        }

        fn associate_asset(&mut self, _asset: &Self::Asset) {
            self.steps.borrow_mut().push("associate_vault_asset");
        }
    }

    struct TestSink {
        steps: Rc<RefCell<Vec<&'static str>>>,
    }

    impl LoanPayTailMutationSink for TestSink {
        type Vault = TestVault;

        fn update_vault(&mut self, _vault: &Self::Vault) {
            self.steps.borrow_mut().push("update_vault");
        }
    }

    #[test]
    fn loan_pay_tail_mutation_maps_vault_overflow_to_tec_internal() {
        let steps = Rc::new(RefCell::new(Vec::new()));
        let mut loan = TestLoan {
            broker_id: "broker",
            steps: Rc::clone(&steps),
        };
        let mut broker = TestBroker {
            owner: "owner",
            pseudo_account: "broker-pseudo",
            vault_id: "vault",
            debt_total: 50,
            cover_available: 20,
            steps: Rc::clone(&steps),
        };
        let mut vault = TestVault {
            pseudo_account: "vault-pseudo",
            asset: "USD",
            assets_available: 10,
            assets_total: 10,
            steps: Rc::clone(&steps),
        };
        let mut sink = TestSink {
            steps: Rc::clone(&steps),
        };

        let result = run_loan_pay_tail_mutation(
            &mut sink,
            &mut loan,
            &mut broker,
            &mut vault,
            LoanPayTailMutationFacts {
                asset: "USD",
                payment_value_change: 0,
                total_paid_to_vault_rounded: 1,
                total_paid_to_broker: 0,
                send_broker_fee_to_owner: true,
            },
        );

        assert_eq!(result, Ter::TEC_INTERNAL);
        assert_eq!(
            steps.borrow().as_slice(),
            &["update_vault", "add_assets_available", "add_assets_total"]
        );
    }

    #[test]
    fn loan_pay_tail_mutation_runs_current_on_fallback_fee_path() {
        let steps = Rc::new(RefCell::new(Vec::new()));
        let mut loan = TestLoan {
            broker_id: "broker",
            steps: Rc::clone(&steps),
        };
        let mut broker = TestBroker {
            owner: "owner",
            pseudo_account: "broker-pseudo",
            vault_id: "vault",
            debt_total: 50,
            cover_available: 20,
            steps: Rc::clone(&steps),
        };
        let mut vault = TestVault {
            pseudo_account: "vault-pseudo",
            asset: "USD",
            assets_available: 80,
            assets_total: 100,
            steps: Rc::clone(&steps),
        };
        let mut sink = TestSink {
            steps: Rc::clone(&steps),
        };

        let result = run_loan_pay_tail_mutation(
            &mut sink,
            &mut loan,
            &mut broker,
            &mut vault,
            LoanPayTailMutationFacts {
                asset: "USD",
                payment_value_change: 2,
                total_paid_to_vault_rounded: 7,
                total_paid_to_broker: 3,
                send_broker_fee_to_owner: false,
            },
        );

        assert_eq!(result, Ter::TES_SUCCESS);
        assert_eq!(
            steps.borrow().as_slice(),
            &[
                "update_vault",
                "add_assets_available",
                "add_assets_total",
                "add_cover_available",
                "associate_loan_asset",
                "associate_broker_asset",
                "associate_vault_asset",
            ]
        );
        assert_eq!(broker.cover_available, 23);
        assert_eq!(vault.assets_available, 87);
        assert_eq!(vault.assets_total, 102);
    }

    #[test]
    fn loan_pay_tail_mutation_skips_cover_increment_on_owner_fee_path() {
        let steps = Rc::new(RefCell::new(Vec::new()));
        let mut loan = TestLoan {
            broker_id: "broker",
            steps: Rc::clone(&steps),
        };
        let mut broker = TestBroker {
            owner: "owner",
            pseudo_account: "broker-pseudo",
            vault_id: "vault",
            debt_total: 50,
            cover_available: 20,
            steps: Rc::clone(&steps),
        };
        let mut vault = TestVault {
            pseudo_account: "vault-pseudo",
            asset: "USD",
            assets_available: 30,
            assets_total: 40,
            steps: Rc::clone(&steps),
        };
        let mut sink = TestSink {
            steps: Rc::clone(&steps),
        };

        let result = run_loan_pay_tail_mutation(
            &mut sink,
            &mut loan,
            &mut broker,
            &mut vault,
            LoanPayTailMutationFacts {
                asset: "USD",
                payment_value_change: 2,
                total_paid_to_vault_rounded: 0,
                total_paid_to_broker: 0,
                send_broker_fee_to_owner: true,
            },
        );

        assert_eq!(result, Ter::TES_SUCCESS);
        assert_eq!(
            steps.borrow().as_slice(),
            &[
                "update_vault",
                "add_assets_available",
                "add_assets_total",
                "associate_loan_asset",
                "associate_broker_asset",
                "associate_vault_asset",
            ]
        );
        assert_eq!(broker.cover_available, 20);
    }
}
