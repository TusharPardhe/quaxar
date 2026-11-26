//! Record every validated ledger and check that all validators agree.
//!
//! Subscribes to the `ledger` stream of the first validator. For each
//! validated ledger it records the transaction count, ledger close time and
//! the local arrival time; the gap between arrivals is the publish interval
//! the 5-second deadline applies to. It asks every validator for the hash of
//! that ledger. Every `sample_every` ledgers it reads the ledger's
//! transactions and counts results by transaction type.

use crate::ws::{Client, result};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::io::Write;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

pub async fn watch(validators: &[String], seconds: f64, out_path: &str, sample_every: u64) -> Result<(), String> {
    let mut clients = Vec::new();
    for url in validators {
        clients.push(Client::connect(url).await?);
    }
    let stream_client = Client::connect(&validators[0]).await?;
    let sub = stream_client.call(json!({"command": "subscribe", "streams": ["ledger"]})).await?;
    result(&sub)?;
    let clients = Arc::new(clients);
    let mut out = std::fs::File::create(out_path).map_err(|e| format!("{out_path}: {e}"))?;
    let (line_tx, mut line_rx) = tokio::sync::mpsc::unbounded_channel::<Value>();
    let start = Instant::now();
    let mut stream = stream_client.stream.lock().await;
    let mut last_arrival: Option<f64> = None;
    loop {
        let remaining = seconds - start.elapsed().as_secs_f64();
        if remaining <= 0.0 {
            break;
        }
        tokio::select! {
            message = stream.recv() => {
                let Some(m) = message else { return Err("ledger stream closed".into()) };
                if m.get("type").and_then(Value::as_str) != Some("ledgerClosed") {
                    continue;
                }
                let now = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0.0, |d| d.as_secs_f64());
                let index = m.get("ledger_index").and_then(Value::as_u64).unwrap_or(0);
                let hash = m.get("ledger_hash").and_then(Value::as_str).unwrap_or("").to_owned();
                let interval = last_arrival.map(|t| now - t);
                last_arrival = Some(now);
                let line_tx = line_tx.clone();
                let clients = Arc::clone(&clients);
                let txn_count = m.get("txn_count").cloned().unwrap_or(Value::Null);
                let ledger_time = m.get("ledger_time").cloned().unwrap_or(Value::Null);
                tokio::spawn(async move {
                    let hashes = agree(&clients, index).await;
                    let agree_all = hashes.iter().all(|h| h.as_deref() == Some(hash.as_str()));
                    let _ = line_tx.send(json!({
                        "kind": "ledger", "unix": now, "ledger_index": index, "ledger_hash": hash,
                        "txn_count": txn_count, "ledger_time": ledger_time,
                        "interval_s": interval, "validators_agree": agree_all, "validator_hashes": hashes,
                    }));
                    if sample_every > 0 && index % sample_every == 0 {
                        if let Ok(counts) = results(&clients[0], index).await {
                            let _ = line_tx.send(json!({"kind": "results", "ledger_index": index, "counts": counts}));
                        }
                    }
                });
            }
            line = line_rx.recv() => {
                if let Some(line) = line {
                    writeln!(out, "{line}").map_err(|e| e.to_string())?;
                }
            }
            _ = tokio::time::sleep(Duration::from_secs_f64(remaining.min(1.0))) => {}
        }
    }
    // Drain lines still being produced.
    tokio::time::sleep(Duration::from_secs(3)).await;
    while let Ok(line) = line_rx.try_recv() {
        writeln!(out, "{line}").map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Each validator's hash for ledger `index`, waiting briefly for laggards.
async fn agree(clients: &[Arc<Client>], index: u64) -> Vec<Option<String>> {
    let mut hashes = Vec::new();
    for c in clients {
        let mut found = None;
        for _ in 0..10 {
            if let Ok(r) = c.call(json!({"command": "ledger", "ledger_index": index})).await {
                if let Some(h) = r.pointer("/result/ledger_hash").and_then(Value::as_str) {
                    found = Some(h.to_owned());
                    break;
                }
            }
            tokio::time::sleep(Duration::from_millis(300)).await;
        }
        hashes.push(found);
    }
    hashes
}

/// `TransactionType -> TransactionResult -> count` for ledger `index`.
async fn results(client: &Client, index: u64) -> Result<BTreeMap<String, BTreeMap<String, u64>>, String> {
    let r = client
        .call(json!({"command": "ledger", "ledger_index": index, "transactions": true, "expand": true}))
        .await?;
    let txs = r
        .pointer("/result/ledger/transactions")
        .and_then(Value::as_array)
        .ok_or("no transactions")?;
    let mut counts: BTreeMap<String, BTreeMap<String, u64>> = BTreeMap::new();
    for t in txs {
        let ty = t.get("TransactionType").and_then(Value::as_str).unwrap_or("?").to_owned();
        let res = t
            .pointer("/metaData/TransactionResult")
            .or_else(|| t.pointer("/meta/TransactionResult"))
            .and_then(Value::as_str)
            .unwrap_or("?")
            .to_owned();
        *counts.entry(ty).or_default().entry(res).or_insert(0) += 1;
    }
    Ok(counts)
}
