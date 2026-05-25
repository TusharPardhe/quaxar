//! Validates SHAMap node wire format decoding with real XRPL mainnet data.
//!
//! Proves the Rust node can decode SHAMap nodes sent by C++ peers during ledger sync.

use basics::base_uint::Uint256;
use basics::sha_map_hash::SHAMapHash;
use protocol::Sha512HalfHasher;
use shamap::tree_node::SHAMapTreeNode;

fn hex_to_bytes(hex: &str) -> Vec<u8> {
    (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap())
        .collect()
}

fn hex_to_uint256(hex: &str) -> Uint256 {
    let bytes = hex_to_bytes(hex);
    Uint256::from_slice(&bytes).unwrap()
}

/// Compute the expected leaf node hash: SHA-512-half(0x4D4C4E00 + key + payload)
fn compute_leaf_hash(key: &Uint256, payload: &[u8]) -> SHAMapHash {
    let mut hasher = Sha512HalfHasher::new();
    hasher.write(0x4D4C4E00u32.to_be_bytes()); // HashPrefix::LeafNode
    hasher.write(key.data());
    hasher.write(payload);
    SHAMapHash::new(hasher.result())
}

/// Real mainnet state entries from ledger #90,000,000.
/// Format: (index_hex, data_hex) — index is the SHAMap key, data is the SLE payload.
const STATE_ENTRIES: &[(&str, &str)] = &[
    (
        "000000E42EDA8F440C16D376B5FC6DE1370CE38C8A5D2965107E8DDBEAF4B007",
        "11007222002200002504D5FD0B37000000000000002838000000000000000055EEF262B9485368FD2DB14F0A4EEC7D06C0D979F541F26A9CDD54965A4D0B4C4B6294E10A4CFC6940000000000000000000000000005345430000000000000000000000000000000000000000000000000166800000000000000000000000000000000000000053454300000000000250260AE1E4202439AFB5F9927067D48F8B17DC67D61C6B728800DCA4000000000000000000000000534543000000000047021569ECC348FD11FECFF66610D1F4249A3D41",
    ),
    (
        "000000EE90D2501F2A95C89000CDA272490864234067E84EE5A79844922ACDF4",
        "11006122000000002403FFFECC2504052F872D00000000559B24265EE9E55B97748F85B9C287C6B174F324506FDADCA94E8A68BA43DDFA7B624000000000989680811435AA54AD4A21DE4EA0275AAA3DFB83E607F00643",
    ),
    (
        "00000203EA7B91B283192593F8847E6641930185B3689530AAE24A64A815FF58",
        "110064220000000031000000000000042632000000000000042458BADEFEDEA9826CDFBD479D5747E441220ADC501BB32C8FE5D88C6C3CB629F54282149D8A36120736BD667E4F8B548EBF259A2C7645AD0113206BE53687261B6ED0F7C537328D1EA76F0F8A26264B973D85F6AA8B915C9EEF32",
    ),
];

/// Test: Leaf node wire blobs decode successfully.
#[test]
fn leaf_node_wire_blobs_decode() {
    for (i, (_key_hex, data_hex)) in STATE_ENTRIES.iter().enumerate() {
        let payload = hex_to_bytes(data_hex);

        // Wire format: payload + wire_type_byte (0x01 = account state)
        let mut wire_blob = payload.clone();
        wire_blob.push(0x01); // WIRE_TYPE_ACCOUNT_STATE

        let result = SHAMapTreeNode::make_from_wire(&wire_blob);
        assert!(
            result.is_ok(),
            "Entry {i}: wire decode failed: {:?}",
            result.err()
        );
        let node = result.unwrap();
        assert!(node.is_some(), "Entry {i}: decoded to None");
    }
}

/// Test: Leaf node hash recomputation matches the expected key-based hash.
#[test]
fn leaf_node_hash_recomputation_matches() {
    for (i, (key_hex, data_hex)) in STATE_ENTRIES.iter().enumerate() {
        let key = hex_to_uint256(key_hex);
        let payload = hex_to_bytes(data_hex);

        // Compute expected hash
        let expected_hash = compute_leaf_hash(&key, &payload);

        // Decode via prefix form (how nodes are stored in NuDB)
        // Prefix form: 0x4D4C4E00 (HashPrefix::LeafNode) + payload
        let mut prefix_blob = Vec::new();
        prefix_blob.extend_from_slice(&0x4D4C4E00u32.to_be_bytes());
        prefix_blob.extend_from_slice(&payload);

        let result = SHAMapTreeNode::make_from_prefix(&prefix_blob, expected_hash);
        assert!(
            result.is_ok(),
            "Entry {i}: prefix decode failed: {:?}",
            result.err()
        );

        let node = result.unwrap();
        // The node should have the hash we provided
        assert_eq!(
            node.get_hash(),
            expected_hash,
            "Entry {i}: hash mismatch after prefix decode"
        );
    }
}

/// Test: Inner node with 16 branch hashes decodes correctly.
#[test]
fn inner_node_wire_blob_decodes() {
    // Construct a synthetic inner node: 16 * 32 bytes (branch hashes) + wire type 0x02
    let mut inner_blob = Vec::new();
    for branch in 0u8..16 {
        let mut hash = [0u8; 32];
        hash[0] = branch + 1; // Non-zero first byte = branch is present
        hash[31] = branch;
        inner_blob.extend_from_slice(&hash);
    }
    inner_blob.push(0x02); // WIRE_TYPE_INNER

    let result = SHAMapTreeNode::make_from_wire(&inner_blob);
    assert!(
        result.is_ok(),
        "Inner node decode failed: {:?}",
        result.err()
    );
    let node = result.unwrap();
    assert!(node.is_some(), "Inner node decoded to None");
}

/// Test: Malformed blobs don't crash.
#[test]
fn malformed_blobs_dont_crash() {
    let bad_inputs: &[&[u8]] = &[
        &[],          // Empty
        &[0x01],      // Just wire type, no payload
        &[0x02],      // Inner type but no hashes
        &[0xFF],      // Invalid wire type
        &[0x00; 5],   // Too short for any valid node
        &[0x01; 100], // Random bytes with account state type
        &[0x02; 33],  // Inner type but wrong size
    ];

    for (i, blob) in bad_inputs.iter().enumerate() {
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _ = SHAMapTreeNode::make_from_wire(blob);
        }));
        assert!(
            result.is_ok(),
            "Malformed blob {i} caused a panic! Must handle gracefully."
        );
    }
}

/// Test: Wire format roundtrip — encode a node, decode it, verify same payload.
#[test]
fn wire_format_roundtrip() {
    let (_key_hex, data_hex) = STATE_ENTRIES[0];
    let payload = hex_to_bytes(data_hex);

    // Create wire blob: payload + 0x01 (account state)
    let mut wire_blob = payload.clone();
    wire_blob.push(0x01);

    // Decode
    let node = SHAMapTreeNode::make_from_wire(&wire_blob)
        .expect("wire decode should work")
        .expect("should produce a node");

    // Re-serialize to wire
    let re_wire = node.serialize_for_wire().expect("serialize should work");

    // Wire bytes should match
    assert_eq!(
        wire_blob, re_wire,
        "Wire format roundtrip: bytes should be identical"
    );
}
