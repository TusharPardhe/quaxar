//! Byte-identical parity between the direct JSON writer and the
//! `JsonValue` tree path over real ledger data.
//!
//! Fixture: `tests/fixtures/json_corpus.txt.gz`, produced by
//! `scripts/fetch_json_corpus.py` from public mainnet and testnet nodes
//! (ledger entries of every type seen in recent metadata, a natural
//! `ledger_data` mix, singletons, and recent transactions with metadata).

use std::collections::BTreeMap;
use std::io::Read;
use std::panic::{AssertUnwindSafe, catch_unwind};

use basics::base_uint::Uint256;
use basics::string_utilities::str_unhex;
use protocol::json_writer::{to_json_bytes, with_raw_rendering};
use protocol::{
    JsonOptions, STLedgerEntry, STObject, STTx, SerialIter, StBase, TxMeta, get_field_by_symbol,
};

pub(crate) enum Record {
    Entry {
        index: Uint256,
        data: Vec<u8>,
    },
    Tx {
        ledger: u32,
        tx: Vec<u8>,
        meta: Vec<u8>,
    },
}

pub(crate) fn load_corpus() -> Vec<Record> {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/json_corpus.txt.gz"
    );
    let file = std::fs::File::open(path).expect("json corpus fixture present");
    let mut text = String::new();
    flate2::read::GzDecoder::new(file)
        .read_to_string(&mut text)
        .expect("corpus is gzip text");
    text.lines()
        .filter(|line| !line.is_empty())
        .map(|line| {
            let parts: Vec<&str> = line.split(' ').collect();
            match parts[0] {
                "S" => Record::Entry {
                    index: Uint256::from_hex(parts[3]).expect("index hex"),
                    data: str_unhex(parts[4]).expect("entry hex"),
                },
                "T" => Record::Tx {
                    ledger: parts[2].parse().expect("ledger seq"),
                    tx: str_unhex(parts[3]).expect("tx hex"),
                    meta: str_unhex(parts[4]).expect("meta hex"),
                },
                other => panic!("unknown record kind {other}"),
            }
        })
        .collect()
}

fn assert_same(label: &str, value: &dyn StBase, options: JsonOptions) {
    let direct = to_json_bytes(value, options);
    let tree = sonic_rs::to_vec(&value.json(options)).expect("tree serializes");
    if direct != tree {
        panic!(
            "{label}: direct writer differs\n direct: {}\n   tree: {}",
            String::from_utf8_lossy(&direct),
            String::from_utf8_lossy(&tree)
        );
    }
}

fn entry_type_name(sle: &STLedgerEntry) -> String {
    format!("{:?}", sle.get_type())
}

fn tx_type_name(tx: &STTx) -> String {
    format!("{:?}", tx.get_txn_type())
}

#[test]
fn direct_writer_matches_tree_on_real_ledger_corpus() {
    let corpus = load_corpus();
    let mut entry_types: BTreeMap<String, usize> = BTreeMap::new();
    let mut tx_types: BTreeMap<String, usize> = BTreeMap::new();
    let mut undecodable = 0;

    for record in &corpus {
        match record {
            Record::Entry { index, data } => {
                let Ok(sle) =
                    STLedgerEntry::try_from_serial_iter(&mut SerialIter::new(data), *index)
                else {
                    undecodable += 1;
                    continue;
                };
                assert_same(&format!("entry {index}"), &sle, JsonOptions::NONE);
                *entry_types.entry(entry_type_name(&sle)).or_default() += 1;
            }
            Record::Tx { ledger, tx, meta } => {
                let tx = STTx::from_serial_iter(&mut SerialIter::new(tx));
                let id = tx.get_transaction_id();
                for options in [JsonOptions::NONE, JsonOptions::DISABLE_API_PRIOR_V2] {
                    assert_same(&format!("tx {id}"), &tx, options);
                }
                let meta_object = STObject::from_serial_iter(
                    &mut SerialIter::new(meta),
                    get_field_by_symbol("sfMetadata"),
                    0,
                );
                assert_same(&format!("meta {id}"), &meta_object, JsonOptions::NONE);
                let tx_meta = TxMeta::from_raw(id, *ledger, meta);
                assert_same(
                    &format!("TxMeta {id}"),
                    &tx_meta.get_as_object(),
                    JsonOptions::NONE,
                );

                // Shallow raw rendering (RPC dispatch / streams): top level
                // stays a tree, nested objects and arrays are raw.
                for options in [
                    JsonOptions::NONE,
                    JsonOptions::DISABLE_API_PRIOR_V2,
                    JsonOptions::INCLUDE_DATE | JsonOptions::DISABLE_API_PRIOR_V2,
                ] {
                    let tree = sonic_rs::to_vec(&tx.get_json_binary(options, false)).unwrap();
                    let raw = with_raw_rendering(|| {
                        sonic_rs::to_vec(&tx.get_json_binary(options, false)).unwrap()
                    });
                    assert_eq!(raw, tree, "shallow tx {id}");
                    let tree = sonic_rs::to_vec(&tx_meta.get_json(options)).unwrap();
                    let raw = with_raw_rendering(|| {
                        sonic_rs::to_vec(&tx_meta.get_json(options)).unwrap()
                    });
                    assert_eq!(raw, tree, "shallow meta {id}");
                }
                *tx_types.entry(tx_type_name(&tx)).or_default() += 1;
            }
        }
    }

    let entries: usize = entry_types.values().sum();
    let txs: usize = tx_types.values().sum();
    println!(
        "ledger entries: {entries} across {} types: {entry_types:?}",
        entry_types.len()
    );
    println!(
        "transactions: {txs} across {} types: {tx_types:?}",
        tx_types.len()
    );
    println!("undecodable entries skipped: {undecodable}");
    // Guard against a silently empty or truncated fixture.
    assert!(entries >= 2_000, "corpus too small: {entries} entries");
    assert!(
        entry_types.len() >= 12,
        "too few entry types: {entry_types:?}"
    );
    assert!(txs >= 500, "corpus too small: {txs} transactions");
    assert!(tx_types.len() >= 10, "too few tx types: {tx_types:?}");
    assert_eq!(undecodable, 0);
}

/// Differential fuzzing: corrupt real blobs and require that whenever the
/// existing decoder + tree path succeeds, the direct writer matches it.
#[test]
fn direct_writer_matches_tree_on_mutated_corpus() {
    let corpus = load_corpus();
    let mut state = 0x9E37_79B9_7F4A_7C15_u64;
    let mut next = move || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    };
    let mut compared = 0;
    let original_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    for record in corpus.iter().cycle().take(30_000) {
        let (mut bytes, index) = match record {
            Record::Entry { index, data } => (data.clone(), Some(*index)),
            Record::Tx { meta, .. } => (meta.clone(), None),
        };
        if bytes.is_empty() {
            continue;
        }
        for _ in 0..(1 + next() % 3) {
            let position = (next() as usize) % bytes.len();
            bytes[position] = next() as u8;
        }
        let outcome = catch_unwind(AssertUnwindSafe(|| {
            let value: Box<dyn StBase> = match index {
                Some(index) => Box::new(
                    STLedgerEntry::try_from_serial_iter(&mut SerialIter::new(&bytes), index)
                        .ok()?,
                ),
                None => Box::new(STObject::from_serial_iter(
                    &mut SerialIter::new(&bytes),
                    get_field_by_symbol("sfMetadata"),
                    0,
                )),
            };
            let tree = sonic_rs::to_vec(&value.json(JsonOptions::NONE)).ok()?;
            Some((value, tree))
        }));
        let Ok(Some((value, tree))) = outcome else {
            continue;
        };
        let direct = catch_unwind(AssertUnwindSafe(|| {
            to_json_bytes(&*value, JsonOptions::NONE)
        }));
        let direct = direct.expect("direct writer must not panic where the tree path succeeds");
        assert_eq!(
            String::from_utf8_lossy(&direct),
            String::from_utf8_lossy(&tree),
            "mutated record diverged"
        );
        compared += 1;
    }
    std::panic::set_hook(original_hook);
    println!("mutated values compared: {compared}");
    assert!(compared > 5_000, "too few decodable mutations: {compared}");
}
