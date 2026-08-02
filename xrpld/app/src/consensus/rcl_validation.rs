//! RCL-specific validation types. Ported from `RCLValidations.h`.

use std::sync::Arc;

use basics::base_uint::Uint256;
use basics::chrono::NetClockTimePoint;
use consensus::model::TrieLedger;
use consensus::rcl_support::{ValidationT, ValidationsLedger};
use ledger::{Ledger, LedgerJournal, NullLedgerJournal};
use protocol::{NodeID, PublicKey, STValidation, get_field_by_symbol};

use crate::job::{job_queue::JobQueue, job_types::JobType};

#[derive(Clone)]
pub struct RclValidation {
    val: Arc<STValidation>,
}

impl RclValidation {
    pub fn new(val: Arc<STValidation>) -> Self {
        Self { val }
    }

    pub fn unwrap_arc(&self) -> Arc<STValidation> {
        Arc::clone(&self.val)
    }

    fn cookie(&self) -> u64 {
        u64::from(self.val.get_field_u32(get_field_by_symbol("sfCookie")))
    }

    fn load_fee(&self) -> Option<u32> {
        let field = get_field_by_symbol("sfLoadFee");
        self.val
            .is_field_present(field)
            .then(|| self.val.get_field_u32(field))
    }
}

impl ValidationT for RclValidation {
    type LedgerId = Uint256;
    type Seq = u32;
    type NodeId = NodeID;
    type NodeKey = PublicKey;
    type Wrapped = Arc<STValidation>;

    fn ledger_id(&self) -> Uint256 {
        self.val.get_ledger_hash()
    }

    fn seq(&self) -> u32 {
        self.val
            .get_field_u32(get_field_by_symbol("sfLedgerSequence"))
    }

    fn sign_time(&self) -> NetClockTimePoint {
        NetClockTimePoint::new(self.val.get_sign_time())
    }

    fn seen_time(&self) -> NetClockTimePoint {
        NetClockTimePoint::new(self.val.get_seen_time())
    }

    fn key(&self) -> PublicKey {
        *self.val.get_signer_public()
    }

    fn trusted(&self) -> bool {
        self.val.is_trusted()
    }

    fn set_trusted(&mut self) {
        Arc::make_mut(&mut self.val).set_trusted();
    }

    fn set_untrusted(&mut self) {
        Arc::make_mut(&mut self.val).set_untrusted();
    }

    fn full(&self) -> bool {
        self.val.is_full()
    }

    fn node_id(&self) -> NodeID {
        self.val.get_node_id()
    }

    fn load_fee(&self) -> Option<u32> {
        RclValidation::load_fee(self)
    }

    fn cookie(&self) -> u64 {
        RclValidation::cookie(self)
    }

    fn unwrap(self) -> Arc<STValidation> {
        self.val
    }
}

const MAX_ANCESTORS_TRACKED: u32 = 256;

#[derive(Clone)]
pub struct RclValidatedLedger {
    ledger_id: Uint256,
    ledger_seq: u32,
    ancestors: Arc<Vec<Uint256>>,
}

impl RclValidatedLedger {
    pub fn genesis() -> Self {
        Self {
            ledger_id: Uint256::zero(),
            ledger_seq: 0,
            ancestors: Arc::new(vec![Uint256::zero()]),
        }
    }

    pub fn from_ledger(ledger: &Ledger) -> Self {
        Self::from_ledger_with_journal(ledger, &NullLedgerJournal)
    }

    pub fn from_ledger_with_journal<J: LedgerJournal>(ledger: &Ledger, journal: &J) -> Self {
        let header = ledger.header();
        let ledger_seq = header.seq;
        let ledger_id = *header.hash.as_uint256();

        let min_seq = ledger_seq.saturating_sub(MAX_ANCESTORS_TRACKED.min(ledger_seq));
        let mut ancestors = Vec::with_capacity((ledger_seq - min_seq + 1) as usize);
        for seq in min_seq..=ledger_seq {
            let hash = ledger
                .hash_of_seq(seq, journal)
                .map(|h| *h.as_uint256())
                .unwrap_or_else(Uint256::zero);
            ancestors.push(hash);
        }

        Self {
            ledger_id,
            ledger_seq,
            ancestors: Arc::new(ancestors),
        }
    }

    fn min_seq(&self) -> u32 {
        self.ledger_seq + 1 - self.ancestors.len() as u32
    }
}

impl TrieLedger for RclValidatedLedger {
    type Seq = u32;
    type Id = Uint256;

    fn genesis() -> Self {
        RclValidatedLedger::genesis()
    }

    fn seq(&self) -> u32 {
        self.ledger_seq
    }

    fn ancestor(&self, s: u32) -> Uint256 {
        if s > self.ledger_seq {
            return Uint256::zero();
        }
        if s == self.ledger_seq {
            return self.ledger_id;
        }
        let min_seq = self.min_seq();
        if s < min_seq {
            return Uint256::zero();
        }
        self.ancestors[(s - min_seq) as usize]
    }

    fn mismatch(&self, other: &Self) -> u32 {
        let max_check = self.ledger_seq.min(other.ledger_seq) + 1;
        let mut lo = 0u32;
        let mut hi = max_check;
        while lo < hi {
            let mid = lo + (hi - lo) / 2;
            if self.ancestor(mid) == other.ancestor(mid) {
                lo = mid + 1;
            } else {
                hi = mid;
            }
        }
        lo
    }
}

impl ValidationsLedger for RclValidatedLedger {
    fn id(&self) -> Uint256 {
        self.ledger_id
    }
}

pub struct RclValidationsAdaptor {
    ledgers: parking_lot::Mutex<std::collections::HashMap<Uint256, RclValidatedLedger>>,
    now: Arc<dyn Fn() -> NetClockTimePoint + Send + Sync>,
    ledger_master_runtime: parking_lot::Mutex<
        Option<Arc<crate::ledger::ledger_master_runtime::AppLedgerMasterRuntime>>,
    >,
    /// The application-owned cache/provider/closed-slot lookup path. This is
    /// intentionally separate from `ledger_master_runtime`, whose purpose is
    /// inbound acquisition after a durable exact-hash miss.
    loaded_ledger_runtime: parking_lot::Mutex<
        Option<Arc<crate::ledger::loaded_ledger_runtime::AppLoadedLedgerRuntime>>,
    >,
    overlay: parking_lot::Mutex<Option<Arc<overlay::runtime::overlay_impl::OverlayImpl>>>,
    job_queue: parking_lot::Mutex<Option<Arc<JobQueue>>>,
}

impl RclValidationsAdaptor {
    /// Construct an adaptor with the given network-time source. The caller
    /// supplies this because the generic `Validations` tracker has no access
    /// to the application's clock — rippled's `RCLValidationsAdaptor` reads
    /// the same clock the rest of the node's networking layer uses, so the
    /// app layer must inject it here.
    pub fn new(now: impl Fn() -> NetClockTimePoint + Send + Sync + 'static) -> Self {
        Self {
            ledgers: parking_lot::Mutex::new(std::collections::HashMap::new()),
            now: Arc::new(now),
            ledger_master_runtime: parking_lot::Mutex::new(None),
            loaded_ledger_runtime: parking_lot::Mutex::new(None),
            overlay: parking_lot::Mutex::new(None),
            job_queue: parking_lot::Mutex::new(None),
        }
    }

    pub fn register_ledger(&self, ledger: &Ledger) {
        let wrapped = RclValidatedLedger::from_ledger(ledger);
        self.ledgers.lock().insert(wrapped.id(), wrapped);
    }

    /// Attach (or detach) the ledger master runtime this adaptor consults
    /// on a cache miss in `acquire`, matching the reference's
    /// `RCLValidationsAdaptor` holding a reference to the owning
    /// `Application` for `app_.getLedgerMaster()`/`app_.getInboundLedgers()`
    /// access.
    pub fn set_ledger_master_runtime(
        &self,
        runtime: Option<Arc<crate::ledger::ledger_master_runtime::AppLedgerMasterRuntime>>,
    ) {
        *self.ledger_master_runtime.lock() = runtime;
    }

    /// Attach the application-owned exact-hash lookup path. In a configured
    /// node it performs the same LedgerMaster lookup rippled's adaptor calls:
    /// history cache, durable provider reload, then the current closed-ledger
    /// slot.
    pub fn set_loaded_ledger_runtime(
        &self,
        runtime: Option<Arc<crate::ledger::loaded_ledger_runtime::AppLoadedLedgerRuntime>>,
    ) {
        *self.loaded_ledger_runtime.lock() = runtime;
    }

    /// Attach (or detach) the overlay so `acquire` can resolve a ledger
    /// sequence number from peers when the local cache does not have it.
    pub fn set_overlay(&self, overlay: Option<Arc<overlay::runtime::overlay_impl::OverlayImpl>>) {
        *self.overlay.lock() = overlay;
    }

    /// Attach the application job queue used for rippled-equivalent
    /// `GetConsL2` cache-miss acquisition jobs.
    pub fn set_job_queue(&self, job_queue: Option<Arc<JobQueue>>) {
        *self.job_queue.lock() = job_queue;
    }
}

impl consensus::rcl_support::ValidationsAdaptor for RclValidationsAdaptor {
    type Ledger = RclValidatedLedger;
    type Validation = RclValidation;

    fn now(&self) -> NetClockTimePoint {
        (self.now)()
    }

    fn acquire(&self, ledger_id: &Uint256) -> Option<RclValidatedLedger> {
        if let Some(ledger) = self.ledgers.lock().get(ledger_id).cloned() {
            return Some(ledger);
        }

        // Matches the reference's `RCLValidationsAdaptor::acquire`: after the
        // adaptor-local map misses, consult the application-owned
        // LedgerMaster path (history cache -> durable provider reload ->
        // current closed-ledger slot). Only a complete exact-hash miss dispatches
        // `GetConsL2`/`InboundLedgers::acquireAsync`, rather than silently
        // giving up. This is the third of the reference's three redundant
        // acquisition triggers (the other two being
        // `Consensus::checkLedger`'s `acquireLedger` and `InboundLedger`'s
        // own retry timer). `Validations::updateTrie` calls this adaptor
        // method every time a new TRUSTED validation references a ledger not
        // yet available, so a cache-only lookup left the trie unable to pull
        // in persisted ledger ancestry after restart or cache eviction.
        let runtime = { self.ledger_master_runtime.lock().clone() };
        let Some(runtime) = runtime else {
            return None;
        };

        let hash = basics::sha_map_hash::SHAMapHash::new(*ledger_id);
        let loaded_runtime = { self.loaded_ledger_runtime.lock().clone() };
        if let Some(loaded_runtime) = loaded_runtime {
            // Match rippled RCLValidationsAdaptor::acquire exactly: call the
            // application LedgerMaster lookup before requesting an inbound
            // ledger. AppLoadedLedgerRuntime performs cache -> provider/NuDB
            // reload -> current closed slot and canonicalizes a provider hit in
            // LedgerHistory. Do not hold an adaptor mutex during this I/O.
            match loaded_runtime.get_history_ledger_by_hash(hash) {
                Ok(Some(ledger)) => return Some(RclValidatedLedger::from_ledger(&ledger)),
                Ok(None) => {}
                Err(error) => {
                    tracing::warn!(target: "consensus", %ledger_id, ?error,
                        "provider-backed exact-hash ledger lookup failed before consensus acquisition");
                }
            }
        } else if let Some(ledger) = runtime.ledger_master().get_ledger_by_hash(hash) {
            // Preserve the cache/closed-slot behavior for isolated users that
            // have wired an inbound runtime but no application storage runtime.
            return Some(RclValidatedLedger::from_ledger(&ledger));
        }

        let requested_hash = *ledger_id;
        let acquire = move || {
            if let Some(guard) = runtime.inbound_ledgers.lock().ok()
                && let Some(shared) = guard.as_ref()
            {
                // This path has only a ledger hash. A peer's history range is
                // not an authoritative hash-to-sequence binding, so acquire by
                // hash and learn the sequence from the response header.
                shared.acquire_closed_ledger_async(
                    requested_hash,
                    crate::ledger::inbound_ledgers::AcquireReason::Consensus,
                );
            }
        };
        if let Some(job_queue) = self.job_queue.lock().clone() {
            // Match rippled RCLValidationsAdaptor::acquire: cache-miss
            // recovery runs as a JtAdvance "GetConsL2" job, not inline in
            // validation trie maintenance.
            if !job_queue.add_job(JobType::JtAdvance, "GetConsL2", acquire) {
                tracing::debug!(target: "consensus", %ledger_id, "GetConsL2 rejected because job queue is stopping");
            }
        } else {
            // The queue is attached during application runtime wiring. Keep
            // the adaptor useful in isolated construction/tests before that
            // wiring has occurred.
            acquire();
        }
        None
    }
}

impl consensus::rcl::AsValidationKey<RclValidationsAdaptor> for Arc<STValidation> {
    fn node_key(&self) -> PublicKey {
        *self.get_signer_public()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use basics::basic_config::BasicConfig;
    use basics::sha_map_hash::SHAMapHash;
    use basics::tagged_cache::MonotonicClock;
    use ledger::{Ledger as LedgerImpl, LedgerHeader, LedgerMasterConfig, calculate_ledger_hash};
    use nodestore::{DummyScheduler, ManagerImp, NullJournal, Scheduler};
    use protocol::{
        KeyType, SecretKey, calc_node_id, derive_public_key, generate_secret_key, random_seed,
    };
    use std::time::Duration;
    use tempfile::TempDir;
    use xrpld_core::{DatabaseCon, LEDGER_DB_INIT};

    fn memory_node_store() -> (TempDir, crate::SHAMapStoreNodeStore) {
        let temp = TempDir::new().expect("tempdir");
        let mut config = BasicConfig::new();
        config.set_legacy("database_path", temp.path().join("sql").to_string_lossy());
        let node_db = config.section_mut("node_db");
        node_db.set("type", "Memory");
        node_db.set("path", temp.path().join("node").to_string_lossy());

        let bootstrap = crate::shamap::shamap_store_bootstrap::bootstrap_shamap_store(
            &config,
            false,
            128,
            1,
            8,
            64,
            2,
            &ManagerImp::new(),
            Arc::new(DummyScheduler) as Arc<dyn Scheduler>,
            Arc::new(NullJournal),
        )
        .expect("memory node store");
        (temp, bootstrap.node_store)
    }

    fn insert_ledger_header(db: &DatabaseCon, header: LedgerHeader) {
        db.get_session()
            .execute(
                "INSERT INTO Ledgers (LedgerHash, LedgerSeq, PrevHash, TotalCoins, ClosingTime, PrevClosingTime, CloseTimeRes, CloseFlags, AccountSetHash, TransSetHash) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                rusqlite::params![
                    header.hash.as_uint256().to_string(),
                    i64::from(header.seq),
                    header.parent_hash.as_uint256().to_string(),
                    i64::try_from(header.drops).expect("drops fit SQLite integer"),
                    i64::from(header.close_time),
                    i64::from(header.parent_close_time),
                    i64::from(header.close_time_resolution),
                    i64::from(header.close_flags),
                    header.account_hash.as_uint256().to_string(),
                    header.tx_hash.as_uint256().to_string(),
                ],
            )
            .expect("ledger header insert");
    }

    fn signed_validation(ledger_hash: Uint256, seq: u32, sign_time: u32) -> Arc<STValidation> {
        let seed = random_seed();
        let secret_key: SecretKey = generate_secret_key(KeyType::Secp256k1, &seed)
            .expect("secret key generation should succeed");
        let public_key = derive_public_key(KeyType::Secp256k1, &secret_key)
            .expect("public key derivation should succeed");
        let node_id = calc_node_id(&public_key);

        let val = STValidation::new_signed(sign_time, &public_key, node_id, &secret_key, |v| {
            v.set_field_h256(get_field_by_symbol("sfLedgerHash"), ledger_hash);
            v.set_field_u32(get_field_by_symbol("sfLedgerSequence"), seq);
        })
        .expect("validation signing should succeed");
        Arc::new(val)
    }

    #[test]
    fn rcl_validation_exposes_ledger_hash_and_seq() {
        let hash = Uint256::from_slice(&[7; 32]).unwrap();
        let val = RclValidation::new(signed_validation(hash, 42, 1000));

        assert_eq!(val.ledger_id(), hash);
        assert_eq!(ValidationT::seq(&val), 42);
        assert!(val.trusted());
    }

    #[test]
    fn rcl_validation_cookie_defaults_to_zero() {
        let hash = Uint256::from_slice(&[7; 32]).unwrap();
        let val = RclValidation::new(signed_validation(hash, 1, 1000));
        assert_eq!(ValidationT::cookie(&val), 0);
    }

    #[test]
    fn rcl_validated_ledger_ancestor_lookups_match_genesis_and_self() {
        let ledger = LedgerImpl::from_ledger_seq_and_close_time(5, 500, false);
        let wrapped = RclValidatedLedger::from_ledger(&ledger);

        assert_eq!(TrieLedger::seq(&wrapped), 5);
        assert_eq!(wrapped.ancestor(5), wrapped.ledger_id);
    }

    #[test]
    fn rcl_validations_adaptor_acquires_registered_ledgers() {
        let adaptor = RclValidationsAdaptor::new(|| NetClockTimePoint::new(1000));
        let ledger = LedgerImpl::from_ledger_seq_and_close_time(3, 300, false);
        adaptor.register_ledger(&ledger);

        let id = *ledger.header().hash.as_uint256();
        assert!(consensus::rcl_support::ValidationsAdaptor::acquire(&adaptor, &id).is_some());
    }

    #[test]
    fn rcl_validations_adaptor_reloads_persisted_hash_after_local_and_cache_misses() {
        let (temp, node_store) = memory_node_store();
        let ledger_db = Arc::new(
            DatabaseCon::new_at_path(temp.path(), "ledger.db", &[], LEDGER_DB_INIT)
                .expect("ledger db"),
        );
        let relational = Arc::new(crate::SqliteSHAMapStoreRelational::new(
            Arc::clone(&ledger_db),
            None,
            false,
            100,
            Duration::from_secs(0),
        ));

        // Empty state and transaction roots require no fetched SHAMap node,
        // so this header can be loaded exclusively from the persisted ledger
        // table while the cache, closed slot, and adaptor map all remain empty.
        let header = LedgerHeader {
            seq: 777,
            drops: 1_000_000,
            parent_hash: SHAMapHash::new(Uint256::from_u64(776)),
            close_time: 1_777,
            parent_close_time: 1_767,
            close_time_resolution: 10,
            close_flags: 0,
            ..LedgerHeader::default()
        };
        let header = LedgerHeader {
            hash: calculate_ledger_hash(&header),
            ..header
        };
        insert_ledger_header(&ledger_db, header);

        let mut root = crate::ApplicationRoot::new(0).expect("application root");
        let ledger_master = Arc::new(crate::AppLedgerMaster::new(
            MonotonicClock::default(),
            LedgerMasterConfig::default(),
        ));
        let master_runtime = Arc::new(crate::AppLedgerMasterRuntime::with_ledger_master(
            Arc::clone(&ledger_master),
        ));
        let _ = root.attach_ledger_master_runtime(Arc::clone(&master_runtime));
        // The real LCL lives in ApplicationRoot's shared ledger-master state,
        // not in the wrapped AppLedgerMaster slot. Give it the same hash as
        // the persisted header: successful lookup must still populate history,
        // proving provider reload happens before the closed-slot fallback.
        root.on_closed_ledger(Arc::new(LedgerImpl::from_header_hashes(header)));
        let lookup_runtime = Arc::new(
            crate::AppLoadedLedgerRuntime::with_sources_and_ledger_master_state(
                Arc::clone(&ledger_master),
                Some(relational),
                Some(node_store),
                Some(root.ledger_master_state()),
            ),
        );
        let adaptor = RclValidationsAdaptor::new(|| NetClockTimePoint::new(1000));
        adaptor.set_ledger_master_runtime(Some(master_runtime));
        adaptor.set_loaded_ledger_runtime(Some(lookup_runtime));

        assert!(ledger_master.get_ledger_by_hash(header.hash).is_none());
        assert!(
            ledger_master
                .ledger_history()
                .get_cached_ledger_by_hash(header.hash)
                .is_none()
        );
        let loaded =
            consensus::rcl_support::ValidationsAdaptor::acquire(&adaptor, header.hash.as_uint256())
                .expect("provider-backed exact-hash lookup must resolve the persisted ledger");

        assert_eq!(loaded.ledger_id, *header.hash.as_uint256());
        assert_eq!(loaded.ledger_seq, header.seq);
        assert!(
            ledger_master
                .ledger_history()
                .get_cached_ledger_by_hash(header.hash)
                .is_some()
        );
    }

    #[test]
    fn rcl_validations_adaptor_uses_application_roots_current_closed_ledger_on_durable_miss() {
        let mut root = crate::ApplicationRoot::new(0).expect("application root");
        let ledger_master = Arc::new(crate::AppLedgerMaster::new(
            MonotonicClock::default(),
            LedgerMasterConfig::default(),
        ));
        let master_runtime = Arc::new(crate::AppLedgerMasterRuntime::with_ledger_master(
            Arc::clone(&ledger_master),
        ));
        let _ = root.attach_ledger_master_runtime(Arc::clone(&master_runtime));

        let closed = Arc::new(LedgerImpl::from_ledger_seq_and_close_time(
            778, 1_778, false,
        ));
        let closed_hash = closed.header().hash;
        root.on_closed_ledger(closed);
        assert!(ledger_master.get_ledger_by_hash(closed_hash).is_none());

        let adaptor = RclValidationsAdaptor::new(|| NetClockTimePoint::new(1000));
        adaptor.set_ledger_master_runtime(Some(master_runtime));
        adaptor.set_loaded_ledger_runtime(Some(Arc::new(
            crate::AppLoadedLedgerRuntime::with_sources_and_ledger_master_state(
                Arc::clone(&ledger_master),
                None,
                None,
                Some(root.ledger_master_state()),
            ),
        )));

        let loaded =
            consensus::rcl_support::ValidationsAdaptor::acquire(&adaptor, closed_hash.as_uint256())
                .expect("current ApplicationRoot closed ledger must be a lookup fallback");
        assert_eq!(loaded.ledger_id, *closed_hash.as_uint256());
        assert_eq!(loaded.ledger_seq, 778);
    }
}
