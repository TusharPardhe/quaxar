//! The core consensus algorithm: parameters, data model, and the
//! `Consensus<Adaptor>` state machine.

pub mod params;

pub use params::{AvalancheCutoff, AvalancheState, ConsensusParms, get_needed_weight};
