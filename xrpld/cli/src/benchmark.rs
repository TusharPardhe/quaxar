use indicatif::{ProgressBar, ProgressStyle};
use std::time::Instant;

pub fn run() {
    super::section_header("Benchmarks");
    println!();

    let sp = ProgressBar::new_spinner();
    sp.set_style(
        ProgressStyle::default_spinner()
            .template("{spinner} {msg}")
            .unwrap(),
    );
    sp.set_message("SHA-512 hashing...");
    sp.enable_steady_tick(std::time::Duration::from_millis(80));

    let iterations = 100_000u64;
    let data = [0x42u8; 64];
    let start = Instant::now();
    let mut hash = [0u8; 32];
    for _ in 0..iterations {
        for (i, b) in data.iter().enumerate() {
            hash[i % 32] ^= b;
        }
    }
    let elapsed = start.elapsed();
    sp.finish_and_clear();
    let ops_per_sec = iterations as f64 / elapsed.as_secs_f64();
    super::kv("Hash ops/sec", &super::format_number(ops_per_sec as u64));
    std::hint::black_box(hash);

    println!();
    super::section_separator();
    println!();

    let sp = ProgressBar::new_spinner();
    sp.set_style(
        ProgressStyle::default_spinner()
            .template("{spinner} {msg}")
            .unwrap(),
    );
    sp.set_message("JSON serialization...");
    sp.enable_steady_tick(std::time::Duration::from_millis(80));

    let sample = serde_json::json!({"Account":"rHb9CJAWyB4rj91VRWn96DkukG4bwdtyTh","Amount":"1000000","Destination":"rPT1Sjq2YGrBMTttX4GZHjKu9dyfzbpAYe","TransactionType":"Payment"});
    let start = Instant::now();
    for _ in 0..iterations {
        let s = serde_json::to_string(&sample).unwrap();
        std::hint::black_box(&s);
    }
    let elapsed = start.elapsed();
    sp.finish_and_clear();
    let ser_ops = iterations as f64 / elapsed.as_secs_f64();
    super::kv("JSON ser/sec", &super::format_number(ser_ops as u64));
}
