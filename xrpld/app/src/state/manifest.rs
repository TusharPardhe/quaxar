#![cfg_attr(test, allow(dead_code))]

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::OnceLock;
use std::sync::RwLock;
use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};

use basics::base_uint::Uint256;
use basics::base64::base64_decode;
use basics::basic_config::BasicConfig;
use basics::string_utilities::{is_properly_formed_toml_domain, str_unhex};
use protocol::{
    HashPrefix, PublicKey, SOEStyle, SOElement, SOTemplate, STObject, SecretKey, SerialIter,
    get_field_by_symbol, sf_generic, verify_st_object,
};
use quaxar_core::DatabaseCon;
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Manifest {
    pub serialized: Vec<u8>,
    pub master_key: PublicKey,
    pub signing_key: Option<PublicKey>,
    pub sequence: u32,
    pub domain: String,
}

impl Manifest {
    pub const fn revoked_sequence(sequence: u32) -> bool {
        sequence == u32::MAX
    }

    pub const fn revoked(&self) -> bool {
        Self::revoked_sequence(self.sequence)
    }

    pub fn verify(&self) -> bool {
        let Some(st) = parse_manifest_stobject(&self.serialized) else {
            return false;
        };

        if !self.revoked() && self.signing_key.is_none() {
            return false;
        }

        if let Some(signing_key) = self.signing_key.as_ref()
            && !self.revoked()
            && !verify_st_object(
                &st,
                HashPrefix::Manifest,
                signing_key,
                get_field_by_symbol("sfSignature"),
            )
        {
            return false;
        }

        verify_st_object(
            &st,
            HashPrefix::Manifest,
            &self.master_key,
            get_field_by_symbol("sfMasterSignature"),
        )
    }

    pub fn hash(&self) -> Option<Uint256> {
        parse_manifest_stobject(&self.serialized).map(|st| st.get_hash(HashPrefix::Manifest))
    }

    pub fn get_signature(&self) -> Option<Vec<u8>> {
        let st = parse_manifest_stobject(&self.serialized)?;
        st.is_field_present(get_field_by_symbol("sfSignature"))
            .then(|| st.get_field_vl(get_field_by_symbol("sfSignature")))
    }

    pub fn get_master_signature(&self) -> Option<Vec<u8>> {
        let st = parse_manifest_stobject(&self.serialized)?;
        Some(st.get_field_vl(get_field_by_symbol("sfMasterSignature")))
    }
}

impl fmt::Display for Manifest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let master = self.master_key.to_node_public_base58();
        if self.revoked() {
            return write!(formatter, "Revocation Manifest {master}");
        }

        let signing = self
            .signing_key
            .map(PublicKey::to_node_public_base58)
            .unwrap_or_else(|| panic!("No SigningKey in manifest {master}"));
        write!(
            formatter,
            "Manifest {master} ({}: {signing})",
            self.sequence
        )
    }
}

#[derive(Debug, Clone)]
pub struct ValidatorToken {
    pub manifest: String,
    pub validation_secret: SecretKey,
}

pub const MAX_UNTRUSTED_MANIFESTS: usize = 300;
pub const MAX_TRUSTED_MANIFESTS: usize = 300;
pub const MIN_MANIFEST_COUNT: usize = 50;
pub const MAX_MANIFEST_COUNT: usize = 1000;

/// Resolved `[overlay]` manifest limits. Both settings default to 300 and are
/// constrained to the same 50–1000 range as rippled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ManifestLimits {
    pub max_untrusted_count: usize,
    pub max_trusted_count: usize,
}

impl Default for ManifestLimits {
    fn default() -> Self {
        Self {
            max_untrusted_count: MAX_UNTRUSTED_MANIFESTS,
            max_trusted_count: MAX_TRUSTED_MANIFESTS,
        }
    }
}

impl ManifestLimits {
    pub fn from_config(config: &BasicConfig) -> Result<Self, String> {
        let overlay = config.section("overlay");
        let parse = |key: &str, default: usize| -> Result<usize, String> {
            let count = overlay.get::<usize>(key).map_err(|_| {
                format!("invalid [overlay] {key}: must be an integer count of manifests")
            })?;
            let count = count.unwrap_or(default);
            if !(MIN_MANIFEST_COUNT..=MAX_MANIFEST_COUNT).contains(&count) {
                return Err(format!(
                    "invalid [overlay] {key}: must be between {MIN_MANIFEST_COUNT} and {MAX_MANIFEST_COUNT}, inclusive"
                ));
            }
            Ok(count)
        };

        Ok(Self {
            max_untrusted_count: parse("max_untrusted_count", MAX_UNTRUSTED_MANIFESTS)?,
            max_trusted_count: parse("max_trusted_count", MAX_TRUSTED_MANIFESTS)?,
        })
    }

    pub const fn maximum_message_size(self) -> usize {
        overlay::message::maximum_manifests_message_size(
            self.max_trusted_count,
            self.max_untrusted_count,
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ManifestRateLimitCapPolicy {
    Capped,
    Uncapped,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ManifestDisposition {
    Accepted = 0,
    Stale,
    BadMasterKey,
    BadEphemeralKey,
    UntrustedCapacity,
    Invalid,
}

#[derive(Debug, Default)]
struct ManifestCacheState {
    manifests: HashMap<PublicKey, Manifest>,
    signing_to_master_keys: HashMap<PublicKey, PublicKey>,
    untrusted_master_keys: HashSet<PublicKey>,
}

#[derive(Debug)]
pub struct ManifestCache {
    state: RwLock<ManifestCacheState>,
    sequence: AtomicU32,
    max_untrusted_count: AtomicUsize,
}

impl Default for ManifestCache {
    fn default() -> Self {
        Self::new()
    }
}

impl Clone for ManifestCache {
    fn clone(&self) -> Self {
        let state = self.state.read().expect("manifest cache read lock");
        Self {
            state: RwLock::new(ManifestCacheState {
                manifests: state.manifests.clone(),
                signing_to_master_keys: state.signing_to_master_keys.clone(),
                untrusted_master_keys: state.untrusted_master_keys.clone(),
            }),
            sequence: AtomicU32::new(self.sequence.load(Ordering::Relaxed)),
            max_untrusted_count: AtomicUsize::new(self.max_untrusted_count.load(Ordering::Relaxed)),
        }
    }
}

impl ManifestCache {
    pub fn new() -> Self {
        Self {
            state: RwLock::new(ManifestCacheState::default()),
            sequence: AtomicU32::new(0),
            max_untrusted_count: AtomicUsize::new(MAX_UNTRUSTED_MANIFESTS),
        }
    }

    /// Set the cap for newly admitted untrusted master keys. Existing entries
    /// and uncapped configured, wallet, and validator-list manifests are not
    /// affected.
    pub fn set_max_untrusted_count(&self, max_untrusted_count: usize) {
        self.max_untrusted_count
            .store(max_untrusted_count, Ordering::Relaxed);
    }

    pub fn sequence(&self) -> u32 {
        self.sequence.load(Ordering::Relaxed)
    }

    pub fn get_signing_key(&self, master_key: &PublicKey) -> PublicKey {
        let state = self.state.read().expect("manifest cache read lock");
        state
            .manifests
            .get(master_key)
            .filter(|manifest| !manifest.revoked())
            .and_then(|manifest| manifest.signing_key)
            .unwrap_or(*master_key)
    }

    pub fn get_master_key(&self, signing_key: &PublicKey) -> PublicKey {
        let state = self.state.read().expect("manifest cache read lock");
        state
            .signing_to_master_keys
            .get(signing_key)
            .copied()
            .unwrap_or(*signing_key)
    }

    pub fn get_sequence(&self, master_key: &PublicKey) -> Option<u32> {
        let state = self.state.read().expect("manifest cache read lock");
        state
            .manifests
            .get(master_key)
            .filter(|manifest| !manifest.revoked())
            .map(|manifest| manifest.sequence)
    }

    pub fn get_domain(&self, master_key: &PublicKey) -> Option<String> {
        let state = self.state.read().expect("manifest cache read lock");
        state
            .manifests
            .get(master_key)
            .filter(|manifest| !manifest.revoked())
            .map(|manifest| manifest.domain.clone())
    }

    pub fn get_manifest(&self, master_key: &PublicKey) -> Option<Vec<u8>> {
        let state = self.state.read().expect("manifest cache read lock");
        state
            .manifests
            .get(master_key)
            .filter(|manifest| !manifest.revoked())
            .map(|manifest| manifest.serialized.clone())
    }

    /// Snapshot every current manifest, including revocations. Rippled sends
    /// the cache's complete current manifest set at protocol start.
    pub fn serialized_manifests(&self) -> Vec<Vec<u8>> {
        self.state
            .read()
            .expect("manifest cache read lock")
            .manifests
            .values()
            .map(|manifest| manifest.serialized.clone())
            .collect()
    }

    /// Snapshot all known master keys, including revoked entries. TMManifests
    /// relay policy must compare against the cache state before the message,
    /// not the state after another entry in the same message was admitted.
    pub fn known_master_keys(&self) -> HashSet<PublicKey> {
        self.state
            .read()
            .expect("manifest cache read lock")
            .manifests
            .keys()
            .copied()
            .collect()
    }

    pub fn revoked(&self, master_key: &PublicKey) -> bool {
        let state = self.state.read().expect("manifest cache read lock");
        state
            .manifests
            .get(master_key)
            .is_some_and(Manifest::revoked)
    }

    /// Load persisted manifests from the wallet database. Malformed or stale
    /// rows are ignored just as rippled's ManifestCache only retains accepted
    /// entries; SQLite failures remain startup errors.
    pub fn load_from_wallet(&self, wallet_db: &DatabaseCon, table: &str) -> Result<(), String> {
        let table = manifest_wallet_table(table)?;
        let rows = {
            let connection = wallet_db.get_session();
            let mut statement = connection
                .prepare(&format!("SELECT RawData FROM {table}"))
                .map_err(|error| error.to_string())?;
            let rows = statement
                .query_map([], |row| row.get::<_, Vec<u8>>(0))
                .map_err(|error| error.to_string())?;
            rows.collect::<Result<Vec<_>, _>>()
                .map_err(|error| error.to_string())?
        };
        for raw in rows {
            if let Some(manifest) = deserialize_manifest(&raw) {
                let _ = self.apply_manifest(manifest);
            }
        }
        Ok(())
    }

    /// Replace one persisted manifest table atomically with the cache entries
    /// selected by the caller, matching rippled's shutdown-time filtered save.
    pub fn save_to_wallet(
        &self,
        wallet_db: &DatabaseCon,
        table: &str,
        include: impl Fn(&PublicKey) -> bool,
    ) -> Result<(), String> {
        let table = manifest_wallet_table(table)?;
        let rows = self
            .state
            .read()
            .expect("manifest cache read lock")
            .manifests
            .iter()
            .filter(|(public_key, _)| include(public_key))
            .map(|(_, manifest)| manifest.serialized.clone())
            .collect::<Vec<_>>();
        let mut connection = wallet_db.get_session();
        let transaction = connection
            .transaction()
            .map_err(|error| error.to_string())?;
        transaction
            .execute(&format!("DELETE FROM {table}"), [])
            .map_err(|error| error.to_string())?;
        for raw in rows {
            transaction
                .execute(&format!("INSERT INTO {table} (RawData) VALUES (?1)"), [raw])
                .map_err(|error| error.to_string())?;
        }
        transaction.commit().map_err(|error| error.to_string())
    }

    pub fn apply_manifest(&self, manifest: Manifest) -> ManifestDisposition {
        self.apply_manifest_with_policy(manifest, ManifestRateLimitCapPolicy::Uncapped)
    }

    /// Apply a manifest under rippled's trusted-gossip capacity policy. New
    /// untrusted keys consume one configured slot; configured, wallet-loaded,
    /// and listed keys are uncapped and release any prior untrusted slot.
    pub fn apply_manifest_with_policy(
        &self,
        manifest: Manifest,
        policy: ManifestRateLimitCapPolicy,
    ) -> ManifestDisposition {
        let mut state = self.state.write().expect("manifest cache write lock");
        let is_new = !state.manifests.contains_key(&manifest.master_key);
        if is_new
            && matches!(policy, ManifestRateLimitCapPolicy::Capped)
            && state.untrusted_master_keys.len() >= self.max_untrusted_count.load(Ordering::Relaxed)
        {
            return ManifestDisposition::UntrustedCapacity;
        }

        if let Some(existing) = state.manifests.get(&manifest.master_key)
            && manifest.sequence <= existing.sequence
        {
            return ManifestDisposition::Stale;
        }

        if !manifest.verify() {
            return ManifestDisposition::Invalid;
        }

        let revoked = manifest.revoked();

        if state
            .signing_to_master_keys
            .contains_key(&manifest.master_key)
        {
            return ManifestDisposition::BadMasterKey;
        }

        if !revoked {
            let Some(signing_key) = manifest.signing_key else {
                return ManifestDisposition::Invalid;
            };

            if state.signing_to_master_keys.contains_key(&signing_key) {
                return ManifestDisposition::BadEphemeralKey;
            }

            if state.manifests.contains_key(&signing_key) {
                return ManifestDisposition::BadEphemeralKey;
            }
        }

        if let Some(existing) = state.manifests.get(&manifest.master_key)
            && let Some(old_signing_key) = existing.signing_key
        {
            state.signing_to_master_keys.remove(&old_signing_key);
        }

        if let Some(signing_key) = manifest.signing_key {
            state
                .signing_to_master_keys
                .insert(signing_key, manifest.master_key);
        }

        if is_new && matches!(policy, ManifestRateLimitCapPolicy::Capped) {
            state.untrusted_master_keys.insert(manifest.master_key);
        } else if matches!(policy, ManifestRateLimitCapPolicy::Uncapped) {
            state.untrusted_master_keys.remove(&manifest.master_key);
        }

        state.manifests.insert(manifest.master_key, manifest);
        self.sequence.fetch_add(1, Ordering::Relaxed);
        ManifestDisposition::Accepted
    }

    /// Mark a key trusted after validator-list processing. This deliberately
    /// never re-adds a key when it is later delisted, matching rippled's
    /// permanent promotion behavior.
    pub fn promote_to_trusted(&self, master_key: &PublicKey) {
        self.state
            .write()
            .expect("manifest cache write lock")
            .untrusted_master_keys
            .remove(master_key);
    }
}

fn manifest_wallet_table(table: &str) -> Result<&'static str, String> {
    match table {
        "ValidatorManifests" => Ok("ValidatorManifests"),
        "PublisherManifests" => Ok("PublisherManifests"),
        _ => Err(format!("unsupported manifest wallet table: {table}")),
    }
}

pub fn deserialize_manifest(serialized: &[u8]) -> Option<Manifest> {
    if serialized.is_empty() {
        return None;
    }

    let st = parse_manifest_stobject(serialized)?;

    if st.is_field_present(get_field_by_symbol("sfVersion"))
        && st.get_field_u16(get_field_by_symbol("sfVersion")) != 0
    {
        return None;
    }

    let master_key = PublicKey::from_slice(&protocol::exchange_get::<Vec<u8>>(
        &st,
        get_field_by_symbol("sfPublicKey"),
    )?)
    .ok()?;
    let sequence = st.get_field_u32(get_field_by_symbol("sfSequence"));

    let domain = if let Some(domain_bytes) =
        protocol::exchange_get::<Vec<u8>>(&st, get_field_by_symbol("sfDomain"))
    {
        let domain = String::from_utf8(domain_bytes).ok()?;
        if !is_properly_formed_toml_domain(&domain) {
            return None;
        }
        domain
    } else {
        String::new()
    };

    let has_ephemeral_key = st.is_field_present(get_field_by_symbol("sfSigningPubKey"));
    let has_ephemeral_signature = st.is_field_present(get_field_by_symbol("sfSignature"));

    let signing_key = if Manifest::revoked_sequence(sequence) {
        if has_ephemeral_key || has_ephemeral_signature {
            return None;
        }
        None
    } else {
        if !has_ephemeral_key || !has_ephemeral_signature {
            return None;
        }

        let signing_key = PublicKey::from_slice(&protocol::exchange_get::<Vec<u8>>(
            &st,
            get_field_by_symbol("sfSigningPubKey"),
        )?)
        .ok()?;
        if signing_key == master_key {
            return None;
        }
        Some(signing_key)
    };

    Some(Manifest {
        serialized: serialized.to_vec(),
        master_key,
        signing_key,
        sequence,
        domain,
    })
}

pub fn deserialize_manifest_base64(serialized: &str) -> Option<Manifest> {
    deserialize_manifest(&base64_decode(serialized))
}

pub fn load_validator_token<I, S>(blob: I) -> Option<ValidatorToken>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let mut token_string = String::new();
    for line in blob {
        token_string.push_str(line.as_ref().trim());
    }

    let decoded = base64_decode(&token_string);
    let token: Value = serde_json::from_slice(&decoded).ok()?;
    let manifest = token.get("manifest")?.as_str()?.to_owned();
    let validation_secret_key = token.get("validation_secret_key")?.as_str()?;
    let secret_bytes = str_unhex(validation_secret_key)?;
    let validation_secret = SecretKey::from_slice(&secret_bytes).ok()?;

    Some(ValidatorToken {
        manifest,
        validation_secret,
    })
}

fn parse_manifest_stobject(serialized: &[u8]) -> Option<STObject> {
    catch_unwind(AssertUnwindSafe(|| {
        let mut sit = SerialIter::new(serialized);
        let mut st = STObject::from_serial_iter(&mut sit, sf_generic(), 0);
        if !sit.empty() {
            return None;
        }
        st.apply_template(manifest_template());
        Some(st)
    }))
    .ok()
    .flatten()
}

fn manifest_template() -> &'static SOTemplate {
    static TEMPLATE: OnceLock<SOTemplate> = OnceLock::new();
    TEMPLATE.get_or_init(|| {
        SOTemplate::new(
            vec![
                SOElement::new(get_field_by_symbol("sfPublicKey"), SOEStyle::Required)
                    .expect("manifest sfPublicKey"),
                SOElement::new(get_field_by_symbol("sfMasterSignature"), SOEStyle::Required)
                    .expect("manifest sfMasterSignature"),
                SOElement::new(get_field_by_symbol("sfSequence"), SOEStyle::Required)
                    .expect("manifest sfSequence"),
                SOElement::new(get_field_by_symbol("sfVersion"), SOEStyle::Default)
                    .expect("manifest sfVersion"),
                SOElement::new(get_field_by_symbol("sfDomain"), SOEStyle::Optional)
                    .expect("manifest sfDomain"),
                SOElement::new(get_field_by_symbol("sfSigningPubKey"), SOEStyle::Optional)
                    .expect("manifest sfSigningPubKey"),
                SOElement::new(get_field_by_symbol("sfSignature"), SOEStyle::Optional)
                    .expect("manifest sfSignature"),
            ],
            vec![],
        )
        .expect("manifest template")
    })
}
