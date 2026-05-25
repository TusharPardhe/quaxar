//! Validates that real XRPL mainnet validation signatures verify correctly.
//!
//! These are actual validation messages captured from the live XRPL network.
//! If these pass, the node can verify validator signatures from C++ peers.

use protocol::{STValidation, SerialIter, calc_node_id, get_field_by_symbol};

fn hex_to_bytes(hex: &str) -> Vec<u8> {
    (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap())
        .collect()
}

/// Real mainnet validation blobs captured from wss://s1.ripple.com:51233
/// Ledger #104,429,996 — May 23, 2026
const VALIDATION_BLOBS: &[(&str, &str, u32)] = &[
    // (data_hex, expected_ledger_hash, signing_time)
    (
        "228000000126063979AC2931A49BC33A27CCFD80B0329489517FF136F3E1A2ED1DA1225B0A59CFD4D4B2B1D7EEF95CEBAE97798C59174E33F55017977EB9DB760FF134DD81EEAB515A230B20415C08974E83317CB902B5CBE686CD5019CBD081CF3F7677B664BC08E318C13EBBBCD3F174657C8466E1B7226FA9D8FEA9732103466BCD847AF21C71BD411B520ABECDE2646686B3C62F2F15DD73B46ABF16228076473045022100A0B7850D1C47367903AD558F050408A6E8D0713C58B2582A1B9DFF09059472A302207731DFE0B5315646BEFDDF0E7641DF326A0844724EFC8E67CE2A02E02E8C7474",
        "7FF136F3E1A2ED1DA1225B0A59CFD4D4B2B1D7EEF95CEBAE97798C59174E33F5",
        832871363,
    ),
    (
        "228000000126063979AC2931A49BC33ACE7CBA15447A5B62517FF136F3E1A2ED1DA1225B0A59CFD4D4B2B1D7EEF95CEBAE97798C59174E33F55017977EB9DB760FF134DD81EEAB515A230B20415C08974E83317CB902B5CBE686CD5019CBD081CF3F7677B664BC08E318C13EBBBCD3F174657C8466E1B7226FA9D8FEA973210320C4D10F2F28DD2011B26E97C76DDF64E14037C5C8F3A61774B5AE929F3650737647304502210083D4A551442972554D94037F4935020DF69D4F4611D0B914E2B408653BEF4FA7022004C5581A1DDBD4E1BE5CC5BE57B8320DF518423EA0E437B346EBDAFAA5564A79",
        "7FF136F3E1A2ED1DA1225B0A59CFD4D4B2B1D7EEF95CEBAE97798C59174E33F5",
        832871363,
    ),
    (
        "228000000126063979AC2931A49BC43AFB24ABCC9BF9320D517FF136F3E1A2ED1DA1225B0A59CFD4D4B2B1D7EEF95CEBAE97798C59174E33F55017977EB9DB760FF134DD81EEAB515A230B20415C08974E83317CB902B5CBE686CD5019CBD081CF3F7677B664BC08E318C13EBBBCD3F174657C8466E1B7226FA9D8FEA9732103C5557AD9C2A005CCE1BAD8A9606C642023987B8509D927FD5E1C156838EE63F576463044022062ABB1F5D0D073C00E84D307AABCECD1979D2D16E8BEF2F27017EF4EB785199D02203E22F4AFDD278FF6574FF325D99FC3A0F301700F80121966DA43A44FF7A62D9A",
        "7FF136F3E1A2ED1DA1225B0A59CFD4D4B2B1D7EEF95CEBAE97798C59174E33F5",
        832871364,
    ),
];

/// Core test: deserialize real mainnet validations and verify their signatures.
#[test]
fn real_mainnet_validation_signatures_verify() {
    for (i, (data_hex, expected_ledger_hash, expected_time)) in VALIDATION_BLOBS.iter().enumerate()
    {
        let bytes = hex_to_bytes(data_hex);
        let mut iter = SerialIter::new(&bytes);

        let validation = STValidation::from_serial_iter(&mut iter, calc_node_id, true)
            .unwrap_or_else(|e| {
                panic!(
                    "Validation {i} failed to deserialize/verify: {e:?}\n  blob: {}...",
                    &data_hex[..40]
                )
            });

        // Verify signature is valid
        assert!(
            validation.is_valid(),
            "Validation {i}: signature verification failed"
        );

        // Verify ledger hash matches
        let ledger_hash = validation.get_field_h256(get_field_by_symbol("sfLedgerHash"));
        let expected_hash_bytes = hex_to_bytes(expected_ledger_hash);
        assert_eq!(
            ledger_hash.data(),
            expected_hash_bytes.as_slice(),
            "Validation {i}: ledger hash mismatch"
        );

        // Verify signing time
        assert_eq!(
            validation.get_sign_time(),
            *expected_time,
            "Validation {i}: signing time mismatch"
        );

        // Verify it's a full validation (flags & 1)
        assert!(
            validation.is_full(),
            "Validation {i}: should be a full validation"
        );
    }
}

/// Test: Validation with corrupted signature fails verification.
#[test]
fn corrupted_signature_fails_verification() {
    let mut bytes = hex_to_bytes(VALIDATION_BLOBS[0].0);

    // Corrupt the last byte of the signature
    let len = bytes.len();
    bytes[len - 1] ^= 0xFF;

    let mut iter = SerialIter::new(&bytes);
    let result = STValidation::from_serial_iter(&mut iter, calc_node_id, true);

    // Should fail with InvalidSignature
    assert!(
        result.is_err(),
        "Corrupted signature should fail verification"
    );
}

/// Test: Validation roundtrip — serialize and re-deserialize preserves validity.
#[test]
fn validation_roundtrip_preserves_signature() {
    let bytes = hex_to_bytes(VALIDATION_BLOBS[0].0);
    let mut iter = SerialIter::new(&bytes);

    let validation = STValidation::from_serial_iter(&mut iter, calc_node_id, true)
        .expect("first parse should succeed");

    // Re-serialize
    let reserialized = validation.get_serialized();

    // Re-deserialize
    let mut iter2 = SerialIter::new(&reserialized);
    let reparsed = STValidation::from_serial_iter(&mut iter2, calc_node_id, true)
        .expect("roundtrip should succeed");

    assert!(reparsed.is_valid());
    assert_eq!(validation.get_ledger_hash(), reparsed.get_ledger_hash());
}
