//! Integration tests that pin the pure Rust `LoanManage::defaultLoan(...)`
//! helper slice to the current C++ formula and guard ordering.

use std::{cell::RefCell, rc::Rc};

use tx::loan_manage_default::{
    LoanManageDefaultError, LoanManageDefaultFacts, LoanManageDefaultMath,
    LoanManageDefaultRoundingMode, run_loan_manage_default,
};

#[derive(Clone)]
struct TraceMath {
    calls: Rc<RefCell<Vec<String>>>,
}

impl LoanManageDefaultMath for TraceMath {
    type Amount = i64;
    type Asset = &'static str;

    fn tenth_bips_of_value(&mut self, value: Self::Amount, rate: u32) -> Self::Amount {
        self.calls
            .borrow_mut()
            .push(format!("tenth_bips_of_value:{value}:{rate}"));
        value + rate as i64
    }

    fn round_to_asset(
        &mut self,
        asset: &Self::Asset,
        value: Self::Amount,
        scale: i32,
        mode: LoanManageDefaultRoundingMode,
    ) -> Self::Amount {
        self.calls.borrow_mut().push(format!(
            "round_to_asset:{}:{value}:{scale}:{mode:?}",
            *asset
        ));
        value
            + scale as i64
            + match mode {
                LoanManageDefaultRoundingMode::Upward => 1,
                LoanManageDefaultRoundingMode::Downward => -1,
            }
    }

    fn asset_is_integral(&mut self, asset: &Self::Asset) -> bool {
        self.calls
            .borrow_mut()
            .push(format!("asset_is_integral:{}", *asset));
        false
    }

    fn exponent(&mut self, value: Self::Amount) -> i32 {
        self.calls.borrow_mut().push(format!("exponent:{value}"));
        value as i32
    }

    fn adjust_imprecise_subtract(
        &mut self,
        asset: &Self::Asset,
        value: Self::Amount,
        decrement: Self::Amount,
        scale: i32,
    ) -> Self::Amount {
        self.calls.borrow_mut().push(format!(
            "adjust_imprecise_subtract:{}:{value}:{decrement}:{scale}",
            *asset
        ));
        value - decrement + scale as i64
    }
}

fn base_facts() -> LoanManageDefaultFacts<i64, &'static str> {
    LoanManageDefaultFacts {
        asset: "USD",
        loan_scale: 2,
        vault_scale: 5,
        total_value_outstanding: 125,
        management_fee_outstanding: 25,
        broker_debt_total: 200,
        cover_rate_minimum: 10,
        cover_rate_liquidation: 50,
        cover_available: 7,
        vault_total_assets: 1_000,
        vault_available_assets: 100,
        vault_loss_unrealized: 150,
        loan_is_impaired: true,
    }
}

#[test]
fn loan_manage_default_pins_total_default_and_coverage_order() {
    let calls = Rc::new(RefCell::new(Vec::new()));
    let mut math = TraceMath {
        calls: Rc::clone(&calls),
    };

    let plan = run_loan_manage_default(base_facts(), &mut math).unwrap();

    assert_eq!(plan.total_default_amount, 100);
    assert_eq!(plan.minimum_cover, 210);
    assert_eq!(plan.liquidation_cover, 260);
    assert_eq!(plan.liquidation_cover_capped, 100);
    assert_eq!(plan.covered_before_cover_available, 103);
    assert_eq!(plan.default_covered, 7);
    assert_eq!(plan.vault_default_amount, 93);
    assert_eq!(plan.vault_default_rounded, 97);
    assert_eq!(plan.cover_available_after, 0);
    assert_eq!(plan.broker_debt_after, 105);
    assert_eq!(plan.vault_total_after, 903);
    assert_eq!(plan.vault_available_after, 107);
    assert!(!plan.dust_reconciled);
    assert_eq!(plan.vault_loss_unrealized_after, Some(55));

    assert_eq!(
        *calls.borrow(),
        vec![
            "tenth_bips_of_value:200:10",
            "tenth_bips_of_value:210:50",
            "round_to_asset:USD:100:2:Upward",
            "round_to_asset:USD:93:5:Downward",
            "adjust_imprecise_subtract:USD:150:100:5",
            "adjust_imprecise_subtract:USD:200:100:5",
        ]
    );
}

#[test]
fn loan_manage_default_caps_default_covered_by_cover_available() {
    let mut facts = base_facts();
    facts.cover_available = 3;

    let mut math = TraceMath {
        calls: Rc::new(RefCell::new(Vec::new())),
    };

    let plan = run_loan_manage_default(facts, &mut math).unwrap();

    assert_eq!(plan.covered_before_cover_available, 103);
    assert_eq!(plan.default_covered, 3);
    assert_eq!(plan.vault_default_amount, 97);
    assert_eq!(plan.cover_available_after, 0);
}

#[test]
fn loan_manage_default_reconciles_non_integral_dust() {
    let mut facts = base_facts();
    facts.vault_total_assets = 1000;
    facts.vault_available_assets = 1000;
    facts.cover_available = 0;

    #[derive(Clone)]
    struct DustMath;

    impl LoanManageDefaultMath for DustMath {
        type Amount = i64;
        type Asset = &'static str;

        fn tenth_bips_of_value(&mut self, value: Self::Amount, _rate: u32) -> Self::Amount {
            value
        }

        fn round_to_asset(
            &mut self,
            _asset: &Self::Asset,
            value: Self::Amount,
            _scale: i32,
            _mode: LoanManageDefaultRoundingMode,
        ) -> Self::Amount {
            value
        }

        fn asset_is_integral(&mut self, _asset: &Self::Asset) -> bool {
            false
        }

        fn exponent(&mut self, value: Self::Amount) -> i32 {
            match value {
                1000 => 20,
                900 => 20,
                100 => 3,
                _ => 0,
            }
        }

        fn adjust_imprecise_subtract(
            &mut self,
            _asset: &Self::Asset,
            value: Self::Amount,
            decrement: Self::Amount,
            _scale: i32,
        ) -> Self::Amount {
            value - decrement
        }
    }

    let mut math = DustMath;
    let plan = run_loan_manage_default(facts, &mut math).expect("dust should reconcile");

    assert!(plan.dust_reconciled);
    assert_eq!(plan.vault_total_after, plan.vault_available_after);
}

#[test]
fn loan_manage_default_rejects_vault_shortfall_before_later_guards() {
    let mut facts = base_facts();
    facts.vault_total_assets = 92;

    let calls = Rc::new(RefCell::new(Vec::new()));
    let mut math = TraceMath {
        calls: Rc::clone(&calls),
    };

    let result = run_loan_manage_default(facts, &mut math);

    assert_eq!(result, Err(LoanManageDefaultError::VaultTotalShortfall));
    assert_eq!(
        *calls.borrow(),
        vec![
            "tenth_bips_of_value:200:10",
            "tenth_bips_of_value:210:50",
            "round_to_asset:USD:100:2:Upward",
        ]
    );
}

#[test]
fn loan_manage_default_rejects_impaired_loss_shortfall_after_vault_checks() {
    let mut facts = base_facts();
    facts.vault_loss_unrealized = 99;

    let mut math = TraceMath {
        calls: Rc::new(RefCell::new(Vec::new())),
    };

    let result = run_loan_manage_default(facts, &mut math);

    assert_eq!(
        result,
        Err(LoanManageDefaultError::VaultUnrealizedLossShortfall)
    );
}

#[test]
fn loan_manage_default_rejects_vault_asset_total_inconsistency() {
    let mut facts = base_facts();
    facts.vault_available_assets = 950;
    facts.vault_total_assets = 940;

    #[derive(Clone)]
    struct NonDustMath;

    impl LoanManageDefaultMath for NonDustMath {
        type Amount = i64;
        type Asset = &'static str;

        fn tenth_bips_of_value(&mut self, value: Self::Amount, rate: u32) -> Self::Amount {
            value + rate as i64
        }

        fn round_to_asset(
            &mut self,
            _asset: &Self::Asset,
            value: Self::Amount,
            scale: i32,
            mode: LoanManageDefaultRoundingMode,
        ) -> Self::Amount {
            value
                + scale as i64
                + match mode {
                    LoanManageDefaultRoundingMode::Upward => 1,
                    LoanManageDefaultRoundingMode::Downward => -1,
                }
        }

        fn asset_is_integral(&mut self, _asset: &Self::Asset) -> bool {
            false
        }

        fn exponent(&mut self, _value: Self::Amount) -> i32 {
            0
        }

        fn adjust_imprecise_subtract(
            &mut self,
            _asset: &Self::Asset,
            value: Self::Amount,
            decrement: Self::Amount,
            scale: i32,
        ) -> Self::Amount {
            value - decrement + scale as i64
        }
    }

    let mut math = NonDustMath;

    let result = run_loan_manage_default(facts, &mut math);

    assert_eq!(
        result,
        Err(LoanManageDefaultError::VaultAvailableExceedsTotal)
    );
}
