use console::Style;
use indicatif::{ProgressBar, ProgressStyle};

pub fn run(url: &str) {
    let sp = ProgressBar::new_spinner();
    sp.set_style(
        ProgressStyle::default_spinner()
            .template("{spinner} {msg}")
            .unwrap(),
    );
    sp.set_message("Fetching peers...");
    sp.enable_steady_tick(std::time::Duration::from_millis(80));

    let result = super::rpc_call(url, "peers", serde_json::json!({}));
    sp.finish_and_clear();

    let dim = Style::new().dim();

    match result {
        Ok(r) => {
            let peers = r["peers"].as_array();
            let count = peers.map(|p| p.len()).unwrap_or(0);
            println!(
                "    {} {}",
                Style::new().green().apply_to("●"),
                Style::new()
                    .bold()
                    .apply_to(format!("{} peers connected", count))
            );
            println!();
            println!(
                "    {}",
                dim.apply_to(format!(
                    "{:<3} {:<22} {:>7} {:>10} {:<16} {:>8}",
                    "#", "Address", "Latency", "Ledger", "Version", "Uptime"
                ))
            );
            println!(
                "    {}",
                dim.apply_to("───────────────────────────────────────────────────────────────────")
            );
            if let Some(list) = peers {
                for (i, p) in list.iter().enumerate() {
                    let addr = p["address"].as_str().unwrap_or("?");
                    let latency = p["latency"].as_u64().unwrap_or(0);
                    let ledger = p["ledger"].as_str().unwrap_or("?");
                    let version = p["version"].as_str().unwrap_or("?");
                    let uptime = p["uptime"].as_u64().unwrap_or(0);
                    let short_addr = if addr.len() > 21 { &addr[..21] } else { addr };
                    let short_ver = if version.len() > 15 {
                        &version[..15]
                    } else {
                        version
                    };
                    let short_ledger = if ledger.len() > 9 {
                        &ledger[..9]
                    } else {
                        ledger
                    };
                    println!(
                        "    {:<3} {:<22} {:>5}ms {:>10} {:<16} {:>6}s",
                        i + 1,
                        short_addr,
                        latency,
                        short_ledger,
                        short_ver,
                        uptime
                    );
                }
            }
        }
        Err(e) => super::print_error(&e),
    }
}
