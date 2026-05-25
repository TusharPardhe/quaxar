use std::cell::{Cell, RefCell};

use basics::base_uint::Uint256;
use protocol::{Rules, SeqProxy, Ter, feature_single_asset_vault};
use tx::{ApplyFlags, ApplyResult, MaybeTx, PreclaimResult, PreflightResult, TxConsequences};

fn ledger_rules(seed: u8) -> Rules {
    Rules::from_ledger(
        [feature_single_asset_vault()],
        Uint256::from_array([seed; 32]),
        std::iter::empty(),
    )
}

fn sample_preflight(
    tx: &'static str,
    rules: Rules,
    flags: ApplyFlags,
    journal: &'static str,
    ter: Ter,
) -> PreflightResult<&'static str, TxConsequences, &'static str, &'static str> {
    PreflightResult::new(
        tx,
        Some("batch"),
        rules,
        TxConsequences::new(12, SeqProxy::sequence(5)),
        flags,
        journal,
        ter,
    )
}

fn sample_queued(
    rules: Rules,
    flags: ApplyFlags,
) -> MaybeTx<&'static str, &'static str, &'static str, &'static str> {
    MaybeTx::new(
        Uint256::from_u64(9),
        44_u64,
        "acct",
        Some(99),
        SeqProxy::sequence(5),
        flags,
        sample_preflight("tx", rules, flags, "journal", Ter::TES_SUCCESS),
    )
}

#[test]
fn maybe_tx_refresh_preflight_if_needed_skips_when_rules_and_flags_match() {
    let rules = Rules::new(std::iter::empty());
    let mut queued = sample_queued(rules.clone(), ApplyFlags::RETRY);
    let debug_messages = RefCell::new(Vec::new());
    let reflighted = queued.refresh_preflight_if_needed(
        &rules,
        |message| debug_messages.borrow_mut().push(message),
        |_tx, _flags, _journal| unreachable!("matching rules should skip reflight"),
    );

    assert!(!reflighted);
    assert!(debug_messages.borrow().is_empty());
    assert_eq!(queued.pf_result.rules, rules);
    assert_eq!(queued.pf_result.flags, ApplyFlags::RETRY);
}

#[test]
fn maybe_tx_refresh_preflight_if_needed_reflights_when_rules_change() {
    let old_rules = ledger_rules(0x11);
    let new_rules = ledger_rules(0x22);
    let mut queued = sample_queued(old_rules, ApplyFlags::RETRY);
    let debug_messages = RefCell::new(Vec::new());

    let reflighted = queued.refresh_preflight_if_needed(
        &new_rules,
        |message| debug_messages.borrow_mut().push(message),
        |tx, flags, journal| {
            sample_preflight(tx, new_rules.clone(), flags, journal, Ter::TES_SUCCESS)
        },
    );

    assert!(reflighted);
    assert_eq!(
        debug_messages.into_inner(),
        vec![format!(
            "Queued transaction 0000000000000000000000000000000000000000000000000000000000000009 rules or flags have changed. Flags from {} to {}",
            ApplyFlags::RETRY,
            ApplyFlags::RETRY
        )]
    );
    assert_eq!(queued.pf_result.rules, new_rules);
}

#[test]
fn maybe_tx_refresh_preflight_if_needed_reflights_when_flags_change() {
    let rules = Rules::new(std::iter::empty());
    let mut queued = sample_queued(rules.clone(), ApplyFlags::RETRY);
    queued.flags = ApplyFlags::FAIL_HARD;
    let debug_messages = RefCell::new(Vec::new());

    let reflighted = queued.refresh_preflight_if_needed(
        &rules,
        |message| debug_messages.borrow_mut().push(message),
        |tx, flags, journal| sample_preflight(tx, rules.clone(), flags, journal, Ter::TES_SUCCESS),
    );

    assert!(reflighted);
    assert_eq!(queued.pf_result.flags, ApplyFlags::FAIL_HARD);
    assert_eq!(
        debug_messages.into_inner(),
        vec![format!(
            "Queued transaction 0000000000000000000000000000000000000000000000000000000000000009 rules or flags have changed. Flags from {} to {}",
            ApplyFlags::RETRY,
            ApplyFlags::FAIL_HARD
        )]
    );
}

#[test]
fn maybe_tx_apply_with_current_rules_reuses_preclaim_and_do_apply_after_optional_reflight() {
    let old_rules = ledger_rules(0x33);
    let new_rules = ledger_rules(0x44);
    let mut queued = sample_queued(old_rules, ApplyFlags::RETRY);
    let preflight_calls = Cell::new(0);
    let preclaim_calls = Cell::new(0);
    let apply_calls = Cell::new(0);
    let debug_messages = RefCell::new(Vec::new());

    let result = queued.apply_with_current_rules(
        &new_rules,
        |message| debug_messages.borrow_mut().push(message),
        |tx, flags, journal| {
            preflight_calls.set(preflight_calls.get() + 1);
            sample_preflight(tx, new_rules.clone(), flags, journal, Ter::TES_SUCCESS)
        },
        |preflight_result| {
            preclaim_calls.set(preclaim_calls.get() + 1);
            PreclaimResult::new(
                7,
                preflight_result.tx,
                preflight_result.parent_batch_id,
                preflight_result.flags,
                preflight_result.journal,
                Ter::TEC_CLAIM,
            )
        },
        |preclaim_result| {
            apply_calls.set(apply_calls.get() + 1);
            assert_eq!(preclaim_result.flags, ApplyFlags::RETRY);
            assert_eq!(preclaim_result.tx, "tx");
            ApplyResult::new(Ter::TEC_CLAIM, true, false)
        },
    );

    assert_eq!(preflight_calls.get(), 1);
    assert_eq!(preclaim_calls.get(), 1);
    assert_eq!(apply_calls.get(), 1);
    assert_eq!(queued.pf_result.rules, new_rules);
    assert_eq!(result, ApplyResult::new(Ter::TEC_CLAIM, true, false));
    assert_eq!(debug_messages.borrow().len(), 1);
}
