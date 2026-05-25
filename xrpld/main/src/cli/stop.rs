use console::Style;

pub fn run(url: &str) {
    let result = super::rpc_call(url, "stop", serde_json::json!({}));
    match result {
        Ok(_) => println!(
            "    {} Shutdown signal sent",
            Style::new().green().apply_to("●")
        ),
        Err(e) => super::print_error(&e),
    }
}
