//! Optional application-owned SNTP correction service.
//!
//! rippled no longer owns an SNTP client. Quaxar retains this as an operator
//! convenience, but its worker is explicitly stop-signalled and joined by the
//! application runtime so it cannot outlive the timekeeper it updates.

use std::collections::VecDeque;
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};
use std::time::Duration;

use tokio::net::UdpSocket;
use tokio::sync::watch;
use tokio::time::Instant;

use crate::state::app_registry::AppJournal;

const NTP_QUERY: [u8; 48] = {
    let mut buf = [0u8; 48];
    buf[0] = 0x1B; // LI=0, VN=3, Mode=3 (client)
    buf
};
const NTP_UNIX_OFFSET: i64 = 2_208_988_800;
const NTP_QUERY_FREQUENCY: Duration = Duration::from_secs(240);
const NTP_MIN_QUERY: Duration = Duration::from_secs(180);
const NTP_SAMPLE_WINDOW: usize = 9;
const NTP_TIMESTAMP_VALID: Duration = Duration::from_secs((240 + 180) * 2);

type OffsetSink = Arc<dyn Fn(i64) + Send + Sync + 'static>;

#[derive(Default)]
struct SntpLifecycle {
    stop: std::sync::Mutex<Option<watch::Sender<bool>>>,
    worker: std::sync::Mutex<Option<std::thread::JoinHandle<()>>>,
}

/// A lightweight SNTP client with a single owned worker. Positive offset means
/// the local clock is behind the remote server and should be advanced.
#[derive(Clone)]
pub struct SntpClient {
    offset_secs: Arc<AtomicI64>,
    #[allow(dead_code)]
    journal: Arc<AppJournal>,
    lifecycle: Arc<SntpLifecycle>,
}

impl SntpClient {
    pub fn new(journal: Arc<AppJournal>) -> Self {
        Self {
            offset_secs: Arc::new(AtomicI64::new(0)),
            journal,
            lifecycle: Arc::new(SntpLifecycle::default()),
        }
    }

    /// Compatibility entry point for standalone callers that only need the
    /// offset. ApplicationRoot uses `start` to install its TimeKeeper sink.
    pub fn run(&self, servers: Vec<String>) {
        let _ = self.start(servers, |_| {});
    }

    /// Start the unique owned worker. Repeated starts preserve the first
    /// worker rather than creating detached concurrent NTP loops.
    pub fn start(
        &self,
        servers: Vec<String>,
        on_offset: impl Fn(i64) + Send + Sync + 'static,
    ) -> Result<(), String> {
        if servers.is_empty() {
            return Ok(());
        }

        let mut worker = self.lifecycle.worker.lock().expect("SNTP worker lock");
        if worker.is_some() {
            return Ok(());
        }
        let (stop_tx, stop_rx) = watch::channel(false);
        *self.lifecycle.stop.lock().expect("SNTP stop lock") = Some(stop_tx);
        let client = self.clone();
        let on_offset: OffsetSink = Arc::new(on_offset);
        match std::thread::Builder::new()
            .name("quaxar-sntp".to_owned())
            .spawn(move || {
                let runtime = match tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                {
                    Ok(runtime) => runtime,
                    Err(error) => {
                        tracing::error!(target: "sntp", %error, "SNTP runtime failed to start");
                        return;
                    }
                };
                runtime.block_on(client.query_loop(servers, stop_rx, on_offset));
            }) {
            Ok(handle) => {
                *worker = Some(handle);
                Ok(())
            }
            Err(error) => {
                *self.lifecycle.stop.lock().expect("SNTP stop lock") = None;
                Err(format!("SNTP worker start failed: {error}"))
            }
        }
    }

    /// Signal cancellation and join the worker before its application-owned
    /// TimeKeeper can be destroyed.
    pub fn stop(&self) {
        let worker = {
            let mut worker = self.lifecycle.worker.lock().expect("SNTP worker lock");
            if worker.is_none() {
                return;
            }
            if let Some(stop) = self.lifecycle.stop.lock().expect("SNTP stop lock").take() {
                let _ = stop.send(true);
            }
            worker.take()
        };
        if let Some(worker) = worker {
            let _ = worker.join();
        }
    }

    pub fn offset_seconds(&self) -> i64 {
        self.offset_secs.load(Ordering::Acquire)
    }

    async fn wait_or_stop(delay: Duration, stop: &mut watch::Receiver<bool>) -> bool {
        tokio::select! {
            biased;
            changed = stop.changed() => {
                let _ = changed;
                false
            }
            _ = tokio::time::sleep(delay) => true,
        }
    }

    async fn query_loop(
        &self,
        servers: Vec<String>,
        mut stop: watch::Receiver<bool>,
        on_offset: OffsetSink,
    ) {
        let socket = match UdpSocket::bind("0.0.0.0:0").await {
            Ok(socket) => Arc::new(socket),
            Err(error) => {
                tracing::warn!(target: "sntp", %error, "failed to bind UDP socket");
                return;
            }
        };

        let mut offsets = VecDeque::with_capacity(NTP_SAMPLE_WINDOW);
        let mut last_update: Option<Instant> = None;
        let mut server_state: Vec<(usize, Option<Instant>)> = servers
            .iter()
            .enumerate()
            .map(|(index, _)| (index, None))
            .collect();

        while !*stop.borrow() {
            let target = server_state
                .iter()
                .filter(|(_, last)| last.is_none_or(|time| time.elapsed() >= NTP_MIN_QUERY))
                .min_by_key(|(_, last)| *last)
                .map(|(index, _)| *index);
            let Some(index) = target else {
                if !Self::wait_or_stop(NTP_MIN_QUERY, &mut stop).await {
                    break;
                }
                continue;
            };

            let server = &servers[index];
            let offset = self.query_server(socket.as_ref(), server, &mut stop).await;
            if *stop.borrow() {
                break;
            }
            server_state[index].1 = Some(Instant::now());

            if let Some(offset) = offset {
                if offset.abs() <= 3600 {
                    offsets.push_back(offset);
                    if offsets.len() > NTP_SAMPLE_WINDOW {
                        offsets.pop_front();
                    }
                    let mut sorted = offsets.iter().copied().collect::<Vec<_>>();
                    sorted.sort_unstable();
                    let median = sorted[sorted.len() / 2];
                    let median = if median.abs() <= 1 { 0 } else { median };
                    self.offset_secs.store(median, Ordering::Release);
                    on_offset(median);
                    last_update = Some(Instant::now());
                    tracing::info!(target: "sntp", server, median, samples = offsets.len(), "offset updated");
                } else {
                    tracing::debug!(target: "sntp", server, offset, "rejected unreasonable offset");
                }
            }

            if last_update.is_some_and(|updated| updated.elapsed() > NTP_TIMESTAMP_VALID) {
                self.offset_secs.store(0, Ordering::Release);
                on_offset(0);
                tracing::warn!(target: "sntp", "SNTP offset stale; falling back to system clock");
            }
            if !Self::wait_or_stop(NTP_QUERY_FREQUENCY, &mut stop).await {
                break;
            }
        }
    }

    async fn query_server(
        &self,
        socket: &UdpSocket,
        server: &str,
        stop: &mut watch::Receiver<bool>,
    ) -> Option<i64> {
        let address: SocketAddr = match format!("{server}:123").parse() {
            Ok(address) => address,
            Err(error) => {
                tracing::debug!(target: "sntp", server, %error, "invalid NTP address");
                return None;
            }
        };
        let mut query = NTP_QUERY;
        let nonce = rand_nonce();
        query[44..48].copy_from_slice(&nonce.to_be_bytes());
        let t1 = i64::from(unix_now_secs());

        tokio::select! {
            biased;
            changed = stop.changed() => {
                let _ = changed;
                return None;
            }
            result = socket.send_to(&query, address) => {
                if result.is_err() {
                    return None;
                }
            }
        }

        let mut buffer = [0u8; 256];
        let received = tokio::select! {
            biased;
            changed = stop.changed() => {
                let _ = changed;
                return None;
            }
            result = tokio::time::timeout(Duration::from_secs(2), socket.recv_from(&mut buffer)) => result,
        };
        let Ok(Ok((length, _))) = received else {
            return None;
        };
        if length < 48 {
            return None;
        }
        let t4 = i64::from(unix_now_secs());
        let response_nonce = u32::from_be_bytes([buffer[20], buffer[21], buffer[22], buffer[23]]);
        if response_nonce != nonce || buffer[1] == 0 || buffer[1] > 14 || (buffer[0] >> 6) == 3 {
            return None;
        }
        let t2 = i64::from(u32::from_be_bytes([
            buffer[32], buffer[33], buffer[34], buffer[35],
        ])) - NTP_UNIX_OFFSET;
        let t3 = i64::from(u32::from_be_bytes([
            buffer[40], buffer[41], buffer[42], buffer[43],
        ])) - NTP_UNIX_OFFSET;
        Some(((t2 - t1) + (t3 - t4)) / 2)
    }
}

fn unix_now_secs() -> u32 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as u32
}

fn rand_nonce() -> u32 {
    use std::collections::hash_map::RandomState;
    use std::hash::{BuildHasher, Hasher};
    let state = RandomState::new();
    let mut hasher = state.build_hasher();
    hasher.write_u64(u64::from(unix_now_secs()));
    hasher.finish() as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ntp_query_packet_layout() {
        assert_eq!(NTP_QUERY[0], 0x1B);
        assert_eq!(NTP_QUERY.len(), 48);
        assert_eq!(&NTP_QUERY[44..48], &[0u8; 4]);
        assert_eq!(&NTP_QUERY[12..16], &[0u8; 4]);
    }

    #[test]
    fn offset_median_and_debounce_contract() {
        let mut offsets = vec![10, 11, 10, 10, 10, 10, 10, 10, 100];
        offsets.sort_unstable();
        assert_eq!(offsets[offsets.len() / 2], 10);
        assert_eq!(if 1_i64.abs() <= 1 { 0 } else { 1 }, 0);
    }
}
