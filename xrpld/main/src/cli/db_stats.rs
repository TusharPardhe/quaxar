use super::rpc_call;

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

    super::kv(
        "Write Load",
        &result["write_load"]
            .as_u64()
            .map(|v| super::format_number(v))
            .unwrap_or_else(|| "—".to_string()),
    );
    super::kv(
        "Cache Hit Rate",
        &result["AL_hit_rate"]
            .as_f64()
            .map(|v| format!("{v:.1}%"))
            .unwrap_or_else(|| "—".to_string()),
    );
    super::kv(
        "Tree Nodes",
        &super::format_number(result["treenode_cache_size"].as_u64().unwrap_or(0)),
    );
    super::kv(
        "Full Below",
        &super::format_number(result["fullbelow_size"].as_u64().unwrap_or(0)),
    );
    super::kv(
        "Ledger Hit Rate",
        &result["ledger_hit_rate"]
            .as_f64()
            .map(|v| format!("{v:.1}%"))
            .unwrap_or_else(|| "—".to_string()),
    );
    super::kv("Uptime", result["uptime"].as_str().unwrap_or("—"));

    // Show hint if everything is idle
    let tree = result["treenode_cache_size"].as_u64().unwrap_or(0);
    let writes = result["write_load"].as_u64();
    if tree == 0 && writes.is_none() {
        println!();
        println!(
            "    {}",
            console::Style::new()
                .dim()
                .apply_to("Cache is cold — data flows directly to NuDB on disk."),
        );
    }
}
