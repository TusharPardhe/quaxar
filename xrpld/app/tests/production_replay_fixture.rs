//! TEST ONLY: immutable production-path replay fixture gate.
//!
//! This integration test is deliberately ignored. It must never read or write
//! the running node's NuDB. Fixture capture/load tooling supplies an immutable
//! parent snapshot and canonical child payload under a separate directory.

use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use basics::{base_uint::Uint256, sha_map_hash::SHAMapHash, string_utilities::str_unhex};
use ledger::{Fees, Ledger};
use protocol::{LedgerHeader, Rules, STTx, SerialIter, Serializer};
use shamap::{
    item::SHAMapItem,
    mutation::MutableTree,
    sync::{SHAMapType, SyncState, SyncTree},
    tree_node::SHAMapNodeType,
};

const LIVE_NODE_STATE_COMPONENT: &str = "state/mainnet-probe";
const MAX_FIXTURE_BYTES: u64 = 8 * 1024 * 1024 * 1024;
const MAX_REPLAY_SECONDS: u64 = 120;

struct ReplayManifest {
    version: u32,
    parent_seq: u32,
    parent_hash: String,
    child_seq: u32,
    child_hash: String,
    transaction_hash: String,
    account_hash: String,
    max_fixture_bytes: u64,
    max_replay_seconds: u64,
}

impl ReplayManifest {
    fn parse(bytes: &[u8]) -> Self {
        let value: serde_json::Value =
            serde_json::from_slice(bytes).expect("parse fixture manifest");
        let field = |name: &str| {
            value[name]
                .as_str()
                .unwrap_or_else(|| panic!("missing string field {name}"))
                .to_owned()
        };
        let number = |name: &str| {
            value[name]
                .as_u64()
                .unwrap_or_else(|| panic!("missing integer field {name}"))
        };
        Self {
            version: number("version") as u32,
            parent_seq: number("parent_seq") as u32,
            parent_hash: field("parent_hash"),
            child_seq: number("child_seq") as u32,
            child_hash: field("child_hash"),
            transaction_hash: field("transaction_hash"),
            account_hash: field("account_hash"),
            max_fixture_bytes: number("max_fixture_bytes"),
            max_replay_seconds: number("max_replay_seconds"),
        }
    }
}

fn fixture_size(path: &Path) -> u64 {
    fs::read_dir(path)
        .expect("fixture directory must be readable")
        .filter_map(Result::ok)
        .map(|entry| {
            let path = entry.path();
            if path.is_dir() {
                fixture_size(&path)
            } else {
                entry.metadata().expect("fixture entry metadata").len()
            }
        })
        .sum()
}

#[test]
#[ignore = "TEST ONLY: requires an explicitly captured immutable parent snapshot"]
fn production_replay_fixture_is_isolated_and_bounded() {
    let root = PathBuf::from(
        std::env::var("QUAXAR_PRODUCTION_REPLAY_FIXTURE")
            .expect("set QUAXAR_PRODUCTION_REPLAY_FIXTURE to an immutable fixture directory"),
    );
    let rendered = root.to_string_lossy();
    assert!(
        !rendered.contains(LIVE_NODE_STATE_COMPONENT),
        "TEST ONLY fixture must not point at the running node state directory"
    );

    let manifest = ReplayManifest::parse(
        &fs::read(root.join("manifest.json")).expect("read fixture manifest"),
    );
    assert_eq!(manifest.version, 1, "unsupported fixture version");
    assert_eq!(manifest.child_seq, manifest.parent_seq + 1);
    for hash in [
        &manifest.parent_hash,
        &manifest.child_hash,
        &manifest.transaction_hash,
        &manifest.account_hash,
    ] {
        assert_eq!(hash.len(), 64, "fixture hashes must be 256-bit hex");
    }
    assert!(
        root.join("parent.snapshot").is_file(),
        "missing parent snapshot"
    );
    assert!(
        root.join("child.json").is_file(),
        "missing canonical child payload"
    );

    let bytes = fixture_size(&root);
    assert!(bytes <= MAX_FIXTURE_BYTES);
    assert!(bytes <= manifest.max_fixture_bytes);
    assert!(manifest.max_replay_seconds <= MAX_REPLAY_SECONDS);

    // The actual replay is intentionally enabled only after the capture tool
    // materializes a full parent snapshot. Keeping this gate separate ensures
    // no production code path can accidentally use live NuDB state.
    let _bounded_budget = Duration::from_secs(manifest.max_replay_seconds);
}

fn json_u64(value: &serde_json::Value, field: &str) -> u64 {
    value[field]
        .as_u64()
        .or_else(|| value[field].as_str().and_then(|value| value.parse().ok()))
        .unwrap_or_else(|| panic!("missing integer field {field}"))
}

fn json_hash(value: &serde_json::Value, field: &str) -> Uint256 {
    Uint256::from_hex(value[field].as_str().expect("hash string"))
        .unwrap_or_else(|_| panic!("invalid 256-bit field {field}"))
}

/// Replays a canonical multi-transaction close over the captured sparse SLE
/// subset supplied by the fixture. This deliberately validates serialized
/// metadata rather than the state root: a sparse state fixture cannot reproduce
/// the untouched branches of the canonical SHAMap, and the test does not prove
/// that the fixture includes every parent SLE read by execution. It can still
/// expose the first transaction whose ApplyStateTable/TxMeta behavior differs
/// from rippled for that supplied view.
#[test]
#[ignore = "TEST ONLY: requires QUAXAR_SPARSE_REPLAY_JSONL"]
fn sparse_production_close_matches_every_canonical_metadata_blob() {
    let path = PathBuf::from(
        std::env::var("QUAXAR_SPARSE_REPLAY_JSONL")
            .expect("set QUAXAR_SPARSE_REPLAY_JSONL to a captured fixture"),
    );
    let rendered = path.to_string_lossy();
    assert!(
        !rendered.contains(LIVE_NODE_STATE_COMPONENT),
        "sparse replay fixture must not reference live node state"
    );
    let fixture_bytes = fs::metadata(&path)
        .expect("stat sparse replay fixture")
        .len();
    assert!(
        fixture_bytes <= MAX_FIXTURE_BYTES,
        "sparse replay fixture exceeds the test-only size cap"
    );
    let file = fs::File::open(path).expect("open sparse replay fixture");
    let mut lines = BufReader::new(file).lines();
    let manifest: serde_json::Value = serde_json::from_str(
        &lines
            .next()
            .expect("manifest row")
            .expect("read manifest row"),
    )
    .expect("parse manifest row");
    assert_eq!(manifest["kind"], "manifest");

    let parent_json = &manifest["parent"];
    let parent_seq = json_u64(parent_json, "ledger_index") as u32;
    let mut state = MutableTree::new(parent_seq);
    for line in lines {
        let row: serde_json::Value =
            serde_json::from_str(&line.expect("read SLE row")).expect("parse SLE row");
        assert_eq!(row["kind"], "sle");
        let key = json_hash(&row, "index");
        let bytes =
            str_unhex(row["data"].as_str().expect("serialized SLE")).expect("valid serialized SLE");
        state
            .add_item(SHAMapNodeType::AccountState, SHAMapItem::new(key, bytes))
            .expect("unique sparse SLE");
    }
    let state_map = SyncTree::from_root_with_type(
        state.root(),
        SHAMapType::State,
        false,
        parent_seq,
        SyncState::Modifying,
    );
    let tx_map = SyncTree::new_with_type(SHAMapType::Transaction, false, parent_seq);
    let header = LedgerHeader {
        seq: parent_seq,
        drops: json_u64(parent_json, "total_coins"),
        hash: SHAMapHash::new(json_hash(parent_json, "ledger_hash")),
        parent_hash: SHAMapHash::new(json_hash(parent_json, "parent_hash")),
        tx_hash: SHAMapHash::new(json_hash(parent_json, "transaction_hash")),
        account_hash: SHAMapHash::new(json_hash(parent_json, "account_hash")),
        parent_close_time: json_u64(parent_json, "parent_close_time") as u32,
        close_time: json_u64(parent_json, "close_time") as u32,
        close_time_resolution: json_u64(parent_json, "close_time_resolution") as u8,
        close_flags: json_u64(parent_json, "close_flags") as u8,
        validated: true,
        accepted: true,
    };
    let mut parent = Ledger::from_maps(header, state_map, tx_map);
    parent.set_fees(Fees {
        base: json_u64(&manifest["fees"], "base"),
        reserve: json_u64(&manifest["fees"], "reserve"),
        increment: json_u64(&manifest["fees"], "increment"),
    });
    parent.set_rules(Rules::new(
        manifest["enabled_amendments"]
            .as_array()
            .expect("enabled amendments")
            .iter()
            .map(|value| Uint256::from_hex(value.as_str().expect("amendment ID")).unwrap()),
    ));

    let transactions = manifest["transactions"].as_array().expect("transactions");
    let txs = transactions
        .iter()
        .map(|row| {
            let bytes = str_unhex(row["tx_hex"].as_str().expect("transaction blob"))
                .expect("valid transaction blob");
            Arc::new(STTx::from_serial_iter(&mut SerialIter::new(&bytes)))
        })
        .collect::<Vec<_>>();
    let child = &manifest["child"];
    let root = app::state::application_root::ApplicationRoot::with_options(
        app::state::application_root::ApplicationRootOptions {
            io_threads: 0,
            job_queue_threads: 1,
            network_id: json_u64(&manifest, "network_id") as u32,
            ..Default::default()
        },
    )
    .expect("sparse replay application");
    root.on_closed_ledger(Arc::new(parent));
    root.accept_ledger_with_txns(
        json_u64(child, "ledger_index") as u32,
        json_u64(child, "close_time") as u32,
        json_u64(child, "close_time_resolution") as u8,
        json_u64(child, "close_flags") == 0,
        json_u64(&manifest["fees"], "base"),
        txs,
    )
    .expect("replay canonical close");
    let built = root.closed_ledger().expect("built child");

    for expected in transactions {
        let tx_id = json_hash(expected, "hash");
        let (_, mut meta) = built
            .tx_read(tx_id)
            .expect("read transaction map")
            .unwrap_or_else(|| panic!("transaction {tx_id} was not accepted"));
        let mut bytes = Serializer::default();
        let result = meta.get_result_ter();
        let index = meta.get_index();
        meta.add_raw(&mut bytes, result, index);
        let actual = basics::str_hex::str_hex(bytes.data());
        assert_eq!(
            actual,
            expected["metadata_hex"]
                .as_str()
                .expect("canonical metadata"),
            "serialized metadata mismatch for transaction {tx_id} at index {index}"
        );
    }
    assert_eq!(
        *built.header().tx_hash.as_uint256(),
        json_hash(child, "transaction_hash"),
        "canonical transaction/metadata SHAMap root"
    );
}
