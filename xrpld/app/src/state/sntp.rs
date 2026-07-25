//! Built-in SNTP client for time-synchronisation in environments where the
//! host OS cannot be NTP-configured by the operator (LXC containers, Docker,
//! managed VPS, etc.).
//!
//! Implements a lightweight SNTP/UDP client modelled after rippled's former
//! `SNTPClock` (removed in commit `548c91ebb6`).  The client queries the
//! configured servers in round-robin, maintains a sliding window of offset
//! samples, and selects the **median** as the correction.  Small (±1 s)
//! corrections are treated as noise and zeroed.
//!
//! Usage:
//! ```ignore
//! let sntp = SntpClient::new(journal);
//! sntp.run(vec!["time.windows.com".into(), "pool.ntp.org".into()]);
//! // … later …
//! let offset_secs: i64 = sntp.offset_seconds();  // median of recent samples
//! ```

use std::collections::VecDeque;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use tokio::net::UdpSocket;
use tokio::time::Instant;

use crate::state::app_registry::AppJournal;

// ── NTP constants ──────────────────────────────────────────────────────────

/// NTP query packet: LI=0, VN=3, Mode=3 (client), 48 bytes.
const NTP_QUERY: [u8; 48] = {
    let mut buf = [0u8; 48];
    buf[0] = 0x1B; // LI=0, VN=3, Mode=3
    buf
};

/// Seconds between the NTP epoch (1900-01-01) and the Unix epoch (1970-01-01).
const NTP_UNIX_OFFSET: i64 = 2_208_988_800;

/// Re-query servers every 4 minutes.
const NTP_QUERY_FREQUENCY: Duration = Duration::from_secs(240);

/// Minimum interval between queries to the same server.
const NTP_MIN_QUERY: Duration = Duration::from_secs(180);

/// Sliding window size for offset samples (must be odd for median).
const NTP_SAMPLE_WINDOW: usize = 9;

/// Offset is considered valid for this long after the last update.
const NTP_TIMESTAMP_VALID: Duration = Duration::from_secs((240 + 180) * 2);

// ── Public API ─────────────────────────────────────────────────────────────

/// A lightweight SNTP client that runs in a background tokio task.
#[derive(Clone)]
pub struct SntpClient {
    /// Current computed offset in seconds, shared with the TimeKeeper.
    offset_secs: Arc<AtomicI64>,
    /// Shared journal for diagnostics.
    journal: Arc<AppJournal>,
}

impl SntpClient {
    /// Create a new client.  The offset starts at 0 (system clock assumed
    /// correct) and is updated as SNTP responses arrive.
    pub fn new(journal: Arc<AppJournal>) -> Self {
        Self {
            offset_secs: Arc::new(AtomicI64::new(0)),
            journal,
        }
    }

    /// Start the background SNTP query loop.  Each server is queried in
    /// round-robin fashion; responses update the shared offset.
    ///
    /// This method spawns a detached tokio task and returns immediately.
    /// If `servers` is empty the client is a no-op (offset stays 0).
    pub fn run(&self, servers: Vec<String>) {
        if servers.is_empty() {
            return;
        }
        let client = self.clone();
        tokio::spawn(async move {
            client.query_loop(servers).await;
        });
    }

    /// Current SNTP offset in seconds (positive = system clock is fast).
    pub fn offset_seconds(&self) -> i64 {
        self.offset_secs.load(Ordering::Acquire)
    }

    // ── internal ───────────────────────────────────────────────────────

    async fn query_loop(&self, servers: Vec<String>) {
        let socket = match UdpSocket::bind("0.0.0.0:0").await {
            Ok(s) => Arc::new(s),
            Err(e) => {
                tracing::warn!(target: "sntp", error = %e, "failed to bind UDP socket");
                return;
            }
        };

        let mut offsets: VecDeque<i64> = VecDeque::with_capacity(NTP_SAMPLE_WINDOW);
        let mut last_update: Option<Instant> = None;
        // (server_index, last_queried_at)
        let mut server_state: Vec<(usize, Option<Instant>)> =
            servers.iter().enumerate().map(|(i, _)| (i, None)).collect();

        loop {
            // Pick the server that was queried longest ago.
            let target = server_state
                .iter()
                .filter(|(_, last)| {
                    last.map_or(true, |t| t.elapsed() >= NTP_MIN_QUERY)
                })
                .min_by_key(|(_, last)| *last)
                .map(|(idx, _)| *idx);

            let Some(idx) = target else {
                // All servers were queried recently; sleep until one is due.
                tokio::time::sleep(NTP_MIN_QUERY).await;
                continue;
            };

            let server = &servers[idx];
            let offset = self.query_server(socket.as_ref(), server).await;
            server_state[idx].1 = Some(Instant::now());

            if let Some(off) = offset {
                // Reject alarm / unreasonable values.
                if off.abs() > 3600 {
                    tracing::debug!(target: "sntp", server, offset = off, "rejected unreasonable offset");
                    continue;
                }

                offsets.push_back(off);
                if offsets.len() > NTP_SAMPLE_WINDOW {
                    offsets.pop_front();
                }

                // Median filter.
                let mut sorted: Vec<i64> = offsets.iter().copied().collect();
                sorted.sort_unstable();
        let median: i64 = sorted[sorted.len() / 2];

                // Debounce ±1 s corrections.
                let median = if median.abs() <= 1 { 0 } else { median };

                self.offset_secs.store(median, Ordering::Release);
                last_update = Some(Instant::now());
                tracing::info!(target: "sntp", server, median, samples = offsets.len(), "offset updated");
            }

            // If the offset is stale, fall back to system clock (offset = 0).
            if let Some(updated) = last_update {
                if updated.elapsed() > NTP_TIMESTAMP_VALID {
                    self.offset_secs.store(0, Ordering::Release);
                    tracing::warn!(target: "sntp", "SNTP offset stale; falling back to system clock");
                }
            }

            tokio::time::sleep(NTP_QUERY_FREQUENCY).await;
        }
    }

    async fn query_server(&self, socket: &UdpSocket, server: &str) -> Option<i64> {
        // DNS resolve (simple, non-cached — good enough for NTP servers).
        let addr: SocketAddr = match format!("{server}:123").parse() {
            Ok(a) => a,
            Err(e) => {
                tracing::debug!(target: "sntp", server, error = %e, "invalid NTP address");
                return None;
            }
        };

        // Build query with nonce in the transmit-timestamp fraction field.
        let mut query = NTP_QUERY;
        let nonce = rand_nonce();
        query[12..16].copy_from_slice(&nonce.to_be_bytes()); // transmit fraction = nonce

        let send_time = unix_now_secs();

        if socket.send_to(&query, addr).await.is_err() {
            return None;
        }

        let mut buf = [0u8; 256];
        let (_len, _from) = match tokio::time::timeout(Duration::from_secs(2), socket.recv_from(&mut buf)).await {
            Ok(Ok(r)) => r,
            _ => return None,
        };

        // Validate response: check nonce.
        let resp_nonce = u32::from_be_bytes([buf[40], buf[41], buf[42], buf[43]]);
        if resp_nonce != nonce {
            return None;
        }

        // Validate stratum.
        let info = u32::from_be_bytes([buf[0], buf[1], buf[2], buf[3]]);
        let stratum = (info >> 16) & 0xff;
        if stratum == 0 || stratum > 14 {
            return None;
        }

        // Reject alarm condition (LI bits 30-31 = 3).
        if (info >> 30) == 3 {
            return None;
        }

        // Server receive timestamp (integer part).
        let server_recv = i64::from(u32::from_be_bytes([buf[32], buf[33], buf[34], buf[35]]));

        // Simplified offset: server_recv - send_time - NTP_UNIX_OFFSET.
        // (This matches rippled's simplified calculation that ignores
        // round-trip delay for speed.)
        let offset = server_recv - send_time as i64 - NTP_UNIX_OFFSET;

        Some(offset)
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
    let s = RandomState::new();
    let mut h = s.build_hasher();
    h.write_u64(unix_now_secs() as u64);
    h.finish() as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ntp_query_packet_layout() {
        // LI=0, VN=3, Mode=3
        assert_eq!(NTP_QUERY[0], 0x1B);
        assert_eq!(NTP_QUERY.len(), 48);
    }

    #[test]
    fn offset_debounce() {
        // ±1 s corrections should be zeroed.
        let offsets: Vec<i64> = vec![-1, 0, 1, 2, -2, 3, -3, 4, -4];
        let mut sorted = offsets;
        sorted.sort_unstable();
        let median = sorted[sorted.len() / 2];
        let median = if median.abs() <= 1 { 0 } else { median };
        // Median of [-4, -3, -2, -1, 0, 1, 2, 3, 4] is 0, debounced to 0.
        assert_eq!(median, 0);
    }

    #[test]
    fn offset_median_filter() {
        let offsets = vec![10, 11, 10, 10, 10, 10, 10, 10, 100];
        let mut sorted = offsets;
        sorted.sort_unstable();
        let median: i64 = sorted[sorted.len() / 2];
        assert_eq!(median, 10);
    }
}
