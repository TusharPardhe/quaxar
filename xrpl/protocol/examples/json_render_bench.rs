//! JSON rendering benchmark over the real-ledger corpus
//! (`tests/fixtures/json_corpus.txt.gz`).
//!
//! cargo run --release -p protocol --example json_render_bench
//!
//! For each record class reports ns per record for:
//!   decode  - binary -> STLedgerEntry / STTx / metadata STObject
//!   tree    - value.json() + sonic_rs::to_vec   (previous path)
//!   direct  - json_writer::to_json_bytes        (no JsonValue tree)
//!   shallow - raw-mode shallow_json + to_vec    (what tx/meta RPC paths use)
//! Every direct rendering is asserted byte-identical to the tree rendering.

use std::collections::BTreeMap;
use std::hint::black_box;
use std::io::Read;
use std::time::Instant;

use basics::base_uint::Uint256;
use basics::string_utilities::str_unhex;
use protocol::json_writer::{shallow_json, to_json_bytes, with_raw_rendering};
use protocol::{
    JsonOptions, STLedgerEntry, STObject, STTx, SerialIter, StBase, get_field_by_symbol,
};

fn load() -> (Vec<(Uint256, Vec<u8>)>, Vec<(Vec<u8>, Vec<u8>)>) {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/json_corpus.txt.gz"
    );
    let mut text = String::new();
    flate2::read::GzDecoder::new(std::fs::File::open(path).expect("corpus"))
        .read_to_string(&mut text)
        .expect("gzip");
    let mut entries = Vec::new();
    let mut txs = Vec::new();
    for line in text.lines().filter(|line| !line.is_empty()) {
        let parts: Vec<&str> = line.split(' ').collect();
        match parts[0] {
            "S" => entries.push((
                Uint256::from_hex(parts[3]).unwrap(),
                str_unhex(parts[4]).unwrap(),
            )),
            "T" => txs.push((str_unhex(parts[3]).unwrap(), str_unhex(parts[4]).unwrap())),
            _ => {}
        }
    }
    (entries, txs)
}

/// ns per item for `op` over `items`, repeated until ~0.4 s has elapsed.
fn ns_per<T>(items: &[T], mut op: impl FnMut(&T)) -> f64 {
    for item in items.iter().take(200) {
        op(item);
    }
    let start = Instant::now();
    let mut rounds = 0_u64;
    while start.elapsed().as_millis() < 400 || rounds == 0 {
        for item in items {
            op(item);
        }
        rounds += 1;
    }
    start.elapsed().as_nanos() as f64 / (rounds as f64 * items.len() as f64)
}

fn row(label: &str, count: usize, bytes: f64, decode: f64, tree: f64, fast: f64, fast_label: &str) {
    println!(
        "{label:<22} {count:>6} {bytes:>8.0} {decode:>9.0} {tree:>9.0} {fast:>9.0} {:>7.2}x  ({fast_label})",
        tree / fast
    );
}

fn main() {
    let (raw_entries, raw_txs) = load();
    let entries: Vec<STLedgerEntry> = raw_entries
        .iter()
        .map(|(key, data)| STLedgerEntry::from_serial_iter(&mut SerialIter::new(data), *key))
        .collect();
    let txs: Vec<STTx> = raw_txs
        .iter()
        .map(|(tx, _)| STTx::from_serial_iter(&mut SerialIter::new(tx)))
        .collect();
    let metas: Vec<STObject> = raw_txs
        .iter()
        .map(|(_, meta)| {
            STObject::from_serial_iter(
                &mut SerialIter::new(meta),
                get_field_by_symbol("sfMetadata"),
                0,
            )
        })
        .collect();

    // `JSON_BENCH_PROFILE=<EntryType>` spins the direct writer for that type
    // (for an external sampling profiler) instead of benchmarking.
    if let Ok(kind) = std::env::var("JSON_BENCH_PROFILE") {
        let sample: Vec<&STLedgerEntry> = entries
            .iter()
            .filter(|e| format!("{:?}", e.get_type()) == kind)
            .collect();
        let start = Instant::now();
        while start.elapsed().as_secs() < 8 {
            for entry in &sample {
                black_box(to_json_bytes(*entry, JsonOptions::NONE));
            }
        }
        return;
    }

    // Parity first.
    for value in entries
        .iter()
        .map(|v| v as &dyn StBase)
        .chain(txs.iter().map(|v| v as &dyn StBase))
        .chain(metas.iter().map(|v| v as &dyn StBase))
    {
        assert_eq!(
            to_json_bytes(value, JsonOptions::NONE),
            sonic_rs::to_vec(&value.json(JsonOptions::NONE)).unwrap()
        );
    }

    println!(
        "{:<22} {:>6} {:>8} {:>9} {:>9} {:>9} {:>8}",
        "class", "count", "bytes", "decode", "tree", "new", "speedup"
    );
    println!(
        "{:<22} {:>6} {:>8} {:>9} {:>9} {:>9}",
        "", "", "(json)", "ns", "ns", "ns"
    );

    // Ledger entries, per type with enough samples, then all.
    let mut by_type: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    for (i, entry) in entries.iter().enumerate() {
        by_type
            .entry(format!("{:?}", entry.get_type()))
            .or_default()
            .push(i);
    }
    let mut groups: Vec<(String, Vec<usize>)> =
        by_type.into_iter().filter(|(_, v)| v.len() >= 20).collect();
    groups.push(("all ledger entries".into(), (0..entries.len()).collect()));
    for (name, indices) in groups {
        let sample: Vec<&STLedgerEntry> = indices.iter().map(|i| &entries[*i]).collect();
        let raw: Vec<&(Uint256, Vec<u8>)> = indices.iter().map(|i| &raw_entries[*i]).collect();
        let bytes = sample
            .iter()
            .map(|e| to_json_bytes(*e, JsonOptions::NONE).len())
            .sum::<usize>() as f64
            / sample.len() as f64;
        let decode = ns_per(&raw, |(key, data)| {
            black_box(STLedgerEntry::from_serial_iter(
                &mut SerialIter::new(data),
                *key,
            ));
        });
        let tree = ns_per(&sample, |e| {
            black_box(sonic_rs::to_vec(&e.json(JsonOptions::NONE)).unwrap());
        });
        let direct = ns_per(&sample, |e| {
            black_box(to_json_bytes(*e, JsonOptions::NONE));
        });
        row(&name, sample.len(), bytes, decode, tree, direct, "direct");
    }

    let tx_bytes = txs
        .iter()
        .map(|t| to_json_bytes(t, JsonOptions::NONE).len())
        .sum::<usize>() as f64
        / txs.len() as f64;
    let decode = ns_per(&raw_txs, |(tx, _)| {
        black_box(STTx::from_serial_iter(&mut SerialIter::new(tx)));
    });
    let tree = ns_per(&txs, |t| {
        black_box(sonic_rs::to_vec(&t.json(JsonOptions::NONE)).unwrap());
    });
    let direct = ns_per(&txs, |t| {
        black_box(to_json_bytes(t, JsonOptions::NONE));
    });
    let shallow = ns_per(&txs, |t| {
        black_box(with_raw_rendering(|| {
            sonic_rs::to_vec(&shallow_json(t, JsonOptions::NONE)).unwrap()
        }));
    });
    row(
        "transactions",
        txs.len(),
        tx_bytes,
        decode,
        tree,
        direct,
        "direct",
    );
    row(
        "transactions",
        txs.len(),
        tx_bytes,
        decode,
        tree,
        shallow,
        "shallow",
    );

    let meta_bytes = metas
        .iter()
        .map(|m| to_json_bytes(m, JsonOptions::NONE).len())
        .sum::<usize>() as f64
        / metas.len() as f64;
    let decode = ns_per(&raw_txs, |(_, meta)| {
        black_box(STObject::from_serial_iter(
            &mut SerialIter::new(meta),
            get_field_by_symbol("sfMetadata"),
            0,
        ));
    });
    let tree = ns_per(&metas, |m| {
        black_box(sonic_rs::to_vec(&m.json(JsonOptions::NONE)).unwrap());
    });
    let direct = ns_per(&metas, |m| {
        black_box(to_json_bytes(m, JsonOptions::NONE));
    });
    let shallow = ns_per(&metas, |m| {
        black_box(with_raw_rendering(|| {
            sonic_rs::to_vec(&shallow_json(m, JsonOptions::NONE)).unwrap()
        }));
    });
    row(
        "metadata",
        metas.len(),
        meta_bytes,
        decode,
        tree,
        direct,
        "direct",
    );
    row(
        "metadata",
        metas.len(),
        meta_bytes,
        decode,
        tree,
        shallow,
        "shallow",
    );
}
