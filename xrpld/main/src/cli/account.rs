use indicatif::{ProgressBar, ProgressStyle};

pub fn run(url: &str, address: &str) {
    let sp = ProgressBar::new_spinner();
    sp.set_style(
        ProgressStyle::default_spinner()
            .template("{spinner} {msg}")
            .unwrap(),
    );
    sp.set_message("Fetching account...");
    sp.enable_steady_tick(std::time::Duration::from_millis(80));

    let result = super::rpc_call(url, "account_info", serde_json::json!({"account": address}));
    sp.finish_and_clear();

    match result {
        Ok(r) => {
            let info = &r["account_data"];
            let balance_drops: u64 = info["Balance"].as_str().unwrap_or("0").parse().unwrap_or(0);
            let balance_xrp = balance_drops as f64 / 1_000_000.0;
            super::section_header("Account Info");
            println!();
            super::kv("Account", address);
            super::kv("Balance", &format!("{:.6} XRP", balance_xrp));
            super::kv(
                "Sequence",
                &super::format_number(info["Sequence"].as_u64().unwrap_or(0)),
            );
            super::kv(
                "Owner Count",
                &super::format_number(info["OwnerCount"].as_u64().unwrap_or(0)),
            );
            super::kv("Flags", &format!("{}", info["Flags"].as_u64().unwrap_or(0)));
        }
        Err(e) => super::print_error(&e),
    }
}
