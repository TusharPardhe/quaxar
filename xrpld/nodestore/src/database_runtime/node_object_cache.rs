use crate::NodeObject;
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
const DEFAULT_TARGET_NODES: u64 = 1_500_000;
const DEFAULT_EXPECTED_NODE_BYTES: u64 = 512;
const DEFAULT_MAX_ENTRY_BYTES: usize = 1_048_576;
const ENTRY_METADATA_BYTES: usize = 128;

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
        let target_nodes = get(
            config,
            "node_object_cache_target_nodes",
            DEFAULT_TARGET_NODES,
        );
        let expected_node_bytes = get(
            config,
            "node_object_cache_expected_node_bytes",
            DEFAULT_EXPECTED_NODE_BYTES,
        );
        let configured_capacity = get(config, "node_object_cache_capacity_bytes", 0u64);
        let max_entry_bytes = get(
            config,
            "node_object_cache_max_entry_bytes",
            DEFAULT_MAX_ENTRY_BYTES,
        );

        if target_nodes == 0 {
            return Err("Invalid node_object_cache_target_nodes".to_owned());
        }
        if expected_node_bytes == 0 {
            return Err("Invalid node_object_cache_expected_node_bytes".to_owned());
        }
        if max_entry_bytes == 0 {
            return Err("Invalid node_object_cache_max_entry_bytes".to_owned());
        }

        let capacity_bytes = if configured_capacity == 0 {
            let expected_entry_bytes = expected_node_bytes
                .checked_add(ENTRY_METADATA_BYTES as u64)
                .ok_or_else(|| "NodeObject cache entry weight overflows u64".to_owned())?;
            target_nodes
                .checked_mul(expected_entry_bytes)
                .ok_or_else(|| "NodeObject cache capacity overflows u64".to_owned())?
        } else {
            configured_capacity
        };
        if capacity_bytes == 0 {
            return Err("Invalid node_object_cache_capacity_bytes".to_owned());
        }

        let cache = Cache::builder()
            .max_capacity(capacity_bytes)
            .weigher(|_hash: &Uint256, object: &Arc<NodeObject>| {
                object
                    .data()
                    .len()
                    .saturating_add(ENTRY_METADATA_BYTES)
                    .min(u32::MAX as usize) as u32
            })
            .build();

        Ok(Self {
            cache,
            max_entry_bytes,
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
                return Some(object);
            }
            self.rejected.fetch_add(1, Ordering::Relaxed);
            self.cache.invalidate(&hash);
        }

        self.misses.fetch_add(1, Ordering::Relaxed);
        let result = self.cache.try_get_with(hash, || {
            self.durable_loads.fetch_add(1, Ordering::Relaxed);
            let Some(object) = load() else {
                return Err(CacheLoadError::NotFound);
            };
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
            Ok(object) => Some(object),
            Err(error) => match error.as_ref() {
                CacheLoadError::NotFound | CacheLoadError::Stale => None,
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
            "node_object_cache_capacity_bytes".to_owned(),
            JsonValue::String(self.capacity_bytes.to_string()),
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
