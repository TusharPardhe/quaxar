use console::Style;
use indicatif::{ProgressBar, ProgressStyle};

pub fn run(url: &str) {
    let sp = ProgressBar::new_spinner();
    sp.set_style(
        ProgressStyle::default_spinner()
            .template("{spinner} {msg}")
            .unwrap(),
    );
    sp.set_message("Fetching amendments...");
    sp.enable_steady_tick(std::time::Duration::from_millis(80));

    let result = super::rpc_call(url, "feature", serde_json::json!({}));
    sp.finish_and_clear();

    let dim = Style::new().dim();
    let green = Style::new().green();
    let red = Style::new().red();

    match result {
        Ok(r) => {
            let features = r["features"].as_object();
            if let Some(map) = features {
                let mut enabled = 0u32;
                let mut supported = 0u32;
                for (_hash, info) in map {
                    if info["enabled"].as_bool() == Some(true) {
                        enabled += 1;
                    }
                    if info["supported"].as_bool() == Some(true) {
                        supported += 1;
                    }
                }
                super::section_header("Amendments");
                println!();
                super::kv("Enabled", &super::format_number(enabled as u64));
                super::kv("Supported", &super::format_number(supported as u64));
                super::kv("Total", &super::format_number(map.len() as u64));
                println!();
                super::section_separator();
                println!();
                for (hash, info) in map.iter().take(15) {
                    let name = info["name"].as_str().unwrap_or(&hash[..8]);
                    let dot = if info["enabled"].as_bool() == Some(true) {
                        green.apply_to("●")
                    } else {
                        red.apply_to("●")
                    };
                    println!("    {} {}", dot, name);
                }
                if map.len() > 15 {
                    println!(
                        "    {}",
                        dim.apply_to(format!("... and {} more", map.len() - 15))
                    );
                }
            } else {
                println!("    {}", dim.apply_to("No amendment data available"));
            }
        }
        Err(e) => super::print_error(&e),
    }
}
