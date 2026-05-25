use console::Style;
use serde_json::json;
use std::fs;

const KEYS_FILE: &str = "validator-keys.json";

fn dim() -> Style {
    Style::new().dim()
}
fn bold() -> Style {
    Style::new().bold().white()
}
fn green() -> Style {
    Style::new().green()
}
fn red() -> Style {
    Style::new().red()
}

fn read_keys_file() -> Result<serde_json::Value, String> {
    let data =
        fs::read_to_string(KEYS_FILE).map_err(|e| format!("Cannot read {KEYS_FILE}: {e}"))?;
    serde_json::from_str(&data).map_err(|e| format!("Invalid JSON in {KEYS_FILE}: {e}"))
}

fn load_secret(secret_flag: Option<&str>) -> Result<protocol::SecretKey, String> {
    let secret_hex = match secret_flag {
        Some(s) => s.to_string(),
        None => {
            let keys = read_keys_file()?;
            keys["master_secret"].as_str().unwrap_or("").to_string()
        }
    };
    if secret_hex.is_empty() {
        return Err("No master secret found".to_string());
    }
    let bytes = hex::decode(&secret_hex).map_err(|e| format!("Invalid hex: {e}"))?;
    protocol::SecretKey::from_slice(&bytes).map_err(|_| "Invalid secret key".to_string())
}

pub fn run_generate() {
    let seed = protocol::seed::random_seed();
    let secret = protocol::generate_secret_key(protocol::KeyType::Ed25519, &seed)
        .expect("key generation should succeed");
    let public = protocol::derive_public_key(protocol::KeyType::Ed25519, &secret)
        .expect("public key derivation should succeed");

    let public_hex = hex::encode(public.as_bytes());
    let secret_hex = hex::encode(secret.as_bytes());
    let seed_b58 = protocol::seed::to_base58(&seed);
    let created = chrono::Utc::now().to_rfc3339();

    let keys = json!({
        "master_public": public_hex,
        "master_secret": secret_hex,
        "master_seed": seed_b58,
        "created": created,
    });

    fs::write(KEYS_FILE, serde_json::to_string_pretty(&keys).unwrap())
        .expect("failed to write validator-keys.json");

    println!("  {} Validator keypair generated", green().apply_to("●"));
    println!();
    println!(
        "  {} {}",
        dim().apply_to("public key"),
        bold().apply_to(&public_hex)
    );
    println!(
        "  {} {}",
        dim().apply_to("secret key"),
        bold().apply_to(&secret_hex)
    );
    println!(
        "  {} {}",
        dim().apply_to("seed      "),
        bold().apply_to(&seed_b58)
    );
    println!(
        "  {} {}",
        dim().apply_to("created   "),
        bold().apply_to(&created)
    );
    println!();
    println!(
        "  {} Saved to {}",
        dim().apply_to("●"),
        dim().apply_to(KEYS_FILE)
    );
}

pub fn run_create_token(secret_flag: Option<&str>) {
    let master_secret = match load_secret(secret_flag) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("  {} {e}", red().apply_to("●"));
            return;
        }
    };
    let master_public = protocol::derive_public_key(protocol::KeyType::Ed25519, &master_secret)
        .expect("master public key derivation should succeed");

    // Generate ephemeral keypair
    let eph_seed = protocol::seed::random_seed();
    let eph_secret = protocol::generate_secret_key(protocol::KeyType::Ed25519, &eph_seed)
        .expect("ephemeral key generation should succeed");
    let eph_public = protocol::derive_public_key(protocol::KeyType::Ed25519, &eph_secret)
        .expect("ephemeral public key derivation should succeed");

    // Build manifest payload: sequence + master_public + ephemeral_public
    let sequence: u32 = 1;
    let mut manifest_data = Vec::new();
    manifest_data.extend_from_slice(&sequence.to_be_bytes());
    manifest_data.extend_from_slice(master_public.as_bytes());
    manifest_data.extend_from_slice(eph_public.as_bytes());

    let signature = protocol::sign::sign(&master_public, &master_secret, &manifest_data)
        .expect("signing should succeed");

    let mut token_data = manifest_data;
    token_data.extend_from_slice(&signature);

    use base64::Engine;
    let token = base64::engine::general_purpose::STANDARD.encode(&token_data);

    println!("  {} Validator token created", green().apply_to("●"));
    println!();
    println!(
        "  {} Add this to your config [validator_token] section:",
        dim().apply_to("●")
    );
    println!();
    println!("  {}", bold().apply_to(&token));
}

pub fn run_sign(data: &str) {
    let secret = match load_secret(None) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("  {} {e}", red().apply_to("●"));
            return;
        }
    };
    let public = protocol::derive_public_key(protocol::KeyType::Ed25519, &secret)
        .expect("public key derivation should succeed");

    let signature =
        protocol::sign::sign(&public, &secret, data.as_bytes()).expect("signing should succeed");

    println!("  {} Data signed", green().apply_to("●"));
    println!();
    println!(
        "  {} {}",
        dim().apply_to("signature"),
        bold().apply_to(hex::encode(&signature))
    );
}

pub fn run_revoke() {
    let secret = match load_secret(None) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("  {} {e}", red().apply_to("●"));
            return;
        }
    };
    let public = protocol::derive_public_key(protocol::KeyType::Ed25519, &secret)
        .expect("public key derivation should succeed");

    // Revocation manifest uses sequence 0xFFFFFFFF
    let sequence: u32 = 0xFFFFFFFF;
    let mut manifest_data = Vec::new();
    manifest_data.extend_from_slice(&sequence.to_be_bytes());
    manifest_data.extend_from_slice(public.as_bytes());

    let signature =
        protocol::sign::sign(&public, &secret, &manifest_data).expect("signing should succeed");

    let mut token_data = manifest_data;
    token_data.extend_from_slice(&signature);

    use base64::Engine;
    let token = base64::engine::general_purpose::STANDARD.encode(&token_data);

    println!("  {} Revocation token created", green().apply_to("●"));
    println!();
    println!(
        "  {} Publish this to revoke your validator:",
        dim().apply_to("●")
    );
    println!();
    println!("  {}", bold().apply_to(&token));
}

pub fn run_show() {
    let keys = match read_keys_file() {
        Ok(k) => k,
        Err(e) => {
            eprintln!("  {} {e}", red().apply_to("●"));
            return;
        }
    };

    let public = keys["master_public"].as_str().unwrap_or("unknown");
    let created = keys["created"].as_str().unwrap_or("unknown");

    println!("  {} Validator keys", green().apply_to("●"));
    println!();
    println!(
        "  {} {}",
        dim().apply_to("public key"),
        bold().apply_to(public)
    );
    println!(
        "  {} {}",
        dim().apply_to("created   "),
        bold().apply_to(created)
    );
    println!();
    println!("  {} {}", dim().apply_to("file"), dim().apply_to(KEYS_FILE));
}
