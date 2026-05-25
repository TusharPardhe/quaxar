//! Tests for the ctid RPC handler.

use rpc::{decode_ctid, encode_ctid};

#[test]
fn ctid_round_trip() {
    let encoded = encode_ctid(0x1234567, 0xABCD, 0xEEFF).expect("ctid should encode");
    assert_eq!(encoded, "C1234567ABCDEEFF");
    assert_eq!(
        decode_ctid(encoded.as_str()),
        Some((0x1234567, 0xABCD, 0xEEFF))
    );
    assert_eq!(
        decode_ctid(u64::from_str_radix(&encoded, 16).expect("hex")),
        Some((0x1234567, 0xABCD, 0xEEFF))
    );
    assert_eq!(decode_ctid(-1i32), None);
}

#[test]
fn ctid_encode_decode_edge_cases() {
    // Zero values
    let zero = encode_ctid(0, 0, 0).expect("zero ctid should encode");
    assert_eq!(zero, "C000000000000000");
    assert_eq!(decode_ctid(zero.as_str()), Some((0, 0, 0)));

    // Max values
    let max = encode_ctid(0x0FFFFFFF, 0xFFFF, 0xFFFF).expect("max ctid should encode");
    assert_eq!(
        decode_ctid(max.as_str()),
        Some((0x0FFFFFFF, 0xFFFF, 0xFFFF))
    );

    // Ledger seq too large
    assert!(encode_ctid(0x10000000, 0, 0).is_none());

    // Invalid decode - missing C prefix
    assert_eq!(decode_ctid("0000000000000000"), None);

    // Invalid decode - too short
    assert_eq!(decode_ctid("C123"), None);

    // Invalid decode - not hex
    assert_eq!(decode_ctid("CZZZZZZZZZZZZZZ"), None);
}

#[test]
fn ctid_decode_from_integer() {
    let encoded = encode_ctid(1, 2, 3).expect("ctid should encode");
    let as_u64 = u64::from_str_radix(&encoded, 16).expect("hex");
    assert_eq!(decode_ctid(as_u64), Some((1, 2, 3)));

    // Zero integer
    assert_eq!(decode_ctid(0u64), None);
}
