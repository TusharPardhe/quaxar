//! Bounded `OpenLedger` owner shell for the current Rust app crate.
//!
//! This ports the reference owner control flow from `xrpld/app/ledger/OpenLedger.*`
//! without pretending the full `Ledger` / `OpenView` / `CachedSLEs` runtime is
//! already present in this crate. Callers provide the missing runtime seams:
//! view construction, duplicate checking, transaction application, local-queue
//! application, and relay decisions.
//!
//! The scope is intentionally narrow:
//! - publish-on-change `modify(...)`
//! - snapshot `current()`
//! - `accept(...)` ordering over retries, current transactions, modifier, and
//!   locals
//! - reference-matching retry pass constants and `apply_one(...)` result
//!   classification
//!
//! This does not claim parity for the missing concrete owners (`Ledger`,
//! `OpenView`, `TxQ`, `HashRouter`, `Overlay`, `CachedLedger`, `CachedSLEs`).

use std::{
    mem,
    sync::{Arc, Mutex},
};

use protocol::{Ter, is_tef_failure, is_tem_malformed};
use tx::{ApplyFlags, ApplyResult};

pub const LEDGER_TOTAL_PASSES: usize = 3;
pub const LEDGER_RETRY_PASSES: usize = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpenLedgerApplyDisposition {
    Success,
    Failure,
    Retry,
}

pub trait OpenLedgerTx: Clone {
    type Id: Clone;

    fn tx_id(&self) -> Self::Id;
}

pub trait OpenLedgerView<Tx: OpenLedgerTx>: Clone {
    fn tx_count(&self) -> usize;
    fn ordered_txs(&self) -> Vec<Tx>;
}

pub trait OpenLedgerRetries<Tx> {
    fn insert_retry(&mut self, tx: Tx);
    fn take_retries(&mut self) -> Vec<Tx>;
    fn restore_retries(&mut self, txs: Vec<Tx>);
    fn is_empty(&self) -> bool;
}

impl<Tx> OpenLedgerRetries<Tx> for Vec<Tx> {
    fn insert_retry(&mut self, tx: Tx) {
        self.push(tx);
    }

    fn take_retries(&mut self) -> Vec<Tx> {
        mem::take(self)
    }

    fn restore_retries(&mut self, txs: Vec<Tx>) {
        *self = txs;
    }

    fn is_empty(&self) -> bool {
        Vec::is_empty(self)
    }
}

pub struct OpenLedger<V> {
    modify_mutex: Mutex<()>,
    current: Mutex<Arc<V>>,
}

impl<V> OpenLedger<V> {
    pub fn new(current: V) -> Self {
        Self {
            modify_mutex: Mutex::new(()),
            current: Mutex::new(Arc::new(current)),
        }
    }

    pub fn current(&self) -> Arc<V> {
        self.current
            .lock()
            .expect("OpenLedger current lock poisoned")
            .clone()
    }
}

impl<V> OpenLedger<V> {
    pub fn modify<F>(&self, f: F) -> bool
    where
        V: Clone,
        F: FnOnce(&mut V) -> bool,
    {
        let _modify_lock = self
            .modify_mutex
            .lock()
            .expect("OpenLedger modify lock poisoned");
        let mut next = self.current();
        let changed = f(Arc::make_mut(&mut next));
        if changed {
            *self
                .current
                .lock()
                .expect("OpenLedger current lock poisoned") = next;
        }
        changed
    }

    pub fn accept<Tx, Retries, Check, Create, Apply, LocalApply, Modify, ShouldRelay, Relay, I>(
        &self,
        create: Create,
        check: &Check,
        locals: I,
        retries_first: bool,
        retries: &mut Retries,
        flags: ApplyFlags,
        apply: &mut Apply,
        apply_local: &mut LocalApply,
        modify: Option<Modify>,
        should_relay: &mut ShouldRelay,
        relay: &mut Relay,
    ) where
        V: Clone + OpenLedgerView<Tx>,
        Tx: OpenLedgerTx,
        Retries: OpenLedgerRetries<Tx>,
        Check: Fn(&Tx::Id) -> bool,
        Create: FnOnce() -> V,
        Apply: FnMut(&mut V, &Tx, ApplyFlags) -> ApplyResult,
        LocalApply: FnMut(&mut V, &Tx, ApplyFlags),
        Modify: FnOnce(&mut V) -> bool,
        ShouldRelay: FnMut(&Tx::Id) -> bool,
        Relay: FnMut(&Tx),
        I: IntoIterator<Item = Tx>,
    {
        let mut next = create();

        if retries_first {
            run_open_ledger_apply::<V, Tx, _, _, _, _>(
                &mut next,
                check,
                std::iter::empty::<Tx>(),
                retries,
                flags,
                apply,
            );
        }

        let _modify_lock = self
            .modify_mutex
            .lock()
            .expect("OpenLedger modify lock poisoned");
        let current_txs = self.current().ordered_txs();

        run_open_ledger_apply(&mut next, check, current_txs, retries, flags, apply);

        if let Some(modify) = modify {
            let _ = modify(&mut next);
        }

        for tx in locals {
            apply_local(&mut next, &tx, flags);
        }

        for tx in next.ordered_txs() {
            let tx_id = tx.tx_id();
            if should_relay(&tx_id) {
                relay(&tx);
            }
        }

        *self
            .current
            .lock()
            .expect("OpenLedger current lock poisoned") = Arc::new(next);
    }
}

impl<V> OpenLedger<V> {
    pub fn empty<Tx>(&self) -> bool
    where
        V: OpenLedgerView<Tx>,
        Tx: OpenLedgerTx,
    {
        let _modify_lock = self
            .modify_mutex
            .lock()
            .expect("OpenLedger modify lock poisoned");
        self.current().tx_count() == 0
    }
}

pub fn classify_open_ledger_apply_result(result: ApplyResult) -> OpenLedgerApplyDisposition {
    if result.applied || result.ter == Ter::TER_QUEUED {
        return OpenLedgerApplyDisposition::Success;
    }
    if is_tef_failure(result.ter)
        || is_tem_malformed(result.ter)
        || is_open_ledger_tel_local(result.ter)
    {
        return OpenLedgerApplyDisposition::Failure;
    }
    OpenLedgerApplyDisposition::Retry
}

pub fn apply_one_open_ledger<V, Tx, Apply>(
    view: &mut V,
    tx: &Tx,
    retry: bool,
    flags: ApplyFlags,
    apply: &mut Apply,
) -> OpenLedgerApplyDisposition
where
    Apply: FnMut(&mut V, &Tx, ApplyFlags) -> ApplyResult,
{
    let mut effective_flags = flags;
    if retry {
        effective_flags |= ApplyFlags::RETRY;
    }

    classify_open_ledger_apply_result(apply(view, tx, effective_flags))
}

pub fn run_open_ledger_apply<V, Tx, Retries, Check, Apply, Txs>(
    view: &mut V,
    check: &Check,
    txs: Txs,
    retries: &mut Retries,
    flags: ApplyFlags,
    apply: &mut Apply,
) where
    Tx: OpenLedgerTx,
    Retries: OpenLedgerRetries<Tx>,
    Check: Fn(&Tx::Id) -> bool,
    Apply: FnMut(&mut V, &Tx, ApplyFlags) -> ApplyResult,
    Txs: IntoIterator<Item = Tx>,
{
    for tx in txs {
        let tx_id = tx.tx_id();
        if check(&tx_id) {
            continue;
        }

        if apply_one_open_ledger(view, &tx, true, flags, apply) == OpenLedgerApplyDisposition::Retry
        {
            retries.insert_retry(tx);
        }
    }

    let mut retry = true;
    for pass in 0..LEDGER_TOTAL_PASSES {
        let mut changes = 0;
        let pending = retries.take_retries();
        let mut remaining = Vec::new();

        for tx in pending {
            match apply_one_open_ledger(view, &tx, retry, flags, apply) {
                OpenLedgerApplyDisposition::Success => {
                    changes += 1;
                }
                OpenLedgerApplyDisposition::Failure => {}
                OpenLedgerApplyDisposition::Retry => remaining.push(tx),
            }
        }

        retries.restore_retries(remaining);

        if changes == 0 && !retry {
            return;
        }

        if changes == 0 || pass >= LEDGER_RETRY_PASSES {
            retry = false;
        }
    }

    debug_assert!(retries.is_empty() || !retry);
}

const fn is_open_ledger_tel_local(code: Ter) -> bool {
    code.to_int() < Ter::TEM_MALFORMED.to_int()
}
