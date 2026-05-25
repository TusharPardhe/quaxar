//! Integration tests that pin the narrowed Rust owner-account guard slice for
//! `PaymentChannelFund.cpp` to the current C++ behavior.

use std::{cell::Cell, rc::Rc};

use protocol::{Ter, trans_token};

use tx::payment_channel_fund_owner::{
    PaymentChannelFundOwnerGuardFacts, load_payment_channel_fund_owner_guard_facts,
};

#[derive(Debug, Clone, PartialEq, Eq)]
struct StubOwner {
    balance_drops: u64,
    reserve_drops: u64,
}

#[test]
fn payment_channel_fund_owner_returns_tefinternal_when_owner_is_missing() {
    let reserve_called = Cell::new(false);
    let funds_called = Cell::new(false);

    let result = load_payment_channel_fund_owner_guard_facts(
        || None::<StubOwner>,
        |_| {
            reserve_called.set(true);
            true
        },
        |_| {
            funds_called.set(true);
            true
        },
    );

    assert_eq!(result, Err(Ter::TEF_INTERNAL));
    assert_eq!(trans_token(result.unwrap_err()), "tefINTERNAL");
    assert!(!reserve_called.get());
    assert!(!funds_called.get());
}

#[test]
fn payment_channel_fund_owner_checks_reserve_before_funds() {
    let seen = Rc::new(std::cell::RefCell::new(Vec::new()));

    let result = load_payment_channel_fund_owner_guard_facts(
        || {
            Some(StubOwner {
                balance_drops: 100,
                reserve_drops: 40,
            })
        },
        {
            let seen = Rc::clone(&seen);
            move |owner: &StubOwner| {
                seen.borrow_mut().push("reserve");
                assert_eq!(owner.balance_drops, 100);
                assert_eq!(owner.reserve_drops, 40);
                false
            }
        },
        {
            let seen = Rc::clone(&seen);
            move |_| {
                seen.borrow_mut().push("funds");
                true
            }
        },
    );

    assert_eq!(result, Err(Ter::TEC_INSUFFICIENT_RESERVE));
    assert_eq!(trans_token(result.unwrap_err()), "tecINSUFFICIENT_RESERVE");
    assert_eq!(seen.borrow().as_slice(), ["reserve"]);
}

#[test]
fn payment_channel_fund_owner_maps_funds_shortfall_after_reserve() {
    let seen = Rc::new(std::cell::RefCell::new(Vec::new()));

    let result = load_payment_channel_fund_owner_guard_facts(
        || {
            Some(StubOwner {
                balance_drops: 100,
                reserve_drops: 40,
            })
        },
        {
            let seen = Rc::clone(&seen);
            move |owner: &StubOwner| {
                seen.borrow_mut().push("reserve");
                assert_eq!(owner.balance_drops, 100);
                true
            }
        },
        {
            let seen = Rc::clone(&seen);
            move |owner: &StubOwner| {
                seen.borrow_mut().push("funds");
                assert_eq!(owner.reserve_drops, 40);
                false
            }
        },
    );

    assert_eq!(result, Err(Ter::TEC_UNFUNDED));
    assert_eq!(trans_token(result.unwrap_err()), "tecUNFUNDED");
    assert_eq!(seen.borrow().as_slice(), ["reserve", "funds"]);
}

#[test]
fn payment_channel_fund_owner_returns_loaded_owner_on_success() {
    let result = load_payment_channel_fund_owner_guard_facts(
        || {
            Some(StubOwner {
                balance_drops: 300,
                reserve_drops: 120,
            })
        },
        |owner| owner.balance_drops >= owner.reserve_drops,
        |owner| owner.balance_drops >= owner.reserve_drops + 100,
    );

    assert_eq!(
        result,
        Ok(PaymentChannelFundOwnerGuardFacts {
            owner: StubOwner {
                balance_drops: 300,
                reserve_drops: 120,
            },
        })
    );
}
