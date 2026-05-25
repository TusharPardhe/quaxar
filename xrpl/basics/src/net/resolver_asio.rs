//! Rust port of `xrpl/basics/ResolverAsio.h`.

use crate::log::Journal;
use crate::resolver::{Endpoint, ResolveHandler, Resolver};
use std::collections::VecDeque;
use std::net::{IpAddr, SocketAddr, ToSocketAddrs};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::{self, JoinHandle};

#[derive(Clone, Debug)]
pub struct ResolverAsio {
    state: Arc<ResolverAsioState>,
}

#[derive(Debug)]
struct ResolverAsioState {
    journal: Option<Journal>,
    stop_called: AtomicBool,
    stopped: AtomicBool,
    worker: Mutex<Option<JoinHandle<()>>>,
    inner: Mutex<ResolverInner>,
    cv: Condvar,
}

#[derive(Default)]
struct ResolverInner {
    work: VecDeque<Work>,
}

struct Work {
    names: Vec<String>,
    handler: ResolveHandler,
}

impl std::fmt::Debug for ResolverInner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ResolverInner")
            .field("queued_work", &self.work.len())
            .finish()
    }
}

impl std::fmt::Debug for Work {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Work")
            .field("remaining_names", &self.names.len())
            .finish()
    }
}

impl Work {
    fn new(names: &[String], handler: ResolveHandler) -> Self {
        let mut reversed = Vec::with_capacity(names.len());
        reversed.extend(names.iter().rev().cloned());
        Self {
            names: reversed,
            handler,
        }
    }
}

impl Default for ResolverAsio {
    fn default() -> Self {
        Self {
            state: Arc::new(ResolverAsioState {
                journal: None,
                stop_called: AtomicBool::new(false),
                stopped: AtomicBool::new(true),
                worker: Mutex::new(None),
                inner: Mutex::new(ResolverInner::default()),
                cv: Condvar::new(),
            }),
        }
    }
}

impl ResolverAsio {
    pub fn with_journal(journal: Journal) -> Self {
        Self {
            state: Arc::new(ResolverAsioState {
                journal: Some(journal),
                stop_called: AtomicBool::new(false),
                stopped: AtomicBool::new(true),
                worker: Mutex::new(None),
                inner: Mutex::new(ResolverInner::default()),
                cv: Condvar::new(),
            }),
        }
    }

    #[allow(non_snake_case)]
    pub fn New<T>(_io_context: T, journal: Journal) -> Box<Self> {
        Box::new(Self::with_journal(journal))
    }

    pub fn is_stopped(&self) -> bool {
        self.state.stopped.load(Ordering::SeqCst)
    }

    pub fn parse_name(name: &str) -> (String, String) {
        if let Ok(endpoint) = name.parse::<SocketAddr>() {
            return (endpoint.ip().to_string(), endpoint.port().to_string());
        }

        let trimmed = name.trim();
        if trimmed.is_empty() {
            return (String::new(), String::new());
        }

        let mut host_end = trimmed.len();
        for (index, ch) in trimmed.char_indices() {
            if ch.is_ascii_whitespace() || ch == ':' {
                host_end = index;
                break;
            }
        }

        let host = trimmed[..host_end].trim().to_owned();
        let port = trimmed[host_end..]
            .trim_matches(|ch: char| ch.is_ascii_whitespace() || ch == ':')
            .to_owned();
        (host, port)
    }

    fn worker_loop(state: Arc<ResolverAsioState>) {
        loop {
            if state.stop_called.load(Ordering::SeqCst) {
                break;
            }

            let task = {
                let mut inner = state.inner.lock().expect("resolver mutex poisoned");
                loop {
                    if state.stop_called.load(Ordering::SeqCst) {
                        break None;
                    }

                    if let Some(work) = inner.work.front_mut()
                        && let Some(name) = work.names.pop()
                    {
                        let handler = Arc::clone(&work.handler);
                        if work.names.is_empty() {
                            inner.work.pop_front();
                        }
                        break Some((name, handler));
                    }

                    inner = state.cv.wait(inner).expect("resolver wait poisoned");
                }
            };

            let Some((name, handler)) = task else {
                break;
            };

            let (host, port) = Self::parse_name(&name);
            if host.is_empty() {
                if let Some(journal) = &state.journal {
                    journal.error(&format!("Unable to parse '{name}'"));
                }
                continue;
            }

            let endpoints = resolve_host(&host, &port);
            if state.stop_called.load(Ordering::SeqCst) {
                break;
            }
            handler(name, endpoints);
        }

        state.stopped.store(true, Ordering::SeqCst);
        state.cv.notify_all();
    }
}

impl Resolver for ResolverAsio {
    fn stop_async(&self) {
        if !self.state.stop_called.swap(true, Ordering::SeqCst) {
            self.state
                .inner
                .lock()
                .expect("resolver mutex poisoned")
                .work
                .clear();
            self.state.cv.notify_all();
        }
    }

    fn stop(&self) {
        self.stop_async();
        if let Some(handle) = self
            .state
            .worker
            .lock()
            .expect("resolver worker mutex poisoned")
            .take()
        {
            let _ = handle.join();
        }
        self.state.stopped.store(true, Ordering::SeqCst);
        self.state.cv.notify_all();
    }

    fn start(&self) {
        assert!(
            self.state.stopped.load(Ordering::SeqCst),
            "ResolverAsio::start requires a stopped resolver"
        );
        assert!(
            !self.state.stop_called.load(Ordering::SeqCst),
            "ResolverAsio::start cannot restart a stopping resolver"
        );

        self.state.stopped.store(false, Ordering::SeqCst);
        let worker_state = Arc::clone(&self.state);
        let handle = thread::Builder::new()
            .name("resolver-asio".to_owned())
            .spawn(move || Self::worker_loop(worker_state))
            .expect("resolver thread");
        *self
            .state
            .worker
            .lock()
            .expect("resolver worker mutex poisoned") = Some(handle);
    }

    fn resolve(&self, names: &[String], handler: ResolveHandler) {
        assert!(
            !names.is_empty(),
            "ResolverAsio::resolve requires a non-empty name list"
        );
        assert!(
            !self.state.stop_called.load(Ordering::SeqCst),
            "ResolverAsio::resolve cannot run while stopping"
        );

        self.state
            .inner
            .lock()
            .expect("resolver mutex poisoned")
            .work
            .push_back(Work::new(names, handler));
        self.state.cv.notify_all();
    }
}

fn resolve_host(host: &str, port: &str) -> Vec<Endpoint> {
    let port_number = port.parse::<u16>().unwrap_or(0);
    if let Ok(ip) = host.parse::<IpAddr>() {
        return vec![SocketAddr::new(ip, port_number)];
    }

    (host, port_number)
        .to_socket_addrs()
        .map(|iter| iter.collect())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::ResolverAsio;
    use crate::log::{LogSeverity, Logs, RecordingLogSink};
    use crate::resolver::Resolver;
    use std::sync::{Arc, Condvar, Mutex};
    use std::time::{Duration, Instant};

    #[test]
    fn parse_name_ip_and_trimmed_host_cases() {
        assert_eq!(
            ResolverAsio::parse_name("127.0.0.1:51235"),
            ("127.0.0.1".to_owned(), "51235".to_owned())
        );
        assert_eq!(
            ResolverAsio::parse_name("  example.com  :  443  "),
            ("example.com".to_owned(), "443".to_owned())
        );
        assert_eq!(
            ResolverAsio::parse_name("   "),
            (String::new(), String::new())
        );
    }

    #[test]
    fn resolve_invokes_handler_async_for_parseable_names_and_skips_invalid_entries() {
        let sink = Arc::new(RecordingLogSink::default());
        let resolver = ResolverAsio::with_journal(
            Logs::with_sink(LogSeverity::Error, sink.clone()).journal("resolver"),
        );
        resolver.start();

        let seen = Arc::new((Mutex::new(Vec::new()), Condvar::new()));
        let seen_handler = Arc::clone(&seen);
        resolver.resolve(
            &["127.0.0.1:8080".to_owned(), "   ".to_owned()],
            Arc::new(move |name, endpoints| {
                let (lock, cv) = &*seen_handler;
                lock.lock()
                    .expect("seen mutex poisoned")
                    .push((name, endpoints));
                cv.notify_all();
            }),
        );

        let (lock, cv) = &*seen;
        let entries = cv
            .wait_timeout_while(
                lock.lock().expect("seen mutex poisoned"),
                Duration::from_secs(2),
                |entries| entries.is_empty(),
            )
            .expect("wait for resolver");
        assert_eq!(entries.0.len(), 1);
        assert_eq!(entries.0[0].0, "127.0.0.1:8080");
        assert_eq!(entries.0[0].1[0].port(), 8080);

        let log_deadline = Instant::now() + Duration::from_secs(2);
        while sink.entries().is_empty() && Instant::now() < log_deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(sink.entries().len(), 1);

        resolver.stop();
        assert!(resolver.is_stopped());
    }

    #[test]
    fn stop_async_clears_pending_work() {
        let resolver = ResolverAsio::default();
        resolver.start();

        let seen = Arc::new((Mutex::new(Vec::new()), Condvar::new()));
        let gate = Arc::new((Mutex::new(false), Condvar::new()));
        let seen_handler = Arc::clone(&seen);
        let gate_handler = Arc::clone(&gate);
        resolver.resolve(
            &[
                "127.0.0.1:8001".to_owned(),
                "127.0.0.1:8002".to_owned(),
                "127.0.0.1:8003".to_owned(),
            ],
            Arc::new(move |name, endpoints| {
                let (lock, cv) = &*seen_handler;
                lock.lock()
                    .expect("seen mutex poisoned")
                    .push((name, endpoints));
                cv.notify_all();

                let (gate_lock, gate_cv) = &*gate_handler;
                let _ = gate_cv
                    .wait_timeout_while(
                        gate_lock.lock().expect("gate mutex poisoned"),
                        Duration::from_secs(2),
                        |released| !*released,
                    )
                    .expect("wait for gate");
            }),
        );

        let (lock, cv) = &*seen;
        let _ = cv
            .wait_timeout_while(
                lock.lock().expect("seen mutex poisoned"),
                Duration::from_secs(2),
                |seen| seen.is_empty(),
            )
            .expect("wait for first callback");
        resolver.stop_async();
        {
            let (gate_lock, gate_cv) = &*gate;
            *gate_lock.lock().expect("gate mutex poisoned") = true;
            gate_cv.notify_all();
        }
        resolver.stop();

        let seen = lock.lock().expect("seen mutex poisoned");
        assert_eq!(seen.len(), 1);
    }
}
