use basics::{
    base_uint::Uint256, sha_map_hash::SHAMapHash, str_hex::str_hex, string_utilities::str_unhex,
};
use ledger::{Fees, Ledger};
use protocol::{
    LedgerHeader, Rules, STTx, SerialIter, Serializer, dispatchable_tx_types, trans_token,
};
use serde_json::Value;
use shamap::{
    item::SHAMapItem,
    mutation::MutableTree,
    sync::{SHAMapType, SyncState, SyncTree},
    tree_node::SHAMapNodeType,
};
use std::{collections::BTreeMap, sync::Arc};

fn hex256(value: &Value, field: &str) -> Uint256 {
    Uint256::from_hex(value[field].as_str().expect("oracle hex string"))
        .expect("canonical 256-bit oracle value")
}

fn header(value: &Value) -> LedgerHeader {
    LedgerHeader {
        seq: value["seq"].as_u64().expect("header sequence") as u32,
        drops: value["drops"].as_u64().expect("header drops"),
        hash: SHAMapHash::new(hex256(value, "ledger_hash")),
        parent_hash: SHAMapHash::new(hex256(value, "parent_hash")),
        tx_hash: SHAMapHash::new(hex256(value, "tx_hash")),
        account_hash: SHAMapHash::new(hex256(value, "account_hash")),
        parent_close_time: value["parent_close_time"]
            .as_u64()
            .expect("parent close time") as u32,
        close_time: value["close_time"].as_u64().expect("close time") as u32,
        close_time_resolution: value["close_time_resolution"]
            .as_u64()
            .expect("close resolution") as u8,
        close_flags: value["close_flags"].as_i64().expect("close flags") as u8,
        validated: true,
        accepted: true,
    }
}

fn run_oracle_fixture(pre: &Value, result: &Value) {
    let mut state = MutableTree::new(pre["header"]["seq"].as_u64().unwrap() as u32);
    for entry in pre["sles"].as_array().expect("oracle SLE array") {
        let key = hex256(entry, "key");
        let bytes = str_unhex(entry["sle_hex"].as_str().expect("serialized SLE"))
            .expect("canonical serialized SLE");
        state
            .add_item(SHAMapNodeType::AccountState, SHAMapItem::new(key, bytes))
            .expect("unique oracle SLE");
    }
    let mut state_map = SyncTree::from_root_with_type(
        state.root(),
        SHAMapType::State,
        false,
        pre["header"]["seq"].as_u64().unwrap() as u32,
        SyncState::Modifying,
    );
    let tx_map = SyncTree::new_with_type(
        SHAMapType::Transaction,
        false,
        pre["header"]["seq"].as_u64().unwrap() as u32,
    );
    assert_eq!(
        state_map.hash().as_uint256(),
        &hex256(&pre["header"], "account_hash"),
        "exported SLEs must reconstruct the exact pinned prestate root"
    );
    let mut parent = Ledger::from_maps(header(&pre["header"]), state_map, tx_map);
    parent.set_fees(Fees {
        base: pre["fees"]["base"].as_u64().unwrap(),
        reserve: pre["fees"]["reserve"].as_u64().unwrap(),
        increment: pre["fees"]["increment"].as_u64().unwrap(),
    });
    parent.set_rules(Rules::new(
        pre["enabled_amendments"]
            .as_array()
            .unwrap()
            .iter()
            .map(|feature| {
                Uint256::from_hex(feature.as_str().unwrap()).expect("canonical amendment ID")
            }),
    ));

    let tx_bytes = str_unhex(result["tx_hex"].as_str().expect("oracle transaction"))
        .expect("canonical transaction bytes");
    let tx = STTx::from_serial_iter(&mut SerialIter::new(&tx_bytes));
    assert_eq!(
        tx.get_txn_type().to_u16() as u64,
        result["tx_type_code"].as_u64().expect("oracle TxType code")
    );
    assert_eq!(
        tx.get_txn_type().format_name(),
        Some(result["tx_type_name"].as_str().expect("oracle TxType name"))
    );
    assert!(result["applied"].as_bool().expect("oracle applied flag"));
    let tx_id = tx.get_transaction_id();
    let post = &result["post_header"];
    // Exercise the real ApplicationRoot close path, not the bootstrap-only
    // consensus reconstruction helper. Live RCLConsensus calls
    // `accept_ledger_with_txns_outcome_from_consensus_parent`, and this public
    // adapter delegates to the same `accept_ledger_with_txns_outcome_on_parent`
    // transaction/retry/metadata/finalization implementation.
    // The oracle exporter normally runs on the legacy network (ID 0). A
    // modern-network transaction carries sfNetworkID; use that exact value as
    // the node context so this adapter exercises the same preflight0 boundary
    // as the pinned node rather than silently falling back to network 0.
    let network_id_field = protocol::get_field_by_symbol("sfNetworkID");
    let node_network_id = tx
        .is_field_present(network_id_field)
        .then(|| tx.get_field_u32(network_id_field))
        .unwrap_or(0);
    let root = app::state::application_root::ApplicationRoot::with_options(
        app::state::application_root::ApplicationRootOptions {
            io_threads: 0,
            job_queue_threads: 1,
            network_id: node_network_id,
            ..Default::default()
        },
    )
    .expect("oracle application root");
    root.on_closed_ledger(Arc::new(parent));
    let close_time = post["close_time"].as_u64().unwrap() as u32;
    let close_resolution = post["close_time_resolution"].as_u64().unwrap() as u8;
    let correct_close_time = post["close_flags"].as_i64().unwrap() == 0;
    root.accept_ledger_with_txns(
        post["seq"].as_u64().unwrap() as u32,
        close_time,
        close_resolution,
        correct_close_time,
        pre["fees"]["base"].as_u64().unwrap(),
        vec![Arc::new(tx)],
    )
    .expect("Quaxar live close path must build the pinned single-transaction ledger");
    let built = root
        .closed_ledger()
        .expect("live close path must install the built ledger");

    let (_, mut metadata) = built
        .tx_read(tx_id)
        .expect("read built tx map")
        .expect("built transaction present");
    let mut serialized = Serializer::default();
    let ter = metadata.get_result_ter();
    let index = metadata.get_index();
    metadata.add_raw(&mut serialized, ter, index);
    let actual_metadata = str_hex(serialized.data());
    let expected_metadata = result["metadata_hex"].as_str().unwrap();
    let actual_state_root = built.header().account_hash.as_uint256().to_string();
    let expected_state_root = result["state_root"].as_str().unwrap();
    let actual_tx_root = built.header().tx_hash.as_uint256().to_string();
    let expected_tx_root = result["tx_root"].as_str().unwrap();

    if actual_state_root != expected_state_root
        || actual_tx_root != expected_tx_root
        || actual_metadata != expected_metadata
        || ter.to_int() as i64 != result["ter_code"].as_i64().expect("oracle TER code")
        || trans_token(ter) != result["ter_token"].as_str().expect("oracle TER token")
    {
        let mut before = BTreeMap::new();
        for entry in pre["sles"].as_array().expect("oracle SLE array") {
            before.insert(
                entry["key"].as_str().unwrap().to_owned(),
                entry["sle_hex"].as_str().unwrap().to_owned(),
            );
        }
        let mut after = BTreeMap::new();
        built
            .state_map()
            .visit_leaves(&mut |_| None, &mut |item| {
                after.insert(item.key().to_string(), str_hex(item.data()));
            })
            .expect("walk actual post-state");
        let changed = before
            .keys()
            .chain(after.keys())
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .filter_map(|key| {
                let old = before.get(key.as_str());
                let new = after.get(key.as_str());
                (old != new).then(|| {
                    format!(
                        "key={key} before={} after={}",
                        old.map(String::as_str).unwrap_or("<absent>"),
                        new.map(String::as_str).unwrap_or("<absent>")
                    )
                })
            })
            .collect::<Vec<_>>();
        let first_metadata_diff = actual_metadata
            .bytes()
            .zip(expected_metadata.bytes())
            .position(|(actual, expected)| actual != expected)
            .unwrap_or_else(|| actual_metadata.len().min(expected_metadata.len()));
        panic!(
            "exact replay mismatch:\nTER actual={}({}) expected={}({})\nstate_root actual={} expected={}\ntx_root actual={} expected={}\nmetadata first_diff={} actual_len={} expected_len={}\nmetadata_actual={}\nmetadata_expected={}\nactual_post_changes:\n{}",
            trans_token(ter),
            ter.to_int(),
            result["ter_token"].as_str().unwrap(),
            result["ter_code"].as_i64().unwrap(),
            actual_state_root,
            expected_state_root,
            actual_tx_root,
            expected_tx_root,
            first_metadata_diff,
            actual_metadata.len(),
            expected_metadata.len(),
            actual_metadata,
            expected_metadata,
            changed.join("\n")
        );
    }
}

#[test]
#[ignore = "requires QUAXAR_PARITY_ORACLE_JSONL from the pinned rippled oracle"]
fn every_complete_pinned_oracle_matches_exact_roots_and_metadata() {
    let path = std::env::var("QUAXAR_PARITY_ORACLE_JSONL")
        .expect("QUAXAR_PARITY_ORACLE_JSONL is required for the explicit parity gate");
    let rows = std::fs::read_to_string(path)
        .expect("read pinned parity oracle")
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).expect("valid oracle JSONL"))
        .collect::<Vec<_>>();
    let prestates = rows
        .iter()
        .filter(|row| row["kind"] == "prestate")
        .collect::<Vec<_>>();
    assert!(!prestates.is_empty(), "oracle corpus must not be empty");
    let mut pair_counts = BTreeMap::<(String, String), (usize, usize)>::new();
    for row in &rows {
        let fixture = row["fixture"].as_str().expect("fixture name").to_owned();
        let label = row["label"].as_str().expect("fixture label").to_owned();
        let counts = pair_counts.entry((fixture, label)).or_default();
        match row["kind"].as_str().expect("oracle row kind") {
            "prestate" => counts.0 += 1,
            "result" => counts.1 += 1,
            kind => panic!("unsupported oracle row kind {kind}"),
        }
    }
    for ((fixture, label), (prestate_count, result_count)) in &pair_counts {
        assert_eq!(
            (*prestate_count, *result_count),
            (1, 1),
            "fixture {fixture}/{label} must contain exactly one prestate and one result"
        );
    }
    let mut replay_failures = Vec::new();
    for pre in prestates {
        let fixture = pre["fixture"].as_str().expect("fixture name");
        let label = pre["label"].as_str().expect("fixture label");
        let results = rows
            .iter()
            .filter(|row| {
                row["fixture"] == fixture && row["label"] == label && row["kind"] == "result"
            })
            .collect::<Vec<_>>();
        assert_eq!(
            results.len(),
            1,
            "fixture {fixture}/{label} must have exactly one result row"
        );
        let pre_header = &pre["header"];
        let post_header = &results[0]["post_header"];
        assert_eq!(
            post_header["seq"].as_u64().expect("post sequence"),
            pre_header["seq"].as_u64().expect("pre sequence") + 1,
            "fixture {fixture}/{label} must describe one adjacent ledger transition"
        );
        assert_eq!(
            post_header["parent_hash"]
                .as_str()
                .expect("post parent hash"),
            pre_header["ledger_hash"].as_str().expect("pre ledger hash"),
            "fixture {fixture}/{label} post ledger must descend from its exported prestate"
        );
        eprintln!(
            "replaying {fixture}/{label}: {} ({})",
            results[0]["tx_type_name"].as_str().expect("TxType name"),
            results[0]["tx_type_code"].as_u64().expect("TxType code")
        );
        if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            run_oracle_fixture(pre, results[0]);
        }))
        .is_err()
        {
            replay_failures.push(format!(
                "{fixture}/{label}: {} ({})",
                results[0]["tx_type_name"].as_str().expect("TxType name"),
                results[0]["tx_type_code"].as_u64().expect("TxType code")
            ));
        }
    }

    let corpus_types = rows
        .iter()
        .filter(|row| row["kind"] == "result")
        .map(|row| row["tx_type_code"].as_u64().expect("oracle TxType code") as u16)
        .collect::<std::collections::BTreeSet<_>>();
    let supported_types = dispatchable_tx_types()
        .map(|tx_type| tx_type.to_u16())
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(
        corpus_types, supported_types,
        "pinned oracle must contain an exact applied golden for every supported transactor TxType"
    );
    assert!(
        replay_failures.is_empty(),
        "exact pinned-rippled replay failures:\n{}",
        replay_failures.join("\n")
    );
}
