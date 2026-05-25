//! `xrpl/protocol/LedgerHeader.*` compatibility surface.

use basics::sha_map_hash::SHAMapHash;

use crate::{HashPrefix, SerialIter, Serializer, Sha512HalfHasher};

pub const SLCF_NO_CONSENSUS_TIME: u8 = 0x01;
pub const LEDGER_HEADER_WIRE_SIZE: usize = 118;
pub const PREFIXED_LEDGER_HEADER_WIRE_SIZE: usize = 122;
pub const LEDGER_HEADER_WITH_HASH_WIRE_SIZE: usize = 150;
pub const PREFIXED_LEDGER_HEADER_WITH_HASH_WIRE_SIZE: usize = 154;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct LedgerHeader {
    pub seq: u32,
    pub drops: u64,
    pub hash: SHAMapHash,
    pub parent_hash: SHAMapHash,
    pub tx_hash: SHAMapHash,
    pub account_hash: SHAMapHash,
    pub parent_close_time: u32,
    pub close_time: u32,
    pub validated: bool,
    pub accepted: bool,
    pub close_time_resolution: u8,
    pub close_flags: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LedgerHeaderCodecError {
    TooShort,
}

pub fn get_close_agree(header: &LedgerHeader) -> bool {
    (header.close_flags & SLCF_NO_CONSENSUS_TIME) == 0
}

pub fn calculate_ledger_hash(header: &LedgerHeader) -> SHAMapHash {
    let mut hasher = Sha512HalfHasher::new();
    hasher.write(HashPrefix::LedgerMaster.as_u32().to_be_bytes());
    hasher.write(header.seq.to_be_bytes());
    hasher.write(header.drops.to_be_bytes());
    hasher.write(header.parent_hash.as_uint256().data());
    hasher.write(header.tx_hash.as_uint256().data());
    hasher.write(header.account_hash.as_uint256().data());
    hasher.write(header.parent_close_time.to_be_bytes());
    hasher.write(header.close_time.to_be_bytes());
    hasher.write([header.close_time_resolution]);
    hasher.write([header.close_flags]);
    SHAMapHash::new(hasher.result())
}

pub fn add_raw_ledger_header(
    header: &LedgerHeader,
    serializer: &mut Serializer,
    include_hash: bool,
) {
    serializer.add32(header.seq);
    serializer.add64(header.drops);
    serializer.add_bit_string(*header.parent_hash.as_uint256());
    serializer.add_bit_string(*header.tx_hash.as_uint256());
    serializer.add_bit_string(*header.account_hash.as_uint256());
    serializer.add32(header.parent_close_time);
    serializer.add32(header.close_time);
    serializer.add8(header.close_time_resolution);
    serializer.add8(header.close_flags);

    if include_hash {
        serializer.add_bit_string(*header.hash.as_uint256());
    }
}

pub fn serialize_ledger_header(header: &LedgerHeader, include_hash: bool) -> Vec<u8> {
    let mut serializer = Serializer::new(if include_hash {
        LEDGER_HEADER_WITH_HASH_WIRE_SIZE
    } else {
        LEDGER_HEADER_WIRE_SIZE
    });
    add_raw_ledger_header(header, &mut serializer, include_hash);
    serializer.data().to_vec()
}

pub fn serialize_prefixed_ledger_header(header: &LedgerHeader, include_hash: bool) -> Vec<u8> {
    let mut serializer = Serializer::new(if include_hash {
        PREFIXED_LEDGER_HEADER_WITH_HASH_WIRE_SIZE
    } else {
        PREFIXED_LEDGER_HEADER_WIRE_SIZE
    });
    serializer.add32_prefix(HashPrefix::LedgerMaster);
    add_raw_ledger_header(header, &mut serializer, include_hash);
    serializer.data().to_vec()
}

pub fn deserialize_ledger_header(
    data: &[u8],
    has_hash: bool,
) -> Result<LedgerHeader, LedgerHeaderCodecError> {
    let required = if has_hash {
        LEDGER_HEADER_WITH_HASH_WIRE_SIZE
    } else {
        LEDGER_HEADER_WIRE_SIZE
    };
    if data.len() < required {
        return Err(LedgerHeaderCodecError::TooShort);
    }

    let mut iter = SerialIter::new(&data[..required]);
    let mut header = LedgerHeader {
        seq: iter.get32(),
        drops: iter.get64(),
        parent_hash: SHAMapHash::new(iter.get256()),
        tx_hash: SHAMapHash::new(iter.get256()),
        account_hash: SHAMapHash::new(iter.get256()),
        parent_close_time: iter.get32(),
        close_time: iter.get32(),
        close_time_resolution: iter.get8(),
        close_flags: iter.get8(),
        ..LedgerHeader::default()
    };

    if has_hash {
        header.hash = SHAMapHash::new(iter.get256());
    }

    Ok(header)
}

pub fn deserialize_prefixed_ledger_header(
    data: &[u8],
    has_hash: bool,
) -> Result<LedgerHeader, LedgerHeaderCodecError> {
    if data.len() < 4 {
        return Err(LedgerHeaderCodecError::TooShort);
    }
    deserialize_ledger_header(&data[4..], has_hash)
}

#[cfg(test)]
mod tests {
    use basics::base_uint::Uint256;

    use super::{
        LEDGER_HEADER_WIRE_SIZE, LedgerHeader, calculate_ledger_hash, deserialize_ledger_header,
        deserialize_prefixed_ledger_header, get_close_agree, serialize_ledger_header,
        serialize_prefixed_ledger_header,
    };

    fn sample_hash(fill: u8) -> basics::sha_map_hash::SHAMapHash {
        basics::sha_map_hash::SHAMapHash::new(Uint256::from_array([fill; 32]))
    }

    #[test]
    fn ledger_header_hash_and_wire_codec_match_current_cpp_layout() {
        let header = LedgerHeader {
            seq: 807,
            hash: sample_hash(0xC0),
            parent_hash: sample_hash(0xC1),
            tx_hash: sample_hash(0xC2),
            account_hash: sample_hash(0xC3),
            drops: 91,
            parent_close_time: 14,
            close_time: 28,
            validated: true,
            accepted: true,
            close_time_resolution: 30,
            close_flags: 1,
        };

        let encoded = serialize_ledger_header(&header, false);
        let mut wire_only = header;
        wire_only.hash = basics::sha_map_hash::SHAMapHash::default();
        wire_only.validated = false;
        wire_only.accepted = false;
        assert_eq!(encoded.len(), LEDGER_HEADER_WIRE_SIZE);
        assert_eq!(deserialize_ledger_header(&encoded, false), Ok(wire_only));
        assert_eq!(
            deserialize_prefixed_ledger_header(
                &serialize_prefixed_ledger_header(&header, false),
                false
            ),
            Ok(wire_only)
        );

        let mut expected = header;
        expected.hash = calculate_ledger_hash(&expected);
        assert_eq!(
            expected.hash.as_uint256().to_string(),
            "164FE77BF8DFEF69714253C0ABEEBE485C7A9CEFDD884202092DF4A1E5954801"
        );
        assert!(!get_close_agree(&expected));
    }
}
