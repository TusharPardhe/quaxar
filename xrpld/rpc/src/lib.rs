#![allow(ambiguous_glob_reexports, unused_imports)]
//! First `xrpld/rpc` caller seams above the landed `xrpl/tx/apply.h`
//! validity surface.
//!
//! This crate ports the deterministic `checkSigs()` / `forceValidity(...)` /
//! `checkValidity(...)` control flow from the the reference implementation RPC entry points.

#![allow(
    clippy::clone_on_copy,
    clippy::collapsible_if,
    clippy::large_enum_variant,
    clippy::manual_contains,
    clippy::needless_lifetimes,
    clippy::question_mark,
    clippy::too_many_arguments,
    clippy::unnecessary_cast,
    clippy::unnecessary_lazy_evaluations,
    clippy::unnecessary_map_or,
    clippy::unwrap_or_default,
    clippy::vec_init_then_push
)]

extern crate self as rpc;

// Module declarations
pub mod amm;
pub mod commands;
pub mod gateway;
pub mod handlers;
pub mod ledger_path;
pub mod nft;
pub mod pathfinding;
pub mod response;
pub mod signing;
pub mod state;
pub mod subscriptions;
pub mod validation;

pub mod status {
    pub use xrpld_core::{RpcErrorCode, RpcStatus, Status};
}
pub use status::{RpcErrorCode, RpcStatus, Status};

// Re-export all public items from modules for backward compatibility
pub use amm::*;
pub use commands::*;

pub use handlers::*;

pub use nft::*;
pub use pathfinding::*;

pub use signing::*;
pub use state::*;
pub use subscriptions::*;
pub use validation::*;

// Re-export from external modules
pub use app::paths::{
    PathFindSession, PathFindTuning, PathFinderRequest, PathFinderSource, PathRequest,
    PathRequestManager, make_path_find_status, parse_path_finder_request,
};
pub use protocol::TxSearched;
