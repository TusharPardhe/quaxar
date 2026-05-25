use super::rpc_call;
use console::Style;

pub fn run(url: &str, level: Option<&str>) {
    match level {
        Some(new_level) => {
            let result =
                match rpc_call(url, "log_level", serde_json::json!({"severity": new_level})) {
                    Ok(r) => r,
                    Err(e) => {
                        super::print_error(&e);
                        return;
                    }
                };
            if result["status"].as_str() == Some("success") {
                println!(
                    "    {} Log level set to: {}",
                    Style::new().green().apply_to("●"),
                    new_level
                );
            } else {
                super::print_error(&format!("Failed: {}", result));
            }
        }
        None => {
            let result = match rpc_call(url, "log_level", serde_json::json!({})) {
                Ok(r) => r,
                Err(e) => {
                    super::print_error(&e);
                    return;
                }
            };
            super::section_header("Current Log Levels");
            println!();
            if let Some(levels) = result["levels"].as_object() {
                for (module, lvl) in levels {
                    super::kv(module, lvl.as_str().unwrap_or("?"));
                }
            } else {
                super::kv("Base", result["severity"].as_str().unwrap_or("unknown"));
            }
        }
    }
}
