//! Pure data structures used by the consensus algorithm: proposals,
//! disputed transactions, and the ledger trie used for validation-based
//! preference tracking.

pub mod disputed_tx;
pub mod ledger_trie;
pub mod proposal;

pub use disputed_tx::DisputedTx;
pub use ledger_trie::{LedgerTrie, SpanTip, TrieLedger};
pub use proposal::ConsensusProposal;
