//! The RCL (Ripple Consensus Ledger) adaptation layer: generic validation
//! tracking ([`validations`]) that bridges the algorithm-layer
//! [`crate::model::LedgerTrie`] to a concrete application.

pub mod validations;

pub use validations::{ValStatus, ValidationParms, ValidationT, Validations, ValidationsAdaptor, ValidationsLedger, is_current};
