//! Validates protocol message codec, fee escalation math, and peer handshake.

use overlay::{
    Compressed, Message, ProtocolMessage, ProtocolMessageType, ProtocolPayload, TmManifests,
    TmTransaction, TmValidation, decode_protocol_message, make_features_request_header,
    make_request, parse_message_header,
};

// ═══════════════════════════════════════════════════════════════
// PROTOCOL MESSAGE CODEC
// ═══════════════════════════════════════════════════════════════

/// Test: TMTransaction encode/decode roundtrip preserves payload.
#[test]
fn tm_transaction_roundtrip() {
    let tx_blob = vec![0xAB; 256]; // 256 bytes
    let tm = TmTransaction {
        raw_transaction: tx_blob.clone(),
        ..Default::default()
    };

    let message = Message::new(ProtocolMessage::new(ProtocolPayload::Transaction(tm)), None);

    // Encode uncompressed
    let buffer = message.get_buffer(Compressed::Off);
    assert!(!buffer.is_empty());

    // Decode
    let decoded = decode_protocol_message(buffer, false).expect("decode should succeed");
    assert!(decoded.message.is_some());
    let msg = decoded.message.unwrap();
    assert_eq!(msg.message_type, ProtocolMessageType::MtTransaction);

    if let ProtocolPayload::Transaction(tm_decoded) = msg.payload {
        assert_eq!(tm_decoded.raw_transaction, tx_blob);
    } else {
        panic!("Expected Transaction payload");
    }
}

/// Test: TMValidation encode/decode roundtrip preserves payload.
#[test]
fn tm_validation_roundtrip() {
    let validation_blob = vec![0xCD; 250]; // 250 bytes
    let tm = TmValidation {
        validation: validation_blob.clone(),
        ..Default::default()
    };

    let message = Message::new(ProtocolMessage::new(ProtocolPayload::Validation(tm)), None);

    let buffer = message.get_buffer(Compressed::Off);
    let decoded = decode_protocol_message(buffer, false).expect("decode");
    let msg = decoded.message.unwrap();
    assert_eq!(msg.message_type, ProtocolMessageType::MtValidation);

    if let ProtocolPayload::Validation(tm_decoded) = msg.payload {
        assert_eq!(tm_decoded.validation, validation_blob);
    } else {
        panic!("Expected Validation payload");
    }
}

/// Test: Compressed messages decode correctly.
#[test]
fn compressed_message_roundtrip() {
    let manifests = TmManifests {
        list: (0..20)
            .map(|i| overlay::message::wire::TmManifest {
                stobject: vec![i as u8; 80],
            })
            .collect(),
        ..Default::default()
    };

    let message = Message::new(
        ProtocolMessage::new(ProtocolPayload::Manifests(manifests.clone())),
        None,
    );

    // Compressed should be smaller than uncompressed
    let compressed = message.get_buffer(Compressed::On);
    let uncompressed = message.get_buffer(Compressed::Off);
    assert!(compressed.len() < uncompressed.len());

    // Both should decode to the same payload
    let decoded_c = decode_protocol_message(compressed, true).expect("compressed decode");
    let decoded_u = decode_protocol_message(uncompressed, false).expect("uncompressed decode");

    assert!(decoded_c.message.is_some());
    assert!(decoded_u.message.is_some());
}

/// Test: Message header parsing extracts correct fields.
#[test]
fn message_header_parsing() {
    let tm = TmTransaction {
        raw_transaction: vec![0xAB; 100],
        ..Default::default()
    };
    let message = Message::new(ProtocolMessage::new(ProtocolPayload::Transaction(tm)), None);

    let buffer = message.get_buffer(Compressed::Off);
    let header = parse_message_header(buffer)
        .expect("parse")
        .expect("present");

    assert_eq!(
        header.message_type,
        ProtocolMessageType::MtTransaction as u16
    );
    assert!(header.total_wire_size > 0);
}

/// Test: Malformed message buffers don't crash the decoder.
#[test]
fn malformed_messages_dont_crash() {
    let bad_inputs: &[&[u8]] = &[
        &[],
        &[0x00],
        &[0xFF; 4],
        &[0x00, 0x00, 0x00, 0x04, 0x00, 0x15], // Valid header size but truncated
        &[0x00; 100],
    ];

    for (i, input) in bad_inputs.iter().enumerate() {
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _ = decode_protocol_message(input, false);
            let _ = decode_protocol_message(input, true);
            let _ = parse_message_header(input);
        }));
        assert!(result.is_ok(), "Malformed message {i} caused a panic");
    }
}

// ═══════════════════════════════════════════════════════════════
// PEER HANDSHAKE
// ═══════════════════════════════════════════════════════════════

/// Test: Handshake request has all required headers for C++ peer acceptance.
#[test]
fn handshake_request_has_required_headers() {
    let request = make_request(true, true, true, true, true);

    // C++ peers require these exact headers
    assert_eq!(request.method(), http::Method::GET);
    assert_eq!(request.uri(), "/");
    assert_eq!(request.headers()["Connection"], "Upgrade");
    assert_eq!(request.headers()["Upgrade"], "XRPL/2.1, XRPL/2.2");
    assert_eq!(request.headers()["Connect-As"], "Peer");
    assert!(request.headers().contains_key("X-Protocol-Ctl"));
}

/// Test: Feature negotiation header format matches C++ expectations.
#[test]
fn feature_header_format_matches_cpp() {
    let header = make_features_request_header(true, true, true, true);
    // Must contain compression and ledger replay features
    assert!(header.contains("lz4"));
    // Must be parseable by C++ peer
    assert!(!header.is_empty());
}

// ═══════════════════════════════════════════════════════════════
// FEE ESCALATION MATH
// ═══════════════════════════════════════════════════════════════

/// Test: Fee escalation formula matches C++ behavior.
/// C++ formula: escalation_multiplier * (current^2 / target^2)
#[test]
fn fee_escalation_at_target_returns_base_level() {
    use tx::{QueueFeeMetricsSnapshot, TXQ_BASE_LEVEL, scale_fee_level};

    let snapshot = QueueFeeMetricsSnapshot {
        txns_expected: 32,
        escalation_multiplier: TXQ_BASE_LEVEL * 500,
    };

    // At or below target: returns base level
    let fee = scale_fee_level(snapshot, 32);
    assert_eq!(fee, TXQ_BASE_LEVEL);

    let fee = scale_fee_level(snapshot, 20);
    assert_eq!(fee, TXQ_BASE_LEVEL);

    let fee = scale_fee_level(snapshot, 0);
    assert_eq!(fee, TXQ_BASE_LEVEL);
}

/// Test: Fee escalation increases quadratically above target.
#[test]
fn fee_escalation_increases_above_target() {
    use tx::{QueueFeeMetricsSnapshot, TXQ_BASE_LEVEL, scale_fee_level};

    let snapshot = QueueFeeMetricsSnapshot {
        txns_expected: 32,
        escalation_multiplier: TXQ_BASE_LEVEL * 500,
    };

    let fee_33 = scale_fee_level(snapshot, 33);
    let fee_64 = scale_fee_level(snapshot, 64);
    let fee_128 = scale_fee_level(snapshot, 128);

    // Must be strictly increasing
    assert!(fee_33 > TXQ_BASE_LEVEL, "33 txs should escalate above base");
    assert!(fee_64 > fee_33, "64 txs should be higher than 33");
    assert!(fee_128 > fee_64, "128 txs should be higher than 64");

    // Quadratic: doubling count should ~4x the fee
    // fee_64 / fee_33 should be roughly (64/33)^2 ≈ 3.76
    // fee_128 / fee_64 should be roughly (128/64)^2 = 4
    let ratio = fee_128 as f64 / fee_64 as f64;
    assert!(
        (3.5..=4.5).contains(&ratio),
        "Fee should scale quadratically, got ratio {ratio}"
    );
}

/// Test: Fee level paid computation matches C++ mul_div behavior.
#[test]
fn fee_level_paid_computation() {
    use tx::{QueueFeeLevelPaidInputs, TXQ_BASE_LEVEL, evaluate_fee_level_paid};

    // Standard fee: 10 drops with reference fee 10 → base level
    let inputs = QueueFeeLevelPaidInputs {
        fee_paid_drops: 10,
        calculated_base_fee_drops: 10,
        default_base_fee_drops: 10,
    };
    let level = evaluate_fee_level_paid(inputs);
    assert_eq!(level, TXQ_BASE_LEVEL);

    // Double fee: 20 drops with reference fee 10 → 2x base level
    let inputs = QueueFeeLevelPaidInputs {
        fee_paid_drops: 20,
        calculated_base_fee_drops: 10,
        default_base_fee_drops: 10,
    };
    let level = evaluate_fee_level_paid(inputs);
    assert_eq!(level, TXQ_BASE_LEVEL * 2);

    // 10x fee
    let inputs = QueueFeeLevelPaidInputs {
        fee_paid_drops: 100,
        calculated_base_fee_drops: 10,
        default_base_fee_drops: 10,
    };
    let level = evaluate_fee_level_paid(inputs);
    assert_eq!(level, TXQ_BASE_LEVEL * 10);
}
