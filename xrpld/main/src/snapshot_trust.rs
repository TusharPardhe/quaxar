//! Snapshot trust: key generation, signing, and the checks `load-snapshot`
//! runs before importing (signature by a trusted key, ledger validated on a
//! reference server).

use std::path::Path;

use nodestore::snapshot::signature::{self, SnapshotIdentity};
use protocol::serde_json::{self, Value, json};

/// Key file format written by `snapshot-keygen`.
const KEY_FILE_FORMAT: u64 = 1;

/// Write a new Ed25519 key file (mode 0600, never overwritten). Returns the
/// public key in `n…` form.
pub fn keygen(output: &Path) -> Result<String, String> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let seed = protocol::seed::random_seed();
    let secret = protocol::generate_root_secret_key(protocol::KeyType::Ed25519, &seed)
        .map_err(|_| "key generation failed".to_owned())?;
    let public = protocol::derive_public_key(protocol::KeyType::Ed25519, &secret)
        .map_err(|_| "public key derivation failed".to_owned())?
        .to_node_public_base58();
    let body = json!({
        "format": KEY_FILE_FORMAT,
        "key_type": "ed25519",
        "public_key": public,
        "secret_key": hex::encode_upper(secret.as_bytes()),
    });
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(output)
        .map_err(|e| format!("cannot create {}: {e}", output.display()))?;
    file.write_all(serde_json::to_string_pretty(&body).expect("json").as_bytes())
        .and_then(|()| file.write_all(b"\n"))
        .and_then(|()| file.sync_all())
        .map_err(|e| format!("cannot write {}: {e}", output.display()))?;
    Ok(public)
}

pub fn read_key_file(path: &Path) -> Result<protocol::SecretKey, String> {
    let text =
        std::fs::read_to_string(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    let value: Value =
        serde_json::from_str(&text).map_err(|e| format!("invalid key file: {e}"))?;
    if value["format"].as_u64() != Some(KEY_FILE_FORMAT) || value["key_type"] != "ed25519" {
        return Err("key file is not a quaxar Ed25519 snapshot key".to_owned());
    }
    let bytes = value["secret_key"]
        .as_str()
        .and_then(|text| hex::decode(text).ok())
        .ok_or_else(|| "secret_key is not hex".to_owned())?;
    protocol::SecretKey::from_slice(&bytes).map_err(|_| "secret_key is invalid".to_owned())
}

/// Check a `ledger` RPC result (from a reference server) says `identity`'s
/// ledger is validated with the same hash.
pub fn check_ledger_response(identity: &SnapshotIdentity, response: &Value) -> Result<(), String> {
    let result = &response["result"];
    if result["status"] != "success" {
        return Err(format!(
            "reference server did not return ledger {}: {}",
            identity.ledger_seq,
            result["error_message"].as_str().or(result["error"].as_str()).unwrap_or("error")
        ));
    }
    if result["validated"] != Value::Bool(true) {
        return Err(format!(
            "reference server does not report ledger {} as validated",
            identity.ledger_seq
        ));
    }
    let reported = result["ledger_hash"]
        .as_str()
        .or(result["ledger"]["ledger_hash"].as_str())
        .unwrap_or("");
    let expected = hex::encode_upper(identity.ledger_hash);
    if !reported.eq_ignore_ascii_case(&expected) {
        return Err(format!(
            "ledger {} hash differs: snapshot {expected}, reference {reported}",
            identity.ledger_seq
        ));
    }
    Ok(())
}

pub fn check_ledger(identity: &SnapshotIdentity, url: &str) -> Result<(), String> {
    let request = json!({
        "method": "ledger",
        "params": [{ "ledger_index": identity.ledger_seq }],
    });
    let response: Value = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(20))
        .build()
        .map_err(|e| e.to_string())?
        .post(url)
        .json(&request)
        .send()
        .and_then(|r| r.error_for_status())
        .map_err(|e| format!("reference server request failed: {e}"))?
        .json()
        .map_err(|e| format!("reference server reply is not JSON: {e}"))?;
    check_ledger_response(identity, &response)
}

/// Pre-import checks for `load-snapshot`. Returns a line per check passed.
pub fn verify_before_load(
    input: &Path,
    trusted_keys: &[String],
    require_signature: bool,
    check_ledger_url: Option<&str>,
) -> Result<Vec<String>, String> {
    let mut passed = Vec::new();
    let identity = signature::read_identity(input).map_err(|e| e.to_string())?;
    if require_signature || !trusted_keys.is_empty() {
        if trusted_keys.is_empty() {
            return Err("--require-signature needs at least one --trusted-key".to_owned());
        }
        let keys = trusted_keys
            .iter()
            .map(|key| signature::parse_public_key(key))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        let sig = signature::verify_file(input, &keys).map_err(|e| e.to_string())?;
        passed.push(format!(
            "signature by {} for ledger {}",
            sig.public_key.to_node_public_base58(),
            identity.ledger_seq
        ));
    }
    if let Some(url) = check_ledger_url {
        check_ledger(&identity, url)?;
        passed.push(format!("ledger {} validated on {url}", identity.ledger_seq));
    }
    Ok(passed)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn identity() -> SnapshotIdentity {
        SnapshotIdentity {
            ledger_seq: 108_283_244,
            ledger_hash: [0xAB; 32],
            file_sha256: [0x01; 32],
        }
    }

    #[test]
    fn ledger_check_accepts_only_a_validated_matching_hash() {
        let hash = hex::encode_upper([0xAB; 32]);
        let ok = json!({"result": {"status": "success", "validated": true, "ledger_hash": hash}});
        assert!(check_ledger_response(&identity(), &ok).is_ok());

        let lower = json!({"result": {"status": "success", "validated": true,
            "ledger": {"ledger_hash": hash.to_ascii_lowercase()}}});
        assert!(check_ledger_response(&identity(), &lower).is_ok());

        let not_validated = json!({"result": {"status": "success", "validated": false, "ledger_hash": hash}});
        assert!(check_ledger_response(&identity(), &not_validated).is_err());

        let other = json!({"result": {"status": "success", "validated": true,
            "ledger_hash": hex::encode_upper([0xCD; 32])}});
        assert!(check_ledger_response(&identity(), &other).unwrap_err().contains("differs"));

        let missing = json!({"result": {"status": "error", "error": "lgrNotFound"}});
        assert!(check_ledger_response(&identity(), &missing).unwrap_err().contains("lgrNotFound"));
    }

    #[test]
    fn key_file_round_trips_and_is_never_overwritten() {
        let dir = std::env::temp_dir().join(format!("qx-snapkey-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("key.json");
        let _ = std::fs::remove_file(&path);
        let public = keygen(&path).expect("keygen");
        let secret = read_key_file(&path).expect("read");
        let derived = protocol::derive_public_key(protocol::KeyType::Ed25519, &secret).unwrap();
        assert_eq!(derived.to_node_public_base58(), public);
        assert!(keygen(&path).is_err(), "existing key file is not overwritten");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600);
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
