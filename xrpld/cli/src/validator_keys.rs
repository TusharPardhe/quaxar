use console::Style;
use serde_json::json;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;

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
    let secret = match secret_flag {
        Some(s) => s.to_string(),
        None => {
            let keys = read_keys_file()?;
            if let Some(seed) = keys["validation_seed"].as_str() {
                let seed = protocol::parse_base58_seed(seed)
                    .ok_or_else(|| "Invalid validation seed in key file".to_string())?;
                return protocol::generate_root_secret_key(protocol::KeyType::Secp256k1, &seed)
                    .map_err(|_| "Invalid validator master key".to_string());
            }
            keys["master_secret"].as_str().unwrap_or("").to_string()
        }
    };
    if secret.is_empty() {
        return Err("No master secret found".to_string());
    }
    if let Some(seed) = protocol::parse_base58_seed(&secret) {
        return protocol::generate_root_secret_key(protocol::KeyType::Secp256k1, &seed)
            .map_err(|_| "Invalid validator master key".to_string());
    }
    if let Some(secret) = protocol::parse_base58_with_type::<protocol::SecretKey>(
        protocol::TokenType::NodePrivate,
        &secret,
    ) {
        return Ok(secret);
    }
    let bytes = hex::decode(&secret).map_err(|e| format!("Invalid validator secret: {e}"))?;
    protocol::SecretKey::from_slice(&bytes).map_err(|_| "Invalid secret key".to_string())
}

fn generated_keys(seed: &protocol::Seed, created: &str) -> serde_json::Value {
    // Rippled validator identities are secp256k1 root keys. They are encoded
    // as NodePublic (`nH...`) and NodePrivate (`p...`) tokens; an Ed25519 hex
    // key is not accepted by `[validation_seed]` or the validator tooling.
    let secret = protocol::generate_root_secret_key(protocol::KeyType::Secp256k1, seed)
        .expect("key generation should succeed");
    let public = protocol::derive_public_key(protocol::KeyType::Secp256k1, &secret)
        .expect("public key derivation should succeed");

    let public_base58 = public.to_node_public_base58();
    let private_base58 =
        protocol::encode_base58_token(protocol::TokenType::NodePrivate, secret.as_bytes());
    let seed_base58 = protocol::seed::to_base58(seed);
    let validation_key = protocol::seed::seed_as_1751(seed);

    json!({
        "format": 1,
        "key_type": "secp256k1",
        "validation_public_key": public_base58,
        "validation_private_key": private_base58,
        "validation_seed": seed_base58,
        "validation_key": validation_key,
        "created": created,
    })
}

fn write_keys_file(keys: &serde_json::Value) -> Result<(), String> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(KEYS_FILE)
        .map_err(|error| format!("Cannot create {KEYS_FILE}: {error}"))?;
    let serialized = serde_json::to_vec_pretty(keys).map_err(|error| error.to_string())?;
    file.write_all(&serialized)
        .and_then(|()| file.write_all(b"\n"))
        .and_then(|()| file.sync_all())
        .map_err(|error| format!("Cannot write {KEYS_FILE}: {error}"))
}

pub fn run_generate() {
    let seed = protocol::seed::random_seed();
    let created = chrono::Utc::now().to_rfc3339();
    let keys = generated_keys(&seed, &created);

    if let Err(error) = write_keys_file(&keys) {
        eprintln!("  {} {error}", red().apply_to("●"));
        return;
    }

    let public = keys["validation_public_key"].as_str().unwrap_or("unknown");
    let private = keys["validation_private_key"].as_str().unwrap_or("unknown");
    let seed = keys["validation_seed"].as_str().unwrap_or("unknown");
    let validation_key = keys["validation_key"].as_str().unwrap_or("unknown");

    println!("  {} Validator keypair generated", green().apply_to("●"));
    println!();
    println!(
        "  {} {}",
        dim().apply_to("validation public key "),
        bold().apply_to(public)
    );
    println!(
        "  {} {}",
        dim().apply_to("validation private key"),
        bold().apply_to(private)
    );
    println!(
        "  {} {}",
        dim().apply_to("validation seed       "),
        bold().apply_to(seed)
    );
    println!(
        "  {} {}",
        dim().apply_to("validation key        "),
        bold().apply_to(validation_key)
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
    let master_public = protocol::derive_public_key(protocol::KeyType::Secp256k1, &master_secret)
        .expect("master public key derivation should succeed");

    // Generate ephemeral keypair
    let eph_seed = protocol::seed::random_seed();
    let eph_secret = protocol::generate_root_secret_key(protocol::KeyType::Secp256k1, &eph_seed)
        .expect("ephemeral key generation should succeed");
    let eph_public = protocol::derive_public_key(protocol::KeyType::Secp256k1, &eph_secret)
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

    let public = keys["validation_public_key"].as_str().unwrap_or("unknown");
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

#[cfg(test)]
mod tests {
    use super::generated_keys;
    use protocol::{
        KeyType, TokenType, derive_public_key, parse_base58_seed, parse_base58_with_type,
    };

    #[test]
    fn generated_validator_keys_use_rippled_secp256k1_token_formats() {
        let seed = parse_base58_seed("snoPBrXtMeMyMHUVTgbuqAfg1SUTb").expect("seed vector");
        let keys = generated_keys(&seed, "2026-08-21T00:00:00Z");

        assert_eq!(keys["key_type"], "secp256k1");
        assert_eq!(keys["validation_seed"], "snoPBrXtMeMyMHUVTgbuqAfg1SUTb");
        let private = parse_base58_with_type::<protocol::SecretKey>(
            TokenType::NodePrivate,
            keys["validation_private_key"]
                .as_str()
                .expect("private key"),
        )
        .expect("NodePrivate token");
        let public = derive_public_key(KeyType::Secp256k1, &private).expect("public key");
        assert_eq!(
            keys["validation_public_key"],
            public.to_node_public_base58()
        );
        assert!(
            keys["validation_public_key"]
                .as_str()
                .expect("public key")
                .starts_with("n")
        );
        assert!(
            keys["validation_private_key"]
                .as_str()
                .expect("private key")
                .starts_with('p')
        );
    }
}
