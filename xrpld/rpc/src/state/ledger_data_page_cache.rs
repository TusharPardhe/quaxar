//! Pre-serialized ledger_data response page cache.

use basics::base_uint::Uint256;
use basics::str_hex::str_hex;
use protocol::JsonValue;
use std::sync::{Arc, RwLock};

pub const DEFAULT_PAGE_SIZE: usize = 256;

/// A pre-serialized page of ledger_data state entries.
pub struct LedgerDataPage {
    /// First key of this page (for binary search).
    pub start_key: Uint256,
    /// Marker for the next page (first key of next page - 1), or None if last page.
    pub next_marker: Option<Uint256>,
    /// Pre-serialized JSON array bytes for binary=false responses.
    pub json_state_bytes: Arc<[u8]>,
    /// Pre-serialized JSON array bytes for binary=true responses.
    pub binary_state_bytes: Arc<[u8]>,
    pub entry_count: usize,
}

pub struct LedgerDataPageCache {
    pages: Vec<LedgerDataPage>,
    pub page_size: usize,
    pub ledger_seq: u32,
    pub ledger_hash: Uint256,
}

impl LedgerDataPageCache {
    /// Build the page cache from an iterator of (key, binary_data, json) tuples.
    pub fn build<I>(ledger_seq: u32, ledger_hash: Uint256, entries: I, page_size: usize) -> Self
    where
        I: Iterator<Item = (Uint256, Vec<u8>, JsonValue)>,
    {
        let mut all_entries: Vec<(Uint256, Vec<u8>, JsonValue)> = entries.collect();
        all_entries.sort_unstable_by_key(|(k, _, _)| *k);

        let mut pages = Vec::new();
        let mut i = 0;

        while i < all_entries.len() {
            let end = (i + page_size).min(all_entries.len());
            let page_entries = &all_entries[i..end];

            // Build JSON state array
            let json_state: Vec<serde_json::Value> = page_entries
                .iter()
                .map(|(key, _, json)| {
                    let mut node = json.clone();
                    if let JsonValue::Object(obj) = &mut node {
                        obj.insert("index".to_owned(), JsonValue::String(key.to_string()));
                    }
                    from_protocol_json(&node)
                })
                .collect();

            // Build binary state array
            let binary_state: Vec<serde_json::Value> = page_entries
                .iter()
                .map(|(key, binary, _)| {
                    let mut node = std::collections::BTreeMap::new();
                    node.insert(
                        "data".to_owned(),
                        JsonValue::String(str_hex(binary.as_slice())),
                    );
                    node.insert("index".to_owned(), JsonValue::String(key.to_string()));
                    from_protocol_json(&JsonValue::Object(node))
                })
                .collect();

            // Compute marker for this page
            let next_marker = if end < all_entries.len() {
                let mut marker = all_entries[end].0;
                marker.decrement();
                Some(marker)
            } else {
                None
            };

            // Build json result envelope
            let mut json_obj = serde_json::Map::new();
            json_obj.insert(
                "ledger_hash".to_owned(),
                serde_json::Value::String(ledger_hash.to_string()),
            );
            json_obj.insert(
                "ledger_index".to_owned(),
                serde_json::Value::Number(serde_json::Number::from(ledger_seq)),
            );
            if let Some(m) = next_marker {
                json_obj.insert(
                    "marker".to_owned(),
                    serde_json::Value::String(m.to_string()),
                );
            }
            json_obj.insert("state".to_owned(), serde_json::Value::Array(json_state));

            // Build binary result envelope
            let mut binary_obj = serde_json::Map::new();
            binary_obj.insert(
                "ledger_hash".to_owned(),
                serde_json::Value::String(ledger_hash.to_string()),
            );
            binary_obj.insert(
                "ledger_index".to_owned(),
                serde_json::Value::Number(serde_json::Number::from(ledger_seq)),
            );
            if let Some(m) = next_marker {
                binary_obj.insert(
                    "marker".to_owned(),
                    serde_json::Value::String(m.to_string()),
                );
            }
            binary_obj.insert("state".to_owned(), serde_json::Value::Array(binary_state));

            let json_bytes =
                serde_json::to_vec(&serde_json::Value::Object(json_obj)).unwrap_or_default();
            let binary_bytes =
                serde_json::to_vec(&serde_json::Value::Object(binary_obj)).unwrap_or_default();

            pages.push(LedgerDataPage {
                start_key: page_entries[0].0,
                next_marker,
                json_state_bytes: json_bytes.into(),
                binary_state_bytes: binary_bytes.into(),
                entry_count: page_entries.len(),
            });

            i = end;
        }

        Self {
            pages,
            page_size,
            ledger_seq,
            ledger_hash,
        }
    }

    pub fn find_page_for_marker(&self, marker: Option<Uint256>) -> Option<&LedgerDataPage> {
        match marker {
            None => self.pages.first(),
            Some(m) => {
                // Binary search for the page whose start_key <= m
                let idx = self.pages.partition_point(|p| p.start_key <= m);
                if idx == 0 {
                    self.pages.first()
                } else {
                    self.pages.get(idx - 1)
                }
            }
        }
    }

    pub fn page_count(&self) -> usize {
        self.pages.len()
    }
    pub fn ledger_seq(&self) -> u32 {
        self.ledger_seq
    }
}

pub struct LedgerDataPageCacheStore {
    inner: RwLock<Option<Arc<LedgerDataPageCache>>>,
}

impl LedgerDataPageCacheStore {
    pub fn new() -> Self {
        Self {
            inner: RwLock::new(None),
        }
    }

    pub fn get(&self, ledger_seq: u32) -> Option<Arc<LedgerDataPageCache>> {
        let g = self.inner.read().unwrap();
        g.as_ref().filter(|c| c.ledger_seq == ledger_seq).cloned()
    }

    pub fn insert(&self, cache: Arc<LedgerDataPageCache>) {
        let mut g = self.inner.write().unwrap();
        *g = Some(cache);
    }
}

impl Default for LedgerDataPageCacheStore {
    fn default() -> Self {
        Self::new()
    }
}

static GLOBAL_PAGE_CACHE: std::sync::OnceLock<LedgerDataPageCacheStore> =
    std::sync::OnceLock::new();

pub fn get_global_page_cache() -> &'static LedgerDataPageCacheStore {
    GLOBAL_PAGE_CACHE.get_or_init(LedgerDataPageCacheStore::new)
}

/// Convert protocol::JsonValue to serde_json::Value
fn from_protocol_json(value: &JsonValue) -> serde_json::Value {
    match value {
        JsonValue::Null => serde_json::Value::Null,
        JsonValue::Bool(value) => serde_json::Value::Bool(*value),
        JsonValue::Signed(value) => serde_json::Value::Number((*value).into()),
        JsonValue::Unsigned(value) => serde_json::Value::Number((*value).into()),
        JsonValue::String(value) => serde_json::Value::String(value.clone()),
        JsonValue::Array(values) => {
            serde_json::Value::Array(values.iter().map(from_protocol_json).collect())
        }
        JsonValue::Object(object) => serde_json::Value::Object(
            object
                .iter()
                .map(|(key, value)| (key.clone(), from_protocol_json(value)))
                .collect(),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_find_page_for_marker() {
        let mut key1 = Uint256::default();
        key1.as_mut_slice()[31] = 10;

        let mut key2 = Uint256::default();
        key2.as_mut_slice()[31] = 20;

        let mut key3 = Uint256::default();
        key3.as_mut_slice()[31] = 30;

        let pages = vec![
            LedgerDataPage {
                start_key: key1,
                next_marker: Some(key2),
                json_state_bytes: Arc::new([]),
                binary_state_bytes: Arc::new([]),
                entry_count: 10,
            },
            LedgerDataPage {
                start_key: key2,
                next_marker: Some(key3),
                json_state_bytes: Arc::new([]),
                binary_state_bytes: Arc::new([]),
                entry_count: 10,
            },
            LedgerDataPage {
                start_key: key3,
                next_marker: None,
                json_state_bytes: Arc::new([]),
                binary_state_bytes: Arc::new([]),
                entry_count: 5,
            },
        ];

        let cache = LedgerDataPageCache {
            pages,
            page_size: 10,
            ledger_seq: 1,
            ledger_hash: Uint256::default(),
        };

        // None marker -> first page
        let page = cache.find_page_for_marker(None).unwrap();
        assert_eq!(page.start_key, key1);

        // Marker < key1 -> first page
        let mut marker0 = Uint256::default();
        marker0.as_mut_slice()[31] = 5;
        let page = cache.find_page_for_marker(Some(marker0)).unwrap();
        assert_eq!(page.start_key, key1);

        // Marker == key2 -> second page
        let page = cache.find_page_for_marker(Some(key2)).unwrap();
        assert_eq!(page.start_key, key2);

        // Marker between key2 and key3 -> second page
        let mut marker_mid = Uint256::default();
        marker_mid.as_mut_slice()[31] = 25;
        let page = cache.find_page_for_marker(Some(marker_mid)).unwrap();
        assert_eq!(page.start_key, key2);

        // Marker > key3 -> third page
        let mut marker_high = Uint256::default();
        marker_high.as_mut_slice()[31] = 40;
        let page = cache.find_page_for_marker(Some(marker_high)).unwrap();
        assert_eq!(page.start_key, key3);
    }
}
