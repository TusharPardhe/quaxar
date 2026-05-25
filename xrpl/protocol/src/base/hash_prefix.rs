//! Hash prefixes from `xrpl/protocol/HashPrefix.h`.

/// Build a protocol hash prefix from three ASCII bytes.
pub const fn make_hash_prefix(a: char, b: char, c: char) -> u32 {
    ((a as u32) << 24) + ((b as u32) << 16) + ((c as u32) << 8)
}

/// Prefixes inserted before hashed protocol payloads.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u32)]
pub enum HashPrefix {
    TransactionId = make_hash_prefix('T', 'X', 'N'),
    TxNode = make_hash_prefix('S', 'N', 'D'),
    LeafNode = make_hash_prefix('M', 'L', 'N'),
    InnerNode = make_hash_prefix('M', 'I', 'N'),
    LedgerMaster = make_hash_prefix('L', 'W', 'R'),
    TxSign = make_hash_prefix('S', 'T', 'X'),
    TxMultiSign = make_hash_prefix('S', 'M', 'T'),
    Validation = make_hash_prefix('V', 'A', 'L'),
    Proposal = make_hash_prefix('P', 'R', 'P'),
    Manifest = make_hash_prefix('M', 'A', 'N'),
    PaymentChannelClaim = make_hash_prefix('C', 'L', 'M'),
    Batch = make_hash_prefix('B', 'C', 'H'),
}

impl HashPrefix {
    pub const fn as_u32(self) -> u32 {
        self as u32
    }
}
