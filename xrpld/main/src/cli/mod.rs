use clap::{Parser, Subcommand};

pub mod account;
pub mod amendments;
pub mod benchmark;
pub mod config_check;
pub mod db_stats;
pub mod doctor;
pub mod fee;
pub mod health;
pub mod interactive;
pub mod ledger_cmd;
pub mod log_level;
pub mod logo;
pub mod peers;
pub mod status;
pub mod stop;
pub mod sync_status;
pub mod validator_keys;
pub mod validators;
pub mod version;

#[derive(Parser)]
#[command(name = "xrpld", about = "XRPL Rust Node", version)]
pub struct Cli {
    /// Subcommand to run. If none, starts the node.
    #[command(subcommand)]
    pub command: Option<Command>,

    /// RPC endpoint URL
    #[arg(long, default_value = "http://127.0.0.1:5005", global = true)]
    pub rpc_url: String,

    /// Config file path (for config check and node startup)
    #[arg(long, short, global = true)]
    pub conf: Option<String>,
}

#[derive(Subcommand)]
pub enum Command {
    /// Show node status (state, peers, validated ledger, uptime)
    Status,
    /// Health check — exits 0 if healthy, 1 if not
    Health,
    /// List connected peers with details
    Peers,
    /// Show sync progress (useful during initial sync)
    SyncStatus,
    /// Show database statistics (NuDB size, entries, hit rate)
    DbStats,
    /// Get or set the log level at runtime
    LogLevel {
        /// New log level to set (trace, debug, info, warn, error)
        level: Option<String>,
    },
    /// Validate config file without starting the node
    #[command(name = "config")]
    ConfigCheck,
    /// Pre-flight check: config, ports, disk, connectivity
    Doctor,
    /// Show version, git commit, build info
    Version,
    /// List trusted validators
    Validators,
    /// Show enabled/supported amendments
    Amendments,
    /// Show current fee info
    Fee,
    /// Show ledger details
    Ledger {
        /// Ledger sequence (omit for validated)
        seq: Option<u64>,
    },
    /// Show account info
    Account {
        /// Account address (rXXX...)
        address: String,
    },
    /// Graceful node shutdown
    Stop,
    /// Connect to a peer
    Connect {
        /// Peer address (ip:port)
        address: String,
    },
    /// Run crypto/serialization benchmarks
    Benchmark,
    /// Validator key management
    #[command(name = "validator-keys")]
    ValidatorKeys {
        #[command(subcommand)]
        action: ValidatorKeysAction,
    },
    /// Interactive CLI mode
    #[command(name = "cli")]
    Cli,
}

#[derive(Subcommand)]
pub enum ValidatorKeysAction {
    /// Generate a new validator keypair
    Generate,
    /// Create a validator token (manifest)
    CreateToken {
        /// Master secret hex (reads from validator-keys.json if omitted)
        #[arg(long)]
        secret: Option<String>,
    },
    /// Sign data with the validator master key
    Sign {
        /// Data to sign
        data: String,
    },
    /// Create a revocation manifest
    Revoke,
    /// Show validator public key and creation date
    Show,
}

// ─── UI Helpers (Kiro-style output formatting) ───────────────────────────────

/// Format a number with comma separators (e.g. 104452398 → "104,452,398")
pub fn format_number(n: u64) -> String {
    let s = n.to_string();
    let mut result = String::with_capacity(s.len() + s.len() / 3);
    for (i, c) in s.chars().rev().enumerate() {
        if i > 0 && i % 3 == 0 {
            result.push(',');
        }
        result.push(c);
    }
    result.chars().rev().collect()
}

/// Print a spaced-dash section separator (dimmed)
pub fn section_separator() {
    let dim = console::Style::new().dim();
    println!(
        "    {}",
        dim.apply_to("─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─")
    );
}

/// Print a key-value pair with 4-space indent, label dim and right-padded to 18 chars
pub fn kv(label: &str, value: &str) {
    let dim = console::Style::new().dim();
    let bold = console::Style::new().bold();
    println!(
        "    {} {}",
        dim.apply_to(format!("{:<18}", label)),
        bold.apply_to(value)
    );
}

/// Print a bold section header with 4-space indent
pub fn section_header(title: &str) {
    println!("    {}", console::Style::new().bold().apply_to(title));
}

/// Print an error line with red dot
pub fn print_error(msg: &str) {
    eprintln!("    {} {}", console::Style::new().red().apply_to("●"), msg);
}

/// The solid divider printed between command output and next prompt
pub fn command_divider() {
    let width = crossterm::terminal::size()
        .map(|(w, _)| w as usize)
        .unwrap_or(80);
    let line_width = (width * 9) / 10; // 90% of terminal width
    let padding = (width - line_width) / 2;
    let dim = console::Style::new().color256(238); // very dim
    println!(
        "{}{}",
        " ".repeat(padding),
        dim.apply_to("╌".repeat(line_width))
    );
}

/// Send an RPC request and return the parsed JSON result.
pub fn rpc_call(
    url: &str,
    method: &str,
    params: serde_json::Value,
) -> Result<serde_json::Value, String> {
    let body = serde_json::json!({
        "method": method,
        "params": [params]
    });
    let resp = ureq::post(url)
        .set("Content-Type", "application/json")
        .send_string(&body.to_string())
        .map_err(|e| format!("Connection failed: {e}"))?;
    let text = resp
        .into_string()
        .map_err(|e| format!("Read failed: {e}"))?;
    let json: serde_json::Value =
        serde_json::from_str(&text).map_err(|e| format!("Invalid JSON: {e}"))?;
    Ok(json["result"].clone())
}

/// RPC call with a spinner shown while waiting.
#[cfg(test)]
#[path = "tests.rs"]
mod tests;
