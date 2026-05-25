//! Integration tests for the `LedgerHashes` skip-list entry seam.

use basics::base_uint::Uint256;
use protocol::{
    DecodedLedgerHashesEntry, LedgerEntryDecodeError, decode_ledger_hashes_entry,
    encode_ledger_hashes_entry,
};

#[test]
fn ledger_hashes_entry_round_trips_with_last_ledger_sequence() {
    let entry = DecodedLedgerHashesEntry {
        hashes: vec![Uint256::from_u64(1), Uint256::from_u64(2)],
        last_ledger_sequence: Some(0x1234_5678),
    };

    let encoded = encode_ledger_hashes_entry(&entry);
    let mut expected = vec![0x11, 0x00, 0x68, 0x20, 0x1B];
    expected.extend_from_slice(&0x1234_5678u32.to_be_bytes());
    expected.extend_from_slice(&[0x02, 0x13, 0x40]);
    expected.extend_from_slice(Uint256::from_u64(1).data());
    expected.extend_from_slice(Uint256::from_u64(2).data());
    expected.push(0xE1);

    assert_eq!(encoded, expected);
    assert_eq!(
        decode_ledger_hashes_entry(&encoded).expect("ledger hashes entry should decode"),
        entry
    );
}

#[test]
fn ledger_hashes_entry_round_trips_without_last_ledger_sequence() {
    let entry = DecodedLedgerHashesEntry {
        hashes: vec![Uint256::from_u64(0xA5A5)],
        last_ledger_sequence: None,
    };

    let encoded = encode_ledger_hashes_entry(&entry);
    let mut expected = vec![0x11, 0x00, 0x68, 0x02, 0x13, 0x20];
    expected.extend_from_slice(Uint256::from_u64(0xA5A5).data());
    expected.push(0xE1);

    assert_eq!(encoded, expected);
    assert_eq!(
        decode_ledger_hashes_entry(&encoded).expect("ledger hashes entry should decode"),
        entry
    );
}

#[test]
fn ledger_hashes_entry_requires_hashes_field() {
    let payload = vec![0x11, 0x00, 0x68, 0x20, 0x1B, 0x00, 0x00, 0x00, 0x01, 0xE1];

    assert_eq!(
        decode_ledger_hashes_entry(&payload),
        Err(LedgerEntryDecodeError::MissingField("sfHashes"))
    );
}
