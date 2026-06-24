//! Columnar SLE storage: splits hot (frequently-accessed) fields from cold blob data.

/// Hot fields that fit in a single cache line for fast access.
pub struct HotColumn {
    pub balance: i64,
    pub sequence: u32,
    pub owner_count: u32,
}

/// Remaining SLE data stored as a raw blob.
pub struct ColdColumn {
    pub raw_blob: Vec<u8>,
}

/// Split a raw SLE blob into hot and cold columns.
///
/// Layout assumption: first 16 bytes are `[balance(8) | sequence(4) | owner_count(4)]`.
pub fn split_sle(raw: &[u8]) -> (HotColumn, ColdColumn) {
    let balance = if raw.len() >= 8 {
        i64::from_be_bytes(raw[..8].try_into().unwrap())
    } else {
        0
    };
    let sequence = if raw.len() >= 12 {
        u32::from_be_bytes(raw[8..12].try_into().unwrap())
    } else {
        0
    };
    let owner_count = if raw.len() >= 16 {
        u32::from_be_bytes(raw[12..16].try_into().unwrap())
    } else {
        0
    };
    let cold_start = 16.min(raw.len());
    (
        HotColumn {
            balance,
            sequence,
            owner_count,
        },
        ColdColumn {
            raw_blob: raw[cold_start..].to_vec(),
        },
    )
}

/// Merge hot and cold columns back into a single raw blob.
pub fn merge_columns(hot: &HotColumn, cold: &ColdColumn) -> Vec<u8> {
    let mut out = Vec::with_capacity(16 + cold.raw_blob.len());
    out.extend_from_slice(&hot.balance.to_be_bytes());
    out.extend_from_slice(&hot.sequence.to_be_bytes());
    out.extend_from_slice(&hot.owner_count.to_be_bytes());
    out.extend_from_slice(&cold.raw_blob);
    out
}
