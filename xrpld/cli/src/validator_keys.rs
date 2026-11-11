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

/// A built `[validator_token]`: the config value and the manifest inside it.
pub struct BuiltToken {
    pub token: String,
    pub manifest_b64: String,
}

/// Build a validator token exactly as rippled's validator-keys tool does
/// (`ValidatorKeys::createValidatorToken`): a manifest with sfSequence,
/// sfPublicKey (master), sfSigningPubKey (a fresh ephemeral key) and, when
/// set, sfDomain; signed by the ephemeral key (sfSignature) and the master key
/// (sfMasterSignature) over HashPrefix::Manifest. The token is
/// base64({"validation_secret_key": hex, "manifest": base64}).
pub fn build_validator_token(
    master_secret: &protocol::SecretKey,
    sequence: u32,
    domain: Option<&str>,
) -> BuiltToken {
    let master_public = protocol::derive_public_key(protocol::KeyType::Secp256k1, master_secret)
        .expect("master public key derivation should succeed");
    let eph_seed = protocol::seed::random_seed();
    let eph_secret = protocol::generate_root_secret_key(protocol::KeyType::Secp256k1, &eph_seed)
        .expect("ephemeral key generation should succeed");
    let eph_public = protocol::derive_public_key(protocol::KeyType::Secp256k1, &eph_secret)
        .expect("ephemeral public key derivation should succeed");

    let mut manifest = protocol::STObject::new(protocol::sf_generic());
    manifest.set_field_u32(protocol::get_field_by_symbol("sfSequence"), sequence);
    manifest.set_field_vl(
        protocol::get_field_by_symbol("sfPublicKey"),
        master_public.as_bytes(),
    );
    manifest.set_field_vl(
        protocol::get_field_by_symbol("sfSigningPubKey"),
        eph_public.as_bytes(),
    );
    if let Some(domain) = domain {
        manifest.set_field_vl(protocol::get_field_by_symbol("sfDomain"), domain.as_bytes());
    }

    // The signed bytes are HashPrefix::Manifest followed by the manifest fields
    // excluding the signature fields (STObject::add_without_signing_fields).
    let signing_bytes = |st: &protocol::STObject| -> Vec<u8> {
        let mut serializer = protocol::Serializer::default();
        serializer.add32_prefix(protocol::HashPrefix::Manifest);
        st.add_without_signing_fields(&mut serializer);
        serializer.data().to_vec()
    };
    let eph_signature = protocol::sign::sign(&eph_public, &eph_secret, &signing_bytes(&manifest))
        .expect("ephemeral manifest signing should succeed");
    manifest.set_field_vl(protocol::get_field_by_symbol("sfSignature"), &eph_signature);
    let master_signature =
        protocol::sign::sign(&master_public, master_secret, &signing_bytes(&manifest))
            .expect("master manifest signing should succeed");
    manifest.set_field_vl(
        protocol::get_field_by_symbol("sfMasterSignature"),
        &master_signature,
    );

    use base64::Engine;
    let manifest_b64 = base64::engine::general_purpose::STANDARD
        .encode(manifest.get_serializer().data());
    let token_json = json!({
        "validation_secret_key": hex::encode_upper(eph_secret.as_bytes()),
        "manifest": manifest_b64,
    });
    let token = base64::engine::general_purpose::STANDARD.encode(token_json.to_string().as_bytes());
    BuiltToken {
        token,
        manifest_b64,
    }
}

/// rippled validator-keys tool `attestDomain`: the master key's signature, as
/// upper-case hex, over `[domain-attestation-blob:<domain>:<master key>]`.
pub fn domain_attestation(master_secret: &protocol::SecretKey, domain: &str) -> String {
    let master_public = protocol::derive_public_key(protocol::KeyType::Secp256k1, master_secret)
        .expect("master public key derivation should succeed");
    let blob = format!(
        "[domain-attestation-blob:{domain}:{}]",
        master_public.to_node_public_base58()
    );
    let signature = protocol::sign::sign(&master_public, master_secret, blob.as_bytes())
        .expect("attestation signing should succeed");
    hex::encode_upper(signature)
}

/// rippled validator-keys tool `ValidatorKeys::domain`: 4-128 characters,
/// `[host.][subdomain.]domain.tld`.
pub fn validate_domain(domain: &str) -> Result<(), String> {
    if !(4..=128).contains(&domain.len()) {
        return Err("The domain must be between 4 and 128 characters long.".to_string());
    }
    let labels: Vec<&str> = domain.split('.').collect();
    let label_ok = |label: &str| {
        !label.is_empty()
            && label.len() <= 63
            && !label.starts_with('-')
            && !label.ends_with('-')
            && label.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
    };
    let tld = labels.last().copied().unwrap_or("");
    if labels.len() < 2
        || !labels.iter().all(|label| label_ok(label))
        || tld.len() < 2
        || !tld.chars().all(|c| c.is_ascii_alphabetic())
    {
        return Err(
            "The domain field must use the '[host.][subdomain.]domain.tld' format".to_string(),
        );
    }
    Ok(())
}

pub fn run_create_token(secret_flag: Option<&str>) {
    let master_secret = match load_secret(secret_flag) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("  {} {e}", red().apply_to("●"));
            return;
        }
    };
    let built = build_validator_token(&master_secret, 1, None);
    println!("  {} Validator token created", green().apply_to("●"));
    println!();
    println!(
        "  {} Add this to your config [validator_token] section:",
        dim().apply_to("●")
    );
    println!();
    println!("  {}", bold().apply_to(&built.token));
}

/// Rewrite the key file in place (mode 0600) after keeping a `.bak` copy.
fn update_keys_file(keys: &serde_json::Value) -> Result<(), String> {
    let backup = format!("{KEYS_FILE}.bak");
    fs::copy(KEYS_FILE, &backup).map_err(|e| format!("Cannot back up {KEYS_FILE}: {e}"))?;
    let temp = format!("{KEYS_FILE}.tmp");
    let _ = fs::remove_file(&temp);
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&temp)
        .map_err(|e| format!("Cannot create {temp}: {e}"))?;
    let serialized = serde_json::to_vec_pretty(keys).map_err(|e| e.to_string())?;
    file.write_all(&serialized)
        .and_then(|()| file.write_all(b"\n"))
        .and_then(|()| file.sync_all())
        .map_err(|e| format!("Cannot write {temp}: {e}"))?;
    fs::rename(&temp, KEYS_FILE).map_err(|e| format!("Cannot replace {KEYS_FILE}: {e}"))
}

/// rippled validator-keys tool `set_domain` / `clear_domain`: record the
/// domain and the next token sequence in the key file, issue a new token whose
/// manifest carries sfDomain, and print the token and the attestation.
pub fn run_set_domain(domain: &str) {
    let fail = |e: String| eprintln!("  {} {e}", red().apply_to("●"));
    let domain = domain.trim().to_ascii_lowercase();
    if !domain.is_empty()
        && let Err(e) = validate_domain(&domain)
    {
        return fail(e);
    }
    let mut keys = match read_keys_file() {
        Ok(k) => k,
        Err(e) => return fail(e),
    };
    if keys["revoked"].as_bool() == Some(true) {
        return fail("The specified master key has been revoked".to_string());
    }
    if keys["domain"].as_str().unwrap_or("") == domain {
        println!("  {} The domain is already set to '{domain}'", dim().apply_to("●"));
        return;
    }
    let master_secret = match load_secret(None) {
        Ok(s) => s,
        Err(e) => return fail(e),
    };
    let sequence = keys["token_sequence"].as_u64().unwrap_or(0) as u32 + 1;
    let built = build_validator_token(
        &master_secret,
        sequence,
        (!domain.is_empty()).then_some(domain.as_str()),
    );
    keys["token_sequence"] = json!(sequence);
    if domain.is_empty() {
        if let Some(object) = keys.as_object_mut() {
            object.remove("domain");
        }
    } else {
        keys["domain"] = json!(domain);
    }
    if let Err(e) = update_keys_file(&keys) {
        return fail(e);
    }
    println!(
        "  {} Domain {} (token sequence {sequence})",
        green().apply_to("●"),
        if domain.is_empty() { "cleared".to_string() } else { format!("set to {domain}") }
    );
    if !domain.is_empty() {
        println!();
        println!("  Add this to the [[VALIDATORS]] entry in https://{domain}/.well-known/xrp-ledger.toml:");
        println!();
        println!("  attestation = \"{}\"", domain_attestation(&master_secret, &domain));
    }
    println!();
    println!("  Replace [validation_seed] in the config with this [validator_token] and restart:");
    println!();
    println!("  [validator_token]");
    println!("  {}", built.token);
}

/// rippled validator-keys tool `attest_domain`.
pub fn run_attest_domain() {
    let fail = |e: String| eprintln!("  {} {e}", red().apply_to("●"));
    let keys = match read_keys_file() {
        Ok(k) => k,
        Err(e) => return fail(e),
    };
    let Some(domain) = keys["domain"].as_str().filter(|d| !d.is_empty()) else {
        return fail("No domain is set; run `validator-keys set-domain <domain>` first".to_string());
    };
    let master_secret = match load_secret(None) {
        Ok(s) => s,
        Err(e) => return fail(e),
    };
    println!("  attestation = \"{}\"", domain_attestation(&master_secret, domain));
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

    /// The domain token's manifest parses with the node's own manifest code,
    /// keeps the master key, carries the domain and verifies; the attestation
    /// verifies against the master key over rippled's blob format.
    #[test]
    fn domain_token_and_attestation_match_rippled_formats() {
        use base64::Engine;
        let seed = parse_base58_seed("snoPBrXtMeMyMHUVTgbuqAfg1SUTb").expect("seed vector");
        let master =
            protocol::generate_root_secret_key(KeyType::Secp256k1, &seed).expect("master");
        let master_public = derive_public_key(KeyType::Secp256k1, &master).expect("public");
        let built = super::build_validator_token(&master, 2, Some("quaxar.xyz"));

        let manifest = base64::engine::general_purpose::STANDARD
            .decode(&built.manifest_b64)
            .expect("manifest base64");
        let mut sit = protocol::SerialIter::new(&manifest);
        let st = protocol::STObject::from_serial_iter(&mut sit, protocol::sf_generic(), 0);
        assert_eq!(st.get_field_u32(protocol::get_field_by_symbol("sfSequence")), 2);
        assert_eq!(
            st.get_field_vl(protocol::get_field_by_symbol("sfPublicKey")),
            master_public.as_bytes()
        );
        assert_eq!(
            st.get_field_vl(protocol::get_field_by_symbol("sfDomain")),
            b"quaxar.xyz"
        );
        let mut signed = protocol::Serializer::default();
        signed.add32_prefix(protocol::HashPrefix::Manifest);
        st.add_without_signing_fields(&mut signed);
        let master_sig = st.get_field_vl(protocol::get_field_by_symbol("sfMasterSignature"));
        assert!(protocol::sign::verify(&master_public, signed.data(), &master_sig));

        let token_json: serde_json::Value = serde_json::from_slice(
            &base64::engine::general_purpose::STANDARD
                .decode(&built.token)
                .expect("token base64"),
        )
        .expect("token json");
        assert_eq!(token_json["manifest"], built.manifest_b64);

        let attestation = super::domain_attestation(&master, "quaxar.xyz");
        let blob = format!(
            "[domain-attestation-blob:quaxar.xyz:{}]",
            master_public.to_node_public_base58()
        );
        let signature = hex::decode(&attestation).expect("attestation hex");
        assert_eq!(attestation, attestation.to_ascii_uppercase());
        assert!(protocol::sign::verify(&master_public, blob.as_bytes(), &signature));
    }

    #[test]
    fn domain_validation_matches_the_rippled_tool_rules() {
        assert!(super::validate_domain("quaxar.xyz").is_ok());
        assert!(super::validate_domain("validator.quaxar.xyz").is_ok());
        for bad in ["abc", "quaxar", "-quaxar.xyz", "quaxar..xyz", "quaxar.x", "qu axar.xyz"] {
            assert!(super::validate_domain(bad).is_err(), "{bad} must be rejected");
        }
    }

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
