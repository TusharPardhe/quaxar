use indicatif::{ProgressBar, ProgressStyle};

pub fn run(url: &str) -> bool {
    let sp = ProgressBar::new_spinner();
    sp.set_style(
        ProgressStyle::default_spinner()
            .template("{spinner} {msg}")
            .unwrap(),
    );
    sp.set_message("Fetching fee info...");
    sp.enable_steady_tick(std::time::Duration::from_millis(80));

    let result = super::rpc_call(url, "fee", serde_json::json!({}));
    sp.finish_and_clear();

    match result {
        Ok(r) => {
            super::section_header("Fee Information");
            println!();
            let drops = &r["drops"];
            super::kv(
                "Base Fee",
                &format!("{} drops", drops["minimum_fee"].as_str().unwrap_or("?")),
            );
            super::kv(
                "Open Ledger",
                &format!("{} drops", drops["open_ledger_fee"].as_str().unwrap_or("?")),
            );
            super::kv(
                "Median",
                &format!("{} drops", drops["median_fee"].as_str().unwrap_or("?")),
            );
            println!();
            super::section_separator();
            println!();
            let queue = r["current_queue_size"].as_u64().unwrap_or(0);
            let max = r["expected_ledger_size"].as_u64().unwrap_or(0);
            super::kv(
                "Queue",
                &format!(
                    "{} / {}",
                    super::format_number(queue),
                    super::format_number(max)
                ),
            );
            true
        }
        Err(e) => {
            super::print_error(&e);
            false
        }
    }
}
