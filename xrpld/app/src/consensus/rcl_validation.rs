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

        // Matches the reference's `RCLValidationsAdaptor::acquire`: a
        // local-map miss falls back to the shared ledger history cache
        // (populated by completed acquisitions/validated ledgers from
        // anywhere in the app, not just this adaptor's own
        // `register_ledger` calls), and if THAT also misses, actively
        // dispatches a fetch (matching the reference's `GetConsL2` job
        // calling `InboundLedgers::acquireAsync`) rather than silently
        // giving up. This is the third of the reference's three
        // redundant acquisition triggers (the other two being
        // `Consensus::checkLedger`'s `acquireLedger` and `InboundLedger`'s
        // own retry timer) -- `Validations::updateTrie` calls this
        // adaptor method every time a new TRUSTED validation references a
        // ledger not yet cached, so leaving this as a pure cache read
        // (the bug this replaces) meant the trie could never actively
        // pull in ledgers referenced only by validations, weakening
        // fork-recovery specifically in the scenario where a node has
        // fallen behind and peers are validating ledgers it has not
        // acquired via any other path.
        let Some(runtime) = self.ledger_master_runtime.lock().clone() else {
            return None;
        };

        let hash = basics::sha_map_hash::SHAMapHash::new(*ledger_id);
        if let Some(ledger) = runtime
            .ledger_master()
            .ledger_history()
            .get_cached_ledger_by_hash(hash)
        {
            return Some(RclValidatedLedger::from_ledger(&ledger));
        }

        // Rippled's LedgerMaster::getLedgerByHash (LedgerMaster.cpp:1722-1724)
        // also checks the actual closed-ledger slot as a fallback when the
        // history cache misses. This handles the case where the ledger was
        // swept from the TaggedCache but is still the node's current LCL.
        if let Some(closed) = runtime.ledger_master().closed_ledger() {
            if closed.header().hash.as_uint256() == ledger_id {
                return Some(RclValidatedLedger::from_ledger(&closed));
            }
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
    use ledger::Ledger as LedgerImpl;
    use protocol::{
        KeyType, SecretKey, calc_node_id, derive_public_key, generate_secret_key, random_seed,
    };

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
}
