//! App-owned validation intake seams above the generic consensus owner.

use crate::ledger::ledger_master_runtime::AppLedgerMasterRuntime;
use crate::ledger::ledger_master_state::SharedLedgerMasterState;
use crate::state::time_keeper::{TimeKeeper, TimeKeeperClock};
use basics::base_uint::Uint256;
use basics::chrono::NetClockTimePoint;
use basics::sha_map_hash::SHAMapHash;
use consensus::{
    RclValidatedLedger, RclValidation, RclValidations, RclValidationsAdapter, ValidationStatus,
};
use ledger::Ledger;
use protocol::{PublicKey, STValidation, get_field_by_symbol, skip_keylet};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};

pub trait RclValidationJournal {
    fn trace(&self, message: &str);
    fn info(&self, message: &str);
    fn error(&self, message: &str);
    fn warn(&self, message: &str);
}

impl<T> RclValidationJournal for Arc<T>
where
    T: RclValidationJournal + ?Sized,
{
    fn trace(&self, message: &str) {
        (**self).trace(message);
    }

    fn info(&self, message: &str) {
        (**self).info(message);
    }

    fn error(&self, message: &str) {
        (**self).error(message);
    }

    fn warn(&self, message: &str) {
        (**self).warn(message);
    }
}

#[derive(Debug, Clone, Default)]
pub struct NullRclValidationJournal;

impl RclValidationJournal for NullRclValidationJournal {
    fn trace(&self, _message: &str) {}
    fn info(&self, _message: &str) {}
    fn error(&self, _message: &str) {}
    fn warn(&self, _message: &str) {}
}

pub trait RclValidationLedgerSource: Clone {
    fn now(&self) -> NetClockTimePoint;
    fn acquire_ledger(&self, ledger_id: &Uint256) -> Option<RclValidatedLedger>;
    fn request_validated_ledger(&self, ledger_id: &Uint256);
}

#[derive(Debug, Clone)]
pub struct AppRclValidationsAdaptor<S, J = NullRclValidationJournal> {
    source: S,
    journal: J,
}

pub type AppValidationJournalHandle = Arc<dyn RclValidationJournal + Send + Sync>;

pub struct AppRclValidationLedgerSource<C = crate::state::time_keeper::SystemTimeKeeperClock>
where
    C: TimeKeeperClock,
{
    time_keeper: Arc<TimeKeeper<C>>,
    ledger_master_state: Arc<SharedLedgerMasterState>,
    ledger_master_runtime: Arc<Mutex<Option<Arc<AppLedgerMasterRuntime>>>>,
}

impl<C> Clone for AppRclValidationLedgerSource<C>
where
    C: TimeKeeperClock,
{
    fn clone(&self) -> Self {
        Self {
            time_keeper: Arc::clone(&self.time_keeper),
            ledger_master_state: Arc::clone(&self.ledger_master_state),
            ledger_master_runtime: Arc::clone(&self.ledger_master_runtime),
        }
    }
}

#[derive(Debug)]
pub struct AppRclValidationStore {
    retained_ledgers: u32,
    inner: Mutex<AppRclValidationStoreState>,
}

#[derive(Debug, Default)]
struct AppRclValidationStoreState {
    newest_seq: u32,
    by_ledger: BTreeMap<(u32, Uint256), BTreeMap<PublicKey, STValidation>>,
}

impl AppRclValidationStore {
    pub fn new(retained_ledgers: u32) -> Self {
        Self {
            retained_ledgers: retained_ledgers.max(1),
            inner: Mutex::new(AppRclValidationStoreState::default()),
        }
    }

    pub fn record(&self, validation: &STValidation) {
        if !validation.is_trusted() || !validation.is_full() {
            return;
        }

        let ledger_id = validation.get_ledger_hash();
        let seq = validation.get_field_u32(get_field_by_symbol("sfLedgerSequence"));
        let signer = *validation.get_signer_public();
        let mut inner = self
            .inner
            .lock()
            .expect("validation store mutex must not be poisoned");
        inner.newest_seq = inner.newest_seq.max(seq);
        inner
            .by_ledger
            .entry((seq, ledger_id))
            .or_default()
            .insert(signer, validation.clone());

        let floor = inner.newest_seq.saturating_sub(self.retained_ledgers);
        inner
            .by_ledger
            .retain(|(entry_seq, _), _| *entry_seq >= floor);
    }

    pub fn trusted_for_ledger_by_sequence(
        &self,
        ledger_id: Uint256,
        seq: u32,
    ) -> Vec<STValidation> {
        self.inner
            .lock()
            .expect("validation store mutex must not be poisoned")
            .by_ledger
            .get(&(seq, ledger_id))
            .map(|bucket| bucket.values().cloned().collect())
            .unwrap_or_default()
    }

    pub fn tracked_keys(&self, ledger_id: Uint256, seq: u32) -> BTreeSet<PublicKey> {
        self.inner
            .lock()
            .expect("validation store mutex must not be poisoned")
            .by_ledger
            .get(&(seq, ledger_id))
            .map(|bucket| bucket.keys().copied().collect())
            .unwrap_or_default()
    }

    /// for a given ledger hash. Returns fee values for median computation.
    pub fn fees_for_ledger(&self, ledger_id: Uint256, seq: u32, base_fee: u32) -> Vec<u32> {
        let guard = self
            .inner
            .lock()
            .expect("validation store mutex must not be poisoned");
        let Some(bucket) = guard.by_ledger.get(&(seq, ledger_id)) else {
            return Vec::new();
        };
        bucket
            .values()
            .filter(|v| v.is_trusted())
            .map(|v| {
                let load_fee_field = protocol::get_field_by_symbol("sfLoadFee");
                if v.is_field_present(load_fee_field) {
                    v.get_field_u32(load_fee_field)
                } else {
                    base_fee
                }
            })
            .collect()
    }
}

impl Default for AppRclValidationStore {
    fn default() -> Self {
        Self::new(256)
    }
}

#[derive(Clone)]
pub struct AppRclConsensusValidationBridge<A>
where
    A: RclValidationsAdapter + Send + 'static,
{
    validations: Arc<Mutex<RclValidations<A>>>,
    store: Arc<AppRclValidationStore>,
}

pub type AppValidationsCore<C = crate::state::time_keeper::SystemTimeKeeperClock> = RclValidations<
    AppRclValidationsAdaptor<AppRclValidationLedgerSource<C>, AppValidationJournalHandle>,
>;

#[derive(Clone)]
pub struct SharedAppValidations<C = crate::state::time_keeper::SystemTimeKeeperClock>
where
    C: TimeKeeperClock,
{
    bridge: AppRclConsensusValidationBridge<
        AppRclValidationsAdaptor<AppRclValidationLedgerSource<C>, AppValidationJournalHandle>,
    >,
    source: AppRclValidationLedgerSource<C>,
}

impl<C> std::fmt::Debug for AppRclValidationLedgerSource<C>
where
    C: TimeKeeperClock,
{
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("AppRclValidationLedgerSource")
            .field(
                "has_ledger_master_runtime",
                &self
                    .ledger_master_runtime
                    .lock()
                    .expect("validation ledger source mutex must not be poisoned")
                    .is_some(),
            )
            .field(
                "validated_ledger_seq",
                &self.ledger_master_state.validated_ledger_seq(),
            )
            .finish()
    }
}

impl<C> std::fmt::Debug for SharedAppValidations<C>
where
    C: TimeKeeperClock,
{
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SharedAppValidations")
            .field("source", &self.source)
            .field(
                "tracked_parent_ledgers",
                &self
                    .bridge
                    .store()
                    .tracked_keys(Uint256::default(), 0)
                    .len(),
            )
            .finish()
    }
}

impl<A> AppRclConsensusValidationBridge<A>
where
    A: RclValidationsAdapter + Send + 'static,
{
    pub fn new(
        validations: Arc<Mutex<RclValidations<A>>>,
        store: Arc<AppRclValidationStore>,
    ) -> Self {
        Self { validations, store }
    }

    pub fn validations(&self) -> &Arc<Mutex<RclValidations<A>>> {
        &self.validations
    }

    pub fn store(&self) -> &Arc<AppRclValidationStore> {
        &self.store
    }
}

impl<S, J> AppRclValidationsAdaptor<S, J> {
    pub fn new(source: S, journal: J) -> Self {
        Self { source, journal }
    }

    pub fn source(&self) -> &S {
        &self.source
    }

    pub fn journal(&self) -> &J {
        &self.journal
    }
}

impl<C> AppRclValidationLedgerSource<C>
where
    C: TimeKeeperClock,
{
    pub fn new(
        time_keeper: Arc<TimeKeeper<C>>,
        ledger_master_state: Arc<SharedLedgerMasterState>,
    ) -> Self {
        Self {
            time_keeper,
            ledger_master_state,
            ledger_master_runtime: Arc::new(Mutex::new(None)),
        }
    }

    pub fn set_ledger_master_runtime(
        &self,
        ledger_master_runtime: Option<Arc<AppLedgerMasterRuntime>>,
    ) -> Option<Arc<AppLedgerMasterRuntime>> {
        let mut current = self
            .ledger_master_runtime
            .lock()
            .expect("validation ledger source mutex must not be poisoned");
        std::mem::replace(&mut *current, ledger_master_runtime)
    }

    fn lookup_ledger(&self, ledger_id: &Uint256) -> Option<Arc<Ledger>> {
        let runtime = self
            .ledger_master_runtime
            .lock()
            .expect("validation ledger source mutex must not be poisoned")
            .clone();
        if let Some(runtime) = runtime
            && let Some(ledger) = runtime
                .ledger_master()
                .get_ledger_by_hash(SHAMapHash::new(*ledger_id))
        {
            return Some(ledger);
        }

        self.ledger_master_state
            .validated_ledger()
            .filter(|ledger| ledger.header().hash.as_uint256() == ledger_id)
            .or_else(|| {
                self.ledger_master_state
                    .published_ledger()
                    .filter(|ledger| ledger.header().hash.as_uint256() == ledger_id)
            })
            .or_else(|| {
                self.ledger_master_state
                    .closed_ledger()
                    .filter(|ledger| ledger.header().hash.as_uint256() == ledger_id)
            })
    }
}

impl<C> SharedAppValidations<C>
where
    C: TimeKeeperClock,
{
    pub fn new(
        time_keeper: Arc<TimeKeeper<C>>,
        ledger_master_state: Arc<SharedLedgerMasterState>,
        journal: AppValidationJournalHandle,
    ) -> Self {
        let source = AppRclValidationLedgerSource::new(time_keeper, ledger_master_state);
        let validations = Arc::new(Mutex::new(RclValidations::new(
            AppRclValidationsAdaptor::new(source.clone(), journal),
            consensus::ConsensusParms::default(),
        )));
        let store = Arc::new(AppRclValidationStore::default());
        Self {
            bridge: AppRclConsensusValidationBridge::new(validations, store),
            source,
        }
    }

    pub fn validations(&self) -> &Arc<Mutex<AppValidationsCore<C>>> {
        self.bridge.validations()
    }

    pub fn store(&self) -> &Arc<AppRclValidationStore> {
        self.bridge.store()
    }

    pub fn bridge(
        &self,
    ) -> &AppRclConsensusValidationBridge<
        AppRclValidationsAdaptor<AppRclValidationLedgerSource<C>, AppValidationJournalHandle>,
    > {
        &self.bridge
    }

    pub fn set_ledger_master_runtime(
        &self,
        ledger_master_runtime: Option<Arc<AppLedgerMasterRuntime>>,
    ) -> Option<Arc<AppLedgerMasterRuntime>> {
        self.source.set_ledger_master_runtime(ledger_master_runtime)
    }
}

impl<S, J> RclValidationsAdapter for AppRclValidationsAdaptor<S, J>
where
    S: RclValidationLedgerSource,
    J: RclValidationJournal + Clone,
{
    fn now(&self) -> NetClockTimePoint {
        self.source.now()
    }

    fn acquire(&mut self, ledger_id: &Uint256) -> Option<RclValidatedLedger> {
        if let Some(ledger) = self.source.acquire_ledger(ledger_id) {
            return Some(ledger);
        }

        self.journal.warn(&format!(
            "Need validated ledger for preferred ledger analysis {ledger_id}"
        ));
        self.source.request_validated_ledger(ledger_id);
        None
    }
}

impl<C> RclValidationLedgerSource for AppRclValidationLedgerSource<C>
where
    C: TimeKeeperClock,
{
    fn now(&self) -> NetClockTimePoint {
        self.time_keeper.close_time()
    }

    fn acquire_ledger(&self, ledger_id: &Uint256) -> Option<RclValidatedLedger> {
        self.lookup_ledger(ledger_id)
            .map(|ledger| validated_ledger_from_ledger(ledger.as_ref(), &NullRclValidationJournal))
    }

    fn request_validated_ledger(&self, _ledger_id: &Uint256) {}
}

pub trait RclValidationTrustSource {
    fn get_trusted_key(&self, identity: &PublicKey) -> Option<PublicKey>;
    fn get_listed_key(&self, identity: &PublicKey) -> Option<PublicKey>;
}

pub trait RclValidationAcceptanceSink {
    fn check_accept(&self, hash: Uint256, seq: u32);
}

pub fn wrap_st_validation(validation: &STValidation) -> RclValidation {
    RclValidation {
        ledger_id: validation.get_ledger_hash(),
        seq: validation.get_field_u32(get_field_by_symbol("sfLedgerSequence")),
        sign_time: NetClockTimePoint::new(validation.get_sign_time()),
        seen_time: NetClockTimePoint::new(validation.get_seen_time()),
        key: *validation.get_signer_public(),
        trusted: validation.is_trusted(),
        full: validation.is_full(),
        load_fee: validation
            .is_field_present(get_field_by_symbol("sfLoadFee"))
            .then(|| validation.get_field_u32(get_field_by_symbol("sfLoadFee"))),
        cookie: validation.get_field_u64(get_field_by_symbol("sfCookie")),
    }
}

pub fn validated_ledger_from_ledger<J>(ledger: &Ledger, journal: &J) -> RclValidatedLedger
where
    J: RclValidationJournal,
{
    let mut ancestors = Vec::new();
    match ledger.read(skip_keylet()) {
        Ok(Some(hashes)) => {
            if ledger.header().seq > 0 {
                assert_eq!(
                    hashes.get_field_u32(get_field_by_symbol("sfLastLedgerSequence")),
                    ledger.header().seq - 1,
                    "xrpl::RCLValidatedLedger::RCLValidatedLedger(Ledger) : valid last ledger sequence"
                );
            }
            ancestors = hashes
                .get_field_v256(get_field_by_symbol("sfHashes"))
                .value()
                .to_vec();
        }
        Ok(None) | Err(_) => {
            journal.warn(&format!(
                "Ledger {}:{} missing recent ancestor hashes",
                ledger.header().seq,
                ledger.header().hash
            ));
        }
    }

    RclValidatedLedger {
        ledger_id: *ledger.header().hash.as_uint256(),
        ledger_seq: ledger.header().seq,
        ancestors,
    }
}

pub fn handle_new_validation<A, T, S, J>(
    trust_source: &T,
    validations: &mut RclValidations<A>,
    validation: &mut STValidation,
    bypass_accept: bool,
    accept_sink: Option<&S>,
    journal: Option<&J>,
) -> ValidationStatus
where
    A: RclValidationsAdapter,
    T: RclValidationTrustSource + ?Sized,
    S: RclValidationAcceptanceSink + ?Sized,
    J: RclValidationJournal + ?Sized,
{
    let signing_key = *validation.get_signer_public();
    let hash = validation.get_ledger_hash();
    let seq = validation.get_field_u32(get_field_by_symbol("sfLedgerSequence"));

    let mut master_key = trust_source.get_trusted_key(&signing_key);
    if !validation.is_trusted() && master_key.is_some() {
        validation.set_trusted();
    }
    if master_key.is_none() {
        master_key = trust_source.get_listed_key(&signing_key);
    }

    let node_id = master_key.unwrap_or(signing_key);
    let outcome = validations.add(node_id, wrap_st_validation(validation));

    if outcome == ValidationStatus::Current {
        if validation.is_trusted() {
            if bypass_accept {
                if let Some(journal) = journal {
                    journal.trace(&format!("Bypassing checkAccept for validation {hash}"));
                }
            } else if let Some(accept_sink) = accept_sink {
                accept_sink.check_accept(hash, seq);
            }
        }
        return outcome;
    }

    // Also trigger checkAccept for trusted non-current validations.
    // On testnet with few peers, validations often arrive slightly late
    // (current=false) but are still valid for advancing the validated ledger.
    if validation.is_trusted() && !bypass_accept {
        if let Some(accept_sink) = accept_sink {
            accept_sink.check_accept(hash, seq);
        }
    }

    if matches!(
        outcome,
        ValidationStatus::Conflicting | ValidationStatus::Multiple
    ) {
        let node_label = if let Some(master_key) = master_key
            && master_key != signing_key
        {
            format!(
                "{}:{}",
                signing_key.to_node_public_base58(),
                master_key.to_node_public_base58()
            )
        } else {
            signing_key.to_node_public_base58()
        };

        if let Some(journal) = journal {
            let message = match outcome {
                ValidationStatus::Conflicting => format!(
                    "Byzantine Behavior Detector: {}{}: Conflicting validation for {seq}!",
                    if validation.is_trusted() {
                        "trusted "
                    } else {
                        "untrusted "
                    },
                    node_label,
                ),
                ValidationStatus::Multiple => format!(
                    "Byzantine Behavior Detector: {}{}: Multiple validations for {seq}/{hash}!",
                    if validation.is_trusted() {
                        "trusted "
                    } else {
                        "untrusted "
                    },
                    node_label,
                ),
                _ => unreachable!("guarded above"),
            };

            if validation.is_trusted() {
                journal.error(&message);
            } else {
                journal.info(&message);
            }
        }
    }

    outcome
}

pub fn handle_new_validation_with_store<A, T, S, J>(
    trust_source: &T,
    validations: &mut RclValidations<A>,
    validation: &mut STValidation,
    bypass_accept: bool,
    accept_sink: Option<&S>,
    validation_store: Option<&AppRclValidationStore>,
    journal: Option<&J>,
) -> ValidationStatus
where
    A: RclValidationsAdapter,
    T: RclValidationTrustSource + ?Sized,
    S: RclValidationAcceptanceSink + ?Sized,
    J: RclValidationJournal + ?Sized,
{
    let outcome = handle_new_validation(
        trust_source,
        validations,
        validation,
        bypass_accept,
        accept_sink,
        journal,
    );

    if outcome == ValidationStatus::Current
        && validation.is_trusted()
        && validation.is_full()
        && let Some(store) = validation_store
    {
        store.record(validation);
    }

    outcome
}

#[cfg(test)]
mod tests {
    use super::{
        AppRclValidationStore, AppRclValidationsAdaptor, NullRclValidationJournal,
        RclValidationJournal, RclValidationLedgerSource, handle_new_validation_with_store,
        wrap_st_validation,
    };
    use basics::base_uint::Uint256;
    use basics::chrono::NetClockTimePoint;
    use consensus::{
        ConsensusParms, RclValidatedLedger, RclValidations, RclValidationsAdapter, ValidationStatus,
    };
    use protocol::{
        KeyType, STValidation, SecretKey, VF_FULL_VALIDATION, calc_node_id, derive_public_key,
        get_field_by_symbol,
    };
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};

    #[derive(Clone)]
    struct MockSource {
        now: NetClockTimePoint,
        ledgers: HashMap<Uint256, RclValidatedLedger>,
        requested: Arc<Mutex<Vec<Uint256>>>,
    }

    impl MockSource {
        fn new(now: u32) -> Self {
            Self {
                now: NetClockTimePoint::new(now),
                ledgers: HashMap::new(),
                requested: Arc::new(Mutex::new(Vec::new())),
            }
        }
    }

    impl RclValidationLedgerSource for MockSource {
        fn now(&self) -> NetClockTimePoint {
            self.now
        }

        fn acquire_ledger(&self, ledger_id: &Uint256) -> Option<RclValidatedLedger> {
            self.ledgers.get(ledger_id).cloned()
        }

        fn request_validated_ledger(&self, ledger_id: &Uint256) {
            self.requested
                .lock()
                .expect("requested mutex")
                .push(*ledger_id);
        }
    }

    #[derive(Debug, Clone, Default)]
    struct RecordingJournal {
        entries: Arc<Mutex<Vec<(String, String)>>>,
    }

    impl RecordingJournal {
        fn entries(&self) -> Vec<(String, String)> {
            self.entries.lock().expect("entries mutex").clone()
        }
    }

    impl RclValidationJournal for RecordingJournal {
        fn trace(&self, message: &str) {
            self.entries
                .lock()
                .expect("entries mutex")
                .push(("trace".to_owned(), message.to_owned()));
        }

        fn info(&self, message: &str) {
            self.entries
                .lock()
                .expect("entries mutex")
                .push(("info".to_owned(), message.to_owned()));
        }

        fn error(&self, message: &str) {
            self.entries
                .lock()
                .expect("entries mutex")
                .push(("error".to_owned(), message.to_owned()));
        }

        fn warn(&self, message: &str) {
            self.entries
                .lock()
                .expect("entries mutex")
                .push(("warn".to_owned(), message.to_owned()));
        }
    }

    fn signed_validation(seed: u8, ledger_id: Uint256, seq: u32, trusted: bool) -> STValidation {
        let secret = SecretKey::from_bytes([seed; 32]);
        let public = derive_public_key(KeyType::Secp256k1, &secret).expect("public key");
        let mut validation = STValidation::new_signed(
            1000,
            &public,
            calc_node_id(&public),
            &secret,
            |validation| {
                validation.set_field_h256(get_field_by_symbol("sfLedgerHash"), ledger_id);
                validation.set_field_u32(get_field_by_symbol("sfLedgerSequence"), seq);
                validation.set_field_u64(get_field_by_symbol("sfCookie"), 11);
                validation.set_flag(VF_FULL_VALIDATION);
            },
        )
        .expect("signed validation");
        if trusted {
            validation.set_trusted();
        }
        validation
    }

    #[test]
    fn adaptor_requests_missing_ledgers() {
        let requested = Arc::new(Mutex::new(Vec::new()));
        let source = MockSource {
            now: NetClockTimePoint::new(10),
            ledgers: HashMap::new(),
            requested: Arc::clone(&requested),
        };
        let journal = RecordingJournal::default();
        let mut adaptor = AppRclValidationsAdaptor::new(source, journal.clone());
        let missing = Uint256::from_u64(901);

        assert!(adaptor.acquire(&missing).is_none());
        assert_eq!(
            requested.lock().expect("requested mutex").as_slice(),
            &[missing]
        );
        assert_eq!(
            journal.entries()[0],
            (
                "warn".to_owned(),
                format!("Need validated ledger for preferred ledger analysis {missing}")
            )
        );
    }

    #[test]
    fn wrap_st_validation_preserves_validation_metadata() {
        let validation = signed_validation(7, Uint256::from_u64(77), 55, true);
        let wrapped = wrap_st_validation(&validation);

        assert_eq!(wrapped.ledger_id, Uint256::from_u64(77));
        assert_eq!(wrapped.seq, 55);
        assert!(wrapped.trusted);
        assert!(wrapped.full);
        assert_eq!(wrapped.cookie, 11);
    }

    #[test]
    fn adaptor_exposes_source_and_journal_handles() {
        let source = MockSource::new(33);
        let adaptor = AppRclValidationsAdaptor::new(source.clone(), NullRclValidationJournal);

        assert_eq!(adaptor.source().now(), NetClockTimePoint::new(33));
        assert!(
            adaptor
                .source()
                .acquire_ledger(&Uint256::from_u64(1))
                .is_none()
        );
        let _ = adaptor.journal();
    }

    #[derive(Default)]
    struct TrustSource;

    impl super::RclValidationTrustSource for TrustSource {
        fn get_trusted_key(&self, identity: &protocol::PublicKey) -> Option<protocol::PublicKey> {
            Some(*identity)
        }

        fn get_listed_key(&self, _identity: &protocol::PublicKey) -> Option<protocol::PublicKey> {
            None
        }
    }

    struct NoopAcceptSink;

    impl super::RclValidationAcceptanceSink for NoopAcceptSink {
        fn check_accept(&self, _hash: Uint256, _seq: u32) {}
    }

    #[test]
    fn validation_store_records_current_trusted_validations() {
        let source = MockSource::new(1_000);
        let mut validations = RclValidations::new(
            AppRclValidationsAdaptor::new(source, NullRclValidationJournal),
            ConsensusParms::default(),
        );
        let store = AppRclValidationStore::new(8);
        let ledger_id = Uint256::from_u64(333);
        let mut validation = signed_validation(10, ledger_id, 88, true);

        let outcome = handle_new_validation_with_store(
            &TrustSource,
            &mut validations,
            &mut validation,
            false,
            None::<&NoopAcceptSink>,
            Some(&store),
            None::<&NullRclValidationJournal>,
        );

        assert_eq!(outcome, ValidationStatus::Current);
        assert_eq!(store.trusted_for_ledger_by_sequence(ledger_id, 88).len(), 1);
        assert_eq!(
            store.tracked_keys(ledger_id, 88),
            std::iter::once(*validation.get_signer_public()).collect()
        );
    }

    #[test]
    fn validation_store_prunes_entries_older_than_retention_window() {
        let store = AppRclValidationStore::new(2);
        store.record(&signed_validation(11, Uint256::from_u64(1), 10, true));
        store.record(&signed_validation(12, Uint256::from_u64(2), 11, true));
        store.record(&signed_validation(13, Uint256::from_u64(3), 13, true));

        assert!(
            store
                .trusted_for_ledger_by_sequence(Uint256::from_u64(1), 10)
                .is_empty()
        );
        assert_eq!(
            store
                .trusted_for_ledger_by_sequence(Uint256::from_u64(2), 11)
                .len(),
            1
        );
        assert_eq!(
            store
                .trusted_for_ledger_by_sequence(Uint256::from_u64(3), 13)
                .len(),
            1
        );
    }
}
