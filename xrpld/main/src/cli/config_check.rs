use std::path::Path;

use basics::basic_config::{BasicConfig, IniFileSections};

use crate::{LEDGER_FETCH_LIMIT_OVERRIDE_MAX, LEDGER_FETCH_LIMIT_OVERRIDE_MIN};

pub fn run(conf_path: Option<&str>) {
    let path = conf_path.unwrap_or("/etc/xrpld/xrpld.cfg");
    println!("Checking config: {path}");
    println!("───────────────────────────────────");

    // Check file exists
    if !Path::new(path).exists() {
        eprintln!("  ❌ Config file not found: {path}");
        return;
    }
    println!("  ✅ File exists");

    // Parse config
    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("  ❌ Cannot read: {e}");
            return;
        }
    };
    println!("  ✅ File readable ({} bytes)", content.len());

    // Check required sections
    let required = ["[server]", "[node_db]"];
    let mut all_ok = true;
    let mut has_errors = false;
    for section in required {
        if content.contains(section) {
            println!("  ✅ {section}");
        } else {
            println!("  ❌ {section} — missing");
            all_ok = false;
            has_errors = true;
        }
    }

    if content.contains("[validators_file]")
        || (content.contains("[validator_list_sites]") && content.contains("[validator_list_keys]"))
    {
        println!("  ✅ validator list configured");
    } else {
        println!("  ⚠️  validator list not configured");
        all_ok = false;
    }

    for optional in ["[ips]", "[ips_fixed]"] {
        if content.contains(optional) {
            println!("  ✅ {optional}");
        }
    }

    match validate_ledger_fetch_limit(&content) {
        Ok(Some(limit)) => println!("  ✅ [ledger_acquisition] ledger_fetch_limit = {limit}"),
        Ok(None) => {}
        Err(error) => {
            println!("  ❌ {error}");
            all_ok = false;
            has_errors = true;
        }
    }

    // Check ports
    for port_section in ["port_rpc_admin_local", "port_ws_public"] {
        if content.contains(port_section) {
            println!("  ✅ [{port_section}]");
        }
    }

    // Check node_db path
    if let Some(line) = content.lines().find(|l| l.starts_with("path")) {
        let db_path = line.split('=').nth(1).unwrap_or("").trim();
        if !db_path.is_empty() {
            if Path::new(db_path).exists() {
                println!("  ✅ DB path exists: {db_path}");
            } else {
                println!("  ⚠️  DB path does not exist (will be created): {db_path}");
            }
        }
    }

    println!("───────────────────────────────────");
    if all_ok {
        println!("✅ Config looks good");
    } else if has_errors {
        println!("❌ Config has errors — node may not start");
    } else {
        println!("⚠️  Config has warnings — node may still start");
    }
}

fn validate_ledger_fetch_limit(content: &str) -> Result<Option<usize>, String> {
    let config = parse_basic_config_text(content);
    if !config.exists("ledger_acquisition") {
        return Ok(None);
    }

    let Some(limit) = config
        .section("ledger_acquisition")
        .get::<usize>("ledger_fetch_limit")
        .map_err(|_| "Configured ledger_acquisition.ledger_fetch_limit is invalid".to_owned())?
    else {
        return Ok(None);
    };

    if !(LEDGER_FETCH_LIMIT_OVERRIDE_MIN..=LEDGER_FETCH_LIMIT_OVERRIDE_MAX).contains(&limit) {
        return Err(format!(
            "Configured ledger_acquisition.ledger_fetch_limit must be between {LEDGER_FETCH_LIMIT_OVERRIDE_MIN} and {LEDGER_FETCH_LIMIT_OVERRIDE_MAX}"
        ));
    }

    Ok(Some(limit))
}

fn parse_basic_config_text(text: &str) -> BasicConfig {
    let mut sections = IniFileSections::new();
    let mut current_section = String::new();

    for raw_line in text.lines() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        if line.starts_with('[') && line.ends_with(']') {
            current_section = line[1..line.len() - 1].trim().to_owned();
            let _ = sections.entry(current_section.clone()).or_default();
            continue;
        }

        sections
            .entry(current_section.clone())
            .or_default()
            .push(raw_line.to_owned());
    }

    let mut config = BasicConfig::new();
    config.build(&sections);
    config
}

#[cfg(test)]
mod tests {
    use super::validate_ledger_fetch_limit;

    #[test]
    fn ledger_fetch_limit_check_rejects_values_above_supported_max() {
        let error = validate_ledger_fetch_limit("[ledger_acquisition]\nledger_fetch_limit = 9\n")
            .expect_err("limit above huge profile should be rejected");

        assert!(error.contains("between 1 and 8"));
    }

    #[test]
    fn ledger_fetch_limit_check_accepts_huge_profile_limit() {
        assert_eq!(
            validate_ledger_fetch_limit("[ledger_acquisition]\nledger_fetch_limit = 8\n")
                .expect("limit should parse"),
            Some(8)
        );
    }
}
