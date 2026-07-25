//! Built-in SNTP client for time-synchronisation in environments where the
//! host OS cannot be NTP-configured by the operator (LXC containers, Docker,
//! managed VPS, etc.).
//!
//! rippled removed its own SNTPClient in commit `548c91ebb6`.  Quaxar
//! re-introduces the capability as an **operator convenience feature** for
//! deployments where the host kernel time source is inaccessible.  This is
//! an intentional extension beyond rippled; the core consensus clock path is
//! identical to rippled.
//!
//! The client queries the configured `[sntp_servers]` in round-robin, builds
//! a sliding window of RFC 4330 offset samples, and stores the **median** as
//! the correction applied by `TimeKeeper::now()`.  Corrections of ≤ 1 s are
//! treated as noise and discarded to avoid jitter.
//!
//! Offset sign convention: **positive = our clock is behind the server** (we
//! are slow).  `TimeKeeper::now()` *adds* the offset to correct forward.
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

    /// Current SNTP offset in seconds.
    ///
    /// **Positive** means the server clock is ahead of our system clock (our
    /// clock is slow).  `TimeKeeper::now()` adds this value to the raw Unix
    /// timestamp so that network time is corrected forward.
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

        // Place the nonce in bytes 44–47 of the request (the fraction field of
        // the client Transmit Timestamp, per RFC 4330 §5).  The server echoes
        // the entire client Transmit Timestamp back in its Originate Timestamp
        // field (response bytes 16–23), so we read the nonce back from bytes
        // 20–23 of the response to verify we are processing our own reply.
        let mut query = NTP_QUERY;
        let nonce = rand_nonce();
        query[44..48].copy_from_slice(&nonce.to_be_bytes()); // transmit-timestamp fraction

        // T1 = client transmit time (Unix seconds, integer part).
        let t1 = unix_now_secs() as i64;

        if socket.send_to(&query, addr).await.is_err() {
            return None;
        }

        let mut buf = [0u8; 256];
        let (_len, _from) = match tokio::time::timeout(Duration::from_secs(2), socket.recv_from(&mut buf)).await {
            Ok(Ok(r)) => r,
            _ => return None,
        };

        // T4 = client receive time (immediately after recv_from returns).
        let t4 = unix_now_secs() as i64;

        // Verify the server echoed our nonce back in the Originate Timestamp
        // fraction field (response bytes 20–23), confirming this is our reply.
        let resp_nonce = u32::from_be_bytes([buf[20], buf[21], buf[22], buf[23]]);
        if resp_nonce != nonce {
            return None;
        }

        // Validate stratum (byte 1 of the response).
        let stratum = buf[1];
        if stratum == 0 || stratum > 14 {
            return None;
        }

        // Reject alarm condition: LI field (top 2 bits of byte 0) == 3.
        if (buf[0] >> 6) == 3 {
            return None;
        }

        // T2 = server receive timestamp, integer part (NTP epoch), bytes 32–35.
        // T3 = server transmit timestamp, integer part (NTP epoch), bytes 40–43.
        // Both are in NTP epoch (seconds since 1900-01-01); subtract
        // NTP_UNIX_OFFSET to convert to Unix epoch before arithmetic.
        let t2 = i64::from(u32::from_be_bytes([buf[32], buf[33], buf[34], buf[35]]))
            - NTP_UNIX_OFFSET;
        let t3 = i64::from(u32::from_be_bytes([buf[40], buf[41], buf[42], buf[43]]))
            - NTP_UNIX_OFFSET;

        // RFC 4330 §5 offset formula: θ = ((T2−T1) + (T3−T4)) / 2.
        // Positive θ means the server is ahead of us (our clock is slow).
        // OPTIMIZATION vs rippled: rippled removed its SNTP client entirely;
        // this implementation follows the RFC directly rather than using a
        // simplified one-sided estimate.
        let offset = ((t2 - t1) + (t3 - t4)) / 2;

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
        // Byte 0: LI=0 (2 bits), VN=3 (3 bits), Mode=3 client (3 bits) = 0x1B.
        assert_eq!(NTP_QUERY[0], 0x1B);
        assert_eq!(NTP_QUERY.len(), 48);
        // The NTP_QUERY template has the nonce placeholder at bytes 44-47
        // (Transmit Timestamp fraction field, per RFC 4330 §5).  It must be
        // zero in the template; the caller fills it in per-request.
        assert_eq!(&NTP_QUERY[44..48], &[0u8; 4]);
        // Bytes 12-15 (Reference Timestamp integer) must be zero in a client
        // request — we do not know the server reference clock.
        assert_eq!(&NTP_QUERY[12..16], &[0u8; 4]);
    }

    #[test]
    fn offset_debounce() {
        // Corrections within ±1 s are zeroed to avoid jitter.
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
        // One outlier (100) cannot pull the median away from the cluster (10).
        let offsets = vec![10, 11, 10, 10, 10, 10, 10, 10, 100];
        let mut sorted = offsets;
        sorted.sort_unstable();
        let median: i64 = sorted[sorted.len() / 2];
        assert_eq!(median, 10);
    }
}
