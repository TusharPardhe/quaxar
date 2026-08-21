use crate::{NodeObject, NodeObjectType};
use basics::base_uint::Uint256;
use basics::basic_config::{Section, get};
use moka::sync::Cache;
use protocol::JsonValue;
use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

/// Target occupancy for the always-on encoded NodeObject working set. The
/// weighted byte budget bounds Moka admission and eviction policy; Moka's
/// concurrent maintenance is intentionally not an instantaneous process-RSS
/// limit. This value derives the default capacity when the operator has not
/// supplied one.
const DEFAULT_TARGET_NODES: u64 = 1_000_000;
const DEFAULT_EXPECTED_NODE_BYTES: u64 = 512;
const DEFAULT_MAX_ENTRY_BYTES: usize = 1_048_576;
const ENTRY_METADATA_BYTES: usize = 128;
/// Default idle timeout in seconds — entries not accessed for this duration
/// are evicted. Matches rippled's `cache_age` for medium node_size (90s).
const DEFAULT_CACHE_IDLE_SECONDS: u64 = 90;
/// Default hard TTL in seconds — entries are evicted after this duration
/// regardless of access, ensuring post-rotation stale data is flushed.
const DEFAULT_CACHE_TTL_SECONDS: u64 = 0;

#[derive(Debug)]
enum CacheLoadError {
    NotFound,
    /// The cache was invalidated while the durable loader was running. The
    /// caller must retry through the current store state instead of allowing a
    /// pre-rotation result to repopulate the cache.
    Stale,
    Invalid(Arc<NodeObject>),
    Oversized(Arc<NodeObject>),
}

#[cfg(test)]
mod tests {
    use super::NodeObjectCache;
    use crate::{NodeObject, NodeObjectType};
    use basics::base_uint::Uint256;
    use basics::basic_config::Section;
    use std::sync::Arc;

    #[test]
    fn rippled_cache_size_and_age_control_entry_capacity_and_minutes() {
        let mut config = Section::new("node_db");
        config.set("cache_size", "4194304");
        config.set("cache_age", "120");
        config.set("cache_ttl_seconds", "0");

        let cache = NodeObjectCache::from_config(&config).expect("large node cache");

        assert_eq!(cache.capacity_entries, 4_194_304);
        assert_eq!(cache.idle_seconds, 7_200);
        assert_eq!(cache.ttl_seconds, 0);
    }

    #[test]
    fn explicit_node_object_settings_override_legacy_settings() {
        let mut config = Section::new("node_db");
        config.set("cache_size", "8");
        config.set("cache_age", "2");
        config.set("node_object_cache_target_nodes", "16");
        config.set("cache_idle_seconds", "30");

        let cache = NodeObjectCache::from_config(&config).expect("explicit node cache");

        assert_eq!(cache.capacity_entries, 16);
        assert_eq!(cache.idle_seconds, 30);
    }

    #[test]
    fn invalidation_during_loader_does_not_repopulate_cache() {
        let cache = NodeObjectCache::from_config(&Section::new("node_db")).expect("cache");
        let hash = Uint256::from_array([0xD1; 32]);
        let object = Arc::new(NodeObject::new(NodeObjectType::AccountNode, vec![1], hash));

        let first = cache.get_or_load(hash, || {
            cache.invalidate_all();
            Some(Arc::clone(&object))
        });
        assert!(first.is_none(), "stale loader result must not be cached");

        let second = cache
            .get_or_load(hash, || Some(Arc::clone(&object)))
            .expect("fresh generation must load the object");
        assert_eq!(second.hash(), object.hash());
    }
}

/// A bounded, concurrent cache of immutable encoded NodeObjects.
///
/// `max_capacity` limits weighted admission and drives eviction. As with Moka
/// generally, maintenance is concurrent and best-effort rather than a strict
/// instantaneous allocator/RSS ceiling.
///
/// The cache is deliberately separate from SHAMap's decoded TreeNodeCache:
/// it retains only durable-store shaped bytes and never owns decoded trees.
pub(crate) struct NodeObjectCache {
    cache: Cache<Uint256, Arc<NodeObject>>,
    max_entry_bytes: usize,
    capacity_entries: u64,
    idle_seconds: u64,
    ttl_seconds: u64,
    /// A sizing estimate only. The rippled-parity admission boundary is the
    /// entry count above, not a byte weigher.
    capacity_bytes: u64,
    hits: AtomicU64,
    misses: AtomicU64,
    durable_loads: AtomicU64,
    promotions: AtomicU64,
    rejected: AtomicU64,
    oversized: AtomicU64,
    invalidations: AtomicU64,
    /// Advances before every bulk invalidation. A Moka initializer records the
    /// generation it started in and refuses to insert if a rotation fence has
    /// begun while the durable read was in progress.
    generation: AtomicU64,
}

impl NodeObjectCache {
    pub(crate) fn from_config(config: &Section) -> Result<Self, String> {
        let cache_size = config
            .get::<i64>("cache_size")
            .map_err(|_| "Invalid cache_size".to_owned())?
            .map(|value| u64::try_from(value).map_err(|_| "Invalid cache_size".to_owned()))
            .transpose()?;
        let cache_age = config
            .get::<i64>("cache_age")
            .map_err(|_| "Invalid cache_age".to_owned())?
            .map(|value| u64::try_from(value).map_err(|_| "Invalid cache_age".to_owned()))
            .transpose()?;
        let target_nodes = if config.exists("node_object_cache_target_nodes") {
            get(
                config,
                "node_object_cache_target_nodes",
                DEFAULT_TARGET_NODES,
            )
        } else if let Some(cache_size) = cache_size {
            cache_size
        } else {
            DEFAULT_TARGET_NODES
        };
        let expected_node_bytes = get(
            config,
            "node_object_cache_expected_node_bytes",
            DEFAULT_EXPECTED_NODE_BYTES,
        );
        // Operator can set capacity directly in MB (preferred), or fall back
        // to legacy target_nodes × expected_node_bytes calculation.
        let configured_capacity_mb: u64 = get(config, "cache_capacity_mb", 0);
        let configured_capacity = get(config, "node_object_cache_capacity_bytes", 0u64);
        let max_entry_bytes = get(config, "cache_max_entry_bytes", DEFAULT_MAX_ENTRY_BYTES);
        let idle_seconds: u64 = if config.exists("cache_idle_seconds") {
            get(config, "cache_idle_seconds", DEFAULT_CACHE_IDLE_SECONDS)
        } else if let Some(cache_age) = cache_age {
            cache_age.saturating_mul(60)
        } else {
            DEFAULT_CACHE_IDLE_SECONDS
        };
        let ttl_seconds: u64 = get(config, "cache_ttl_seconds", DEFAULT_CACHE_TTL_SECONDS);

        if max_entry_bytes == 0 {
            return Err("Invalid cache_max_entry_bytes".to_owned());
        }

        let capacity_bytes = if configured_capacity_mb > 0 {
            configured_capacity_mb * 1_048_576
        } else if configured_capacity > 0 {
            configured_capacity
        } else {
            let expected_entry_bytes = expected_node_bytes
                .checked_add(ENTRY_METADATA_BYTES as u64)
                .ok_or_else(|| "NodeObject cache entry weight overflows u64".to_owned())?;
            target_nodes
                .checked_mul(expected_entry_bytes)
                .ok_or_else(|| "NodeObject cache capacity overflows u64".to_owned())?
        };
        let entry_capacity = if configured_capacity_mb > 0 || configured_capacity > 0 {
            capacity_bytes
                / expected_node_bytes
                    .saturating_add(ENTRY_METADATA_BYTES as u64)
                    .max(1)
        } else {
            target_nodes
        };

        // rippled's NodeObject TaggedCache is bounded by entry count and
        // starts empty. Do not byte-weight or preallocate millions of slots.
        let mut builder = Cache::builder()
            .name("node-object-cache")
            .max_capacity(entry_capacity);

        if idle_seconds > 0 {
            builder = builder.time_to_idle(std::time::Duration::from_secs(idle_seconds));
        }
        if ttl_seconds > 0 {
            builder = builder.time_to_live(std::time::Duration::from_secs(ttl_seconds));
        }

        let cache = builder.build();

        Ok(Self {
            cache,
            max_entry_bytes,
            capacity_entries: entry_capacity,
            idle_seconds,
            ttl_seconds,
            capacity_bytes,
            hits: AtomicU64::new(0),
            misses: AtomicU64::new(0),
            durable_loads: AtomicU64::new(0),
            promotions: AtomicU64::new(0),
            rejected: AtomicU64::new(0),
            oversized: AtomicU64::new(0),
            invalidations: AtomicU64::new(0),
            generation: AtomicU64::new(0),
        })
    }

    /// Returns a validated cached value or performs one Moka-coalesced durable
    /// load. `None` is represented as an error and is therefore never cached.
    pub(crate) fn get_or_load<F>(&self, hash: Uint256, load: F) -> Option<Arc<NodeObject>>
    where
        F: FnOnce() -> Option<Arc<NodeObject>>,
    {
        let generation = self.generation();
        if let Some(object) = self.cache.get(&hash) {
            if object.hash() == &hash {
                self.hits.fetch_add(1, Ordering::Relaxed);
                return (object.object_type() != NodeObjectType::Dummy).then_some(object);
            }
            self.rejected.fetch_add(1, Ordering::Relaxed);
            self.cache.invalidate(&hash);
        }

        self.misses.fetch_add(1, Ordering::Relaxed);
        let result = self.cache.try_get_with(hash, || {
            self.durable_loads.fetch_add(1, Ordering::Relaxed);
            let object = load().ok_or(CacheLoadError::NotFound)?;
            if object.hash() != &hash {
                return Err(CacheLoadError::Invalid(object));
            }
            if object.data().len() > self.max_entry_bytes {
                return Err(CacheLoadError::Oversized(object));
            }
            if self.generation() != generation {
                return Err(CacheLoadError::Stale);
            }
            self.promotions.fetch_add(1, Ordering::Relaxed);
            Ok(object)
        });

        match result {
            Ok(object) => (object.object_type() != NodeObjectType::Dummy).then_some(object),
            Err(error) => match error.as_ref() {
                CacheLoadError::NotFound => None,
                CacheLoadError::Stale => None,
                CacheLoadError::Invalid(object) => {
                    self.rejected.fetch_add(1, Ordering::Relaxed);
                    Some(Arc::clone(object))
                }
                CacheLoadError::Oversized(object) => {
                    self.oversized.fetch_add(1, Ordering::Relaxed);
                    Some(Arc::clone(object))
                }
            },
        }
    }

    /// Promote a fetch result only when it matches the requested content hash.
    pub(crate) fn promote_for_hash(&self, expected: &Uint256, object: Arc<NodeObject>) {
        if object.hash() != expected {
            self.rejected.fetch_add(1, Ordering::Relaxed);
            return;
        }
        self.promote(object);
    }

    /// Promote only a valid, reasonably sized positive object. Store callers
    /// invoke this after their backend write returns normally.
    pub(crate) fn promote(&self, object: Arc<NodeObject>) {
        if object.data().len() > self.max_entry_bytes {
            self.oversized.fetch_add(1, Ordering::Relaxed);
            return;
        }
        self.cache.insert(*object.hash(), object);
        self.promotions.fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn generation(&self) -> u64 {
        self.generation.load(Ordering::Acquire)
    }

    pub(crate) fn invalidate_all(&self) {
        self.generation.fetch_add(1, Ordering::AcqRel);
        self.cache.invalidate_all();
        self.invalidations.fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn add_counts_json(&self, obj: &mut BTreeMap<String, JsonValue>) {
        obj.insert(
            "node_object_cache_capacity_entries".to_owned(),
            JsonValue::String(self.capacity_entries.to_string()),
        );
        obj.insert(
            "node_object_cache_idle_seconds".to_owned(),
            JsonValue::String(self.idle_seconds.to_string()),
        );
        obj.insert(
            "node_object_cache_ttl_seconds".to_owned(),
            JsonValue::String(self.ttl_seconds.to_string()),
        );
        // Preserve the established key for RPC compatibility, but make its
        // modeled nature explicit. Moka admission is entry-count bounded.
        obj.insert(
            "node_object_cache_capacity_bytes".to_owned(),
            JsonValue::String(self.capacity_bytes.to_string()),
        );
        obj.insert(
            "node_object_cache_capacity_bytes_is_estimate".to_owned(),
            JsonValue::Bool(true),
        );
        obj.insert(
            "node_object_cache_entries".to_owned(),
            JsonValue::String(self.cache.entry_count().to_string()),
        );
        obj.insert(
            "node_object_cache_hits".to_owned(),
            JsonValue::String(self.hits.load(Ordering::Relaxed).to_string()),
        );
        obj.insert(
            "node_object_cache_misses".to_owned(),
            JsonValue::String(self.misses.load(Ordering::Relaxed).to_string()),
        );
        obj.insert(
            "node_object_cache_durable_loads".to_owned(),
            JsonValue::String(self.durable_loads.load(Ordering::Relaxed).to_string()),
        );
        obj.insert(
            "node_object_cache_promotions".to_owned(),
            JsonValue::String(self.promotions.load(Ordering::Relaxed).to_string()),
        );
        obj.insert(
            "node_object_cache_rejected".to_owned(),
            JsonValue::String(self.rejected.load(Ordering::Relaxed).to_string()),
        );
        obj.insert(
            "node_object_cache_oversized".to_owned(),
            JsonValue::String(self.oversized.load(Ordering::Relaxed).to_string()),
        );
        obj.insert(
            "node_object_cache_invalidations".to_owned(),
            JsonValue::String(self.invalidations.load(Ordering::Relaxed).to_string()),
        );
    }
}
