use console::Style;
use indicatif::{ProgressBar, ProgressStyle};

/// Returns true if healthy, false if not. Never calls process::exit.
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
            let healthy = matches!(state, "full" | "proposing" | "validating" | "tracking");
            if healthy {
                println!(
                    "    {} healthy  {}",
                    Style::new().green().apply_to("●"),
                    Style::new().dim().apply_to(state)
                );
            } else {
                println!(
                    "    {} unhealthy  {}",
                    Style::new().red().apply_to("●"),
                    Style::new().dim().apply_to(state)
                );
            }
            healthy
        }
        Err(e) => {
            eprintln!(
                "    {} unhealthy  {}",
                Style::new().red().apply_to("●"),
                Style::new().dim().apply_to(e)
            );
            false
        }
    }
}
