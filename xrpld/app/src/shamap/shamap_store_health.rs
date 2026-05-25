use crate::ledger::ledger_master_state::SharedLedgerMasterState;
use crate::network::network_ops::{NetworkOpsOperatingMode, SharedNetworkOpsState};
use crate::state::time_keeper::{TimeKeeper, TimeKeeperClock};
use ledger::Ledger;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU32, Ordering};
use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SHAMapStoreOperatingMode {
    Full,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SHAMapStoreHealthStatus {
    KeepGoing,
    Waiting(Duration),
    Stopping,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SHAMapStoreHealthPolicy {
    pub age_threshold: Duration,
    pub recovery_wait: Duration,
}

pub trait SHAMapStoreHealthRuntime {
    fn is_stopping(&self) -> bool;
    fn operating_mode(&self) -> SHAMapStoreOperatingMode;
    fn validated_ledger_age(&self) -> Duration;
}

pub trait SHAMapStoreCloseTimeProvider: Send + Sync + 'static {
    fn current_close_time(&self) -> u32;
}

impl<C> SHAMapStoreCloseTimeProvider for TimeKeeper<C>
where
    C: TimeKeeperClock,
{
    fn current_close_time(&self) -> u32 {
        self.close_time().as_seconds()
    }
}

pub struct SharedSHAMapStoreHealthState {
    close_time_provider: Arc<dyn SHAMapStoreCloseTimeProvider>,
    ledger_master_state: Option<Arc<SharedLedgerMasterState>>,
    network_ops_state: Option<Arc<SharedNetworkOpsState>>,
    stopping: AtomicBool,
    operating_mode: AtomicU8,
    validated_ledger_close_time: AtomicU32,
    validated_ledger: Mutex<Option<Arc<Ledger>>>,
}

impl std::fmt::Debug for SharedSHAMapStoreHealthState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SharedSHAMapStoreHealthState")
            .field("stopping", &self.is_stopping())
            .field("operating_mode", &self.operating_mode())
            .field("validated_ledger_seq", &self.validated_ledger_seq())
            .field("validated_ledger_age", &self.validated_ledger_age())
            .finish()
    }
}

impl SharedSHAMapStoreHealthState {
    pub fn new(close_time_provider: Arc<dyn SHAMapStoreCloseTimeProvider>) -> Self {
        Self::with_owner_states(close_time_provider, None, None)
    }

    pub fn new_with_network_ops(
        close_time_provider: Arc<dyn SHAMapStoreCloseTimeProvider>,
        network_ops_state: Arc<SharedNetworkOpsState>,
    ) -> Self {
        Self::with_owner_states(close_time_provider, Some(network_ops_state), None)
    }

    pub fn new_with_app_state(
        close_time_provider: Arc<dyn SHAMapStoreCloseTimeProvider>,
        network_ops_state: Arc<SharedNetworkOpsState>,
        ledger_master_state: Arc<SharedLedgerMasterState>,
    ) -> Self {
        Self::with_owner_states(
            close_time_provider,
            Some(network_ops_state),
            Some(ledger_master_state),
        )
    }

    fn with_owner_states(
        close_time_provider: Arc<dyn SHAMapStoreCloseTimeProvider>,
        network_ops_state: Option<Arc<SharedNetworkOpsState>>,
        ledger_master_state: Option<Arc<SharedLedgerMasterState>>,
    ) -> Self {
        Self {
            close_time_provider,
            ledger_master_state,
            network_ops_state,
            stopping: AtomicBool::new(false),
            operating_mode: AtomicU8::new(encode_operating_mode(SHAMapStoreOperatingMode::Other)),
            validated_ledger_close_time: AtomicU32::new(0),
            validated_ledger: Mutex::new(None),
        }
    }

    pub fn set_stopping(&self, stopping: bool) {
        self.stopping.store(stopping, Ordering::Release);
    }

    pub fn set_operating_mode(&self, operating_mode: SHAMapStoreOperatingMode) {
        self.operating_mode
            .store(encode_operating_mode(operating_mode), Ordering::Release);
    }

    pub fn note_validated_ledger(&self, ledger: Arc<Ledger>) {
        self.set_validated_ledger_close_time(ledger.header().close_time);
        if let Some(ledger_master_state) = &self.ledger_master_state {
            ledger_master_state.note_validated_ledger(Arc::clone(&ledger));
        }
        *self
            .validated_ledger
            .lock()
            .expect("validated ledger mutex must not be poisoned") = Some(ledger);
    }

    pub fn set_validated_ledger_close_time(&self, close_time: u32) {
        if let Some(ledger_master_state) = &self.ledger_master_state {
            ledger_master_state.set_validated_close_time(close_time);
        }
        self.validated_ledger_close_time
            .store(close_time, Ordering::Release);
    }

    pub fn set_validated_ledger_age(&self, age: Duration) {
        let now_close_time = self.close_time_provider.current_close_time();
        let age_seconds = age.as_secs();
        let age_seconds = u32::try_from(age_seconds).unwrap_or(u32::MAX);
        self.set_validated_ledger_close_time(now_close_time.saturating_sub(age_seconds));
        *self
            .validated_ledger
            .lock()
            .expect("validated ledger mutex must not be poisoned") = None;
    }

    pub fn validated_ledger(&self) -> Option<Arc<Ledger>> {
        if let Some(ledger_master_state) = &self.ledger_master_state {
            return ledger_master_state.validated_ledger();
        }
        self.validated_ledger
            .lock()
            .expect("validated ledger mutex must not be poisoned")
            .clone()
    }

    pub fn validated_ledger_seq(&self) -> Option<u32> {
        self.validated_ledger()
            .as_ref()
            .map(|ledger| ledger.header().seq)
    }

    pub fn clear_validated_ledger(&self) {
        if let Some(ledger_master_state) = &self.ledger_master_state {
            ledger_master_state.clear_validated_ledger();
        }
        *self
            .validated_ledger
            .lock()
            .expect("validated ledger mutex must not be poisoned") = None;
        self.validated_ledger_close_time.store(0, Ordering::Release);
    }
}

impl SHAMapStoreHealthRuntime for SharedSHAMapStoreHealthState {
    fn is_stopping(&self) -> bool {
        self.stopping.load(Ordering::Acquire)
    }

    fn operating_mode(&self) -> SHAMapStoreOperatingMode {
        if let Some(network_ops_state) = &self.network_ops_state {
            return map_network_ops_operating_mode(network_ops_state.operating_mode());
        }
        decode_operating_mode(self.operating_mode.load(Ordering::Acquire))
    }

    fn validated_ledger_age(&self) -> Duration {
        if let Some(ledger_master_state) = &self.ledger_master_state {
            return ledger_master_state.validated_ledger_age();
        }
        let validated_close_time = self.validated_ledger_close_time.load(Ordering::Acquire);
        if validated_close_time == 0 {
            return Duration::from_secs(14 * 24 * 60 * 60);
        }

        Duration::from_secs(u64::from(
            self.close_time_provider
                .current_close_time()
                .saturating_sub(validated_close_time),
        ))
    }
}

const fn encode_operating_mode(mode: SHAMapStoreOperatingMode) -> u8 {
    match mode {
        SHAMapStoreOperatingMode::Full => 1,
        SHAMapStoreOperatingMode::Other => 0,
    }
}

const fn decode_operating_mode(mode: u8) -> SHAMapStoreOperatingMode {
    match mode {
        1 => SHAMapStoreOperatingMode::Full,
        _ => SHAMapStoreOperatingMode::Other,
    }
}

const fn map_network_ops_operating_mode(mode: NetworkOpsOperatingMode) -> SHAMapStoreOperatingMode {
    match mode {
        NetworkOpsOperatingMode::Full => SHAMapStoreOperatingMode::Full,
        NetworkOpsOperatingMode::Disconnected
        | NetworkOpsOperatingMode::Connected
        | NetworkOpsOperatingMode::Syncing
        | NetworkOpsOperatingMode::Tracking => SHAMapStoreOperatingMode::Other,
    }
}

impl SHAMapStoreHealthPolicy {
    pub fn evaluate<R>(&self, runtime: &R) -> SHAMapStoreHealthStatus
    where
        R: SHAMapStoreHealthRuntime + ?Sized,
    {
        if runtime.is_stopping() {
            return SHAMapStoreHealthStatus::Stopping;
        }

        if runtime.operating_mode() != SHAMapStoreOperatingMode::Full
            || runtime.validated_ledger_age() > self.age_threshold
        {
            return SHAMapStoreHealthStatus::Waiting(self.recovery_wait);
        }

        SHAMapStoreHealthStatus::KeepGoing
    }
}

pub fn wait_for_health<R, Sleep>(
    policy: &SHAMapStoreHealthPolicy,
    runtime: &mut R,
    mut sleep: Sleep,
) -> SHAMapStoreHealthStatus
where
    R: SHAMapStoreHealthRuntime + ?Sized,
    Sleep: FnMut(&mut R, Duration),
{
    loop {
        match policy.evaluate(runtime) {
            SHAMapStoreHealthStatus::Waiting(duration) => sleep(runtime, duration),
            status => return status,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        SHAMapStoreCloseTimeProvider, SHAMapStoreHealthPolicy, SHAMapStoreHealthRuntime,
        SHAMapStoreHealthStatus, SHAMapStoreOperatingMode, SharedSHAMapStoreHealthState,
        wait_for_health,
    };
    use crate::ledger::ledger_master_state::{
        LedgerMasterCloseTimeProvider, SharedLedgerMasterState,
    };
    use crate::network::network_ops::{NetworkOpsOperatingMode, SharedNetworkOpsState};
    use ledger::Ledger;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};
    use std::time::Duration;

    struct Runtime {
        stopping: bool,
        mode: SHAMapStoreOperatingMode,
        age: Duration,
    }

    impl SHAMapStoreHealthRuntime for Runtime {
        fn is_stopping(&self) -> bool {
            self.stopping
        }

        fn operating_mode(&self) -> SHAMapStoreOperatingMode {
            self.mode
        }

        fn validated_ledger_age(&self) -> Duration {
            self.age
        }
    }

    #[test]
    fn health_policy_wait_gate() {
        let policy = SHAMapStoreHealthPolicy {
            age_threshold: Duration::from_secs(60),
            recovery_wait: Duration::from_secs(5),
        };

        assert_eq!(
            policy.evaluate(&Runtime {
                stopping: false,
                mode: SHAMapStoreOperatingMode::Other,
                age: Duration::from_secs(1),
            }),
            SHAMapStoreHealthStatus::Waiting(Duration::from_secs(5))
        );
        assert_eq!(
            policy.evaluate(&Runtime {
                stopping: false,
                mode: SHAMapStoreOperatingMode::Full,
                age: Duration::from_secs(61),
            }),
            SHAMapStoreHealthStatus::Waiting(Duration::from_secs(5))
        );
        assert_eq!(
            policy.evaluate(&Runtime {
                stopping: true,
                mode: SHAMapStoreOperatingMode::Full,
                age: Duration::from_secs(1),
            }),
            SHAMapStoreHealthStatus::Stopping
        );
    }

    #[test]
    fn health_wait_loops_until_runtime_is_healthy() {
        struct LoopingRuntime {
            calls: Arc<AtomicUsize>,
        }

        impl SHAMapStoreHealthRuntime for LoopingRuntime {
            fn is_stopping(&self) -> bool {
                false
            }

            fn operating_mode(&self) -> SHAMapStoreOperatingMode {
                if self.calls.load(Ordering::Relaxed) < 2 {
                    SHAMapStoreOperatingMode::Other
                } else {
                    SHAMapStoreOperatingMode::Full
                }
            }

            fn validated_ledger_age(&self) -> Duration {
                Duration::from_secs(1)
            }
        }

        let calls = Arc::new(AtomicUsize::new(0));
        let mut sleeps = Vec::new();
        let status = wait_for_health(
            &SHAMapStoreHealthPolicy {
                age_threshold: Duration::from_secs(60),
                recovery_wait: Duration::from_secs(5),
            },
            &mut LoopingRuntime {
                calls: Arc::clone(&calls),
            },
            |runtime, duration| {
                sleeps.push(duration);
                runtime.calls.fetch_add(1, Ordering::Relaxed);
            },
        );

        assert_eq!(status, SHAMapStoreHealthStatus::KeepGoing);
        assert_eq!(sleeps, vec![Duration::from_secs(5), Duration::from_secs(5)]);
    }

    #[derive(Debug)]
    struct FixedCloseTimeProvider {
        now_close_time: AtomicU32,
    }

    impl FixedCloseTimeProvider {
        fn new(now_close_time: u32) -> Self {
            Self {
                now_close_time: AtomicU32::new(now_close_time),
            }
        }
    }

    impl SHAMapStoreCloseTimeProvider for FixedCloseTimeProvider {
        fn current_close_time(&self) -> u32 {
            self.now_close_time.load(Ordering::Acquire)
        }
    }

    impl LedgerMasterCloseTimeProvider for FixedCloseTimeProvider {
        fn current_close_time(&self) -> u32 {
            self.now_close_time.load(Ordering::Acquire)
        }
    }

    #[test]
    fn shared_health_state_recomputes_age_from_validated_close_time() {
        let close_time = Arc::new(FixedCloseTimeProvider::new(120));
        let state = SharedSHAMapStoreHealthState::new(close_time.clone());

        assert_eq!(
            state.validated_ledger_age(),
            Duration::from_secs(14 * 24 * 60 * 60)
        );

        state.note_validated_ledger(Arc::new(Ledger::from_ledger_seq_and_close_time(
            1_156, 100, false,
        )));
        assert_eq!(state.validated_ledger_seq(), Some(1_156));
        assert_eq!(state.validated_ledger_age(), Duration::from_secs(20));

        close_time.now_close_time.store(125, Ordering::Release);
        assert_eq!(state.validated_ledger_age(), Duration::from_secs(25));
    }

    #[test]
    fn shared_health_state_backfills_close_time_from_requested_age() {
        let state = SharedSHAMapStoreHealthState::new(Arc::new(FixedCloseTimeProvider::new(200)));

        state.set_operating_mode(SHAMapStoreOperatingMode::Full);
        state.set_validated_ledger_age(Duration::from_secs(7));
        state.set_stopping(true);

        assert!(state.is_stopping());
        assert_eq!(state.operating_mode(), SHAMapStoreOperatingMode::Full);
        assert_eq!(state.validated_ledger_seq(), None);
        assert_eq!(state.validated_ledger_age(), Duration::from_secs(7));
    }

    #[test]
    fn shared_health_state_can_clear_validated_ledger_and_reset_age() {
        let state = SharedSHAMapStoreHealthState::new(Arc::new(FixedCloseTimeProvider::new(200)));
        state.note_validated_ledger(Arc::new(Ledger::from_ledger_seq_and_close_time(
            1_200, 180, false,
        )));
        assert_eq!(state.validated_ledger_seq(), Some(1_200));

        state.clear_validated_ledger();

        assert_eq!(state.validated_ledger_seq(), None);
        assert_eq!(
            state.validated_ledger_age(),
            Duration::from_secs(14 * 24 * 60 * 60)
        );
    }

    #[test]
    fn shared_health_state_can_follow_live_network_ops_mode() {
        let network_ops = Arc::new(SharedNetworkOpsState::new(
            NetworkOpsOperatingMode::Disconnected,
        ));
        let state = SharedSHAMapStoreHealthState::new_with_network_ops(
            Arc::new(FixedCloseTimeProvider::new(200)),
            network_ops.clone(),
        );

        assert_eq!(state.operating_mode(), SHAMapStoreOperatingMode::Other);

        network_ops.set_operating_mode(NetworkOpsOperatingMode::Tracking);
        assert_eq!(state.operating_mode(), SHAMapStoreOperatingMode::Other);

        network_ops.set_operating_mode(NetworkOpsOperatingMode::Full);
        assert_eq!(state.operating_mode(), SHAMapStoreOperatingMode::Full);
    }

    #[test]
    fn shared_health_state_can_follow_live_ledger_master_age() {
        let close_time = Arc::new(FixedCloseTimeProvider::new(200));
        let network_ops = Arc::new(SharedNetworkOpsState::new(NetworkOpsOperatingMode::Full));
        let ledger_master = Arc::new(SharedLedgerMasterState::new(close_time.clone()));
        let state = SharedSHAMapStoreHealthState::new_with_app_state(
            close_time.clone(),
            network_ops,
            ledger_master.clone(),
        );

        ledger_master.note_validated_ledger(Arc::new(Ledger::from_ledger_seq_and_close_time(
            1_156, 180, false,
        )));
        assert_eq!(state.validated_ledger_seq(), Some(1_156));
        assert_eq!(state.validated_ledger_age(), Duration::from_secs(20));

        close_time.now_close_time.store(208, Ordering::Release);
        assert_eq!(state.validated_ledger_age(), Duration::from_secs(28));
    }
}
