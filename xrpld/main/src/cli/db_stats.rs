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

    if let Some(writes) = result["write_load"].as_u64() {
        super::kv("Write Load", &super::format_number(writes));
    }
    if let Some(sz) = result["node_store_size"].as_str() {
        super::kv("Node Store Size", sz);
    }
    if let Some(rate) = result["AL_hit_rate"].as_f64() {
        super::kv("Cache Hit Rate", &format!("{rate:.1}%"));
    }
    if let Some(count) = result["dbKBTotal"].as_u64() {
        super::kv(
            "DB Total",
            &format!("{} MB", super::format_number(count / 1024)),
        );
    }

    println!();
    super::section_separator();
    println!();

    for key in ["SLE_hit_rate", "node_hit_rate", "ledger_hit_rate"] {
        if let Some(v) = result[key].as_f64() {
            super::kv(key, &format!("{v:.1}%"));
        }
    }

    if let Ok(info) = rpc_call(url, "server_info", serde_json::json!({})) {
        if let Some(complete) = info["info"]["complete_ledgers"].as_str() {
            super::kv("Complete Ledgers", complete);
        }
    }
}
