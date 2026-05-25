//! Prometheus metrics for the XRPL Rust node.
//!
//! Exposes rich operational metrics via a `/metrics` HTTP endpoint.
//! Uses the `metrics` crate facade for zero-cost recording when disabled.
//!
//! # Usage
//!
//! ```rust
//! use xrpld_metrics::{init_prometheus, ledger, network, rpc};
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
        gauge!("xrpld_ledger_validated_seq").set(seq as f64);
    }

    /// Set the current open ledger sequence.
    pub fn current_seq(seq: u64) {
        gauge!("xrpld_ledger_current_seq").set(seq as f64);
    }

    /// Record ledger close duration in seconds.
    pub fn record_close_duration(seconds: f64) {
        histogram!("xrpld_ledger_close_duration_seconds").record(seconds);
    }

    /// Record number of transactions in a closed ledger.
    pub fn record_tx_count(count: u64) {
        histogram!("xrpld_ledger_tx_count").record(count as f64);
    }

    /// Set the number of state entries in the current ledger.
    pub fn state_entries(count: u64) {
        gauge!("xrpld_ledger_state_entries").set(count as f64);
    }

    /// Increment total ledgers closed.
    pub fn increment_closed() {
        counter!("xrpld_ledger_closed_total").increment(1);
    }
}

// ═══════════════════════════════════════════════════════════════
// CONSENSUS METRICS
// ═══════════════════════════════════════════════════════════════

pub mod consensus {
    use super::*;

    /// Record consensus round duration in seconds.
    pub fn record_round_duration(seconds: f64) {
        histogram!("xrpld_consensus_round_duration_seconds").record(seconds);
    }

    /// Set the number of proposers in the current round.
    pub fn proposers(count: u64) {
        gauge!("xrpld_consensus_proposers").set(count as f64);
    }

    /// Set the current operating mode (0=disconnected, 1=connected, 2=syncing, 3=tracking, 4=full).
    pub fn mode(mode: u8) {
        gauge!("xrpld_consensus_mode").set(f64::from(mode));
    }

    /// Increment total validations received.
    pub fn increment_validations_received() {
        counter!("xrpld_validations_received_total").increment(1);
    }

    /// Increment trusted validations.
    pub fn increment_validations_trusted() {
        counter!("xrpld_validations_trusted_total").increment(1);
    }

    /// Increment stale validations (too old).
    pub fn increment_validations_stale() {
        counter!("xrpld_validations_stale_total").increment(1);
    }
}

// ═══════════════════════════════════════════════════════════════
// NETWORK METRICS
// ═══════════════════════════════════════════════════════════════

pub mod network {
    use super::*;

    /// Set total connected peers.
    pub fn peers_connected(count: u64) {
        gauge!("xrpld_peers_connected").set(count as f64);
    }

    /// Set inbound peer count.
    pub fn peers_inbound(count: u64) {
        gauge!("xrpld_peers_inbound").set(count as f64);
    }

    /// Set outbound peer count.
    pub fn peers_outbound(count: u64) {
        gauge!("xrpld_peers_outbound").set(count as f64);
    }

    /// Increment messages received by type.
    pub fn increment_messages_received(msg_type: &str) {
        counter!("xrpld_messages_received_total", "type" => msg_type.to_owned()).increment(1);
    }

    /// Increment messages sent by type.
    pub fn increment_messages_sent(msg_type: &str) {
        counter!("xrpld_messages_sent_total", "type" => msg_type.to_owned()).increment(1);
    }

    /// Add bytes received.
    pub fn add_bytes_received(bytes: u64) {
        counter!("xrpld_bandwidth_rx_bytes_total").increment(bytes);
    }

    /// Add bytes sent.
    pub fn add_bytes_sent(bytes: u64) {
        counter!("xrpld_bandwidth_tx_bytes_total").increment(bytes);
    }
}

// ═══════════════════════════════════════════════════════════════
// TX QUEUE METRICS
// ═══════════════════════════════════════════════════════════════

pub mod txqueue {
    use super::*;

    /// Set current queue size.
    pub fn size(count: u64) {
        gauge!("xrpld_txqueue_size").set(count as f64);
    }

    /// Set current escalated fee level.
    pub fn fee_level(level: u64) {
        gauge!("xrpld_txqueue_fee_level").set(level as f64);
    }

    /// Increment applied transactions.
    pub fn increment_applied() {
        counter!("xrpld_txqueue_applied_total").increment(1);
    }

    /// Increment rejected transactions by reason.
    pub fn increment_rejected(reason: &str) {
        counter!("xrpld_txqueue_rejected_total", "reason" => reason.to_owned()).increment(1);
    }
}

// ═══════════════════════════════════════════════════════════════
// NODESTORE METRICS
// ═══════════════════════════════════════════════════════════════

pub mod nodestore {
    use super::*;

    /// Increment read operations.
    pub fn increment_reads() {
        counter!("xrpld_nodestore_reads_total").increment(1);
    }

    /// Increment write operations.
    pub fn increment_writes() {
        counter!("xrpld_nodestore_writes_total").increment(1);
    }

    /// Increment cache hits.
    pub fn increment_cache_hits() {
        counter!("xrpld_nodestore_cache_hits_total").increment(1);
    }

    /// Increment cache misses.
    pub fn increment_cache_misses() {
        counter!("xrpld_nodestore_cache_misses_total").increment(1);
    }

    /// Record read duration in seconds.
    pub fn record_read_duration(seconds: f64) {
        histogram!("xrpld_nodestore_read_duration_seconds").record(seconds);
    }

    /// Record write duration in seconds.
    pub fn record_write_duration(seconds: f64) {
        histogram!("xrpld_nodestore_write_duration_seconds").record(seconds);
    }
}

// ═══════════════════════════════════════════════════════════════
// RPC METRICS
// ═══════════════════════════════════════════════════════════════

pub mod rpc {
    use super::*;

    /// Record an RPC request with its method and duration.
    pub fn record_request(method: &str, duration_seconds: f64) {
        counter!("xrpld_rpc_requests_total", "method" => method.to_owned()).increment(1);
        histogram!("xrpld_rpc_duration_seconds", "method" => method.to_owned())
            .record(duration_seconds);
    }

    /// Increment RPC errors by error code.
    pub fn increment_error(method: &str, error_code: &str) {
        counter!("xrpld_rpc_errors_total", "method" => method.to_owned(), "code" => error_code.to_owned()).increment(1);
    }
}

// ═══════════════════════════════════════════════════════════════
// SYSTEM METRICS
// ═══════════════════════════════════════════════════════════════

pub mod system {
    use super::*;

    /// Set uptime in seconds.
    pub fn uptime(seconds: f64) {
        gauge!("xrpld_uptime_seconds").set(seconds);
    }

    /// Set RSS memory usage in bytes.
    pub fn memory_rss(bytes: u64) {
        gauge!("xrpld_memory_rss_bytes").set(bytes as f64);
    }

    /// Set open file descriptor count.
    pub fn open_fds(count: u64) {
        gauge!("xrpld_open_file_descriptors").set(count as f64);
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
}
