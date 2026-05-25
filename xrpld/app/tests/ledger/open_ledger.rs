use std::{cell::RefCell, collections::BTreeMap, sync::Arc};

use app::{
    LEDGER_RETRY_PASSES, LEDGER_TOTAL_PASSES, OpenLedger, OpenLedgerApplyDisposition, OpenLedgerTx,
    OpenLedgerView, apply_one_open_ledger, classify_open_ledger_apply_result,
    run_open_ledger_apply,
};
use protocol::Ter;
use tx::{ApplyFlags, ApplyResult};

#[derive(Clone, Debug, PartialEq, Eq)]
struct StubTx {
    id: u32,
}

impl StubTx {
    const fn new(id: u32) -> Self {
        Self { id }
    }
}

impl OpenLedgerTx for StubTx {
    type Id = u32;

    fn tx_id(&self) -> Self::Id {
        self.id
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct StubView {
    txs: Vec<StubTx>,
    label: &'static str,
}

impl StubView {
    fn empty(label: &'static str) -> Self {
        Self {
            txs: Vec::new(),
            label,
        }
    }
}

impl OpenLedgerView<StubTx> for StubView {
    fn tx_count(&self) -> usize {
        self.txs.len()
    }

    fn ordered_txs(&self) -> Vec<StubTx> {
        self.txs.clone()
    }
}

#[test]
fn openledger_constants_match_current_cpp() {
    assert_eq!(LEDGER_TOTAL_PASSES, 3);
    assert_eq!(LEDGER_RETRY_PASSES, 1);
}

#[test]
fn openledger_apply_result_classification_buckets() {
    assert_eq!(
        classify_open_ledger_apply_result(ApplyResult::new(Ter::TES_SUCCESS, true, false)),
        OpenLedgerApplyDisposition::Success
    );
    assert_eq!(
        classify_open_ledger_apply_result(ApplyResult::new(Ter::TER_QUEUED, false, false)),
        OpenLedgerApplyDisposition::Success
    );
    assert_eq!(
        classify_open_ledger_apply_result(ApplyResult::new(Ter::TEF_FAILURE, false, false)),
        OpenLedgerApplyDisposition::Failure
    );
    assert_eq!(
        classify_open_ledger_apply_result(ApplyResult::new(Ter::TEM_MALFORMED, false, false)),
        OpenLedgerApplyDisposition::Failure
    );
    assert_eq!(
        classify_open_ledger_apply_result(ApplyResult::new(Ter::TEL_LOCAL_ERROR, false, false)),
        OpenLedgerApplyDisposition::Failure
    );
    assert_eq!(
        classify_open_ledger_apply_result(ApplyResult::new(Ter::TEC_CLAIM, false, false)),
        OpenLedgerApplyDisposition::Retry
    );
}

#[test]
fn openledger_apply_one_adds_retry_flag_only_when_requested() {
    let seen = RefCell::new(Vec::new());
    let mut apply = |_: &mut StubView, tx: &StubTx, flags: ApplyFlags| {
        seen.borrow_mut().push((tx.id, flags.bits()));
        ApplyResult::new(Ter::TES_SUCCESS, true, false)
    };
    let mut view = StubView::empty("next");

    let first = apply_one_open_ledger(
        &mut view,
        &StubTx::new(1),
        true,
        ApplyFlags::FAIL_HARD,
        &mut apply,
    );
    let second = apply_one_open_ledger(
        &mut view,
        &StubTx::new(2),
        false,
        ApplyFlags::FAIL_HARD,
        &mut apply,
    );

    assert_eq!(first, OpenLedgerApplyDisposition::Success);
    assert_eq!(second, OpenLedgerApplyDisposition::Success);
    assert_eq!(
        seen.into_inner(),
        vec![
            (1, (ApplyFlags::FAIL_HARD | ApplyFlags::RETRY).bits()),
            (2, ApplyFlags::FAIL_HARD.bits()),
        ]
    );
}

#[test]
fn openledger_apply_skips_transactions_already_in_closed_ledger() {
    let mut retries = Vec::new();
    let seen = RefCell::new(Vec::new());
    let mut apply = |view: &mut StubView, tx: &StubTx, _flags: ApplyFlags| {
        view.txs.push(tx.clone());
        seen.borrow_mut().push(tx.id);
        ApplyResult::new(Ter::TES_SUCCESS, true, false)
    };
    let mut view = StubView::empty("next");

    run_open_ledger_apply(
        &mut view,
        &|tx_id: &u32| *tx_id == 2,
        vec![StubTx::new(1), StubTx::new(2), StubTx::new(3)],
        &mut retries,
        ApplyFlags::NONE,
        &mut apply,
    );

    assert_eq!(seen.into_inner(), vec![1, 3]);
    assert_eq!(view.txs, vec![StubTx::new(1), StubTx::new(3)]);
    assert!(retries.is_empty());
}

#[test]
fn openledger_apply_runs_final_non_retry_pass_after_retry_passes() {
    let mut retries = vec![StubTx::new(10), StubTx::new(20)];
    let seen_flags = RefCell::new(BTreeMap::<u32, Vec<u32>>::new());
    let mut apply = |view: &mut StubView, tx: &StubTx, flags: ApplyFlags| {
        seen_flags
            .borrow_mut()
            .entry(tx.id)
            .or_default()
            .push(flags.bits());

        if tx.id == 10 {
            view.txs.push(tx.clone());
            return ApplyResult::new(Ter::TES_SUCCESS, true, false);
        }

        if (flags & ApplyFlags::RETRY).bits() != 0 {
            return ApplyResult::new(Ter::TER_RETRY, false, false);
        }

        view.txs.push(tx.clone());
        ApplyResult::new(Ter::TES_SUCCESS, true, false)
    };
    let mut view = StubView::empty("next");

    run_open_ledger_apply(
        &mut view,
        &|_: &u32| false,
        std::iter::empty::<StubTx>(),
        &mut retries,
        ApplyFlags::NONE,
        &mut apply,
    );

    let seen = seen_flags.into_inner();
    assert_eq!(seen.get(&10), Some(&vec![ApplyFlags::RETRY.bits()]));
    assert_eq!(
        seen.get(&20),
        Some(&vec![
            ApplyFlags::RETRY.bits(),
            ApplyFlags::RETRY.bits(),
            ApplyFlags::NONE.bits(),
        ])
    );
    assert_eq!(view.txs, vec![StubTx::new(10), StubTx::new(20)]);
    assert!(retries.is_empty());
}

#[test]
fn openledger_modify_only_publishes_when_callback_reports_change() {
    let owner = OpenLedger::new(StubView {
        txs: vec![StubTx::new(1)],
        label: "current",
    });
    let before = owner.current();

    let changed = owner.modify(|next| {
        next.txs.push(StubTx::new(2));
        false
    });

    let after = owner.current();
    assert!(!changed);
    assert!(Arc::ptr_eq(&before, &after));
    assert_eq!(after.txs, vec![StubTx::new(1)]);
}

#[test]
fn openledger_modify_publishes_new_snapshot_when_callback_changes_view() {
    let owner = OpenLedger::new(StubView {
        txs: vec![StubTx::new(1)],
        label: "current",
    });
    let before = owner.current();

    let changed = owner.modify(|next| {
        next.txs.push(StubTx::new(2));
        true
    });

    let after = owner.current();
    assert!(changed);
    assert!(!Arc::ptr_eq(&before, &after));
    assert_eq!(before.txs, vec![StubTx::new(1)]);
    assert_eq!(after.txs, vec![StubTx::new(1), StubTx::new(2)]);
}

#[test]
fn openledger_empty_reflects_current_snapshot_tx_count() {
    let empty = OpenLedger::new(StubView::empty("empty"));
    let non_empty = OpenLedger::new(StubView {
        txs: vec![StubTx::new(1)],
        label: "non-empty",
    });

    assert!(empty.empty::<StubTx>());
    assert!(!non_empty.empty::<StubTx>());
}

#[test]
fn openledger_accept_preservesing_for_retries_current_modifier_locals_and_relay() {
    let owner = OpenLedger::new(StubView {
        txs: vec![StubTx::new(2)],
        label: "current",
    });
    let mut retries = vec![StubTx::new(1)];
    let calls = RefCell::new(Vec::new());
    let mut apply = |view: &mut StubView, tx: &StubTx, flags: ApplyFlags| {
        calls
            .borrow_mut()
            .push(format!("apply:{}:{}", tx.id, flags.bits()));
        view.txs.push(tx.clone());
        ApplyResult::new(Ter::TES_SUCCESS, true, false)
    };
    let mut apply_local = |view: &mut StubView, tx: &StubTx, flags: ApplyFlags| {
        calls
            .borrow_mut()
            .push(format!("local:{}:{}", tx.id, flags.bits()));
        view.txs.push(tx.clone());
    };
    let mut should_relay = |tx_id: &u32| {
        calls.borrow_mut().push(format!("should:{tx_id}"));
        *tx_id != 3
    };
    let mut relay = |tx: &StubTx| {
        calls.borrow_mut().push(format!("relay:{}", tx.id));
    };

    owner.accept(
        || {
            calls.borrow_mut().push("create".to_string());
            StubView::empty("next")
        },
        &|_: &u32| false,
        vec![StubTx::new(3)],
        true,
        &mut retries,
        ApplyFlags::FAIL_HARD,
        &mut apply,
        &mut apply_local,
        Some(|view: &mut StubView| {
            calls.borrow_mut().push("modify".to_string());
            view.label = "modified";
            true
        }),
        &mut should_relay,
        &mut relay,
    );

    assert!(retries.is_empty());
    assert_eq!(
        calls.into_inner(),
        vec![
            "create".to_string(),
            format!(
                "apply:1:{}",
                (ApplyFlags::FAIL_HARD | ApplyFlags::RETRY).bits()
            ),
            format!(
                "apply:2:{}",
                (ApplyFlags::FAIL_HARD | ApplyFlags::RETRY).bits()
            ),
            "modify".to_string(),
            format!("local:3:{}", ApplyFlags::FAIL_HARD.bits()),
            "should:1".to_string(),
            "relay:1".to_string(),
            "should:2".to_string(),
            "relay:2".to_string(),
            "should:3".to_string(),
        ]
    );

    let current = owner.current();
    assert_eq!(current.label, "modified");
    assert_eq!(
        current.txs,
        vec![StubTx::new(1), StubTx::new(2), StubTx::new(3)]
    );
}
