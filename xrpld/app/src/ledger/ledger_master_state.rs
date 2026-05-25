use crate::state::time_keeper::{TimeKeeper, TimeKeeperClock};
use ledger::{Ledger, LedgerMasterCaughtUp};
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

pub trait LedgerMasterCloseTimeProvider: Send + Sync + 'static {
    fn current_close_time(&self) -> u32;
}

impl<C> LedgerMasterCloseTimeProvider for TimeKeeper<C>
where
    C: TimeKeeperClock,
{
    fn current_close_time(&self) -> u32 {
        self.close_time().as_seconds()
    }
}

pub struct SharedLedgerMasterState {
    close_time_provider: Arc<dyn LedgerMasterCloseTimeProvider>,
    closed_ledger: Mutex<Option<Arc<Ledger>>>,
    validated_ledger: Mutex<Option<Arc<Ledger>>>,
    published_ledger: Mutex<Option<Arc<Ledger>>>,
    validated_close_time: AtomicU32,
    published_close_time: AtomicU32,
}

impl std::fmt::Debug for SharedLedgerMasterState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SharedLedgerMasterState")
            .field("closed_ledger_seq", &self.closed_ledger_seq())
            .field("validated_ledger_seq", &self.validated_ledger_seq())
            .field("published_ledger_seq", &self.published_ledger_seq())
            .field("validated_ledger_age", &self.validated_ledger_age())
            .field("published_ledger_age", &self.published_ledger_age())
            .finish()
    }
}

impl SharedLedgerMasterState {
    pub fn new(close_time_provider: Arc<dyn LedgerMasterCloseTimeProvider>) -> Self {
        Self {
            close_time_provider,
            closed_ledger: Mutex::new(None),
            validated_ledger: Mutex::new(None),
            published_ledger: Mutex::new(None),
            validated_close_time: AtomicU32::new(0),
            published_close_time: AtomicU32::new(0),
        }
    }

    pub fn note_closed_ledger(&self, ledger: Arc<Ledger>) {
        *self
            .closed_ledger
            .lock()
            .expect("closed ledger mutex must not be poisoned") = Some(ledger);
    }

    pub fn note_validated_ledger(&self, ledger: Arc<Ledger>) {
        self.set_validated_close_time(ledger.header().close_time);
        *self
            .validated_ledger
            .lock()
            .expect("validated ledger mutex must not be poisoned") = Some(ledger);
    }

    pub fn note_published_ledger(&self, ledger: Arc<Ledger>) {
        self.set_published_close_time(ledger.header().close_time);
        *self
            .published_ledger
            .lock()
            .expect("published ledger mutex must not be poisoned") = Some(ledger);
    }

    pub fn set_validated_close_time(&self, close_time: u32) {
        self.validated_close_time
            .store(close_time, Ordering::Release);
    }

    pub fn set_published_close_time(&self, close_time: u32) {
        self.published_close_time
            .store(close_time, Ordering::Release);
    }

    pub fn clear_validated_ledger(&self) {
        *self
            .validated_ledger
            .lock()
            .expect("validated ledger mutex must not be poisoned") = None;
        self.validated_close_time.store(0, Ordering::Release);
    }

    pub fn closed_ledger(&self) -> Option<Arc<Ledger>> {
        self.closed_ledger
            .lock()
            .expect("closed ledger mutex must not be poisoned")
            .clone()
    }

    pub fn validated_ledger(&self) -> Option<Arc<Ledger>> {
        self.validated_ledger
            .lock()
            .expect("validated ledger mutex must not be poisoned")
            .clone()
    }

    pub fn published_ledger(&self) -> Option<Arc<Ledger>> {
        self.published_ledger
            .lock()
            .expect("published ledger mutex must not be poisoned")
            .clone()
    }

    pub fn closed_ledger_seq(&self) -> Option<u32> {
        self.closed_ledger().map(|ledger| ledger.header().seq)
    }

    pub fn validated_ledger_seq(&self) -> Option<u32> {
        self.validated_ledger().map(|ledger| ledger.header().seq)
    }

    pub fn published_ledger_seq(&self) -> Option<u32> {
        self.published_ledger().map(|ledger| ledger.header().seq)
    }

    pub fn published_ledger_age(&self) -> Duration {
        ledger_age(
            self.published_close_time.load(Ordering::Acquire),
            self.close_time_provider.current_close_time(),
        )
    }

    pub fn validated_ledger_age(&self) -> Duration {
        ledger_age(
            self.validated_close_time.load(Ordering::Acquire),
            self.close_time_provider.current_close_time(),
        )
    }

    pub fn is_caught_up(&self) -> LedgerMasterCaughtUp {
        if self.published_ledger_age() > Duration::from_secs(3 * 60) {
            return LedgerMasterCaughtUp::No {
                reason: "No recently-published ledger",
            };
        }

        let valid_close = self.validated_close_time.load(Ordering::Acquire);
        let pub_close = self.published_close_time.load(Ordering::Acquire);
        if valid_close == 0 || pub_close == 0 {
            return LedgerMasterCaughtUp::No {
                reason: "No published ledger",
            };
        }

        if valid_close > pub_close.saturating_add(90) {
            return LedgerMasterCaughtUp::No {
                reason: "Published ledger lags validated ledger",
            };
        }

        LedgerMasterCaughtUp::Yes
    }
}

fn ledger_age(then_close_time: u32, now_close_time: u32) -> Duration {
    if then_close_time == 0 {
        return Duration::from_secs(14 * 24 * 60 * 60);
    }

    Duration::from_secs(u64::from(now_close_time.saturating_sub(then_close_time)))
}

#[cfg(test)]
mod tests {
    use super::{LedgerMasterCloseTimeProvider, SharedLedgerMasterState};
    use ledger::{Ledger, LedgerMasterCaughtUp};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::time::Duration;

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

    impl LedgerMasterCloseTimeProvider for FixedCloseTimeProvider {
        fn current_close_time(&self) -> u32 {
            self.now_close_time.load(Ordering::Acquire)
        }
    }

    #[test]
    fn ledger_master_state_tracks_live_validated_and_published_age() {
        let close_time = Arc::new(FixedCloseTimeProvider::new(120));
        let state = SharedLedgerMasterState::new(close_time.clone());

        state.note_validated_ledger(Arc::new(Ledger::from_ledger_seq_and_close_time(
            1_156, 100, false,
        )));
        state.note_published_ledger(Arc::new(Ledger::from_ledger_seq_and_close_time(
            1_155, 98, false,
        )));

        assert_eq!(state.validated_ledger_seq(), Some(1_156));
        assert_eq!(state.published_ledger_seq(), Some(1_155));
        assert_eq!(state.validated_ledger_age(), Duration::from_secs(20));
        assert_eq!(state.published_ledger_age(), Duration::from_secs(22));

        close_time.now_close_time.store(130, Ordering::Release);
        assert_eq!(state.validated_ledger_age(), Duration::from_secs(30));
        assert_eq!(state.published_ledger_age(), Duration::from_secs(32));
    }

    #[test]
    fn ledger_master_state_matches_caught_up_reasons() {
        let close_time = Arc::new(FixedCloseTimeProvider::new(500));
        let state = SharedLedgerMasterState::new(close_time.clone());

        assert_eq!(
            state.is_caught_up(),
            LedgerMasterCaughtUp::No {
                reason: "No recently-published ledger",
            }
        );

        state.set_published_close_time(499);
        assert_eq!(
            state.is_caught_up(),
            LedgerMasterCaughtUp::No {
                reason: "No published ledger",
            }
        );

        state.set_validated_close_time(590);
        assert_eq!(
            state.is_caught_up(),
            LedgerMasterCaughtUp::No {
                reason: "Published ledger lags validated ledger",
            }
        );

        state.set_published_close_time(520);
        assert_eq!(state.is_caught_up(), LedgerMasterCaughtUp::Yes);
    }

    #[test]
    fn ledger_master_state_can_clear_validated_owner_state() {
        let state = SharedLedgerMasterState::new(Arc::new(FixedCloseTimeProvider::new(200)));
        state.note_validated_ledger(Arc::new(Ledger::from_ledger_seq_and_close_time(
            1_200, 180, false,
        )));

        state.clear_validated_ledger();

        assert_eq!(state.validated_ledger_seq(), None);
        assert_eq!(
            state.validated_ledger_age(),
            Duration::from_secs(14 * 24 * 60 * 60)
        );
    }
}
