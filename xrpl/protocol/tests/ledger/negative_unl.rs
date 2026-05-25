//! Integration tests for the `NegativeUNL` ledger-entry codec.

use basics::base_uint::Uint256;
use protocol::{
    DecodedDisabledValidator, DecodedNegativeUnlEntry, LedgerEntryDecodeError,
    decode_negative_unl_entry, encode_negative_unl_entry,
};

const OBJECT_END: u8 = 0xE1;
const ARRAY_END: u8 = 0xF1;

fn encode_field_id(field_type: u8, field_name: u8) -> Vec<u8> {
    if field_type < 16 && field_name < 16 {
        vec![(field_type << 4) | field_name]
    } else if field_type < 16 {
        vec![field_type << 4, field_name]
    } else if field_name < 16 {
        vec![field_name, field_type]
    } else {
        vec![0, field_type, field_name]
    }
}

fn append_vl_field(bytes: &mut Vec<u8>, field_name: u8, value: &[u8]) {
    bytes.extend_from_slice(&encode_field_id(7, field_name));
    bytes.push(u8::try_from(value.len()).expect("test payloads should fit one-byte VL prefixes"));
    bytes.extend_from_slice(value);
}

fn append_u16_field(bytes: &mut Vec<u8>, field_name: u8, value: u16) {
    bytes.extend_from_slice(&encode_field_id(1, field_name));
    bytes.extend_from_slice(&value.to_be_bytes());
}

fn append_u32_field(bytes: &mut Vec<u8>, field_name: u8, value: u32) {
    bytes.extend_from_slice(&encode_field_id(2, field_name));
    bytes.extend_from_slice(&value.to_be_bytes());
}

fn append_u256_field(bytes: &mut Vec<u8>, field_name: u8, value: Uint256) {
    bytes.extend_from_slice(&encode_field_id(5, field_name));
    bytes.extend_from_slice(value.data());
}

#[test]
fn negative_unl_entry_round_trips_with_disabled_validators_and_optional_fields() {
    let entry = DecodedNegativeUnlEntry {
        disabled_validators: vec![
            DecodedDisabledValidator {
                public_key: vec![0x02, 0xAA, 0xBB, 0xCC],
                first_ledger_sequence: 512,
            },
            DecodedDisabledValidator {
                public_key: vec![0x03, 0x11, 0x22, 0x33],
                first_ledger_sequence: 768,
            },
        ],
        validator_to_disable: Some(vec![0x02, 0x44, 0x55, 0x66]),
        validator_to_re_enable: Some(vec![0x02, 0x77, 0x88, 0x99]),
        previous_txn_id: Some(Uint256::from_u64(0x1234)),
        previous_txn_lgr_seq: Some(0x0102_0304),
    };

    let encoded = encode_negative_unl_entry(&entry);
    let mut expected = Vec::new();
    append_u16_field(&mut expected, 1, 0x004E);
    expected.extend_from_slice(&encode_field_id(15, 17));

    expected.extend_from_slice(&encode_field_id(14, 19));
    append_vl_field(&mut expected, 1, &[0x02, 0xAA, 0xBB, 0xCC]);
    append_u32_field(&mut expected, 26, 512);
    expected.push(OBJECT_END);

    expected.extend_from_slice(&encode_field_id(14, 19));
    append_vl_field(&mut expected, 1, &[0x03, 0x11, 0x22, 0x33]);
    append_u32_field(&mut expected, 26, 768);
    expected.push(OBJECT_END);

    expected.push(ARRAY_END);
    append_vl_field(&mut expected, 20, &[0x02, 0x44, 0x55, 0x66]);
    append_vl_field(&mut expected, 21, &[0x02, 0x77, 0x88, 0x99]);
    append_u256_field(&mut expected, 5, Uint256::from_u64(0x1234));
    append_u32_field(&mut expected, 5, 0x0102_0304);
    expected.push(OBJECT_END);

    assert_eq!(encoded, expected);
    assert_eq!(
        decode_negative_unl_entry(&encoded).expect("negative unl entry should decode"),
        entry
    );
}

#[test]
fn negative_unl_entry_round_trips_when_optional_fields_are_absent() {
    let entry = DecodedNegativeUnlEntry::default();
    let encoded = encode_negative_unl_entry(&entry);

    assert_eq!(encoded, vec![0x11, 0x00, 0x4E, OBJECT_END]);
    assert_eq!(
        decode_negative_unl_entry(&encoded).expect("negative unl entry should decode"),
        entry
    );
}

#[test]
fn negative_unl_entry_requires_public_key_in_disabled_validator_object() {
    let mut payload = Vec::new();
    append_u16_field(&mut payload, 1, 0x004E);
    payload.extend_from_slice(&encode_field_id(15, 17));
    payload.extend_from_slice(&encode_field_id(14, 19));
    append_u32_field(&mut payload, 26, 512);
    payload.push(OBJECT_END);
    payload.push(ARRAY_END);
    payload.push(OBJECT_END);

    assert_eq!(
        decode_negative_unl_entry(&payload),
        Err(LedgerEntryDecodeError::MissingField("sfPublicKey"))
    );
}

#[test]
fn negative_unl_entry_requires_first_ledger_sequence_in_disabled_validator_object() {
    let mut payload = Vec::new();
    append_u16_field(&mut payload, 1, 0x004E);
    payload.extend_from_slice(&encode_field_id(15, 17));
    payload.extend_from_slice(&encode_field_id(14, 19));
    append_vl_field(&mut payload, 1, &[0x02, 0xAA, 0xBB, 0xCC]);
    payload.push(OBJECT_END);
    payload.push(ARRAY_END);
    payload.push(OBJECT_END);

    assert_eq!(
        decode_negative_unl_entry(&payload),
        Err(LedgerEntryDecodeError::MissingField(
            "sfFirstLedgerSequence"
        ))
    );
}
