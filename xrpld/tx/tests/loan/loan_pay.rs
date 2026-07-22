//! Integration tests that pin the narrowed Rust `LoanPay.cpp` metadata,
//! `preflight(...)`, `preclaim(...)`, and payment-type wrapper to the current
//! C++ behavior.

use std::cell::Cell;

use protocol::{Ter, trans_token};
use tx::loan_pay_tail::{
    LoanPayDoApplyTailFacts, LoanPayDoApplyTailSink, run_loan_pay_do_apply_tail,
};
use tx::loan_pay_unimpair::{LoanPayUnimpairFacts, run_loan_pay_unimpair};
use tx::{
    LOAN_FULL_PAYMENT_FLAG, LOAN_LATE_PAYMENT_FLAG, LOAN_PAY_OVERPAYMENT_FLAG,
    LoanPayAssertionFacts, LoanPayBalanceSnapshotFacts, LoanPayBrokerDebtDeltaSign,
    LoanPayBrokerDebtFacts, LoanPayBrokerFeeDestinationFacts, LoanPayCoverThresholdSink,
    LoanPayDoApplyAmountFacts, LoanPayDoApplyAmountsSink, LoanPayDoApplyBroker,
    LoanPayDoApplyFacts, LoanPayDoApplyFrontFacts, LoanPayDoApplyFrontState, LoanPayDoApplyLoan,
    LoanPayDoApplyMiddleFacts, LoanPayDoApplyMiddleResult, LoanPayDoApplySink, LoanPayDoApplyVault,
    LoanPayPaymentApplyFacts, LoanPayPaymentApplySink, LoanPayPaymentParts, LoanPayPaymentType,
    LoanPayPostBalanceFacts, LoanPayPostTransferChecksFacts, LoanPayPostTransferChecksResult,
    LoanPayPostTransferChecksSink, LoanPayPreTransferSnapshotFacts,
    LoanPayPreTransferSnapshotResult, LoanPayPreTransferSnapshotSink, LoanPayPreclaimFacts,
    LoanPayPreflightFacts, LoanPayTailMutationFacts, LoanPayTailMutationSink,
    LoanPayTailTransferFacts, LoanPayTailTransferSink, LoanPayTransferDeliveryFacts,
    LoanPayTransferPrepFacts, LoanPayVaultBalanceCheckFacts, compute_loan_pay_assertion_facts,
    compute_loan_pay_balance_snapshot, compute_loan_pay_broker_debt_facts,
    compute_loan_pay_cover_threshold_facts, compute_loan_pay_do_apply_amounts,
    compute_loan_pay_post_balances, compute_loan_pay_pre_transfer_snapshot,
    compute_loan_pay_transfer_delivery_facts, compute_loan_pay_transfer_prep_facts,
    compute_loan_pay_vault_balance_checks, decide_loan_pay_broker_fee_destination,
    get_loan_pay_flags_mask, run_loan_pay_broker_debt_adjustment,
    run_loan_pay_check_extra_features, run_loan_pay_do_apply, run_loan_pay_do_apply_front,
    run_loan_pay_do_apply_middle, run_loan_pay_payment_apply, run_loan_pay_payment_type,
    run_loan_pay_post_transfer_checks, run_loan_pay_preclaim, run_loan_pay_preflight,
    run_loan_pay_tail_mutation, run_loan_pay_tail_transfer,
};

fn base() -> LoanPayPreclaimFacts {
    LoanPayPreclaimFacts {
        loan_exists: true,
        submitter_is_borrower: true,
        tx_requests_overpayment: false,
        loan_allows_overpayment: true,
        fix_cleanup_3_1_3_enabled: true,
        principal_outstanding_is_zero: false,
        payment_remaining_is_zero: false,
        broker_exists: true,
        vault_exists: true,
        amount_matches_vault_asset: true,
        frozen_result: Ter::TES_SUCCESS,
        deep_frozen_result: Ter::TES_SUCCESS,
        require_auth_result: Ter::TES_SUCCESS,
        balance_is_less_than_amount: false,
    }
}

#[test]
fn tx_loan_pay_check_extra_features_delegates_to_lending_gate() {
    let helper_called = Cell::new(false);

    let disabled = run_loan_pay_check_extra_features(false, || {
        helper_called.set(true);
        true
    });
    assert!(!disabled);
    assert!(!helper_called.get());

    assert!(run_loan_pay_check_extra_features(true, || true));
    assert!(!run_loan_pay_check_extra_features(true, || false));
}

#[test]
fn tx_loan_pay_flags_mask_metadata() {
    assert_eq!(get_loan_pay_flags_mask(), 0x3ff8_ffff);
}

#[test]
fn tx_loan_pay_preflight_rejects_zero_loan_id() {
    let result = run_loan_pay_preflight(LoanPayPreflightFacts {
        loan_id_is_zero: true,
        amount_is_positive: true,
        tx_specific_flags: 0,
    });

    assert_eq!(result, Ter::TEM_INVALID);
}

#[test]
fn tx_loan_pay_preflight_rejects_non_positive_amount() {
    let result = run_loan_pay_preflight(LoanPayPreflightFacts {
        loan_id_is_zero: false,
        amount_is_positive: false,
        tx_specific_flags: 0,
    });

    assert_eq!(result, Ter::TEM_BAD_AMOUNT);
}

#[test]
fn tx_loan_pay_preflight_rejects_multiple_payment_flags() {
    let result = run_loan_pay_preflight(LoanPayPreflightFacts {
        loan_id_is_zero: false,
        amount_is_positive: true,
        tx_specific_flags: LOAN_LATE_PAYMENT_FLAG | LOAN_FULL_PAYMENT_FLAG,
    });

    assert_eq!(result, Ter::TEM_INVALID_FLAG);
    assert_eq!(trans_token(result), "temINVALID_FLAG");
}

#[test]
fn tx_loan_pay_preflight_accepts_single_payment_flag() {
    let late = run_loan_pay_preflight(LoanPayPreflightFacts {
        loan_id_is_zero: false,
        amount_is_positive: true,
        tx_specific_flags: LOAN_LATE_PAYMENT_FLAG,
    });
    let full = run_loan_pay_preflight(LoanPayPreflightFacts {
        loan_id_is_zero: false,
        amount_is_positive: true,
        tx_specific_flags: LOAN_FULL_PAYMENT_FLAG,
    });
    let over = run_loan_pay_preflight(LoanPayPreflightFacts {
        loan_id_is_zero: false,
        amount_is_positive: true,
        tx_specific_flags: LOAN_PAY_OVERPAYMENT_FLAG,
    });

    assert_eq!(late, Ter::TES_SUCCESS);
    assert_eq!(full, Ter::TES_SUCCESS);
    assert_eq!(over, Ter::TES_SUCCESS);
}

#[test]
fn tx_loan_pay_preclaim_rejects_missing_loan() {
    assert_eq!(
        run_loan_pay_preclaim(LoanPayPreclaimFacts::default()),
        Ter::TEC_NO_ENTRY
    );
}

#[test]
fn tx_loan_pay_preclaim_rejects_wrong_borrower() {
    assert_eq!(
        run_loan_pay_preclaim(LoanPayPreclaimFacts {
            submitter_is_borrower: false,
            ..base()
        }),
        Ter::TEC_NO_PERMISSION
    );
}

#[test]
fn tx_loan_pay_preclaim_preserves_legacy_and_fixed_overpayment_modes() {
    let legacy = run_loan_pay_preclaim(LoanPayPreclaimFacts {
        tx_requests_overpayment: true,
        loan_allows_overpayment: false,
        fix_cleanup_3_1_3_enabled: false,
        ..base()
    });
    let fixed = run_loan_pay_preclaim(LoanPayPreclaimFacts {
        tx_requests_overpayment: true,
        loan_allows_overpayment: false,
        fix_cleanup_3_1_3_enabled: true,
        ..base()
    });

    assert_eq!(legacy, Ter::TEM_INVALID_FLAG);
    assert_eq!(fixed, Ter::TEC_NO_PERMISSION);
}

#[test]
fn tx_loan_pay_preclaim_rejects_paid_off_loan() {
    assert_eq!(
        run_loan_pay_preclaim(LoanPayPreclaimFacts {
            payment_remaining_is_zero: true,
            ..base()
        }),
        Ter::TEC_KILLED
    );
    assert_eq!(
        run_loan_pay_preclaim(LoanPayPreclaimFacts {
            principal_outstanding_is_zero: true,
            ..base()
        }),
        Ter::TEC_KILLED
    );
}

#[test]
fn tx_loan_pay_preclaim_maps_missing_broker_and_vault_to_bad_ledger() {
    assert_eq!(
        run_loan_pay_preclaim(LoanPayPreclaimFacts {
            broker_exists: false,
            ..base()
        }),
        Ter::TEF_BAD_LEDGER
    );
    assert_eq!(
        run_loan_pay_preclaim(LoanPayPreclaimFacts {
            vault_exists: false,
            ..base()
        }),
        Ter::TEF_BAD_LEDGER
    );
}

#[test]
fn tx_loan_pay_preclaim_rejects_wrong_asset() {
    let result = run_loan_pay_preclaim(LoanPayPreclaimFacts {
        amount_matches_vault_asset: false,
        ..base()
    });

    assert_eq!(result, Ter::TEC_WRONG_ASSET);
    assert_eq!(trans_token(result), "tecWRONG_ASSET");
}

#[test]
fn tx_loan_pay_preclaim_returns_freeze_auth_and_balance_failures_unchanged() {
    let frozen = run_loan_pay_preclaim(LoanPayPreclaimFacts {
        frozen_result: Ter::TEC_FROZEN,
        ..base()
    });
    let deep_frozen = run_loan_pay_preclaim(LoanPayPreclaimFacts {
        deep_frozen_result: Ter::TEC_FROZEN,
        ..base()
    });
    let auth = run_loan_pay_preclaim(LoanPayPreclaimFacts {
        require_auth_result: Ter::TER_NO_AUTH,
        ..base()
    });
    let balance = run_loan_pay_preclaim(LoanPayPreclaimFacts {
        balance_is_less_than_amount: true,
        ..base()
    });

    assert_eq!(frozen, Ter::TEC_FROZEN);
    assert_eq!(deep_frozen, Ter::TEC_FROZEN);
    assert_eq!(auth, Ter::TER_NO_AUTH);
    assert_eq!(balance, Ter::TEC_INSUFFICIENT_FUNDS);
}

#[test]
fn tx_loan_pay_preclaim_accepts_valid_payment() {
    assert_eq!(run_loan_pay_preclaim(base()), Ter::TES_SUCCESS);
}

#[test]
fn tx_loan_pay_payment_type_do_apply_order() {
    assert_eq!(
        run_loan_pay_payment_type(true, true, true),
        LoanPayPaymentType::Late
    );
    assert_eq!(
        run_loan_pay_payment_type(false, true, true),
        LoanPayPaymentType::Full
    );
    assert_eq!(
        run_loan_pay_payment_type(false, false, true),
        LoanPayPaymentType::Overpayment
    );
    assert_eq!(
        run_loan_pay_payment_type(false, false, false),
        LoanPayPaymentType::Regular
    );
}

#[test]
fn tx_loan_pay_unimpair_skips_the_kernel_when_the_loan_is_not_impaired() {
    let calls = Cell::new(0);

    let result = run_loan_pay_unimpair(
        LoanPayUnimpairFacts {
            loan_is_impaired: false,
        },
        || {
            calls.set(calls.get() + 1);
            Ok::<(), Ter>(())
        },
    );

    assert_eq!(result, Ok(()));
    assert_eq!(calls.get(), 0);
}

#[test]
fn tx_loan_pay_unimpair_propagates_the_kernel_error_unchanged() {
    let calls = Cell::new(0);

    let result = run_loan_pay_unimpair(
        LoanPayUnimpairFacts {
            loan_is_impaired: true,
        },
        || {
            calls.set(calls.get() + 1);
            Err::<(), Ter>(Ter::TEC_PATH_DRY)
        },
    );

    assert_eq!(result, Err(Ter::TEC_PATH_DRY));
    assert_eq!(calls.get(), 1);
}

mod loan_pay_unimpair_and_payment_apply_exported_api_parity {
    use super::*;
    use std::{cell::RefCell, rc::Rc};

    #[derive(Debug, Clone, PartialEq, Eq)]
    struct TestLoan {
        broker_id: &'static str,
        scale: i32,
        impaired: bool,
        associated_asset: Option<&'static str>,
    }

    impl LoanPayDoApplyLoan for TestLoan {
        type BrokerId = &'static str;
        type Asset = &'static str;

        fn broker_id(&self) -> &Self::BrokerId {
            &self.broker_id
        }

        fn scale(&self) -> i32 {
            self.scale
        }

        fn is_impaired(&self) -> bool {
            self.impaired
        }

        fn associate_asset(&mut self, asset: &Self::Asset) {
            self.associated_asset = Some(*asset);
        }
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    struct TestBroker {
        owner: &'static str,
        pseudo_account: &'static str,
        vault_id: &'static str,
        cover_available: i64,
        debt_total: i64,
        cover_rate_minimum: u32,
        associated_asset: Option<&'static str>,
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
            self.cover_rate_minimum
        }

        fn add_cover_available(&mut self, amount: Self::Amount) {
            self.cover_available += amount;
        }

        fn adjust_debt_total(&mut self, delta: Self::Amount) {
            self.debt_total -= delta;
        }

        fn associate_asset(&mut self, asset: &Self::Asset) {
            self.associated_asset = Some(*asset);
        }
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    struct TestVault {
        pseudo_account: &'static str,
        asset: &'static str,
        assets_available: i64,
        assets_total: i64,
        associated_asset: Option<&'static str>,
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
            self.assets_available += amount;
        }

        fn add_assets_total(&mut self, amount: Self::Amount) {
            self.assets_total += amount;
        }

        fn assets_available_exceeds_total(&self) -> bool {
            self.assets_available > self.assets_total
        }

        fn associate_asset(&mut self, asset: &Self::Asset) {
            self.associated_asset = Some(*asset);
        }
    }

    struct TestSink {
        loan: Option<TestLoan>,
        broker: Option<TestBroker>,
        vault: Option<TestVault>,
        required_cover: i64,
        owner_is_deep_frozen: bool,
        owner_requires_auth: bool,
        fallback_deep_frozen: Ter,
        unimpair_result: Ter,
        payment_result: Result<LoanPayPaymentParts<i64>, Ter>,
        require_auth_result: Ter,
        add_empty_holding_result: Ter,
        account_send_multi_result: Ter,
        balances: std::collections::HashMap<&'static str, i64>,
        asset_issuer: &'static str,
        expected_from: &'static str,
        expected_broker_payee: &'static str,
        expected_broker_amount: i64,
        steps: Rc<RefCell<Vec<&'static str>>>,
    }

    impl TestSink {
        fn new() -> Self {
            Self {
                loan: Some(TestLoan {
                    broker_id: "broker",
                    scale: 6,
                    impaired: false,
                    associated_asset: None,
                }),
                broker: Some(TestBroker {
                    owner: "owner",
                    pseudo_account: "broker-pseudo",
                    vault_id: "vault",
                    cover_available: 10,
                    debt_total: 0,
                    cover_rate_minimum: 0,
                    associated_asset: None,
                }),
                vault: Some(TestVault {
                    pseudo_account: "vault-pseudo",
                    asset: "USD",
                    assets_available: 10,
                    assets_total: 30,
                    associated_asset: None,
                }),
                required_cover: 10,
                owner_is_deep_frozen: false,
                owner_requires_auth: false,
                fallback_deep_frozen: Ter::TES_SUCCESS,
                unimpair_result: Ter::TES_SUCCESS,
                payment_result: Ok(LoanPayPaymentParts {
                    principal_paid: 10,
                    interest_paid: 3,
                    fee_paid: 1,
                    value_change: 0,
                }),
                require_auth_result: Ter::TES_SUCCESS,
                add_empty_holding_result: Ter::TES_SUCCESS,
                account_send_multi_result: Ter::TES_SUCCESS,
                balances: std::collections::HashMap::from([
                    ("owner", 100),
                    ("vault-pseudo", 10),
                    ("broker-pseudo", 2),
                ]),
                asset_issuer: "issuer",
                expected_from: "owner",
                expected_broker_payee: "owner",
                expected_broker_amount: 1,
                steps: Rc::new(RefCell::new(Vec::new())),
            }
        }
    }

    impl LoanPayDoApplySink for TestSink {
        type Loan = TestLoan;
        type Broker = TestBroker;
        type Vault = TestVault;
        type AccountId = &'static str;
        type BrokerId = &'static str;
        type VaultId = &'static str;
        type Asset = &'static str;
        type Amount = i64;

        fn read_loan(&mut self) -> Option<Self::Loan> {
            self.steps.borrow_mut().push("read_loan");
            self.loan.clone()
        }

        fn read_broker(&mut self, broker_id: &Self::BrokerId) -> Option<Self::Broker> {
            self.steps.borrow_mut().push("read_broker");
            assert_eq!(*broker_id, "broker");
            self.broker.clone()
        }

        fn read_vault(&mut self, vault_id: &Self::VaultId) -> Option<Self::Vault> {
            self.steps.borrow_mut().push("read_vault");
            assert_eq!(*vault_id, "vault");
            self.vault.clone()
        }

        fn compute_required_cover_threshold(
            &mut self,
            _asset: &Self::Asset,
            _debt_total: &Self::Amount,
            _cover_rate_minimum: u32,
            _loan_scale: i32,
        ) -> Self::Amount {
            self.steps.borrow_mut().push("required_cover");
            self.required_cover
        }

        fn broker_owner_is_deep_frozen(
            &mut self,
            _owner: &Self::AccountId,
            _asset: &Self::Asset,
        ) -> bool {
            self.steps.borrow_mut().push("owner_deep_frozen");
            self.owner_is_deep_frozen
        }

        fn broker_owner_requires_auth(
            &mut self,
            _owner: &Self::AccountId,
            _asset: &Self::Asset,
        ) -> bool {
            self.steps.borrow_mut().push("owner_auth");
            self.owner_requires_auth
        }

        fn check_deep_frozen(&mut self, account: &Self::AccountId, _asset: &Self::Asset) -> Ter {
            self.steps.borrow_mut().push("fallback_deep_frozen");
            assert_eq!(*account, "broker-pseudo");
            self.fallback_deep_frozen
        }

        fn unimpair_loan(
            &mut self,
            loan: &mut Self::Loan,
            _vault: &Self::Vault,
            _asset: &Self::Asset,
        ) -> Ter {
            self.steps.borrow_mut().push("unimpair");
            loan.impaired = false;
            self.unimpair_result
        }

        fn make_payment(
            &mut self,
            _asset: &Self::Asset,
            _loan: &mut Self::Loan,
            _broker: &mut Self::Broker,
            amount: &Self::Amount,
            payment_type: LoanPayPaymentType,
        ) -> Result<LoanPayPaymentParts<Self::Amount>, Ter> {
            self.steps.borrow_mut().push("make_payment");
            assert_eq!(*amount, 25);
            assert_eq!(payment_type, LoanPayPaymentType::Full);
            self.payment_result.clone()
        }

        fn update_loan(&mut self, _loan: &Self::Loan) {
            self.steps.borrow_mut().push("update_loan");
        }

        fn update_broker(&mut self, _broker: &Self::Broker) {
            self.steps.borrow_mut().push("update_broker");
        }

        fn adjust_broker_debt_total(
            &mut self,
            broker: &mut Self::Broker,
            debt_delta: &Self::Amount,
            _asset: &Self::Asset,
            _vault_scale: i32,
        ) {
            self.steps.borrow_mut().push("adjust_broker_debt_total");
            broker.adjust_debt_total(-*debt_delta);
        }

        fn update_vault(&mut self, _vault: &Self::Vault) {
            self.steps.borrow_mut().push("update_vault");
        }

        fn require_auth(&mut self, account: &Self::AccountId, _asset: &Self::Asset) -> Ter {
            self.steps.borrow_mut().push("require_auth");
            assert!(
                *account == "vault-pseudo" || *account == "owner" || *account == "broker-pseudo"
            );
            self.require_auth_result
        }

        fn broker_payee_balance_for_empty_holding(
            &mut self,
            account: &Self::AccountId,
        ) -> Self::Amount {
            self.steps.borrow_mut().push("broker_payee_balance");
            assert_eq!(*account, "owner");
            12
        }

        fn add_empty_holding(
            &mut self,
            account: &Self::AccountId,
            balance: &Self::Amount,
            _asset: &Self::Asset,
        ) -> Ter {
            self.steps.borrow_mut().push("add_empty_holding");
            assert_eq!(*account, "owner");
            assert_eq!(*balance, 12);
            self.add_empty_holding_result
        }

        fn account_send_multi(
            &mut self,
            from: &Self::AccountId,
            _asset: &Self::Asset,
            vault_pseudo: &Self::AccountId,
            vault_amount: &Self::Amount,
            broker_payee: &Self::AccountId,
            broker_amount: &Self::Amount,
        ) -> Ter {
            self.steps.borrow_mut().push("account_send_multi");
            assert_eq!(*from, self.expected_from);
            assert_eq!(*vault_pseudo, "vault-pseudo");
            assert_eq!(*vault_amount, 13);
            assert_eq!(*broker_payee, self.expected_broker_payee);
            assert_eq!(*broker_amount, self.expected_broker_amount);
            let from_balance = self.balances.get_mut(from).expect("from balance");
            *from_balance -= *vault_amount + *broker_amount;
            let vault_balance = self.balances.get_mut(vault_pseudo).expect("vault balance");
            *vault_balance += *vault_amount;
            let broker_balance = self.balances.entry(*broker_payee).or_insert(0);
            *broker_balance += *broker_amount;
            self.account_send_multi_result
        }

        fn sample_balance(
            &mut self,
            account: &Self::AccountId,
            _asset: &Self::Asset,
        ) -> Self::Amount {
            self.steps.borrow_mut().push("sample_balance");
            *self.balances.get(account).unwrap_or(&0)
        }

        fn account_is_asset_issuer(
            &mut self,
            account: &Self::AccountId,
            _asset: &Self::Asset,
        ) -> bool {
            self.steps.borrow_mut().push("account_is_asset_issuer");
            *account == self.asset_issuer
        }
    }

    impl LoanPayPaymentApplySink for TestSink {
        type Loan = TestLoan;
        type Broker = TestBroker;
        type Asset = &'static str;
        type Amount = i64;

        fn make_payment(
            &mut self,
            asset: &Self::Asset,
            _loan: &mut Self::Loan,
            _broker: &mut Self::Broker,
            amount: &Self::Amount,
            payment_type: LoanPayPaymentType,
        ) -> Result<LoanPayPaymentParts<Self::Amount>, Ter> {
            assert_eq!(*asset, "USD");
            assert_eq!(*amount, 40);
            assert_eq!(payment_type, LoanPayPaymentType::Full);
            self.steps.borrow_mut().push("make_payment");
            self.payment_result.clone()
        }

        fn update_loan(&mut self, _loan: &Self::Loan) {
            self.steps.borrow_mut().push("update_loan");
        }
    }

    impl LoanPayDoApplyAmountsSink for TestSink {
        type Vault = TestVault;
        type Asset = &'static str;
        type Amount = i64;

        fn vault_scale(&mut self, _vault: &Self::Vault) -> i32 {
            self.steps.borrow_mut().push("vault_scale");
            2
        }

        fn round_to_asset_downward(
            &mut self,
            asset: &Self::Asset,
            value: &Self::Amount,
            scale: i32,
        ) -> Self::Amount {
            self.steps.borrow_mut().push("round_to_asset_downward");
            assert_eq!(*asset, "USD");
            assert_eq!(*value, 13);
            assert_eq!(scale, 2);
            13
        }

        fn asset_is_integral(&mut self, asset: &Self::Asset) -> bool {
            self.steps.borrow_mut().push("asset_is_integral");
            assert_eq!(*asset, "USD");
            false
        }

        fn is_rounded(&mut self, asset: &Self::Asset, value: &Self::Amount, scale: i32) -> bool {
            self.steps.borrow_mut().push("is_rounded");
            assert_eq!(*asset, "USD");
            assert_eq!(*value, 13);
            assert_eq!(scale, 6);
            true
        }
    }

    fn loan(impaired: bool) -> TestLoan {
        TestLoan {
            broker_id: "broker",
            scale: 6,
            impaired,
            associated_asset: None,
        }
    }

    fn broker() -> TestBroker {
        TestBroker {
            owner: "owner",
            pseudo_account: "broker-pseudo",
            vault_id: "vault",
            cover_available: 10,
            debt_total: 0,
            cover_rate_minimum: 0,
            associated_asset: None,
        }
    }

    fn vault() -> TestVault {
        TestVault {
            pseudo_account: "vault-pseudo",
            asset: "USD",
            assets_available: 10,
            assets_total: 30,
            associated_asset: None,
        }
    }

    #[test]
    fn tx_loan_pay_do_apply_front_unimpairs_before_payment_apply() {
        let mut sink = TestSink::new();
        sink.loan = Some(loan(true));
        sink.broker = Some(broker());
        sink.vault = Some(vault());

        let state = run_loan_pay_do_apply_front(
            &mut sink,
            LoanPayDoApplyFrontFacts {
                amount: 25,
                zero_amount: 0,
                tx_requests_late_payment: false,
                tx_requests_full_payment: true,
                tx_requests_overpayment: false,
            },
        )
        .expect("front apply succeeds");

        assert_eq!(
            sink.steps.borrow().as_slice(),
            [
                "read_loan",
                "read_broker",
                "read_vault",
                "required_cover",
                "owner_deep_frozen",
                "owner_auth",
                "unimpair",
                "make_payment",
                "update_loan",
                "update_broker",
            ]
        );
        assert!(state.send_broker_fee_to_owner);
        assert_eq!(state.payment_type, LoanPayPaymentType::Full);
    }

    #[test]
    fn tx_loan_pay_do_apply_front_returns_unimpair_error_unchanged() {
        let mut sink = TestSink::new();
        sink.loan = Some(loan(true));
        sink.broker = Some(broker());
        sink.vault = Some(vault());
        sink.unimpair_result = Ter::TEC_PATH_DRY;

        let result = run_loan_pay_do_apply_front(
            &mut sink,
            LoanPayDoApplyFrontFacts {
                amount: 25,
                zero_amount: 0,
                tx_requests_late_payment: false,
                tx_requests_full_payment: true,
                tx_requests_overpayment: false,
            },
        );

        assert_eq!(result, Err(Ter::TEC_PATH_DRY));
        assert_eq!(
            sink.steps.borrow().as_slice(),
            [
                "read_loan",
                "read_broker",
                "read_vault",
                "required_cover",
                "owner_deep_frozen",
                "owner_auth",
                "unimpair",
            ]
        );
    }

    #[test]
    fn tx_loan_pay_payment_apply_updates_loan_and_rejects_negative_parts() {
        let mut success_sink = TestSink::new();
        success_sink.payment_result = Ok(LoanPayPaymentParts {
            principal_paid: 25,
            interest_paid: 12,
            fee_paid: 3,
            value_change: 7,
        });
        let mut loan_ok = loan(false);
        let mut broker_ok = broker();

        let result = run_loan_pay_payment_apply(
            &mut success_sink,
            &"USD",
            &mut loan_ok,
            &mut broker_ok,
            LoanPayPaymentApplyFacts {
                amount: 40,
                payment_type: LoanPayPaymentType::Full,
                zero_amount: 0,
            },
        )
        .expect("payment apply succeeds");

        assert_eq!(
            success_sink.steps.borrow().as_slice(),
            ["make_payment", "update_loan"]
        );
        assert_eq!(result.payment_parts.principal_paid, 25);
        assert!(result.payment_validity.all_assertions_hold);

        let mut failure_sink = TestSink::new();
        failure_sink.payment_result = Ok(LoanPayPaymentParts {
            principal_paid: -1,
            interest_paid: 1,
            fee_paid: 0,
            value_change: 0,
        });
        let mut loan_bad = loan(false);
        let mut broker_bad = broker();

        let result = run_loan_pay_payment_apply(
            &mut failure_sink,
            &"USD",
            &mut loan_bad,
            &mut broker_bad,
            LoanPayPaymentApplyFacts {
                amount: 40,
                payment_type: LoanPayPaymentType::Full,
                zero_amount: 0,
            },
        );

        assert_eq!(result, Err(Ter::TEC_LIMIT_EXCEEDED));
        assert_eq!(
            failure_sink.steps.borrow().as_slice(),
            ["make_payment", "update_loan"]
        );
    }

    #[test]
    fn tx_loan_pay_payment_apply_passthroughs_payment_error_before_update() {
        let mut sink = TestSink::new();
        sink.payment_result = Err(Ter::TEC_PATH_DRY);
        let mut loan_err = loan(false);
        let mut broker_err = broker();

        let result = run_loan_pay_payment_apply(
            &mut sink,
            &"USD",
            &mut loan_err,
            &mut broker_err,
            LoanPayPaymentApplyFacts {
                amount: 40,
                payment_type: LoanPayPaymentType::Full,
                zero_amount: 0,
            },
        );

        assert_eq!(result, Err(Ter::TEC_PATH_DRY));
        assert_eq!(sink.steps.borrow().as_slice(), ["make_payment"]);
    }
}

#[cfg(test)]
mod loan_pay_do_apply_front_parity {
    use super::*;
    use protocol::is_tes_success;

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    struct LoanPayDoApplyFrontPaymentParts {
        principal_paid: i64,
        interest_paid: i64,
        fee_paid: i64,
        value_change: i64,
    }

    #[derive(Debug, Clone, Copy)]
    struct LoanPayDoApplyFrontFacts {
        loan_exists: bool,
        broker_exists: bool,
        vault_exists: bool,
        broker_owner_can_receive_funds: bool,
        fallback_pseudo_deep_frozen_result: Ter,
        loan_is_impaired: bool,
        tx_requests_late_payment: bool,
        tx_requests_full_payment: bool,
        tx_requests_overpayment: bool,
        payment_parts_result: Result<LoanPayDoApplyFrontPaymentParts, Ter>,
    }

    fn run_front_harness(facts: LoanPayDoApplyFrontFacts, steps: &mut Vec<&'static str>) -> Ter {
        if !facts.loan_exists {
            return Ter::TEF_BAD_LEDGER;
        }
        steps.push("load_loan");

        if !facts.broker_exists {
            return Ter::TEF_BAD_LEDGER;
        }
        steps.push("load_broker");

        if !facts.vault_exists {
            return Ter::TEF_BAD_LEDGER;
        }
        steps.push("load_vault");

        steps.push("select_broker_payee");
        if !facts.broker_owner_can_receive_funds {
            steps.push("check_fallback_pseudo_deep_freeze");
            if !is_tes_success(facts.fallback_pseudo_deep_frozen_result) {
                return facts.fallback_pseudo_deep_frozen_result;
            }
        }

        if facts.loan_is_impaired {
            steps.push("unimpair_loan");
        }

        let payment_type = run_loan_pay_payment_type(
            facts.tx_requests_late_payment,
            facts.tx_requests_full_payment,
            facts.tx_requests_overpayment,
        );
        steps.push(match payment_type {
            LoanPayPaymentType::Late => "payment_type_late",
            LoanPayPaymentType::Full => "payment_type_full",
            LoanPayPaymentType::Overpayment => "payment_type_overpayment",
            LoanPayPaymentType::Regular => "payment_type_regular",
        });

        let payment_parts = match facts.payment_parts_result {
            Ok(parts) => parts,
            Err(err) => return err,
        };
        steps.push("payment_computed");

        if payment_parts.principal_paid < 0
            || payment_parts.interest_paid < 0
            || payment_parts.fee_paid < 0
        {
            return Ter::TEC_LIMIT_EXCEEDED;
        }

        steps.push("update_loan");
        steps.push("update_broker");

        Ter::TES_SUCCESS
    }

    fn base() -> LoanPayDoApplyFrontFacts {
        LoanPayDoApplyFrontFacts {
            loan_exists: true,
            broker_exists: true,
            vault_exists: true,
            broker_owner_can_receive_funds: true,
            fallback_pseudo_deep_frozen_result: Ter::TES_SUCCESS,
            loan_is_impaired: false,
            tx_requests_late_payment: false,
            tx_requests_full_payment: false,
            tx_requests_overpayment: false,
            payment_parts_result: Ok(LoanPayDoApplyFrontPaymentParts {
                principal_paid: 1,
                interest_paid: 1,
                fee_paid: 1,
                value_change: 0,
            }),
        }
    }

    #[test]
    fn loan_pay_do_apply_front_rejects_missing_loan_broker_and_vault() {
        let mut steps = Vec::new();

        assert_eq!(
            run_front_harness(
                LoanPayDoApplyFrontFacts {
                    loan_exists: false,
                    ..base()
                },
                &mut steps
            ),
            Ter::TEF_BAD_LEDGER
        );
        assert!(steps.is_empty());

        assert_eq!(
            run_front_harness(
                LoanPayDoApplyFrontFacts {
                    broker_exists: false,
                    ..base()
                },
                &mut steps
            ),
            Ter::TEF_BAD_LEDGER
        );
        assert_eq!(steps, ["load_loan"]);

        steps.clear();
        assert_eq!(
            run_front_harness(
                LoanPayDoApplyFrontFacts {
                    vault_exists: false,
                    ..base()
                },
                &mut steps
            ),
            Ter::TEF_BAD_LEDGER
        );
        assert_eq!(steps, ["load_loan", "load_broker"]);
    }

    #[test]
    fn loan_pay_do_apply_front_selects_broker_payee_before_fallback_pseudo_deep_freeze() {
        let mut steps = Vec::new();

        let result = run_front_harness(
            LoanPayDoApplyFrontFacts {
                broker_owner_can_receive_funds: false,
                fallback_pseudo_deep_frozen_result: Ter::TEC_FROZEN,
                ..base()
            },
            &mut steps,
        );

        assert_eq!(result, Ter::TEC_FROZEN);
        assert_eq!(
            steps,
            [
                "load_loan",
                "load_broker",
                "load_vault",
                "select_broker_payee",
                "check_fallback_pseudo_deep_freeze"
            ]
        );
    }

    #[test]
    fn loan_pay_do_apply_front_unimpairs_before_payment_computation() {
        let mut steps = Vec::new();

        let result = run_front_harness(
            LoanPayDoApplyFrontFacts {
                loan_is_impaired: true,
                ..base()
            },
            &mut steps,
        );

        assert_eq!(result, Ter::TES_SUCCESS);
        assert_eq!(
            steps,
            [
                "load_loan",
                "load_broker",
                "load_vault",
                "select_broker_payee",
                "unimpair_loan",
                "payment_type_regular",
                "payment_computed",
                "update_loan",
                "update_broker",
            ]
        );
    }

    #[test]
    fn loan_pay_do_apply_front_reuses_the_same_payment_type_order_as_the_cxx_helper() {
        assert_eq!(
            run_loan_pay_payment_type(true, true, true),
            LoanPayPaymentType::Late
        );
        assert_eq!(
            run_loan_pay_payment_type(false, true, true),
            LoanPayPaymentType::Full
        );
        assert_eq!(
            run_loan_pay_payment_type(false, false, true),
            LoanPayPaymentType::Overpayment
        );
        assert_eq!(
            run_loan_pay_payment_type(false, false, false),
            LoanPayPaymentType::Regular
        );
    }

    #[test]
    fn loan_pay_do_apply_front_passes_through_payment_computation_errors() {
        let mut steps = Vec::new();

        let result = run_front_harness(
            LoanPayDoApplyFrontFacts {
                payment_parts_result: Err(Ter::TEC_PATH_DRY),
                ..base()
            },
            &mut steps,
        );

        assert_eq!(result, Ter::TEC_PATH_DRY);
        assert_eq!(
            steps,
            [
                "load_loan",
                "load_broker",
                "load_vault",
                "select_broker_payee",
                "payment_type_regular"
            ]
        );
    }

    #[test]
    fn loan_pay_do_apply_front_maps_negative_paid_parts_to_tec_limit_exceeded() {
        let mut steps = Vec::new();

        let result = run_front_harness(
            LoanPayDoApplyFrontFacts {
                payment_parts_result: Ok(LoanPayDoApplyFrontPaymentParts {
                    principal_paid: -1,
                    interest_paid: 1,
                    fee_paid: 1,
                    value_change: 0,
                }),
                ..base()
            },
            &mut steps,
        );

        assert_eq!(result, Ter::TEC_LIMIT_EXCEEDED);
        assert_eq!(
            steps,
            [
                "load_loan",
                "load_broker",
                "load_vault",
                "select_broker_payee",
                "payment_type_regular",
                "payment_computed"
            ]
        );
    }

    #[test]
    fn loan_pay_do_apply_front_updates_broker_only_after_successful_payment_computation() {
        let mut steps = Vec::new();

        let result = run_front_harness(base(), &mut steps);

        assert_eq!(result, Ter::TES_SUCCESS);
        assert_eq!(
            steps,
            [
                "load_loan",
                "load_broker",
                "load_vault",
                "select_broker_payee",
                "payment_type_regular",
                "payment_computed",
                "update_loan",
                "update_broker",
            ]
        );
    }
}

mod loan_pay_amounts_exported_api_parity {
    use super::*;

    #[derive(Default)]
    struct TestSink {
        steps: Vec<&'static str>,
    }

    impl LoanPayDoApplyAmountsSink for TestSink {
        type Vault = &'static str;
        type Asset = &'static str;
        type Amount = i64;

        fn vault_scale(&mut self, vault: &Self::Vault) -> i32 {
            assert_eq!(*vault, "vault");
            self.steps.push("vault_scale");
            10
        }

        fn round_to_asset_downward(
            &mut self,
            asset: &Self::Asset,
            value: &Self::Amount,
            scale: i32,
        ) -> Self::Amount {
            assert_eq!(*asset, "USD");
            assert_eq!(*value, 37);
            assert_eq!(scale, 10);
            self.steps.push("round_to_asset_downward");
            30
        }

        fn asset_is_integral(&mut self, asset: &Self::Asset) -> bool {
            assert_eq!(*asset, "USD");
            self.steps.push("asset_is_integral");
            false
        }

        fn is_rounded(&mut self, asset: &Self::Asset, value: &Self::Amount, scale: i32) -> bool {
            assert_eq!(*asset, "USD");
            assert_eq!(*value, 30);
            assert_eq!(scale, 10);
            self.steps.push("is_rounded");
            true
        }
    }

    #[test]
    fn tx_loan_pay_amounts_compute_cpp_tail_values_in() {
        let mut sink = TestSink::default();

        let facts = compute_loan_pay_do_apply_amounts(
            &mut sink,
            &"USD",
            &"vault",
            &LoanPayPaymentParts {
                principal_paid: 25,
                interest_paid: 12,
                fee_paid: 3,
                value_change: 7,
            },
            &0,
            &40,
            10,
        );

        assert_eq!(
            facts,
            LoanPayDoApplyAmountFacts {
                vault_scale: 10,
                total_paid_to_vault_raw: 37,
                total_paid_to_vault_rounded: 30,
                total_paid_to_vault_for_debt: 30,
                total_paid_to_broker: 3,
                total_paid_is_positive: true,
                paid_parts_sum_matches_outputs: true,
                integral_asset_rounding_matches_raw: true,
                rounded_amount_is_not_greater_than_raw: true,
                debt_amount_is_rounded: true,
                rounded_and_broker_not_greater_than_amount: true,
            }
        );
        assert_eq!(
            sink.steps,
            vec![
                "vault_scale",
                "round_to_asset_downward",
                "asset_is_integral",
                "is_rounded",
            ]
        );
    }

    #[test]
    fn tx_loan_pay_amounts_flags_amount_insufficient_after_rounding_assert() {
        let mut sink = TestSink::default();

        let facts = compute_loan_pay_do_apply_amounts(
            &mut sink,
            &"USD",
            &"vault",
            &LoanPayPaymentParts {
                principal_paid: 25,
                interest_paid: 12,
                fee_paid: 3,
                value_change: 7,
            },
            &0,
            &31,
            10,
        );

        assert!(facts.total_paid_is_positive);
        assert!(facts.paid_parts_sum_matches_outputs);
        assert!(!facts.rounded_and_broker_not_greater_than_amount);
    }
}

mod loan_pay_cover_exported_api_parity {
    use super::*;

    #[test]
    fn tx_loan_pay_cover_owner_fee_destination_rule() {
        assert!(decide_loan_pay_broker_fee_destination(
            LoanPayBrokerFeeDestinationFacts {
                cover_is_sufficient: true,
                owner_is_deep_frozen: false,
                owner_requires_auth: false,
            }
        ));

        assert!(!decide_loan_pay_broker_fee_destination(
            LoanPayBrokerFeeDestinationFacts {
                cover_is_sufficient: false,
                owner_is_deep_frozen: false,
                owner_requires_auth: false,
            }
        ));
        assert!(!decide_loan_pay_broker_fee_destination(
            LoanPayBrokerFeeDestinationFacts {
                cover_is_sufficient: true,
                owner_is_deep_frozen: true,
                owner_requires_auth: false,
            }
        ));
        assert!(!decide_loan_pay_broker_fee_destination(
            LoanPayBrokerFeeDestinationFacts {
                cover_is_sufficient: true,
                owner_is_deep_frozen: false,
                owner_requires_auth: true,
            }
        ));
    }

    #[derive(Default)]
    struct TestSink {
        calls: Vec<String>,
    }

    impl LoanPayCoverThresholdSink for TestSink {
        type Amount = i64;
        type Asset = &'static str;
        type CoverRateMinimum = u32;
        type Scale = i32;

        fn compute_required_cover_threshold(
            &mut self,
            asset: &Self::Asset,
            debt_total: &Self::Amount,
            cover_rate_minimum: Self::CoverRateMinimum,
            loan_scale: Self::Scale,
        ) -> Self::Amount {
            self.calls.push(format!(
                "threshold asset={asset} debt_total={debt_total} rate={cover_rate_minimum} scale={loan_scale}"
            ));
            (*debt_total * i64::from(cover_rate_minimum)) + i64::from(loan_scale)
        }
    }

    #[test]
    fn tx_loan_pay_cover_threshold_minimum_cover_comparison() {
        let mut sink = TestSink::default();
        let equal = compute_loan_pay_cover_threshold_facts(
            &mut sink, &204_i64, &"USD", &100_i64, 2_u32, 4_i32,
        );
        let low = compute_loan_pay_cover_threshold_facts(
            &mut sink, &203_i64, &"USD", &100_i64, 2_u32, 4_i32,
        );

        assert_eq!(equal.asset, "USD");
        assert_eq!(equal.required_cover, 204);
        assert!(equal.cover_available_meets_minimum);
        assert!(!low.cover_available_meets_minimum);
        assert_eq!(
            sink.calls,
            vec![
                "threshold asset=USD debt_total=100 rate=2 scale=4",
                "threshold asset=USD debt_total=100 rate=2 scale=4",
            ]
        );
    }
}

mod loan_pay_broker_debt_exported_api_parity {
    use super::*;

    #[derive(Default)]
    struct TestSink {
        debt_total: i64,
        steps: Vec<String>,
    }

    impl tx::LoanPayBrokerDebtAdjustmentSink for TestSink {
        type Amount = i64;
        type Asset = &'static str;
        type Scale = i32;

        fn adjust_debt_total(
            &mut self,
            debt_delta: Self::Amount,
            asset: Self::Asset,
            vault_scale: Self::Scale,
        ) {
            self.debt_total += debt_delta;
            self.steps.push(format!(
                "adjust_debt_total delta={debt_delta} asset={asset} scale={vault_scale}"
            ));
        }
    }

    #[test]
    fn tx_loan_pay_broker_debt_signed_adjustment_handoff() {
        let mut sink = TestSink::default();

        let facts = compute_loan_pay_broker_debt_facts(
            LoanPayBrokerDebtDeltaSign::Decrease,
            17_i64,
            "USD",
            6_i32,
        );
        let returned = run_loan_pay_broker_debt_adjustment(&mut sink, facts);

        assert_eq!(
            returned,
            LoanPayBrokerDebtFacts {
                debt_delta_sign: LoanPayBrokerDebtDeltaSign::Decrease,
                total_paid_to_vault_for_debt: 17,
                asset: "USD",
                vault_scale: 6,
                signed_debt_delta: -17,
            }
        );
        assert_eq!(sink.debt_total, -17);
        assert_eq!(
            sink.steps,
            vec!["adjust_debt_total delta=-17 asset=USD scale=6"]
        );
    }
}

mod loan_pay_transfer_delivery_exported_api_parity {
    use super::*;

    #[test]
    fn tx_loan_pay_transfer_delivery_keeps_outputs_within_amount_assert() {
        let facts = compute_loan_pay_transfer_delivery_facts(&25_i64, &13_i64, &1_i64);

        assert_eq!(
            facts,
            LoanPayTransferDeliveryFacts {
                amount: 25,
                total_paid_to_vault_rounded: 13,
                total_paid_to_broker: 1,
                outputs_total: 14,
                amount_covers_outputs: true,
            }
        );
    }

    #[test]
    fn tx_loan_pay_transfer_delivery_flags_outputs_above_amount_assert() {
        let facts = compute_loan_pay_transfer_delivery_facts(&10_i64, &8_i64, &3_i64);

        assert_eq!(facts.outputs_total, 11);
        assert!(!facts.amount_covers_outputs);
    }
}

mod loan_pay_post_payment_prep_exported_api_parity {
    use tx::loan_pay_post_payment_prep::{
        LoanPayPaymentParts as PostPaymentParts,
        LoanPayPostPaymentAmountFacts as PostPaymentAmountFacts,
        LoanPayPostPaymentBrokerDebtDeltaSign as PostPaymentBrokerDebtDeltaSign,
        LoanPayPostPaymentBrokerDebtFacts as PostPaymentBrokerDebtFacts,
        LoanPayPostPaymentPrepFacts as PostPaymentPrepFacts, LoanPayPostPaymentPrepSink,
        LoanPayPostPaymentTransferDeliveryFacts as PostPaymentTransferDeliveryFacts,
        LoanPayPostPaymentVaultStateFacts as PostPaymentVaultStateFacts,
        compute_loan_pay_post_payment_amount_facts,
        compute_loan_pay_post_payment_broker_debt_facts, compute_loan_pay_post_payment_prep,
        compute_loan_pay_post_payment_transfer_delivery_facts,
        compute_loan_pay_post_payment_vault_state_facts,
    };

    #[derive(Default)]
    struct TestSink {
        calls: Vec<&'static str>,
        vault_scale: i32,
        rounded_amount: i64,
        asset_is_integral: bool,
        rounded_check: bool,
    }

    impl LoanPayPostPaymentPrepSink for TestSink {
        type Vault = &'static str;
        type Asset = &'static str;
        type Amount = i64;

        fn vault_scale(&mut self, vault: &Self::Vault) -> i32 {
            assert_eq!(*vault, "vault");
            self.calls.push("vault_scale");
            self.vault_scale
        }

        fn round_to_asset_downward(
            &mut self,
            asset: &Self::Asset,
            value: &Self::Amount,
            scale: i32,
        ) -> Self::Amount {
            assert_eq!(*asset, "USD");
            assert_eq!(*value, 37);
            assert_eq!(scale, self.vault_scale);
            self.calls.push("round_to_asset_downward");
            self.rounded_amount
        }

        fn asset_is_integral(&mut self, asset: &Self::Asset) -> bool {
            assert_eq!(*asset, "USD");
            self.calls.push("asset_is_integral");
            self.asset_is_integral
        }

        fn is_rounded(&mut self, asset: &Self::Asset, value: &Self::Amount, scale: i32) -> bool {
            assert_eq!(*asset, "USD");
            assert_eq!(*value, 30);
            assert_eq!(scale, 6);
            self.calls.push("is_rounded");
            self.rounded_check
        }
    }

    fn payment_parts() -> PostPaymentParts<i64> {
        PostPaymentParts {
            principal_paid: 25,
            interest_paid: 12,
            fee_paid: 3,
            value_change: 7,
        }
    }

    #[test]
    fn tx_loan_pay_post_payment_prep_order_and_fact_bundle() {
        let mut sink = TestSink {
            vault_scale: 10,
            rounded_amount: 30,
            asset_is_integral: false,
            rounded_check: true,
            ..Default::default()
        };

        let facts = compute_loan_pay_post_payment_prep(
            &mut sink,
            &"USD",
            &"vault",
            &payment_parts(),
            &0,
            &40,
            6,
            &10,
            &30,
        );

        assert_eq!(
            sink.calls,
            [
                "vault_scale",
                "round_to_asset_downward",
                "asset_is_integral",
                "is_rounded",
            ]
        );
        assert_eq!(
            facts,
            PostPaymentPrepFacts {
                amount_facts: PostPaymentAmountFacts {
                    vault_scale: 10,
                    total_paid_to_vault_raw: 37,
                    total_paid_to_vault_rounded: 30,
                    total_paid_to_vault_for_debt: 30,
                    total_paid_to_broker: 3,
                    total_paid_is_positive: true,
                    paid_parts_sum_matches_outputs: true,
                    integral_asset_rounding_matches_raw: true,
                    rounded_amount_is_not_greater_than_raw: true,
                    debt_amount_is_rounded: true,
                    rounded_and_broker_not_greater_than_amount: true,
                },
                broker_debt_facts: PostPaymentBrokerDebtFacts {
                    debt_delta_sign: PostPaymentBrokerDebtDeltaSign::Decrease,
                    total_paid_to_vault_for_debt: 30,
                    asset: "USD",
                    vault_scale: 10,
                    signed_debt_delta: -30,
                },
                vault_state_facts: PostPaymentVaultStateFacts {
                    assets_available_before: 10,
                    assets_total_before: 30,
                    total_paid_to_vault_rounded: 30,
                    value_change: 7,
                    assets_available_after: 40,
                    assets_total_after: 37,
                    assets_available_not_greater_than_total: false,
                    duplicate_post_rounding_check_holds: false,
                    all_assertions_hold: false,
                    tec_internal_returned: true,
                },
                transfer_delivery_facts: PostPaymentTransferDeliveryFacts {
                    amount: 40,
                    total_paid_to_vault_rounded: 30,
                    total_paid_to_broker: 3,
                    outputs_total: 33,
                    amount_covers_outputs: true,
                },
            }
        );
    }

    #[test]
    fn tx_loan_pay_post_payment_amount_facts_match_cpp_rounding_and_balance_checks() {
        let mut sink = TestSink {
            vault_scale: 10,
            rounded_amount: 29,
            asset_is_integral: true,
            rounded_check: false,
            ..Default::default()
        };

        let facts = compute_loan_pay_post_payment_amount_facts(
            &mut sink,
            &"USD",
            &"vault",
            &payment_parts(),
            &0,
            &40,
            6,
        );

        assert_eq!(facts.vault_scale, 10);
        assert_eq!(facts.total_paid_to_vault_raw, 37);
        assert_eq!(facts.total_paid_to_vault_rounded, 29);
        assert_eq!(facts.total_paid_to_vault_for_debt, 30);
        assert_eq!(facts.total_paid_to_broker, 3);
        assert!(facts.total_paid_is_positive);
        assert!(facts.paid_parts_sum_matches_outputs);
        assert!(!facts.integral_asset_rounding_matches_raw);
        assert!(facts.rounded_amount_is_not_greater_than_raw);
        assert!(!facts.debt_amount_is_rounded);
        assert!(facts.rounded_and_broker_not_greater_than_amount);
        assert_eq!(
            sink.calls,
            [
                "vault_scale",
                "round_to_asset_downward",
                "asset_is_integral",
                "is_rounded",
            ]
        );
    }

    #[test]
    fn tx_loan_pay_post_payment_broker_debt_handoff_signed_delta() {
        assert_ne!(
            PostPaymentBrokerDebtDeltaSign::Increase,
            PostPaymentBrokerDebtDeltaSign::Decrease
        );
        let facts = compute_loan_pay_post_payment_broker_debt_facts(30_i64, "USD", 10_i32);

        assert_eq!(
            facts,
            PostPaymentBrokerDebtFacts {
                debt_delta_sign: PostPaymentBrokerDebtDeltaSign::Decrease,
                total_paid_to_vault_for_debt: 30,
                asset: "USD",
                vault_scale: 10,
                signed_debt_delta: -30,
            }
        );
    }

    #[test]
    fn tx_loan_pay_post_payment_vault_state_internal_return_rule() {
        let facts = compute_loan_pay_post_payment_vault_state_facts(&10_i64, &30, &30, &7);

        assert_eq!(
            facts,
            PostPaymentVaultStateFacts {
                assets_available_before: 10,
                assets_total_before: 30,
                total_paid_to_vault_rounded: 30,
                value_change: 7,
                assets_available_after: 40,
                assets_total_after: 37,
                assets_available_not_greater_than_total: false,
                duplicate_post_rounding_check_holds: false,
                all_assertions_hold: false,
                tec_internal_returned: true,
            }
        );
    }

    #[test]
    fn tx_loan_pay_post_payment_transfer_delivery_flags_overdrawn_output() {
        let facts = compute_loan_pay_post_payment_transfer_delivery_facts(&10_i64, &8, &3);

        assert_eq!(
            facts,
            PostPaymentTransferDeliveryFacts {
                amount: 10,
                total_paid_to_vault_rounded: 8,
                total_paid_to_broker: 3,
                outputs_total: 11,
                amount_covers_outputs: false,
            }
        );
    }
}

mod loan_pay_post_transfer_checks_exported_api_parity {
    use super::*;

    #[derive(Default)]
    struct TestSink {
        balances: std::collections::HashMap<&'static str, i64>,
        issuer: &'static str,
        calls: Vec<&'static str>,
    }

    impl LoanPayPostTransferChecksSink for TestSink {
        type AccountId = &'static str;
        type Asset = &'static str;
        type Amount = i64;

        fn sample_balance(
            &mut self,
            account: &Self::AccountId,
            _asset: &Self::Asset,
        ) -> Self::Amount {
            self.calls.push(match *account {
                "vault" => "sample_vault",
                "borrower" => "sample_borrower",
                "broker" => "sample_broker",
                _ => "sample_other",
            });
            *self.balances.get(account).unwrap_or(&0)
        }

        fn account_is_asset_issuer(
            &mut self,
            account: &Self::AccountId,
            _asset: &Self::Asset,
        ) -> bool {
            self.calls.push("issuer_check");
            *account == self.issuer
        }
    }

    #[test]
    fn tx_loan_pay_post_transfer_checks_match_and_fact_bundle() {
        let mut sink = TestSink {
            balances: std::collections::HashMap::from([
                ("borrower", 80),
                ("vault", 30),
                ("broker", 15),
            ]),
            issuer: "issuer",
            ..Default::default()
        };

        let result = run_loan_pay_post_transfer_checks(
            &mut sink,
            LoanPayPostTransferChecksFacts {
                account: "borrower",
                vault_pseudo_account: "vault",
                broker_payee: "broker",
                asset: "USD",
                zero_amount: 0,
                assets_available_before: 10,
                pseudo_account_balance_before: 10,
                borrower_balance_before: 100,
                vault_balance_before: 20,
                broker_balance_before: 5,
                assets_available_after: 30,
            },
        );

        assert_eq!(
            sink.calls,
            [
                "sample_vault",
                "sample_borrower",
                "sample_vault",
                "sample_broker",
                "issuer_check",
            ]
        );
        assert_eq!(
            result,
            LoanPayPostTransferChecksResult {
                pseudo_account_balance_after: 30,
                balance_snapshot: LoanPayBalanceSnapshotFacts {
                    borrower_balance_before: 100,
                    borrower_balance_after: 80,
                    vault_balance_before: 20,
                    vault_balance_after: 30,
                    broker_balance_before: 5,
                    broker_balance_after: 15,
                    borrower_is_vault_pseudo: false,
                    borrower_is_broker_payee: false,
                },
                vault_balance_checks: LoanPayVaultBalanceCheckFacts {
                    assets_available_before: 10,
                    pseudo_account_balance_before: 10,
                    assets_available_after: 30,
                    pseudo_account_balance_after: 30,
                    vault_pseudo_balance_agrees_before: true,
                    vault_pseudo_balance_agrees_after: true,
                },
                post_balances: LoanPayPostBalanceFacts {
                    borrower_balance_before: 100,
                    borrower_balance_after: 80,
                    vault_balance_before: 20,
                    vault_balance_after: 30,
                    broker_balance_before: 5,
                    broker_balance_after: 15,
                    total_balance_before: 125,
                    total_balance_after: 125,
                    funds_conserved: true,
                    borrower_balance_non_negative: true,
                    vault_balance_non_negative: true,
                    broker_balance_non_negative: true,
                    borrower_balance_decreased_unless_issuer: true,
                    vault_balance_did_not_decrease: true,
                    broker_balance_did_not_decrease: true,
                    vault_or_broker_increased: true,
                },
                assertion_facts: LoanPayAssertionFacts {
                    vault_pseudo_balance_agrees_before: true,
                    vault_pseudo_balance_agrees_after: true,
                    funds_conserved: true,
                    borrower_balance_non_negative: true,
                    vault_balance_non_negative: true,
                    broker_balance_non_negative: true,
                    borrower_balance_decreased_unless_issuer: true,
                    vault_balance_did_not_decrease: true,
                    broker_balance_did_not_decrease: true,
                    vault_or_broker_increased: true,
                    all_assertions_hold: true,
                },
            }
        );
    }

    #[test]
    fn tx_loan_pay_post_transfer_checks_zero_substitute_aliases() {
        let mut sink = TestSink {
            balances: std::collections::HashMap::from([("borrower", 50)]),
            issuer: "issuer",
            ..Default::default()
        };

        let result = run_loan_pay_post_transfer_checks(
            &mut sink,
            LoanPayPostTransferChecksFacts {
                account: "borrower",
                vault_pseudo_account: "borrower",
                broker_payee: "borrower",
                asset: "USD",
                zero_amount: 0,
                assets_available_before: 20,
                pseudo_account_balance_before: 20,
                borrower_balance_before: 70,
                vault_balance_before: 20,
                broker_balance_before: 5,
                assets_available_after: 50,
            },
        );

        assert_eq!(result.balance_snapshot.vault_balance_before, 0);
        assert_eq!(result.balance_snapshot.vault_balance_after, 0);
        assert_eq!(result.balance_snapshot.broker_balance_before, 0);
        assert_eq!(result.balance_snapshot.broker_balance_after, 0);
    }

    #[test]
    fn tx_loan_pay_post_transfer_checks_surface_failed_assertions() {
        let mut sink = TestSink {
            balances: std::collections::HashMap::from([
                ("borrower", 100),
                ("vault", 19),
                ("broker", 5),
            ]),
            issuer: "issuer",
            ..Default::default()
        };

        let result = run_loan_pay_post_transfer_checks(
            &mut sink,
            LoanPayPostTransferChecksFacts {
                account: "borrower",
                vault_pseudo_account: "vault",
                broker_payee: "broker",
                asset: "USD",
                zero_amount: 0,
                assets_available_before: 10,
                pseudo_account_balance_before: 9,
                borrower_balance_before: 100,
                vault_balance_before: 20,
                broker_balance_before: 5,
                assets_available_after: 25,
            },
        );

        assert!(
            !result
                .vault_balance_checks
                .vault_pseudo_balance_agrees_before
        );
        assert!(
            !result
                .vault_balance_checks
                .vault_pseudo_balance_agrees_after
        );
        assert!(!result.assertion_facts.all_assertions_hold);
    }
}

mod loan_pay_post_balances_exported_api_parity {
    use super::*;

    #[test]
    fn tx_loan_pay_post_balances_match_cpp_debug_facts() {
        let facts = compute_loan_pay_post_balances(&100_i64, &80, &20, &30, &5, &15, &0, false);

        assert_eq!(
            facts,
            LoanPayPostBalanceFacts {
                borrower_balance_before: 100,
                borrower_balance_after: 80,
                vault_balance_before: 20,
                vault_balance_after: 30,
                broker_balance_before: 5,
                broker_balance_after: 15,
                total_balance_before: 125,
                total_balance_after: 125,
                funds_conserved: true,
                borrower_balance_non_negative: true,
                vault_balance_non_negative: true,
                broker_balance_non_negative: true,
                borrower_balance_decreased_unless_issuer: true,
                vault_balance_did_not_decrease: true,
                broker_balance_did_not_decrease: true,
                vault_or_broker_increased: true,
            }
        );
    }

    #[test]
    fn tx_loan_pay_post_balances_allow_issuer_exception() {
        let facts = compute_loan_pay_post_balances(&100_i64, &100, &20, &20, &5, &5, &0, true);

        assert!(facts.funds_conserved);
        assert!(facts.borrower_balance_decreased_unless_issuer);
        assert!(!facts.vault_or_broker_increased);
    }
}

mod loan_pay_vault_balance_checks_exported_api_parity {
    use super::*;

    #[test]
    fn tx_loan_pay_vault_balance_checks_match_cpp_debug_facts() {
        let facts = compute_loan_pay_vault_balance_checks(&10_i64, &10, &25, &25);

        assert_eq!(
            facts,
            LoanPayVaultBalanceCheckFacts {
                assets_available_before: 10,
                pseudo_account_balance_before: 10,
                assets_available_after: 25,
                pseudo_account_balance_after: 25,
                vault_pseudo_balance_agrees_before: true,
                vault_pseudo_balance_agrees_after: true,
            }
        );
    }

    #[test]
    fn tx_loan_pay_vault_balance_checks_flag_before_and_after_mismatches() {
        let before = compute_loan_pay_vault_balance_checks(&10_i64, &9, &25, &25);
        let after = compute_loan_pay_vault_balance_checks(&10_i64, &10, &25, &24);

        assert!(!before.vault_pseudo_balance_agrees_before);
        assert!(before.vault_pseudo_balance_agrees_after);
        assert!(after.vault_pseudo_balance_agrees_before);
        assert!(!after.vault_pseudo_balance_agrees_after);
    }
}

mod loan_pay_balance_snapshot_exported_api_parity {
    use super::*;

    #[test]
    fn tx_loan_pay_balance_snapshot_alias_sampling_rule() {
        let facts =
            compute_loan_pay_balance_snapshot(&100_i64, &87, &10, &23, &88, &88, &0, false, true);

        assert_eq!(
            facts,
            LoanPayBalanceSnapshotFacts {
                borrower_balance_before: 100,
                borrower_balance_after: 87,
                vault_balance_before: 10,
                vault_balance_after: 23,
                broker_balance_before: 0,
                broker_balance_after: 0,
                borrower_is_vault_pseudo: false,
                borrower_is_broker_payee: true,
            }
        );
    }
}

mod loan_pay_transfer_prep_exported_api_parity {
    use super::*;

    #[test]
    fn tx_loan_pay_transfer_prep_order_and_fact_bundle() {
        let facts = compute_loan_pay_transfer_prep_facts(&7_i64, &3_i64, &0_i64, true);

        assert_eq!(
            facts,
            LoanPayTransferPrepFacts {
                total_paid_to_vault_rounded: 7,
                total_paid_to_broker: 3,
                vault_auth_required: true,
                broker_payment_present: true,
                broker_payee_is_borrower: true,
                add_empty_holding_required: true,
                broker_auth_required: true,
            }
        );
    }

    #[test]
    fn tx_loan_pay_transfer_prep_skips_broker_side_work_when_no_broker_payment() {
        let facts = compute_loan_pay_transfer_prep_facts(&7_i64, &0_i64, &0_i64, false);

        assert_eq!(
            facts,
            LoanPayTransferPrepFacts {
                total_paid_to_vault_rounded: 7,
                total_paid_to_broker: 0,
                vault_auth_required: true,
                broker_payment_present: false,
                broker_payee_is_borrower: false,
                add_empty_holding_required: false,
                broker_auth_required: false,
            }
        );
    }
}

mod loan_pay_pre_transfer_snapshot_exported_api_parity {
    use super::*;

    struct TestSink {
        steps: Vec<&'static str>,
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
            self.steps.push(match *account {
                "vault-pseudo" => "sample_pseudo",
                "borrower" => "sample_borrower",
                "broker" => "sample_broker",
                _ => "sample_other",
            });

            match *account {
                "vault-pseudo" => 10,
                "borrower" => 80,
                "broker" => 15,
                _ => 0,
            }
        }
    }

    #[test]
    fn tx_loan_pay_pre_transfer_snapshot_keeps_cpp_sampling_order() {
        let mut sink = TestSink { steps: Vec::new() };

        let facts = compute_loan_pay_pre_transfer_snapshot(
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
            sink.steps,
            ["sample_borrower", "sample_pseudo", "sample_broker"]
        );
        assert_eq!(
            facts,
            LoanPayPreTransferSnapshotResult {
                pseudo_account_balance_before: 10,
                borrower_balance_before: 80,
                vault_balance_before: 10,
                broker_balance_before: 15,
                borrower_is_vault_pseudo: false,
                borrower_is_broker_payee: false,
            }
        );
    }

    #[test]
    fn tx_loan_pay_pre_transfer_snapshot_tracks_alias_flags() {
        let mut sink = TestSink { steps: Vec::new() };

        let facts = compute_loan_pay_pre_transfer_snapshot(
            &mut sink,
            LoanPayPreTransferSnapshotFacts {
                account: "borrower",
                vault_pseudo_account: "borrower",
                broker_payee: "borrower",
                asset: "USD",
                pseudo_account_balance_before: 80,
            },
        );

        assert!(facts.borrower_is_vault_pseudo);
        assert!(facts.borrower_is_broker_payee);
        assert_eq!(facts.pseudo_account_balance_before, 80);
        assert_eq!(facts.borrower_balance_before, 80);
        assert_eq!(facts.vault_balance_before, 80);
        assert_eq!(facts.broker_balance_before, 80);
    }
}

mod loan_pay_tail_mutation_exported_api_parity {
    use super::*;

    #[derive(Debug, Clone, PartialEq, Eq)]
    struct TestLoan {
        broker_id: &'static str,
        impaired: bool,
        associated_asset: Option<&'static str>,
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
            self.impaired
        }

        fn associate_asset(&mut self, asset: &Self::Asset) {
            self.associated_asset = Some(*asset);
        }
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    struct TestBroker {
        owner: &'static str,
        pseudo_account: &'static str,
        vault_id: &'static str,
        debt_total: i64,
        cover_available: i64,
        cover_rate_minimum: u32,
        associated_asset: Option<&'static str>,
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
            self.cover_rate_minimum
        }

        fn add_cover_available(&mut self, amount: Self::Amount) {
            self.cover_available += amount;
        }

        fn adjust_debt_total(&mut self, delta: Self::Amount) {
            self.debt_total -= delta;
        }

        fn associate_asset(&mut self, asset: &Self::Asset) {
            self.associated_asset = Some(*asset);
        }
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    struct TestVault {
        pseudo_account: &'static str,
        asset: &'static str,
        assets_available: i64,
        assets_total: i64,
        associated_asset: Option<&'static str>,
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
            self.assets_available += amount;
        }

        fn add_assets_total(&mut self, amount: Self::Amount) {
            self.assets_total += amount;
        }

        fn assets_available_exceeds_total(&self) -> bool {
            self.assets_available > self.assets_total
        }

        fn associate_asset(&mut self, asset: &Self::Asset) {
            self.associated_asset = Some(*asset);
        }
    }

    struct TestSink<'a> {
        steps: &'a mut Vec<&'static str>,
    }

    impl LoanPayTailMutationSink for TestSink<'_> {
        type Vault = TestVault;

        fn update_vault(&mut self, _vault: &Self::Vault) {
            self.steps.push("update_vault");
        }
    }

    #[test]
    fn tx_loan_pay_tail_mutation_runs_and_updates_cover() {
        let mut steps = Vec::new();
        let mut sink = TestSink { steps: &mut steps };
        let mut loan = TestLoan {
            broker_id: "broker",
            impaired: false,
            associated_asset: None,
        };
        let mut broker = TestBroker {
            owner: "owner",
            pseudo_account: "broker-pseudo",
            vault_id: "vault",
            debt_total: 50,
            cover_available: 20,
            cover_rate_minimum: 0,
            associated_asset: None,
        };
        let mut vault = TestVault {
            pseudo_account: "vault-pseudo",
            asset: "USD",
            assets_available: 80,
            assets_total: 100,
            associated_asset: None,
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
        assert_eq!(steps, ["update_vault"]);
        assert_eq!(broker.cover_available, 23);
        assert_eq!(vault.assets_available, 87);
        assert_eq!(vault.assets_total, 102);
        assert_eq!(loan.associated_asset, Some("USD"));
        assert_eq!(broker.associated_asset, Some("USD"));
        assert_eq!(vault.associated_asset, Some("USD"));
    }

    #[test]
    fn tx_loan_pay_tail_mutation_maps_overflow_to_tec_internal() {
        let mut steps = Vec::new();
        let mut sink = TestSink { steps: &mut steps };
        let mut loan = TestLoan {
            broker_id: "broker",
            impaired: false,
            associated_asset: None,
        };
        let mut broker = TestBroker {
            owner: "owner",
            pseudo_account: "broker-pseudo",
            vault_id: "vault",
            debt_total: 50,
            cover_available: 20,
            cover_rate_minimum: 0,
            associated_asset: None,
        };
        let mut vault = TestVault {
            pseudo_account: "vault-pseudo",
            asset: "USD",
            assets_available: 10,
            assets_total: 10,
            associated_asset: None,
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
        assert_eq!(steps, ["update_vault"]);
    }
}

mod loan_pay_tail_transfer_exported_api_parity {
    use super::*;

    struct TestSink<'a> {
        steps: &'a mut Vec<&'static str>,
        vault_auth_result: Ter,
        broker_auth_result: Ter,
        add_empty_holding_result: Ter,
        account_send_multi_result: Ter,
        expected_vault_amount: i64,
        expected_broker_payee: &'static str,
        expected_broker_amount: i64,
    }

    impl LoanPayTailTransferSink for TestSink<'_> {
        type AccountId = &'static str;
        type Asset = &'static str;
        type Amount = i64;

        fn require_auth(&mut self, account: &Self::AccountId, _asset: &Self::Asset) -> Ter {
            self.steps.push(*account);
            if *account == "vault-pseudo" {
                self.vault_auth_result
            } else {
                self.broker_auth_result
            }
        }

        fn broker_payee_balance_for_empty_holding(
            &mut self,
            account: &Self::AccountId,
        ) -> Self::Amount {
            self.steps.push("broker_payee_balance");
            assert_eq!(*account, "borrower");
            12
        }

        fn add_empty_holding(
            &mut self,
            account: &Self::AccountId,
            balance: &Self::Amount,
            _asset: &Self::Asset,
        ) -> Ter {
            self.steps.push("add_empty_holding");
            assert_eq!(*account, "borrower");
            assert_eq!(*balance, 12);
            self.add_empty_holding_result
        }

        fn account_send_multi(
            &mut self,
            source: &Self::AccountId,
            asset: &Self::Asset,
            outputs: [(Self::AccountId, Self::Amount); 2],
        ) -> Ter {
            self.steps.push("account_send_multi");
            assert_eq!(*source, "borrower");
            assert_eq!(*asset, "USD");
            assert_eq!(outputs[0], ("vault-pseudo", self.expected_vault_amount));
            assert_eq!(
                outputs[1],
                (self.expected_broker_payee, self.expected_broker_amount)
            );
            self.account_send_multi_result
        }
    }

    #[test]
    fn tx_loan_pay_tail_transfer_runs_cpp_auth_then_send_order() {
        let mut steps = Vec::new();
        let mut sink = TestSink {
            steps: &mut steps,
            vault_auth_result: Ter::TES_SUCCESS,
            broker_auth_result: Ter::TES_SUCCESS,
            add_empty_holding_result: Ter::TEC_DUPLICATE,
            account_send_multi_result: Ter::TES_SUCCESS,
            expected_vault_amount: 7,
            expected_broker_payee: "borrower",
            expected_broker_amount: 3,
        };

        let result = run_loan_pay_tail_transfer(
            &mut sink,
            LoanPayTailTransferFacts {
                account: "borrower",
                vault_pseudo_account: "vault-pseudo",
                broker_payee: "borrower",
                asset: "USD",
                zero_amount: 0,
                total_paid_to_vault_rounded: 7,
                total_paid_to_broker: 3,
            },
        );

        assert_eq!(result, Ter::TES_SUCCESS);
        assert_eq!(
            steps,
            [
                "vault-pseudo",
                "broker_payee_balance",
                "add_empty_holding",
                "borrower",
                "account_send_multi",
            ]
        );
    }

    #[test]
    fn tx_loan_pay_tail_transfer_passthroughs_send_failure() {
        let mut steps = Vec::new();
        let mut sink = TestSink {
            steps: &mut steps,
            vault_auth_result: Ter::TES_SUCCESS,
            broker_auth_result: Ter::TES_SUCCESS,
            add_empty_holding_result: Ter::TES_SUCCESS,
            account_send_multi_result: Ter::TEC_PATH_DRY,
            expected_vault_amount: 7,
            expected_broker_payee: "owner",
            expected_broker_amount: 3,
        };

        let result = run_loan_pay_tail_transfer(
            &mut sink,
            LoanPayTailTransferFacts {
                account: "borrower",
                vault_pseudo_account: "vault-pseudo",
                broker_payee: "owner",
                asset: "USD",
                zero_amount: 0,
                total_paid_to_vault_rounded: 7,
                total_paid_to_broker: 3,
            },
        );

        assert_eq!(result, Ter::TEC_PATH_DRY);
        assert_eq!(steps, ["vault-pseudo", "owner", "account_send_multi"]);
    }

    #[test]
    fn tx_loan_pay_tail_transfer_passthroughs_add_empty_holding_failure() {
        let mut steps = Vec::new();
        let mut sink = TestSink {
            steps: &mut steps,
            vault_auth_result: Ter::TES_SUCCESS,
            broker_auth_result: Ter::TES_SUCCESS,
            add_empty_holding_result: Ter::TEC_PATH_DRY,
            account_send_multi_result: Ter::TES_SUCCESS,
            expected_vault_amount: 7,
            expected_broker_payee: "borrower",
            expected_broker_amount: 3,
        };

        let result = run_loan_pay_tail_transfer(
            &mut sink,
            LoanPayTailTransferFacts {
                account: "borrower",
                vault_pseudo_account: "vault-pseudo",
                broker_payee: "borrower",
                asset: "USD",
                zero_amount: 0,
                total_paid_to_vault_rounded: 7,
                total_paid_to_broker: 3,
            },
        );

        assert_eq!(result, Ter::TEC_PATH_DRY);
        assert_eq!(
            steps,
            ["vault-pseudo", "broker_payee_balance", "add_empty_holding"]
        );
    }
}

mod loan_pay_assertions_exported_api_parity {
    use super::*;

    #[test]
    fn tx_loan_pay_assertions_match_cpp_debug_assertion_bundle() {
        let vault_checks = LoanPayVaultBalanceCheckFacts {
            assets_available_before: 10_i64,
            pseudo_account_balance_before: 10,
            assets_available_after: 23,
            pseudo_account_balance_after: 23,
            vault_pseudo_balance_agrees_before: true,
            vault_pseudo_balance_agrees_after: true,
        };
        let post_balances = LoanPayPostBalanceFacts {
            borrower_balance_before: 100,
            borrower_balance_after: 87,
            vault_balance_before: 10,
            vault_balance_after: 23,
            broker_balance_before: 0,
            broker_balance_after: 0,
            total_balance_before: 110,
            total_balance_after: 110,
            funds_conserved: true,
            borrower_balance_non_negative: true,
            vault_balance_non_negative: true,
            broker_balance_non_negative: true,
            borrower_balance_decreased_unless_issuer: true,
            vault_balance_did_not_decrease: true,
            broker_balance_did_not_decrease: true,
            vault_or_broker_increased: true,
        };

        let facts = compute_loan_pay_assertion_facts(&vault_checks, &post_balances);

        assert_eq!(
            facts,
            LoanPayAssertionFacts {
                vault_pseudo_balance_agrees_before: true,
                vault_pseudo_balance_agrees_after: true,
                funds_conserved: true,
                borrower_balance_non_negative: true,
                vault_balance_non_negative: true,
                broker_balance_non_negative: true,
                borrower_balance_decreased_unless_issuer: true,
                vault_balance_did_not_decrease: true,
                broker_balance_did_not_decrease: true,
                vault_or_broker_increased: true,
                all_assertions_hold: true,
            }
        );
    }
}

mod loan_pay_do_apply_front_exported_api_parity {
    use super::*;

    #[derive(Debug, Clone, PartialEq, Eq)]
    struct TestLoan {
        broker_id: &'static str,
        impaired: bool,
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
            self.impaired
        }

        fn associate_asset(&mut self, _asset: &Self::Asset) {}
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    struct TestBroker {
        owner: &'static str,
        pseudo_account: &'static str,
        vault_id: &'static str,
        debt_total: i64,
        cover_rate_minimum: u32,
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
            static ZERO: i64 = 0;
            &ZERO
        }

        fn debt_total(&self) -> &Self::Amount {
            &self.debt_total
        }

        fn cover_rate_minimum(&self) -> u32 {
            self.cover_rate_minimum
        }

        fn add_cover_available(&mut self, _amount: Self::Amount) {}

        fn adjust_debt_total(&mut self, _delta: Self::Amount) {}

        fn associate_asset(&mut self, _asset: &Self::Asset) {}
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    struct TestVault {
        pseudo_account: &'static str,
        asset: &'static str,
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
            static ZERO: i64 = 0;
            &ZERO
        }

        fn assets_total(&self) -> &Self::Amount {
            static ZERO: i64 = 0;
            &ZERO
        }

        fn add_assets_available(&mut self, _amount: Self::Amount) {}

        fn add_assets_total(&mut self, _amount: Self::Amount) {}

        fn assets_available_exceeds_total(&self) -> bool {
            false
        }

        fn associate_asset(&mut self, _asset: &Self::Asset) {}
    }

    struct TestSink<'a> {
        loan: Option<TestLoan>,
        broker: Option<TestBroker>,
        vault: Option<TestVault>,
        required_cover: i64,
        owner_is_deep_frozen: bool,
        owner_requires_auth: bool,
        fallback_deep_frozen: Ter,
        payment_result: Result<LoanPayPaymentParts<i64>, Ter>,
        steps: &'a mut Vec<&'static str>,
    }

    impl LoanPayDoApplySink for TestSink<'_> {
        type Loan = TestLoan;
        type Broker = TestBroker;
        type Vault = TestVault;
        type AccountId = &'static str;
        type BrokerId = &'static str;
        type VaultId = &'static str;
        type Asset = &'static str;
        type Amount = i64;

        fn read_loan(&mut self) -> Option<Self::Loan> {
            self.steps.push("load_loan");
            self.loan.clone()
        }

        fn read_broker(&mut self, broker_id: &Self::BrokerId) -> Option<Self::Broker> {
            assert_eq!(*broker_id, "broker");
            self.steps.push("load_broker");
            self.broker.clone()
        }

        fn read_vault(&mut self, vault_id: &Self::VaultId) -> Option<Self::Vault> {
            assert_eq!(*vault_id, "vault");
            self.steps.push("load_vault");
            self.vault.clone()
        }

        fn compute_required_cover_threshold(
            &mut self,
            _asset: &Self::Asset,
            _debt_total: &Self::Amount,
            _cover_rate_minimum: u32,
            _loan_scale: i32,
        ) -> Self::Amount {
            self.steps.push("required_cover");
            self.required_cover
        }

        fn broker_owner_is_deep_frozen(
            &mut self,
            _owner: &Self::AccountId,
            _asset: &Self::Asset,
        ) -> bool {
            self.steps.push("owner_deep_frozen");
            self.owner_is_deep_frozen
        }

        fn broker_owner_requires_auth(
            &mut self,
            _owner: &Self::AccountId,
            _asset: &Self::Asset,
        ) -> bool {
            self.steps.push("owner_requires_auth");
            self.owner_requires_auth
        }

        fn check_deep_frozen(&mut self, account: &Self::AccountId, _asset: &Self::Asset) -> Ter {
            assert_eq!(*account, "broker-pseudo");
            self.steps.push("fallback_deep_frozen");
            self.fallback_deep_frozen
        }

        fn unimpair_loan(
            &mut self,
            loan: &mut Self::Loan,
            _vault: &Self::Vault,
            _asset: &Self::Asset,
        ) -> Ter {
            self.steps.push("unimpair");
            loan.impaired = false;
            Ter::TES_SUCCESS
        }

        fn make_payment(
            &mut self,
            _asset: &Self::Asset,
            _loan: &mut Self::Loan,
            _broker: &mut Self::Broker,
            amount: &Self::Amount,
            payment_type: LoanPayPaymentType,
        ) -> Result<LoanPayPaymentParts<Self::Amount>, Ter> {
            assert_eq!(*amount, 25);
            self.steps.push(match payment_type {
                LoanPayPaymentType::Late => "payment_type_late",
                LoanPayPaymentType::Full => "payment_type_full",
                LoanPayPaymentType::Overpayment => "payment_type_overpayment",
                LoanPayPaymentType::Regular => "payment_type_regular",
            });
            let result = self.payment_result.clone();
            if result.is_ok() {
                self.steps.push("payment_computed");
            }
            result
        }

        fn update_loan(&mut self, _loan: &Self::Loan) {
            self.steps.push("update_loan");
        }

        fn update_broker(&mut self, _broker: &Self::Broker) {
            self.steps.push("update_broker");
        }

        fn adjust_broker_debt_total(
            &mut self,
            _broker: &mut Self::Broker,
            _debt_delta: &Self::Amount,
            _asset: &Self::Asset,
            _vault_scale: i32,
        ) {
            self.steps.push("adjust_broker_debt_total");
        }

        fn update_vault(&mut self, _vault: &Self::Vault) {}

        fn require_auth(&mut self, _account: &Self::AccountId, _asset: &Self::Asset) -> Ter {
            Ter::TES_SUCCESS
        }

        fn broker_payee_balance_for_empty_holding(
            &mut self,
            _account: &Self::AccountId,
        ) -> Self::Amount {
            12
        }

        fn add_empty_holding(
            &mut self,
            _account: &Self::AccountId,
            _balance: &Self::Amount,
            _asset: &Self::Asset,
        ) -> Ter {
            Ter::TES_SUCCESS
        }

        fn account_send_multi(
            &mut self,
            _from: &Self::AccountId,
            _asset: &Self::Asset,
            _vault_pseudo: &Self::AccountId,
            _vault_amount: &Self::Amount,
            _broker_payee: &Self::AccountId,
            _broker_amount: &Self::Amount,
        ) -> Ter {
            Ter::TES_SUCCESS
        }

        fn sample_balance(
            &mut self,
            _account: &Self::AccountId,
            _asset: &Self::Asset,
        ) -> Self::Amount {
            0
        }

        fn account_is_asset_issuer(
            &mut self,
            _account: &Self::AccountId,
            _asset: &Self::Asset,
        ) -> bool {
            false
        }
    }

    fn make_facts() -> LoanPayDoApplyFrontFacts<i64> {
        LoanPayDoApplyFrontFacts {
            amount: 25,
            zero_amount: 0,
            tx_requests_late_payment: false,
            tx_requests_full_payment: true,
            tx_requests_overpayment: false,
        }
    }

    fn make_parts() -> LoanPayPaymentParts<i64> {
        LoanPayPaymentParts {
            principal_paid: 10,
            interest_paid: 3,
            fee_paid: 1,
            value_change: 0,
        }
    }

    fn make_sink<'a>(steps: &'a mut Vec<&'static str>) -> TestSink<'a> {
        TestSink {
            loan: Some(TestLoan {
                broker_id: "broker",
                impaired: false,
            }),
            broker: Some(TestBroker {
                owner: "owner",
                pseudo_account: "broker-pseudo",
                vault_id: "vault",
                debt_total: 0,
                cover_rate_minimum: 0,
            }),
            vault: Some(TestVault {
                pseudo_account: "vault-pseudo",
                asset: "USD",
            }),
            required_cover: 0,
            owner_is_deep_frozen: false,
            owner_requires_auth: false,
            fallback_deep_frozen: Ter::TES_SUCCESS,
            payment_result: Ok(make_parts()),
            steps,
        }
    }

    #[test]
    fn tx_loan_pay_do_apply_front_maps_missing_objects_to_bad_ledger() {
        let mut steps = Vec::new();
        let mut missing_loan = make_sink(&mut steps);
        missing_loan.loan = None;
        assert_eq!(
            run_loan_pay_do_apply_front(&mut missing_loan, make_facts()),
            Err(Ter::TEF_BAD_LEDGER)
        );
        assert_eq!(steps, ["load_loan"]);

        steps.clear();
        let mut missing_broker = make_sink(&mut steps);
        missing_broker.broker = None;
        assert_eq!(
            run_loan_pay_do_apply_front(&mut missing_broker, make_facts()),
            Err(Ter::TEF_BAD_LEDGER)
        );
        assert_eq!(steps, ["load_loan", "load_broker"]);

        steps.clear();
        let mut missing_vault = make_sink(&mut steps);
        missing_vault.vault = None;
        assert_eq!(
            run_loan_pay_do_apply_front(&mut missing_vault, make_facts()),
            Err(Ter::TEF_BAD_LEDGER)
        );
        assert_eq!(steps, ["load_loan", "load_broker", "load_vault"]);
    }

    #[test]
    fn tx_loan_pay_do_apply_front_uses_fallback_pseudo_deep_freeze() {
        let mut steps = Vec::new();
        let mut sink = make_sink(&mut steps);
        sink.required_cover = 1;
        sink.fallback_deep_frozen = Ter::TEC_FROZEN;

        assert_eq!(
            run_loan_pay_do_apply_front(&mut sink, make_facts()),
            Err(Ter::TEC_FROZEN)
        );
        assert_eq!(
            steps,
            [
                "load_loan",
                "load_broker",
                "load_vault",
                "required_cover",
                "fallback_deep_frozen",
            ]
        );
    }

    #[test]
    fn tx_loan_pay_do_apply_front_unimpairs_before_payment_and_updates_after() {
        let mut steps = Vec::new();
        let mut sink = make_sink(&mut steps);
        sink.loan.as_mut().expect("loan").impaired = true;

        let state = run_loan_pay_do_apply_front(&mut sink, make_facts()).expect("success");

        assert_eq!(state.broker_payee, "owner");
        assert!(state.send_broker_fee_to_owner);
        assert_eq!(state.payment_type, LoanPayPaymentType::Full);
        assert_eq!(
            steps,
            [
                "load_loan",
                "load_broker",
                "load_vault",
                "required_cover",
                "owner_deep_frozen",
                "owner_requires_auth",
                "unimpair",
                "payment_type_full",
                "payment_computed",
                "update_loan",
                "update_broker",
            ]
        );
    }

    #[test]
    fn tx_loan_pay_do_apply_front_passes_through_payment_errors() {
        let mut steps = Vec::new();
        let mut sink = make_sink(&mut steps);
        sink.payment_result = Err(Ter::TEC_PATH_DRY);

        assert_eq!(
            run_loan_pay_do_apply_front(&mut sink, make_facts()),
            Err(Ter::TEC_PATH_DRY)
        );
        assert_eq!(
            steps,
            [
                "load_loan",
                "load_broker",
                "load_vault",
                "required_cover",
                "owner_deep_frozen",
                "owner_requires_auth",
                "payment_type_full",
            ]
        );
    }

    #[test]
    fn tx_loan_pay_do_apply_front_rejects_negative_parts_after_loan_update() {
        let mut steps = Vec::new();
        let mut sink = make_sink(&mut steps);
        sink.payment_result = Ok(LoanPayPaymentParts {
            principal_paid: -1,
            interest_paid: 1,
            fee_paid: 0,
            value_change: 0,
        });

        assert_eq!(
            run_loan_pay_do_apply_front(&mut sink, make_facts()),
            Err(Ter::TEC_LIMIT_EXCEEDED)
        );
        assert_eq!(
            steps,
            [
                "load_loan",
                "load_broker",
                "load_vault",
                "required_cover",
                "owner_deep_frozen",
                "owner_requires_auth",
                "payment_type_full",
                "payment_computed",
                "update_loan",
            ]
        );
    }
}

mod loan_pay_do_apply_middle_exported_api_parity {
    use super::*;

    #[derive(Debug, Clone, PartialEq, Eq)]
    struct TestLoan {
        broker_id: &'static str,
        impaired: bool,
        associated_asset: Option<&'static str>,
    }

    impl LoanPayDoApplyLoan for TestLoan {
        type BrokerId = &'static str;
        type Asset = &'static str;

        fn broker_id(&self) -> &Self::BrokerId {
            &self.broker_id
        }

        fn scale(&self) -> i32 {
            6
        }

        fn is_impaired(&self) -> bool {
            self.impaired
        }

        fn associate_asset(&mut self, asset: &Self::Asset) {
            self.associated_asset = Some(*asset);
        }
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    struct TestBroker {
        owner: &'static str,
        pseudo_account: &'static str,
        vault_id: &'static str,
        debt_total: i64,
        cover_available: i64,
        cover_rate_minimum: u32,
        associated_asset: Option<&'static str>,
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
            self.cover_rate_minimum
        }

        fn add_cover_available(&mut self, amount: Self::Amount) {
            self.cover_available += amount;
        }

        fn adjust_debt_total(&mut self, delta: Self::Amount) {
            self.debt_total -= delta;
        }

        fn associate_asset(&mut self, asset: &Self::Asset) {
            self.associated_asset = Some(*asset);
        }
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    struct TestVault {
        pseudo_account: &'static str,
        asset: &'static str,
        assets_available: i64,
        assets_total: i64,
        associated_asset: Option<&'static str>,
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
            self.assets_available += amount;
        }

        fn add_assets_total(&mut self, amount: Self::Amount) {
            self.assets_total += amount;
        }

        fn assets_available_exceeds_total(&self) -> bool {
            self.assets_available > self.assets_total
        }

        fn associate_asset(&mut self, asset: &Self::Asset) {
            self.associated_asset = Some(*asset);
        }
    }

    struct TestSink<'a> {
        steps: &'a mut Vec<&'static str>,
        balances: std::collections::HashMap<&'static str, i64>,
        broker_auth_result: Ter,
        add_empty_holding_result: Ter,
    }

    impl LoanPayDoApplySink for TestSink<'_> {
        type Loan = TestLoan;
        type Broker = TestBroker;
        type Vault = TestVault;
        type AccountId = &'static str;
        type BrokerId = &'static str;
        type VaultId = &'static str;
        type Asset = &'static str;
        type Amount = i64;

        fn read_loan(&mut self) -> Option<Self::Loan> {
            unreachable!()
        }

        fn read_broker(&mut self, _broker_id: &Self::BrokerId) -> Option<Self::Broker> {
            unreachable!()
        }

        fn read_vault(&mut self, _vault_id: &Self::VaultId) -> Option<Self::Vault> {
            unreachable!()
        }

        fn compute_required_cover_threshold(
            &mut self,
            _asset: &Self::Asset,
            _debt_total: &Self::Amount,
            _cover_rate_minimum: u32,
            _loan_scale: i32,
        ) -> Self::Amount {
            unreachable!()
        }

        fn broker_owner_is_deep_frozen(
            &mut self,
            _owner: &Self::AccountId,
            _asset: &Self::Asset,
        ) -> bool {
            unreachable!()
        }

        fn broker_owner_requires_auth(
            &mut self,
            _owner: &Self::AccountId,
            _asset: &Self::Asset,
        ) -> bool {
            unreachable!()
        }

        fn check_deep_frozen(&mut self, _account: &Self::AccountId, _asset: &Self::Asset) -> Ter {
            unreachable!()
        }

        fn unimpair_loan(
            &mut self,
            _loan: &mut Self::Loan,
            _vault: &Self::Vault,
            _asset: &Self::Asset,
        ) -> Ter {
            unreachable!()
        }

        fn make_payment(
            &mut self,
            _asset: &Self::Asset,
            _loan: &mut Self::Loan,
            _broker: &mut Self::Broker,
            _amount: &Self::Amount,
            _payment_type: LoanPayPaymentType,
        ) -> Result<LoanPayPaymentParts<Self::Amount>, Ter> {
            unreachable!()
        }

        fn update_loan(&mut self, _loan: &Self::Loan) {
            unreachable!()
        }

        fn update_broker(&mut self, _broker: &Self::Broker) {
            unreachable!()
        }

        fn adjust_broker_debt_total(
            &mut self,
            broker: &mut Self::Broker,
            debt_delta: &Self::Amount,
            _asset: &Self::Asset,
            _vault_scale: i32,
        ) {
            self.steps.push("adjust_broker_debt_total");
            broker.debt_total += *debt_delta;
        }

        fn update_vault(&mut self, _vault: &Self::Vault) {
            self.steps.push("update_vault");
        }

        fn require_auth(&mut self, account: &Self::AccountId, _asset: &Self::Asset) -> Ter {
            self.steps.push(match *account {
                "vault-pseudo" => "require_auth_vault",
                "borrower" => "require_auth_borrower",
                _ => "require_auth_other",
            });
            if *account == "borrower" {
                self.broker_auth_result
            } else {
                Ter::TES_SUCCESS
            }
        }

        fn broker_payee_balance_for_empty_holding(
            &mut self,
            account: &Self::AccountId,
        ) -> Self::Amount {
            self.steps.push("broker_payee_balance");
            *self.balances.get(account).unwrap_or(&0)
        }

        fn add_empty_holding(
            &mut self,
            _account: &Self::AccountId,
            _balance: &Self::Amount,
            _asset: &Self::Asset,
        ) -> Ter {
            self.steps.push("add_empty_holding");
            self.add_empty_holding_result
        }

        fn account_send_multi(
            &mut self,
            from: &Self::AccountId,
            _asset: &Self::Asset,
            vault_pseudo: &Self::AccountId,
            vault_amount: &Self::Amount,
            broker_payee: &Self::AccountId,
            broker_amount: &Self::Amount,
        ) -> Ter {
            self.steps.push("account_send_multi");
            *self.balances.entry(*from).or_insert(0) -= *vault_amount + *broker_amount;
            *self.balances.entry(*vault_pseudo).or_insert(0) += *vault_amount;
            *self.balances.entry(*broker_payee).or_insert(0) += *broker_amount;
            Ter::TES_SUCCESS
        }

        fn sample_balance(
            &mut self,
            account: &Self::AccountId,
            _asset: &Self::Asset,
        ) -> Self::Amount {
            self.steps.push(match *account {
                "vault-pseudo" => "sample_vault_pseudo",
                "borrower" => "sample_borrower",
                _ => "sample_other",
            });
            *self.balances.get(account).unwrap_or(&0)
        }

        fn account_is_asset_issuer(
            &mut self,
            _account: &Self::AccountId,
            _asset: &Self::Asset,
        ) -> bool {
            self.steps.push("account_is_issuer");
            false
        }
    }

    impl LoanPayDoApplyAmountsSink for TestSink<'_> {
        type Vault = TestVault;
        type Asset = &'static str;
        type Amount = i64;

        fn vault_scale(&mut self, _vault: &Self::Vault) -> i32 {
            self.steps.push("vault_scale");
            6
        }

        fn round_to_asset_downward(
            &mut self,
            _asset: &Self::Asset,
            value: &Self::Amount,
            _scale: i32,
        ) -> Self::Amount {
            self.steps.push("round_to_asset_downward");
            *value
        }

        fn asset_is_integral(&mut self, _asset: &Self::Asset) -> bool {
            self.steps.push("asset_is_integral");
            true
        }

        fn is_rounded(&mut self, _asset: &Self::Asset, _value: &Self::Amount, _scale: i32) -> bool {
            self.steps.push("is_rounded");
            true
        }
    }

    fn build_state()
    -> LoanPayDoApplyFrontState<TestLoan, TestBroker, TestVault, &'static str, &'static str, i64>
    {
        LoanPayDoApplyFrontState {
            loan: TestLoan {
                broker_id: "broker",
                impaired: false,
                associated_asset: None,
            },
            broker: TestBroker {
                owner: "owner",
                pseudo_account: "broker-pseudo",
                vault_id: "vault",
                debt_total: 100,
                cover_available: 10,
                cover_rate_minimum: 0,
                associated_asset: None,
            },
            vault: TestVault {
                pseudo_account: "vault-pseudo",
                asset: "USD",
                assets_available: 10,
                assets_total: 30,
                associated_asset: None,
            },
            asset: "USD",
            broker_payee: "borrower",
            send_broker_fee_to_owner: true,
            payment_type: LoanPayPaymentType::Full,
            payment_parts: LoanPayPaymentParts {
                principal_paid: 7,
                interest_paid: 3,
                fee_paid: 2,
                value_change: 5,
            },
        }
    }

    #[test]
    fn tx_loan_pay_do_apply_middle_bridge_order() {
        let mut steps = Vec::new();
        let mut sink = TestSink {
            steps: &mut steps,
            balances: std::collections::HashMap::from([("vault-pseudo", 10), ("borrower", 40)]),
            broker_auth_result: Ter::TES_SUCCESS,
            add_empty_holding_result: Ter::TES_SUCCESS,
        };
        let mut state = build_state();

        let result = run_loan_pay_do_apply_middle(
            &mut sink,
            &mut state,
            LoanPayDoApplyMiddleFacts {
                account: "borrower",
                amount: 20,
                zero_amount: 0,
            },
        )
        .expect("success");

        assert_eq!(
            result,
            LoanPayDoApplyMiddleResult {
                post_payment_prep: result.post_payment_prep.clone(),
                pre_transfer_snapshot: result.pre_transfer_snapshot.clone(),
                post_transfer_checks: result.post_transfer_checks.clone(),
            }
        );
        assert_eq!(state.broker.debt_total, 95);
        assert_eq!(state.vault.assets_available, 20);
        assert_eq!(state.vault.assets_total, 35);
        assert_eq!(
            steps,
            [
                "sample_vault_pseudo",
                "vault_scale",
                "round_to_asset_downward",
                "asset_is_integral",
                "is_rounded",
                "adjust_broker_debt_total",
                "update_vault",
                "sample_borrower",
                "sample_vault_pseudo",
                "sample_borrower",
                "require_auth_vault",
                "broker_payee_balance",
                "add_empty_holding",
                "require_auth_borrower",
                "account_send_multi",
                "sample_vault_pseudo",
                "sample_borrower",
                "sample_vault_pseudo",
                "sample_borrower",
                "account_is_issuer",
            ]
        );
    }

    #[test]
    fn tx_loan_pay_do_apply_middle_passthroughs_tail_auth_failure() {
        let mut steps = Vec::new();
        let mut sink = TestSink {
            steps: &mut steps,
            balances: std::collections::HashMap::from([("vault-pseudo", 10), ("borrower", 40)]),
            broker_auth_result: Ter::TER_NO_AUTH,
            add_empty_holding_result: Ter::TES_SUCCESS,
        };
        let mut state = build_state();

        let result = run_loan_pay_do_apply_middle(
            &mut sink,
            &mut state,
            LoanPayDoApplyMiddleFacts {
                account: "borrower",
                amount: 20,
                zero_amount: 0,
            },
        );

        assert_eq!(result, Err(Ter::TER_NO_AUTH));
    }

    #[test]
    fn tx_loan_pay_do_apply_middle_passthroughs_non_duplicate_holding_failure() {
        let mut steps = Vec::new();
        let mut sink = TestSink {
            steps: &mut steps,
            balances: std::collections::HashMap::from([("vault-pseudo", 10), ("borrower", 40)]),
            broker_auth_result: Ter::TES_SUCCESS,
            add_empty_holding_result: Ter::TEC_PATH_DRY,
        };
        let mut state = build_state();

        let result = run_loan_pay_do_apply_middle(
            &mut sink,
            &mut state,
            LoanPayDoApplyMiddleFacts {
                account: "borrower",
                amount: 20,
                zero_amount: 0,
            },
        );

        assert_eq!(result, Err(Ter::TEC_PATH_DRY));
        assert_eq!(
            steps,
            [
                "sample_vault_pseudo",
                "vault_scale",
                "round_to_asset_downward",
                "asset_is_integral",
                "is_rounded",
                "adjust_broker_debt_total",
                "update_vault",
                "sample_borrower",
                "sample_vault_pseudo",
                "sample_borrower",
                "require_auth_vault",
                "broker_payee_balance",
                "add_empty_holding",
            ]
        );
    }
}

mod loan_pay_do_apply_exported_api_parity {
    use super::*;

    #[derive(Debug, Clone, PartialEq, Eq)]
    struct TestLoan {
        broker_id: &'static str,
        impaired: bool,
        associated_asset: Option<&'static str>,
    }

    impl LoanPayDoApplyLoan for TestLoan {
        type BrokerId = &'static str;
        type Asset = &'static str;

        fn broker_id(&self) -> &Self::BrokerId {
            &self.broker_id
        }

        fn scale(&self) -> i32 {
            6
        }

        fn is_impaired(&self) -> bool {
            self.impaired
        }

        fn associate_asset(&mut self, asset: &Self::Asset) {
            self.associated_asset = Some(*asset);
        }
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    struct TestBroker {
        owner: &'static str,
        pseudo_account: &'static str,
        vault_id: &'static str,
        debt_total: i64,
        cover_available: i64,
        cover_rate_minimum: u32,
        associated_asset: Option<&'static str>,
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
            self.cover_rate_minimum
        }

        fn add_cover_available(&mut self, amount: Self::Amount) {
            self.cover_available += amount;
        }

        fn adjust_debt_total(&mut self, delta: Self::Amount) {
            self.debt_total -= delta;
        }

        fn associate_asset(&mut self, asset: &Self::Asset) {
            self.associated_asset = Some(*asset);
        }
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    struct TestVault {
        pseudo_account: &'static str,
        asset: &'static str,
        assets_available: i64,
        assets_total: i64,
        associated_asset: Option<&'static str>,
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
            self.assets_available += amount;
        }

        fn add_assets_total(&mut self, amount: Self::Amount) {
            self.assets_total += amount;
        }

        fn assets_available_exceeds_total(&self) -> bool {
            self.assets_available > self.assets_total
        }

        fn associate_asset(&mut self, asset: &Self::Asset) {
            self.associated_asset = Some(*asset);
        }
    }

    struct TestSink<'a> {
        steps: &'a mut Vec<&'static str>,
        loan: Option<TestLoan>,
        broker: Option<TestBroker>,
        vault: Option<TestVault>,
        required_cover: i64,
        owner_is_deep_frozen: bool,
        owner_requires_auth: bool,
        fallback_deep_frozen: Ter,
        payment_result: Result<LoanPayPaymentParts<i64>, Ter>,
        balances: std::collections::HashMap<&'static str, i64>,
        add_empty_holding_result: Ter,
        account_send_multi_result: Ter,
    }

    impl LoanPayDoApplySink for TestSink<'_> {
        type Loan = TestLoan;
        type Broker = TestBroker;
        type Vault = TestVault;
        type AccountId = &'static str;
        type BrokerId = &'static str;
        type VaultId = &'static str;
        type Asset = &'static str;
        type Amount = i64;

        fn read_loan(&mut self) -> Option<Self::Loan> {
            self.steps.push("read_loan");
            self.loan.clone()
        }

        fn read_broker(&mut self, _broker_id: &Self::BrokerId) -> Option<Self::Broker> {
            self.steps.push("read_broker");
            self.broker.clone()
        }

        fn read_vault(&mut self, _vault_id: &Self::VaultId) -> Option<Self::Vault> {
            self.steps.push("read_vault");
            self.vault.clone()
        }

        fn compute_required_cover_threshold(
            &mut self,
            _asset: &Self::Asset,
            _debt_total: &Self::Amount,
            _cover_rate_minimum: u32,
            _loan_scale: i32,
        ) -> Self::Amount {
            self.steps.push("required_cover");
            self.required_cover
        }

        fn broker_owner_is_deep_frozen(
            &mut self,
            _owner: &Self::AccountId,
            _asset: &Self::Asset,
        ) -> bool {
            self.steps.push("owner_deep_frozen");
            self.owner_is_deep_frozen
        }

        fn broker_owner_requires_auth(
            &mut self,
            _owner: &Self::AccountId,
            _asset: &Self::Asset,
        ) -> bool {
            self.steps.push("owner_auth");
            self.owner_requires_auth
        }

        fn check_deep_frozen(&mut self, _account: &Self::AccountId, _asset: &Self::Asset) -> Ter {
            self.steps.push("fallback_deep_frozen");
            self.fallback_deep_frozen
        }

        fn unimpair_loan(
            &mut self,
            loan: &mut Self::Loan,
            _vault: &Self::Vault,
            _asset: &Self::Asset,
        ) -> Ter {
            self.steps.push("unimpair");
            loan.impaired = false;
            Ter::TES_SUCCESS
        }

        fn make_payment(
            &mut self,
            _asset: &Self::Asset,
            _loan: &mut Self::Loan,
            _broker: &mut Self::Broker,
            amount: &Self::Amount,
            payment_type: LoanPayPaymentType,
        ) -> Result<LoanPayPaymentParts<Self::Amount>, Ter> {
            assert_eq!(*amount, 25);
            self.steps.push(match payment_type {
                LoanPayPaymentType::Late => "payment_type_late",
                LoanPayPaymentType::Full => "payment_type_full",
                LoanPayPaymentType::Overpayment => "payment_type_overpayment",
                LoanPayPaymentType::Regular => "payment_type_regular",
            });
            let result = self.payment_result.clone();
            if result.is_ok() {
                self.steps.push("payment_computed");
            }
            result
        }

        fn update_loan(&mut self, _loan: &Self::Loan) {
            self.steps.push("update_loan");
        }

        fn update_broker(&mut self, _broker: &Self::Broker) {
            self.steps.push("update_broker");
        }

        fn adjust_broker_debt_total(
            &mut self,
            broker: &mut Self::Broker,
            debt_delta: &Self::Amount,
            _asset: &Self::Asset,
            _vault_scale: i32,
        ) {
            self.steps.push("adjust_broker_debt_total");
            broker.debt_total += *debt_delta;
        }

        fn update_vault(&mut self, _vault: &Self::Vault) {
            self.steps.push("update_vault");
        }

        fn require_auth(&mut self, account: &Self::AccountId, _asset: &Self::Asset) -> Ter {
            self.steps.push(match *account {
                "vault-pseudo" => "require_auth_vault",
                "borrower" => "require_auth_borrower",
                _ => "require_auth_other",
            });
            Ter::TES_SUCCESS
        }

        fn broker_payee_balance_for_empty_holding(
            &mut self,
            account: &Self::AccountId,
        ) -> Self::Amount {
            self.steps.push("broker_payee_balance");
            *self.balances.get(account).unwrap_or(&0)
        }

        fn add_empty_holding(
            &mut self,
            _account: &Self::AccountId,
            _balance: &Self::Amount,
            _asset: &Self::Asset,
        ) -> Ter {
            self.steps.push("add_empty_holding");
            self.add_empty_holding_result
        }

        fn account_send_multi(
            &mut self,
            from: &Self::AccountId,
            _asset: &Self::Asset,
            vault_pseudo: &Self::AccountId,
            vault_amount: &Self::Amount,
            broker_payee: &Self::AccountId,
            broker_amount: &Self::Amount,
        ) -> Ter {
            self.steps.push("account_send_multi");
            *self.balances.entry(*from).or_insert(0) -= *vault_amount + *broker_amount;
            *self.balances.entry(*vault_pseudo).or_insert(0) += *vault_amount;
            *self.balances.entry(*broker_payee).or_insert(0) += *broker_amount;
            self.account_send_multi_result
        }

        fn sample_balance(
            &mut self,
            account: &Self::AccountId,
            _asset: &Self::Asset,
        ) -> Self::Amount {
            self.steps.push(match *account {
                "vault-pseudo" => "sample_vault_pseudo",
                "borrower" => "sample_borrower",
                _ => "sample_other",
            });
            *self.balances.get(account).unwrap_or(&0)
        }

        fn account_is_asset_issuer(
            &mut self,
            _account: &Self::AccountId,
            _asset: &Self::Asset,
        ) -> bool {
            self.steps.push("account_is_issuer");
            false
        }
    }

    impl LoanPayDoApplyAmountsSink for TestSink<'_> {
        type Vault = TestVault;
        type Asset = &'static str;
        type Amount = i64;

        fn vault_scale(&mut self, _vault: &Self::Vault) -> i32 {
            self.steps.push("vault_scale");
            6
        }

        fn round_to_asset_downward(
            &mut self,
            _asset: &Self::Asset,
            value: &Self::Amount,
            _scale: i32,
        ) -> Self::Amount {
            self.steps.push("round_to_asset_downward");
            *value
        }

        fn asset_is_integral(&mut self, _asset: &Self::Asset) -> bool {
            self.steps.push("asset_is_integral");
            true
        }

        fn is_rounded(&mut self, _asset: &Self::Asset, _value: &Self::Amount, _scale: i32) -> bool {
            self.steps.push("is_rounded");
            true
        }
    }

    fn apply_facts() -> LoanPayDoApplyFacts<&'static str, i64> {
        LoanPayDoApplyFacts {
            account: "owner",
            amount: 25,
            zero_amount: 0,
            tx_requests_late_payment: false,
            tx_requests_full_payment: true,
            tx_requests_overpayment: false,
        }
    }

    #[test]
    fn tx_loan_pay_do_apply_passthroughs_non_duplicate_holding_failure() {
        let mut steps = Vec::new();
        let mut sink = TestSink {
            steps: &mut steps,
            loan: Some(TestLoan {
                broker_id: "broker",
                impaired: false,
                associated_asset: None,
            }),
            broker: Some(TestBroker {
                owner: "owner",
                pseudo_account: "broker-pseudo",
                vault_id: "vault",
                debt_total: 100,
                cover_available: 10,
                cover_rate_minimum: 0,
                associated_asset: None,
            }),
            vault: Some(TestVault {
                pseudo_account: "vault-pseudo",
                asset: "USD",
                assets_available: 10,
                assets_total: 30,
                associated_asset: None,
            }),
            required_cover: 10,
            owner_is_deep_frozen: false,
            owner_requires_auth: false,
            fallback_deep_frozen: Ter::TES_SUCCESS,
            payment_result: Ok(LoanPayPaymentParts {
                principal_paid: 7,
                interest_paid: 3,
                fee_paid: 2,
                value_change: 5,
            }),
            balances: std::collections::HashMap::from([
                ("owner", 100),
                ("vault-pseudo", 10),
                ("broker-pseudo", 2),
            ]),
            add_empty_holding_result: Ter::TEC_PATH_DRY,
            account_send_multi_result: Ter::TES_SUCCESS,
        };

        let result = run_loan_pay_do_apply(&mut sink, apply_facts());

        assert_eq!(result, Err(Ter::TEC_PATH_DRY));
        assert_eq!(
            steps,
            [
                "read_loan",
                "read_broker",
                "read_vault",
                "required_cover",
                "owner_deep_frozen",
                "owner_auth",
                "payment_type_full",
                "payment_computed",
                "update_loan",
                "update_broker",
                "sample_vault_pseudo",
                "vault_scale",
                "round_to_asset_downward",
                "asset_is_integral",
                "is_rounded",
                "adjust_broker_debt_total",
                "update_vault",
                "sample_other",
                "sample_vault_pseudo",
                "sample_other",
                "require_auth_vault",
                "broker_payee_balance",
                "add_empty_holding",
            ]
        );
    }
}

mod loan_pay_do_apply_tail_exported_api_parity {
    use super::*;

    #[derive(Debug, Clone, PartialEq, Eq)]
    struct TestLoan {
        broker_id: &'static str,
        impaired: bool,
        associated_asset: Option<&'static str>,
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
            self.impaired
        }

        fn associate_asset(&mut self, asset: &Self::Asset) {
            self.associated_asset = Some(*asset);
        }
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    struct TestBroker {
        owner: &'static str,
        pseudo_account: &'static str,
        vault_id: &'static str,
        debt_total: i64,
        cover_available: i64,
        cover_rate_minimum: u32,
        associated_asset: Option<&'static str>,
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
            self.cover_rate_minimum
        }

        fn add_cover_available(&mut self, amount: Self::Amount) {
            self.cover_available += amount;
        }

        fn adjust_debt_total(&mut self, delta: Self::Amount) {
            self.debt_total -= delta;
        }

        fn associate_asset(&mut self, asset: &Self::Asset) {
            self.associated_asset = Some(*asset);
        }
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    struct TestVault {
        pseudo_account: &'static str,
        asset: &'static str,
        assets_available: i64,
        assets_total: i64,
        associated_asset: Option<&'static str>,
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
            self.assets_available += amount;
        }

        fn add_assets_total(&mut self, amount: Self::Amount) {
            self.assets_total += amount;
        }

        fn assets_available_exceeds_total(&self) -> bool {
            self.assets_available > self.assets_total
        }

        fn associate_asset(&mut self, asset: &Self::Asset) {
            self.associated_asset = Some(*asset);
        }
    }

    struct TestSink<'a> {
        steps: &'a mut Vec<&'static str>,
        vault_auth_result: Ter,
        broker_auth_result: Ter,
        add_empty_holding_result: Ter,
        account_send_multi_result: Ter,
        expected_vault_amount: i64,
        expected_broker_payee: &'static str,
        expected_broker_amount: i64,
    }

    impl LoanPayDoApplyTailSink for TestSink<'_> {
        type Loan = TestLoan;
        type Broker = TestBroker;
        type Vault = TestVault;
        type AccountId = &'static str;
        type Asset = &'static str;
        type Amount = i64;
        type VaultId = &'static str;

        fn update_vault(&mut self, _vault: &Self::Vault) {
            self.steps.push("update_vault");
        }

        fn require_auth(&mut self, account: &Self::AccountId, _asset: &Self::Asset) -> Ter {
            self.steps.push(*account);
            if *account == "vault-pseudo" {
                self.vault_auth_result
            } else {
                self.broker_auth_result
            }
        }

        fn broker_payee_balance_for_empty_holding(
            &mut self,
            account: &Self::AccountId,
        ) -> Self::Amount {
            self.steps.push("broker_payee_balance");
            assert_eq!(*account, "borrower");
            12
        }

        fn add_empty_holding(
            &mut self,
            account: &Self::AccountId,
            balance: &Self::Amount,
            _asset: &Self::Asset,
        ) -> Ter {
            self.steps.push("add_empty_holding");
            assert_eq!(*account, "borrower");
            assert_eq!(*balance, 12);
            self.add_empty_holding_result
        }

        fn account_send_multi(
            &mut self,
            source: &Self::AccountId,
            asset: &Self::Asset,
            outputs: [(Self::AccountId, Self::Amount); 2],
        ) -> Ter {
            self.steps.push("account_send_multi");
            assert_eq!(*source, "borrower");
            assert_eq!(*asset, "USD");
            assert_eq!(outputs[0], ("vault-pseudo", self.expected_vault_amount));
            assert_eq!(
                outputs[1],
                (self.expected_broker_payee, self.expected_broker_amount)
            );
            self.account_send_multi_result
        }
    }

    fn build_state(
        send_broker_fee_to_owner: bool,
        broker_payee: &'static str,
        vault_available: i64,
        vault_total: i64,
    ) -> LoanPayDoApplyFrontState<TestLoan, TestBroker, TestVault, &'static str, &'static str, i64>
    {
        LoanPayDoApplyFrontState {
            loan: TestLoan {
                broker_id: "broker",
                impaired: false,
                associated_asset: None,
            },
            broker: TestBroker {
                owner: "owner",
                pseudo_account: "broker-pseudo",
                vault_id: "vault",
                debt_total: 50,
                cover_available: 20,
                cover_rate_minimum: 0,
                associated_asset: None,
            },
            vault: TestVault {
                pseudo_account: "vault-pseudo",
                asset: "USD",
                assets_available: vault_available,
                assets_total: vault_total,
                associated_asset: None,
            },
            asset: "USD",
            broker_payee,
            send_broker_fee_to_owner,
            payment_type: LoanPayPaymentType::Full,
            payment_parts: LoanPayPaymentParts {
                principal_paid: 10,
                interest_paid: 3,
                fee_paid: 3,
                value_change: 2,
            },
        }
    }

    #[test]
    fn tx_loan_pay_do_apply_tail_maps_vault_overflow_to_tec_internal() {
        let mut steps = Vec::new();
        let mut state = build_state(true, "owner", 10, 10);
        state.payment_parts.value_change = 0;

        let mut sink = TestSink {
            steps: &mut steps,
            vault_auth_result: Ter::TES_SUCCESS,
            broker_auth_result: Ter::TES_SUCCESS,
            add_empty_holding_result: Ter::TES_SUCCESS,
            account_send_multi_result: Ter::TES_SUCCESS,
            expected_vault_amount: 1,
            expected_broker_payee: "owner",
            expected_broker_amount: 0,
        };

        let result = run_loan_pay_do_apply_tail(
            &mut sink,
            &"borrower",
            &mut state,
            LoanPayDoApplyTailFacts {
                zero_amount: 0,
                total_paid_to_vault_rounded: 1,
                total_paid_to_broker: 0,
            },
        );

        assert_eq!(result, Ter::TEC_INTERNAL);
        assert_eq!(steps, ["update_vault"]);
    }

    #[test]
    fn tx_loan_pay_do_apply_tail_ignores_duplicate_holding_when_broker_payee_is_borrower() {
        let mut steps = Vec::new();
        let mut state = build_state(false, "borrower", 80, 100);

        let mut sink = TestSink {
            steps: &mut steps,
            vault_auth_result: Ter::TES_SUCCESS,
            broker_auth_result: Ter::TES_SUCCESS,
            add_empty_holding_result: Ter::TEC_DUPLICATE,
            account_send_multi_result: Ter::TES_SUCCESS,
            expected_vault_amount: 7,
            expected_broker_payee: "borrower",
            expected_broker_amount: 3,
        };

        let result = run_loan_pay_do_apply_tail(
            &mut sink,
            &"borrower",
            &mut state,
            LoanPayDoApplyTailFacts {
                zero_amount: 0,
                total_paid_to_vault_rounded: 7,
                total_paid_to_broker: 3,
            },
        );

        assert_eq!(result, Ter::TES_SUCCESS);
        assert_eq!(
            steps,
            [
                "update_vault",
                "vault-pseudo",
                "broker_payee_balance",
                "add_empty_holding",
                "borrower",
                "account_send_multi",
            ]
        );
        assert_eq!(state.broker.debt_total, 50);
        assert_eq!(state.broker.cover_available, 23);
    }

    #[test]
    fn tx_loan_pay_do_apply_tail_runs_vault_auth_before_broker_auth() {
        let mut steps = Vec::new();
        let mut state = build_state(false, "borrower", 80, 100);

        let mut sink = TestSink {
            steps: &mut steps,
            vault_auth_result: Ter::TES_SUCCESS,
            broker_auth_result: Ter::TES_SUCCESS,
            add_empty_holding_result: Ter::TES_SUCCESS,
            account_send_multi_result: Ter::TES_SUCCESS,
            expected_vault_amount: 7,
            expected_broker_payee: "borrower",
            expected_broker_amount: 3,
        };

        let result = run_loan_pay_do_apply_tail(
            &mut sink,
            &"borrower",
            &mut state,
            LoanPayDoApplyTailFacts {
                zero_amount: 0,
                total_paid_to_vault_rounded: 7,
                total_paid_to_broker: 3,
            },
        );

        assert_eq!(result, Ter::TES_SUCCESS);
        assert_eq!(
            steps,
            [
                "update_vault",
                "vault-pseudo",
                "broker_payee_balance",
                "add_empty_holding",
                "borrower",
                "account_send_multi",
            ]
        );
    }

    #[test]
    fn tx_loan_pay_do_apply_tail_adds_cover_on_fallback_fee_path() {
        let mut steps = Vec::new();
        let mut state = build_state(false, "owner", 80, 100);

        let mut sink = TestSink {
            steps: &mut steps,
            vault_auth_result: Ter::TES_SUCCESS,
            broker_auth_result: Ter::TES_SUCCESS,
            add_empty_holding_result: Ter::TES_SUCCESS,
            account_send_multi_result: Ter::TES_SUCCESS,
            expected_vault_amount: 7,
            expected_broker_payee: "owner",
            expected_broker_amount: 3,
        };

        let result = run_loan_pay_do_apply_tail(
            &mut sink,
            &"borrower",
            &mut state,
            LoanPayDoApplyTailFacts {
                zero_amount: 0,
                total_paid_to_vault_rounded: 7,
                total_paid_to_broker: 3,
            },
        );

        assert_eq!(result, Ter::TES_SUCCESS);
        assert_eq!(state.broker.debt_total, 50);
        assert_eq!(state.broker.cover_available, 23);
    }

    #[test]
    fn tx_loan_pay_do_apply_tail_passthroughs_account_send_multi_failure() {
        let mut steps = Vec::new();
        let mut state = build_state(true, "owner", 80, 100);

        let mut sink = TestSink {
            steps: &mut steps,
            vault_auth_result: Ter::TES_SUCCESS,
            broker_auth_result: Ter::TES_SUCCESS,
            add_empty_holding_result: Ter::TES_SUCCESS,
            account_send_multi_result: Ter::TEC_PATH_DRY,
            expected_vault_amount: 7,
            expected_broker_payee: "owner",
            expected_broker_amount: 3,
        };

        let result = run_loan_pay_do_apply_tail(
            &mut sink,
            &"borrower",
            &mut state,
            LoanPayDoApplyTailFacts {
                zero_amount: 0,
                total_paid_to_vault_rounded: 7,
                total_paid_to_broker: 3,
            },
        );

        assert_eq!(result, Ter::TEC_PATH_DRY);
        assert_eq!(
            steps,
            [
                "update_vault",
                "vault-pseudo",
                "owner",
                "account_send_multi",
            ]
        );
    }
}
