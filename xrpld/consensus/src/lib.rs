//! `consensus`: the generic, adaptor-parameterized XRP Ledger consensus
//! algorithm, ported from rippled's `xrpld/consensus` C++ sources.
//!
//! This crate contains the pure algorithm and its supporting data
//! structures, plus (in [`rcl`]) a thin set of concrete Ripple Consensus
//! Ledger (RCL) types -- `RclCxTx`, `RclCxLedger`, `RclValidations`, and
//! `ValidationStatus` -- that the `app` crate wires against real
//! `Ledger`/`SHAMap`/`STValidation` types via its own adaptor
//! implementations (`AppRclConsensusAdaptor`, `RclValidationsAdaptor`,
//! etc. in `xrpld/app/src/consensus`).

pub mod algorithm;
pub mod model;
pub mod rcl;
pub mod rcl_support;

pub use algorithm::{Consensus, ConsensusAdaptor, ConsensusClock, ConsensusLedger, ConsensusParms, ConsensusTx, ConsensusTxSet, PeerPosition, SystemConsensusClock};
pub use model::ConsensusProposal;
pub use rcl::{RclCxLedger, RclCxTx, RclCxTxRef, RclTxSet, RclValidations, RclValidationsAdapter, ValidationStatus};
