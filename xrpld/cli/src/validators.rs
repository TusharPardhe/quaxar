use console::Style;
use indicatif::{ProgressBar, ProgressStyle};

pub fn run(url: &str) -> bool {
    let sp = ProgressBar::new_spinner();
    sp.set_style(
        ProgressStyle::default_spinner()
            .template("{spinner} {msg}")
            .unwrap(),
    );
    sp.set_message("Fetching validators...");
    sp.enable_steady_tick(std::time::Duration::from_millis(80));

    let result = super::rpc_call(url, "validators", serde_json::json!({}));
    sp.finish_and_clear();

    match result {
        Ok(r) => {
            let validators = r["trusted_validators"]
                .as_array()
                .or_else(|| r["validators"].as_array());
            if let Some(list) = validators {
                super::section_header(&format!("Trusted Validators ({})", list.len()));
                println!();
                for v in list.iter().take(20) {
                    let key = v["validation_public_key"]
                        .as_str()
                        .unwrap_or(v["pubkey_validator"].as_str().unwrap_or("?"));
                    println!("    {}", &key[..key.len().min(52)]);
                }
                if list.len() > 20 {
                    println!(
                        "    {}",
                        Style::new()
                            .dim()
                            .apply_to(format!("... and {} more", list.len() - 20))
                    );
                }
            } else {
                println!(
                    "    {}",
                    Style::new().dim().apply_to("No validator data available")
                );
            }
            true
        }
        Err(e) => {
            super::print_error(&e);
            false
        }
    }
}
