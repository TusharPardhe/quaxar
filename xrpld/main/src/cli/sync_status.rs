use console::Style;
use indicatif::{ProgressBar, ProgressStyle};

pub fn run(url: &str) {
    let sp = ProgressBar::new_spinner();
    sp.set_style(
        ProgressStyle::default_spinner()
            .template("{spinner} {msg}")
            .unwrap(),
    );
    sp.set_message("Checking sync...");
    sp.enable_steady_tick(std::time::Duration::from_millis(80));

    let result = super::rpc_call(url, "server_info", serde_json::json!({}));
    sp.finish_and_clear();

    match result {
        Ok(r) => {
            let info = &r["info"];
            let state = info["server_state"].as_str().unwrap_or("unknown");
            if matches!(state, "full" | "proposing" | "validating") {
                println!(
                    "    {} Synced  {}",
                    Style::new().green().apply_to("●"),
                    Style::new().dim().apply_to(state)
                );
                return;
            }
            println!(
                "    {} Syncing  {}",
                Style::new().yellow().apply_to("●"),
                Style::new().dim().apply_to(state)
            );
            println!();
            let complete = info["complete_ledgers"].as_str().unwrap_or("empty");
            super::kv("State", state);
            super::kv("Ledgers", complete);
            if let Some(fetch) = info["fetch_pack"].as_u64() {
                super::kv("Fetching", &super::format_number(fetch));
            }
            if let Some(node_size) = info["node_size"].as_str() {
                super::kv("Node Size", node_size);
            }
        }
        Err(e) => super::print_error(&e),
    }
}
