#![allow(ambiguous_glob_reexports)]
//! `xrpld/tx` crate surface.
//!
//! This crate starts with the ambient transaction-runtime setup that current

#![allow(
    clippy::clone_on_copy,
    clippy::collapsible_if,
    clippy::duplicate_mod,
    clippy::field_reassign_with_default,
    clippy::large_enum_variant,
    clippy::manual_contains,
    clippy::redundant_closure,
    clippy::too_many_arguments,
    clippy::type_complexity,
    clippy::unnecessary_lazy_evaluations
)]

pub mod account;
pub mod amm;
pub mod apply;
pub mod check;
pub mod context;
pub mod credential;
pub mod dex;
pub mod did;
pub mod escrow;
pub mod fees;
pub mod lending;
pub mod loan;
pub mod paths;
pub mod preflight;
pub mod transactor;
pub mod utility;

// Re-export all public types and functions for backward compatibility
pub use account::*;
pub use amm::*;
pub use apply::*;
pub use check::*;
pub use context::*;
pub use credential::*;
pub use dex::*;
pub use did::*;
pub use escrow::*;
pub use fees::*;
pub use lending::*;
pub use loan::*;
pub use paths::*;
pub use preflight::*;
pub use transactor::*;
pub use utility::*;

// Specific aliases for backward compatibility
pub use escrow::escrow_create::{
    EscrowCreatePreflightFacts, MAX_MPTOKEN_AMOUNT as ESCROW_CREATE_MAX_MPTOKEN_AMOUNT,
};
pub use utility::signer_list_set::{
    MAX_MULTI_SIGNERS as SIGNER_LIST_SET_MAX_MULTI_SIGNERS,
    MIN_MULTI_SIGNERS as SIGNER_LIST_SET_MIN_MULTI_SIGNERS,
    run_signer_list_set_validate_quorum_and_signer_entries,
};
pub use utility::transactor_defaults::FULLY_CANONICAL_SIGNATURE_FLAG as TRANSACTOR_FULLY_CANONICAL_SIGNATURE_FLAG;
pub use utility::vault_preflight_dispatch::FULLY_CANONICAL_SIGNATURE_FLAG as VAULT_FULLY_CANONICAL_SIGNATURE_FLAG;
