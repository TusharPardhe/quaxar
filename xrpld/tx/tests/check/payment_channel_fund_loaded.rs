//! Integration tests that pin loaded `PaymentChannelFund.cpp` fact shaping to
//! the current C++ behavior.

use tx::payment_channel_fund_loaded::{
    PaymentChannelFundLoadedChannelFacts, PaymentChannelFundLoadedTxFacts,
    build_payment_channel_fund_loaded_apply_facts,
};

#[test]
fn loaded_fund_facts_shape_due_and_extension_floor() {
    let facts = build_payment_channel_fund_loaded_apply_facts(
        PaymentChannelFundLoadedChannelFacts {
            source_account: 7_u32,
            cancel_after: Some(40_u32),
            current_expiration: Some(55_u32),
            settle_delay: 30,
            channel_amount_drops: 1_000,
            channel_balance_drops: 250,
            destination_owner_directory_present: true,
        },
        PaymentChannelFundLoadedTxFacts {
            tx_account: 7_u32,
            extend_expiration: Some(70_u32),
            fund_amount_drops: 300,
        },
        50_u32,
    );

    assert!(facts.tx_account_is_owner);
    assert_eq!(facts.due_facts.cancel_after, Some(40));
    assert_eq!(facts.due_facts.expiration, Some(55));
    assert_eq!(facts.due_facts.close_time, 50);
    assert_eq!(facts.min_extend_expiration, 55);
    assert_eq!(facts.channel_amount_drops, 1_000);
    assert_eq!(facts.fund_amount_drops, 300);
}

#[test]
fn loaded_fund_facts_preserve_close_and_owner_shape() {
    let facts = build_payment_channel_fund_loaded_apply_facts(
        PaymentChannelFundLoadedChannelFacts {
            source_account: 7_u32,
            cancel_after: None,
            current_expiration: None,
            settle_delay: 30,
            channel_amount_drops: 1_000,
            channel_balance_drops: 250,
            destination_owner_directory_present: false,
        },
        PaymentChannelFundLoadedTxFacts {
            tx_account: 8_u32,
            extend_expiration: None,
            fund_amount_drops: 300,
        },
        50_u32,
    );

    assert!(!facts.tx_account_is_owner);
    assert_eq!(facts.min_extend_expiration, 80);
    assert!(!facts.close_facts.destination_owner_directory_present);
    assert_eq!(facts.close_facts.channel_amount_drops, 1_000);
    assert_eq!(facts.close_facts.channel_balance_drops, 250);
}
