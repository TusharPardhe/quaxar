use console::Style;

pub fn parse_params(params: Option<&str>) -> Result<Vec<serde_json::Value>, String> {
    let Some(params) = params else {
        return Ok(vec![serde_json::json!({})]);
    };
    let parsed: serde_json::Value =
        serde_json::from_str(params).map_err(|e| format!("Invalid JSON params: {e}"))?;
    Ok(match parsed {
        serde_json::Value::Array(values) => values,
        value => vec![value],
    })
}

pub fn run(url: &str, method: &str, params: Option<&str>, raw: bool) -> bool {
    let params = match parse_params(params) {
        Ok(params) => params,
        Err(error) => {
            super::print_error(&error);
            return false;
        }
    };

    match super::rpc_call_params(url, method, params) {
        Ok(result) => {
            if raw {
                println!(
                    "{}",
                    serde_json::to_string(&result).unwrap_or_else(|_| result.to_string())
                );
            } else {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&result).unwrap_or_else(|_| result.to_string())
                );
            }
            true
        }
        Err(error) => {
            super::print_error(&error);
            false
        }
    }
}

pub fn run_no_params(url: &str, method: &str) -> bool {
    run(url, method, None, false)
}

pub fn run_can_delete(url: &str, value: Option<&str>) -> bool {
    let params = value
        .map(|value| serde_json::json!({ "can_delete": value }))
        .unwrap_or_else(|| serde_json::json!({}));
    match super::rpc_call(url, "can_delete", params) {
        Ok(result) => {
            super::section_header("Can Delete");
            println!();
            super::kv(
                "Ledger",
                &result["can_delete"]
                    .as_u64()
                    .map(super::format_number)
                    .or_else(|| result["can_delete"].as_str().map(ToOwned::to_owned))
                    .unwrap_or_else(|| "—".to_owned()),
            );
            true
        }
        Err(error) => {
            super::print_error(&error);
            false
        }
    }
}

pub fn run_logrotate(url: &str) -> bool {
    match super::rpc_call(url, "logrotate", serde_json::json!({})) {
        Ok(_) => {
            println!(
                "    {} Log rotation requested",
                Style::new().green().apply_to("●")
            );
            true
        }
        Err(error) => {
            super::print_error(&error);
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::parse_params;

    #[test]
    fn parse_params_defaults_to_single_empty_object() {
        assert_eq!(
            parse_params(None).expect("params"),
            vec![serde_json::json!({})]
        );
    }

    #[test]
    fn parse_params_accepts_object_or_array() {
        assert_eq!(
            parse_params(Some(r#"{"ledger_index":"validated"}"#)).expect("object"),
            vec![serde_json::json!({"ledger_index":"validated"})]
        );
        assert_eq!(
            parse_params(Some(r#"[{"a":1},{"b":2}]"#)).expect("array"),
            vec![serde_json::json!({"a":1}), serde_json::json!({"b":2})]
        );
    }
}
