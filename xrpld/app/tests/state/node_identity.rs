use app::{NodeIdentityOptions, NodeIdentityStore, get_node_identity};
use protocol::{KeyType, PublicKey, SecretKey, derive_public_key};
use std::sync::atomic::{AtomicUsize, Ordering};

struct RecordingStore {
    clears: AtomicUsize,
    pair: (PublicKey, SecretKey),
}

impl RecordingStore {
    fn new(secret_bytes: [u8; 32]) -> Self {
        let secret = SecretKey::from_bytes(secret_bytes);
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
fn node_identity_prefers_cmdline_seed_then_config_then_store() {
    let store = RecordingStore::new([7; 32]);

    let (cmd_public, _) = get_node_identity(
        &NodeIdentityOptions {
            node_id: Some("snoPBrXtMeMyMHUVTgbuqAfg1SUTb".to_owned()),
            new_node_id: true,
        },
        Some("shUwVw52ofnCUX5m7kPTKzJdr4HEH"),
        &store,
    )
    .expect("command-line node id should parse");
    assert_eq!(
        cmd_public.to_node_public_base58(),
        "n94a1u4jAz288pZLtw6yFWVbi89YamiC6JBXPVUj5zmExe5fTVg9"
    );
    assert_eq!(store.clears.load(Ordering::Relaxed), 0);

    let (config_public, _) = get_node_identity(
        &NodeIdentityOptions::default(),
        Some("snoPBrXtMeMyMHUVTgbuqAfg1SUTb"),
        &store,
    )
    .expect("config node seed should parse");
    assert_eq!(
        config_public.to_node_public_base58(),
        "n94a1u4jAz288pZLtw6yFWVbi89YamiC6JBXPVUj5zmExe5fTVg9"
    );

    let (stored_public, stored_secret) = get_node_identity(
        &NodeIdentityOptions {
            node_id: None,
            new_node_id: true,
        },
        None,
        &store,
    )
    .expect("store fallback should work");
    assert_eq!(stored_public, store.pair.0);
    assert_eq!(stored_secret.to_hex(), store.pair.1.to_hex());
    assert_eq!(store.clears.load(Ordering::Relaxed), 1);
}

#[test]
fn node_identity_cmdline_allows_generic_seed_shapes_without_widening_config_seed() {
    let store = RecordingStore::new([8; 32]);

    let (hex_public, _) = get_node_identity(
        &NodeIdentityOptions {
            node_id: Some("000102030405060708090A0B0C0D0E0F".to_owned()),
            new_node_id: false,
        },
        None,
        &store,
    )
    .expect("command-line hex seed should parse");

    assert_ne!(hex_public, store.pair.0);
    assert!(
        get_node_identity(
            &NodeIdentityOptions::default(),
            Some("000102030405060708090A0B0C0D0E0F"),
            &store,
        )
        .is_err()
    );

    let (passphrase_public, _) = get_node_identity(
        &NodeIdentityOptions {
            node_id: Some("runtime root passphrase".to_owned()),
            new_node_id: false,
        },
        None,
        &store,
    )
    .expect("command-line passphrase should hash into a seed");

    assert_ne!(passphrase_public, store.pair.0);
    assert!(
        get_node_identity(
            &NodeIdentityOptions::default(),
            Some("runtime root passphrase"),
            &store,
        )
        .is_err()
    );
}

#[test]
fn node_identity_rejects_non_seed_base58_tokens_for_cmdline_nodeid() {
    let store = RecordingStore::new([9; 32]);
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
