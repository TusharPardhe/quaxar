use super::rpc_call;
use std::path::Path;

pub fn run(url: &str, conf: Option<&str>) -> bool {
    let result = match rpc_call(url, "get_counts", serde_json::json!({})) {
        Ok(r) => r,
        Err(e) => {
            super::print_error(&e);
            return false;
        }
    };

    super::section_header("Database Statistics");
    println!();

    if let Some(nudb_path) = find_nudb_path(conf) {
        let (data_size, key_size, log_size) = nudb_file_sizes(&nudb_path);
        let total = data_size + key_size + log_size;
        super::kv("Node DB Path", &nudb_path);
        if total > 0 {
            super::kv("NuDB Data", &format_bytes(data_size));
            super::kv("NuDB Keys", &format_bytes(key_size));
            if log_size > 0 {
                super::kv("NuDB Log", &format_bytes(log_size));
            }
            super::kv("NuDB Total", &format_bytes(total));
        } else {
            super::kv("NuDB Total", "0 B");
        }
        println!();
        super::section_separator();
        println!();
    }

    if let Some(kind) = result["node_store"].as_str() {
        super::kv("Node Store", kind);
    }
    kv_json_number_or_string("Earliest Seq", &result["node_db_earliest_seq"]);
    kv_json_number_or_string("Read Queue", &result["read_queue"]);
    kv_json_number_or_string("Read Threads", &result["read_threads_running"]);
    kv_json_number_or_string("Node Writes", &result["node_writes"]);
    kv_json_number_or_string("Node Reads", &result["node_reads_total"]);
    kv_json_number_or_string("Node Hits", &result["node_reads_hit"]);
    kv_json_bytes("Written", &result["node_written_bytes"]);
    kv_json_bytes("Read", &result["node_read_bytes"]);
    kv_hit_rate(
        "Read Hit Rate",
        &result["node_reads_total"],
        &result["node_reads_hit"],
    );
    println!();
    super::section_separator();
    println!();

    super::kv(
        "Hist/min",
        &result["historical_perminute"]
            .as_u64()
            .or_else(|| {
                result["historical_perminute"]
                    .as_i64()
                    .and_then(|v| v.try_into().ok())
            })
            .map(|v| super::format_number(v))
            .unwrap_or_else(|| "—".to_string()),
    );
    kv_json_number_or_string("AL Size", &result["AL_size"]);
    kv_json_number_or_string("Tree Cache", &result["treenode_cache_size"]);
    kv_json_number_or_string("Full Below", &result["fullbelow_size"]);
    super::kv("Uptime", result["uptime"].as_str().unwrap_or("—"));
    true
}

fn find_nudb_path(conf: Option<&str>) -> Option<String> {
    let cfg_path = conf.unwrap_or("xrpld.cfg");
    if !Path::new(cfg_path).exists() {
        return None;
    }
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

fn json_u64(value: &serde_json::Value) -> Option<u64> {
    value
        .as_u64()
        .or_else(|| value.as_i64().and_then(|value| value.try_into().ok()))
        .or_else(|| value.as_str().and_then(|value| value.parse::<u64>().ok()))
}

fn kv_json_number_or_string(label: &str, value: &serde_json::Value) {
    let rendered = json_u64(value)
        .map(super::format_number)
        .or_else(|| value.as_str().map(ToOwned::to_owned))
        .unwrap_or_else(|| "—".to_owned());
    super::kv(label, &rendered);
}

fn kv_json_bytes(label: &str, value: &serde_json::Value) {
    let rendered = json_u64(value)
        .map(format_bytes)
        .unwrap_or_else(|| "—".to_owned());
    super::kv(label, &rendered);
}

fn kv_hit_rate(label: &str, total: &serde_json::Value, hits: &serde_json::Value) {
    let Some(total) = json_u64(total) else {
        super::kv(label, "—");
        return;
    };
    let hits = json_u64(hits).unwrap_or(0);
    if total == 0 {
        super::kv(label, "—");
        return;
    }
    super::kv(
        label,
        &format!("{:.1}%", hits as f64 * 100.0 / total as f64),
    );
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

#[cfg(test)]
mod tests {
    use super::{format_bytes, json_u64};

    #[test]
    fn json_u64_accepts_numeric_and_string_count_fields() {
        assert_eq!(json_u64(&serde_json::json!(12)), Some(12));
        assert_eq!(json_u64(&serde_json::json!("34")), Some(34));
        assert_eq!(json_u64(&serde_json::json!(-1)), None);
        assert_eq!(json_u64(&serde_json::json!("bad")), None);
    }

    #[test]
    fn format_bytes_uses_human_readable_units() {
        assert_eq!(format_bytes(0), "0 B");
        assert_eq!(format_bytes(1024), "1.0 KB");
        assert_eq!(format_bytes(1_048_576), "1.0 MB");
    }
}
