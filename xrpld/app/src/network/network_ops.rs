use parking_lot::Mutex;
use protocol::{Ter, is_tef_failure, is_tem_malformed, is_ter_retry, is_tes_success};
use serde_json::{Map, Value};
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::time::{Duration, Instant};
use tx::{ApplyFlags, ApplyResult, CheckValidityResult, Validity};
use xrpl_core::{HashRouterFlags, any};

pub const SUBMIT_INNER_BATCH_MESSAGE: &str =
    "Submitted transaction invalid: tfInnerBatchTxn flag present.";
pub const SUBMIT_CACHED_BAD_MESSAGE: &str = "Submitted transaction cached bad";
pub const SUBMIT_INVALID_PREFIX: &str = "Submitted transaction invalid: ";
pub const SUBMIT_EXCEPTION_PREFIX: &str = "Exception checking transaction ";
pub const PREPROCESS_CACHED_BAD_SUFFIX: &str = ": cached bad!\n";
pub const PREPROCESS_BAD_SIGNATURE_PREFIX: &str = "Transaction has bad signature: ";
pub const NO_TRANSACTION_TO_PROCESS_MESSAGE: &str = "No transaction to process!";
pub const NETWORKOPS_HOLD_LEDGERS: u32 = 5;
const SYNCING_VALIDATED_LEDGER_AGE: Duration = Duration::from_secs(60);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum NetworkOpsOperatingMode {
    Disconnected,
    Connected,
    Syncing,
    Tracking,
    Full,
}

impl NetworkOpsOperatingMode {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Disconnected => "disconnected",
            Self::Connected => "connected",
            Self::Syncing => "syncing",
            Self::Tracking => "tracking",
            Self::Full => "full",
        }
    }
}

pub fn normalize_operating_mode_for_validated_age(
    requested: NetworkOpsOperatingMode,
    validated_ledger_age: Duration,
    is_blocked: bool,
) -> NetworkOpsOperatingMode {
    let mut mode = match requested {
        NetworkOpsOperatingMode::Connected
            if validated_ledger_age < SYNCING_VALIDATED_LEDGER_AGE =>
        {
            NetworkOpsOperatingMode::Syncing
        }
        NetworkOpsOperatingMode::Syncing
            if validated_ledger_age >= SYNCING_VALIDATED_LEDGER_AGE =>
        {
            let age_seconds = validated_ledger_age.as_secs();
            tracing::warn!(target: "app", age_seconds, "Validated ledger is stale");
            NetworkOpsOperatingMode::Connected
        }
        mode => mode,
    };

    if mode > NetworkOpsOperatingMode::Connected && is_blocked {
        mode = NetworkOpsOperatingMode::Connected;
    }

    mode
}

#[derive(Debug, Clone, Copy, Default)]
struct StateAccountingCounters {
    duration_us: u64,
    transitions: u64,
}

#[derive(Debug)]
struct StateAccounting {
    mode: NetworkOpsOperatingMode,
    last_transition: Instant,
    counters: [StateAccountingCounters; 5],
    initial_sync_duration_us: Option<u64>,
}

impl StateAccounting {
    fn new(mode: NetworkOpsOperatingMode) -> Self {
        let mut counters = [StateAccountingCounters::default(); 5];
        counters[encode_operating_mode(mode) as usize].transitions = 1;
        Self {
            mode,
            last_transition: Instant::now(),
            counters,
            initial_sync_duration_us: None,
        }
    }

    fn set_operating_mode(&mut self, new_mode: NetworkOpsOperatingMode) {
        if self.mode == new_mode {
            return;
        }

        let now = Instant::now();
        let duration = now.saturating_duration_since(self.last_transition);
        let old_mode_idx = encode_operating_mode(self.mode) as usize;
        self.counters[old_mode_idx].duration_us += duration.as_micros() as u64;

        if self.initial_sync_duration_us.is_none() && new_mode == NetworkOpsOperatingMode::Full {
            let mut total_sync_duration = 0u64;
            for i in 0..5 {
                total_sync_duration += self.counters[i].duration_us;
            }
            // Add the duration of the state we just left
            self.initial_sync_duration_us = Some(total_sync_duration);
        }

        self.mode = new_mode;
        self.last_transition = now;
        self.counters[encode_operating_mode(new_mode) as usize].transitions += 1;
    }

    fn json(&self) -> Value {
        let now = Instant::now();
        let current_duration = now
            .saturating_duration_since(self.last_transition)
            .as_micros() as u64;

        let mut map = Map::new();
        let modes = [
            NetworkOpsOperatingMode::Disconnected,
            NetworkOpsOperatingMode::Connected,
            NetworkOpsOperatingMode::Syncing,
            NetworkOpsOperatingMode::Tracking,
            NetworkOpsOperatingMode::Full,
        ];

        for mode in modes {
            let idx = encode_operating_mode(mode) as usize;
            let mut mode_counters = self.counters[idx];
            if mode == self.mode {
                mode_counters.duration_us += current_duration;
            }

            let mut obj = Map::new();
            obj.insert(
                "duration_us".to_owned(),
                Value::String(mode_counters.duration_us.to_string()),
            );
            obj.insert(
                "transitions".to_owned(),
                Value::String(mode_counters.transitions.to_string()),
            );
            map.insert(mode.as_str().to_owned(), Value::Object(obj));
        }

        Value::Object(map)
    }

    fn server_state_duration_us(&self) -> String {
        Instant::now()
            .saturating_duration_since(self.last_transition)
            .as_micros()
            .to_string()
    }

    fn initial_sync_duration_us(&self) -> Option<String> {
        self.initial_sync_duration_us.map(|d| d.to_string())
    }
}

#[derive(Debug)]
pub struct SharedNetworkOpsState {
    operating_mode: AtomicU8,
    consensus_mode: AtomicU8,
    need_network_ledger: AtomicBool,
    amendment_blocked: AtomicBool,
    unl_blocked: AtomicBool,
    state_accounting: Mutex<StateAccounting>,
}

impl Default for SharedNetworkOpsState {
    fn default() -> Self {
        Self::new(NetworkOpsOperatingMode::Disconnected)
    }
}

impl SharedNetworkOpsState {
    pub fn new(operating_mode: NetworkOpsOperatingMode) -> Self {
        Self {
            operating_mode: AtomicU8::new(encode_operating_mode(operating_mode)),
            consensus_mode: AtomicU8::new(0),
            need_network_ledger: AtomicBool::new(false),
            amendment_blocked: AtomicBool::new(false),
            unl_blocked: AtomicBool::new(false),
            state_accounting: Mutex::new(StateAccounting::new(operating_mode)),
        }
    }

    pub fn consensus_mode(&self) -> u8 {
        self.consensus_mode.load(Ordering::Acquire)
    }

    pub fn set_consensus_mode(&self, mode: u8) {
        self.consensus_mode.store(mode, Ordering::Release);
    }

    pub fn set_operating_mode(&self, operating_mode: NetworkOpsOperatingMode) {
        self.operating_mode
            .store(encode_operating_mode(operating_mode), Ordering::Release);
        self.state_accounting
            .lock()
            .set_operating_mode(operating_mode);
    }

    pub fn operating_mode(&self) -> NetworkOpsOperatingMode {
        decode_operating_mode(self.operating_mode.load(Ordering::Acquire))
    }

    pub fn is_full(&self) -> bool {
        !self.need_network_ledger() && self.operating_mode() == NetworkOpsOperatingMode::Full
    }

    pub fn is_blocked(&self) -> bool {
        self.amendment_blocked() || self.unl_blocked()
    }

    pub fn str_operating_mode(&self) -> &'static str {
        self.operating_mode().as_str()
    }

    pub fn set_need_network_ledger(&self, need_network_ledger: bool) {
        self.need_network_ledger
            .store(need_network_ledger, Ordering::Release);
    }

    pub fn need_network_ledger(&self) -> bool {
        self.need_network_ledger.load(Ordering::Acquire)
    }

    pub fn set_amendment_blocked(&self, amendment_blocked: bool) {
        self.amendment_blocked
            .store(amendment_blocked, Ordering::Release);
    }

    pub fn amendment_blocked(&self) -> bool {
        self.amendment_blocked.load(Ordering::Acquire)
    }

    pub fn set_unl_blocked(&self, unl_blocked: bool) {
        self.unl_blocked.store(unl_blocked, Ordering::Release);
    }

    pub fn unl_blocked(&self) -> bool {
        self.unl_blocked.load(Ordering::Acquire)
    }

    pub fn state_accounting_json(&self) -> Value {
        self.state_accounting.lock().json()
    }

    pub fn server_state_duration_us(&self) -> String {
        self.state_accounting.lock().server_state_duration_us()
    }

    pub fn initial_sync_duration_us(&self) -> Option<String> {
        self.state_accounting.lock().initial_sync_duration_us()
    }
}

#[derive(Clone)]
pub struct AppNetworkOpsModeOwner {
    state: std::sync::Arc<SharedNetworkOpsState>,
    validated_ledger_age: std::sync::Arc<dyn Fn() -> Duration + Send + Sync>,
}

impl std::fmt::Debug for AppNetworkOpsModeOwner {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("AppNetworkOpsModeOwner")
            .field("operating_mode", &self.operating_mode())
            .field("blocked", &self.is_blocked())
            .finish()
    }
}

impl AppNetworkOpsModeOwner {
    pub fn new(
        state: std::sync::Arc<SharedNetworkOpsState>,
        validated_ledger_age: std::sync::Arc<dyn Fn() -> Duration + Send + Sync>,
    ) -> Self {
        Self {
            state,
            validated_ledger_age,
        }
    }

    pub fn operating_mode(&self) -> NetworkOpsOperatingMode {
        self.state.operating_mode()
    }

    pub fn set_operating_mode(
        &self,
        operating_mode: NetworkOpsOperatingMode,
    ) -> NetworkOpsOperatingMode {
        let previous = self.state.operating_mode();
        self.state
            .set_operating_mode(normalize_operating_mode_for_validated_age(
                operating_mode,
                (self.validated_ledger_age)(),
                self.state.is_blocked(),
            ));
        previous
    }

    /// downgrades where the mode must be set exactly as requested.
    pub fn set_operating_mode_direct(
        &self,
        operating_mode: NetworkOpsOperatingMode,
    ) -> NetworkOpsOperatingMode {
        let previous = self.state.operating_mode();
        self.state.set_operating_mode(operating_mode);
        previous
    }

    pub fn is_blocked(&self) -> bool {
        self.state.is_blocked()
    }

    pub fn set_consensus_mode(&self, mode: u8) {
        self.state.set_consensus_mode(mode);
    }

    pub fn need_network_ledger(&self) -> bool {
        self.state.need_network_ledger()
    }
}

const fn encode_operating_mode(mode: NetworkOpsOperatingMode) -> u8 {
    match mode {
        NetworkOpsOperatingMode::Disconnected => 0,
        NetworkOpsOperatingMode::Connected => 1,
        NetworkOpsOperatingMode::Syncing => 2,
        NetworkOpsOperatingMode::Tracking => 3,
        NetworkOpsOperatingMode::Full => 4,
    }
}

const fn decode_operating_mode(mode: u8) -> NetworkOpsOperatingMode {
    match mode {
        1 => NetworkOpsOperatingMode::Connected,
        2 => NetworkOpsOperatingMode::Syncing,
        3 => NetworkOpsOperatingMode::Tracking,
        4 => NetworkOpsOperatingMode::Full,
        _ => NetworkOpsOperatingMode::Disconnected,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NetworkOpsSubmitDecision {
    Accept,
    RejectInnerBatch,
    RejectCachedBad,
    RejectInvalid(String),
    RejectException(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NetworkOpsPreprocessDecision {
    Continue,
    RejectCachedBad,
    RejectInnerBatch,
    RejectBadSignature(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NetworkOpsSubmitFlowOutcome {
    NeedNetworkLedger,
    Rejected(NetworkOpsSubmitDecision),
    Queued,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetworkOpsSubmitDispatch {
    Returned,
    Queued,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetworkOpsProcessDispatch {
    Rejected,
    Sync,
    Async,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetworkOpsDispatchState {
    None,
    Scheduled,
    Running,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetworkOpsAsyncDispatch {
    AlreadyApplying,
    Enqueued,
    Scheduled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetworkOpsSyncDispatch {
    ExistingApplying,
    Staged,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NetworkOpsSetBuildDecision {
    RejectInvalid { reason: String, set_bad_flag: bool },
    RejectPreprocess,
    Candidate,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetworkOpsBatchDispatch {
    AlreadyRunning,
    AppliedLoop { iterations: usize },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NetworkOpsTransactionSetOutcome {
    NoTransactions,
    SyncBatch { added_count: usize },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NetworkOpsSyncBatchOutcome {
    pub waited: usize,
    pub applied: usize,
    pub scheduled: bool,
    pub dispatch_state: NetworkOpsDispatchState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NetworkOpsSyncOwnerOutcome {
    pub dispatch: NetworkOpsSyncDispatch,
    pub batch: NetworkOpsSyncBatchOutcome,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NetworkOpsApplyResultPreamble {
    pub published: bool,
    pub malformed: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetworkOpsApplyStatusBranch {
    Included,
    Obsolete,
    Queued,
    RetryCandidate,
    Invalid,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetworkOpsRetryHoldBranch {
    FailHard,
    Held { ledgers_left: Option<u32> },
    NotHeld { ledgers_left: Option<u32> },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetworkOpsRelayBranch {
    SkippedEligibility,
    NotRelayed,
    InnerBatchSuppressed,
    Relayed { deferred: bool },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NetworkOpsCurrentLedgerState<Fee, Seq> {
    pub fee: Fee,
    pub account_seq: Seq,
    pub available_seq: Seq,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NetworkOpsApplyBatchTail {
    pub cleared: usize,
    pub pending_transactions: usize,
    pub dispatch_state: NetworkOpsDispatchState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NetworkOpsApplyBatchStart {
    pub taken_transactions: usize,
    pub dispatch_state: NetworkOpsDispatchState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NetworkOpsProcessSetOwnerSync {
    pub added_count: usize,
    pub had_pending_before: bool,
    pub has_applying_after_merge: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NetworkOpsProcessSetFrontDecision<Tx> {
    RejectInvalid { reason: String, set_bad_flag: bool },
    RejectPreprocess,
    Candidate(Tx),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NetworkOpsApplyBatchEntry<Tx> {
    pub transaction: Tx,
    pub admin: bool,
    pub local: bool,
    pub fail_hard: bool,
    pub applied: bool,
    pub result: Option<Ter>,
}

impl<Tx> NetworkOpsApplyBatchEntry<Tx> {
    pub fn new(transaction: Tx, admin: bool, local: bool, fail_hard: bool) -> Self {
        debug_assert!(
            local || !fail_hard,
            "xrpl::NetworkOPsImp::TransactionStatus::TransactionStatus : valid inputs"
        );

        Self {
            transaction,
            admin,
            local,
            fail_hard,
            applied: false,
            result: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NetworkOpsRuntimeState<Pending, Held = Pending> {
    pending_transactions: Vec<Pending>,
    submit_held: Vec<Held>,
    dispatch_state: NetworkOpsDispatchState,
}

impl<Pending, Held> Default for NetworkOpsRuntimeState<Pending, Held> {
    fn default() -> Self {
        Self::new(Vec::new(), Vec::new(), NetworkOpsDispatchState::None)
    }
}

impl<Pending, Held> NetworkOpsRuntimeState<Pending, Held> {
    pub const fn new(
        pending_transactions: Vec<Pending>,
        submit_held: Vec<Held>,
        dispatch_state: NetworkOpsDispatchState,
    ) -> Self {
        Self {
            pending_transactions,
            submit_held,
            dispatch_state,
        }
    }

    pub fn pending_transactions(&self) -> &[Pending] {
        &self.pending_transactions
    }

    pub fn pending_transactions_mut(&mut self) -> &mut Vec<Pending> {
        &mut self.pending_transactions
    }

    pub fn submit_held(&self) -> &[Held] {
        &self.submit_held
    }

    pub fn submit_held_mut(&mut self) -> &mut Vec<Held> {
        &mut self.submit_held
    }

    pub const fn dispatch_state(&self) -> NetworkOpsDispatchState {
        self.dispatch_state
    }

    pub fn transaction_async(
        &mut self,
        applying: bool,
        transaction: Pending,
        set_applying: impl FnOnce(),
        add_batch_job: impl FnOnce() -> bool,
    ) -> NetworkOpsAsyncDispatch {
        let (dispatch, next_state) = run_networkops_transaction_async(
            applying,
            self.dispatch_state,
            || self.pending_transactions.push(transaction),
            set_applying,
            add_batch_job,
        );
        self.dispatch_state = next_state;
        dispatch
    }

    pub fn transaction_batch(
        &mut self,
        mut apply_batch: impl FnMut(&mut Vec<Pending>),
    ) -> NetworkOpsBatchDispatch {
        let dispatch = if self.dispatch_state == NetworkOpsDispatchState::Running {
            NetworkOpsBatchDispatch::AlreadyRunning
        } else {
            let mut iterations = 0;
            while !self.pending_transactions.is_empty() {
                apply_batch(&mut self.pending_transactions);
                iterations += 1;
            }
            NetworkOpsBatchDispatch::AppliedLoop { iterations }
        };
        if matches!(dispatch, NetworkOpsBatchDispatch::AppliedLoop { .. }) {
            self.dispatch_state = NetworkOpsDispatchState::None;
        }
        dispatch
    }

    pub fn process_transaction_set_owner<Tx>(
        &mut self,
        candidates: impl IntoIterator<Item = Tx>,
        is_applying: impl FnMut(&Tx) -> bool,
        stage_transaction: impl FnMut(Tx) -> Pending,
        is_pending_applying: impl FnMut(&Pending) -> bool,
        run_sync_batch: impl FnMut(NetworkOpsProcessSetOwnerSync),
    ) -> NetworkOpsTransactionSetOutcome {
        run_networkops_process_transaction_set_owner(
            candidates,
            &mut self.pending_transactions,
            is_applying,
            stage_transaction,
            is_pending_applying,
            run_sync_batch,
        )
    }

    pub fn process_transaction_set_entrypoint<Input, Candidate>(
        &mut self,
        make_load_event: impl FnOnce(),
        inputs: impl IntoIterator<Item = Input>,
        build_decision: impl FnMut(Input) -> NetworkOpsProcessSetFrontDecision<Candidate>,
        trace_invalid_reason: impl FnMut(&str),
        set_bad_flag: impl FnMut(),
        is_applying: impl FnMut(&Candidate) -> bool,
        stage_transaction: impl FnMut(Candidate) -> Pending,
        is_pending_applying: impl FnMut(&Pending) -> bool,
        run_sync_batch: impl FnMut(NetworkOpsProcessSetOwnerSync),
    ) -> NetworkOpsTransactionSetOutcome {
        run_networkops_process_transaction_set_entrypoint(
            make_load_event,
            inputs,
            &mut self.pending_transactions,
            build_decision,
            trace_invalid_reason,
            set_bad_flag,
            is_applying,
            stage_transaction,
            is_pending_applying,
            run_sync_batch,
        )
    }

    pub fn begin_apply_batch(
        &mut self,
        unlock: impl FnOnce(),
    ) -> (Vec<Pending>, NetworkOpsApplyBatchStart) {
        let (transactions, start) = run_networkops_begin_apply_batch(
            &mut self.pending_transactions,
            self.dispatch_state,
            unlock,
        );
        self.dispatch_state = start.dispatch_state;
        (transactions, start)
    }

    pub fn finish_apply_batch<Tx>(
        &mut self,
        transactions: &[NetworkOpsApplyBatchEntry<Tx>],
        relock: impl FnOnce(),
        clear_applying: impl FnMut(&Tx),
        notify_all: impl FnMut(),
    ) -> NetworkOpsApplyBatchTail
    where
        Held: Into<Pending>,
    {
        relock();
        let mut submit_held = self
            .submit_held
            .drain(..)
            .map(Into::into)
            .collect::<Vec<_>>();
        let tail = run_networkops_apply_batch_tail(
            transactions,
            &mut self.pending_transactions,
            &mut submit_held,
            clear_applying,
            notify_all,
        );
        self.dispatch_state = tail.dispatch_state;
        tail
    }

    pub fn push_submit_held(&mut self, held: Held) {
        self.submit_held.push(held);
    }
}

impl NetworkOpsPreprocessDecision {
    pub fn result_code(&self) -> Option<Ter> {
        match self {
            Self::Continue => None,
            Self::RejectCachedBad | Self::RejectBadSignature(_) => Some(Ter::TEM_BAD_SIGNATURE),
            Self::RejectInnerBatch => Some(Ter::TEM_INVALID_FLAG),
        }
    }

    pub fn should_set_bad_flag(&self) -> bool {
        matches!(self, Self::RejectInnerBatch | Self::RejectBadSignature(_))
    }
}

pub fn format_submit_invalid_message(reason: &str) -> String {
    format!("{SUBMIT_INVALID_PREFIX}{reason}")
}

pub fn format_submit_exception_message(txid: &str, error: &str) -> String {
    format!("{SUBMIT_EXCEPTION_PREFIX}{txid}: {error}")
}

pub fn format_preprocess_cached_bad_message(txid: &str) -> String {
    format!("{txid}{PREPROCESS_CACHED_BAD_SUFFIX}")
}

pub fn format_preprocess_bad_signature_message(reason: &str) -> String {
    format!("{PREPROCESS_BAD_SIGNATURE_PREFIX}{reason}")
}

pub fn no_transaction_to_process_message() -> &'static str {
    NO_TRANSACTION_TO_PROCESS_MESSAGE
}

const fn is_networkops_tel_local(code: Ter) -> bool {
    code.to_int() >= Ter::TEL_LOCAL_ERROR.to_int() && code.to_int() < Ter::TEM_MALFORMED.to_int()
}

pub const fn networkops_enforce_fail_hard(fail_hard: bool, result: Ter) -> bool {
    fail_hard && !is_tes_success(result)
}

pub fn networkops_ledgers_left(
    last_ledger_seq: Option<u32>,
    current_ledger_index: u32,
) -> Option<u32> {
    last_ledger_seq.map(|seq| seq.wrapping_sub(current_ledger_index))
}

pub fn networkops_apply_flags(admin: bool, fail_hard: bool) -> ApplyFlags {
    let mut flags = ApplyFlags::NONE;

    if admin {
        flags |= ApplyFlags::UNLIMITED;
    }

    if fail_hard {
        flags |= ApplyFlags::FAIL_HARD;
    }

    flags
}

pub fn run_networkops_submit_transaction_gate(
    inner_batch_flag_set: bool,
    _feature_batch_enabled: bool,
    get_flags: impl FnOnce() -> HashRouterFlags,
    check_validity: impl FnOnce() -> Result<CheckValidityResult, String>,
) -> NetworkOpsSubmitDecision {
    if inner_batch_flag_set {
        return NetworkOpsSubmitDecision::RejectInnerBatch;
    }

    if any(get_flags() & HashRouterFlags::BAD) {
        return NetworkOpsSubmitDecision::RejectCachedBad;
    }

    match check_validity() {
        Ok(result) if result.validity == Validity::Valid => NetworkOpsSubmitDecision::Accept,
        Ok(result) => NetworkOpsSubmitDecision::RejectInvalid(result.reason),
        Err(error) => NetworkOpsSubmitDecision::RejectException(error),
    }
}

pub fn run_networkops_preprocess_transaction_gate(
    inner_batch_flag_set: bool,
    _feature_batch_enabled: bool,
    get_flags: impl FnOnce() -> HashRouterFlags,
    check_validity: impl FnOnce() -> CheckValidityResult,
) -> NetworkOpsPreprocessDecision {
    if any(get_flags() & HashRouterFlags::BAD) {
        return NetworkOpsPreprocessDecision::RejectCachedBad;
    }

    if inner_batch_flag_set {
        return NetworkOpsPreprocessDecision::RejectInnerBatch;
    }

    let result = check_validity();
    debug_assert_eq!(
        result.validity,
        Validity::Valid,
        "xrpl::NetworkOPsImp::processTransaction : valid validity"
    );

    if result.validity == Validity::SigBad {
        return NetworkOpsPreprocessDecision::RejectBadSignature(result.reason);
    }

    NetworkOpsPreprocessDecision::Continue
}

pub fn run_networkops_submit_transaction(
    need_network_ledger: bool,
    gate: impl FnOnce() -> NetworkOpsSubmitDecision,
    construct_transaction: impl FnOnce(),
    enqueue_process_transaction: impl FnOnce(),
) -> NetworkOpsSubmitFlowOutcome {
    if need_network_ledger {
        return NetworkOpsSubmitFlowOutcome::NeedNetworkLedger;
    }

    match gate() {
        NetworkOpsSubmitDecision::Accept => {
            construct_transaction();
            enqueue_process_transaction();
            NetworkOpsSubmitFlowOutcome::Queued
        }
        decision => NetworkOpsSubmitFlowOutcome::Rejected(decision),
    }
}

pub fn run_networkops_preprocess_transaction(
    decision: NetworkOpsPreprocessDecision,
    mut set_invalid_result: impl FnMut(Ter),
    mut set_bad_flag: impl FnMut(),
    canonicalize: impl FnOnce(),
) -> bool {
    match decision {
        NetworkOpsPreprocessDecision::Continue => {
            canonicalize();
            true
        }
        NetworkOpsPreprocessDecision::RejectCachedBad => {
            set_invalid_result(Ter::TEM_BAD_SIGNATURE);
            false
        }
        NetworkOpsPreprocessDecision::RejectInnerBatch => {
            set_invalid_result(Ter::TEM_INVALID_FLAG);
            set_bad_flag();
            false
        }
        NetworkOpsPreprocessDecision::RejectBadSignature(_) => {
            set_invalid_result(Ter::TEM_BAD_SIGNATURE);
            set_bad_flag();
            false
        }
    }
}

pub fn run_networkops_process_transaction(
    pre_process_transaction: impl FnOnce() -> bool,
    local: bool,
    do_transaction_sync: impl FnOnce(),
    do_transaction_async: impl FnOnce(),
) -> NetworkOpsProcessDispatch {
    if !pre_process_transaction() {
        return NetworkOpsProcessDispatch::Rejected;
    }

    if local {
        do_transaction_sync();
        return NetworkOpsProcessDispatch::Sync;
    }

    do_transaction_async();
    NetworkOpsProcessDispatch::Async
}

pub fn run_networkops_process_transaction_shell(
    make_load_event: impl FnOnce(),
    pre_process_transaction: impl FnOnce() -> bool,
    local: bool,
    do_transaction_sync: impl FnOnce(),
    do_transaction_async: impl FnOnce(),
) -> NetworkOpsProcessDispatch {
    make_load_event();
    run_networkops_process_transaction(
        pre_process_transaction,
        local,
        do_transaction_sync,
        do_transaction_async,
    )
}

pub fn run_networkops_transaction_async(
    applying: bool,
    dispatch_state: NetworkOpsDispatchState,
    push_transaction: impl FnOnce(),
    set_applying: impl FnOnce(),
    add_batch_job: impl FnOnce() -> bool,
) -> (NetworkOpsAsyncDispatch, NetworkOpsDispatchState) {
    if applying {
        return (NetworkOpsAsyncDispatch::AlreadyApplying, dispatch_state);
    }

    push_transaction();
    set_applying();

    if dispatch_state == NetworkOpsDispatchState::None && add_batch_job() {
        return (
            NetworkOpsAsyncDispatch::Scheduled,
            NetworkOpsDispatchState::Scheduled,
        );
    }

    (NetworkOpsAsyncDispatch::Enqueued, dispatch_state)
}

pub fn run_networkops_transaction_async_owner(
    applying: bool,
    dispatch_state: NetworkOpsDispatchState,
    acquire_lock: impl FnOnce(),
    push_transaction: impl FnOnce(),
    set_applying: impl FnOnce(),
    add_batch_job: impl FnOnce() -> bool,
) -> (NetworkOpsAsyncDispatch, NetworkOpsDispatchState) {
    acquire_lock();
    run_networkops_transaction_async(
        applying,
        dispatch_state,
        push_transaction,
        set_applying,
        add_batch_job,
    )
}

pub fn run_networkops_transaction_sync(
    applying: bool,
    stage_transaction: impl FnOnce(),
    run_sync_batch: impl FnOnce(),
) -> NetworkOpsSyncDispatch {
    let dispatch = if applying {
        NetworkOpsSyncDispatch::ExistingApplying
    } else {
        stage_transaction();
        NetworkOpsSyncDispatch::Staged
    };

    run_sync_batch();
    dispatch
}

pub fn run_networkops_transaction_sync_owner<Lock>(
    dispatch_state: NetworkOpsDispatchState,
    applying: bool,
    lock: &mut Lock,
    mut stage_transaction: impl FnMut(&mut Lock),
    mut set_applying: impl FnMut(),
    retry_callback: impl FnMut(&Lock) -> bool,
    wait_for_running_batch: impl FnMut(&mut Lock),
    apply_batch: impl FnMut(&mut Lock),
    has_transactions: impl FnMut(&Lock) -> bool,
    add_sync_batch_job: impl FnMut(&Lock) -> bool,
) -> NetworkOpsSyncOwnerOutcome {
    let dispatch = if applying {
        NetworkOpsSyncDispatch::ExistingApplying
    } else {
        stage_transaction(lock);
        set_applying();
        NetworkOpsSyncDispatch::Staged
    };

    let batch = run_networkops_transaction_sync_batch_owner(
        dispatch_state,
        lock,
        retry_callback,
        wait_for_running_batch,
        apply_batch,
        has_transactions,
        add_sync_batch_job,
    );

    NetworkOpsSyncOwnerOutcome { dispatch, batch }
}

pub fn run_networkops_transaction_sync_batch(
    mut dispatch_state: NetworkOpsDispatchState,
    mut retry_callback: impl FnMut() -> bool,
    mut wait_for_running_batch: impl FnMut(),
    mut apply_batch: impl FnMut(),
    mut has_transactions: impl FnMut() -> bool,
    mut add_sync_batch_job: impl FnMut() -> bool,
) -> NetworkOpsSyncBatchOutcome {
    let mut waited = 0;
    let mut applied = 0;
    let mut scheduled = false;

    loop {
        if dispatch_state == NetworkOpsDispatchState::Running {
            wait_for_running_batch();
            waited += 1;
        } else {
            apply_batch();
            applied += 1;

            if has_transactions() && add_sync_batch_job() {
                dispatch_state = NetworkOpsDispatchState::Scheduled;
                scheduled = true;
            }
        }

        if !retry_callback() {
            break;
        }
    }

    NetworkOpsSyncBatchOutcome {
        waited,
        applied,
        scheduled,
        dispatch_state,
    }
}

pub fn run_networkops_transaction_sync_batch_owner<Lock>(
    mut dispatch_state: NetworkOpsDispatchState,
    lock: &mut Lock,
    mut retry_callback: impl FnMut(&Lock) -> bool,
    mut wait_for_running_batch: impl FnMut(&mut Lock),
    mut apply_batch: impl FnMut(&mut Lock),
    mut has_transactions: impl FnMut(&Lock) -> bool,
    mut add_sync_batch_job: impl FnMut(&Lock) -> bool,
) -> NetworkOpsSyncBatchOutcome {
    let mut waited = 0;
    let mut applied = 0;
    let mut scheduled = false;

    loop {
        if dispatch_state == NetworkOpsDispatchState::Running {
            wait_for_running_batch(lock);
            waited += 1;
        } else {
            apply_batch(lock);
            applied += 1;

            if has_transactions(lock) && add_sync_batch_job(lock) {
                dispatch_state = NetworkOpsDispatchState::Scheduled;
                scheduled = true;
            }
        }

        if !retry_callback(lock) {
            break;
        }
    }

    NetworkOpsSyncBatchOutcome {
        waited,
        applied,
        scheduled,
        dispatch_state,
    }
}

pub fn run_networkops_transaction_batch(
    dispatch_state: NetworkOpsDispatchState,
    mut has_transactions: impl FnMut() -> bool,
    mut apply_batch: impl FnMut(),
) -> NetworkOpsBatchDispatch {
    if dispatch_state == NetworkOpsDispatchState::Running {
        return NetworkOpsBatchDispatch::AlreadyRunning;
    }

    let mut iterations = 0;
    while has_transactions() {
        apply_batch();
        iterations += 1;
    }

    NetworkOpsBatchDispatch::AppliedLoop { iterations }
}

pub fn run_networkops_apply_txq_batch<Tx>(
    transactions: &mut [NetworkOpsApplyBatchEntry<Tx>],
    mut apply_tx: impl FnMut(&Tx, ApplyFlags) -> ApplyResult,
) -> bool {
    let mut changed = false;

    for entry in transactions {
        let result = apply_tx(
            &entry.transaction,
            networkops_apply_flags(entry.admin, entry.fail_hard),
        );
        entry.result = Some(result.ter);
        entry.applied = result.applied;
        changed = changed || result.applied;
    }

    changed
}

pub fn run_networkops_apply_result_preamble<Tx>(
    entry: &NetworkOpsApplyBatchEntry<Tx>,
    mut clear_submit_result: impl FnMut(&Tx),
    mut publish_proposed: impl FnMut(&Tx, Ter),
    mut set_applied: impl FnMut(&Tx),
    mut set_result: impl FnMut(&Tx, Ter),
    mut set_bad_flag: impl FnMut(&Tx),
) -> NetworkOpsApplyResultPreamble {
    let result = entry
        .result
        .expect("xrpl::NetworkOPsImp::apply : apply result must already be set");

    clear_submit_result(&entry.transaction);

    let published = if entry.applied {
        publish_proposed(&entry.transaction, result);
        set_applied(&entry.transaction);
        true
    } else {
        false
    };

    set_result(&entry.transaction, result);

    let malformed = is_tem_malformed(result);
    if malformed {
        set_bad_flag(&entry.transaction);
    }

    NetworkOpsApplyResultPreamble {
        published,
        malformed,
    }
}

pub fn classify_networkops_apply_status(result: Ter) -> NetworkOpsApplyStatusBranch {
    if is_tes_success(result) {
        NetworkOpsApplyStatusBranch::Included
    } else if result == Ter::TEF_PAST_SEQ {
        NetworkOpsApplyStatusBranch::Obsolete
    } else if result == Ter::TER_QUEUED {
        NetworkOpsApplyStatusBranch::Queued
    } else if is_ter_retry(result) || is_networkops_tel_local(result) || is_tef_failure(result) {
        NetworkOpsApplyStatusBranch::RetryCandidate
    } else {
        NetworkOpsApplyStatusBranch::Invalid
    }
}

pub fn run_networkops_apply_status_branch<Tx>(
    entry: &NetworkOpsApplyBatchEntry<Tx>,
    mut on_included: impl FnMut(&Tx),
    mut set_status_included: impl FnMut(&Tx),
    mut set_status_obsolete: impl FnMut(&Tx),
    mut set_status_held: impl FnMut(&Tx),
    mut add_held_transaction: impl FnMut(&Tx),
    mut set_queued: impl FnMut(&Tx),
    mut set_kept: impl FnMut(&Tx),
    mut set_status_invalid: impl FnMut(&Tx),
) -> NetworkOpsApplyStatusBranch {
    let result = entry
        .result
        .expect("xrpl::NetworkOPsImp::apply : apply result must already be set");

    match classify_networkops_apply_status(result) {
        NetworkOpsApplyStatusBranch::Included => {
            set_status_included(&entry.transaction);
            on_included(&entry.transaction);
            NetworkOpsApplyStatusBranch::Included
        }
        NetworkOpsApplyStatusBranch::Obsolete => {
            set_status_obsolete(&entry.transaction);
            NetworkOpsApplyStatusBranch::Obsolete
        }
        NetworkOpsApplyStatusBranch::Queued => {
            set_status_held(&entry.transaction);
            add_held_transaction(&entry.transaction);
            set_queued(&entry.transaction);
            set_kept(&entry.transaction);
            NetworkOpsApplyStatusBranch::Queued
        }
        NetworkOpsApplyStatusBranch::RetryCandidate => NetworkOpsApplyStatusBranch::RetryCandidate,
        NetworkOpsApplyStatusBranch::Invalid => {
            set_status_invalid(&entry.transaction);
            NetworkOpsApplyStatusBranch::Invalid
        }
    }
}

pub fn run_networkops_retry_hold_branch<Tx>(
    entry: &NetworkOpsApplyBatchEntry<Tx>,
    current_ledger_index: u32,
    last_ledger_seq: Option<u32>,
    mut set_held_flag: impl FnMut(&Tx) -> bool,
    mut set_status_held: impl FnMut(&Tx),
    mut add_held_transaction: impl FnMut(&Tx),
    mut set_kept: impl FnMut(&Tx),
) -> NetworkOpsRetryHoldBranch {
    if entry.fail_hard {
        return NetworkOpsRetryHoldBranch::FailHard;
    }

    let ledgers_left = networkops_ledgers_left(last_ledger_seq, current_ledger_index);
    let should_hold = entry.local
        || ledgers_left.is_some_and(|left| left <= NETWORKOPS_HOLD_LEDGERS)
        || set_held_flag(&entry.transaction);

    if should_hold {
        set_status_held(&entry.transaction);
        add_held_transaction(&entry.transaction);
        set_kept(&entry.transaction);
        NetworkOpsRetryHoldBranch::Held { ledgers_left }
    } else {
        NetworkOpsRetryHoldBranch::NotHeld { ledgers_left }
    }
}

pub fn run_networkops_local_keep<Tx>(
    entry: &NetworkOpsApplyBatchEntry<Tx>,
    result: Ter,
    mut push_local: impl FnMut(&Tx),
    mut set_kept: impl FnMut(&Tx),
) -> bool {
    if entry.local && !networkops_enforce_fail_hard(entry.fail_hard, result) {
        push_local(&entry.transaction);
        set_kept(&entry.transaction);
        true
    } else {
        false
    }
}

pub fn run_networkops_relay_branch<Tx, Skip>(
    entry: &NetworkOpsApplyBatchEntry<Tx>,
    operating_mode_full: bool,
    result: Ter,
    inner_batch_flag_set: bool,
    mut should_relay: impl FnMut(&Tx) -> Option<Skip>,
    mut relay: impl FnMut(&Tx, bool, Skip),
    mut set_broadcast: impl FnMut(&Tx),
) -> NetworkOpsRelayBranch {
    let should_consider_relay = !networkops_enforce_fail_hard(entry.fail_hard, result)
        && (entry.applied
            || (!operating_mode_full && !entry.fail_hard && entry.local)
            || result == Ter::TER_QUEUED);

    if !should_consider_relay {
        return NetworkOpsRelayBranch::SkippedEligibility;
    }

    let Some(to_skip) = should_relay(&entry.transaction) else {
        return NetworkOpsRelayBranch::NotRelayed;
    };

    if inner_batch_flag_set {
        return NetworkOpsRelayBranch::InnerBatchSuppressed;
    }

    let deferred = result == Ter::TER_QUEUED;
    relay(&entry.transaction, deferred, to_skip);
    set_broadcast(&entry.transaction);
    NetworkOpsRelayBranch::Relayed { deferred }
}

pub fn run_networkops_set_current_ledger_state<Tx, Fee, Seq>(
    entry: &NetworkOpsApplyBatchEntry<Tx>,
    validated_ledger_index: Option<u32>,
    mut get_current_ledger_state: impl FnMut(&Tx) -> NetworkOpsCurrentLedgerState<Fee, Seq>,
    mut set_current_ledger_state: impl FnMut(&Tx, u32, NetworkOpsCurrentLedgerState<Fee, Seq>),
) -> bool {
    let Some(validated_ledger_index) = validated_ledger_index else {
        return false;
    };

    let state = get_current_ledger_state(&entry.transaction);
    set_current_ledger_state(&entry.transaction, validated_ledger_index, state);
    true
}

pub fn run_networkops_begin_apply_batch<T>(
    pending_transactions: &mut Vec<T>,
    dispatch_state: NetworkOpsDispatchState,
    unlock: impl FnOnce(),
) -> (Vec<T>, NetworkOpsApplyBatchStart) {
    debug_assert!(
        !pending_transactions.is_empty(),
        "xrpl::NetworkOPsImp::apply : non-empty transactions"
    );
    debug_assert!(
        dispatch_state != NetworkOpsDispatchState::Running,
        "xrpl::NetworkOPsImp::apply : is not running"
    );

    let mut transactions = Vec::new();
    std::mem::swap(&mut transactions, pending_transactions);
    let taken_transactions = transactions.len();
    unlock();

    (
        transactions,
        NetworkOpsApplyBatchStart {
            taken_transactions,
            dispatch_state: NetworkOpsDispatchState::Running,
        },
    )
}

pub fn run_networkops_merge_pending_transactions<T>(
    pending_transactions: &mut Vec<T>,
    incoming_transactions: &mut Vec<T>,
) -> usize {
    if incoming_transactions.is_empty() {
        return pending_transactions.len();
    }

    if pending_transactions.is_empty() {
        std::mem::swap(pending_transactions, incoming_transactions);
    } else {
        pending_transactions.reserve(incoming_transactions.len());
        pending_transactions.append(incoming_transactions);
    }

    pending_transactions.len()
}

pub fn run_networkops_merge_submit_held<T>(
    pending_transactions: &mut Vec<T>,
    submit_held: &mut Vec<T>,
) -> usize {
    run_networkops_merge_pending_transactions(pending_transactions, submit_held)
}

pub fn run_networkops_process_transaction_set_owner<Tx, Pending>(
    candidates: impl IntoIterator<Item = Tx>,
    pending_transactions: &mut Vec<Pending>,
    mut is_applying: impl FnMut(&Tx) -> bool,
    mut stage_transaction: impl FnMut(Tx) -> Pending,
    mut is_pending_applying: impl FnMut(&Pending) -> bool,
    mut run_sync_batch: impl FnMut(NetworkOpsProcessSetOwnerSync),
) -> NetworkOpsTransactionSetOutcome {
    let had_pending_before = !pending_transactions.is_empty();
    let mut transactions = Vec::new();

    for transaction in candidates {
        if !is_applying(&transaction) {
            transactions.push(stage_transaction(transaction));
        }
    }

    let added_count = transactions.len();
    run_networkops_merge_pending_transactions(pending_transactions, &mut transactions);

    if !had_pending_before && pending_transactions.is_empty() {
        return NetworkOpsTransactionSetOutcome::NoTransactions;
    }

    run_sync_batch(NetworkOpsProcessSetOwnerSync {
        added_count,
        had_pending_before,
        has_applying_after_merge: pending_transactions.iter().any(&mut is_pending_applying),
    });

    NetworkOpsTransactionSetOutcome::SyncBatch { added_count }
}

pub fn run_networkops_transaction_batch_owner(
    dispatch_state: NetworkOpsDispatchState,
    acquire_lock: impl FnOnce(),
    has_transactions: impl FnMut() -> bool,
    apply_batch: impl FnMut(),
) -> NetworkOpsBatchDispatch {
    acquire_lock();
    run_networkops_transaction_batch(dispatch_state, has_transactions, apply_batch)
}

pub fn run_networkops_finish_apply_batch<Tx, Held>(
    transactions: &[NetworkOpsApplyBatchEntry<Tx>],
    pending_transactions: &mut Vec<Held>,
    submit_held: &mut Vec<Held>,
    relock: impl FnOnce(),
    clear_applying: impl FnMut(&Tx),
    notify_all: impl FnMut(),
) -> NetworkOpsApplyBatchTail {
    relock();
    run_networkops_apply_batch_tail(
        transactions,
        pending_transactions,
        submit_held,
        clear_applying,
        notify_all,
    )
}

pub fn run_networkops_apply_batch_tail<Tx, Held>(
    transactions: &[NetworkOpsApplyBatchEntry<Tx>],
    pending_transactions: &mut Vec<Held>,
    submit_held: &mut Vec<Held>,
    mut clear_applying: impl FnMut(&Tx),
    mut notify_all: impl FnMut(),
) -> NetworkOpsApplyBatchTail {
    for entry in transactions {
        clear_applying(&entry.transaction);
    }

    let pending_transactions = run_networkops_merge_submit_held(pending_transactions, submit_held);
    notify_all();

    NetworkOpsApplyBatchTail {
        cleared: transactions.len(),
        pending_transactions,
        dispatch_state: NetworkOpsDispatchState::None,
    }
}

pub fn run_networkops_process_transaction_set<Tx>(
    inputs: impl IntoIterator<Item = Tx>,
    mut build_decision: impl FnMut(&Tx) -> NetworkOpsSetBuildDecision,
    mut trace_invalid_reason: impl FnMut(&str),
    mut set_bad_flag: impl FnMut(),
) -> Vec<Tx> {
    let mut candidates = Vec::new();

    for tx in inputs {
        match build_decision(&tx) {
            NetworkOpsSetBuildDecision::RejectInvalid {
                reason,
                set_bad_flag: set_bad,
            } => {
                if !reason.is_empty() {
                    trace_invalid_reason(&reason);
                }
                if set_bad {
                    set_bad_flag();
                }
            }
            NetworkOpsSetBuildDecision::RejectPreprocess => {}
            NetworkOpsSetBuildDecision::Candidate => candidates.push(tx),
        }
    }

    candidates
}

pub fn run_networkops_process_transaction_set_front<Input, Candidate>(
    inputs: impl IntoIterator<Item = Input>,
    mut build_decision: impl FnMut(Input) -> NetworkOpsProcessSetFrontDecision<Candidate>,
    mut trace_invalid_reason: impl FnMut(&str),
    mut set_bad_flag: impl FnMut(),
) -> Vec<Candidate> {
    let mut candidates = Vec::new();

    for input in inputs {
        match build_decision(input) {
            NetworkOpsProcessSetFrontDecision::RejectInvalid {
                reason,
                set_bad_flag: set_bad,
            } => {
                if !reason.is_empty() {
                    trace_invalid_reason(&reason);
                }
                if set_bad {
                    set_bad_flag();
                }
            }
            NetworkOpsProcessSetFrontDecision::RejectPreprocess => {}
            NetworkOpsProcessSetFrontDecision::Candidate(candidate) => {
                candidates.push(candidate);
            }
        }
    }

    candidates
}

pub fn run_networkops_process_transaction_set_shell<Input, Candidate, Pending>(
    inputs: impl IntoIterator<Item = Input>,
    pending_transactions: &mut Vec<Pending>,
    build_decision: impl FnMut(Input) -> NetworkOpsProcessSetFrontDecision<Candidate>,
    trace_invalid_reason: impl FnMut(&str),
    set_bad_flag: impl FnMut(),
    is_applying: impl FnMut(&Candidate) -> bool,
    stage_transaction: impl FnMut(Candidate) -> Pending,
    is_pending_applying: impl FnMut(&Pending) -> bool,
    run_sync_batch: impl FnMut(NetworkOpsProcessSetOwnerSync),
) -> NetworkOpsTransactionSetOutcome {
    let candidates = run_networkops_process_transaction_set_front(
        inputs,
        build_decision,
        trace_invalid_reason,
        set_bad_flag,
    );

    run_networkops_process_transaction_set_owner(
        candidates,
        pending_transactions,
        is_applying,
        stage_transaction,
        is_pending_applying,
        run_sync_batch,
    )
}

pub fn run_networkops_process_transaction_set_entrypoint<Input, Candidate, Pending>(
    make_load_event: impl FnOnce(),
    inputs: impl IntoIterator<Item = Input>,
    pending_transactions: &mut Vec<Pending>,
    build_decision: impl FnMut(Input) -> NetworkOpsProcessSetFrontDecision<Candidate>,
    trace_invalid_reason: impl FnMut(&str),
    set_bad_flag: impl FnMut(),
    is_applying: impl FnMut(&Candidate) -> bool,
    stage_transaction: impl FnMut(Candidate) -> Pending,
    is_pending_applying: impl FnMut(&Pending) -> bool,
    run_sync_batch: impl FnMut(NetworkOpsProcessSetOwnerSync),
) -> NetworkOpsTransactionSetOutcome {
    make_load_event();
    let queue_size = pending_transactions.len();
    tracing::debug!(target: "network", queue_size, "Transaction submitted to queue");
    run_networkops_process_transaction_set_shell(
        inputs,
        pending_transactions,
        build_decision,
        trace_invalid_reason,
        set_bad_flag,
        is_applying,
        stage_transaction,
        is_pending_applying,
        run_sync_batch,
    )
}

pub fn run_networkops_process_transaction_set_stage<Tx>(
    candidates: impl IntoIterator<Item = Tx>,
    queued_was_empty: bool,
    is_applying: impl Fn(&Tx) -> bool,
    mut set_applying: impl FnMut(&Tx),
    mut merge_new_transactions: impl FnMut(Vec<Tx>),
    run_sync_batch: impl FnOnce(),
) -> NetworkOpsTransactionSetOutcome {
    let mut transactions = Vec::new();

    for transaction in candidates {
        if !is_applying(&transaction) {
            set_applying(&transaction);
            transactions.push(transaction);
        }
    }

    let added_count = transactions.len();
    merge_new_transactions(transactions);

    if queued_was_empty && added_count == 0 {
        return NetworkOpsTransactionSetOutcome::NoTransactions;
    }

    run_sync_batch();
    NetworkOpsTransactionSetOutcome::SyncBatch { added_count }
}

#[cfg(test)]
mod tests;
