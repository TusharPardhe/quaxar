use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::load_monitor::LoadMonitor;

#[derive(Debug)]
pub struct LoadEvent {
    monitor: Arc<LoadMonitor>,
    running: bool,
    name: String,
    mark: Instant,
    time_waiting: Duration,
    time_running: Duration,
}

impl LoadEvent {
    pub fn new(monitor: Arc<LoadMonitor>, name: impl Into<String>, should_start: bool) -> Self {
        Self {
            monitor,
            running: should_start,
            name: name.into(),
            mark: Instant::now(),
            time_waiting: Duration::ZERO,
            time_running: Duration::ZERO,
        }
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn wait_time(&self) -> Duration {
        self.time_waiting
    }

    pub fn run_time(&self) -> Duration {
        self.time_running
    }

    pub fn set_name(&mut self, name: impl Into<String>) {
        self.name = name.into();
    }

    pub fn start(&mut self) {
        let now = Instant::now();
        self.time_waiting += now.saturating_duration_since(self.mark);
        self.mark = now;
        self.running = true;
    }

    pub fn stop(&mut self) {
        assert!(self.running, "LoadEvent::stop requires a running event");
        self.stop_inner();
    }

    fn stop_inner(&mut self) {
        let now = Instant::now();
        self.time_running += now.saturating_duration_since(self.mark);
        self.mark = now;
        self.running = false;
        self.monitor.add_load_sample(self);
    }
}

impl Drop for LoadEvent {
    fn drop(&mut self) {
        if self.running {
            self.stop_inner();
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::thread;
    use std::time::Duration;

    use crate::LoadMonitor;

    use super::LoadEvent;

    #[test]
    fn load_event_tracks_wait_and_run_segments() {
        let monitor = Arc::new(LoadMonitor::new());
        let mut event = LoadEvent::new(monitor, "job", false);
        thread::sleep(Duration::from_millis(2));
        event.start();
        thread::sleep(Duration::from_millis(2));
        event.stop();

        assert_eq!(event.name(), "job");
        assert!(event.wait_time() >= Duration::from_millis(1));
        assert!(event.run_time() >= Duration::from_millis(1));
    }
}
