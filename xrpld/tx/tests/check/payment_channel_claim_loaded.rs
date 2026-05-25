//! Integration tests that pin loaded `PaymentChannelClaim.cpp` fact shaping to
//! the current C++ behavior.

use tx::payment_channel_claim_loaded::{
    PaymentChannelClaimLoadedChannelFacts, PaymentChannelClaimLoadedTxFacts,
    build_payment_channel_claim_apply_facts,
};

#[test]
fn loaded_claim_facts_shape_due_and_account_flags() {
    let facts = build_payment_channel_claim_apply_facts(
        PaymentChannelClaimLoadedChannelFacts {
            source_account: 1_u32,
            destination_account: 2_u32,
            cancel_after: Some(50_u32),
            current_expiration: Some(75_u32),
            settle_delay: 30,
            channel_amount_drops: 1_000,
            channel_balance_drops: 250,
            destination_owner_directory_present: true,
        },
        PaymentChannelClaimLoadedTxFacts {
            tx_account: 2_u32,
            balance_present: true,
            signature_present: false,
            provided_public_key_matches_channel: true,
            requested_balance_drops: 400,
            renew_flag: false,
            close_flag: true,
        },
        60_u32,
    );

    assert!(!facts.tx_account_is_source);
    assert!(facts.tx_account_is_destination);
    assert_eq!(facts.due_facts.cancel_after, Some(50));
    assert_eq!(facts.due_facts.expiration, Some(75));
    assert_eq!(facts.due_facts.close_time, 60);
    assert_eq!(facts.close_facts.channel_amount_drops, 1_000);
    assert_eq!(facts.close_facts.channel_balance_drops, 250);
    assert!(facts.balance_present);
    assert_eq!(facts.current_expiration, Some(75));
    assert_eq!(facts.settle_delay, 30);
}

#[test]
fn loaded_claim_facts_shape_payment_guards() {
    let exact_paid = build_payment_channel_claim_apply_facts(
        PaymentChannelClaimLoadedChannelFacts {
            source_account: 1_u32,
            destination_account: 2_u32,
            cancel_after: None,
            current_expiration: None,
            settle_delay: 30,
            channel_amount_drops: 1_000,
            channel_balance_drops: 250,
            destination_owner_directory_present: true,
        },
        PaymentChannelClaimLoadedTxFacts {
            tx_account: 1_u32,
            balance_present: true,
            signature_present: true,
            provided_public_key_matches_channel: false,
            requested_balance_drops: 250,
            renew_flag: true,
            close_flag: false,
        },
        60_u32,
    );

    assert!(!exact_paid.requested_balance_exceeds_channel_funds);
    assert!(exact_paid.requested_balance_not_above_channel_balance);
    assert!(!exact_paid.channel_fully_paid);
    assert!(exact_paid.signature_present);
    assert!(!exact_paid.provided_public_key_matches_channel);
    assert!(exact_paid.renew_flag);

    let full = build_payment_channel_claim_apply_facts(
        PaymentChannelClaimLoadedChannelFacts {
            source_account: 1_u32,
            destination_account: 2_u32,
            cancel_after: None,
            current_expiration: None,
            settle_delay: 30,
            channel_amount_drops: 1_000,
            channel_balance_drops: 1_000,
            destination_owner_directory_present: false,
        },
        PaymentChannelClaimLoadedTxFacts {
            tx_account: 1_u32,
            balance_present: true,
            signature_present: true,
            provided_public_key_matches_channel: true,
            requested_balance_drops: 1_100,
            renew_flag: false,
            close_flag: true,
        },
        60_u32,
    );

    assert!(full.requested_balance_exceeds_channel_funds);
    assert!(!full.requested_balance_not_above_channel_balance);
    assert!(full.channel_fully_paid);
    assert!(!full.close_facts.destination_owner_directory_present);
}
