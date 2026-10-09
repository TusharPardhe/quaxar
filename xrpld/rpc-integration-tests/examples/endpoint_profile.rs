//! Spin one endpoint (raw rendering) for profiling.
//! cargo run --release -p rpc-integration-tests --example endpoint_profile <endpoint> [seconds]
use rpc_integration_tests::render_scenario::{build, render};
fn main() {
    let endpoint = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "book_offers".into());
    let seconds: u64 = std::env::args()
        .nth(2)
        .and_then(|s| s.parse().ok())
        .unwrap_or(8);
    let scenario = build(200, 200);
    let start = std::time::Instant::now();
    let mut n = 0u64;
    while start.elapsed().as_secs() < seconds {
        std::hint::black_box(render(&scenario, &endpoint, true));
        n += 1;
    }
    eprintln!("{endpoint}: {n} calls");
}
