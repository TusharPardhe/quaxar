//! Rust compatibility surface for the reference implementation.
//!
//! This module preserves the current deterministic
//! `checkExtraFeatures(...)`, `getFlagsMask(...)`, `preflight(...)`,
//! `preclaim(...)`, and `doApply()` shell behavior around the surrounding
//! keylet, signature, credential, close-time, ledger-read, and mutation work.

use crate::payment_channel_claim_loaded::{
    PaymentChannelClaimLoadedPreparedPaymentFacts,
    build_payment_channel_claim_loaded_prepared_payment_facts,
    run_payment_channel_claim_loaded_prepared_payment_do_apply,
};
use crate::payment_channel_claim_settle::{
    PaymentChannelClaimSettleFacts, PaymentChannelClaimSettleSink, run_payment_channel_claim_settle,
};
use crate::payment_channel_due::{PaymentChannelDueFacts, is_payment_channel_due};
use crate::payment_channel_helpers::{
    PaymentChannelCloseFacts, PaymentChannelCloseSink, run_payment_channel_close,
};
use basics::base_uint::Uint256;
use protocol::{INNER_BATCH_TRANSACTION_FLAG, NotTec, Ter};
use std::marker::PhantomData;
use std::ops::Add;

pub const FULLY_CANONICAL_SIGNATURE_FLAG: u32 = 0x8000_0000;
pub const PAYMENT_CHANNEL_CLAIM_RENEW_FLAG: u32 = 0x0001_0000;
pub const PAYMENT_CHANNEL_CLAIM_CLOSE_FLAG: u32 = 0x0002_0000;
pub const PAYMENT_CHANNEL_CLAIM_FLAGS_MASK: u32 = !(FULLY_CANONICAL_SIGNATURE_FLAG
    | INNER_BATCH_TRANSACTION_FLAG
    | PAYMENT_CHANNEL_CLAIM_RENEW_FLAG
    | PAYMENT_CHANNEL_CLAIM_CLOSE_FLAG);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PaymentChannelClaimPreflightFacts {
    pub balance_present: bool,
    pub balance_is_xrp: bool,
    pub balance_positive: bool,
    pub amount_present: bool,
    pub amount_is_xrp: bool,
    pub amount_positive: bool,
    pub balance_exceeds_amount: bool,
    pub tx_flags: u32,
    pub signature: Option<PaymentChannelClaimSignaturePreflightFacts>,
}

pub trait PaymentChannelClaimApplySink<Time> {
    fn remove_source_owner_directory(&mut self) -> Ter;
    fn remove_destination_owner_directory(&mut self) -> Ter;
    fn source_account_exists(&mut self) -> bool;
    fn apply_refund_to_source_account(&mut self, refund_drops: u64);
    fn adjust_source_owner_count(&mut self, delta: i32);
    fn erase_channel(&mut self);
    fn destination_exists(&mut self) -> bool;
    fn verify_deposit_preauth(&mut self) -> Ter;
    fn set_channel_balance(&mut self, balance_drops: u64);
    fn add_destination_balance(&mut self, delta_drops: u64);
    fn persist_destination_balance(&mut self);
    fn persist_channel_balance(&mut self);
    fn clear_expiration(&mut self);
    fn set_expiration(&mut self, expiration: Time);
}

struct PaymentChannelClaimCloseSinkAdapter<'a, Time, S> {
    sink: &'a mut S,
    _time: PhantomData<Time>,
}

struct PaymentChannelClaimSettleSinkAdapter<'a, Time, S> {
    sink: &'a mut S,
    close_facts: PaymentChannelCloseFacts,
    _time: PhantomData<Time>,
}

impl<'a, Time, S> PaymentChannelClaimSettleSink<Time>
    for PaymentChannelClaimSettleSinkAdapter<'a, Time, S>
where
    S: PaymentChannelClaimApplySink<Time>,
{
    fn clear_expiration(&mut self) {
        self.sink.clear_expiration();
    }

    fn set_expiration(&mut self, expiration: Time) {
        self.sink.set_expiration(expiration);
    }

    fn close_channel(&mut self) -> Ter {
        run_payment_channel_close(
            self.close_facts,
            &mut PaymentChannelClaimCloseSinkAdapter {
                sink: self.sink,
                _time: PhantomData,
            },
        )
    }
}

impl<'a, Time, S> PaymentChannelCloseSink for PaymentChannelClaimCloseSinkAdapter<'a, Time, S>
where
    S: PaymentChannelClaimApplySink<Time>,
{
    fn remove_source_owner_directory(&mut self) -> Ter {
        self.sink.remove_source_owner_directory()
    }

    fn remove_destination_owner_directory(&mut self) -> Ter {
        self.sink.remove_destination_owner_directory()
    }

    fn source_account_exists(&mut self) -> bool {
        self.sink.source_account_exists()
    }

    fn apply_refund_to_source_account(&mut self, refund_drops: u64) {
        self.sink.apply_refund_to_source_account(refund_drops)
    }

    fn adjust_source_owner_count(&mut self, delta: i32) {
        self.sink.adjust_source_owner_count(delta)
    }

    fn erase_channel(&mut self) {
        self.sink.erase_channel()
    }
}

pub const PAYMENT_CHANNEL_CLAIM_AUTHORIZATION_PREFIX: u32 =
    protocol::PAYMENT_CHANNEL_CLAIM_HASH_PREFIX;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PaymentChannelClaimAuthorizationMessageFacts {
    pub channel_key: Uint256,
    pub authorized_amount_drops: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PaymentChannelClaimSignaturePreflightFacts {
    pub public_key_present: bool,
    pub requested_balance_drops: u64,
    pub authorization_message: PaymentChannelClaimAuthorizationMessageFacts,
    pub public_key_type_valid: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PaymentChannelClaimApplyFacts<Time> {
    pub channel_exists: bool,
    pub due_facts: PaymentChannelDueFacts<Time>,
    pub close_facts: PaymentChannelCloseFacts,
    pub tx_account_is_source: bool,
    pub tx_account_is_destination: bool,
    pub balance_present: bool,
    pub signature_present: bool,
    pub provided_public_key_matches_channel: bool,
    pub requested_balance_exceeds_channel_funds: bool,
    pub requested_balance_not_above_channel_balance: bool,
    pub channel_balance_drops: u64,
    pub requested_balance_drops: u64,
    pub renew_flag: bool,
    pub close_flag: bool,
    pub channel_fully_paid: bool,
    pub current_expiration: Option<Time>,
    pub close_time: Time,
    pub settle_delay: Time,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PaymentChannelClaimPreparedDoApplyFacts<Time> {
    pub channel_exists: bool,
    pub due_facts: PaymentChannelDueFacts<Time>,
    pub close_facts: PaymentChannelCloseFacts,
    pub tx_account_is_source: bool,
    pub tx_account_is_destination: bool,
    pub prepared_payment_facts: Option<PaymentChannelClaimLoadedPreparedPaymentFacts<Time>>,
    pub settle_facts: PaymentChannelClaimSettleFacts<Time>,
}

pub const fn run_payment_channel_claim_check_extra_features(
    credential_ids_present: bool,
    feature_credentials_enabled: bool,
) -> bool {
    !credential_ids_present || feature_credentials_enabled
}

pub const fn get_payment_channel_claim_flags_mask() -> u32 {
    PAYMENT_CHANNEL_CLAIM_FLAGS_MASK
}

pub fn serialize_payment_channel_claim_authorization_message(
    facts: &PaymentChannelClaimAuthorizationMessageFacts,
) -> Vec<u8> {
    protocol::serialize_pay_chan_authorization(&facts.channel_key, facts.authorized_amount_drops)
}

pub fn run_payment_channel_claim_signature_preflight(
    balance_present: bool,
    facts: PaymentChannelClaimSignaturePreflightFacts,
    verify_signature: impl FnOnce(&[u8]) -> bool,
) -> Ter {
    if !balance_present || !facts.public_key_present {
        return Ter::TEM_MALFORMED;
    }

    if facts.requested_balance_drops > facts.authorization_message.authorized_amount_drops {
        return Ter::TEM_BAD_AMOUNT;
    }

    if !facts.public_key_type_valid {
        return Ter::TEM_MALFORMED;
    }

    let authorization_message =
        serialize_payment_channel_claim_authorization_message(&facts.authorization_message);
    if !verify_signature(authorization_message.as_slice()) {
        return Ter::TEM_BAD_SIGNATURE;
    }

    Ter::TES_SUCCESS
}

pub fn run_payment_channel_claim_preflight(
    facts: PaymentChannelClaimPreflightFacts,
    verify_signature: impl FnOnce(&[u8]) -> bool,
    check_credentials_fields: impl FnOnce() -> NotTec,
) -> NotTec {
    if facts.balance_present && (!facts.balance_is_xrp || !facts.balance_positive) {
        return Ter::TEM_BAD_AMOUNT;
    }

    if facts.amount_present && (!facts.amount_is_xrp || !facts.amount_positive) {
        return Ter::TEM_BAD_AMOUNT;
    }

    if facts.balance_exceeds_amount {
        return Ter::TEM_BAD_AMOUNT;
    }

    if (facts.tx_flags & PAYMENT_CHANNEL_CLAIM_CLOSE_FLAG) != 0
        && (facts.tx_flags & PAYMENT_CHANNEL_CLAIM_RENEW_FLAG) != 0
    {
        return Ter::TEM_MALFORMED;
    }

    if let Some(signature_facts) = facts.signature {
        let err = run_payment_channel_claim_signature_preflight(
            facts.balance_present,
            signature_facts,
            verify_signature,
        );
        if err != Ter::TES_SUCCESS {
            return err;
        }
    }

    check_credentials_fields()
}

pub fn run_payment_channel_claim_preclaim(
    feature_credentials_enabled: bool,
    lower_preclaim: impl FnOnce() -> Ter,
    validate_credentials: impl FnOnce() -> Ter,
) -> Ter {
    if !feature_credentials_enabled {
        return lower_preclaim();
    }

    let err = validate_credentials();
    if err != Ter::TES_SUCCESS {
        return err;
    }

    Ter::TES_SUCCESS
}

fn build_payment_channel_claim_settle_facts<Time>(
    facts: PaymentChannelClaimApplyFacts<Time>,
) -> PaymentChannelClaimSettleFacts<Time> {
    PaymentChannelClaimSettleFacts {
        tx_account_is_source: facts.tx_account_is_source,
        renew_flag: facts.renew_flag,
        close_flag: facts.close_flag,
        tx_account_is_destination: facts.tx_account_is_destination,
        channel_fully_paid: facts.channel_fully_paid,
        current_expiration: facts.current_expiration,
        close_time: facts.close_time,
        settle_delay: facts.settle_delay,
    }
}

fn build_payment_channel_claim_settle_branch_facts<Time>(
    facts: PaymentChannelClaimApplyFacts<Time>,
) -> (
    PaymentChannelCloseFacts,
    PaymentChannelClaimSettleFacts<Time>,
) {
    (
        facts.close_facts,
        build_payment_channel_claim_settle_facts(facts),
    )
}

pub fn build_payment_channel_claim_prepared_do_apply_facts<Time>(
    facts: PaymentChannelClaimApplyFacts<Time>,
) -> PaymentChannelClaimPreparedDoApplyFacts<Time>
where
    Time: Copy,
{
    let (close_facts, settle_facts) = build_payment_channel_claim_settle_branch_facts(facts);
    PaymentChannelClaimPreparedDoApplyFacts {
        channel_exists: facts.channel_exists,
        due_facts: facts.due_facts,
        close_facts,
        tx_account_is_source: facts.tx_account_is_source,
        tx_account_is_destination: facts.tx_account_is_destination,
        prepared_payment_facts: facts
            .balance_present
            .then(|| build_payment_channel_claim_loaded_prepared_payment_facts(facts)),
        settle_facts,
    }
}

fn run_payment_channel_claim_payment_branch<Time, S>(
    prepared_payment_facts: Option<PaymentChannelClaimLoadedPreparedPaymentFacts<Time>>,
    sink: &mut S,
) -> Ter
where
    Time: Copy + Ord + Add<Output = Time>,
    S: PaymentChannelClaimApplySink<Time>,
{
    let Some(prepared_payment_facts) = prepared_payment_facts else {
        return Ter::TES_SUCCESS;
    };

    let err =
        run_payment_channel_claim_loaded_prepared_payment_do_apply(prepared_payment_facts, sink);
    if err != Ter::TES_SUCCESS {
        return err;
    }

    Ter::TES_SUCCESS
}

pub fn run_payment_channel_claim_prepared_do_apply<Time, S>(
    prepared: PaymentChannelClaimPreparedDoApplyFacts<Time>,
    sink: &mut S,
) -> Ter
where
    Time: Copy + Ord + Add<Output = Time>,
    S: PaymentChannelClaimApplySink<Time>,
{
    if !prepared.channel_exists {
        return Ter::TEC_NO_TARGET;
    }

    if is_payment_channel_due(prepared.due_facts) {
        return run_payment_channel_close(
            prepared.close_facts,
            &mut PaymentChannelClaimCloseSinkAdapter {
                sink,
                _time: PhantomData,
            },
        );
    }

    if !prepared.tx_account_is_source && !prepared.tx_account_is_destination {
        return Ter::TEC_NO_PERMISSION;
    }

    let err = run_payment_channel_claim_payment_branch(prepared.prepared_payment_facts, sink);
    if err != Ter::TES_SUCCESS {
        return err;
    }

    run_payment_channel_claim_settle(
        prepared.settle_facts,
        &mut PaymentChannelClaimSettleSinkAdapter {
            sink,
            close_facts: prepared.close_facts,
            _time: PhantomData,
        },
    )
}

pub fn run_payment_channel_claim_do_apply<Time, S>(
    facts: PaymentChannelClaimApplyFacts<Time>,
    sink: &mut S,
) -> Ter
where
    Time: Copy + Ord + Add<Output = Time>,
    S: PaymentChannelClaimApplySink<Time>,
{
    run_payment_channel_claim_prepared_do_apply(
        build_payment_channel_claim_prepared_do_apply_facts(facts),
        sink,
    )
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use basics::base_uint::Uint256;

    use super::{
        FULLY_CANONICAL_SIGNATURE_FLAG, PAYMENT_CHANNEL_CLAIM_CLOSE_FLAG,
        PAYMENT_CHANNEL_CLAIM_FLAGS_MASK, PAYMENT_CHANNEL_CLAIM_RENEW_FLAG,
        PaymentChannelClaimApplyFacts, PaymentChannelClaimApplySink,
        PaymentChannelClaimAuthorizationMessageFacts, PaymentChannelClaimPreflightFacts,
        PaymentChannelClaimSignaturePreflightFacts, get_payment_channel_claim_flags_mask,
        run_payment_channel_claim_check_extra_features, run_payment_channel_claim_do_apply,
        run_payment_channel_claim_preclaim, run_payment_channel_claim_preflight,
        run_payment_channel_claim_signature_preflight,
        serialize_payment_channel_claim_authorization_message,
    };
    use crate::payment_channel_due::PaymentChannelDueFacts;
    use crate::payment_channel_helpers::PaymentChannelCloseFacts;
    use protocol::{INNER_BATCH_TRANSACTION_FLAG, Ter};

    #[derive(Debug, Default)]
    struct TestApplySink {
        deposit_preauth_result: Ter,
        source_dir_result: Ter,
        destination_dir_result: Ter,
        source_account_exists: bool,
        events: Vec<String>,
        expirations: Vec<u32>,
        refund_drops: Option<u64>,
        owner_count_deltas: Vec<i32>,
    }

    impl TestApplySink {
        fn new() -> Self {
            Self {
                deposit_preauth_result: Ter::TES_SUCCESS,
                source_dir_result: Ter::TES_SUCCESS,
                destination_dir_result: Ter::TES_SUCCESS,
                source_account_exists: true,
                events: Vec::new(),
                expirations: Vec::new(),
                refund_drops: None,
                owner_count_deltas: Vec::new(),
            }
        }
    }

    impl PaymentChannelClaimApplySink<u32> for TestApplySink {
        fn remove_source_owner_directory(&mut self) -> Ter {
            self.events
                .push("remove_source_owner_directory".to_string());
            self.source_dir_result
        }

        fn remove_destination_owner_directory(&mut self) -> Ter {
            self.events
                .push("remove_destination_owner_directory".to_string());
            self.destination_dir_result
        }

        fn source_account_exists(&mut self) -> bool {
            self.events.push("source_account_exists".to_string());
            self.source_account_exists
        }

        fn apply_refund_to_source_account(&mut self, refund_drops: u64) {
            self.events
                .push("apply_refund_to_source_account".to_string());
            self.refund_drops = Some(refund_drops);
        }

        fn adjust_source_owner_count(&mut self, delta: i32) {
            self.events
                .push(format!("adjust_source_owner_count:{delta}"));
            self.owner_count_deltas.push(delta);
        }

        fn erase_channel(&mut self) {
            self.events.push("erase_channel".to_string());
        }

        fn destination_exists(&mut self) -> bool {
            self.events.push("destination_exists".to_string());
            true
        }

        fn verify_deposit_preauth(&mut self) -> Ter {
            self.events.push("verify_deposit_preauth".to_string());
            self.deposit_preauth_result
        }

        fn set_channel_balance(&mut self, balance_drops: u64) {
            self.events
                .push(format!("set_channel_balance:{balance_drops}"));
        }

        fn add_destination_balance(&mut self, delta_drops: u64) {
            self.events
                .push(format!("add_destination_balance:{delta_drops}"));
        }

        fn persist_destination_balance(&mut self) {
            self.events.push("persist_destination_balance".to_string());
        }

        fn persist_channel_balance(&mut self) {
            self.events.push("persist_channel_balance".to_string());
        }

        fn clear_expiration(&mut self) {
            self.events.push("clear_expiration".to_string());
        }

        fn set_expiration(&mut self, expiration: u32) {
            self.events.push("set_expiration".to_string());
            self.expirations.push(expiration);
        }
    }

    fn preflight_facts() -> PaymentChannelClaimPreflightFacts {
        PaymentChannelClaimPreflightFacts {
            balance_present: false,
            balance_is_xrp: true,
            balance_positive: true,
            amount_present: false,
            amount_is_xrp: true,
            amount_positive: true,
            balance_exceeds_amount: false,
            tx_flags: 0,
            signature: None,
        }
    }

    fn signature_facts() -> PaymentChannelClaimSignaturePreflightFacts {
        PaymentChannelClaimSignaturePreflightFacts {
            public_key_present: true,
            requested_balance_drops: 400,
            authorization_message: PaymentChannelClaimAuthorizationMessageFacts {
                channel_key: Uint256::from_hex(
                    "0123456789ABCDEFFEDCBA98765432100123456789ABCDEFFEDCBA9876543210",
                )
                .expect("channel key should parse"),
                authorized_amount_drops: 500,
            },
            public_key_type_valid: true,
        }
    }

    fn apply_facts() -> PaymentChannelClaimApplyFacts<u32> {
        PaymentChannelClaimApplyFacts {
            channel_exists: true,
            due_facts: PaymentChannelDueFacts {
                cancel_after: None,
                expiration: None,
                close_time: 60,
            },
            close_facts: PaymentChannelCloseFacts {
                destination_owner_directory_present: true,
                channel_amount_drops: 1_000,
                channel_balance_drops: 250,
            },
            tx_account_is_source: true,
            tx_account_is_destination: false,
            balance_present: false,
            signature_present: false,
            provided_public_key_matches_channel: true,
            requested_balance_exceeds_channel_funds: false,
            requested_balance_not_above_channel_balance: false,
            channel_balance_drops: 250,
            requested_balance_drops: 400,
            renew_flag: false,
            close_flag: false,
            channel_fully_paid: false,
            current_expiration: None,
            close_time: 60,
            settle_delay: 30,
        }
    }

    #[test]
    fn claim_check_extra_features_gate() {
        assert!(run_payment_channel_claim_check_extra_features(false, false));
        assert!(run_payment_channel_claim_check_extra_features(true, true));
        assert!(!run_payment_channel_claim_check_extra_features(true, false));
    }

    #[test]
    fn claim_flags_mask_txflags() {
        assert_eq!(PAYMENT_CHANNEL_CLAIM_RENEW_FLAG, 0x0001_0000);
        assert_eq!(PAYMENT_CHANNEL_CLAIM_CLOSE_FLAG, 0x0002_0000);
        assert_eq!(PAYMENT_CHANNEL_CLAIM_FLAGS_MASK, 0x3ffc_ffff);
        assert_eq!(
            get_payment_channel_claim_flags_mask(),
            !(FULLY_CANONICAL_SIGNATURE_FLAG
                | INNER_BATCH_TRANSACTION_FLAG
                | PAYMENT_CHANNEL_CLAIM_RENEW_FLAG
                | PAYMENT_CHANNEL_CLAIM_CLOSE_FLAG)
        );
    }

    #[test]
    fn claim_preflight_keepsing() {
        assert_eq!(
            run_payment_channel_claim_preflight(
                PaymentChannelClaimPreflightFacts {
                    balance_present: true,
                    balance_is_xrp: false,
                    ..preflight_facts()
                },
                |_| panic!("verifier should not run after bad balance"),
                || panic!("credentials should not run after bad balance"),
            ),
            Ter::TEM_BAD_AMOUNT
        );
        assert_eq!(
            run_payment_channel_claim_preflight(
                PaymentChannelClaimPreflightFacts {
                    amount_present: true,
                    amount_positive: false,
                    ..preflight_facts()
                },
                |_| panic!("verifier should not run after bad amount"),
                || panic!("credentials should not run after bad amount"),
            ),
            Ter::TEM_BAD_AMOUNT
        );
        assert_eq!(
            run_payment_channel_claim_preflight(
                PaymentChannelClaimPreflightFacts {
                    balance_exceeds_amount: true,
                    ..preflight_facts()
                },
                |_| panic!("verifier should not run after balance > amount"),
                || panic!("credentials should not run after balance > amount"),
            ),
            Ter::TEM_BAD_AMOUNT
        );
        assert_eq!(
            run_payment_channel_claim_preflight(
                PaymentChannelClaimPreflightFacts {
                    tx_flags: PAYMENT_CHANNEL_CLAIM_RENEW_FLAG | PAYMENT_CHANNEL_CLAIM_CLOSE_FLAG,
                    ..preflight_facts()
                },
                |_| panic!("verifier should not run after malformed flags"),
                || panic!("credentials should not run after malformed flags"),
            ),
            Ter::TEM_MALFORMED
        );
    }

    #[test]
    fn claim_signature_preflight_serializes_cpp_authorization_message() {
        let message = serialize_payment_channel_claim_authorization_message(
            &signature_facts().authorization_message,
        );

        assert_eq!(
            message,
            vec![
                0x43, 0x4c, 0x4d, 0x00, 0x01, 0x23, 0x45, 0x67, 0x89, 0xAB, 0xCD, 0xEF, 0xFE, 0xDC,
                0xBA, 0x98, 0x76, 0x54, 0x32, 0x10, 0x01, 0x23, 0x45, 0x67, 0x89, 0xAB, 0xCD, 0xEF,
                0xFE, 0xDC, 0xBA, 0x98, 0x76, 0x54, 0x32, 0x10, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
                0x01, 0xF4
            ]
        );
    }

    #[test]
    fn claim_preflight_checks_signature_failures_in() {
        assert_eq!(
            run_payment_channel_claim_signature_preflight(false, signature_facts(), |_| panic!(
                "verifier should not run without a balance"
            ),),
            Ter::TEM_MALFORMED
        );
        assert_eq!(
            run_payment_channel_claim_signature_preflight(
                true,
                PaymentChannelClaimSignaturePreflightFacts {
                    public_key_present: false,
                    ..signature_facts()
                },
                |_| panic!("verifier should not run without a public key"),
            ),
            Ter::TEM_MALFORMED
        );
        assert_eq!(
            run_payment_channel_claim_signature_preflight(
                true,
                PaymentChannelClaimSignaturePreflightFacts {
                    requested_balance_drops: 600,
                    ..signature_facts()
                },
                |_| panic!("verifier should not run after bad amount"),
            ),
            Ter::TEM_BAD_AMOUNT
        );
        assert_eq!(
            run_payment_channel_claim_signature_preflight(
                true,
                PaymentChannelClaimSignaturePreflightFacts {
                    public_key_type_valid: false,
                    ..signature_facts()
                },
                |_| panic!("verifier should not run after bad key type"),
            ),
            Ter::TEM_MALFORMED
        );
        assert_eq!(
            run_payment_channel_claim_preflight(
                PaymentChannelClaimPreflightFacts {
                    balance_present: true,
                    signature: Some(signature_facts()),
                    ..preflight_facts()
                },
                |_| false,
                || panic!("credentials should not run after bad signature"),
            ),
            Ter::TEM_BAD_SIGNATURE
        );
    }

    #[test]
    fn claim_preflight_returns_credentials_result_after_local_checks() {
        let result = run_payment_channel_claim_preflight(
            preflight_facts(),
            |_| panic!("verifier should not run without a signature"),
            || Ter::TEC_NO_PERMISSION,
        );
        assert_eq!(result, Ter::TEC_NO_PERMISSION);
    }

    #[test]
    fn claim_preclaim_delegates_without_credentials_feature() {
        let lower_called = Cell::new(false);
        let credentials_called = Cell::new(false);

        let result = run_payment_channel_claim_preclaim(
            false,
            || {
                lower_called.set(true);
                Ter::TER_NO_ACCOUNT
            },
            || {
                credentials_called.set(true);
                Ter::TEC_NO_PERMISSION
            },
        );

        assert_eq!(result, Ter::TER_NO_ACCOUNT);
        assert!(lower_called.get());
        assert!(!credentials_called.get());
    }

    #[test]
    fn claim_preclaim_uses_credentials_validation_when_feature_enabled() {
        let lower_called = Cell::new(false);
        let credentials_called = Cell::new(false);

        let result = run_payment_channel_claim_preclaim(
            true,
            || {
                lower_called.set(true);
                Ter::TER_NO_ACCOUNT
            },
            || {
                credentials_called.set(true);
                Ter::TEC_NO_PERMISSION
            },
        );

        assert_eq!(result, Ter::TEC_NO_PERMISSION);
        assert!(!lower_called.get());
        assert!(credentials_called.get());
    }

    #[test]
    fn claim_do_apply_closes_expired_channel_before_permission() {
        let mut sink = TestApplySink::new();
        sink.source_dir_result = Ter::TES_SUCCESS;
        sink.destination_dir_result = Ter::TES_SUCCESS;
        sink.source_account_exists = true;

        let result = run_payment_channel_claim_do_apply(
            PaymentChannelClaimApplyFacts {
                due_facts: PaymentChannelDueFacts {
                    cancel_after: Some(60),
                    expiration: None,
                    close_time: 60,
                },
                tx_account_is_source: false,
                tx_account_is_destination: false,
                ..apply_facts()
            },
            &mut sink,
        );

        assert_eq!(result, Ter::TES_SUCCESS);
        assert_eq!(
            sink.events,
            [
                "remove_source_owner_directory",
                "remove_destination_owner_directory",
                "source_account_exists",
                "apply_refund_to_source_account",
                "adjust_source_owner_count:-1",
                "erase_channel"
            ]
        );
        assert_eq!(sink.refund_drops, Some(750));
        assert_eq!(sink.owner_count_deltas, [-1]);
    }

    #[test]
    fn claim_do_apply_propagates_close_helper_failures() {
        let mut sink = TestApplySink::new();
        sink.source_dir_result = Ter::TEC_NO_DST;

        let result = run_payment_channel_claim_do_apply(
            PaymentChannelClaimApplyFacts {
                due_facts: PaymentChannelDueFacts {
                    cancel_after: Some(60),
                    expiration: None,
                    close_time: 60,
                },
                ..apply_facts()
            },
            &mut sink,
        );

        assert_eq!(result, Ter::TEF_BAD_LEDGER);
        assert_eq!(sink.events, ["remove_source_owner_directory"]);
    }

    #[test]
    fn claim_do_apply_runs_payment_then_renew_then_close_in() {
        let mut sink = TestApplySink::new();

        let result = run_payment_channel_claim_do_apply(
            PaymentChannelClaimApplyFacts {
                balance_present: true,
                renew_flag: true,
                close_flag: true,
                current_expiration: Some(100),
                ..apply_facts()
            },
            &mut sink,
        );

        assert_eq!(result, Ter::TES_SUCCESS);
        assert_eq!(
            sink.events,
            [
                "destination_exists",
                "verify_deposit_preauth",
                "set_channel_balance:400",
                "add_destination_balance:150",
                "persist_destination_balance",
                "persist_channel_balance",
                "clear_expiration",
                "set_expiration"
            ]
        );
        assert_eq!(sink.expirations, vec![90]);
    }

    #[test]
    fn claim_do_apply_returns_deposit_preauth_error_before_mutation() {
        let mut sink = TestApplySink::new();
        sink.deposit_preauth_result = Ter::TEC_NO_PERMISSION;

        let result = run_payment_channel_claim_do_apply(
            PaymentChannelClaimApplyFacts {
                balance_present: true,
                ..apply_facts()
            },
            &mut sink,
        );

        assert_eq!(result, Ter::TEC_NO_PERMISSION);
        assert_eq!(
            sink.events,
            ["destination_exists", "verify_deposit_preauth"]
        );
    }
}
