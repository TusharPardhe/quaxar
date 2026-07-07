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
pub trait RclValidationStore: Send + Sync {
    fn store(&self, validation: &STValidation);
}

/// A no-op store, used where the caller has no local validation
/// persistence configured.
#[derive(Debug, Default, Clone, Copy)]
pub struct NullRclValidationStore;

impl RclValidationStore for NullRclValidationStore {
    fn store(&self, _validation: &STValidation) {}
}

/// Notified synchronously when a validation for the *current* working
/// ledger is accepted, so the caller (typically `NetworkOPs`) can trigger
/// an immediate `checkAccept`-style re-evaluation without waiting for the
/// next timer tick. Matches the reference's inline `app_.getOPs().pubValidation(val)`
/// plus `checkAccept` call sequence at the end of `NetworkOPsImp::recvValidation`.
pub trait RclValidationAcceptanceSink {
    fn on_validation_current(&self, validation: &STValidation);
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
    store: Arc<dyn RclValidationStore>,
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
            store: Arc::clone(&self.store),
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
            store: Arc::new(NullRclValidationStore),
            ledger_master_state,
            journal,
            ledger_master_runtime: Arc::new(Mutex::new(None)),
            _clock: std::marker::PhantomData,
        }
    }

    /// Attach a validation persistence store (defaults to a no-op store
    /// when constructed via [`Self::new`]).
    pub fn with_store(mut self, store: Arc<dyn RclValidationStore>) -> Self {
        self.store = store;
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

    /// The configured validation persistence store.
    pub fn store(&self) -> &Arc<dyn RclValidationStore> {
        &self.store
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
        std::mem::replace(&mut *slot, runtime)
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
    accept_sink: Option<&dyn RclValidationAcceptanceSink>,
    store: Option<&dyn RclValidationStore>,
    journal: Option<&dyn RclValidationJournal>,
) -> consensus::ValidationStatus {
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

    if status == consensus::ValidationStatus::Current {
        if !bypass_accept
            && let Some(sink) = accept_sink
        {
            sink.on_validation_current(validation);
        }
        if let Some(store) = store {
            store.store(validation);
        }
    }

    status
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
        let status = handle_new_validation_with_store(&AllTrusted, &mut inner, &mut validation, false, None, None, None);

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
        let _ = handle_new_validation_with_store(&NoneTrusted, &mut inner, &mut validation, false, None, None, None);

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
            let _ = handle_new_validation_with_store(&AllTrusted, &mut inner, &mut validation, false, None, None, None);
        }

        assert_eq!(shared.num_trusted_for_ledger(ledger_hash), 1);
    }
}
