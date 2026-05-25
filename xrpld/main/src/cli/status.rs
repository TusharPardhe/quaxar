use console::Style;
use indicatif::{ProgressBar, ProgressStyle};

pub fn run(url: &str) {
    let sp = ProgressBar::new_spinner();
    sp.set_style(
        ProgressStyle::default_spinner()
            .template("{spinner} {msg}")
            .unwrap(),
    );
    sp.set_message("Connecting...");
    sp.enable_steady_tick(std::time::Duration::from_millis(80));

    let result = super::rpc_call(url, "server_info", serde_json::json!({}));
    sp.finish_and_clear();

    match result {
        Ok(r) => {
            let info = &r["info"];
            let state = info["server_state"].as_str().unwrap_or("unknown");
            let peers = info["peers"].as_u64().unwrap_or(0);
            let (dot, label) = match state {
                "full" | "proposing" | "validating" => {
                    (Style::new().green().apply_to("●"), "Online")
                }
                "syncing" | "tracking" | "connected" => {
                    (Style::new().yellow().apply_to("●"), "Syncing")
                }
                _ => (Style::new().red().apply_to("●"), "Offline"),
            };
            let dim = Style::new().dim();
            println!(
                "    {} {:<36}{}",
                dot,
                Style::new().bold().apply_to(label),
                dim.apply_to(format!("{} peers", peers))
            );
            println!();
            super::kv("Server State", state);
            let seq = info["validated_ledger"]["seq"].as_u64().unwrap_or(0);
            super::kv("Validated", &super::format_number(seq));
            super::kv("Complete", info["complete_ledgers"].as_str().unwrap_or("?"));
            let uptime = info["uptime"].as_u64().unwrap_or(0);
            let uptime_str = if uptime >= 3600 {
                format!("{}h {}m", uptime / 3600, (uptime % 3600) / 60)
            } else {
                format!("{}m {}s", uptime / 60, uptime % 60)
            };
            super::kv("Uptime", &uptime_str);
            super::kv(
                "Network",
                &format!("id: {}", info["network_id"].as_u64().unwrap_or(0)),
            );
            println!();
            super::section_separator();
            println!();
            let base_fee = info["validated_ledger"]["base_fee_xrp"]
                .as_f64()
                .map(|f| (f * 1_000_000.0) as u64)
                .unwrap_or(10);
            super::kv("Fee", &format!("{} drops", super::format_number(base_fee)));
            super::kv(
                "Queue",
                &format!("{}", info["txn_queue_size"].as_u64().unwrap_or(0)),
            );
            super::kv("Node Size", info["node_size"].as_str().unwrap_or("?"));
        }
        Err(e) => {
            eprintln!(
                "    {} Offline  {}",
                Style::new().red().apply_to("●"),
                Style::new().dim().apply_to(e)
            );
        }
    }
}
