use indicatif::{ProgressBar, ProgressStyle};

pub fn run(url: &str, seq: Option<u64>) -> bool {
    let sp = ProgressBar::new_spinner();
    sp.set_style(
        ProgressStyle::default_spinner()
            .template("{spinner} {msg}")
            .unwrap(),
    );
    sp.set_message("Fetching ledger...");
    sp.enable_steady_tick(std::time::Duration::from_millis(80));

    let ledger_index = seq
        .map(serde_json::Value::from)
        .unwrap_or(serde_json::json!("validated"));
    let result = super::rpc_call(
        url,
        "ledger",
        serde_json::json!({"ledger_index": ledger_index}),
    );
    sp.finish_and_clear();

    match result {
        Ok(r) => {
            let l = &r["ledger"];
            super::section_header("Ledger Info");
            println!();
            super::kv("Sequence", l["ledger_index"].as_str().unwrap_or("?"));
            super::kv("Hash", l["ledger_hash"].as_str().unwrap_or("?"));
            super::kv("Parent", l["parent_hash"].as_str().unwrap_or("?"));
            super::kv("Close Time", l["close_time_human"].as_str().unwrap_or("?"));
            super::kv(
                "Tx Count",
                &super::format_number(l["transaction_count"].as_u64().unwrap_or(0)),
            );
            super::kv("Total Coins", l["total_coins"].as_str().unwrap_or("?"));
            true
        }
        Err(e) => {
            super::print_error(&e);
            false
        }
    }
}
