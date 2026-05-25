#![allow(clippy::type_complexity)]

mod algorithm;
mod model;
mod rcl_support;
pub mod sim;

pub use algorithm::consensus;
pub use algorithm::params;
pub use algorithm::timing;
pub use algorithm::types;
pub use model::disputed_tx;
pub use model::ledger_trie;
pub use model::proposal;
pub use rcl_support::rcl;
pub use rcl_support::rcl_hash;
pub use rcl_support::validations;

pub use consensus::{
    Consensus, ConsensusAdaptor, ConsensusDecision, ConsensusEvent, ConsensusPeerPosition,
    check_consensus, should_close_ledger,
};
pub use disputed_tx::DisputedTx;
pub use ledger_trie::{LedgerHistory, LedgerTrie, SpanTip, mismatch};
pub use params::{AvalancheCutoff, AvalancheState, ConsensusParms, get_needed_weight};
pub use proposal::{ConsensusHashable, ConsensusProposal};
pub use rcl::{
    RclConsensus, RclConsensusAdapter, RclConsensusState, RclCxLedger, RclCxPeerPos, RclCxTx,
    RclRoundTimer,
};
pub use rcl_hash::{proposal_unique_id, rcl_txset_id};
pub use timing::{
    DECREASE_LEDGER_TIME_RESOLUTION_EVERY, INCREASE_LEDGER_TIME_RESOLUTION_EVERY,
    LEDGER_DEFAULT_TIME_RESOLUTION, LEDGER_GENESIS_TIME_RESOLUTION,
    LEDGER_POSSIBLE_TIME_RESOLUTIONS, effective_close_time, get_next_ledger_time_resolution,
    round_close_time,
};
pub use types::{
    ConsensusCloseTimes, ConsensusMode, ConsensusPhase, ConsensusResult, ConsensusState,
    ConsensusTimer,
};
pub use validations::{
    RclValidatedLedger, RclValidation, RclValidations, RclValidationsAdapter, ValidationStatus,
};
