//! The core consensus algorithm: parameters, data model, and the
//! `Consensus<Adaptor>` state machine.

pub mod consensus;
pub mod functions;
pub mod params;
pub mod timing;
pub mod types;

pub use consensus::{
    Consensus, ConsensusAdaptor, ConsensusClock, ConsensusLedger, ConsensusTx, ConsensusTxSet,
    PeerPosition, SystemConsensusClock,
};
pub use functions::{check_consensus, should_close_ledger};
pub use params::{AvalancheCutoff, AvalancheState, ConsensusParms, get_needed_weight};
pub use types::{
    ConsensusCloseTimes, ConsensusMode, ConsensusPhase, ConsensusResult, ConsensusState,
    ConsensusTimer, participants_needed,
};
