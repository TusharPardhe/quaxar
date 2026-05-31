use console::Style;
use indicatif::{ProgressBar, ProgressStyle};

/// Returns true if node is reachable (healthy or syncing), false if down.
pub fn run(url: &str) -> bool {
    let sp = ProgressBar::new_spinner();
    sp.set_style(
        ProgressStyle::default_spinner()
            .template("{spinner} {msg}")
            .unwrap(),
    );
    sp.set_message("Checking health...");
    sp.enable_steady_tick(std::time::Duration::from_millis(80));

    let result = super::rpc_call(url, "server_info", serde_json::json!({}));
    sp.finish_and_clear();

    match result {
        Ok(r) => {
            let state = r["info"]["server_state"].as_str().unwrap_or("unknown");
            match state {
                "full" | "proposing" | "validating" => {
                    println!(
                        "    {} {}  {}",
                        Style::new().green().apply_to("●"),
                        Style::new().green().bold().apply_to("Healthy"),
                        Style::new().dim().apply_to("fully synced"),
                    );
                }
                "tracking" | "syncing" | "connected" => {
                    println!(
                        "    {} {}  {}",
                        Style::new().yellow().apply_to("◐"),
                        Style::new().yellow().bold().apply_to("Syncing"),
                        Style::new().dim().apply_to(format!("state: {state}")),
                    );
                }
                _ => {
                    println!(
                        "    {} {}  {}",
                        Style::new().red().apply_to("●"),
                        Style::new().red().bold().apply_to("Degraded"),
                        Style::new().dim().apply_to(format!("state: {state}")),
                    );
                }
            }
            true
        }
        Err(e) => {
            eprintln!(
                "    {} {}  {}",
                Style::new().red().apply_to("●"),
                Style::new().red().bold().apply_to("Down"),
                Style::new().dim().apply_to(e),
            );
            false
        }
    }
}
