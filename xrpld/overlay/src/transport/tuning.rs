//! Overlay tuning constants mirrored from `detail/Tuning.h`.

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
