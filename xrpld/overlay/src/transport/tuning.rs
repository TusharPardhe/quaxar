//! Overlay tuning constants mirrored from `detail/Tuning.h`.

use std::time::Duration;

/// `PeerImp.cpp` schedules each active peer's timer at this cadence.
pub const PEER_TIMER_INTERVAL: Duration = Duration::from_secs(60);
/// Outbound peers may not remain in the Not Useful diverged state indefinitely.
pub const MAX_DIVERGED_TIME: Duration = Duration::from_secs(300);
/// Outbound peers that never establish useful tracking are eventually dropped.
pub const MAX_UNKNOWN_TIME: Duration = Duration::from_secs(600);
/// A write must not permanently pin a session task. This is one peer timer interval.
pub const WRITE_DEADLINE: Duration = PEER_TIMER_INTERVAL;
/// Reads permit a full ping request/response interval before considering the session idle.
pub const READ_ACTIVITY_DEADLINE: Duration = Duration::from_secs(120);

pub const CONVERGED_LEDGER_LIMIT: usize = 24;
pub const DIVERGED_LEDGER_LIMIT: usize = 128;
pub const SOFT_MAX_REPLY_NODES: usize = 8192;
pub const HARD_MAX_REPLY_NODES: usize = 12288;
pub const SENDQ_INTERVALS: usize = 4;
pub const DROP_SEND_QUEUE: usize = 192;
pub const TARGET_SEND_QUEUE: usize = 128;
pub const SEND_QUEUE_LOG_FREQ: usize = 64;
pub const CHECK_IDLE_PEERS: usize = 4;
pub const MAX_QUERY_DEPTH: usize = 3;
pub const READ_BUFFER_BYTES: usize = 16_384;

// Differential Pricing Constants (protects against node-store seek attacks)
pub const FREE_OBJECTS_PER_REQUEST: usize = 16;
pub const COST_PER_LOOKUP_MISS: usize = 8;
