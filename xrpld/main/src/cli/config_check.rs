use std::path::Path;

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
    let required = [
        "[server]",
        "[node_db]",
        "[ips_fixed]",
        "[validator_list_sites]",
        "[validator_list_keys]",
    ];
    let mut all_ok = true;
    for section in required {
        if content.contains(section) {
            println!("  ✅ {section}");
        } else {
            println!("  ⚠️  {section} — missing (may be optional)");
            all_ok = false;
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
    } else {
        println!("⚠️  Config has warnings — node may still start");
    }
}
