//! App-owned resolver runtime mirroring `Application` ownership of
//! `ResolverAsio`.

use crate::ManagedComponent;
use basics::resolver::Resolver;
use basics::resolver_asio::ResolverAsio;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Debug)]
pub struct AppResolverRuntime {
    resolver: Arc<ResolverAsio>,
    started: AtomicBool,
    stopped: AtomicBool,
}

impl Default for AppResolverRuntime {
    fn default() -> Self {
        Self::new(ResolverAsio::default())
    }
}

impl AppResolverRuntime {
    pub fn new(resolver: ResolverAsio) -> Self {
        Self {
            resolver: Arc::new(resolver),
            started: AtomicBool::new(false),
            stopped: AtomicBool::new(false),
        }
    }

    pub fn resolver(&self) -> Arc<ResolverAsio> {
        Arc::clone(&self.resolver)
    }

    pub fn started(&self) -> bool {
        self.started.load(Ordering::Acquire)
    }

    pub fn stopped(&self) -> bool {
        self.stopped.load(Ordering::Acquire)
    }
}

impl ManagedComponent for AppResolverRuntime {
    fn start(&self) -> Result<(), String> {
        if self.stopped() {
            return Err("resolver runtime has already been stopped".to_owned());
        }
        if self.started.swap(true, Ordering::AcqRel) {
            return Ok(());
        }
        self.resolver.start();
        Ok(())
    }

    fn stop(&self) {
        if self.stopped.swap(true, Ordering::AcqRel) {
            return;
        }
        self.resolver.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::AppResolverRuntime;
    use crate::ManagedComponent;

    #[test]
    fn resolver_runtime_starts_and_stops_owned_resolver() {
        let runtime = AppResolverRuntime::default();

        assert!(!runtime.started());
        assert!(!runtime.stopped());
        runtime.start().expect("resolver should start");
        assert!(runtime.started());
        assert!(!runtime.resolver().is_stopped());
        runtime.stop();
        assert!(runtime.stopped());
        assert!(runtime.resolver().is_stopped());
    }
}
