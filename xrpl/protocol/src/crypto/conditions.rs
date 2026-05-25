//! `libxrpl/conditions` parity — Crypto-conditions (RFC draft-thomas-crypto-conditions)
//!
//! Only PreimageSha256 is supported (the only type used by XRPL escrow).
//! DER parsing validates condition and fulfillment blobs, and `validate`
//! checks that a fulfillment's SHA-256 fingerprint matches the condition.

use sha2::{Digest, Sha256};

/// Crypto-condition type tags.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum ConditionType {
    PreimageSha256 = 0,
    PrefixSha256 = 1,
    ThresholdSha256 = 2,
    RsaSha256 = 3,
    Ed25519Sha256 = 4,
}

/// Error codes for crypto-condition parsing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CryptoConditionError {
    UnsupportedType,
    UnknownType,
    FingerprintSize,
    IncorrectEncoding,
    TrailingGarbage,
    BufferEmpty,
    BufferOverfull,
    BufferUnderfull,
    MalformedEncoding,
    UnexpectedTag,
    ShortPreamble,
    LongTag,
    LargeSize,
    PreimageTooLong,
}

/// Maximum preimage length (128 bytes per reference spec).
pub const MAX_PREIMAGE_LENGTH: usize = 128;

/// Maximum serialized condition size.
const MAX_SERIALIZED_CONDITION: usize = 128;

/// Maximum serialized fulfillment size.
const MAX_SERIALIZED_FULFILLMENT: usize = 256;

/// A parsed crypto-condition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Condition {
    pub condition_type: ConditionType,
    pub cost: u32,
    pub fingerprint: [u8; 32],
}

/// A parsed crypto-condition fulfillment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fulfillment {
    pub fulfillment_type: ConditionType,
    payload: Vec<u8>,
}

impl Fulfillment {
    /// Derive the condition from this fulfillment.
    pub fn condition(&self) -> Condition {
        Condition {
            condition_type: self.fulfillment_type,
            cost: self.payload.len() as u32,
            fingerprint: self.fingerprint(),
        }
    }

    /// Compute the SHA-256 fingerprint of the preimage payload.
    pub fn fingerprint(&self) -> [u8; 32] {
        let mut hasher = Sha256::new();
        hasher.update(&self.payload);
        hasher.finalize().into()
    }

    /// Validate the fulfillment against a message.
    /// For PreimageSha256, the message is irrelevant — always valid.
    pub fn validate(&self, _message: &[u8]) -> bool {
        true
    }
}

/// Check if a fulfillment matches a condition.
///
pub fn fulfillment_matches(f: &Fulfillment, c: &Condition) -> bool {
    if f.fulfillment_type != c.condition_type {
        return false;
    }
    f.condition() == *c
}

/// Validate a fulfillment against a condition and message.
///
pub fn validate(f: &Fulfillment, c: &Condition, message: &[u8]) -> bool {
    fulfillment_matches(f, c) && f.validate(message)
}

/// Validate a fulfillment against a condition (no message).
///
pub fn validate_no_message(f: &Fulfillment, c: &Condition) -> bool {
    validate(f, c, &[])
}

/// Deserialize a DER-encoded condition.
///
pub fn deserialize_condition(data: &[u8]) -> Result<Condition, CryptoConditionError> {
    if data.is_empty() {
        return Err(CryptoConditionError::BufferEmpty);
    }

    let (tag, constructed, class_bits, content) = parse_outer_preamble(data)?;

    if !constructed || class_bits != 0x80 {
        return Err(CryptoConditionError::MalformedEncoding);
    }

    if content.len() > MAX_SERIALIZED_CONDITION {
        return Err(CryptoConditionError::LargeSize);
    }

    match tag {
        0 => parse_simple_sha256_condition(ConditionType::PreimageSha256, content),
        1..=4 => Err(CryptoConditionError::UnsupportedType),
        _ => Err(CryptoConditionError::UnknownType),
    }
}

/// Deserialize a DER-encoded fulfillment.
///
pub fn deserialize_fulfillment(data: &[u8]) -> Result<Fulfillment, CryptoConditionError> {
    if data.is_empty() {
        return Err(CryptoConditionError::BufferEmpty);
    }

    let (tag, constructed, class_bits, content) = parse_outer_preamble(data)?;

    if !constructed || class_bits != 0x80 {
        return Err(CryptoConditionError::MalformedEncoding);
    }

    if content.len() > MAX_SERIALIZED_FULFILLMENT {
        return Err(CryptoConditionError::LargeSize);
    }

    match tag {
        0 => parse_preimage_fulfillment(content),
        1..=4 => Err(CryptoConditionError::UnsupportedType),
        _ => Err(CryptoConditionError::UnknownType),
    }
}

// --- DER parsing internals ---

/// Parse outer TLV preamble, return (tag, constructed, class_bits, content_slice).
fn parse_outer_preamble(data: &[u8]) -> Result<(u8, bool, u8, &[u8]), CryptoConditionError> {
    if data.len() < 2 {
        return Err(CryptoConditionError::ShortPreamble);
    }

    let id_byte = data[0];
    let constructed = (id_byte & 0x20) != 0;
    let class_bits = id_byte & 0xC0;
    let tag = id_byte & 0x1F;

    if tag == 0x1F {
        return Err(CryptoConditionError::LongTag);
    }

    let (length, header_len) = parse_der_length(&data[1..])?;

    let total = header_len + 1 + length;
    if total > data.len() {
        return Err(CryptoConditionError::BufferUnderfull);
    }
    if total < data.len() {
        return Err(CryptoConditionError::TrailingGarbage);
    }

    Ok((tag, constructed, class_bits, &data[header_len + 1..]))
}

/// Parse inner TLV preamble (primitive, context-specific).
fn parse_inner_preamble(data: &[u8]) -> Result<(u8, usize, usize), CryptoConditionError> {
    if data.len() < 2 {
        return Err(CryptoConditionError::ShortPreamble);
    }

    let id_byte = data[0];
    let is_primitive = (id_byte & 0x20) == 0;
    let is_context_specific = (id_byte & 0xC0) == 0x80;

    if !is_primitive || !is_context_specific {
        return Err(CryptoConditionError::IncorrectEncoding);
    }

    let tag = id_byte & 0x1F;
    let (length, len_bytes) = parse_der_length(&data[1..])?;
    let header_size = 1 + len_bytes;

    Ok((tag, length, header_size))
}

/// Parse DER length encoding. Returns (length_value, bytes_consumed).
fn parse_der_length(data: &[u8]) -> Result<(usize, usize), CryptoConditionError> {
    if data.is_empty() {
        return Err(CryptoConditionError::ShortPreamble);
    }

    let first = data[0];
    if first < 0x80 {
        return Ok((first as usize, 1));
    }

    let num_bytes = (first & 0x7F) as usize;
    if num_bytes == 0 || num_bytes > 4 {
        return Err(CryptoConditionError::LargeSize);
    }
    if data.len() < 1 + num_bytes {
        return Err(CryptoConditionError::ShortPreamble);
    }

    let mut length: usize = 0;
    for i in 0..num_bytes {
        length = (length << 8) | (data[1 + i] as usize);
    }

    Ok((length, 1 + num_bytes))
}

/// Parse a SimpleSha256Condition: fingerprint (32 bytes) + cost (u32).
fn parse_simple_sha256_condition(
    cond_type: ConditionType,
    data: &[u8],
) -> Result<Condition, CryptoConditionError> {
    let mut pos = 0;

    // Parse fingerprint field (tag 0, 32 bytes)
    let (tag, length, header_size) = parse_inner_preamble(&data[pos..])?;
    if tag != 0 {
        return Err(CryptoConditionError::UnexpectedTag);
    }
    if length != 32 {
        return Err(CryptoConditionError::FingerprintSize);
    }
    pos += header_size;
    if pos + length > data.len() {
        return Err(CryptoConditionError::BufferUnderfull);
    }
    let mut fingerprint = [0u8; 32];
    fingerprint.copy_from_slice(&data[pos..pos + 32]);
    pos += 32;

    // Parse cost field (tag 1, integer)
    let (tag, length, header_size) = parse_inner_preamble(&data[pos..])?;
    if tag != 1 {
        return Err(CryptoConditionError::UnexpectedTag);
    }
    pos += header_size;
    if pos + length > data.len() {
        return Err(CryptoConditionError::BufferUnderfull);
    }
    let cost = parse_der_integer(&data[pos..pos + length])?;
    pos += length;

    if pos != data.len() {
        return Err(CryptoConditionError::TrailingGarbage);
    }

    // Validate cost for preimage type
    if cond_type == ConditionType::PreimageSha256 && cost > MAX_PREIMAGE_LENGTH as u32 {
        return Err(CryptoConditionError::PreimageTooLong);
    }

    Ok(Condition {
        condition_type: cond_type,
        cost,
        fingerprint,
    })
}

/// Parse a PreimageFulfillment: single octet string (the preimage).
fn parse_preimage_fulfillment(data: &[u8]) -> Result<Fulfillment, CryptoConditionError> {
    let (tag, length, header_size) = parse_inner_preamble(data)?;
    if tag != 0 {
        return Err(CryptoConditionError::UnexpectedTag);
    }

    let content_start = header_size;
    if content_start + length != data.len() {
        return Err(CryptoConditionError::TrailingGarbage);
    }

    if length > MAX_PREIMAGE_LENGTH {
        return Err(CryptoConditionError::PreimageTooLong);
    }

    let payload = data[content_start..content_start + length].to_vec();

    Ok(Fulfillment {
        fulfillment_type: ConditionType::PreimageSha256,
        payload,
    })
}

/// Parse a big-endian unsigned integer from DER bytes.
fn parse_der_integer(data: &[u8]) -> Result<u32, CryptoConditionError> {
    if data.is_empty() || data.len() > 5 {
        return Err(CryptoConditionError::MalformedEncoding);
    }
    let mut value: u64 = 0;
    for &byte in data {
        value = (value << 8) | (byte as u64);
    }
    if value > u32::MAX as u64 {
        return Err(CryptoConditionError::LargeSize);
    }
    Ok(value as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preimage_fulfillment_round_trips() {
        // A PreimageSha256 fulfillment with preimage "hello"
        // DER: A0 07 80 05 68 65 6C 6C 6F
        let der = [0xA0, 0x07, 0x80, 0x05, b'h', b'e', b'l', b'l', b'o'];
        let f = deserialize_fulfillment(&der).expect("valid fulfillment");
        assert_eq!(f.fulfillment_type, ConditionType::PreimageSha256);
        assert_eq!(f.payload, b"hello");

        // Fingerprint should be SHA-256("hello")
        let expected_hash = Sha256::digest(b"hello");
        assert_eq!(f.fingerprint(), expected_hash.as_slice());

        // Condition derived from fulfillment
        let c = f.condition();
        assert_eq!(c.condition_type, ConditionType::PreimageSha256);
        assert_eq!(c.cost, 5); // length of "hello"
        assert_eq!(c.fingerprint, expected_hash.as_slice());

        // Validate
        assert!(validate(&f, &c, b"any message"));
        assert!(validate_no_message(&f, &c));
    }

    #[test]
    #[allow(clippy::vec_init_then_push)]
    fn condition_deserialize_preimage() {
        // A PreimageSha256 condition with fingerprint=SHA256("hello"), cost=5
        let fingerprint: [u8; 32] = Sha256::digest(b"hello").into();
        let mut der = Vec::new();
        // Outer: constructed, context-specific, tag 0
        der.push(0xA0);
        // Length of inner content: 2 + 32 + 2 + 1 = 37
        der.push(37);
        // Fingerprint: tag 0, length 32
        der.push(0x80);
        der.push(32);
        der.extend_from_slice(&fingerprint);
        // Cost: tag 1, length 1, value 5
        der.push(0x81);
        der.push(1);
        der.push(5);

        let c = deserialize_condition(&der).expect("valid condition");
        assert_eq!(c.condition_type, ConditionType::PreimageSha256);
        assert_eq!(c.cost, 5);
        assert_eq!(c.fingerprint, fingerprint);
    }

    #[test]
    fn empty_buffer_returns_error() {
        assert_eq!(
            deserialize_condition(&[]),
            Err(CryptoConditionError::BufferEmpty)
        );
        assert_eq!(
            deserialize_fulfillment(&[]),
            Err(CryptoConditionError::BufferEmpty)
        );
    }

    #[test]
    fn unsupported_type_returns_error() {
        // Tag 4 = Ed25519Sha256 (unsupported)
        let der = [0xA4, 0x02, 0x80, 0x00];
        assert_eq!(
            deserialize_fulfillment(&der),
            Err(CryptoConditionError::UnsupportedType)
        );
    }

    #[test]
    fn preimage_too_long_returns_error() {
        // Fulfillment with 129-byte preimage (exceeds MAX_PREIMAGE_LENGTH)
        // Outer: A0 [length of inner] inner...
        // Inner: 80 81 81 [129 bytes]  (tag=0, length=129 in 2-byte form)
        let inner_len = 1 + 2 + 129; // tag(1) + length_encoding(2) + data(129) = 132
        let mut der = vec![0xA0, 0x81, inner_len as u8]; // outer: tag 0, length in 2-byte form
        der.push(0x80); // inner tag 0
        der.push(0x81); // length: 1 byte follows
        der.push(129); // length = 129
        der.extend(vec![0x41; 129]);
        assert_eq!(
            deserialize_fulfillment(&der),
            Err(CryptoConditionError::PreimageTooLong)
        );
    }
}
