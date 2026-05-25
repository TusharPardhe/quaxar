//! Rust port of `xrpl/protocol/TxSearched.h`.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TxSearched {
    All,
    Some,
    Unknown,
}
