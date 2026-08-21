//! Prometheus metrics for the XRPL Rust node.
//!
//! Exposes rich operational metrics via a `/metrics` HTTP endpoint.
//! Uses the `metrics` crate facade for zero-cost recording when disabled.
//!
//! # Usage
//!
//! ```rust
//! use quaxar_metrics::{init_prometheus, ledger, network, rpc};
//!
//! // Start the Prometheus exporter on port 7005
//! init_prometheus("0.0.0.0:7005");
//!
//! // Record metrics anywhere in the codebase
//! ledger::validated_seq(90_000_000);
//! ledger::record_close_duration(0.85);
//! network::increment_messages_received("TMTransaction");
//! rpc::record_request("server_info", 0.002);
//! ```

use metrics::{counter, gauge, histogram};
use once_cell::sync::OnceCell;

static EXPORTER_HANDLE: OnceCell<metrics_exporter_prometheus::PrometheusHandle> = OnceCell::new();

/// Initialize the Prometheus metrics exporter.
/// Call once at startup. Metrics are served at `http://{addr}/metrics`.
pub fn init_prometheus(addr: &str) -> Result<(), String> {
    let builder = metrics_exporter_prometheus::PrometheusBuilder::new();
    let builder = builder.with_http_listener(
        addr.parse::<std::net::SocketAddr>()
            .map_err(|e| e.to_string())?,
    );

    let handle = builder.install_recorder().map_err(|e| e.to_string())?;
    let _ = EXPORTER_HANDLE.set(handle);
    Ok(())
}

/// Get the Prometheus handle for rendering metrics manually (e.g., in tests).
pub fn prometheus_handle() -> Option<&'static metrics_exporter_prometheus::PrometheusHandle> {
    EXPORTER_HANDLE.get()
}

// ═══════════════════════════════════════════════════════════════
// LEDGER METRICS
// ═══════════════════════════════════════════════════════════════

pub mod ledger {
    use super::*;

    /// Set the current validated ledger sequence.
    pub fn validated_seq(seq: u64) {
        gauge!("quaxar_ledger_validated_seq").set(seq as f64);
    }

    /// Set the current open ledger sequence.
    pub fn current_seq(seq: u64) {
        gauge!("quaxar_ledger_current_seq").set(seq as f64);
    }

    /// Record ledger close duration in seconds.
    pub fn record_close_duration(seconds: f64) {
        histogram!("quaxar_ledger_close_duration_seconds").record(seconds);
    }

    /// Record number of transactions in a closed ledger.
    pub fn record_tx_count(count: u64) {
        histogram!("quaxar_ledger_tx_count").record(count as f64);
    }

    /// Set the number of state entries in the current ledger.
    pub fn state_entries(count: u64) {
        gauge!("quaxar_ledger_state_entries").set(count as f64);
    }

    /// Increment total ledgers closed.
    pub fn increment_closed() {
        counter!("quaxar_ledger_closed_total").increment(1);
    }
}

// ═══════════════════════════════════════════════════════════════
// CONSENSUS METRICS
// ═══════════════════════════════════════════════════════════════

pub mod consensus {
    use super::*;

    /// Record consensus round duration in seconds.
    pub fn record_round_duration(seconds: f64) {
        histogram!("quaxar_consensus_round_duration_seconds").record(seconds);
    }

    /// Set the number of proposers in the current round.
    pub fn proposers(count: u64) {
        gauge!("quaxar_consensus_proposers").set(count as f64);
    }

    /// Set the current operating mode (0=disconnected, 1=connected, 2=syncing, 3=tracking, 4=full).
    pub fn mode(mode: u8) {
        gauge!("quaxar_consensus_mode").set(f64::from(mode));
    }

    /// Increment total validations received.
    pub fn increment_validations_received() {
        counter!("quaxar_validations_received_total").increment(1);
    }

    /// Increment trusted validations.
    pub fn increment_validations_trusted() {
        counter!("quaxar_validations_trusted_total").increment(1);
    }

    /// Increment stale validations (too old).
    pub fn increment_validations_stale() {
        counter!("quaxar_validations_stale_total").increment(1);
    }
}

// ═══════════════════════════════════════════════════════════════
// NETWORK METRICS
// ═══════════════════════════════════════════════════════════════

pub mod network {
    use super::*;

    /// Set total connected peers.
    pub fn peers_connected(count: u64) {
        gauge!("quaxar_peers_connected").set(count as f64);
    }

    /// Set inbound peer count.
    pub fn peers_inbound(count: u64) {
        gauge!("quaxar_peers_inbound").set(count as f64);
    }

    /// Set outbound peer count.
    pub fn peers_outbound(count: u64) {
        gauge!("quaxar_peers_outbound").set(count as f64);
    }

    /// Increment messages received by type.
    pub fn increment_messages_received(msg_type: &str) {
        counter!("quaxar_messages_received_total", "type" => msg_type.to_owned()).increment(1);
    }

    /// Increment messages sent by type.
    pub fn increment_messages_sent(msg_type: &str) {
        counter!("quaxar_messages_sent_total", "type" => msg_type.to_owned()).increment(1);
    }

    /// Add bytes received.
    pub fn add_bytes_received(bytes: u64) {
        counter!("quaxar_bandwidth_rx_bytes_total").increment(bytes);
    }

    /// Add bytes sent.
    pub fn add_bytes_sent(bytes: u64) {
        counter!("quaxar_bandwidth_tx_bytes_total").increment(bytes);
    }
}

// ═══════════════════════════════════════════════════════════════
// TX QUEUE METRICS
// ═══════════════════════════════════════════════════════════════

pub mod txqueue {
    use super::*;

    /// Set current queue size.
    pub fn size(count: u64) {
        gauge!("quaxar_txqueue_size").set(count as f64);
    }

    /// Set current escalated fee level.
    pub fn fee_level(level: u64) {
        gauge!("quaxar_txqueue_fee_level").set(level as f64);
    }

    /// Increment applied transactions.
    pub fn increment_applied() {
        counter!("quaxar_txqueue_applied_total").increment(1);
    }

    /// Increment rejected transactions by reason.
    pub fn increment_rejected(reason: &str) {
        counter!("quaxar_txqueue_rejected_total", "reason" => reason.to_owned()).increment(1);
    }
}

// ═══════════════════════════════════════════════════════════════
// NODESTORE METRICS
// ═══════════════════════════════════════════════════════════════

pub mod nodestore {
    use super::*;

    /// Increment read operations.
    pub fn increment_reads() {
        counter!("quaxar_nodestore_reads_total").increment(1);
    }

    /// Increment write operations.
    pub fn increment_writes() {
        counter!("quaxar_nodestore_writes_total").increment(1);
    }

    /// Increment cache hits.
    pub fn increment_cache_hits() {
        counter!("quaxar_nodestore_cache_hits_total").increment(1);
    }

    /// Increment cache misses.
    pub fn increment_cache_misses() {
        counter!("quaxar_nodestore_cache_misses_total").increment(1);
    }

    /// Record read duration in seconds.
    pub fn record_read_duration(seconds: f64) {
        histogram!("quaxar_nodestore_read_duration_seconds").record(seconds);
    }

    /// Record write duration in seconds.
    pub fn record_write_duration(seconds: f64) {
        histogram!("quaxar_nodestore_write_duration_seconds").record(seconds);
    }
}

// ═══════════════════════════════════════════════════════════════
// RPC METRICS
// ═══════════════════════════════════════════════════════════════

pub mod rpc {
    use super::*;

    /// Record an RPC request with its method and duration.
    pub fn record_request(method: &str, duration_seconds: f64) {
        counter!("quaxar_rpc_requests_total", "method" => method.to_owned()).increment(1);
        histogram!("quaxar_rpc_duration_seconds", "method" => method.to_owned())
            .record(duration_seconds);
    }

    /// Increment RPC errors by error code.
    pub fn increment_error(method: &str, error_code: &str) {
        counter!("quaxar_rpc_errors_total", "method" => method.to_owned(), "code" => error_code.to_owned()).increment(1);
    }
}

// ═══════════════════════════════════════════════════════════════
// ACQUISITION MIGRATION METRICS
// ═══════════════════════════════════════════════════════════════
//
// M0 baseline observability for the single-owner acquisition migration. These
// record the current inbound-ledger lifecycle boundary: registry lock waits,
// per-ledger actor mailbox occupancy, ready-scheduler dispatch delay,
// NodeReadBroker queue/in-flight delay, NodeStore write and durability-fence
// latency, peer-availability-to-first-request latency, and every operating-mode
// transition with its reason. They are additive observability only and never
// change acquisition behavior.

pub mod acquisition {
    use super::*;

    // ── Registry lock wait ────────────────────────────────────────────────
    // Time spent blocked on the registry's `inner` mutex before the guard is
    // granted. High percentiles signal ingress/lifecycle lock contention.

    /// Record how long a caller waited to acquire the registry inner mutex.
    pub fn record_registry_lock_wait(seconds: f64) {
        histogram!("quaxar_acq_registry_lock_wait_seconds").record(seconds);
    }

    // ── Actor mailbox occupancy and age ───────────────────────────────────
    // Occupancy is sampled into histograms so multiple live sessions aggregate
    // correctly (a last-writer-wins gauge would only reflect one session). The
    // age histogram samples how long admitted packets sit in the mailbox before
    // a bounded turn consumes them.

    /// Sample a session's buffered packet count.
    pub fn mailbox_packet_count(count: usize) {
        histogram!("quaxar_acq_mailbox_packet_count").record(count as f64);
    }

    /// Sample a session's buffered packet bytes.
    pub fn mailbox_packet_bytes(bytes: usize) {
        histogram!("quaxar_acq_mailbox_packet_bytes").record(bytes as f64);
    }

    /// Set the per-session buffered packet count high-water mark.
    pub fn mailbox_packet_high_water(count: usize) {
        gauge!("quaxar_acq_mailbox_packet_high_water").set(count as f64);
    }

    /// Set the per-session buffered packet byte high-water mark.
    pub fn mailbox_byte_high_water(bytes: usize) {
        gauge!("quaxar_acq_mailbox_byte_high_water").set(bytes as f64);
    }

    /// Record how long an admitted packet waited in the mailbox before a turn.
    pub fn record_mailbox_packet_age(seconds: f64) {
        histogram!("quaxar_acq_mailbox_packet_age_seconds").record(seconds);
    }

    // ── Ready-scheduler dispatch delay ────────────────────────────────────
    // Time from a scheduler wake to the actor turn actually being claimed on a
    // worker. Includes ready-queue wait plus worker dispatch.

    /// Record the delay between a scheduler wake and its turn being claimed.
    pub fn record_scheduler_delay(seconds: f64) {
        histogram!("quaxar_acq_scheduler_delay_seconds").record(seconds);
    }

    // ── NodeReadBroker queue and in-flight ────────────────────────────────
    // Queue delay is the time a read key waits between logical request and
    // physical dispatch. In-flight is the number of dispatched physical reads.

    /// Record how long a read key waited between request and physical dispatch.
    pub fn record_read_queue_delay(seconds: f64) {
        histogram!("quaxar_acq_read_queue_delay_seconds").record(seconds);
    }

    /// Set the number of currently dispatched physical NodeStore reads.
    pub fn read_in_flight(count: usize) {
        gauge!("quaxar_acq_read_in_flight").set(count as f64);
    }

    // ── NodeStore write and durability-fence latency ─────────────────────
    // Write duration covers one persistence batch; fence duration covers the
    // final durability barrier (`sync_result`).

    /// Record the duration of a NodeStore persistence write batch.
    pub fn record_nodestore_write_duration(seconds: f64) {
        histogram!("quaxar_acq_nodestore_write_duration_seconds").record(seconds);
    }

    /// Record the payload bytes written by a persistence write batch.
    pub fn record_nodestore_write_bytes(bytes: u64) {
        counter!("quaxar_acq_nodestore_write_bytes_total").increment(bytes);
    }

    /// Record the duration of a durability barrier (fence) completion.
    pub fn record_nodestore_fence_duration(seconds: f64) {
        histogram!("quaxar_acq_nodestore_fence_duration_seconds").record(seconds);
    }

    // ── Peer availability to first request ────────────────────────────────
    //
    // M0 stopgap: overlay owns peer availability, the acquisition actor owns
    // request production. Until the coordinator owns a single `peer_view`
    // snapshot (M2+), the "peers available since" timestamp lives here in the
    // observability layer: overlay records the 0→positive transition, and the
    // acquisition actor reads it back at its first outbound request.
    //
    // This is intentionally a narrow port-local lock over one `Instant`; it is
    // not a backdoor to session lifecycle state.

    static PEERS_AVAILABLE_SINCE: std::sync::Mutex<Option<std::time::Instant>> =
        std::sync::Mutex::new(None);

    /// Record that usable peer capability is present (0→positive transition).
    /// Idempotent until peers become unavailable again.
    pub fn note_peers_available() {
        let mut guard = PEERS_AVAILABLE_SINCE
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if guard.is_none() {
            *guard = Some(std::time::Instant::now());
        }
        gauge!("quaxar_acq_peers_available").set(1.0);
    }

    /// Record that no usable peer capability remains.
    pub fn note_peers_unavailable() {
        let mut guard = PEERS_AVAILABLE_SINCE
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        *guard = None;
        gauge!("quaxar_acq_peers_available").set(0.0);
    }

    /// Elapsed time since usable peers were last observed, if any.
    pub fn peers_available_elapsed() -> Option<std::time::Duration> {
        let guard = PEERS_AVAILABLE_SINCE
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        guard.map(|since| since.elapsed())
    }

    /// Record the latency from usable peer availability to the first outbound
    /// acquisition request.
    pub fn record_peer_availability_to_first_request(seconds: f64) {
        histogram!("quaxar_acq_peer_availability_to_first_request_seconds").record(seconds);
    }

    // ── Operating-mode transitions ─────────────────────────────────────────
    // Every transition is a counter labeled by `from`, `to`, and a human
    // reason supplied by the writer. The current-mode gauge mirrors the latest
    // transition target.

    /// Record an operating-mode transition together with its reason.
    pub fn record_operating_mode_transition(from: &str, to: &str, reason: &str) {
        counter!(
            "quaxar_operating_mode_transitions_total",
            "from" => from.to_owned(),
            "to" => to.to_owned(),
            "reason" => reason.to_owned(),
        )
        .increment(1);
    }

    /// Set the current operating mode code (0=disconnected … 4=full).
    pub fn operating_mode(code: u8) {
        gauge!("quaxar_operating_mode").set(f64::from(code));
    }
}

// ═══════════════════════════════════════════════════════════════
// SYSTEM METRICS
// ═══════════════════════════════════════════════════════════════

pub mod system {
    use super::*;

    /// Set uptime in seconds.
    pub fn uptime(seconds: f64) {
        gauge!("quaxar_uptime_seconds").set(seconds);
    }

    /// Set RSS memory usage in bytes.
    pub fn memory_rss(bytes: u64) {
        gauge!("quaxar_memory_rss_bytes").set(bytes as f64);
    }

    /// Set open file descriptor count.
    pub fn open_fds(count: u64) {
        gauge!("quaxar_open_file_descriptors").set(count as f64);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metrics_record_without_exporter() {
        // Metrics should work even without an exporter installed (no-op sink)
        ledger::validated_seq(100);
        ledger::record_close_duration(0.5);
        ledger::record_tx_count(42);
        ledger::increment_closed();
        consensus::proposers(5);
        consensus::mode(4);
        consensus::increment_validations_received();
        network::peers_connected(21);
        network::increment_messages_received("TMTransaction");
        network::add_bytes_received(1024);
        txqueue::size(10);
        txqueue::increment_applied();
        txqueue::increment_rejected("insufficient_fee");
        nodestore::increment_reads();
        nodestore::increment_cache_hits();
        nodestore::record_read_duration(0.001);
        rpc::record_request("server_info", 0.002);
        rpc::increment_error("account_info", "actNotFound");
        system::uptime(3600.0);
    }

    #[test]
    fn acquisition_metrics_record_without_exporter() {
        acquisition::record_registry_lock_wait(0.001);
        acquisition::mailbox_packet_count(3);
        acquisition::mailbox_packet_bytes(4096);
        acquisition::mailbox_packet_high_water(128);
        acquisition::mailbox_byte_high_water(4 * 1024 * 1024);
        acquisition::record_mailbox_packet_age(0.0005);
        acquisition::record_scheduler_delay(0.002);
        acquisition::record_read_queue_delay(0.003);
        acquisition::read_in_flight(12);
        acquisition::record_nodestore_write_duration(0.01);
        acquisition::record_nodestore_write_bytes(8192);
        acquisition::record_nodestore_fence_duration(0.02);
        acquisition::note_peers_available();
        acquisition::note_peers_unavailable();
        acquisition::record_peer_availability_to_first_request(0.5);
        acquisition::record_operating_mode_transition("connected", "syncing", "test");
        acquisition::operating_mode(2);
    }
}
