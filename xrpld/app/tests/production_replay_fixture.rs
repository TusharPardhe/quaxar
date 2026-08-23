//! TEST ONLY: immutable production-path replay fixture gate.
//!
//! This integration test is deliberately ignored. It must never read or write
//! the running node's NuDB. Fixture capture/load tooling supplies an immutable
//! parent snapshot and canonical child payload under a separate directory.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

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
