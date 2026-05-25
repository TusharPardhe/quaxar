//! Parallel sync configuration — operator-tunable parameters.
//!
//! Node operators can adjust these in their config file to match their hardware:
//!
//! ```toml
//! [sync]
//! # How aggressively to sync (1=conservative, 5=aggressive)
//! speed_profile = "balanced"
//!
//! # Or set individual parameters:
//! max_concurrent_requests = 32
//! max_requests_per_peer = 4
//! write_batch_size = 64
//! max_pending_writes_mb = 256
//! ```

use std::time::Duration;

/// Sync speed profiles — presets for common hardware configurations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncProfile {
    /// Minimal resource usage. Good for VPS with limited IOPS.
    /// ~20-30 min sync time.
    Conservative,
    /// Default. Balances speed with resource usage.
    /// ~8-12 min sync time.
    Balanced,
    /// Fast sync for dedicated hardware (NVMe SSD, 1Gbps+).
    /// ~3-5 min sync time.
    Fast,
    /// Maximum speed. Requires high-end hardware.
    /// ~1-2 min sync time. May stress peers.
    Aggressive,
}

/// Operator-tunable sync parameters.
#[derive(Debug, Clone)]
pub struct SyncConfig {
    /// Maximum concurrent node fetch requests across all peers.
    pub max_concurrent_requests: usize,

    /// Maximum concurrent requests to a single peer.
    pub max_requests_per_peer: usize,

    /// Number of nodes to batch-write to disk at once.
    pub write_batch_size: usize,

    /// Maximum RAM (in MB) for pending writes before applying backpressure.
    pub max_pending_writes_mb: usize,

    /// How many top-level branches to fetch in parallel (1-16).
    pub parallel_branches: usize,

    /// Timeout for a single node fetch request.
    pub request_timeout: Duration,

    /// How often to re-check for missing nodes after a batch completes.
    pub poll_interval: Duration,

    /// Maximum nodes to request in a single TMGetLedger message.
    pub nodes_per_request: usize,
}

impl SyncConfig {
    /// Create config from a named profile.
    pub fn from_profile(profile: SyncProfile) -> Self {
        match profile {
            SyncProfile::Conservative => Self {
                max_concurrent_requests: 8,
                max_requests_per_peer: 2,
                write_batch_size: 16,
                max_pending_writes_mb: 64,
                parallel_branches: 4,
                request_timeout: Duration::from_secs(15),
                poll_interval: Duration::from_millis(500),
                nodes_per_request: 8,
            },
            SyncProfile::Balanced => Self {
                max_concurrent_requests: 32,
                max_requests_per_peer: 4,
                write_batch_size: 64,
                max_pending_writes_mb: 256,
                parallel_branches: 8,
                request_timeout: Duration::from_secs(10),
                poll_interval: Duration::from_millis(200),
                nodes_per_request: 16,
            },
            SyncProfile::Fast => Self {
                max_concurrent_requests: 64,
                max_requests_per_peer: 6,
                write_batch_size: 128,
                max_pending_writes_mb: 512,
                parallel_branches: 16,
                request_timeout: Duration::from_secs(8),
                poll_interval: Duration::from_millis(100),
                nodes_per_request: 32,
            },
            SyncProfile::Aggressive => Self {
                max_concurrent_requests: 128,
                max_requests_per_peer: 8,
                write_batch_size: 256,
                max_pending_writes_mb: 1024,
                parallel_branches: 16,
                request_timeout: Duration::from_secs(5),
                poll_interval: Duration::from_millis(50),
                nodes_per_request: 64,
            },
        }
    }

    /// Parse profile from config string.
    pub fn parse_profile(name: &str) -> Option<SyncProfile> {
        match name.to_lowercase().as_str() {
            "conservative" | "slow" => Some(SyncProfile::Conservative),
            "balanced" | "default" | "normal" => Some(SyncProfile::Balanced),
            "fast" | "quick" => Some(SyncProfile::Fast),
            "aggressive" | "max" => Some(SyncProfile::Aggressive),
            _ => None,
        }
    }
}

impl Default for SyncConfig {
    fn default() -> Self {
        Self::from_profile(SyncProfile::Balanced)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profiles_have_increasing_concurrency() {
        let c = SyncConfig::from_profile(SyncProfile::Conservative);
        let b = SyncConfig::from_profile(SyncProfile::Balanced);
        let f = SyncConfig::from_profile(SyncProfile::Fast);
        let a = SyncConfig::from_profile(SyncProfile::Aggressive);

        assert!(c.max_concurrent_requests < b.max_concurrent_requests);
        assert!(b.max_concurrent_requests < f.max_concurrent_requests);
        assert!(f.max_concurrent_requests < a.max_concurrent_requests);
    }

    #[test]
    fn profiles_have_increasing_per_peer_limits() {
        let c = SyncConfig::from_profile(SyncProfile::Conservative);
        let b = SyncConfig::from_profile(SyncProfile::Balanced);
        let f = SyncConfig::from_profile(SyncProfile::Fast);
        let a = SyncConfig::from_profile(SyncProfile::Aggressive);

        assert!(c.max_requests_per_peer <= b.max_requests_per_peer);
        assert!(b.max_requests_per_peer <= f.max_requests_per_peer);
        assert!(f.max_requests_per_peer <= a.max_requests_per_peer);
    }

    #[test]
    fn profiles_have_increasing_write_batch_size() {
        let c = SyncConfig::from_profile(SyncProfile::Conservative);
        let b = SyncConfig::from_profile(SyncProfile::Balanced);
        let f = SyncConfig::from_profile(SyncProfile::Fast);
        let a = SyncConfig::from_profile(SyncProfile::Aggressive);

        assert!(c.write_batch_size < b.write_batch_size);
        assert!(b.write_batch_size < f.write_batch_size);
        assert!(f.write_batch_size < a.write_batch_size);
    }

    #[test]
    fn profiles_have_decreasing_timeouts() {
        let c = SyncConfig::from_profile(SyncProfile::Conservative);
        let b = SyncConfig::from_profile(SyncProfile::Balanced);
        let f = SyncConfig::from_profile(SyncProfile::Fast);
        let a = SyncConfig::from_profile(SyncProfile::Aggressive);

        assert!(c.request_timeout >= b.request_timeout);
        assert!(b.request_timeout >= f.request_timeout);
        assert!(f.request_timeout >= a.request_timeout);
    }

    #[test]
    fn parallel_branches_never_exceeds_16() {
        for profile in [
            SyncProfile::Conservative,
            SyncProfile::Balanced,
            SyncProfile::Fast,
            SyncProfile::Aggressive,
        ] {
            let config = SyncConfig::from_profile(profile);
            assert!(
                config.parallel_branches <= 16,
                "{:?} has parallel_branches={} (max is 16 for SHAMap)",
                profile,
                config.parallel_branches
            );
            assert!(config.parallel_branches >= 1);
        }
    }

    #[test]
    fn max_requests_per_peer_is_reasonable() {
        for profile in [
            SyncProfile::Conservative,
            SyncProfile::Balanced,
            SyncProfile::Fast,
            SyncProfile::Aggressive,
        ] {
            let config = SyncConfig::from_profile(profile);
            // Should never hammer a single peer with more than 10 requests
            assert!(
                config.max_requests_per_peer <= 10,
                "{:?} has {} requests per peer (too aggressive for peers)",
                profile,
                config.max_requests_per_peer
            );
        }
    }

    #[test]
    fn backpressure_ram_cap_is_bounded() {
        for profile in [
            SyncProfile::Conservative,
            SyncProfile::Balanced,
            SyncProfile::Fast,
            SyncProfile::Aggressive,
        ] {
            let config = SyncConfig::from_profile(profile);
            // Even aggressive should not use more than 2GB for pending writes
            assert!(
                config.max_pending_writes_mb <= 2048,
                "{:?} allows {}MB pending writes (OOM risk)",
                profile,
                config.max_pending_writes_mb
            );
        }
    }

    #[test]
    fn parse_profile_names() {
        assert_eq!(SyncConfig::parse_profile("fast"), Some(SyncProfile::Fast));
        assert_eq!(SyncConfig::parse_profile("quick"), Some(SyncProfile::Fast));
        assert_eq!(
            SyncConfig::parse_profile("default"),
            Some(SyncProfile::Balanced)
        );
        assert_eq!(
            SyncConfig::parse_profile("normal"),
            Some(SyncProfile::Balanced)
        );
        assert_eq!(
            SyncConfig::parse_profile("slow"),
            Some(SyncProfile::Conservative)
        );
        assert_eq!(
            SyncConfig::parse_profile("max"),
            Some(SyncProfile::Aggressive)
        );
        assert_eq!(
            SyncConfig::parse_profile("AGGRESSIVE"),
            Some(SyncProfile::Aggressive)
        );
        assert_eq!(SyncConfig::parse_profile("unknown"), None);
        assert_eq!(SyncConfig::parse_profile(""), None);
    }

    #[test]
    fn default_is_balanced() {
        let config = SyncConfig::default();
        let balanced = SyncConfig::from_profile(SyncProfile::Balanced);
        assert_eq!(
            config.max_concurrent_requests,
            balanced.max_concurrent_requests
        );
        assert_eq!(config.parallel_branches, balanced.parallel_branches);
        assert_eq!(config.write_batch_size, balanced.write_batch_size);
    }

    #[test]
    fn concurrent_requests_always_exceeds_per_peer_limit() {
        // Total concurrency must be higher than per-peer limit
        // (otherwise you can only use 1 peer)
        for profile in [
            SyncProfile::Conservative,
            SyncProfile::Balanced,
            SyncProfile::Fast,
            SyncProfile::Aggressive,
        ] {
            let config = SyncConfig::from_profile(profile);
            assert!(
                config.max_concurrent_requests > config.max_requests_per_peer,
                "{:?}: total {} must exceed per-peer {}",
                profile,
                config.max_concurrent_requests,
                config.max_requests_per_peer
            );
        }
    }

    #[test]
    fn nodes_per_request_is_power_of_two_or_reasonable() {
        for profile in [
            SyncProfile::Conservative,
            SyncProfile::Balanced,
            SyncProfile::Fast,
            SyncProfile::Aggressive,
        ] {
            let config = SyncConfig::from_profile(profile);
            assert!(config.nodes_per_request >= 1);
            assert!(config.nodes_per_request <= 128);
        }
    }

    #[test]
    fn poll_interval_is_reasonable() {
        for profile in [
            SyncProfile::Conservative,
            SyncProfile::Balanced,
            SyncProfile::Fast,
            SyncProfile::Aggressive,
        ] {
            let config = SyncConfig::from_profile(profile);
            // Should poll at least every 2 seconds, at most every 10ms
            assert!(config.poll_interval >= Duration::from_millis(10));
            assert!(config.poll_interval <= Duration::from_secs(2));
        }
    }
}
