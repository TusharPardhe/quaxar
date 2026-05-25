//! Full reference flow engine parity.

pub mod steps;
pub mod strand_builder;
pub mod strand_flow;

use protocol::{AccountID, Asset, Issue, STAmount, Ter};

/// A Strand is an ordered sequence of StepKind from source to destination.
pub type Strand = Vec<StepKind>;

/// Step types that can appear in a strand.
#[derive(Debug, Clone)]
pub enum StepKind {
    Direct {
        src: AccountID,
        dst: AccountID,
        currency: protocol::Currency,
    },
    XrpEndpoint {
        account: AccountID,
        is_last: bool,
    },
    Book {
        book_in: Issue,
        book_out: Issue,
    },
}

/// Context for strand building.
#[derive(Debug, Clone)]
pub struct StrandContext {
    pub src: AccountID,
    pub dst: AccountID,
    pub deliver: Asset,
    pub is_default_path: bool,
    pub owner_pays_transfer_fee: bool,
    pub offer_crossing: bool,
}

/// Result of flow execution.
#[derive(Debug, Clone)]
pub struct FlowResult {
    pub ter: Ter,
    pub actual_in: STAmount,
    pub actual_out: STAmount,
}
