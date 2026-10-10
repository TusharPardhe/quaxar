//! End-to-end loopback latency harness for the RPC server transport.
//!
//! Spins up `RpcServer` with a synthetic dispatcher on 127.0.0.1 and drives it
//! with keep-alive HTTP/1.1 clients or WebSocket clients, reporting latency
//! percentiles and throughput. The dispatcher work is identical across builds,
//! so differences isolate the transport (parse, convert, schedule, serialize,
//! write) path.
//!
//! Usage:
//!   cargo run --release -p server --example rpc_latency -- <mode> <method> <conns> <reqs>
//!     mode:   http | ws | fanout
//!     method: ping | large            (ignored for fanout)
//!   fanout: <conns> = subscribers, <reqs> = published events
//!
//! Not a substitute for a matched rippled comparison; see docs/perf.

use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use futures::{SinkExt, StreamExt};
use protocol::JsonValue;
use server::{RpcDispatcher, RpcReply, RpcRequest, RpcServer, StreamKind};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio_tungstenite::tungstenite::Message;

fn s(v: &str) -> JsonValue {
    JsonValue::String(v.to_owned())
}

fn obj(fields: Vec<(&str, JsonValue)>) -> JsonValue {
    JsonValue::Object(
        fields
            .into_iter()
            .map(|(k, v)| (k.to_owned(), v))
            .collect::<BTreeMap<_, _>>(),
    )
}

/// account_tx-shaped payload: `count` transactions with metadata.
fn large_result(count: usize) -> JsonValue {
    let mut txs = Vec::with_capacity(count);
    for i in 0..count as u64 {
        let node = obj(vec![(
            "ModifiedNode",
            obj(vec![
                (
                    "FinalFields",
                    obj(vec![
                        ("Account", s("rHb9CJAWyB4rj91VRWn96DkukG4bwdtyTh")),
                        ("Balance", s("99999999988")),
                        ("Flags", JsonValue::Unsigned(0)),
                        ("OwnerCount", JsonValue::Unsigned(3)),
                        ("Sequence", JsonValue::Unsigned(1000 + i)),
                    ]),
                ),
                ("LedgerEntryType", s("AccountRoot")),
                (
                    "LedgerIndex",
                    s("13F1A95D7AAB7108D5CE7EEAF504B2894B8C674E6D68499076441C4837282BF8"),
                ),
                (
                    "PreviousFields",
                    obj(vec![
                        ("Balance", s("99999999999")),
                        ("Sequence", JsonValue::Unsigned(999 + i)),
                    ]),
                ),
                (
                    "PreviousTxnID",
                    s("E3FE6EA3D48F0C2B639448020EA4F03D4F4F8FFDB243A852A0F59177921B4879"),
                ),
                ("PreviousTxnLgrSeq", JsonValue::Unsigned(21_000_000 + i)),
            ]),
        )]);
        txs.push(obj(vec![
            (
                "meta",
                obj(vec![
                    ("AffectedNodes", JsonValue::Array(vec![node.clone(), node.clone(), node])),
                    ("TransactionIndex", JsonValue::Unsigned(i)),
                    ("TransactionResult", s("tesSUCCESS")),
                    ("delivered_amount", s("1000000")),
                ]),
            ),
            (
                "tx",
                obj(vec![
                    ("Account", s("rHb9CJAWyB4rj91VRWn96DkukG4bwdtyTh")),
                    ("Amount", s("1000000")),
                    ("Destination", s("rPT1Sjq2YGrBMTttX4GZHjKu9dyfzbpAYe")),
                    ("Fee", s("12")),
                    ("Flags", JsonValue::Unsigned(2_147_483_648)),
                    ("Sequence", JsonValue::Unsigned(1000 + i)),
                    (
                        "SigningPubKey",
                        s("03AB40A0490F9B7ED8DF29D246BF2D6269820A0EE7742ACDD457BEA7C7D0931EDB"),
                    ),
                    ("TransactionType", s("Payment")),
                    (
                        "TxnSignature",
                        s("30450221009C195DBBF7967E223D8626CA19CF02073667F2B22E206727BFE848FF42BEAC8A022048C323B0BED19A988BDBEFA974B6DE8AA9DCAE250AA82BBD1221787032A864E5"),
                    ),
                    ("date", JsonValue::Unsigned(780_000_000 + i)),
                    (
                        "hash",
                        s("E08D6E9754025BA2534A78707605E0601F03ACE063687A0CA1BDDACFCD1698C7"),
                    ),
                    ("ledger_index", JsonValue::Unsigned(21_000_000 + i)),
                ]),
            ),
            ("validated", JsonValue::Bool(true)),
        ]));
    }
    obj(vec![
        ("account", s("rHb9CJAWyB4rj91VRWn96DkukG4bwdtyTh")),
        ("ledger_index_max", JsonValue::Unsigned(21_000_200)),
        ("ledger_index_min", JsonValue::Unsigned(21_000_000)),
        ("limit", JsonValue::Unsigned(count as u64)),
        ("transactions", JsonValue::Array(txs)),
        ("validated", JsonValue::Bool(true)),
    ])
}

struct SynthDispatcher {
    large: JsonValue,
}

impl RpcDispatcher for SynthDispatcher {
    fn dispatch(&self, request: RpcRequest<'_>) -> RpcReply {
        match request.method {
            "subscribe" => {
                if let Some(session) = request.session {
                    session.subscribe_stream(StreamKind::Transactions);
                }
                RpcReply::result(obj(vec![]))
            }
            "account_tx" => RpcReply::result(self.large.clone()),
            _ => RpcReply::result(obj(vec![])),
        }
    }
}

fn method_name(method: &str) -> &'static str {
    match method {
        "large" => "account_tx",
        _ => "ping",
    }
}

fn report(label: &str, mut lat: Vec<Duration>, wall: Duration, bytes: usize) {
    lat.sort_unstable();
    let n = lat.len();
    let pct = |p: f64| lat[((n as f64 * p) as usize).min(n - 1)];
    println!(
        "{label}: n={n} rps={:.0} p50={:?} p90={:?} p99={:?} p999={:?} max={:?} resp_bytes={bytes}",
        n as f64 / wall.as_secs_f64(),
        pct(0.50),
        pct(0.90),
        pct(0.99),
        pct(0.999),
        lat[n - 1],
    );
}

async fn http_client(
    addr: SocketAddr,
    method: &'static str,
    reqs: usize,
) -> (Vec<Duration>, usize) {
    let mut stream = TcpStream::connect(addr).await.expect("connect");
    stream.set_nodelay(true).expect("client nodelay");
    let body = format!(r#"{{"method":"{method}","params":[{{"ledger_index":"current"}}],"id":1}}"#);
    let request = format!(
        "POST / HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
        body.len(),
        body
    );
    let mut buf = Vec::with_capacity(1 << 20);
    let mut lat = Vec::with_capacity(reqs);
    let mut last_len = 0;
    for _ in 0..reqs {
        let start = Instant::now();
        stream.write_all(request.as_bytes()).await.expect("write");
        buf.clear();
        // Read headers.
        let header_end = loop {
            let mut chunk = [0_u8; 16384];
            let read = stream.read(&mut chunk).await.expect("read");
            assert!(read > 0, "server closed connection");
            buf.extend_from_slice(&chunk[..read]);
            if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                break pos + 4;
            }
        };
        let headers = std::str::from_utf8(&buf[..header_end]).expect("utf8 headers");
        let content_length: usize = headers
            .lines()
            .find_map(|line| {
                let lower = line.to_ascii_lowercase();
                lower
                    .strip_prefix("content-length:")
                    .map(|v| v.trim().parse().expect("content-length"))
            })
            .expect("content-length header");
        while buf.len() < header_end + content_length {
            let mut chunk = vec![0_u8; 65536];
            let read = stream.read(&mut chunk).await.expect("read body");
            assert!(read > 0, "server closed mid-body");
            buf.extend_from_slice(&chunk[..read]);
        }
        lat.push(start.elapsed());
        last_len = content_length;
    }
    (lat, last_len)
}

async fn ws_client(addr: SocketAddr, method: &'static str, reqs: usize) -> (Vec<Duration>, usize) {
    let (mut socket, _) = tokio_tungstenite::connect_async(format!("ws://{addr}/"))
        .await
        .expect("ws connect");
    let request = format!(r#"{{"command":"{method}","ledger_index":"current","id":1}}"#);
    let mut lat = Vec::with_capacity(reqs);
    let mut last_len = 0;
    for _ in 0..reqs {
        let start = Instant::now();
        socket
            .send(Message::Text(request.clone().into()))
            .await
            .expect("ws send");
        loop {
            match socket.next().await.expect("ws open").expect("ws frame") {
                Message::Text(text) => {
                    last_len = text.len();
                    break;
                }
                _ => continue,
            }
        }
        lat.push(start.elapsed());
    }
    (lat, last_len)
}

async fn fanout(
    addr: SocketAddr,
    server_subs: Arc<server::SubscriptionManager>,
    subs: usize,
    events: usize,
) {
    let mut sockets = Vec::with_capacity(subs);
    for _ in 0..subs {
        let (mut socket, _) = tokio_tungstenite::connect_async(format!("ws://{addr}/"))
            .await
            .expect("ws connect");
        socket
            .send(Message::Text(
                r#"{"command":"subscribe","streams":["transactions"],"id":1}"#.into(),
            ))
            .await
            .expect("subscribe");
        let _ack = socket.next().await;
        sockets.push(socket);
    }
    let payload = match large_result(1) {
        JsonValue::Object(mut o) => o.remove("transactions").unwrap(),
        other => other,
    };
    let start = Instant::now();
    let mut readers = Vec::new();
    for mut socket in sockets {
        readers.push(tokio::spawn(async move {
            let mut got = 0_usize;
            let mut last = Instant::now();
            while got < events {
                match tokio::time::timeout(Duration::from_secs(3), socket.next()).await {
                    Ok(Some(Ok(Message::Text(_)))) => {
                        got += 1;
                        last = Instant::now();
                    }
                    Ok(Some(Ok(_))) => {}
                    _ => break,
                }
            }
            (got, last)
        }));
    }
    for _ in 0..events {
        server_subs.publish_json(StreamKind::Transactions, payload.clone());
        tokio::task::yield_now().await;
    }
    let mut total = 0;
    let mut finish = start;
    for reader in readers {
        let (got, last) = reader.await.expect("reader");
        total += got;
        finish = finish.max(last);
    }
    let wall = finish - start;
    println!(
        "fanout: subscribers={subs} events={events} delivered={total}/{} ({:.1}%) wall={wall:?} msgs/s={:.0}",
        subs * events,
        100.0 * total as f64 / (subs * events) as f64,
        total as f64 / wall.as_secs_f64(),
    );
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let mode = args.get(1).map(String::as_str).unwrap_or("http").to_owned();
    let method = method_name(args.get(2).map(String::as_str).unwrap_or("ping"));
    let conns: usize = args.get(3).and_then(|v| v.parse().ok()).unwrap_or(16);
    let reqs: usize = args.get(4).and_then(|v| v.parse().ok()).unwrap_or(2000);

    // Server runtime mirrors production listener sizing (runtime.rs).
    let server_rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(4)
        .max_blocking_threads(128)
        .enable_all()
        .build()
        .expect("server runtime");
    let server = RpcServer::new(SynthDispatcher {
        large: large_result(200),
    });
    let subscriptions = server.subscriptions();
    let listener = server_rt
        .block_on(TcpListener::bind("127.0.0.1:0"))
        .expect("bind");
    let addr = listener.local_addr().expect("addr");
    server_rt.spawn(async move {
        server.serve(listener).await.expect("serve");
    });

    let client_rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(4)
        .enable_all()
        .build()
        .expect("client runtime");
    client_rt.block_on(async move {
        if mode == "fanout" {
            fanout(addr, subscriptions, conns, reqs).await;
            return;
        }
        // Warm up.
        let _ = match mode.as_str() {
            "ws" => ws_client(addr, method, 200).await,
            _ => http_client(addr, method, 200).await,
        };
        let start = Instant::now();
        let mut tasks = Vec::new();
        for _ in 0..conns {
            let mode = mode.clone();
            tasks.push(tokio::spawn(async move {
                match mode.as_str() {
                    "ws" => ws_client(addr, method, reqs).await,
                    _ => http_client(addr, method, reqs).await,
                }
            }));
        }
        let mut all = Vec::with_capacity(conns * reqs);
        let mut bytes = 0;
        for task in tasks {
            let (lat, len) = task.await.expect("client task");
            all.extend(lat);
            bytes = len;
        }
        report(
            &format!("{mode}/{method} conns={conns}"),
            all,
            start.elapsed(),
            bytes,
        );
    });
}
