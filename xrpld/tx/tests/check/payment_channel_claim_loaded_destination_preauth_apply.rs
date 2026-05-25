//! Integration tests that pin the destination lookup plus preauth loaded
//! claim seam to the current C++ behavior.

use protocol::Ter;
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use tx::payment_channel_claim::{PaymentChannelClaimApplySink, run_payment_channel_claim_do_apply};
use tx::payment_channel_claim_loaded::{
    PaymentChannelClaimLoadedChannelFacts, PaymentChannelClaimLoadedTxFacts,
    build_payment_channel_claim_apply_facts,
};
use tx::payment_channel_claim_loaded_destination_apply::run_payment_channel_claim_loaded_destination_do_apply;
use tx::payment_channel_claim_loaded_destination_preauth_apply::run_payment_channel_claim_loaded_destination_preauth_do_apply;
use tx::payment_channel_claim_loaded_preauth_apply::run_payment_channel_claim_loaded_preauth_do_apply;

#[derive(Debug, Default)]
struct TestApplySink {
    deposit_preauth_result: Ter,
    source_dir_result: Ter,
    destination_dir_result: Ter,
    source_account_exists: bool,
    destination_exists_result: bool,
    events: Rc<RefCell<Vec<String>>>,
    expirations: Rc<RefCell<Vec<u32>>>,
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
            destination_exists_result: true,
            events: Rc::new(RefCell::new(Vec::new())),
            expirations: Rc::new(RefCell::new(Vec::new())),
            refund_drops: None,
            owner_count_deltas: Vec::new(),
        }
    }

    fn events(&self) -> Vec<String> {
        self.events.borrow().clone()
    }

    fn expirations(&self) -> Vec<u32> {
        self.expirations.borrow().clone()
    }
}

impl PaymentChannelClaimApplySink<u32> for TestApplySink {
    fn remove_source_owner_directory(&mut self) -> Ter {
        self.events
            .borrow_mut()
            .push("remove_source_owner_directory".to_string());
        self.source_dir_result
    }

    fn remove_destination_owner_directory(&mut self) -> Ter {
        self.events
            .borrow_mut()
            .push("remove_destination_owner_directory".to_string());
        self.destination_dir_result
    }

    fn source_account_exists(&mut self) -> bool {
        self.events
            .borrow_mut()
            .push("source_account_exists".to_string());
        self.source_account_exists
    }

    fn apply_refund_to_source_account(&mut self, refund_drops: u64) {
        self.events
            .borrow_mut()
            .push("apply_refund_to_source_account".to_string());
        self.refund_drops = Some(refund_drops);
    }

    fn adjust_source_owner_count(&mut self, delta: i32) {
        self.events
            .borrow_mut()
            .push(format!("adjust_source_owner_count:{delta}"));
        self.owner_count_deltas.push(delta);
    }

    fn erase_channel(&mut self) {
        self.events.borrow_mut().push("erase_channel".to_string());
    }

    fn destination_exists(&mut self) -> bool {
        self.destination_exists_result
    }

    fn verify_deposit_preauth(&mut self) -> Ter {
        self.events
            .borrow_mut()
            .push("verify_deposit_preauth".to_string());
        self.deposit_preauth_result
    }

    fn set_channel_balance(&mut self, balance_drops: u64) {
        self.events
            .borrow_mut()
            .push(format!("set_channel_balance:{balance_drops}"));
    }

    fn add_destination_balance(&mut self, delta_drops: u64) {
        self.events
            .borrow_mut()
            .push(format!("add_destination_balance:{delta_drops}"));
    }

    fn persist_destination_balance(&mut self) {
        self.events
            .borrow_mut()
            .push("persist_destination_balance".to_string());
    }

    fn persist_channel_balance(&mut self) {
        self.events
            .borrow_mut()
            .push("persist_channel_balance".to_string());
    }

    fn clear_expiration(&mut self) {
        self.events
            .borrow_mut()
            .push("clear_expiration".to_string());
    }

    fn set_expiration(&mut self, expiration: u32) {
        self.events.borrow_mut().push("set_expiration".to_string());
        self.expirations.borrow_mut().push(expiration);
    }
}

fn loaded_channel() -> PaymentChannelClaimLoadedChannelFacts<u32, u32> {
    PaymentChannelClaimLoadedChannelFacts {
        source_account: 1,
        destination_account: 2,
        cancel_after: None,
        current_expiration: None,
        settle_delay: 30,
        channel_amount_drops: 1_000,
        channel_balance_drops: 250,
        destination_owner_directory_present: true,
    }
}

fn loaded_tx() -> PaymentChannelClaimLoadedTxFacts<u32> {
    PaymentChannelClaimLoadedTxFacts {
        tx_account: 1,
        balance_present: true,
        signature_present: true,
        provided_public_key_matches_channel: true,
        requested_balance_drops: 400,
        renew_flag: false,
        close_flag: true,
    }
}

#[test]
fn loaded_claim_destination_preauth_apply_returns_no_target_before_destination_lookup() {
    let called = Rc::new(Cell::new(false));
    let called_for_closure = Rc::clone(&called);
    let mut sink = TestApplySink::new();

    let result = run_payment_channel_claim_loaded_destination_preauth_do_apply(
        None,
        loaded_tx(),
        60_u32,
        move || {
            called_for_closure.set(true);
            true
        },
        || Ter::TES_SUCCESS,
        &mut sink,
    );

    assert_eq!(result, Ter::TEC_NO_TARGET);
    assert!(!called.get());
    assert!(sink.events().is_empty());
}

#[test]
fn loaded_claim_destination_preauth_apply_returns_no_dst_before_deposit_preauth() {
    let called = Rc::new(Cell::new(false));
    let called_for_closure = Rc::clone(&called);
    let mut sink = TestApplySink::new();

    let result = run_payment_channel_claim_loaded_destination_preauth_do_apply(
        Some(loaded_channel()),
        loaded_tx(),
        60_u32,
        move || {
            called_for_closure.set(true);
            false
        },
        || Ter::TES_SUCCESS,
        &mut sink,
    );

    assert_eq!(result, Ter::TEC_NO_DST);
    assert!(called.get());
    assert!(sink.events().is_empty());
}

#[test]
fn loaded_claim_destination_preauth_apply_stops_before_mutation_on_preauth_failure() {
    let events = Rc::new(RefCell::new(Vec::new()));
    let events_for_closure = Rc::clone(&events);
    let called = Rc::new(Cell::new(false));
    let called_for_closure = Rc::clone(&called);
    let mut sink = TestApplySink::new();

    let result = run_payment_channel_claim_loaded_destination_preauth_do_apply(
        Some(loaded_channel()),
        loaded_tx(),
        60_u32,
        || true,
        move || {
            called_for_closure.set(true);
            events_for_closure
                .borrow_mut()
                .push("verify_deposit_preauth".to_string());
            Ter::TEM_BAD_SIGNATURE
        },
        &mut sink,
    );

    assert_eq!(result, Ter::TEM_BAD_SIGNATURE);
    assert!(called.get());
    assert_eq!(events.borrow().as_slice(), ["verify_deposit_preauth"]);
    assert!(sink.expirations().is_empty());
}

#[test]
fn loaded_claim_destination_preauth_apply_matches_existing_loaded_preauth_seam() {
    let mut helper_sink = TestApplySink::new();
    let helper_events = Rc::clone(&helper_sink.events);
    let helper_result = run_payment_channel_claim_loaded_destination_preauth_do_apply(
        Some(loaded_channel()),
        loaded_tx(),
        60_u32,
        || true,
        move || {
            helper_events
                .borrow_mut()
                .push("verify_deposit_preauth".to_string());
            Ter::TES_SUCCESS
        },
        &mut helper_sink,
    );

    let mut loaded_destination_sink = TestApplySink::new();
    let loaded_destination_result = run_payment_channel_claim_loaded_destination_do_apply(
        Some(loaded_channel()),
        loaded_tx(),
        60_u32,
        true,
        &mut loaded_destination_sink,
    );

    let mut loaded_preauth_sink = TestApplySink::new();
    let loaded_preauth_events = Rc::clone(&loaded_preauth_sink.events);
    let loaded_preauth_result = run_payment_channel_claim_loaded_preauth_do_apply(
        Some(loaded_channel()),
        loaded_tx(),
        60_u32,
        true,
        move || {
            loaded_preauth_events
                .borrow_mut()
                .push("verify_deposit_preauth".to_string());
            Ter::TES_SUCCESS
        },
        &mut loaded_preauth_sink,
    );

    let mut direct_sink = TestApplySink::new();
    let direct_result = run_payment_channel_claim_do_apply(
        build_payment_channel_claim_apply_facts(loaded_channel(), loaded_tx(), 60_u32),
        &mut direct_sink,
    );

    assert_eq!(helper_result, direct_result);
    assert_eq!(loaded_destination_result, direct_result);
    assert_eq!(loaded_preauth_result, direct_result);
    assert_eq!(helper_sink.events(), direct_sink.events());
    assert_eq!(loaded_destination_sink.events(), direct_sink.events());
    assert_eq!(loaded_preauth_sink.events(), direct_sink.events());
    assert_eq!(
        helper_sink.events(),
        [
            "verify_deposit_preauth",
            "set_channel_balance:400",
            "add_destination_balance:150",
            "persist_destination_balance",
            "persist_channel_balance",
            "set_expiration"
        ]
    );
    assert_eq!(helper_sink.expirations(), vec![90]);
}
