//! Integration tests that pin the `LoanPay::calculateBaseFee(...)` wrapper to
//! the current C++ behavior.

use tx::{
    LOAN_PAYMENTS_PER_FEE_INCREMENT, LoanPayBaseFeeBroker, LoanPayBaseFeeLoan, LoanPayBaseFeeTx,
    LoanPayBaseFeeVault, compute_loan_pay_fee_increments, run_loan_pay_calculate_base_fee,
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
fn loan_pay_base_fee_keeps_normal_cost_for_full_and_late_payments() {
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
fn loan_pay_base_fee_keeps_normal_cost_when_preclaim_guards_would_fail() {
    let loan = TestLoan {
        payment_remaining: LOAN_PAYMENTS_PER_FEE_INCREMENT + 1,
        next_payment_due_date: 42,
        broker_id: "broker",
        scale: 3,
        periodic_payment: 10,
        loan_service_fee: 2,
    };

    let missing_loan = run_loan_pay_calculate_base_fee(
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
        |_| -> Option<TestBroker> { panic!("missing loan should skip broker lookup") },
        |_| -> Option<TestVault> { panic!("missing loan should skip vault lookup") },
        |_, _, _| panic!("missing loan should skip rounding"),
        |_, _, _| panic!("missing loan should skip payment estimate"),
    );
    let expired = run_loan_pay_calculate_base_fee(
        &TestTx {
            full_payment: false,
            late_payment: false,
            overpayment: false,
            amount: 150,
            asset: "USD",
        },
        10_u64,
        || Some(loan),
        |_| true,
        |_| -> Option<TestBroker> { panic!("expired loan should skip broker lookup") },
        |_| -> Option<TestVault> { panic!("expired loan should skip vault lookup") },
        |_, _, _| panic!("expired loan should skip rounding"),
        |_, _, _| panic!("expired loan should skip payment estimate"),
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

    assert_eq!(missing_loan, 10);
    assert_eq!(expired, 10);
    assert_eq!(wrong_asset, 10);
}

#[test]
fn loan_pay_base_fee_uses_rounded_up_fee_increments() {
    assert_eq!(compute_loan_pay_fee_increments(11), 3);

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
        |amount, regular_payment, overpayment| {
            assert_eq!(*amount, 150);
            assert_eq!(*regular_payment, 12);
            assert!(!overpayment);
            11
        },
    );

    assert_eq!(fee, 30);
}

#[test]
fn loan_pay_base_fee_forwards_overpayment_mode_into_payment_estimate() {
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
            assert!(overpayment);
            5
        },
    );

    assert_eq!(fee, 10);
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
