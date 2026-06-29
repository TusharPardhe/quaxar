//! Integration test: verify quaxar's SHAMap + hash computation produces identical
//! transaction_hash and ledger_hash to XRPL mainnet.
//!
//! Loads a real mainnet ledger fixture (binary tx/meta blobs + expected hashes),
//! builds the transaction SHAMap using quaxar's actual tree code, computes the
//! root hash, and verifies it matches the live network value.

use basics::base_uint::Uint256;
use basics::sha_map_hash::SHAMapHash;
use basics::intrusive_pointer::make_shared_intrusive;
use protocol::crypto::digest::Sha512HalfHasher;
use protocol::HashPrefix;
use shamap::item::SHAMapItem;
use shamap::tree_node::{SHAMapTreeNode, SHAMapNodeType};
use shamap::mutation::{MutableTree, add_item};

use serde::Deserialize;

#[derive(Deserialize)]
struct LedgerFixture {
    ledger_index: u32,
    ledger_hash: String,
    parent_hash: String,
    transaction_hash: String,
    account_hash: String,
    total_coins: String,
    close_time: u32,
    parent_close_time: u32,
    close_time_resolution: u8,
    close_flags: u8,
    transactions: Vec<TxFixture>,
}

#[derive(Deserialize)]
struct TxFixture {
    tx_blob: String,
    meta_blob: String,
}

fn hex_to_uint256(hex: &str) -> Uint256 {
    Uint256::from_hex(hex).expect("valid hex")
}

/// Compute transaction ID: SHA-512Half(HashPrefix::TransactionId || serialized_tx)
fn compute_tx_id(tx_blob: &[u8]) -> Uint256 {
    let mut hasher = Sha512HalfHasher::new();
    hasher.write(HashPrefix::TransactionId.as_u32().to_be_bytes());
    hasher.write(tx_blob);
    hasher.result()
}

/// Encode variable-length prefix (matching rippled's Serializer::addVL)
fn vl_encode(dest: &mut Vec<u8>, src: &[u8]) {
    let len = src.len();
    if len <= 192 {
        dest.push(len as u8);
    } else if len <= 12480 {
        let adjusted = len - 193;
        dest.push(193 + (adjusted >> 8) as u8);
        dest.push((adjusted & 0xFF) as u8);
    } else {
        let adjusted = len - 12481;
        dest.push(241 + (adjusted >> 16) as u8);
        dest.push(((adjusted >> 8) & 0xFF) as u8);
        dest.push((adjusted & 0xFF) as u8);
    }
    dest.extend_from_slice(src);
}

/// Build the SHAMap leaf data for a transaction node:
/// variable_length(tx_blob) || variable_length(meta_blob)
fn build_tx_leaf_data(tx_blob: &[u8], meta_blob: &[u8]) -> Vec<u8> {
    let mut data = Vec::with_capacity(tx_blob.len() + meta_blob.len() + 6);
    vl_encode(&mut data, tx_blob);
    vl_encode(&mut data, meta_blob);
    data
}

/// Compute ledger header hash: SHA-512Half(HashPrefix::LedgerMaster || header_fields)
fn compute_ledger_hash(fixture: &LedgerFixture) -> Uint256 {
    let mut hasher = Sha512HalfHasher::new();
    hasher.write(HashPrefix::LedgerMaster.as_u32().to_be_bytes());
    hasher.write(fixture.ledger_index.to_be_bytes());
    hasher.write(fixture.total_coins.parse::<u64>().unwrap().to_be_bytes());
    hasher.write(hex_to_uint256(&fixture.parent_hash).data());
    hasher.write(hex_to_uint256(&fixture.transaction_hash).data());
    hasher.write(hex_to_uint256(&fixture.account_hash).data());
    hasher.write(fixture.parent_close_time.to_be_bytes());
    hasher.write(fixture.close_time.to_be_bytes());
    hasher.write([fixture.close_time_resolution]);
    hasher.write([fixture.close_flags]);
    hasher.result()
}

#[test]
fn mainnet_transaction_shamap_root_matches() {
    let fixture_bytes = include_bytes!("fixtures/mainnet_ledger.json");
    let fixture: LedgerFixture =
        serde_json::from_slice(fixture_bytes).expect("fixture should parse");

    println!(
        "\n  Testing ledger {} ({} transactions)",
        fixture.ledger_index,
        fixture.transactions.len()
    );

    // === 1. Build transaction SHAMap from binary blobs ===
    let root_node = make_shared_intrusive(SHAMapTreeNode::new_inner(0));
    let mut tree = MutableTree::from_loaded_root(root_node, 1);

    for (i, tx) in fixture.transactions.iter().enumerate() {
        let tx_blob = hex::decode(&tx.tx_blob).expect("valid tx hex");
        let meta_blob = hex::decode(&tx.meta_blob).expect("valid meta hex");

        // Compute the tx ID (this is the SHAMap key)
        let tx_id = compute_tx_id(&tx_blob);

        // Build the leaf data using protocol::Serializer (exactly as ledger code does)
        let mut payload = protocol::serialization::serializer::Serializer::new(
            tx_blob.len() + meta_blob.len() + 16,
        );
        payload.add_vl(&tx_blob);
        payload.add_vl(&meta_blob);

        if i == 0 {
            println!("  First tx_id: {:?}", tx_id);
            println!("  First item_data len: {}", payload.data().len());
            println!("  First item_data[:20]: {:02x?}", &payload.data()[..20]);
        }

        // Insert into SHAMap
        let item = SHAMapItem::new(tx_id, payload.data().to_vec());
        tree.add_item(SHAMapNodeType::TransactionMd, item)
            .unwrap_or_else(|e| panic!("insert tx {} failed: {:?}", i, e));
    }

    // === 2. Compute and verify transaction_hash (SHAMap root) ===
    // flush_dirty walks the entire tree bottom-up, computing leaf hashes
    // and inner node hashes correctly.
    tree.flush_dirty(&mut |node| node);
    let root = tree.root();
    let computed_tx_root = *root.get_hash().as_uint256();
    let expected_tx_root = hex_to_uint256(&fixture.transaction_hash);

    println!("  Computed transaction_hash: {}", computed_tx_root);
    println!("  Expected transaction_hash: {}", expected_tx_root);

    assert_eq!(
        computed_tx_root, expected_tx_root,
        "SHAMap transaction root must match mainnet ledger {}",
        fixture.ledger_index
    );
    println!("  ✓ transaction_hash MATCHES mainnet (SHAMap verified)");

    // === 3. Verify ledger header hash ===
    let computed_ledger_hash = compute_ledger_hash(&fixture);
    let expected_ledger_hash = hex_to_uint256(&fixture.ledger_hash);

    assert_eq!(
        computed_ledger_hash, expected_ledger_hash,
        "Ledger header hash must match mainnet"
    );
    println!("  ✓ ledger_hash MATCHES mainnet");

    println!(
        "\n  ✓ FULL PARITY: quaxar's SHAMap + hashing is bit-identical to mainnet"
    );
    println!(
        "    Ledger {}: {} transactions verified through SHAMap tree build",
        fixture.ledger_index,
        fixture.transactions.len()
    );
}
