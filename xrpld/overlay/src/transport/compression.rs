//! Wire-compression helpers matching the current overlay framing rules.

pub const HEADER_BYTES: usize = 6;
pub const HEADER_BYTES_COMPRESSED: usize = 10;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum CompressionAlgorithm {
    None = 0x00,
    Lz4 = 0x90,
}

impl CompressionAlgorithm {
    pub fn from_header_bits(bits: u8) -> Option<Self> {
        match bits {
            0x00 => Some(Self::None),
            0x90 => Some(Self::Lz4),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Compressed {
    On,
    Off,
}

pub fn compress(payload: &[u8], algorithm: CompressionAlgorithm) -> Option<Vec<u8>> {
    match algorithm {
        CompressionAlgorithm::None => Some(payload.to_vec()),
        CompressionAlgorithm::Lz4 => Some(lz4_flex::block::compress(payload)),
    }
}

pub fn decompress(
    payload: &[u8],
    decompressed_size: usize,
    algorithm: CompressionAlgorithm,
) -> Option<Vec<u8>> {
    match algorithm {
        CompressionAlgorithm::None => Some(payload.to_vec()),
        CompressionAlgorithm::Lz4 => lz4_flex::block::decompress(payload, decompressed_size).ok(),
    }
}
