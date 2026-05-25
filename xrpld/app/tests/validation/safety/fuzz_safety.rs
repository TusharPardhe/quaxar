//! Fuzz/panic safety: feeds random and adversarial bytes to every deserializer.
//! Verifies no panics occur — only graceful errors or partial results.
//!
//! This proves the node won't crash when receiving malformed data from peers.

use basics::base_uint::Uint256;
use basics::sha_map_hash::SHAMapHash;
use protocol::{
    STObject, STTx, STValidation, SerialIter, calc_node_id, deserialize_ledger_header,
    deserialize_prefixed_ledger_header, get_field_by_symbol,
};
use shamap::tree_node::SHAMapTreeNode;

/// Adversarial inputs designed to trigger edge cases.
const FUZZ_INPUTS: &[&[u8]] = &[
    &[],                                         // Empty
    &[0x00],                                     // Single zero
    &[0xFF],                                     // Single 0xFF
    &[0xFF; 32],                                 // 32 bytes of 0xFF
    &[0xFF; 64],                                 // 64 bytes of 0xFF
    &[0xFF; 256],                                // 256 bytes of 0xFF
    &[0x00; 32],                                 // 32 zeros
    &[0x00; 128],                                // 128 zeros
    &[0x12, 0x00],                               // Tx type prefix only
    &[0x12, 0x00, 0x22, 0x80, 0x00, 0x00, 0x00], // Partial payment header
    &[0xDE, 0xAD, 0xBE, 0xEF],                   // Classic garbage
    &[0x14, 0x01],                               // Object end marker
    &[0x15, 0x01],                               // Array end marker
    &[0x04, 0x0F],                               // Invalid expanded type
    &[0x20, 0x0F],                               // Invalid expanded name
    &[0xE0, 0x01],                               // Object type + field 1
    &[0xF0, 0x01],                               // Array type + field 1
    // Nested depth bomb: many object-start markers
    &[
        0xE0, 0x02, 0xE0, 0x02, 0xE0, 0x02, 0xE0, 0x02, 0xE0, 0x02, 0xE0, 0x02, 0xE0, 0x02, 0xE0,
        0x02, 0xE0, 0x02, 0xE0, 0x02, 0xE0, 0x02, 0xE0, 0x02,
    ],
    // VL length bomb: claims huge length
    &[0x70, 0x04, 0xFE, 0xFF, 0xFF],
    // Valid-looking but truncated OfferCreate
    &[
        0x12, 0x00, 0x07, 0x22, 0x00, 0x00, 0x00, 0x00, 0x24, 0x00, 0x00, 0x00, 0x01,
    ],
];

/// Test: STTx::from_serial_iter doesn't panic on any fuzz input.
#[test]
fn sttx_deserializer_no_panic() {
    for (i, input) in FUZZ_INPUTS.iter().enumerate() {
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let mut iter = SerialIter::new(input);
            let _ = STTx::from_serial_iter(&mut iter);
        }));
        assert!(result.is_ok(), "STTx panicked on fuzz input {i}");
    }
}

/// Test: STObject::from_serial_iter doesn't panic on any fuzz input.
#[test]
fn stobject_deserializer_no_panic() {
    for (i, input) in FUZZ_INPUTS.iter().enumerate() {
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let mut iter = SerialIter::new(input);
            let _ = STObject::from_serial_iter(&mut iter, get_field_by_symbol("sfGeneric"), 0);
        }));
        assert!(result.is_ok(), "STObject panicked on fuzz input {i}");
    }
}

/// Test: STValidation::from_serial_iter doesn't panic on any fuzz input.
#[test]
fn stvalidation_deserializer_no_panic() {
    for (i, input) in FUZZ_INPUTS.iter().enumerate() {
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let mut iter = SerialIter::new(input);
            let _ = STValidation::from_serial_iter(&mut iter, calc_node_id, false);
        }));
        assert!(result.is_ok(), "STValidation panicked on fuzz input {i}");
    }
}

/// Test: SHAMapTreeNode::make_from_wire doesn't panic on any fuzz input.
#[test]
fn shamap_wire_decoder_no_panic() {
    for (i, input) in FUZZ_INPUTS.iter().enumerate() {
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _ = SHAMapTreeNode::make_from_wire(input);
        }));
        assert!(
            result.is_ok(),
            "SHAMap wire decoder panicked on fuzz input {i}"
        );
    }
}

/// Test: SHAMapTreeNode::make_from_prefix doesn't panic on any fuzz input.
#[test]
fn shamap_prefix_decoder_no_panic() {
    let dummy_hash = SHAMapHash::new(Uint256::from_u64(0xDEAD));
    for (i, input) in FUZZ_INPUTS.iter().enumerate() {
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _ = SHAMapTreeNode::make_from_prefix(input, dummy_hash);
        }));
        assert!(
            result.is_ok(),
            "SHAMap prefix decoder panicked on fuzz input {i}"
        );
    }
}

/// Test: deserialize_ledger_header doesn't panic on any fuzz input.
#[test]
fn ledger_header_deserializer_no_panic() {
    for (i, input) in FUZZ_INPUTS.iter().enumerate() {
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _ = deserialize_ledger_header(input, false);
            let _ = deserialize_ledger_header(input, true);
            let _ = deserialize_prefixed_ledger_header(input, false);
            let _ = deserialize_prefixed_ledger_header(input, true);
        }));
        assert!(
            result.is_ok(),
            "Ledger header deserializer panicked on fuzz input {i}"
        );
    }
}

/// Test: SerialIter methods don't panic on empty/short data.
#[test]
fn serial_iter_methods_no_panic_on_short_data() {
    let inputs: &[&[u8]] = &[&[], &[0x01], &[0x01, 0x02], &[0x01, 0x02, 0x03]];

    for (i, input) in inputs.iter().enumerate() {
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let mut iter = SerialIter::new(input);
            let _ = iter.get8();
            let mut iter = SerialIter::new(input);
            let _ = iter.get16();
            let mut iter = SerialIter::new(input);
            let _ = iter.get32();
            let mut iter = SerialIter::new(input);
            let mut t = 0i32;
            let mut f = 0i32;
            iter.get_field_id(&mut t, &mut f);
        }));
        assert!(result.is_ok(), "SerialIter panicked on short input {i}");
    }
}

/// Test: Random byte sequences (pseudo-random, deterministic) don't crash any deserializer.
#[test]
fn random_byte_sequences_no_panic() {
    // Generate 50 pseudo-random sequences of varying lengths
    let mut seed: u64 = 0xDEADBEEF_CAFEBABE;
    for trial in 0..50 {
        // Simple LCG for deterministic "random" bytes
        let len = ((seed % 200) + 1) as usize;
        let mut data = Vec::with_capacity(len);
        for _ in 0..len {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
            data.push((seed >> 33) as u8);
        }

        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let mut iter = SerialIter::new(&data);
            let _ = STTx::from_serial_iter(&mut iter);

            let mut iter = SerialIter::new(&data);
            let _ = STObject::from_serial_iter(&mut iter, get_field_by_symbol("sfGeneric"), 0);

            let _ = SHAMapTreeNode::make_from_wire(&data);
        }));
        assert!(
            result.is_ok(),
            "Random trial {trial} (len={len}) caused a panic!"
        );
    }
}
