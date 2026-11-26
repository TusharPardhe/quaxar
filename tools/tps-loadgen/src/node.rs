//! Small RPC helpers and a pool of connections to the load-receiving nodes.

use crate::ws::{Client, result};
use protocol::{AccountID, to_base58};
use serde_json::{Value, json};
use std::sync::Arc;
use std::time::Duration;

pub struct Pool {
    pub clients: Vec<Arc<Client>>,
}

impl Pool {
    /// `conns` connections to each of `urls`.
    pub async fn connect(urls: &[String], conns: usize) -> Result<Self, String> {
        let mut clients = Vec::new();
        for url in urls {
            for _ in 0..conns {
                clients.push(Client::connect(url).await?);
            }
        }
        Ok(Self { clients })
    }

    /// The connection for `account`: always the same one, so its
    /// transactions arrive in sequence order.
    pub fn route(&self, account: &AccountID) -> &Arc<Client> {
        let b = account.data();
        let h = u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as usize;
        &self.clients[h % self.clients.len()]
    }

    pub fn any(&self) -> &Arc<Client> {
        &self.clients[0]
    }
}

pub async fn validated_index(client: &Client) -> Result<u32, String> {
    let r = client
        .call(json!({"command": "ledger", "ledger_index": "validated"}))
        .await?;
    let r = result(&r)?;
    r.get("ledger_index")
        .and_then(|v| v.as_u64().or_else(|| v.as_str().and_then(|s| s.parse().ok())))
        .or_else(|| {
            r.pointer("/ledger/ledger_index")
                .and_then(|v| v.as_u64().or_else(|| v.as_str().and_then(|s| s.parse().ok())))
        })
        .map(|v| v as u32)
        .ok_or_else(|| format!("no ledger_index in {r}"))
}

/// Fee in drops for a transaction to enter the open ledger now.
pub async fn open_ledger_fee(client: &Client) -> Result<u64, String> {
    let r = client.call(json!({"command": "fee"})).await?;
    let r = result(&r)?;
    r.pointer("/drops/open_ledger_fee")
        .and_then(Value::as_str)
        .and_then(|s| s.parse().ok())
        .ok_or_else(|| format!("no open_ledger_fee in {r}"))
}

/// The account's next sequence and flags, or None if it does not exist.
pub async fn account_info(client: &Client, account: &AccountID) -> Result<Option<(u32, u32)>, String> {
    let r = client
        .call(json!({"command": "account_info", "account": to_base58(*account), "ledger_index": "current"}))
        .await?;
    if r.get("error").and_then(Value::as_str) == Some("actNotFound")
        || r.pointer("/result/error").and_then(Value::as_str) == Some("actNotFound")
    {
        return Ok(None);
    }
    let r = result(&r)?;
    let seq = r.pointer("/account_data/Sequence").and_then(Value::as_u64);
    let flags = r.pointer("/account_data/Flags").and_then(Value::as_u64).unwrap_or(0);
    Ok(seq.map(|s| (s as u32, flags as u32)))
}

/// Wait until the validated ledger reaches `index`.
pub async fn wait_validated(client: &Client, index: u32) -> Result<u32, String> {
    loop {
        let v = validated_index(client).await?;
        if v >= index {
            return Ok(v);
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

pub fn engine_result(response: &Value) -> String {
    response
        .pointer("/result/engine_result")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .or_else(|| response.get("error").and_then(Value::as_str).map(str::to_owned))
        .unwrap_or_else(|| "no_result".to_owned())
}

/// Engine results that leave the transaction on its way into a ledger.
pub fn accepted(code: &str) -> bool {
    code == "tesSUCCESS" || code == "terQUEUED"
}

/// One request per account, at most `MAX_INFLIGHT` in flight; replies in
/// account order. A reply with an error other than `actNotFound` (for
/// example a rate limit) is retried, so callers see either a result or a
/// definite "not found".
pub async fn query_all(
    pool: &Pool,
    accounts: &[AccountID],
    request: impl Fn(&AccountID) -> Value,
) -> Result<Vec<Value>, String> {
    use futures::stream::{FuturesUnordered, StreamExt};
    const MAX_INFLIGHT: usize = 256;
    let mut out = vec![Value::Null; accounts.len()];
    let mut todo: Vec<usize> = (0..accounts.len()).collect();
    for attempt in 0..8 {
        let mut retry = Vec::new();
        let mut inflight = FuturesUnordered::new();
        for &i in &todo {
            let rx = pool.route(&accounts[i]).send(request(&accounts[i])).await?;
            inflight.push(async move { (i, rx.await) });
            while inflight.len() >= MAX_INFLIGHT {
                if let Some((i, r)) = inflight.next().await {
                    settle(&mut out, &mut retry, i, r)?;
                }
            }
        }
        while let Some((i, r)) = inflight.next().await {
            settle(&mut out, &mut retry, i, r)?;
        }
        if retry.is_empty() {
            return Ok(out);
        }
        tokio::time::sleep(Duration::from_millis(500 << attempt.min(4))).await;
        todo = retry;
    }
    Err(format!("{} queries still failing after retries", todo.len()))
}

fn settle(
    out: &mut [Value],
    retry: &mut Vec<usize>,
    i: usize,
    r: Result<Value, tokio::sync::oneshot::error::RecvError>,
) -> Result<(), String> {
    let v = r.map_err(|_| "connection closed".to_string())?;
    let error = v
        .get("error")
        .or_else(|| v.pointer("/result/error"))
        .and_then(Value::as_str);
    match error {
        None | Some("actNotFound") => out[i] = v,
        Some(_) => retry.push(i),
    }
    Ok(())
}
