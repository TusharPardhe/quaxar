//! Endpoint benchmark: handler + result serialization for RPC methods that
//! return ledger objects, with tree rendering vs direct (raw) rendering.
//!
//! cargo run --release -p rpc-integration-tests --example endpoint_render [offers] [tickets]
//!
//! Each endpoint's two renderings are first checked byte-identical.

use std::hint::black_box;
use std::time::{Duration, Instant};

use rpc_integration_tests::render_scenario::{build, render, ENDPOINTS};

fn time(iterations: u32, mut op: impl FnMut()) -> Duration {
    for _ in 0..iterations / 10 + 1 {
        op();
    }
    let start = Instant::now();
    for _ in 0..iterations {
        op();
    }
    start.elapsed() / iterations
}

fn main() {
    let args: Vec<usize> = std::env::args()
        .skip(1)
        .filter_map(|a| a.parse().ok())
        .collect();
    let offers = args.first().copied().unwrap_or(200);
    let tickets = args.get(1).copied().unwrap_or(200);
    let scenario = build(offers, tickets);
    println!("scenario: {offers} offers in one book, {tickets} tickets\n");
    println!(
        "{:<16} {:>10} {:>12} {:>12} {:>8}",
        "endpoint", "bytes", "tree", "direct", "speedup"
    );
    for endpoint in ENDPOINTS {
        let tree_bytes = render(&scenario, endpoint, false);
        let raw_bytes = render(&scenario, endpoint, true);
        assert_eq!(tree_bytes, raw_bytes, "{endpoint}: renderings differ");
        let iterations = (2_000_000 / tree_bytes.len().max(1000)) as u32;
        let tree = time(iterations, || {
            black_box(render(&scenario, endpoint, false));
        });
        let direct = time(iterations, || {
            black_box(render(&scenario, endpoint, true));
        });
        println!(
            "{endpoint:<16} {:>10} {:>12?} {:>12?} {:>7.2}x",
            tree_bytes.len(),
            tree,
            direct,
            tree.as_secs_f64() / direct.as_secs_f64()
        );
    }
}
