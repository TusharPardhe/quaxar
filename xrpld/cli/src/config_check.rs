use std::path::Path;
use std::{collections::HashSet, net::IpAddr};

use basics::basic_config::{BasicConfig, IniFileSections};
use ipnet::IpNet;

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

    let report = validate_config_content(&content);
    for ok_message in &report.ok {
        println!("  ✅ {ok_message}");
    }
    for warning in &report.warnings {
        println!("  ⚠️  {warning}");
        all_ok = false;
    }
    for error in &report.errors {
        println!("  ❌ {error}");
        all_ok = false;
        has_errors = true;
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

#[derive(Debug, Default)]
struct ConfigValidationReport {
    ok: Vec<String>,
    warnings: Vec<String>,
    errors: Vec<String>,
}

fn validate_config_content(content: &str) -> ConfigValidationReport {
    let config = parse_basic_config_text(content);
    let mut report = ConfigValidationReport::default();

    validate_server_ports(&config, &mut report);
    validate_node_size(&config, &mut report);
    validate_ledger_acquisition(&config, &mut report);
    validate_node_db_and_history(&config, &mut report);
    validate_network_id(&config, &mut report);
    validate_overlay(&config, &mut report);
    validate_crawl(&config, &mut report);
    validate_vl(&config, &mut report);
    validate_reduce_relay(&config, &mut report);
    validate_ssl_verify(&config, &mut report);

    report
}

#[cfg(test)]
fn validate_ledger_fetch_limit(content: &str) -> Result<Option<usize>, String> {
    let config = parse_basic_config_text(content);
    ledger_fetch_limit(&config)
}

fn ledger_fetch_limit(config: &BasicConfig) -> Result<Option<usize>, String> {
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

fn validate_server_ports(config: &BasicConfig, report: &mut ConfigValidationReport) {
    if !config.exists("server") {
        return;
    }

    let mut seen_ports = HashSet::new();
    let mut peer_protocols = 0usize;
    for name in config.section("server").values() {
        if !config.exists(name) {
            report.errors.push(format!("Missing section: [{name}]"));
            continue;
        }

        let section = config.section(name);
        match required_string(section, "port") {
            Ok(raw) => match parse_u16_range(&raw, 1, u16::MAX) {
                Ok(port) => {
                    if !seen_ports.insert(port) {
                        report
                            .errors
                            .push(format!("Duplicate listening port: {port}"));
                    }
                }
                Err(error) => report.errors.push(format!("[{name}] port {error}")),
            },
            Err(error) => report.errors.push(format!("[{name}] {error}")),
        }

        match required_string(section, "ip") {
            Ok(raw) => {
                if raw.parse::<IpAddr>().is_err() {
                    report.errors.push(format!("[{name}] ip is invalid: {raw}"));
                }
            }
            Err(error) => report.errors.push(format!("[{name}] {error}")),
        }

        match required_string(section, "protocol") {
            Ok(raw) => {
                let mut protocols = HashSet::new();
                for protocol in raw
                    .split(',')
                    .map(|value| value.trim().to_ascii_lowercase())
                {
                    if protocol.is_empty() {
                        continue;
                    }
                    if !matches!(protocol.as_str(), "http" | "ws" | "peer") {
                        report
                            .errors
                            .push(format!("[{name}] unsupported protocol: {protocol}"));
                    }
                    if protocol == "peer" {
                        peer_protocols += 1;
                    }
                    protocols.insert(protocol);
                }
                if protocols.is_empty() {
                    report
                        .errors
                        .push(format!("[{name}] protocol cannot be empty"));
                }
            }
            Err(error) => report.errors.push(format!("[{name}] {error}")),
        }

        for field in ["admin", "secure_gateway"] {
            if let Some(raw) = optional_string(section, field)
                && let Err(error) = validate_network_list(&raw)
            {
                report.errors.push(format!("[{name}] {field} {error}"));
            }
        }

        if let Some(raw) = optional_string(section, "send_queue_limit")
            && let Err(error) = parse_usize_range(&raw, 1, 100_000)
        {
            report
                .errors
                .push(format!("[{name}] send_queue_limit {error}"));
        }
    }

    if peer_protocols > 1 {
        report
            .errors
            .push("More than one peer protocol configured in [server]".to_owned());
    }
}

fn validate_node_size(config: &BasicConfig, report: &mut ConfigValidationReport) {
    let value = config
        .legacy("node_size")
        .unwrap_or_else(|_| "medium".to_owned());
    if matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "tiny" | "small" | "medium" | "large" | "huge"
    ) {
        report.ok.push(format!("[node_size] {}", value.trim()));
    } else {
        report.errors.push(format!(
            "[node_size] invalid value '{}'; allowed: tiny, small, medium, large, huge",
            value.trim()
        ));
    }
}

fn validate_ledger_acquisition(config: &BasicConfig, report: &mut ConfigValidationReport) {
    match ledger_fetch_limit(config) {
        Ok(Some(limit)) => report
            .ok
            .push(format!("[ledger_acquisition] ledger_fetch_limit = {limit}")),
        Ok(None) => {}
        Err(error) => report.errors.push(error),
    }
}

fn validate_node_db_and_history(config: &BasicConfig, report: &mut ConfigValidationReport) {
    let node_db = config.section("node_db");
    let db_type = optional_string(node_db, "type").unwrap_or_else(|| "NuDB".to_owned());
    if !matches!(db_type.to_ascii_lowercase().as_str(), "nudb" | "rocksdb") {
        report
            .errors
            .push(format!("[node_db] type is invalid: {db_type}"));
    }

    match required_string(node_db, "path") {
        Ok(path) if path.trim().is_empty() => {
            report
                .errors
                .push("[node_db] path cannot be empty".to_owned());
        }
        Ok(_) => {}
        Err(error) => report.errors.push(format!("[node_db] {error}")),
    }

    if let Some(raw) = optional_string(node_db, "nudb_block_size")
        && !matches!(raw.as_str(), "4096" | "8192" | "16384" | "32768")
    {
        report.errors.push(format!(
            "[node_db] nudb_block_size must be one of 4096, 8192, 16384, 32768; got {raw}"
        ));
    }

    let online_delete = optional_string(node_db, "online_delete").map(|raw| {
        parse_u32_value(&raw).map_err(|error| format!("[node_db] online_delete {error}"))
    });
    let online_delete = match online_delete {
        Some(Ok(value)) => {
            if value != 0 && value < 256 {
                report
                    .errors
                    .push("[node_db] online_delete must be 0 or at least 256".to_owned());
            }
            Some(value)
        }
        Some(Err(error)) => {
            report.errors.push(error);
            None
        }
        None => None,
    };

    if let Some(raw) = optional_string(node_db, "advisory_delete")
        && parse_bool_value(&raw).is_err()
    {
        report
            .errors
            .push(format!("[node_db] advisory_delete is invalid: {raw}"));
    }

    let ledger_history_raw = config
        .legacy("ledger_history")
        .unwrap_or_else(|_| "0".to_owned());
    match parse_ledger_history(&ledger_history_raw) {
        Ok(LedgerHistory::Full) => {
            if online_delete.is_some_and(|value| value != 0) {
                report
                    .errors
                    .push("[ledger_history] full requires [node_db] online_delete = 0".to_owned());
            }
        }
        Ok(LedgerHistory::Count(history)) => {
            if let Some(delete) = online_delete
                && delete != 0
                && history > delete
            {
                report.errors.push(format!(
                    "[ledger_history] {history} cannot be greater than online_delete {delete}"
                ));
            }
        }
        Err(error) => report.errors.push(error),
    }

    let database_path = config.legacy("database_path").unwrap_or_default();
    if database_path.trim().is_empty() {
        report
            .warnings
            .push("[database_path] is empty; relational DBs may not persist".to_owned());
    }
}

fn validate_network_id(config: &BasicConfig, report: &mut ConfigValidationReport) {
    let value = config.legacy("network_id").unwrap_or_default();
    let trimmed = value.trim().to_ascii_lowercase();
    if trimmed.is_empty() {
        return;
    }
    if matches!(trimmed.as_str(), "main" | "testnet" | "devnet") || trimmed.parse::<u32>().is_ok() {
        report.ok.push(format!("[network_id] {trimmed}"));
    } else {
        report.errors.push(format!(
            "[network_id] invalid value '{value}'; use main, testnet, devnet, or a number"
        ));
    }
}

fn validate_overlay(config: &BasicConfig, report: &mut ConfigValidationReport) {
    let section = config.section("overlay");
    if let Some(raw) = optional_string(section, "ip_limit")
        && let Err(error) = parse_usize_range(&raw, 0, 100_000)
    {
        report.errors.push(format!("[overlay] ip_limit {error}"));
    }
    if let Some(raw) = optional_string(section, "verify_endpoints")
        && parse_bool_value(&raw).is_err()
    {
        report
            .errors
            .push(format!("[overlay] verify_endpoints is invalid: {raw}"));
    }
    if let Some(raw) = optional_string(section, "public_ip")
        && !raw.trim().is_empty()
        && raw.parse::<IpAddr>().is_err()
    {
        report
            .errors
            .push(format!("[overlay] public_ip is invalid: {raw}"));
    }
}

fn validate_crawl(config: &BasicConfig, report: &mut ConfigValidationReport) {
    let section = config.section("crawl");
    let values = section.values();
    if values.len() > 1 {
        report
            .errors
            .push("[crawl] section has too many bare values".to_owned());
    }
    if let Some(value) = values.first()
        && parse_bool_value(value).is_err()
    {
        report
            .errors
            .push(format!("[crawl] enabled value is invalid: {value}"));
    }
    for field in ["overlay", "server", "counts", "unl"] {
        if let Some(raw) = optional_string(section, field)
            && parse_bool_value(&raw).is_err()
        {
            report
                .errors
                .push(format!("[crawl] {field} is invalid: {raw}"));
        }
    }
}

fn validate_vl(config: &BasicConfig, report: &mut ConfigValidationReport) {
    if let Some(raw) = optional_string(config.section("vl"), "enabled")
        && parse_bool_value(&raw).is_err()
    {
        report
            .errors
            .push(format!("[vl] enabled is invalid: {raw}"));
    }
}

fn validate_reduce_relay(config: &BasicConfig, report: &mut ConfigValidationReport) {
    let section = config.section("reduce_relay");
    for field in ["vp_base_squelch_enable", "vp_enable", "tx_enable"] {
        if let Some(raw) = optional_string(section, field)
            && parse_bool_value(&raw).is_err()
        {
            report
                .errors
                .push(format!("[reduce_relay] {field} is invalid: {raw}"));
        }
    }
    if let Some(raw) = optional_string(section, "vp_base_squelch_max_selected_peers")
        && let Err(error) = parse_usize_range(&raw, 3, 10_000)
    {
        report.errors.push(format!(
            "[reduce_relay] vp_base_squelch_max_selected_peers {error}"
        ));
    }
    if let Some(raw) = optional_string(section, "tx_min_peers")
        && let Err(error) = parse_usize_range(&raw, 10, 100_000)
    {
        report
            .errors
            .push(format!("[reduce_relay] tx_min_peers {error}"));
    }
    if let Some(raw) = optional_string(section, "tx_relay_percentage")
        && let Err(error) = parse_usize_range(&raw, 10, 100)
    {
        report
            .errors
            .push(format!("[reduce_relay] tx_relay_percentage {error}"));
    }
}

fn validate_ssl_verify(config: &BasicConfig, report: &mut ConfigValidationReport) {
    let value = config.legacy("ssl_verify").unwrap_or_default();
    if !value.trim().is_empty() && parse_bool_value(&value).is_err() {
        report
            .errors
            .push(format!("[ssl_verify] invalid value: {value}"));
    }
}

fn required_string(section: &basics::basic_config::Section, key: &str) -> Result<String, String> {
    optional_string(section, key).ok_or_else(|| format!("missing required field: {key}"))
}

fn optional_string(section: &basics::basic_config::Section, key: &str) -> Option<String> {
    section.get::<String>(key).ok().flatten()
}

fn validate_network_list(raw: &str) -> Result<(), String> {
    for value in raw
        .split(',')
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        if value.parse::<IpAddr>().is_ok() || value.parse::<IpNet>().is_ok() {
            continue;
        }
        return Err(format!("contains invalid network: {value}"));
    }
    Ok(())
}

fn parse_u16_range(raw: &str, min: u16, max: u16) -> Result<u16, String> {
    let value = raw
        .trim()
        .parse::<u16>()
        .map_err(|_| format!("must be an integer from {min} to {max}; got {raw}"))?;
    if value < min || value > max {
        return Err(format!("must be an integer from {min} to {max}; got {raw}"));
    }
    Ok(value)
}

fn parse_usize_range(raw: &str, min: usize, max: usize) -> Result<usize, String> {
    let value = raw
        .trim()
        .parse::<usize>()
        .map_err(|_| format!("must be an integer from {min} to {max}; got {raw}"))?;
    if value < min || value > max {
        return Err(format!("must be an integer from {min} to {max}; got {raw}"));
    }
    Ok(value)
}

fn parse_u32_value(raw: &str) -> Result<u32, String> {
    raw.trim()
        .parse::<u32>()
        .map_err(|_| format!("must be an integer; got {raw}"))
}

fn parse_bool_value(raw: &str) -> Result<bool, ()> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "y" => Ok(true),
        "0" | "false" | "no" | "n" => Ok(false),
        _ => Err(()),
    }
}

enum LedgerHistory {
    Full,
    Count(u32),
}

fn parse_ledger_history(raw: &str) -> Result<LedgerHistory, String> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "full" => Ok(LedgerHistory::Full),
        "none" => Ok(LedgerHistory::Count(0)),
        value => value.parse::<u32>().map(LedgerHistory::Count).map_err(|_| {
            format!("[ledger_history] invalid value '{raw}'; use a number, none, or full")
        }),
    }
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
    use super::{validate_config_content, validate_ledger_fetch_limit};

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

    #[test]
    fn config_validation_rejects_installer_edge_cases() {
        let report = validate_config_content(
            r#"
[server]
port_rpc
port_peer

[port_rpc]
port = 5005
ip = 127.0.0.1
protocol = http
admin = nope
send_queue_limit = 0

[port_peer]
port = 5005
ip = bad-ip
protocol = peer

[node_size]
massive

[ledger_acquisition]
ledger_fetch_limit = 9

[node_db]
type = BadDb
path =
nudb_block_size = 5000
online_delete = 128
advisory_delete = maybe

[ledger_history]
256

[network_id]
sidechain

[overlay]
ip_limit = abc
verify_endpoints = maybe
public_ip = not-ip

[crawl]
1
0
overlay = maybe

[vl]
enabled = maybe

[reduce_relay]
vp_base_squelch_enable = maybe
vp_base_squelch_max_selected_peers = 2
tx_enable = maybe
tx_min_peers = 9
tx_relay_percentage = 101

[ssl_verify]
maybe
"#,
        );

        let errors = report.errors.join("\n");
        for expected in [
            "Duplicate listening port",
            "admin contains invalid network",
            "send_queue_limit",
            "ip is invalid",
            "[node_size] invalid",
            "between 1 and 8",
            "[node_db] type is invalid",
            "[node_db] missing required field: path",
            "nudb_block_size",
            "online_delete must be 0 or at least 256",
            "advisory_delete is invalid",
            "[ledger_history] 256 cannot be greater than online_delete 128",
            "[network_id] invalid",
            "[overlay] ip_limit",
            "[overlay] verify_endpoints",
            "[overlay] public_ip",
            "[crawl] section has too many bare values",
            "[crawl] overlay is invalid",
            "[vl] enabled is invalid",
            "vp_base_squelch_enable",
            "vp_base_squelch_max_selected_peers",
            "tx_enable",
            "tx_min_peers",
            "tx_relay_percentage",
            "[ssl_verify] invalid",
        ] {
            assert!(
                errors.contains(expected),
                "missing expected error '{expected}' in:\n{errors}"
            );
        }
    }

    #[test]
    fn config_validation_accepts_generated_defaults() {
        let report = validate_config_content(
            r#"
[server]
port_rpc_admin_local
port_peer
port_ws_admin_local

[port_rpc_admin_local]
port = 5005
ip = 127.0.0.1
admin = 127.0.0.1
protocol = http

[port_peer]
port = 51235
ip = 0.0.0.0
protocol = peer

[port_ws_admin_local]
port = 6006
ip = 127.0.0.1
admin = 127.0.0.1
protocol = ws
send_queue_limit = 500

[node_size]
medium

[ledger_acquisition]
ledger_fetch_limit = 8

[node_db]
type = NuDB
path = /tmp/xrpld/db/nudb
nudb_block_size = 4096
online_delete = 512
advisory_delete = 0

[database_path]
/tmp/xrpld/db

[ledger_history]
256

[network_id]
testnet

[overlay]
ip_limit = 0
verify_endpoints = 1

[crawl]
1
overlay = 1
server = 1
counts = 0
unl = 1

[vl]
enabled = 1

[reduce_relay]
vp_base_squelch_enable = 0
vp_base_squelch_max_selected_peers = 5
tx_enable = 0
tx_min_peers = 20
tx_relay_percentage = 25

[ssl_verify]
1
"#,
        );

        assert!(
            report.errors.is_empty(),
            "unexpected errors: {:?}",
            report.errors
        );
    }
}
