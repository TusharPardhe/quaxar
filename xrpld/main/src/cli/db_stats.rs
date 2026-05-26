use super::rpc_call;
use std::path::Path;

pub fn run(url: &str) {
    let result = match rpc_call(url, "get_counts", serde_json::json!({})) {
        Ok(r) => r,
        Err(e) => {
            super::print_error(&e);
            return;
        }
    };

    super::section_header("Database Statistics");
    println!();

    // Show NuDB disk usage from config
    if let Some(nudb_path) = find_nudb_path() {
        let (data_size, key_size, log_size) = nudb_file_sizes(&nudb_path);
        let total = data_size + key_size + log_size;
        if total > 0 {
            super::kv("NuDB Data", &format_bytes(data_size));
            super::kv("NuDB Keys", &format_bytes(key_size));
            super::kv("NuDB Total", &format_bytes(total));
            println!();
            super::section_separator();
            println!();
        }
    }

    super::kv(
        "Hist/min",
        &result["historical_perminute"]
            .as_u64()
            .map(|v| super::format_number(v))
            .unwrap_or_else(|| "—".to_string()),
    );
    super::kv("Uptime", result["uptime"].as_str().unwrap_or("—"));
}

fn find_nudb_path() -> Option<String> {
    // Try xrpld.cfg in current dir
    let cfg_path = if Path::new("xrpld.cfg").exists() {
        "xrpld.cfg"
    } else {
        return None;
    };
    let content = std::fs::read_to_string(cfg_path).ok()?;
    let mut in_node_db = false;
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed == "[node_db]" {
            in_node_db = true;
            continue;
        }
        if trimmed.starts_with('[') {
            in_node_db = false;
            continue;
        }
        if in_node_db {
            if let Some(val) = trimmed.strip_prefix("path") {
                if let Some(val) = val.trim().strip_prefix('=') {
                    return Some(val.trim().to_string());
                }
            }
        }
    }
    None
}

fn nudb_file_sizes(base_path: &str) -> (u64, u64, u64) {
    // NuDB stores in subdirectories like xrpldb.0000/
    let base = Path::new(base_path);
    let mut data_total = 0u64;
    let mut key_total = 0u64;
    let mut log_total = 0u64;

    if let Ok(entries) = std::fs::read_dir(base) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                data_total += file_size(&path.join("nudb.dat"));
                key_total += file_size(&path.join("nudb.key"));
                log_total += file_size(&path.join("nudb.log"));
            }
        }
    }
    (data_total, key_total, log_total)
}

fn file_size(path: &Path) -> u64 {
    std::fs::metadata(path).map(|m| m.len()).unwrap_or(0)
}

fn format_bytes(bytes: u64) -> String {
    if bytes >= 1_073_741_824 {
        format!("{:.1} GB", bytes as f64 / 1_073_741_824.0)
    } else if bytes >= 1_048_576 {
        format!("{:.1} MB", bytes as f64 / 1_048_576.0)
    } else if bytes >= 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else {
        format!("{bytes} B")
    }
}
