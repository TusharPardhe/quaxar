//! Trusted validator list ownership and quorum logic.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::PathBuf;
use std::sync::RwLock;

use basics::chrono::{NetClockTimePoint, to_string};
use basics::string_utilities::str_unhex;
use protocol::{
    AccountID, JsonValue, PUBLIC_KEY_LENGTH, PublicKey, STValidation, calc_account_id,
    parse_base58_node_public, sha512_half_slices, verify,
};
use time::OffsetDateTime;

use crate::consensus::rcl_validations::RclValidationTrustSource;
use crate::state::manifest::{ManifestCache, ManifestDisposition, deserialize_manifest_base64};

pub const MAX_SUPPORTED_BLOBS: usize = 5;
const CACHE_FILE_PREFIX: &str = "cache.";
const RIPPLE_EPOCH_OFFSET: i64 = 946_684_800;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ListDisposition {
    Accepted = 0,
    Expired,
    Pending,
    SameSequence,
    KnownSequence,
    Stale,
    Untrusted,
    UnsupportedVersion,
    Invalid,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum PublisherStatus {
    Available = 0,
    Expired,
    Unavailable,
    Revoked,
}

impl Default for PublisherStatus {
    fn default() -> Self {
        Self::Unavailable
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidatorBlobInfo {
    pub blob: String,
    pub signature: String,
    pub manifest: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PublisherListStats {
    pub dispositions: BTreeMap<ListDisposition, usize>,
    pub publisher_key: Option<PublicKey>,
    pub status: PublisherStatus,
    pub sequence: usize,
}

impl PublisherListStats {
    pub fn new(disposition: ListDisposition) -> Self {
        let mut dispositions = BTreeMap::new();
        dispositions.insert(disposition, 1);
        Self {
            dispositions,
            ..Self::default()
        }
    }

    pub fn with_publisher(
        disposition: ListDisposition,
        publisher_key: PublicKey,
        status: PublisherStatus,
        sequence: usize,
    ) -> Self {
        let mut stats = Self::new(disposition);
        stats.publisher_key = Some(publisher_key);
        stats.status = status;
        stats.sequence = sequence;
        stats
    }

    pub fn best_disposition(&self) -> ListDisposition {
        self.dispositions
            .keys()
            .next()
            .copied()
            .unwrap_or(ListDisposition::Invalid)
    }

    pub fn worst_disposition(&self) -> ListDisposition {
        self.dispositions
            .keys()
            .next_back()
            .copied()
            .unwrap_or(ListDisposition::Invalid)
    }

    pub fn merge(&mut self, other: &PublisherListStats) {
        for (disposition, count) in &other.dispositions {
            *self.dispositions.entry(*disposition).or_default() += count;
        }
        if self.publisher_key.is_none() && other.publisher_key.is_some() {
            self.publisher_key = other.publisher_key;
            self.status = other.status;
            self.sequence = other.sequence;
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValidatorListStatus {
    Active,
    Expired,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValidatorListExpiration {
    Unknown,
    Never,
    Seconds(u32),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidatorListStatusSnapshot {
    pub count: usize,
    pub status: ValidatorListStatus,
    pub expiration: ValidatorListExpiration,
    pub validator_list_threshold: usize,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TrustChanges {
    pub added: BTreeSet<AccountID>,
    pub removed: BTreeSet<AccountID>,
}

pub trait ValidatorListClock: Send + Sync + 'static {
    fn now_ripple(&self) -> u32;
}

#[derive(Debug, Clone, Copy, Default)]
pub struct SystemValidatorListClock;

impl ValidatorListClock for SystemValidatorListClock {
    fn now_ripple(&self) -> u32 {
        let unix = OffsetDateTime::now_utc().unix_timestamp();
        u32::try_from(unix.saturating_sub(RIPPLE_EPOCH_OFFSET)).unwrap_or_default()
    }
}

#[derive(Clone, Debug)]
pub struct ValidatorList<C = SystemValidatorListClock> {
    validator_manifests: ManifestCache,
    publisher_manifests: ManifestCache,
    clock: C,
    state: std::sync::Arc<RwLock<ValidatorListState>>,
}

impl<C: ValidatorListClock> ValidatorList<C> {
    pub fn new(
        validator_manifests: ManifestCache,
        publisher_manifests: ManifestCache,
        clock: C,
        data_path: impl Into<PathBuf>,
        minimum_quorum: Option<usize>,
    ) -> Self {
        Self {
            validator_manifests,
            publisher_manifests,
            clock,
            state: std::sync::Arc::new(RwLock::new(ValidatorListState {
                quorum: minimum_quorum.unwrap_or(1),
                minimum_quorum,
                data_path: data_path.into(),
                ..ValidatorListState::default()
            })),
        }
    }

    pub fn load(
        &self,
        local_signing_key: Option<PublicKey>,
        config_keys: &[String],
        publisher_keys: &[String],
        list_threshold: Option<usize>,
    ) -> bool {
        let mut state = self.state.write().expect("validator list write lock");
        for key in publisher_keys {
            let Some(blob) = str_unhex(key) else {
                return false;
            };
            if blob.len() != PUBLIC_KEY_LENGTH {
                return false;
            }
            let Ok(public_key) = PublicKey::from_slice(&blob) else {
                return false;
            };
            if state.publisher_lists.contains_key(&public_key) {
                continue;
            }
            let status = if self.publisher_manifests.revoked(&public_key) {
                PublisherStatus::Revoked
            } else {
                PublisherStatus::Unavailable
            };
            state.publisher_lists.insert(
                public_key,
                PublisherListCollection {
                    status,
                    ..PublisherListCollection::default()
                },
            );
        }

        state.list_threshold = match list_threshold {
            Some(list_threshold) => list_threshold,
            None if state.publisher_lists.len() < 3 => 1,
            None => (state.publisher_lists.len() / 2) + 1,
        };
        let threshold = state.list_threshold;

        if let Some(local_signing_key) = local_signing_key {
            state.local_pub_key = Some(self.validator_manifests.get_master_key(&local_signing_key));
        }
        if let Some(local_pub_key) = state.local_pub_key {
            state.key_listings.insert(local_pub_key, threshold);
        }

        for entry in config_keys {
            let Some(key) = entry.split_whitespace().next() else {
                return false;
            };
            let Some(node_public) = parse_base58_node_public(key) else {
                return false;
            };
            let public_key = PublicKey::from_bytes(node_public);
            if Some(public_key) == state.local_pub_key || Some(public_key) == local_signing_key {
                continue;
            }
            if state.key_listings.contains_key(&public_key) {
                continue;
            }
            state.key_listings.insert(public_key, threshold);
            state.local_publisher_list.list.push(public_key);
        }
        if !state.local_publisher_list.list.is_empty() {
            state.local_publisher_list.valid_until = u32::MAX;
        }
        true
    }

    pub fn parse_blobs(version: u32, body: &serde_json::Value) -> Vec<ValidatorBlobInfo> {
        match version {
            1 => {
                let Some(blob) = body.get("blob").and_then(serde_json::Value::as_str) else {
                    return Vec::new();
                };
                let Some(signature) = body.get("signature").and_then(serde_json::Value::as_str)
                else {
                    return Vec::new();
                };
                if body.get("blobs_v2").is_some() {
                    return Vec::new();
                }
                vec![ValidatorBlobInfo {
                    blob: blob.to_owned(),
                    signature: signature.to_owned(),
                    manifest: None,
                }]
            }
            _ => {
                let Some(entries) = body.get("blobs_v2").and_then(serde_json::Value::as_array)
                else {
                    return Vec::new();
                };
                if entries.len() > MAX_SUPPORTED_BLOBS
                    || body.get("blob").is_some()
                    || body.get("signature").is_some()
                {
                    return Vec::new();
                }
                let mut result = Vec::with_capacity(entries.len());
                for entry in entries {
                    let Some(blob) = entry.get("blob").and_then(serde_json::Value::as_str) else {
                        return Vec::new();
                    };
                    let Some(signature) =
                        entry.get("signature").and_then(serde_json::Value::as_str)
                    else {
                        return Vec::new();
                    };
                    let manifest = match entry.get("manifest") {
                        Some(serde_json::Value::String(text)) => Some(text.clone()),
                        Some(_) => return Vec::new(),
                        None => None,
                    };
                    result.push(ValidatorBlobInfo {
                        blob: blob.to_owned(),
                        signature: signature.to_owned(),
                        manifest,
                    });
                }
                result
            }
        }
    }

    pub fn apply_lists(
        &self,
        manifest: &str,
        version: u32,
        blobs: &[ValidatorBlobInfo],
        site_uri: String,
        hash: Option<basics::base_uint::Uint256>,
    ) -> PublisherListStats {
        if !matches!(version, 1 | 2) {
            let mut result = PublisherListStats::default();
            result
                .dispositions
                .insert(ListDisposition::UnsupportedVersion, blobs.len().max(1));
            return result;
        }

        let mut aggregate = PublisherListStats::default();
        let mut touched_publishers = BTreeSet::new();
        let mut state = self.state.write().expect("validator list write lock");
        for blob in blobs {
            let result = self.apply_list_locked(
                &mut state,
                manifest,
                blob.manifest.as_deref(),
                &blob.blob,
                &blob.signature,
                version,
                site_uri.clone(),
                hash,
            );
            if let Some(publisher_key) = result.publisher_key {
                touched_publishers.insert(publisher_key);
            }
            aggregate.merge(&result);
        }
        for publisher_key in touched_publishers {
            clean_publisher_collection(
                state
                    .publisher_lists
                    .get_mut(&publisher_key)
                    .expect("publisher"),
            );
        }
        aggregate
    }

    pub fn update_trusted(
        &self,
        seen_validators: &HashSet<AccountID>,
        close_time: u32,
    ) -> TrustChanges {
        let mut state = self.state.write().expect("validator list write lock");
        let publisher_keys = state.publisher_lists.keys().copied().collect::<Vec<_>>();
        for publisher_key in publisher_keys {
            let mut collection = state
                .publisher_lists
                .remove(&publisher_key)
                .expect("publisher collection");
            rotate_pending_collection(
                &self.validator_manifests,
                &mut collection,
                close_time,
                &mut state.key_listings,
            );
            if collection.status == PublisherStatus::Available
                && collection.current.valid_until <= close_time
            {
                remove_current_list(
                    &mut collection,
                    &mut state.key_listings,
                    PublisherStatus::Expired,
                );
            }
            state.publisher_lists.insert(publisher_key, collection);
        }

        let mut changes = TrustChanges::default();
        let mut trusted_master_keys = state.trusted_master_keys.clone();
        trusted_master_keys.retain(|public_key| {
            let keep = state
                .key_listings
                .get(public_key)
                .is_some_and(|count| *count >= state.list_threshold)
                && !self.validator_manifests.revoked(public_key);
            if !keep {
                changes
                    .added
                    .remove(&calc_account_id(public_key.as_bytes()));
                changes
                    .removed
                    .insert(calc_account_id(public_key.as_bytes()));
            }
            keep
        });

        for (public_key, count) in &state.key_listings {
            if *count >= state.list_threshold
                && !self.validator_manifests.revoked(public_key)
                && trusted_master_keys.insert(*public_key)
            {
                changes.added.insert(calc_account_id(public_key.as_bytes()));
                changes
                    .removed
                    .remove(&calc_account_id(public_key.as_bytes()));
            }
        }

        state.trusted_master_keys = trusted_master_keys;
        state.trusted_signing_keys.clear();
        let trusted_master_keys = state
            .trusted_master_keys
            .iter()
            .copied()
            .collect::<Vec<_>>();
        for public_key in trusted_master_keys {
            let signing_key = self.validator_manifests.get_signing_key(&public_key);
            state.trusted_signing_keys.insert(signing_key);
        }

        let unl_size = state.trusted_master_keys.len();
        let mut effective_unl_size = unl_size;
        let mut seen_size = seen_validators.len();
        for public_key in &state.trusted_master_keys {
            if state.negative_unl.contains(public_key) && effective_unl_size > 0 {
                effective_unl_size -= 1;
            }
        }
        for negative in &state.negative_unl {
            let node_id = calc_account_id(negative.as_bytes());
            if seen_validators.contains(&node_id) && seen_size > 0 {
                seen_size -= 1;
            }
        }
        state.quorum = calculate_quorum(
            state.minimum_quorum,
            state.publisher_lists.len(),
            state.list_threshold,
            state
                .publisher_lists
                .values()
                .filter(|collection| collection.status != PublisherStatus::Available)
                .count(),
            unl_size,
            effective_unl_size,
            seen_size,
        );
        changes
    }

    pub fn quorum(&self) -> usize {
        self.state.read().expect("validator list read lock").quorum
    }

    pub fn unl_size(&self) -> usize {
        self.state.read().expect("validator list read lock").trusted_master_keys.len()
    }

    pub fn trusted(&self, identity: PublicKey) -> bool {
        let state = self.state.read().expect("validator list read lock");
        let public_key = self.validator_manifests.get_master_key(&identity);
        state.trusted_master_keys.contains(&public_key)
    }

    pub fn listed(&self, identity: PublicKey) -> bool {
        let state = self.state.read().expect("validator list read lock");
        let public_key = self.validator_manifests.get_master_key(&identity);
        state.key_listings.contains_key(&public_key)
    }

    pub fn get_trusted_key(&self, identity: PublicKey) -> Option<PublicKey> {
        let state = self.state.read().expect("validator list read lock");
        let public_key = self.validator_manifests.get_master_key(&identity);
        state
            .trusted_master_keys
            .contains(&public_key)
            .then_some(public_key)
    }

    pub fn negative_unl_filter_validations(
        &self,
        validations: Vec<STValidation>,
    ) -> Vec<STValidation> {
        let state = self.state.read().expect("validator list read lock");
        if state.negative_unl.is_empty() {
            return validations;
        }

        validations
            .into_iter()
            .filter(|validation| {
                let public_key = self
                    .validator_manifests
                    .get_master_key(validation.get_signer_public());
                !state.negative_unl.contains(&public_key)
            })
            .collect()
    }

    pub fn master_key(&self, identity: PublicKey) -> PublicKey {
        self.validator_manifests.get_master_key(&identity)
    }

    pub fn get_listed_key(&self, identity: PublicKey) -> Option<PublicKey> {
        let state = self.state.read().expect("validator list read lock");
        let public_key = self.validator_manifests.get_master_key(&identity);
        state
            .key_listings
            .contains_key(&public_key)
            .then_some(public_key)
    }

    pub fn trusted_publisher(&self, identity: PublicKey) -> bool {
        self.state
            .read()
            .expect("validator list read lock")
            .publisher_lists
            .get(&identity)
            .is_some_and(|collection| collection.status < PublisherStatus::Revoked)
    }

    pub fn local_public_key(&self) -> Option<PublicKey> {
        self.state
            .read()
            .expect("validator list read lock")
            .local_pub_key
    }

    pub fn for_each_listed(&self, mut visitor: impl FnMut(PublicKey, bool)) {
        let state = self.state.read().expect("validator list read lock");
        for public_key in state.key_listings.keys() {
            visitor(*public_key, state.trusted_master_keys.contains(public_key));
        }
    }

    pub fn get_available(
        &self,
        publisher_key_hex: &str,
        force_version: Option<u32>,
    ) -> Option<serde_json::Value> {
        let public_key = PublicKey::from_slice(&str_unhex(publisher_key_hex)?).ok()?;
        let state = self.state.read().expect("validator list read lock");
        let collection = state.publisher_lists.get(&public_key)?;
        (collection.status == PublisherStatus::Available)
            .then(|| build_file_data(publisher_key_hex.to_owned(), collection, force_version))
    }

    pub fn count(&self) -> usize {
        let state = self.state.read().expect("validator list read lock");
        state.publisher_lists.len() + usize::from(!state.local_publisher_list.list.is_empty())
    }

    pub fn expires(&self) -> Option<u32> {
        let state = self.state.read().expect("validator list read lock");
        expires_locked(&state)
    }

    pub fn load_lists(&self) -> Vec<String> {
        let state = self.state.read().expect("validator list read lock");
        let mut sites = Vec::new();
        for (public_key, collection) in &state.publisher_lists {
            if collection.status == PublisherStatus::Available {
                continue;
            }
            let filename = state
                .data_path
                .join(format!("{CACHE_FILE_PREFIX}{}", public_key));
            if filename.is_file() {
                sites.push(format!("file://{}", filename.display()));
            }
        }
        sites
    }

    pub fn get_trusted_master_keys(&self) -> HashSet<PublicKey> {
        self.state
            .read()
            .expect("validator list read lock")
            .trusted_master_keys
            .clone()
    }

    pub fn get_quorum_keys(&self) -> (usize, HashSet<PublicKey>) {
        let state = self.state.read().expect("validator list read lock");
        (state.quorum, state.trusted_signing_keys.clone())
    }

    pub fn get_list_threshold(&self) -> usize {
        self.state
            .read()
            .expect("validator list read lock")
            .list_threshold
    }

    pub fn set_negative_unl(&self, negative_unl: HashSet<PublicKey>) {
        self.state
            .write()
            .expect("validator list write lock")
            .negative_unl = negative_unl;
    }

    pub fn get_negative_unl(&self) -> HashSet<PublicKey> {
        self.state
            .read()
            .expect("validator list read lock")
            .negative_unl
            .clone()
    }

    pub fn get_json(&self) -> JsonValue {
        let state = self.state.read().expect("validator list read lock");
        let mut result = BTreeMap::new();
        let status_snapshot = self.status_snapshot_locked(&state);
        result.insert(
            "list_threshold".to_owned(),
            JsonValue::Unsigned(state.list_threshold as u64),
        );
        result.insert(
            "quorum".to_owned(),
            JsonValue::Unsigned(state.quorum as u64),
        );
        result.insert(
            "validation_quorum".to_owned(),
            JsonValue::Unsigned(state.quorum as u64),
        );
        result.insert(
            "validator_list".to_owned(),
            JsonValue::Object(BTreeMap::from([
                (
                    "count".to_owned(),
                    JsonValue::Unsigned(status_snapshot.count as u64),
                ),
                (
                    "expiration".to_owned(),
                    match status_snapshot.expiration {
                        ValidatorListExpiration::Never => JsonValue::String("never".to_owned()),
                        ValidatorListExpiration::Seconds(seconds) => {
                            JsonValue::String(to_string(NetClockTimePoint::new(seconds)))
                        }
                        ValidatorListExpiration::Unknown => JsonValue::String("unknown".to_owned()),
                    },
                ),
                (
                    "status".to_owned(),
                    match status_snapshot.status {
                        ValidatorListStatus::Active => JsonValue::String("active".to_owned()),
                        ValidatorListStatus::Expired => JsonValue::String("expired".to_owned()),
                        ValidatorListStatus::Unknown => JsonValue::String("unknown".to_owned()),
                    },
                ),
                (
                    "validator_list_threshold".to_owned(),
                    JsonValue::Unsigned(status_snapshot.validator_list_threshold as u64),
                ),
            ])),
        );
        result.insert(
            "local_static_keys".to_owned(),
            JsonValue::Array(
                state
                    .local_publisher_list
                    .list
                    .iter()
                    .map(|public_key| JsonValue::String(public_key.to_node_public_base58()))
                    .collect(),
            ),
        );
        result.insert(
            "trusted_validator_keys".to_owned(),
            JsonValue::Array(
                state
                    .trusted_master_keys
                    .iter()
                    .map(|public_key| JsonValue::String(public_key.to_node_public_base58()))
                    .collect(),
            ),
        );
        JsonValue::Object(result)
    }

    pub fn status_snapshot(&self) -> ValidatorListStatusSnapshot {
        let state = self.state.read().expect("validator list read lock");
        self.status_snapshot_locked(&state)
    }

    fn status_snapshot_locked(&self, state: &ValidatorListState) -> ValidatorListStatusSnapshot {
        let expiration = expires_locked(state);
        let status = match expiration {
            Some(u32::MAX) => ValidatorListStatus::Active,
            Some(seconds) if seconds > self.clock.now_ripple() => ValidatorListStatus::Active,
            Some(_) => ValidatorListStatus::Expired,
            None => ValidatorListStatus::Unknown,
        };

        ValidatorListStatusSnapshot {
            count: state.publisher_lists.len()
                + usize::from(!state.local_publisher_list.list.is_empty()),
            status,
            expiration: match expiration {
                Some(u32::MAX) => ValidatorListExpiration::Never,
                Some(seconds) => ValidatorListExpiration::Seconds(seconds),
                None => ValidatorListExpiration::Unknown,
            },
            validator_list_threshold: state.list_threshold,
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn apply_list_locked(
        &self,
        state: &mut ValidatorListState,
        global_manifest: &str,
        local_manifest: Option<&str>,
        blob: &str,
        signature: &str,
        version: u32,
        site_uri: String,
        hash: Option<basics::base_uint::Uint256>,
    ) -> PublisherListStats {
        let manifest = local_manifest.unwrap_or(global_manifest);
        let Some(manifest) = deserialize_manifest_base64(manifest) else {
            return PublisherListStats::new(ListDisposition::Invalid);
        };
        let (result, publisher_key, list_json) =
            self.verify_locked(state, manifest, blob, signature);
        let Some(publisher_key) = publisher_key else {
            return PublisherListStats::new(result);
        };
        if result > ListDisposition::Pending {
            if let Some(collection) = state.publisher_lists.get(&publisher_key)
                && let Some(max_sequence) = collection.max_sequence
                && matches!(
                    result,
                    ListDisposition::SameSequence | ListDisposition::KnownSequence
                )
            {
                return PublisherListStats::with_publisher(
                    result,
                    publisher_key,
                    collection.status,
                    max_sequence,
                );
            }
            return PublisherListStats::with_publisher(
                result,
                publisher_key,
                state
                    .publisher_lists
                    .get(&publisher_key)
                    .map_or(PublisherStatus::Unavailable, |collection| collection.status),
                0,
            );
        }

        let sequence = list_json
            .get("sequence")
            .and_then(serde_json::Value::as_u64)
            .expect("verified list sequence") as usize;
        let accepted = matches!(result, ListDisposition::Accepted | ListDisposition::Expired);
        let collection = state.publisher_lists.entry(publisher_key).or_default();

        if accepted {
            collection.status = if result == ListDisposition::Accepted {
                PublisherStatus::Available
            } else {
                PublisherStatus::Expired
            };
        }
        collection.raw_manifest = global_manifest.to_owned();
        collection.max_sequence = Some(
            collection
                .max_sequence
                .map_or(sequence, |max| max.max(sequence)),
        );

        let old_list = if accepted && collection.remaining.contains_key(&sequence) {
            let old_list = std::mem::take(&mut collection.current.list);
            collection.current = collection
                .remaining
                .remove(&sequence)
                .expect("pending list should exist");
            old_list
        } else {
            let publisher = if accepted {
                &mut collection.current
            } else {
                collection.remaining.entry(sequence).or_default()
            };
            let old_list = std::mem::take(&mut publisher.list);
            publisher.sequence = sequence;
            publisher.valid_from = list_json
                .get("effective")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(0) as u32;
            publisher.valid_until = list_json
                .get("expiration")
                .and_then(serde_json::Value::as_u64)
                .expect("verified expiration") as u32;
            publisher.site_uri = site_uri;
            publisher.raw_blob = blob.to_owned();
            publisher.raw_signature = signature.to_owned();
            publisher.raw_manifest = local_manifest.map(ToOwned::to_owned);
            publisher.hash = hash;
            publisher.manifests.clear();
            if let Some(validators) = list_json
                .get("validators")
                .and_then(serde_json::Value::as_array)
            {
                for validator in validators {
                    let Some(key_hex) = validator
                        .get("validation_public_key")
                        .and_then(serde_json::Value::as_str)
                    else {
                        continue;
                    };
                    let Some(public_key_bytes) = str_unhex(key_hex) else {
                        continue;
                    };
                    if public_key_bytes.len() != PUBLIC_KEY_LENGTH {
                        continue;
                    }
                    let Ok(public_key) = PublicKey::from_slice(&public_key_bytes) else {
                        continue;
                    };
                    publisher.list.push(public_key);
                    if let Some(manifest) = validator
                        .get("manifest")
                        .and_then(serde_json::Value::as_str)
                    {
                        publisher.manifests.push(manifest.to_owned());
                    }
                }
                publisher.list.sort();
            }
            old_list
        };

        collection.raw_version = collection.raw_version.max(version);
        if !collection.remaining.is_empty() {
            collection.raw_version = collection.raw_version.max(2);
        }

        if accepted {
            update_publisher_list(
                &self.validator_manifests,
                &mut state.key_listings,
                publisher_key,
                &collection.current,
                &old_list,
            );
        }

        PublisherListStats::with_publisher(
            result,
            publisher_key,
            collection.status,
            collection.max_sequence.unwrap_or(sequence),
        )
    }

    fn verify_locked(
        &self,
        state: &mut ValidatorListState,
        manifest: crate::state::manifest::Manifest,
        blob: &str,
        signature: &str,
    ) -> (ListDisposition, Option<PublicKey>, serde_json::Value) {
        if !state.publisher_lists.contains_key(&manifest.master_key) {
            return (ListDisposition::Untrusted, None, serde_json::Value::Null);
        }

        let master_public_key = manifest.master_key;
        let revoked = manifest.revoked();
        let manifest_result = self.publisher_manifests.apply_manifest(manifest);
        if revoked
            && manifest_result == ManifestDisposition::Accepted
            && let Some(collection) = state.publisher_lists.get_mut(&master_public_key)
        {
            remove_current_list(
                collection,
                &mut state.key_listings,
                PublisherStatus::Revoked,
            );
            collection.remaining.clear();
        }

        let signing_key = self.publisher_manifests.get_signing_key(&master_public_key);
        if revoked || manifest_result == ManifestDisposition::Invalid {
            return (
                ListDisposition::Untrusted,
                Some(master_public_key),
                serde_json::Value::Null,
            );
        }

        let Some(sig) = str_unhex(signature) else {
            return (
                ListDisposition::Invalid,
                Some(master_public_key),
                serde_json::Value::Null,
            );
        };
        let data = basics::base64::base64_decode(blob);
        if !verify(&signing_key, &data, &sig) {
            return (
                ListDisposition::Invalid,
                Some(master_public_key),
                serde_json::Value::Null,
            );
        }

        let Ok(list) = serde_json::from_slice::<serde_json::Value>(&data) else {
            return (
                ListDisposition::Invalid,
                Some(master_public_key),
                serde_json::Value::Null,
            );
        };
        let Some(sequence) = list.get("sequence").and_then(serde_json::Value::as_u64) else {
            return (ListDisposition::Invalid, Some(master_public_key), list);
        };
        let Some(valid_until) = list.get("expiration").and_then(serde_json::Value::as_u64) else {
            return (ListDisposition::Invalid, Some(master_public_key), list);
        };
        let valid_from = list
            .get("effective")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0);
        if valid_until <= valid_from
            || !list
                .get("validators")
                .is_some_and(serde_json::Value::is_array)
        {
            return (ListDisposition::Invalid, Some(master_public_key), list);
        }

        let collection = state
            .publisher_lists
            .get(&master_public_key)
            .expect("trusted publisher collection");
        if sequence < collection.current.sequence as u64 {
            return (ListDisposition::Stale, Some(master_public_key), list);
        }
        if sequence == collection.current.sequence as u64 {
            return (ListDisposition::SameSequence, Some(master_public_key), list);
        }

        let now = self.clock.now_ripple() as u64;
        if valid_until <= now {
            return (ListDisposition::Expired, Some(master_public_key), list);
        }
        if valid_from > now {
            let disposition = match collection.max_sequence {
                None => ListDisposition::Pending,
                Some(max_sequence) if sequence as usize > max_sequence => ListDisposition::Pending,
                Some(max_sequence) => {
                    let future = collection.remaining.get(&max_sequence);
                    if !collection.remaining.contains_key(&(sequence as usize))
                        && future.is_some_and(|future| valid_from < u64::from(future.valid_from))
                    {
                        ListDisposition::Pending
                    } else {
                        ListDisposition::KnownSequence
                    }
                }
            };
            return (disposition, Some(master_public_key), list);
        }

        (ListDisposition::Accepted, Some(master_public_key), list)
    }
}

impl<C: ValidatorListClock> RclValidationTrustSource for ValidatorList<C> {
    fn get_trusted_key(&self, identity: &PublicKey) -> Option<PublicKey> {
        Self::get_trusted_key(self, *identity)
    }

    fn get_listed_key(&self, identity: &PublicKey) -> Option<PublicKey> {
        Self::get_listed_key(self, *identity)
    }
}

impl<C: ValidatorListClock> RclValidationTrustSource for std::sync::Arc<ValidatorList<C>> {
    fn get_trusted_key(&self, identity: &PublicKey) -> Option<PublicKey> {
        self.as_ref().get_trusted_key(*identity)
    }

    fn get_listed_key(&self, identity: &PublicKey) -> Option<PublicKey> {
        self.as_ref().get_listed_key(*identity)
    }
}

#[derive(Debug, Clone, Default)]
struct PublisherList {
    list: Vec<PublicKey>,
    manifests: Vec<String>,
    sequence: usize,
    valid_from: u32,
    valid_until: u32,
    site_uri: String,
    raw_blob: String,
    raw_signature: String,
    raw_manifest: Option<String>,
    hash: Option<basics::base_uint::Uint256>,
}

#[derive(Debug, Clone, Default)]
struct PublisherListCollection {
    status: PublisherStatus,
    current: PublisherList,
    remaining: BTreeMap<usize, PublisherList>,
    max_sequence: Option<usize>,
    raw_manifest: String,
    raw_version: u32,
}

#[derive(Debug, Default)]
struct ValidatorListState {
    quorum: usize,
    minimum_quorum: Option<usize>,
    publisher_lists: HashMap<PublicKey, PublisherListCollection>,
    key_listings: HashMap<PublicKey, usize>,
    trusted_master_keys: HashSet<PublicKey>,
    list_threshold: usize,
    trusted_signing_keys: HashSet<PublicKey>,
    local_pub_key: Option<PublicKey>,
    local_publisher_list: PublisherList,
    negative_unl: HashSet<PublicKey>,
    data_path: PathBuf,
}

fn update_publisher_list(
    validator_manifests: &ManifestCache,
    key_listings: &mut HashMap<PublicKey, usize>,
    _publisher_key: PublicKey,
    current: &PublisherList,
    old_list: &[PublicKey],
) {
    let mut new_iter = current.list.iter().peekable();
    let mut old_iter = old_list.iter().peekable();
    while new_iter.peek().is_some() || old_iter.peek().is_some() {
        match (new_iter.peek(), old_iter.peek()) {
            (Some(new_key), Some(old_key)) if new_key == old_key => {
                new_iter.next();
                old_iter.next();
            }
            (Some(new_key), Some(old_key)) if new_key < old_key => {
                *key_listings.entry(**new_key).or_default() += 1;
                new_iter.next();
            }
            (Some(_), Some(old_key)) => {
                decrement_listing(key_listings, **old_key);
                old_iter.next();
            }
            (Some(new_key), None) => {
                *key_listings.entry(**new_key).or_default() += 1;
                new_iter.next();
            }
            (None, Some(old_key)) => {
                decrement_listing(key_listings, **old_key);
                old_iter.next();
            }
            (None, None) => break,
        }
    }

    for manifest in &current.manifests {
        let Some(manifest) = deserialize_manifest_base64(manifest) else {
            continue;
        };
        if !key_listings.contains_key(&manifest.master_key) {
            continue;
        }
        let _ = validator_manifests.apply_manifest(manifest);
    }
}

fn decrement_listing(key_listings: &mut HashMap<PublicKey, usize>, public_key: PublicKey) {
    match key_listings.get_mut(&public_key) {
        Some(count) if *count <= 1 => {
            key_listings.remove(&public_key);
        }
        Some(count) => {
            *count -= 1;
        }
        None => {}
    }
}

fn clean_publisher_collection(collection: &mut PublisherListCollection) {
    collection
        .remaining
        .retain(|sequence, _pending| *sequence > collection.current.sequence);
    loop {
        let mut remove = None;
        let mut iter = collection.remaining.iter();
        let Some((mut previous_sequence, mut previous)) = iter.next() else {
            break;
        };
        for (sequence, current) in iter {
            if current.valid_from <= previous.valid_from {
                remove = Some(*previous_sequence);
            }
            previous_sequence = sequence;
            previous = current;
        }
        match remove {
            Some(sequence) => {
                collection.remaining.remove(&sequence);
            }
            None => break,
        }
    }
}

fn rotate_pending_collection(
    validator_manifests: &ManifestCache,
    collection: &mut PublisherListCollection,
    close_time: u32,
    key_listings: &mut HashMap<PublicKey, usize>,
) {
    let ready_sequences = collection
        .remaining
        .iter()
        .take_while(|(_, pending)| pending.valid_from <= close_time)
        .map(|(sequence, _)| *sequence)
        .collect::<Vec<_>>();
    let Some(sequence) = ready_sequences.last().copied() else {
        return;
    };

    let old_list = collection.current.list.clone();
    collection.current = collection
        .remaining
        .remove(&sequence)
        .expect("ready pending list should exist");
    if collection.current.valid_until <= close_time {
        collection.current.list.clear();
    }
    collection.status = PublisherStatus::Available;
    update_publisher_list(
        validator_manifests,
        key_listings,
        PublicKey::from_bytes([0; PUBLIC_KEY_LENGTH]),
        &collection.current,
        &old_list,
    );
    for skipped in ready_sequences
        .into_iter()
        .filter(|value| *value != sequence)
    {
        collection.remaining.remove(&skipped);
    }
}

fn remove_current_list(
    collection: &mut PublisherListCollection,
    key_listings: &mut HashMap<PublicKey, usize>,
    status: PublisherStatus,
) {
    for public_key in &collection.current.list {
        decrement_listing(key_listings, *public_key);
    }
    collection.current.list.clear();
    collection.status = status;
}

fn calculate_quorum(
    minimum_quorum: Option<usize>,
    publisher_count: usize,
    list_threshold: usize,
    unavailable_publishers: usize,
    unl_size: usize,
    effective_unl_size: usize,
    _seen_size: usize,
) -> usize {
    if let Some(minimum_quorum) = minimum_quorum {
        return minimum_quorum;
    }
    if publisher_count > 0 {
        let error_threshold =
            list_threshold.min(publisher_count.saturating_sub(list_threshold) + 1);
        if unavailable_publishers >= error_threshold {
            return usize::MAX;
        }
    }
    ((effective_unl_size * 8).div_ceil(10)).max((unl_size * 6).div_ceil(10))
}

fn build_file_data(
    publisher_key: String,
    collection: &PublisherListCollection,
    force_version: Option<u32>,
) -> serde_json::Value {
    let effective_version = force_version.unwrap_or(collection.raw_version);
    let mut value = serde_json::json!({
        "manifest": collection.raw_manifest,
        "version": effective_version,
        "public_key": publisher_key,
    });
    match effective_version {
        1 => {
            value["blob"] = serde_json::Value::String(collection.current.raw_blob.clone());
            value["signature"] =
                serde_json::Value::String(collection.current.raw_signature.clone());
            if let Some(local_manifest) = &collection.current.raw_manifest
                && local_manifest != &collection.raw_manifest
            {
                value["manifest"] = serde_json::Value::String(local_manifest.clone());
            }
        }
        _ => {
            let mut blobs = Vec::new();
            let mut push = |publisher: &PublisherList| {
                let mut entry = serde_json::json!({
                    "blob": publisher.raw_blob,
                    "signature": publisher.raw_signature,
                });
                if let Some(local_manifest) = &publisher.raw_manifest
                    && local_manifest != &collection.raw_manifest
                {
                    entry["manifest"] = serde_json::Value::String(local_manifest.clone());
                }
                blobs.push(entry);
            };
            push(&collection.current);
            for publisher in collection.remaining.values() {
                push(publisher);
            }
            value["blobs_v2"] = serde_json::Value::Array(blobs);
        }
    }
    value
}

fn expires_locked(state: &ValidatorListState) -> Option<u32> {
    let mut result = None;
    for collection in state.publisher_lists.values() {
        if collection.current.valid_until == 0 {
            return None;
        }

        let mut chained_expiration = collection.current.valid_until;
        for pending in collection.remaining.values() {
            if pending.valid_from <= chained_expiration {
                chained_expiration = pending.valid_until;
            } else {
                break;
            }
        }

        if result.is_none_or(|current| chained_expiration < current) {
            result = Some(chained_expiration);
        }
    }

    if !state.local_publisher_list.list.is_empty() {
        let chained_expiration = state.local_publisher_list.valid_until;
        if result.is_none_or(|current| chained_expiration < current) {
            result = Some(chained_expiration);
        }
    }

    result
}

pub fn validator_list_collection_hash(
    manifest: &str,
    version: u32,
    blobs: &[ValidatorBlobInfo],
) -> basics::base_uint::Uint256 {
    let mut parts = Vec::with_capacity(2 + blobs.len() * 3);
    let version_bytes = version.to_be_bytes();
    parts.push(manifest.as_bytes());
    parts.push(&version_bytes);
    for blob in blobs {
        parts.push(blob.blob.as_bytes());
        parts.push(blob.signature.as_bytes());
        if let Some(local_manifest) = &blob.manifest {
            parts.push(local_manifest.as_bytes());
        }
    }
    sha512_half_slices(&parts)
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;
    use std::path::Path;

    use basics::base64::base64_encode;
    use protocol::{
        HashPrefix, KeyType, STObject, SecretKey, StBase, calc_account_id, derive_public_key,
        get_field_by_symbol, sign,
    };

    use crate::state::manifest::ManifestCache;

    use super::{
        ListDisposition, PublisherStatus, SystemValidatorListClock, ValidatorBlobInfo,
        ValidatorList, ValidatorListClock, ValidatorListExpiration, ValidatorListStatus,
        validator_list_collection_hash,
    };
    use protocol::JsonValue;

    fn manifest_blob(
        master_secret: &SecretKey,
        signing_secret: &SecretKey,
        sequence: u32,
    ) -> (String, protocol::PublicKey, protocol::PublicKey) {
        let master_public = derive_public_key(KeyType::Ed25519, master_secret).expect("master");
        let signing_public =
            derive_public_key(KeyType::Secp256k1, signing_secret).expect("signing");
        let mut object = STObject::new(protocol::sf_generic());
        object.set_field_u32(get_field_by_symbol("sfSequence"), sequence);
        object.set_field_vl(get_field_by_symbol("sfPublicKey"), master_public.as_bytes());
        object.set_field_vl(
            get_field_by_symbol("sfSigningPubKey"),
            signing_public.as_bytes(),
        );
        set_manifest_signature(
            &mut object,
            &signing_public,
            signing_secret,
            get_field_by_symbol("sfSignature"),
        );
        set_manifest_signature(
            &mut object,
            &master_public,
            master_secret,
            get_field_by_symbol("sfMasterSignature"),
        );
        let mut serializer = protocol::Serializer::default();
        object.add(&mut serializer);
        (
            base64_encode(serializer.data()),
            master_public,
            signing_public,
        )
    }

    fn set_manifest_signature(
        object: &mut STObject,
        public_key: &protocol::PublicKey,
        secret_key: &SecretKey,
        field: &'static protocol::SField,
    ) {
        let mut serializer = protocol::Serializer::default();
        serializer.add32_prefix(HashPrefix::Manifest);
        object.add_without_signing_fields(&mut serializer);
        let signature =
            sign(public_key, secret_key, serializer.data()).expect("manifest signature");
        object.set_field_vl(field, &signature);
    }

    fn signed_list(
        blob: &str,
        signing_public: protocol::PublicKey,
        signing_secret: &SecretKey,
    ) -> String {
        basics::str_hex::str_hex(
            protocol::sign(
                &signing_public,
                signing_secret,
                &basics::base64::base64_decode(blob),
            )
            .expect("list signature"),
        )
    }

    #[test]
    fn parse_blobs_matches_v1_and_v2_shape() {
        let v1 = serde_json::json!({
            "blob": "Zm9v",
            "signature": "DEADBEEF",
            "version": 1
        });
        assert_eq!(
            ValidatorList::<SystemValidatorListClock>::parse_blobs(1, &v1).len(),
            1
        );

        let v2 = serde_json::json!({
            "version": 2,
            "blobs_v2": [
                {"blob": "Zm9v", "signature": "DEADBEEF", "manifest": "ZmFrZQ=="},
                {"blob": "YmFy", "signature": "CAFEBABE"}
            ]
        });
        assert_eq!(
            ValidatorList::<SystemValidatorListClock>::parse_blobs(2, &v2).len(),
            2
        );
    }

    #[test]
    fn status_snapshot_tracks_exact_summary_fields() {
        let list = ValidatorList::new(
            ManifestCache::new(),
            ManifestCache::new(),
            SystemValidatorListClock,
            Path::new("/tmp"),
            None,
        );

        let snapshot = list.status_snapshot();
        assert_eq!(snapshot.count, 0);
        assert_eq!(snapshot.status, ValidatorListStatus::Unknown);
        assert_eq!(snapshot.expiration, ValidatorListExpiration::Unknown);
        assert_eq!(snapshot.validator_list_threshold, 0);

        let JsonValue::Object(json) = list.get_json() else {
            panic!("validator list json should be an object");
        };
        let JsonValue::Object(summary) = json
            .get("validator_list")
            .expect("validator_list summary should exist")
        else {
            panic!("validator_list summary should be an object");
        };
        assert_eq!(summary.get("count"), Some(&JsonValue::Unsigned(0)));
        assert_eq!(
            summary.get("status"),
            Some(&JsonValue::String("unknown".to_owned()))
        );
        assert_eq!(
            summary.get("expiration"),
            Some(&JsonValue::String("unknown".to_owned()))
        );
        assert_eq!(
            summary.get("validator_list_threshold"),
            Some(&JsonValue::Unsigned(0))
        );
    }

    #[test]
    fn validator_list_accepts_trusted_publisher_and_marks_validator_listed() {
        let validator_manifests = ManifestCache::new();
        let publisher_manifests = ManifestCache::new();
        let list = ValidatorList::new(
            validator_manifests.clone(),
            publisher_manifests.clone(),
            SystemValidatorListClock,
            Path::new("/tmp"),
            None,
        );

        let master_secret = SecretKey::from_bytes([3u8; 32]);
        let signing_secret = SecretKey::from_bytes([4u8; 32]);
        let (manifest, publisher_master, publisher_signing) =
            manifest_blob(&master_secret, &signing_secret, 1);
        assert!(list.load(None, &[], &[publisher_master.to_hex()], None));

        let validator_master_secret = SecretKey::from_bytes([5u8; 32]);
        let validator_signing_secret = SecretKey::from_bytes([6u8; 32]);
        let (validator_manifest, validator_master, _) =
            manifest_blob(&validator_master_secret, &validator_signing_secret, 1);
        let clock = SystemValidatorListClock;

        let blob = base64_encode(
            serde_json::json!({
                "sequence": 2,
                "expiration": u64::from(clock.now_ripple()) + 3600,
                "validators": [{
                    "validation_public_key": validator_master.to_hex(),
                    "manifest": validator_manifest
                }]
            })
            .to_string()
            .as_bytes(),
        );
        let signature = signed_list(&blob, publisher_signing, &signing_secret);
        let blobs = vec![ValidatorBlobInfo {
            blob,
            signature,
            manifest: None,
        }];
        let hash = validator_list_collection_hash(&manifest, 1, &blobs);
        let result = list.apply_lists(
            &manifest,
            1,
            &blobs,
            "file:///tmp/vl.json".to_owned(),
            Some(hash),
        );
        assert_eq!(result.best_disposition(), ListDisposition::Accepted);
        assert!(list.listed(validator_master));

        let changes = list.update_trusted(&HashSet::new(), clock.now_ripple());
        assert!(
            changes
                .added
                .contains(&calc_account_id(validator_master.as_bytes()))
        );
        assert!(list.trusted(validator_master));
        assert_eq!(list.quorum(), 1);
    }

    #[test]
    fn validator_list_rejects_untrusted_publishers() {
        let list = ValidatorList::new(
            ManifestCache::new(),
            ManifestCache::new(),
            SystemValidatorListClock,
            Path::new("/tmp"),
            None,
        );
        let result = list.apply_lists("ZmFrZQ==", 1, &[], "file:///tmp/vl.json".to_owned(), None);
        assert_eq!(result.best_disposition(), ListDisposition::Invalid);
        assert_eq!(result.status, PublisherStatus::Unavailable);
    }
}
