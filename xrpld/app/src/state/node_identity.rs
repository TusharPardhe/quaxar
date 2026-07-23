//! Node identity selection with reference-matching seed precedence.

use protocol::{
    KeyType, PublicKey, SecretKey, Seed, TokenType, derive_public_key, encode_base58_token,
    generate_root_secret_key, parse_base58_node_public, parse_base58_seed, parse_base58_with_type,
    parse_generic_seed, random_seed,
};
use rusqlite::OptionalExtension;
use std::fmt;
use xrpld_core::DatabaseCon;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NodeIdentityOptions {
    pub node_id: Option<String>,
    pub new_node_id: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NodeIdentityError {
    InvalidCommandLineNodeId,
    InvalidConfigNodeSeed,
}

impl fmt::Display for NodeIdentityError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidCommandLineNodeId => f.write_str("Invalid 'nodeid' in command line"),
            Self::InvalidConfigNodeSeed => f.write_str("Invalid [node_seed] in configuration file"),
        }
    }
}

impl std::error::Error for NodeIdentityError {}

pub trait NodeIdentityStore {
    fn clear_node_identity(&self);
    fn load_node_identity(&self) -> (PublicKey, SecretKey);
}

pub fn get_node_identity(
    options: &NodeIdentityOptions,
    config_node_seed: Option<&str>,
    store: &dyn NodeIdentityStore,
) -> Result<(PublicKey, SecretKey), NodeIdentityError> {
    if let Some(seed_text) = options.node_id.as_deref() {
        return derive_identity_from_command_line_seed(seed_text)
            .ok_or(NodeIdentityError::InvalidCommandLineNodeId);
    }

    if let Some(seed_text) = config_node_seed {
        let seed = parse_base58_seed(seed_text).ok_or(NodeIdentityError::InvalidConfigNodeSeed)?;
        return derive_identity_from_seed(seed).ok_or(NodeIdentityError::InvalidConfigNodeSeed);
    }

    if options.new_node_id {
        store.clear_node_identity();
    }
    Ok(store.load_node_identity())
}

fn derive_identity_from_command_line_seed(seed_text: &str) -> Option<(PublicKey, SecretKey)> {
    let seed = parse_generic_seed(seed_text, false)?;
    derive_identity_from_seed(seed)
}

fn derive_identity_from_seed(seed: Seed) -> Option<(PublicKey, SecretKey)> {
    let secret_key = generate_root_secret_key(KeyType::Secp256k1, &seed).ok()?;
    let public_key = derive_public_key(KeyType::Secp256k1, &secret_key).ok()?;
    Some((public_key, secret_key))
}

/// Load or generate the node identity from the wallet SQLite database,
/// matching reference `getNodeIdentity(soci::session&)` in the wallet identity module.
pub fn load_or_generate_node_identity(wallet_db: &DatabaseCon) -> (PublicKey, SecretKey) {
    let conn = wallet_db.get_session();

    // Try to load an existing identity.
    let existing: Option<(String, String)> = conn
        .query_row(
            "SELECT PublicKey, PrivateKey FROM NodeIdentity LIMIT 1",
            [],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        )
        .optional()
        .ok()
        .flatten();

    if let Some((pub_str, priv_str)) = existing {
        let sk = parse_base58_with_type::<SecretKey>(TokenType::NodePrivate, &priv_str);
        let pk_bytes = parse_base58_node_public(&pub_str);
        if let (Some(sk), Some(pk_bytes)) = (sk, pk_bytes) {
            let pk = PublicKey::from_bytes(pk_bytes);
            // Verify the pair matches.
            if let Ok(derived) = derive_public_key(KeyType::Secp256k1, &sk) {
                if derived == pk {
                    return (pk, sk);
                }
            }
        }
    }

    // Generate a new random identity and persist it.
    let seed = random_seed();
    let sk = generate_root_secret_key(KeyType::Secp256k1, &seed)
        .expect("secp256k1 key generation must succeed");
    let pk = derive_public_key(KeyType::Secp256k1, &sk)
        .expect("secp256k1 public key derivation must succeed");

    let pub_b58 = pk.to_node_public_base58();
    let priv_b58 = encode_base58_token(TokenType::NodePrivate, sk.as_bytes());
    let _ = conn.execute(
        "INSERT INTO NodeIdentity (PublicKey, PrivateKey) VALUES (?1, ?2)",
        rusqlite::params![pub_b58, priv_b58],
    );

    (pk, sk)
}

#[cfg(test)]
mod tests {
    use super::{NodeIdentityOptions, NodeIdentityStore, get_node_identity};
    use protocol::{
        KeyType, PublicKey, SecretKey, Seed, derive_public_key, generate_root_secret_key,
        generate_seed,
    };
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct RecordingStore {
        clears: AtomicUsize,
        pair: (PublicKey, SecretKey),
    }

    impl RecordingStore {
        fn new(secret: [u8; 32]) -> Self {
            let secret = SecretKey::from_bytes(secret);
            let public = derive_public_key(KeyType::Secp256k1, &secret).expect("public key");
            Self {
                clears: AtomicUsize::new(0),
                pair: (public, secret),
            }
        }
    }

    impl NodeIdentityStore for RecordingStore {
        fn clear_node_identity(&self) {
            self.clears.fetch_add(1, Ordering::Relaxed);
        }

        fn load_node_identity(&self) -> (PublicKey, SecretKey) {
            (self.pair.0, self.pair.1.clone())
        }
    }

    #[test]
    fn node_identity_prefers_explicit_seed_over_store() {
        let store = RecordingStore::new([2; 32]);
        let (public, _) = get_node_identity(
            &NodeIdentityOptions {
                node_id: Some("snoPBrXtMeMyMHUVTgbuqAfg1SUTb".to_owned()),
                new_node_id: true,
            },
            Some("shUwVw52ofnCUX5m7kPTKzJdr4HEH"),
            &store,
        )
        .expect("explicit node id should parse");

        assert_eq!(
            public.to_node_public_base58(),
            "n94a1u4jAz288pZLtw6yFWVbi89YamiC6JBXPVUj5zmExe5fTVg9"
        );
        assert_eq!(store.clears.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn node_identity_command_line_accepts_passphrase_shape_that_config_rejects() {
        let store = RecordingStore::new([3; 32]);
        let passphrase = "runtime-root generic seed";

        let (public, secret) = get_node_identity(
            &NodeIdentityOptions {
                node_id: Some(passphrase.to_owned()),
                new_node_id: false,
            },
            None,
            &store,
        )
        .expect("command-line generic seed should hash into a seed");

        let seed = *generate_seed(passphrase).data();
        let expected_seed = Seed::from_slice(&seed).expect("generated seed should keep width");
        let expected_secret = generate_root_secret_key(KeyType::Secp256k1, &expected_seed)
            .expect("hashed seed should derive a secret");
        let expected_public =
            derive_public_key(KeyType::Secp256k1, &expected_secret).expect("public key");

        assert_eq!(public, expected_public);
        assert_eq!(secret.to_hex(), expected_secret.to_hex());
        assert!(
            get_node_identity(&NodeIdentityOptions::default(), Some(passphrase), &store).is_err()
        );
    }

    #[test]
    fn node_identity_command_line_rejects_other_base58_token_types() {
        let store = RecordingStore::new([4; 32]);
        let result = get_node_identity(
            &NodeIdentityOptions {
                node_id: Some("n94a1u4jAz288pZLtw6yFWVbi89YamiC6JBXPVUj5zmExe5fTVg9".to_owned()),
                new_node_id: false,
            },
            None,
            &store,
        );

        assert!(result.is_err());
    }
}
