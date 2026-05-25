//! Current Rust helper mirroring the reference implementation.
//!
//! This module preserves the deterministic outer behavior around:
//!
//! - short-circuiting to the normal cost for `tfLoanFullPayment` and
//!   `tfLoanLatePayment`,
//! - returning the normal cost unchanged whenever a required loan, broker, or
//!   vault object is missing,
//! - returning the normal cost unchanged when the remaining-payment threshold,
//!   expiration gate, or vault-asset match check says later preclaim will fail,
//! - computing the regular payment from the rounded periodic payment plus the
//!   loan service fee only after those guards succeed, and
//! - charging one normal-cost increment per
//!   `loanPaymentsPerFeeIncrement` estimated payments, rounded up, with a
//!   minimum of one increment.

use std::ops::{Add, Mul};

pub const LOAN_PAYMENTS_PER_FEE_INCREMENT: u32 = 5;

pub trait LoanPayBaseFeeTx {
    type Asset;
    type Amount;

    fn is_full_payment(&self) -> bool;
    fn is_late_payment(&self) -> bool;
    fn is_overpayment(&self) -> bool;
    fn amount(&self) -> &Self::Amount;
    fn amount_asset(&self) -> &Self::Asset;
}

pub trait LoanPayBaseFeeLoan {
    type BrokerId;
    type DueDate;
    type Amount;

    fn payment_remaining(&self) -> u32;
    fn next_payment_due_date(&self) -> &Self::DueDate;
    fn broker_id(&self) -> &Self::BrokerId;
    fn scale(&self) -> i32;
    fn periodic_payment(&self) -> &Self::Amount;
    fn loan_service_fee(&self) -> &Self::Amount;
}

pub trait LoanPayBaseFeeBroker {
    type VaultId;

    fn vault_id(&self) -> &Self::VaultId;
}

pub trait LoanPayBaseFeeVault<Asset> {
    fn asset(&self) -> &Asset;
}

pub fn compute_loan_pay_fee_increments(payment_estimate: i64) -> u64 {
    let bounded_estimate = payment_estimate.max(1);
    let divisor = i64::from(LOAN_PAYMENTS_PER_FEE_INCREMENT);
    let rounded_up = (bounded_estimate + divisor - 1) / divisor;

    u64::try_from(rounded_up.max(1)).expect("loan pay fee increments should fit into u64")
}

#[allow(clippy::too_many_arguments)]
pub fn run_loan_pay_calculate_base_fee<Tx, Loan, Broker, Vault, Fee>(
    tx: &Tx,
    normal_cost: Fee,
    load_loan: impl FnOnce() -> Option<Loan>,
    has_expired: impl FnOnce(&Loan::DueDate) -> bool,
    load_broker: impl FnOnce(&Loan::BrokerId) -> Option<Broker>,
    load_vault: impl FnOnce(&Broker::VaultId) -> Option<Vault>,
    round_periodic_payment: impl FnOnce(&Tx::Asset, &Loan::Amount, i32) -> Loan::Amount,
    estimate_payment_count: impl FnOnce(&Tx::Amount, &Loan::Amount, bool) -> i64,
) -> Fee
where
    Tx: LoanPayBaseFeeTx,
    Loan: LoanPayBaseFeeLoan<Amount = Tx::Amount>,
    Broker: LoanPayBaseFeeBroker,
    Vault: LoanPayBaseFeeVault<Tx::Asset>,
    Tx::Asset: PartialEq,
    Tx::Amount: Clone + Add<Output = Tx::Amount>,
    Fee: Copy + Mul<u64, Output = Fee>,
{
    if tx.is_full_payment() || tx.is_late_payment() {
        return normal_cost;
    }

    let Some(loan) = load_loan() else {
        return normal_cost;
    };

    if loan.payment_remaining() <= LOAN_PAYMENTS_PER_FEE_INCREMENT {
        return normal_cost;
    }

    if has_expired(loan.next_payment_due_date()) {
        return normal_cost;
    }

    let Some(broker) = load_broker(loan.broker_id()) else {
        return normal_cost;
    };

    let Some(vault) = load_vault(broker.vault_id()) else {
        return normal_cost;
    };

    if vault.asset() != tx.amount_asset() {
        return normal_cost;
    }

    let regular_payment =
        round_periodic_payment(vault.asset(), loan.periodic_payment(), loan.scale())
            + loan.loan_service_fee().clone();
    let payment_estimate =
        estimate_payment_count(tx.amount(), &regular_payment, tx.is_overpayment());
    let fee_increments = compute_loan_pay_fee_increments(payment_estimate);

    normal_cost * fee_increments
}

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};

    use super::{
        LOAN_PAYMENTS_PER_FEE_INCREMENT, LoanPayBaseFeeBroker, LoanPayBaseFeeLoan,
        LoanPayBaseFeeTx, LoanPayBaseFeeVault, compute_loan_pay_fee_increments,
        run_loan_pay_calculate_base_fee,
    };

    #[derive(Clone, Copy)]
    struct TestTx {
        full_payment: bool,
        late_payment: bool,
        overpayment: bool,
        amount: i64,
        asset: &'static str,
    }

    impl LoanPayBaseFeeTx for TestTx {
        type Asset = &'static str;
        type Amount = i64;

        fn is_full_payment(&self) -> bool {
            self.full_payment
        }

        fn is_late_payment(&self) -> bool {
            self.late_payment
        }

        fn is_overpayment(&self) -> bool {
            self.overpayment
        }

        fn amount(&self) -> &Self::Amount {
            &self.amount
        }

        fn amount_asset(&self) -> &Self::Asset {
            &self.asset
        }
    }

    #[derive(Clone, Copy)]
    struct TestLoan {
        payment_remaining: u32,
        next_payment_due_date: u32,
        broker_id: &'static str,
        scale: i32,
        periodic_payment: i64,
        loan_service_fee: i64,
    }

    impl LoanPayBaseFeeLoan for TestLoan {
        type BrokerId = &'static str;
        type DueDate = u32;
        type Amount = i64;

        fn payment_remaining(&self) -> u32 {
            self.payment_remaining
        }

        fn next_payment_due_date(&self) -> &Self::DueDate {
            &self.next_payment_due_date
        }

        fn broker_id(&self) -> &Self::BrokerId {
            &self.broker_id
        }

        fn scale(&self) -> i32 {
            self.scale
        }

        fn periodic_payment(&self) -> &Self::Amount {
            &self.periodic_payment
        }

        fn loan_service_fee(&self) -> &Self::Amount {
            &self.loan_service_fee
        }
    }

    #[derive(Clone, Copy)]
    struct TestBroker {
        vault_id: &'static str,
    }

    impl LoanPayBaseFeeBroker for TestBroker {
        type VaultId = &'static str;

        fn vault_id(&self) -> &Self::VaultId {
            &self.vault_id
        }
    }

    #[derive(Clone, Copy)]
    struct TestVault {
        asset: &'static str,
    }

    impl LoanPayBaseFeeVault<&'static str> for TestVault {
        fn asset(&self) -> &&'static str {
            &self.asset
        }
    }

    #[test]
    fn loan_pay_fee_increments_round_up_with_minimum_one() {
        assert_eq!(compute_loan_pay_fee_increments(0), 1);
        assert_eq!(compute_loan_pay_fee_increments(1), 1);
        assert_eq!(compute_loan_pay_fee_increments(5), 1);
        assert_eq!(compute_loan_pay_fee_increments(6), 2);
        assert_eq!(compute_loan_pay_fee_increments(11), 3);
    }

    #[test]
    fn loan_pay_base_fee_short_circuits_full_and_late_flags() {
        for tx in [
            TestTx {
                full_payment: true,
                late_payment: false,
                overpayment: false,
                amount: 100,
                asset: "USD",
            },
            TestTx {
                full_payment: false,
                late_payment: true,
                overpayment: false,
                amount: 100,
                asset: "USD",
            },
        ] {
            let fee = run_loan_pay_calculate_base_fee(
                &tx,
                10_u64,
                || -> Option<TestLoan> { panic!("full/late payment should skip loan lookup") },
                |_| -> bool { panic!("full/late payment should skip expiration") },
                |_| -> Option<TestBroker> { panic!("full/late payment should skip broker lookup") },
                |_| -> Option<TestVault> { panic!("full/late payment should skip vault lookup") },
                |_, _, _| panic!("full/late payment should skip rounding"),
                |_, _, _| panic!("full/late payment should skip payment estimate"),
            );

            assert_eq!(fee, 10);
        }
    }

    #[test]
    fn loan_pay_base_fee_preserves_current_cpp_guard_order() {
        let trace = RefCell::new(Vec::new());

        let fee = run_loan_pay_calculate_base_fee(
            &TestTx {
                full_payment: false,
                late_payment: false,
                overpayment: true,
                amount: 150,
                asset: "USD",
            },
            10_u64,
            || {
                trace.borrow_mut().push("loan");
                Some(TestLoan {
                    payment_remaining: LOAN_PAYMENTS_PER_FEE_INCREMENT + 1,
                    next_payment_due_date: 42,
                    broker_id: "broker",
                    scale: 3,
                    periodic_payment: 10,
                    loan_service_fee: 2,
                })
            },
            |next_due| {
                trace.borrow_mut().push("expired");
                assert_eq!(*next_due, 42);
                false
            },
            |broker_id| {
                trace.borrow_mut().push("broker");
                assert_eq!(*broker_id, "broker");
                Some(TestBroker { vault_id: "vault" })
            },
            |vault_id| {
                trace.borrow_mut().push("vault");
                assert_eq!(*vault_id, "vault");
                Some(TestVault { asset: "USD" })
            },
            |asset, periodic_payment, scale| {
                trace.borrow_mut().push("round");
                assert_eq!(*asset, "USD");
                assert_eq!(*periodic_payment, 10);
                assert_eq!(scale, 3);
                10
            },
            |amount, regular_payment, overpayment| {
                trace.borrow_mut().push("estimate");
                assert_eq!(*amount, 150);
                assert_eq!(*regular_payment, 12);
                assert!(overpayment);
                11
            },
        );

        assert_eq!(fee, 30);
        assert_eq!(
            trace.into_inner(),
            vec!["loan", "expired", "broker", "vault", "round", "estimate"]
        );
    }

    #[test]
    fn loan_pay_base_fee_returns_normal_cost_for_missing_loan() {
        let broker_called = Cell::new(false);

        let fee = run_loan_pay_calculate_base_fee(
            &TestTx {
                full_payment: false,
                late_payment: false,
                overpayment: false,
                amount: 150,
                asset: "USD",
            },
            10_u64,
            || None::<TestLoan>,
            |_| -> bool { panic!("missing loan should skip expiration") },
            |_| {
                broker_called.set(true);
                None::<TestBroker>
            },
            |_| -> Option<TestVault> { panic!("missing loan should skip vault lookup") },
            |_, _, _| panic!("missing loan should skip rounding"),
            |_, _, _| panic!("missing loan should skip payment estimate"),
        );

        assert_eq!(fee, 10);
        assert!(!broker_called.get());
    }

    #[test]
    fn loan_pay_base_fee_returns_normal_cost_when_remaining_payments_are_small() {
        let fee = run_loan_pay_calculate_base_fee(
            &TestTx {
                full_payment: false,
                late_payment: false,
                overpayment: false,
                amount: 150,
                asset: "USD",
            },
            10_u64,
            || {
                Some(TestLoan {
                    payment_remaining: LOAN_PAYMENTS_PER_FEE_INCREMENT,
                    next_payment_due_date: 42,
                    broker_id: "broker",
                    scale: 3,
                    periodic_payment: 10,
                    loan_service_fee: 2,
                })
            },
            |_| -> bool { panic!("small remaining count should skip expiration") },
            |_| -> Option<TestBroker> { panic!("small remaining count should skip broker lookup") },
            |_| -> Option<TestVault> { panic!("small remaining count should skip vault lookup") },
            |_, _, _| panic!("small remaining count should skip rounding"),
            |_, _, _| panic!("small remaining count should skip payment estimate"),
        );

        assert_eq!(fee, 10);
    }

    #[test]
    fn loan_pay_base_fee_returns_normal_cost_for_missing_broker_vault_or_asset_mismatch() {
        let loan = TestLoan {
            payment_remaining: LOAN_PAYMENTS_PER_FEE_INCREMENT + 1,
            next_payment_due_date: 42,
            broker_id: "broker",
            scale: 3,
            periodic_payment: 10,
            loan_service_fee: 2,
        };

        let missing_broker = run_loan_pay_calculate_base_fee(
            &TestTx {
                full_payment: false,
                late_payment: false,
                overpayment: false,
                amount: 150,
                asset: "USD",
            },
            10_u64,
            || Some(loan),
            |_| false,
            |_| None::<TestBroker>,
            |_| -> Option<TestVault> { panic!("missing broker should skip vault lookup") },
            |_, _, _| panic!("missing broker should skip rounding"),
            |_, _, _| panic!("missing broker should skip payment estimate"),
        );
        let missing_vault = run_loan_pay_calculate_base_fee(
            &TestTx {
                full_payment: false,
                late_payment: false,
                overpayment: false,
                amount: 150,
                asset: "USD",
            },
            10_u64,
            || Some(loan),
            |_| false,
            |_| Some(TestBroker { vault_id: "vault" }),
            |_| None::<TestVault>,
            |_, _, _| panic!("missing vault should skip rounding"),
            |_, _, _| panic!("missing vault should skip payment estimate"),
        );
        let wrong_asset = run_loan_pay_calculate_base_fee(
            &TestTx {
                full_payment: false,
                late_payment: false,
                overpayment: false,
                amount: 150,
                asset: "USD",
            },
            10_u64,
            || Some(loan),
            |_| false,
            |_| Some(TestBroker { vault_id: "vault" }),
            |_| Some(TestVault { asset: "EUR" }),
            |_, _, _| panic!("asset mismatch should skip rounding"),
            |_, _, _| panic!("asset mismatch should skip payment estimate"),
        );

        assert_eq!(missing_broker, 10);
        assert_eq!(missing_vault, 10);
        assert_eq!(wrong_asset, 10);
    }

    #[test]
    fn loan_pay_base_fee_preserves_minimum_one_increment_when_payment_estimate_is_zero() {
        let fee = run_loan_pay_calculate_base_fee(
            &TestTx {
                full_payment: false,
                late_payment: false,
                overpayment: false,
                amount: 1,
                asset: "USD",
            },
            10_u64,
            || {
                Some(TestLoan {
                    payment_remaining: LOAN_PAYMENTS_PER_FEE_INCREMENT + 1,
                    next_payment_due_date: 42,
                    broker_id: "broker",
                    scale: 3,
                    periodic_payment: 10,
                    loan_service_fee: 2,
                })
            },
            |_| false,
            |_| Some(TestBroker { vault_id: "vault" }),
            |_| Some(TestVault { asset: "USD" }),
            |_, _, _| 10,
            |_, _, overpayment| {
                assert!(!overpayment);
                0
            },
        );

        assert_eq!(fee, 10);
    }
}
