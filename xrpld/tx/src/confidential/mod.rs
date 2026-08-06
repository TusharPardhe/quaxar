//! Quaxar Confidential-MPT typed-preclaim extension contract.
//!
//! No corresponding Confidential-MPT transactors exist in the locally audited
//! `../rippled` checkout. These helpers are therefore a Quaxar extension, not
//! a claim of rippled parity. Each `*PreclaimFacts` value is the complete,
//! immutable ledger fact set for its helper; the ordered guards in that helper
//! are normative and return the first matching `TER`.
//!
//! The protocol currently has transaction type identifiers but no `STTx`
//! formats for this family. Consequently these fact contracts are deliberately
//! not registered with the application `ReadView` dispatcher: that dispatcher
//! must remain fail-closed until a separately reviewed field-format and
//! fact-extraction adapter exists. These helpers perform no apply or dry-run
//! work and never turn an unverified route into `tesSUCCESS`.

pub mod confidential_mpt_clawback;
pub mod confidential_mpt_convert;
pub mod confidential_mpt_convert_back;
pub mod confidential_mpt_merge_inbox;
pub mod confidential_mpt_send;

pub use confidential_mpt_clawback::*;
pub use confidential_mpt_convert::*;
pub use confidential_mpt_convert_back::*;
pub use confidential_mpt_merge_inbox::*;
pub use confidential_mpt_send::*;
