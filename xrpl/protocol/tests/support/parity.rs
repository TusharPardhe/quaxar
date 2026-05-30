//! Integration tests that pin public `xrpl/protocol` behavior to the current
//! C++ migration surface.

use basics::local_value::{LocalSlotOwner, install_local_slot_owner};
use basics::number::{MantissaScale, get_mantissa_scale, set_mantissa_scale};
use protocol::{
    NumberSo, Rules, SeqProxy, Ter, TransactionApplyRuntimeGuard, TransactionStepRuntimeGuard,
    feature_single_asset_vault, feature_universal_number, fix_cleanup_3_2_0,
    get_current_transaction_rules, get_st_number_switchover, set_current_transaction_rules,
    set_st_number_switchover,
};
use std::thread;

fn with_local_slot_owner<R>(owner: &LocalSlotOwner, f: impl FnOnce() -> R) -> R {
    let _guard = install_local_slot_owner(owner);
    f()
}

#[test]
fn st_number_switchover_defaults_to_enabled() {
    set_st_number_switchover(true);
    assert!(get_st_number_switchover());
}

#[test]
fn st_number_switchover_setter_round_trips() {
    set_st_number_switchover(true);
    assert!(get_st_number_switchover());

    set_st_number_switchover(false);
    assert!(!get_st_number_switchover());

    set_st_number_switchover(true);
    assert!(get_st_number_switchover());
}

#[test]
fn number_so_restores_previous_value_on_drop() {
    set_st_number_switchover(true);

    {
        let _guard = NumberSo::new(false);
        assert!(!get_st_number_switchover());
    }

    assert!(get_st_number_switchover());
}

#[test]
fn st_number_switchover_is_thread_local_outside_entered_contexts() {
    set_st_number_switchover(false);

    let worker_value = thread::spawn(|| {
        let initial = get_st_number_switchover();
        set_st_number_switchover(false);
        (initial, get_st_number_switchover())
    })
    .join()
    .expect("thread should complete");

    assert_eq!(worker_value, (true, false));
    assert!(!get_st_number_switchover());

    set_st_number_switchover(true);
}

#[test]
fn st_number_switchover_follows_local_context_scoping_local_value() {
    set_st_number_switchover(true);
    let owner = LocalSlotOwner::new();

    with_local_slot_owner(&owner, || {
        assert!(get_st_number_switchover());
        set_st_number_switchover(false);
        assert!(!get_st_number_switchover());
    });

    assert!(get_st_number_switchover());

    with_local_slot_owner(&owner, || {
        assert!(!get_st_number_switchover());
    });
}

#[test]
fn transaction_apply_runtime_guard_sets_rules_and_stnumber_from_amendments() {
    set_current_transaction_rules(None);
    set_st_number_switchover(true);
    let rules = Rules::new([feature_universal_number()]);

    {
        let _guard = TransactionApplyRuntimeGuard::new(&rules);
        assert_eq!(get_current_transaction_rules(), Some(rules.clone()));
        assert!(get_st_number_switchover());
        assert_eq!(get_mantissa_scale(), MantissaScale::Small);
    }

    assert_eq!(get_current_transaction_rules(), None);
    assert!(get_st_number_switchover());
    assert_eq!(get_mantissa_scale(), MantissaScale::Large);
}

#[test]
fn transaction_apply_runtime_guard_can_disable_stnumber_for_legacy_rules() {
    set_current_transaction_rules(None);
    set_st_number_switchover(true);
    let rules = Rules::new(std::iter::empty());

    {
        let _guard = TransactionApplyRuntimeGuard::new(&rules);
        assert_eq!(get_current_transaction_rules(), Some(rules.clone()));
        assert!(!get_st_number_switchover());
        assert_eq!(get_mantissa_scale(), MantissaScale::Small);
    }

    assert_eq!(get_current_transaction_rules(), None);
    assert!(get_st_number_switchover());
    assert_eq!(get_mantissa_scale(), MantissaScale::Large);
}

#[test]
fn transaction_step_runtime_guard_uses_legacy_small_mantissa_path_without_new_features() {
    set_current_transaction_rules(None);
    set_st_number_switchover(false);
    set_mantissa_scale(MantissaScale::Large);
    let rules = Rules::new(std::iter::empty());

    {
        let _guard = TransactionStepRuntimeGuard::new(&rules);
        assert_eq!(get_current_transaction_rules(), None);
        assert!(!get_st_number_switchover());
        assert_eq!(get_mantissa_scale(), MantissaScale::Small);
    }

    assert_eq!(get_current_transaction_rules(), None);
    assert!(!get_st_number_switchover());
    assert_eq!(get_mantissa_scale(), MantissaScale::Large);

    set_st_number_switchover(true);
}

#[test]
fn transaction_step_runtime_guard_uses_new_feature_path_when_single_asset_vault_is_enabled() {
    set_current_transaction_rules(None);
    set_st_number_switchover(true);
    let rules = Rules::new([feature_single_asset_vault()]);

    {
        let _guard = TransactionStepRuntimeGuard::new(&rules);
        assert_eq!(get_current_transaction_rules(), Some(rules.clone()));
        assert!(!get_st_number_switchover());
        assert_eq!(get_mantissa_scale(), MantissaScale::LargeLegacy);
    }

    let fixed_rules = Rules::new([feature_single_asset_vault(), fix_cleanup_3_2_0()]);

    {
        let _guard = TransactionStepRuntimeGuard::new(&fixed_rules);
        assert_eq!(get_current_transaction_rules(), Some(fixed_rules.clone()));
        assert!(!get_st_number_switchover());
        assert_eq!(get_mantissa_scale(), MantissaScale::Large);
    }

    assert_eq!(get_current_transaction_rules(), None);
    assert!(get_st_number_switchover());
    assert_eq!(get_mantissa_scale(), MantissaScale::Large);
}

#[test]
fn seq_proxy_ordering_matches_current_cpp_sort_rule() {
    assert!(SeqProxy::sequence(100) < SeqProxy::ticket(1));
    assert!(SeqProxy::sequence(7) < SeqProxy::sequence(8));
    assert!(SeqProxy::ticket(7) < SeqProxy::ticket(8));
}

#[test]
fn seq_proxy_advance_and_display_match_current_cpp_behavior() {
    let mut seq = SeqProxy::sequence(9);
    let mut ticket = SeqProxy::ticket(11);
    seq.advance_by(2);
    ticket.advance_by(3);

    assert_eq!(seq, SeqProxy::sequence(11));
    assert_eq!(ticket, SeqProxy::ticket(14));
    assert_eq!(seq.to_string(), "sequence 11");
    assert_eq!(ticket.to_string(), "ticket 14");
}

#[test]
fn queue_related_ter_codes_match_current_cpp_values() {
    assert_eq!(Ter::TEL_INSUF_FEE_P.to_int(), -394);
    assert_eq!(Ter::TEL_CAN_NOT_QUEUE.to_int(), -392);
    assert_eq!(Ter::TEL_CAN_NOT_QUEUE_BALANCE.to_int(), -391);
    assert_eq!(Ter::TEL_CAN_NOT_QUEUE_BLOCKS.to_int(), -390);
    assert_eq!(Ter::TEL_CAN_NOT_QUEUE_BLOCKED.to_int(), -389);
    assert_eq!(Ter::TEL_CAN_NOT_QUEUE_FEE.to_int(), -388);
    assert_eq!(Ter::TEL_CAN_NOT_QUEUE_FULL.to_int(), -387);
    assert_eq!(Ter::TEF_PAST_SEQ.to_int(), -190);
    assert_eq!(Ter::TEF_NO_TICKET.to_int(), -180);
    assert_eq!(Ter::TER_NO_ACCOUNT.to_int(), -96);
    assert_eq!(Ter::TER_PRE_SEQ.to_int(), -92);
    assert_eq!(Ter::TER_PRE_TICKET.to_int(), -88);
}

#[test]
fn ter_public_lookup_helpers_match_cpp_surface() {
    use protocol::ter::{trans_code, trans_human};

    assert_eq!(
        trans_human(Ter::TEF_BAD_ADD_AUTH),
        "Not authorized to add account."
    );
    assert_eq!(
        trans_code("tefBAD_AUTH_MASTER"),
        Some(Ter::TEF_BAD_AUTH_MASTER)
    );
    assert_eq!(
        trans_code("tecPSEUDO_ACCOUNT"),
        Some(Ter::TEC_PSEUDO_ACCOUNT)
    );
    assert_eq!(trans_human(Ter::TEC_NO_DELEGATE_PERMISSION), "-");
    assert_eq!(trans_code("tecHOOK_REJECTED"), None);
}
