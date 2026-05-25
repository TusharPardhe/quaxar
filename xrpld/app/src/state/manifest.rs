#![cfg_attr(test, allow(dead_code))]

use std::collections::HashMap;
use std::fmt;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::OnceLock;
use std::sync::RwLock;
use std::sync::atomic::{AtomicU32, Ordering};

use basics::base_uint::Uint256;
use basics::base64::base64_decode;
use basics::string_utilities::{is_properly_formed_toml_domain, str_unhex};
use protocol::{
    HashPrefix, PublicKey, SOEStyle, SOElement, SOTemplate, STObject, SecretKey, SerialIter,
    get_field_by_symbol, sf_generic, verify_st_object,
};
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ManifestDisposition {
    Accepted = 0,
    Stale,
    BadMasterKey,
    BadEphemeralKey,
    Invalid,
}

#[derive(Debug, Default)]
struct ManifestCacheState {
    manifests: HashMap<PublicKey, Manifest>,
    signing_to_master_keys: HashMap<PublicKey, PublicKey>,
}

#[derive(Debug)]
pub struct ManifestCache {
    state: RwLock<ManifestCacheState>,
    sequence: AtomicU32,
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
            }),
            sequence: AtomicU32::new(self.sequence.load(Ordering::Relaxed)),
        }
    }
}

impl ManifestCache {
    pub fn new() -> Self {
        Self {
            state: RwLock::new(ManifestCacheState::default()),
            sequence: AtomicU32::new(0),
        }
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

    pub fn revoked(&self, master_key: &PublicKey) -> bool {
        let state = self.state.read().expect("manifest cache read lock");
        state
            .manifests
            .get(master_key)
            .is_some_and(Manifest::revoked)
    }

    pub fn apply_manifest(&self, manifest: Manifest) -> ManifestDisposition {
        let mut state = self.state.write().expect("manifest cache write lock");

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

        state.manifests.insert(manifest.master_key, manifest);
        self.sequence.fetch_add(1, Ordering::Relaxed);
        ManifestDisposition::Accepted
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
