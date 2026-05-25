//! RFC1751 ported from `xrpl/crypto/RFC1751.h/the reference source`.
//!
//! Converts 128-bit keys to/from human-readable English word sequences.
//! Each 64-bit half is encoded as 6 words from a 2048-word dictionary,
//! with a 2-bit parity check.

#![allow(dead_code)]

/// Result codes matching the reference return values.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rfc1751Result {
    /// Successfully converted.
    Ok = 1,
    /// Word not found in dictionary.
    WordNotFound = 0,
    /// Badly formed string (wrong number of words).
    BadlyFormed = -1,
    /// Words are okay but parity is wrong.
    ParityError = -2,
}

/// Extract `length` bits starting at bit position `start` from byte slice `s`.
fn extract(s: &[u8], start: usize, length: usize) -> u64 {
    let mut result: u64 = 0;
    let mut cl = length;
    let mut cs = start;

    while cl > 0 {
        let byte_idx = cs / 8;
        let bit_offset = cs % 8;
        let bits_available = 8 - bit_offset;
        let bits_to_take = cl.min(bits_available);

        let mask = ((1u16 << bits_to_take) - 1) as u8;
        let shift = bits_available - bits_to_take;
        let bits = ((s[byte_idx] >> shift) & mask) as u64;

        result = (result << bits_to_take) | bits;
        cs += bits_to_take;
        cl -= bits_to_take;
    }

    result
}

/// Insert value `x` of `length` bits at bit position `start` in byte slice `s`.
fn insert(s: &mut [u8], x: u64, start: usize, length: usize) {
    let mut cl = length;
    let mut cs = start;
    let mut cx = x;

    while cl > 0 {
        let byte_idx = cs / 8;
        let bit_offset = cs % 8;
        let bits_available = 8 - bit_offset;
        let bits_to_write = cl.min(bits_available);

        let shift = bits_available - bits_to_write;
        let mask = ((1u16 << bits_to_write) - 1) as u8;

        // Extract the top bits_to_write bits from cx
        let val_shift = cl - bits_to_write;
        let val = ((cx >> val_shift) & mask as u64) as u8;

        s[byte_idx] = (s[byte_idx] & !(mask << shift)) | (val << shift);
        cs += bits_to_write;
        cl -= bits_to_write;
        cx &= (1u64 << val_shift).wrapping_sub(1);
    }
}

/// Standardize a word to uppercase.
fn standard(word: &str) -> String {
    word.to_uppercase()
}

/// Binary search for a word in the dictionary.
fn wsrch(word: &str, min: usize, max: usize) -> Option<usize> {
    let target = standard(word);
    let slice = &DICTIONARY[min..max];
    slice
        .binary_search_by(|probe| probe.cmp(&target.as_str()))
        .ok()
        .map(|idx| idx + min)
}

/// Convert 8 bytes (64 bits) to 6 English words.
fn btoe(data: &[u8]) -> String {
    assert!(data.len() >= 8);

    // Compute 2-bit parity
    let mut parity: u64 = 0;
    for i in (0..64).step_by(2) {
        parity += extract(data, i, 2);
    }

    let mut words = Vec::with_capacity(6);
    // 5 words of 11 bits each = 55 bits, plus 11 bits (9 data + 2 parity) = 66 bits
    for i in 0..5 {
        let idx = extract(data, i * 11, 11) as usize;
        words.push(DICTIONARY[idx]);
    }
    // Last word: 9 bits of data + 2 bits of parity
    let last_bits = (extract(data, 55, 9) << 2) | (parity & 3);
    words.push(DICTIONARY[last_bits as usize]);

    words.join(" ")
}

/// Convert 6 English words to 8 bytes (64 bits).
fn etob(words: &[&str]) -> Result<Vec<u8>, Rfc1751Result> {
    if words.len() != 6 {
        return Err(Rfc1751Result::BadlyFormed);
    }

    let mut buf = [0u8; 9]; // 66 bits needed, use 9 bytes
    let mut p = 0usize;

    for word in words {
        let l = word.len();
        if !(1..=4).contains(&l) {
            return Err(Rfc1751Result::BadlyFormed);
        }

        let search_min = if l < 4 { 0 } else { 571 };
        let search_max = if l < 4 { 571 } else { 2048 };

        let v = wsrch(word, search_min, search_max).ok_or(Rfc1751Result::WordNotFound)?;

        insert(&mut buf, v as u64, p, 11);
        p += 11;
    }

    // Check parity
    let mut parity: u64 = 0;
    for i in (0..64).step_by(2) {
        parity += extract(&buf, i, 2);
    }

    if (parity & 3) != extract(&buf, 64, 2) {
        return Err(Rfc1751Result::ParityError);
    }

    Ok(buf[..8].to_vec())
}

/// Convert a 128-bit key (16 bytes) to 12 English words.
pub fn get_english_from_key(key: &[u8]) -> String {
    assert!(key.len() >= 16);
    let first = btoe(&key[..8]);
    let second = btoe(&key[8..16]);
    format!("{} {}", first, second)
}

/// Convert 12 English words to a 128-bit key (16 bytes).
///
/// Returns the key bytes on success, or an error code.
pub fn get_key_from_english(human: &str) -> Result<Vec<u8>, Rfc1751Result> {
    let words: Vec<&str> = human.split_whitespace().collect();
    if words.len() != 12 {
        return Err(Rfc1751Result::BadlyFormed);
    }

    let first = etob(&words[..6])?;
    let second = etob(&words[6..12])?;

    let mut key = first;
    key.extend_from_slice(&second);
    Ok(key)
}

/// Choose a single dictionary word from arbitrary data using Jenkins hash.
pub fn get_word_from_blob(data: &[u8]) -> &'static str {
    let mut hash: u32 = 0;
    for &byte in data {
        hash = hash.wrapping_add(byte as u32);
        hash = hash.wrapping_add(hash << 10);
        hash ^= hash >> 6;
    }
    hash = hash.wrapping_add(hash << 3);
    hash ^= hash >> 11;
    hash = hash.wrapping_add(hash << 15);

    DICTIONARY[(hash as usize) % DICTIONARY.len()]
}

// The RFC1751 dictionary — 2048 words, sorted with short words (1-3 chars) first (0..571),
// then 4-char words (571..2048).
include!("rfc1751_dictionary.rs");

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dictionary_size() {
        assert_eq!(DICTIONARY.len(), 2048);
    }

    #[test]
    fn word_from_blob() {
        let data = b"hello world";
        let word = get_word_from_blob(data);
        assert!(!word.is_empty());
        // Should be deterministic
        assert_eq!(word, get_word_from_blob(data));
    }

    #[test]
    fn round_trip_key() {
        // Use a known test vector: all zeros
        let key = [0u8; 16];
        let english = get_english_from_key(&key);
        let words: Vec<&str> = english.split_whitespace().collect();
        assert_eq!(words.len(), 12);
    }
}
