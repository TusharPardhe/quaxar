//! Closed-loop load generator for any XRPL JSON-RPC endpoint (HTTP or WS),
//! used to compare quaxar and rippled with an identical client.
//!
//! cargo run --release -p server --example rpc_load -- \
//!     <http://host:port/ | ws://host:port/> <request-json> <conns> <reqs-per-conn>
//!
//! HTTP bodies use the JSON-RPC form ({"method":..,"params":[{..}]}); WS
//! messages the command form ({"command":..,..}). Pass the request in the
//! command form; it is converted for HTTP. Keep-alive HTTP/1.1, one
//! outstanding request per connection. Prints one JSON line of results.

use std::time::{Duration, Instant};

use futures::{SinkExt, StreamExt};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::Message;

fn http_body(command: &serde_json::Value) -> String {
    let mut params = command.clone();
    let method = params
        .as_object_mut()
        .and_then(|object| object.remove("command"))
        .and_then(|value| value.as_str().map(str::to_owned))
        .expect("request needs a command");
    serde_json::json!({"method": method, "params": [params]}).to_string()
}

async fn http_client(host: String, body: String, reqs: usize) -> (Vec<Duration>, usize, bool) {
    let mut stream = TcpStream::connect(&host).await.expect("connect");
    stream.set_nodelay(true).expect("nodelay");
    let request = format!(
        "POST / HTTP/1.1\r\nHost: {host}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    );
    let mut buf = Vec::with_capacity(1 << 20);
    let mut chunk = vec![0_u8; 1 << 16];
    let mut lat = Vec::with_capacity(reqs);
    let mut size = 0;
    let mut ok = true;
    for _ in 0..reqs {
        let start = Instant::now();
        stream.write_all(request.as_bytes()).await.expect("write");
        buf.clear();
        let header_end = loop {
            let read = stream.read(&mut chunk).await.expect("read");
            assert!(read > 0, "server closed connection");
            buf.extend_from_slice(&chunk[..read]);
            if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                break pos + 4;
            }
        };
        let headers = String::from_utf8_lossy(&buf[..header_end]).to_ascii_lowercase();
        let length: usize = headers
            .lines()
            .find_map(|line| line.strip_prefix("content-length:"))
            .map(|v| v.trim().parse().expect("content-length"))
            .expect("content-length header (chunked not supported)");
        while buf.len() < header_end + length {
            let read = stream.read(&mut chunk).await.expect("read body");
            assert!(read > 0, "server closed mid-body");
            buf.extend_from_slice(&chunk[..read]);
        }
        lat.push(start.elapsed());
        size = length;
        let body = &buf[header_end..header_end + length];
        ok &= !body.windows(8).any(|w| w == b"\"error\":");
    }
    (lat, size, ok)
}

async fn ws_client(url: String, message: String, reqs: usize) -> (Vec<Duration>, usize, bool) {
    let (mut socket, _) = tokio_tungstenite::connect_async(url)
        .await
        .expect("ws connect");
    let mut lat = Vec::with_capacity(reqs);
    let mut size = 0;
    let mut ok = true;
    for _ in 0..reqs {
        let start = Instant::now();
        socket
            .send(Message::Text(message.clone().into()))
            .await
            .expect("send");
        loop {
            match socket.next().await.expect("open").expect("frame") {
                Message::Text(text) => {
                    size = text.len();
                    ok &= !text.contains("\"error\":");
                    break;
                }
                _ => continue,
            }
        }
        lat.push(start.elapsed());
    }
    (lat, size, ok)
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    assert!(
        args.len() >= 5,
        "usage: rpc_load <url> <request-json> <conns> <reqs>"
    );
    let url = args[1].clone();
    let command: serde_json::Value = serde_json::from_str(&args[2]).expect("request json");
    let conns: usize = args[3].parse().expect("conns");
    let reqs: usize = args[4].parse().expect("reqs");
    let ws = url.starts_with("ws://");
    let host = url
        .trim_start_matches("http://")
        .trim_start_matches("ws://")
        .trim_end_matches('/')
        .to_owned();

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(4)
        .enable_all()
        .build()
        .expect("runtime");
    runtime.block_on(async move {
        // Warm-up connection.
        if ws {
            ws_client(url.clone(), command.to_string(), 50).await;
        } else {
            http_client(host.clone(), http_body(&command), 50).await;
        }
        let start = Instant::now();
        let mut tasks = Vec::new();
        for _ in 0..conns {
            let (url, host) = (url.clone(), host.clone());
            let (message, body) = (command.to_string(), http_body(&command));
            tasks.push(tokio::spawn(async move {
                if ws {
                    ws_client(url, message, reqs).await
                } else {
                    http_client(host, body, reqs).await
                }
            }));
        }
        let mut all = Vec::with_capacity(conns * reqs);
        let mut size = 0;
        let mut ok = true;
        for task in tasks {
            let (lat, len, good) = task.await.expect("client");
            all.extend(lat);
            size = len;
            ok &= good;
        }
        let wall = start.elapsed();
        all.sort_unstable();
        let pct = |p: f64| all[((all.len() as f64 * p) as usize).min(all.len() - 1)].as_secs_f64() * 1e6;
        println!(
            "{{\"n\":{},\"rps\":{:.0},\"p50_us\":{:.1},\"p90_us\":{:.1},\"p99_us\":{:.1},\"p999_us\":{:.1},\"max_us\":{:.1},\"bytes\":{},\"ok\":{}}}",
            all.len(),
            all.len() as f64 / wall.as_secs_f64(),
            pct(0.50),
            pct(0.90),
            pct(0.99),
            pct(0.999),
            all[all.len() - 1].as_secs_f64() * 1e6,
            size,
            ok
        );
    });
}
