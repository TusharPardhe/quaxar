//! `LedgerShortcut` enum from `xrpl/protocol/LedgerShortcut.h`.

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LedgerShortcut {
    Current,
    Closed,
    Validated,
}
