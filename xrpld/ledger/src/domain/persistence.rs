//! `LedgerPersistence` owner seam above the landed Rust `Ledger` load helpers.
//!
//! This keeps the the reference implementation decisions explicit:
//! - hash-router style duplicate-save suppression,
//! - pending-save gating for synchronous versus queued saves,
//! - queued current-vs-old validated save jobs,
//! - and the public load-by-index/hash/latest helpers.

use crate::{
    Ledger, LedgerConfig, LedgerHeader, LedgerInfoProvider, LedgerJournal, LedgerSetupError,
};
use basics::sha_map_hash::SHAMapHash;
use basics::tagged_cache::CacheClock;
use shamap::family::{FullBelowCache, MissingNodeReporter, SHAMapFamily, SHAMapNodeFetcher};
use std::hash::BuildHasher;
use std::sync::Arc;

pub type LedgerPersistenceJob = Box<dyn FnOnce() + Send + 'static>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LedgerPersistenceJobType {
    PubLedger,
    PubOldLedger,
}

/// How one validated-ledger save ended. The two failures are handled
/// differently, as in rippled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValidatedLedgerSave {
    Saved,
    /// The accepted ledger could not be built because nodes were missing
    /// (rippled `detail::saveValidatedLedger` returns `false` after
    /// `LedgerMaster::failedSave`). The SAVED flag stays set.
    MissingNodes,
    /// A database write failed (rippled: the write throws). Since #8662 the
    /// sequence leaves the complete range and both the pending-save mark and
    /// the SAVED flag are released, so a later save can run.
    WriteFailed,
}

pub trait LedgerPersistenceRuntime: Send + Sync + 'static {
    fn mark_saved(&self, hash: SHAMapHash) -> bool;
    fn start_work(&self, seq: u32) -> bool;
    fn finish_work(&self, seq: u32);
    fn should_work(&self, seq: u32, is_synchronous: bool) -> bool;
    fn pending(&self, seq: u32) -> bool;
    fn save_validated_ledger(&self, ledger: Arc<Ledger>, is_current: bool) -> ValidatedLedgerSave;
    /// rippled `LedgerMaster::failedSave`: retract the complete-ledger claim
    /// and reacquire the exact ledger. Runs when the accepted ledger had
    /// missing nodes, for synchronous and queued saves alike.
    fn failed_save(&self, _seq: u32, _hash: SHAMapHash) {}
    /// rippled `LedgerMaster::clearLedger`: stop advertising `seq` as complete.
    fn clear_ledger(&self, _seq: u32) {}
    /// rippled `HashRouter::clearFlags(hash, SAVED)`.
    fn clear_saved(&self, _hash: SHAMapHash) {}
    fn enqueue_job(
        &self,
        job_type: LedgerPersistenceJobType,
        job_name: String,
        job: LedgerPersistenceJob,
    ) -> bool;
}

pub struct LedgerPersistence {
    runtime: Arc<dyn LedgerPersistenceRuntime>,
}

impl LedgerPersistence {
    pub fn new(runtime: Arc<dyn LedgerPersistenceRuntime>) -> Self {
        Self { runtime }
    }

    pub fn runtime(&self) -> &Arc<dyn LedgerPersistenceRuntime> {
        &self.runtime
    }

    pub fn pend_save_validated(
        &self,
        ledger: Arc<Ledger>,
        is_synchronous: bool,
        is_current: bool,
    ) -> bool {
        pend_save_validated(
            Arc::clone(&self.runtime),
            ledger,
            is_synchronous,
            is_current,
        )
    }
}

pub fn pend_save_validated(
    runtime: Arc<dyn LedgerPersistenceRuntime>,
    ledger: Arc<Ledger>,
    is_synchronous: bool,
    is_current: bool,
) -> bool {
    if !runtime.mark_saved(ledger.header().hash)
        && (!is_synchronous || !runtime.pending(ledger.header().seq))
    {
        return true;
    }

    assert!(
        ledger.is_immutable(),
        "xrpl::pendSaveValidated : immutable ledger"
    );

    if !runtime.should_work(ledger.header().seq, is_synchronous) {
        return true;
    }

    if !is_synchronous {
        let runtime_for_job = Arc::clone(&runtime);
        let ledger_for_job = Arc::clone(&ledger);
        let job_type = if is_current {
            LedgerPersistenceJobType::PubLedger
        } else {
            LedgerPersistenceJobType::PubOldLedger
        };
        let job_name = format!("Pub{}", ledger.header().seq);
        if runtime.enqueue_job(
            job_type,
            job_name,
            Box::new(move || {
                let _ = save_validated_ledger(runtime_for_job, ledger_for_job, is_current);
            }),
        ) {
            return true;
        }
    }

    save_validated_ledger(runtime, ledger, is_current)
}

/// rippled `saveValidatedLedger` (LedgerPersistence.cpp).
fn save_validated_ledger(
    runtime: Arc<dyn LedgerPersistenceRuntime>,
    ledger: Arc<Ledger>,
    is_current: bool,
) -> bool {
    let seq = ledger.header().seq;
    if !runtime.start_work(seq) {
        return true;
    }

    let hash = ledger.header().hash;
    match runtime.save_validated_ledger(ledger, is_current) {
        ValidatedLedgerSave::Saved => {
            runtime.finish_work(seq);
            true
        }
        ValidatedLedgerSave::MissingNodes => {
            runtime.finish_work(seq);
            runtime.failed_save(seq, hash);
            false
        }
        ValidatedLedgerSave::WriteFailed => {
            // rippled #8662: the ledger's rows may be gone, so stop
            // advertising the sequence as complete. Then release the
            // in-progress mark and the SAVED flag in the reverse of the order
            // pend_save_validated takes them, or a concurrent enqueue could
            // re-arm the flag and dedupe the retry away. rippled rethrows the
            // database error here; quaxar reports `false` instead.
            runtime.clear_ledger(seq);
            runtime.finish_work(seq);
            runtime.clear_saved(hash);
            false
        }
    }
}

pub fn load_ledger_helper<CLOCK, S, FB, F, MR, NS, J>(
    info: LedgerHeader,
    acquire: bool,
    journal: &J,
    config: &LedgerConfig,
    family: &SHAMapFamily<CLOCK, S, FB, F, MR, NS>,
) -> Result<Option<Ledger>, LedgerSetupError>
where
    CLOCK: CacheClock,
    S: BuildHasher + Clone,
    FB: FullBelowCache,
    F: SHAMapNodeFetcher,
    MR: MissingNodeReporter,
    J: LedgerJournal,
{
    Ledger::load_immutable_with_family_and_config_or_none(info, acquire, journal, config, family)
}

pub fn get_latest_ledger<P, CLOCK, S, FB, F, MR, NS, J>(
    journal: &J,
    config: &LedgerConfig,
    family: &SHAMapFamily<CLOCK, S, FB, F, MR, NS>,
    provider: &P,
) -> Result<(Option<Ledger>, u32, SHAMapHash), LedgerSetupError>
where
    P: LedgerInfoProvider,
    CLOCK: CacheClock,
    S: BuildHasher + Clone,
    FB: FullBelowCache,
    F: SHAMapNodeFetcher,
    MR: MissingNodeReporter,
    J: LedgerJournal,
{
    Ledger::get_latest_ledger_with_provider_and_config(journal, config, family, provider)
}

pub fn load_by_index<P, CLOCK, S, FB, F, MR, NS, J>(
    ledger_index: u32,
    acquire: bool,
    journal: &J,
    config: &LedgerConfig,
    family: &SHAMapFamily<CLOCK, S, FB, F, MR, NS>,
    provider: &P,
) -> Result<Option<Ledger>, LedgerSetupError>
where
    P: LedgerInfoProvider,
    CLOCK: CacheClock,
    S: BuildHasher + Clone,
    FB: FullBelowCache,
    F: SHAMapNodeFetcher,
    MR: MissingNodeReporter,
    J: LedgerJournal,
{
    Ledger::load_by_index_with_provider_and_config_or_none(
        ledger_index,
        acquire,
        journal,
        config,
        family,
        provider,
    )
}

pub fn load_by_hash<P, CLOCK, S, FB, F, MR, NS, J>(
    ledger_hash: SHAMapHash,
    acquire: bool,
    journal: &J,
    config: &LedgerConfig,
    family: &SHAMapFamily<CLOCK, S, FB, F, MR, NS>,
    provider: &P,
) -> Result<Option<Ledger>, LedgerSetupError>
where
    P: LedgerInfoProvider,
    CLOCK: CacheClock,
    S: BuildHasher + Clone,
    FB: FullBelowCache,
    F: SHAMapNodeFetcher,
    MR: MissingNodeReporter,
    J: LedgerJournal,
{
    Ledger::load_by_hash_with_provider_and_config_or_none(
        ledger_hash,
        acquire,
        journal,
        config,
        family,
        provider,
    )
}
