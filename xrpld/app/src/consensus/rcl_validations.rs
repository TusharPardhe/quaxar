//! App-level wiring for Phase 5's generic `Validations<Adaptor>` tracker
//! against real `STValidation`/`Ledger` types, plus the `NetworkOPs`
//! validation-ingress support types (`RclValidationJournal`,
//! `RclValidationAcceptanceSink`, `RclValidationTrustSource`) and the
//! `SharedAppValidations<Clock>` handle used throughout
//! `network_ops_validation_runtime.rs`, `application_root.rs`, and
//! `validator_list.rs`. Ported from `RCLValidations.h`'s
//! `handleNewValidation` free function and the app-level ownership that
//! wraps `RCLValidations` in the reference `Application`.

use std::sync::{Arc, Mutex};

use basics::base_uint::Uint256;
use consensus::rcl_support::Validations;
use protocol::{PublicKey, STValidation};

use crate::consensus::rcl_validation::{RclValidatedLedger, RclValidation, RclValidationsAdaptor};
use crate::ledger::ledger_master_runtime::AppLedgerMasterRuntime;

/// Journal sink for validation-processing diagnostics. Matches the
/// reference's `beast::Journal` usage inside `handleNewValidation`.
pub trait RclValidationJournal {
    fn trace(&self, message: &str);
    fn info(&self, message: &str);
    fn error(&self, message: &str);
    fn warn(&self, message: &str);
}

/// Resolves trusted/listed signing keys for a validator identity. Matches
/// the reference's `ValidatorList::getTrustedKey` / `getListedKey` usage
/// inside `handleNewValidation`.
pub trait RclValidationTrustSource {
    fn get_trusted_key(&self, identity: &PublicKey) -> Option<PublicKey>;
    fn get_listed_key(&self, identity: &PublicKey) -> Option<PublicKey>;
}

/// Persists validations for later retrieval (e.g. by RPC or local
/// storage). Matches the reference's `Application::getValidationsDB`
/// forwarding: `handleNewValidation` calls this fire-and-forget for every
/// full validation, trusted or not, once it has been accepted as current.
///
/// Distinct from [`SharedAppValidations::store`] (which returns a
/// [`RclValidationsStoreView`] over the tracker's own historical query
/// surface, matching the reference's `RCLValidations` being both tracker
/// and queryable store in one type) -- this trait is the narrower
/// external-persistence hook passed into [`handle_new_validation_with_store`],
/// kept separate so a caller can plug in real disk/database persistence
/// without that concern leaking into the query-facing `store()` accessor.
pub trait RclValidationPersistence: Send + Sync {
    fn persist(&self, validation: &STValidation);
}

/// A no-op persistence hook, used where the caller has no external
/// validation storage configured.
#[derive(Debug, Default, Clone, Copy)]
pub struct NullRclValidationPersistence;

impl RclValidationPersistence for NullRclValidationPersistence {
    fn persist(&self, _validation: &STValidation) {}
}

/// Notified synchronously when a validation for the *current* working
/// ledger is accepted, so the caller (typically `NetworkOPs`) can trigger
/// an immediate `checkAccept`-style re-evaluation of whether that ledger
/// now has enough validation support to promote, without waiting for the
/// next timer tick. Matches the reference's
/// `app_.getLedgerMaster().checkAccept(ledgerHash, ledgerSeq)` call at the
/// end of `NetworkOPsImp::recvValidation`.
pub trait RclValidationAcceptanceSink {
    fn check_accept(&self, ledger_hash: Uint256, ledger_seq: u32);
}

/// A view over [`SharedAppValidations`]'s tracker exposing its historical
/// query surface, matching the reference's `RCLValidations` being usable
/// directly as both the live tracker and a queryable store of past
/// validations. Returned by [`SharedAppValidations::store`] so callers
/// (see `xrpld/main`'s ledger-catch-up and fee-voting logic) can chain
/// `.validations().store().trusted_for_ledger_by_sequence(...)` /
/// `.fees_for_ledger(...)` directly.
pub struct RclValidationsStoreView<'a, Clock: crate::state::time_keeper::TimeKeeperClock> {
    shared: &'a SharedAppValidations<Clock>,
}

impl<Clock: crate::state::time_keeper::TimeKeeperClock + 'static> RclValidationsStoreView<'_, Clock> {
    /// The signer public keys of trusted, full validations tracked for
    /// `ledger_id` at sequence `seq`. Matches the reference's
    /// `RCLValidations::getTrustedForLedger` usage in
    /// `NegativeUNLVote`/ledger-catch-up code (see `xrpld/main`'s
    /// `try_promote_ledger_with_validations` and negative-UNL voting call
    /// sites, which feed the result directly into
    /// `ValidatorList::negative_unl_filter_validations(Vec<STValidation>)`).
    /// Returns owned `STValidation`s (cloned out of the tracker's shared
    /// `Arc<STValidation>` wrapper) rather than just signer keys, since
    /// that filter needs the full validation to look up the signer's
    /// current master key.
    pub fn trusted_for_ledger_by_sequence(&self, ledger_id: Uint256, seq: u32) -> Vec<STValidation> {
        self.shared
            .inner
            .lock()
            .expect("shared app validations mutex must not be poisoned")
            .get_trusted_for_ledger(&ledger_id, seq)
            .into_iter()
            .map(|wrapped| (*wrapped).clone())
            .collect()
    }

    /// Fees reported by trusted, full validators for `ledger_id`,
    /// substituting `base_fee` for any validation that did not report a
    /// load fee. Matches `Validations::fees`; the `seq` parameter is
    /// accepted for call-site symmetry with `trusted_for_ledger_by_sequence`
    /// but not used for filtering -- the reference's own `fees()` likewise
    /// has no sequence filter, since fee-voting only cares about which
    /// ledger id validators are on, not which round each individual
    /// validation was issued in.
    pub fn fees_for_ledger(&self, ledger_id: Uint256, _seq: u32, base_fee: u32) -> Vec<u32> {
        self.shared.inner.lock().expect("shared app validations mutex must not be poisoned").fees(&ledger_id, base_fee)
    }
}

/// The concrete `Validations<RclValidationsAdaptor>` instantiation used by
/// the running node. Matches the reference's `RCLValidations` alias.
pub type RclValidationsInner = Validations<RclValidationsAdaptor>;

/// Shared, clonable handle to the node's validation tracker plus its
/// persistence store, generic over the time-keeper clock so it can be
/// constructed against either the system clock or a deterministic test
/// clock. Matches the reference's `Application`-owned `RCLValidations&`
/// accessor, minus the `Application` coupling.
pub struct SharedAppValidations<Clock: crate::state::time_keeper::TimeKeeperClock> {
    inner: Arc<Mutex<RclValidationsInner>>,
    persistence: Arc<dyn RclValidationPersistence>,
    ledger_master_state: Arc<crate::ledger::ledger_master_state::SharedLedgerMasterState>,
    journal: Arc<crate::state::app_registry::AppJournal>,
    ledger_master_runtime: Arc<Mutex<Option<Arc<AppLedgerMasterRuntime>>>>,
    _clock: std::marker::PhantomData<Clock>,
}

impl<Clock: crate::state::time_keeper::TimeKeeperClock> std::fmt::Debug for SharedAppValidations<Clock> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SharedAppValidations").finish_non_exhaustive()
    }
}

impl<Clock: crate::state::time_keeper::TimeKeeperClock> Clone for SharedAppValidations<Clock> {
    fn clone(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
            persistence: Arc::clone(&self.persistence),
            ledger_master_state: Arc::clone(&self.ledger_master_state),
            journal: Arc::clone(&self.journal),
            ledger_master_runtime: Arc::clone(&self.ledger_master_runtime),
            _clock: std::marker::PhantomData,
        }
    }
}

impl<Clock: crate::state::time_keeper::TimeKeeperClock + 'static> SharedAppValidations<Clock> {
    /// Construct a new shared validations tracker bound to `time_keeper`'s
    /// network-time source (see [`RclValidationsAdaptor::new`]),
    /// `ledger_master_state` (consulted for ledger-close-time-driven
    /// staleness bookkeeping shared with the rest of the ledger-master
    /// subsystem), and `journal` for diagnostics. Matches the reference's
    /// `Application`-owned `RCLValidations` construction, which similarly
    /// threads the shared clock and journal through at startup.
    pub fn new(
        time_keeper: Arc<crate::state::time_keeper::TimeKeeper<Clock>>,
        ledger_master_state: Arc<crate::ledger::ledger_master_state::SharedLedgerMasterState>,
        journal: Arc<crate::state::app_registry::AppJournal>,
    ) -> Self {
        let now_source = move || time_keeper.close_time();
        let adaptor = RclValidationsAdaptor::new(now_source);
        Self {
            inner: Arc::new(Mutex::new(Validations::new(consensus::rcl_support::ValidationParms::default(), adaptor))),
            persistence: Arc::new(NullRclValidationPersistence),
            ledger_master_state,
            journal,
            ledger_master_runtime: Arc::new(Mutex::new(None)),
            _clock: std::marker::PhantomData,
        }
    }

    /// Attach an external validation persistence hook (defaults to a
    /// no-op when constructed via [`Self::new`]).
    pub fn with_persistence(mut self, persistence: Arc<dyn RclValidationPersistence>) -> Self {
        self.persistence = persistence;
        self
    }

    /// The raw validation tracker, guarded by a `std::sync::Mutex` (a
    /// separate lock layer from `Validations<A>`'s own internal
    /// `parking_lot::Mutex`, which only guards that struct's private
    /// bookkeeping). Callers such as `handle_new_validation_with_store`
    /// lock this to get exclusive access to call through to the tracker.
    pub fn validations(&self) -> &Mutex<RclValidationsInner> {
        &self.inner
    }

    /// The external validation persistence hook.
    pub fn persistence(&self) -> &Arc<dyn RclValidationPersistence> {
        &self.persistence
    }

    /// A queryable view over this tracker's historical validation data.
    /// Matches the reference's `RCLValidations` being directly usable for
    /// both live tracking and historical queries; see
    /// [`RclValidationsStoreView`].
    pub fn store(&self) -> RclValidationsStoreView<'_, Clock> {
        RclValidationsStoreView { shared: self }
    }

    /// The ledger-master state this tracker was constructed against.
    pub fn ledger_master_state(&self) -> &Arc<crate::ledger::ledger_master_state::SharedLedgerMasterState> {
        &self.ledger_master_state
    }

    /// The diagnostics journal this tracker was constructed against.
    pub fn journal(&self) -> &Arc<crate::state::app_registry::AppJournal> {
        &self.journal
    }

    /// Register a newly-validated ledger with the underlying adaptor so
    /// `Validations::add`'s trie bookkeeping can resolve its ancestry.
    /// Matches the reference's `RCLValidationsAdaptor::onLedgerAcquired`-style
    /// hook, called wherever the node processes a newly built or acquired
    /// ledger.
    pub fn register_ledger(&self, ledger: &ledger::Ledger) {
        self.inner.lock().expect("shared app validations mutex must not be poisoned").adaptor().register_ledger(ledger);
    }

    /// Number of trusted, full validations tracked for `ledger_hash`.
    /// Matches `Validations::numTrustedForLedger`, exposed directly since
    /// `bootstrap.rs`'s catch-up loop calls this without needing the full
    /// tracker.
    pub fn num_trusted_for_ledger(&self, ledger_hash: Uint256) -> usize {
        self.inner.lock().expect("shared app validations mutex must not be poisoned").num_trusted_for_ledger(&ledger_hash)
    }

    /// Attach (or detach) the ledger master runtime this validations
    /// tracker should coordinate with when ledgers are attached/rotated.
    /// Returns the previously-attached runtime, if any. This is a thin
    /// bookkeeping slot -- `Validations<RclValidationsAdaptor>` resolves
    /// ledger ancestry through `register_ledger`/`acquire`, not through
    /// this runtime directly; it is retained here purely so
    /// `ApplicationRoot::attach_ledger_master_runtime` has a place to wire
    /// the two together for future extension (e.g. auto-registering newly
    /// validated ledgers).
    pub fn set_ledger_master_runtime(&self, runtime: Option<Arc<AppLedgerMasterRuntime>>) -> Option<Arc<AppLedgerMasterRuntime>> {
        let mut slot = self.ledger_master_runtime.lock().expect("shared app validations ledger master runtime mutex must not be poisoned");
        let previous = std::mem::replace(&mut *slot, runtime.clone());
        self.inner
            .lock()
            .expect("shared app validations mutex must not be poisoned")
            .adaptor()
            .set_ledger_master_runtime(runtime);
        previous
    }
}

/// Process a newly-received validation against the tracker, mirroring the
/// reference's `handleNewValidation` free function: resolve the signer's
/// trust status via `trust_source`, add it to `validations`, notify
/// `accept_sink` if it became the current validation for its round, and
/// persist it via `store` regardless of trust (matching the reference's
/// unconditional store-on-accept behavior).
///
/// `bypass_accept` mirrors the reference's `NetworkOPsImp::recvValidation`
/// dedup rule: when a validation for the same ledger hash is already
/// mid-flight, later arrivals are still added to the tracker but skip the
/// acceptance-sink notification (since the first arrival already
/// triggered it).
pub fn handle_new_validation_with_store(
    trust_source: &dyn RclValidationTrustSource,
    validations: &mut RclValidationsInner,
    validation: &mut STValidation,
    bypass_accept: bool,
    persistence: Option<&dyn RclValidationPersistence>,
    journal: Option<&dyn RclValidationJournal>,
) -> (consensus::ValidationStatus, Option<(Uint256, u32)>) {
    let signing_key = *validation.get_signer_public();

    // Matches the reference's `handleNewValidation`: a validation is
    // trusted only if its signing key currently maps to a trusted master
    // key in the active UNL. Being merely "listed" (known but not
    // currently trusted) or unrecognized entirely both result in an
    // untrusted validation -- listing only affects whether it is worth
    // tracking at all, which this port always does regardless.
    if trust_source.get_trusted_key(&signing_key).is_some() {
        validation.set_trusted();
    } else {
        validation.set_untrusted();
    }

    let node_id = validation.get_node_id();
    let wrapped = RclValidation::new(Arc::new(validation.clone()));
    let status = validations.add(node_id, wrapped);

    if let Some(journal) = journal {
        journal.trace(&format!("handleNewValidation: {status} for ledger {}", validation.get_ledger_hash()));
    }

    let mut check_accept_args = None;
    if status == consensus::ValidationStatus::Current {
        if !bypass_accept {
            // Matches the reference: `validations.add()` fully returns
            // (releasing its internal lock) BEFORE `checkAccept` runs --
            // they are sequential, not nested. Returning the args here
            // instead of invoking the sink directly lets the caller run
            // it AFTER releasing the validations lock it's holding,
            // avoiding a self-deadlock (num_trusted_for_ledger inside
            // check_accept needs to re-lock the same mutex).
            check_accept_args = Some((
                validation.get_ledger_hash(),
                validation.get_field_u32(protocol::get_field_by_symbol("sfLedgerSequence")),
            ));
        }
        if let Some(persistence) = persistence {
            persistence.persist(validation);
        }
    }

    (status, check_accept_args)
}

/// Marker type distinguishing the ledger type used by
/// [`SharedAppValidations`] from the generic `A::Ledger` associated type,
/// kept here purely for documentation discoverability from
/// `rcl_consensus.rs`.
pub type SharedAppValidationsLedger = RclValidatedLedger;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ledger::ledger_master_state::{LedgerMasterCloseTimeProvider, SharedLedgerMasterState};
    use crate::state::app_registry::AppJournal;
    use crate::state::time_keeper::{SystemTimeKeeperClock, TimeKeeper};
    use protocol::{KeyType, calc_node_id, derive_public_key, generate_secret_key, get_field_by_symbol, random_seed};

    struct AllTrusted;
    impl RclValidationTrustSource for AllTrusted {
        fn get_trusted_key(&self, identity: &PublicKey) -> Option<PublicKey> {
            Some(*identity)
        }
        fn get_listed_key(&self, identity: &PublicKey) -> Option<PublicKey> {
            Some(*identity)
        }
    }

    struct NoneTrusted;
    impl RclValidationTrustSource for NoneTrusted {
        fn get_trusted_key(&self, _identity: &PublicKey) -> Option<PublicKey> {
            None
        }
        fn get_listed_key(&self, _identity: &PublicKey) -> Option<PublicKey> {
            None
        }
    }

    fn signed_validation(ledger_hash: Uint256, seq: u32, sign_time: u32) -> STValidation {
        let seed = random_seed();
        let secret_key = generate_secret_key(KeyType::Secp256k1, &seed).expect("secret key generation should succeed");
        let public_key = derive_public_key(KeyType::Secp256k1, &secret_key).expect("public key derivation should succeed");
        let node_id = calc_node_id(&public_key);

        STValidation::new_signed(sign_time, &public_key, node_id, &secret_key, |v| {
            v.set_field_h256(get_field_by_symbol("sfLedgerHash"), ledger_hash);
            v.set_field_u32(get_field_by_symbol("sfLedgerSequence"), seq);
            v.set_field_u32(get_field_by_symbol("sfFlags"), protocol::VF_FULL_VALIDATION);
        })
        .expect("validation signing should succeed")
    }

    fn shared_validations() -> (SharedAppValidations<SystemTimeKeeperClock>, u32) {
        let time_keeper = Arc::new(TimeKeeper::new());
        let now = time_keeper.close_time().as_seconds();
        let close_time_provider = Arc::clone(&time_keeper) as Arc<dyn LedgerMasterCloseTimeProvider>;
        let ledger_master_state = Arc::new(SharedLedgerMasterState::new(close_time_provider));
        (SharedAppValidations::new(time_keeper, ledger_master_state, Arc::new(AppJournal::new("Validations"))), now)
    }

    #[test]
    fn handle_new_validation_marks_trusted_and_returns_current() {
        let (shared, now) = shared_validations();
        let ledger = ledger::Ledger::from_ledger_seq_and_close_time(1, 100, false);
        shared.register_ledger(&ledger);
        let ledger_hash = *ledger.header().hash.as_uint256();

        let mut validation = signed_validation(ledger_hash, 1, now);
        let mut inner = shared.validations().lock().unwrap();
        let status = handle_new_validation_with_store(&AllTrusted, &mut inner, &mut validation, false, None, None).0;

        assert_eq!(status, consensus::ValidationStatus::Current);
        assert!(validation.is_trusted());
    }

    #[test]
    fn handle_new_validation_marks_untrusted_when_not_listed() {
        let (shared, now) = shared_validations();
        let ledger = ledger::Ledger::from_ledger_seq_and_close_time(1, 100, false);
        shared.register_ledger(&ledger);
        let ledger_hash = *ledger.header().hash.as_uint256();

        let mut validation = signed_validation(ledger_hash, 1, now);
        let mut inner = shared.validations().lock().unwrap();
        let _ = handle_new_validation_with_store(&NoneTrusted, &mut inner, &mut validation, false, None, None);

        assert!(!validation.is_trusted());
    }

    #[test]
    fn num_trusted_for_ledger_reflects_added_validation() {
        let (shared, now) = shared_validations();
        let ledger = ledger::Ledger::from_ledger_seq_and_close_time(1, 100, false);
        shared.register_ledger(&ledger);
        let ledger_hash = *ledger.header().hash.as_uint256();

        let mut validation = signed_validation(ledger_hash, 1, now);
        {
            let mut inner = shared.validations().lock().unwrap();
            let _ = handle_new_validation_with_store(&AllTrusted, &mut inner, &mut validation, false, None, None);
        }

        assert_eq!(shared.num_trusted_for_ledger(ledger_hash), 1);
    }
}
