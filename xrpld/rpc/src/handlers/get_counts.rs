//! Narrow `get_counts` RPC handler port.
//!
//! This keeps the reference control flow shape:
//! - read `min_count` from already-parsed params,
//! - read counted-object entries from the real global registry,
//! - format uptime with the same descending unit order,
//! - and delegate app-owned metrics through an explicit source seam.

use std::collections::BTreeMap;
use std::time::Duration;

use basics::counted_object::CountedObjects;
use basics::memory::malloc_trim::jemalloc_stats;
use basics::uptime_clock::{UptimeClock, UptimeTimePoint};
use protocol::JsonValue;

const DEFAULT_MIN_COUNT: u32 = 10;

pub trait GetCountsSource {
    fn use_tx_tables(&self) -> bool;

    fn db_kb_total(&self) -> u64;

    fn db_kb_ledger(&self) -> u64;

    fn db_kb_transaction(&self) -> u64;

    fn local_tx_count(&self) -> usize;

    fn write_load(&self) -> JsonValue;

    fn historical_perminute(&self) -> i64;

    fn sle_hit_rate(&self) -> JsonValue;

    fn ledger_hit_rate(&self) -> JsonValue;

    fn accepted_ledger_cache_size(&self) -> u64;

    fn accepted_ledger_cache_hit_rate(&self) -> JsonValue;

    fn fullbelow_size(&self) -> i64;

    fn treenode_cache_size(&self) -> u64;

    fn treenode_cache_capacity_entries(&self) -> u64;

    fn treenode_track_size(&self) -> u64;

    fn add_node_store_counts(&self, json: &mut BTreeMap<String, JsonValue>);
    /// Distinct resident (inner, leaf) tree nodes reachable from the ledgers
    /// this node holds. Walks memory only; opt-in because it is O(live nodes).
    fn held_tree_reachable(&self) -> Option<(u64, u64)> {
        None
    }
}

pub fn read_min_count(params: &JsonValue) -> u32 {
    let JsonValue::Object(object) = params else {
        return DEFAULT_MIN_COUNT;
    };

    let Some(value) = object.get("min_count") else {
        return DEFAULT_MIN_COUNT;
    };

    match value {
        JsonValue::Unsigned(value) => u32::try_from(*value).unwrap_or(DEFAULT_MIN_COUNT),
        JsonValue::Signed(value) if *value >= 0 => {
            u32::try_from(*value as u64).unwrap_or(DEFAULT_MIN_COUNT)
        }
        _ => DEFAULT_MIN_COUNT,
    }
}

fn append_uptime_component(
    text: &mut String,
    remaining: &mut UptimeTimePoint,
    unit_name: &str,
    unit_duration: Duration,
) {
    let count = remaining.time_since_epoch().as_secs() / unit_duration.as_secs();
    if count == 0 {
        return;
    }

    *remaining = *remaining - Duration::from_secs(count * unit_duration.as_secs());
    if !text.is_empty() {
        text.push_str(", ");
    }
    text.push_str(&count.to_string());
    text.push(' ');
    text.push_str(unit_name);
    if count > 1 {
        text.push('s');
    }
}

fn format_uptime(now: UptimeTimePoint) -> String {
    let mut text = String::new();
    let mut remaining = now;

    append_uptime_component(
        &mut text,
        &mut remaining,
        "year",
        Duration::from_secs(365 * 24 * 60 * 60),
    );
    append_uptime_component(
        &mut text,
        &mut remaining,
        "day",
        Duration::from_secs(24 * 60 * 60),
    );
    append_uptime_component(
        &mut text,
        &mut remaining,
        "hour",
        Duration::from_secs(60 * 60),
    );
    append_uptime_component(&mut text, &mut remaining, "minute", Duration::from_secs(60));
    append_uptime_component(&mut text, &mut remaining, "second", Duration::from_secs(1));

    text
}

pub fn get_counts_json<S: GetCountsSource>(source: &S, min_count: u32) -> JsonValue {
    let mut result = BTreeMap::new();

    for (name, count) in CountedObjects::get_instance().get_counts(min_count as i32) {
        result.insert(name, JsonValue::Signed(i64::from(count)));
    }

    if source.use_tx_tables() {
        let db_kb_total = source.db_kb_total();
        if db_kb_total > 0 {
            result.insert("dbKBTotal".to_owned(), JsonValue::Unsigned(db_kb_total));
        }

        let db_kb_ledger = source.db_kb_ledger();
        if db_kb_ledger > 0 {
            result.insert("dbKBLedger".to_owned(), JsonValue::Unsigned(db_kb_ledger));
        }

        let db_kb_transaction = source.db_kb_transaction();
        if db_kb_transaction > 0 {
            result.insert(
                "dbKBTransaction".to_owned(),
                JsonValue::Unsigned(db_kb_transaction),
            );
        }

        let local_txs = source.local_tx_count();
        if local_txs > 0 {
            result.insert(
                "local_txs".to_owned(),
                JsonValue::Unsigned(local_txs as u64),
            );
        }
    }

    result.insert("write_load".to_owned(), source.write_load());
    result.insert(
        "historical_perminute".to_owned(),
        JsonValue::Signed(source.historical_perminute()),
    );
    result.insert("SLE_hit_rate".to_owned(), source.sle_hit_rate());
    result.insert("ledger_hit_rate".to_owned(), source.ledger_hit_rate());
    result.insert(
        "AL_size".to_owned(),
        JsonValue::Unsigned(source.accepted_ledger_cache_size()),
    );
    result.insert(
        "AL_hit_rate".to_owned(),
        source.accepted_ledger_cache_hit_rate(),
    );
    result.insert(
        "fullbelow_size".to_owned(),
        JsonValue::Signed(source.fullbelow_size()),
    );
    result.insert(
        "treenode_cache_size".to_owned(),
        JsonValue::Unsigned(source.treenode_cache_size()),
    );
    result.insert(
        "treenode_cache_capacity_entries".to_owned(),
        JsonValue::Unsigned(source.treenode_cache_capacity_entries()),
    );
    result.insert(
        "treenode_track_size".to_owned(),
        JsonValue::Unsigned(source.treenode_track_size()),
    );
    result.insert(
        "uptime".to_owned(),
        JsonValue::String(format_uptime(UptimeClock::now())),
    );

    if let Some(stats) = jemalloc_stats() {
        result.insert(
            "jemalloc_allocated_bytes".to_owned(),
            JsonValue::Unsigned(stats.allocated_bytes as u64),
        );
        result.insert(
            "jemalloc_active_bytes".to_owned(),
            JsonValue::Unsigned(stats.active_bytes as u64),
        );
        result.insert(
            "jemalloc_metadata_bytes".to_owned(),
            JsonValue::Unsigned(stats.metadata_bytes as u64),
        );
        result.insert(
            "jemalloc_mapped_bytes".to_owned(),
            JsonValue::Unsigned(stats.mapped_bytes as u64),
        );
        result.insert(
            "jemalloc_resident_bytes".to_owned(),
            JsonValue::Unsigned(stats.resident_bytes as u64),
        );
        result.insert(
            "jemalloc_retained_bytes".to_owned(),
            JsonValue::Unsigned(stats.retained_bytes as u64),
        );
        for (name, value) in [
            ("jemalloc_dirty_bytes", stats.dirty_bytes),
            ("jemalloc_muzzy_bytes", stats.muzzy_bytes),
            ("jemalloc_small_slab_bytes", stats.small_slab_bytes),
            ("jemalloc_small_allocated_bytes", stats.small_allocated_bytes),
            ("jemalloc_large_allocated_bytes", stats.large_allocated_bytes),
        ] {
            result.insert(name.to_owned(), JsonValue::Unsigned(value as u64));
        }
        result.insert(
            "jemalloc_arenas".to_owned(),
            JsonValue::Unsigned(u64::from(stats.arenas)),
        );
    }

    source.add_node_store_counts(&mut result);
    let memory = memory_section(&result, read_process_rss());
    result.insert("memory".to_owned(), memory);

    JsonValue::Object(result)
}

/// Resident memory as the kernel reports it (`/proc/self/status`), in bytes.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ProcessRss {
    pub rss: u64,
    pub anon: u64,
    pub file: u64,
}

/// Read VmRSS, RssAnon and RssFile from `/proc/self/status` (Linux only).
pub fn read_process_rss() -> Option<ProcessRss> {
    parse_proc_status(&std::fs::read_to_string("/proc/self/status").ok()?)
}

fn parse_proc_status(status: &str) -> Option<ProcessRss> {
    let field = |name: &str| {
        status.lines().find_map(|line| {
            let rest = line.strip_prefix(name)?.strip_prefix(':')?;
            let kb = rest.trim().strip_suffix("kB")?.trim().parse::<u64>().ok()?;
            Some(kb * 1024)
        })
    };
    Some(ProcessRss {
        rss: field("VmRSS")?,
        anon: field("RssAnon").unwrap_or(0),
        file: field("RssFile").unwrap_or(0),
    })
}

/// Memory components with a known size, as (name in `memory`, counter it
/// comes from). Each is memory quaxar itself accounts for.
const CHARGED_COMPONENTS: [(&str, &str); 4] = [
    ("shamap_nodes", "shamap_structural_bytes"),
    ("shamap_items", "shamap_allocated_item_bytes"),
    ("fjall_cache", "fjall_cache_bytes"),
    ("fjall_memtables", "fjall_write_buffer_bytes"),
];

fn counter(counts: &BTreeMap<String, JsonValue>, key: &str) -> Option<u64> {
    match counts.get(key)? {
        JsonValue::Unsigned(v) => Some(*v),
        JsonValue::Signed(v) => u64::try_from(*v).ok(),
        JsonValue::String(v) => v.parse().ok(),
        _ => None,
    }
}

/// The `memory` section: each layer from what quaxar accounts for up to what
/// the kernel counts, and the gaps between them.
///
/// ```text
/// charged  <=  jemalloc allocated  <=  jemalloc resident  ~  process RSS
///   gap: untracked_bytes   gap: allocator_overhead   gap: rss_minus_resident
/// ```
///
/// IMPORTANT: `untracked_bytes` is memory in use that no counter explains
/// (other caches, per-entry overhead, queues). If it grows, look for an
/// unaccounted structure; if `allocator_overhead_bytes` grows instead, it is
/// fragmentation or pages not yet returned to the OS.
pub fn memory_section(
    counts: &BTreeMap<String, JsonValue>,
    rss: Option<ProcessRss>,
) -> JsonValue {
    let mut memory = BTreeMap::new();
    let mut components = BTreeMap::new();
    let mut charged = 0u64;
    for (name, key) in CHARGED_COMPONENTS {
        if let Some(bytes) = counter(counts, key) {
            charged += bytes;
            components.insert(name.to_owned(), JsonValue::Unsigned(bytes));
        }
    }
    memory.insert("charged_bytes".to_owned(), JsonValue::Unsigned(charged));
    memory.insert("charged".to_owned(), JsonValue::Object(components));

    let allocated = counter(counts, "jemalloc_allocated_bytes");
    let resident = counter(counts, "jemalloc_resident_bytes");
    if let Some(allocated) = allocated {
        memory.insert("allocated_bytes".to_owned(), JsonValue::Unsigned(allocated));
        memory.insert(
            "untracked_bytes".to_owned(),
            JsonValue::Signed(allocated as i64 - charged as i64),
        );
        if charged > 0 {
            memory.insert(
                "allocated_per_charged".to_owned(),
                JsonValue::String(format!("{:.2}", allocated as f64 / charged as f64)),
            );
        }
    }
    if let Some(resident) = resident {
        memory.insert("resident_bytes".to_owned(), JsonValue::Unsigned(resident));
        if let Some(allocated) = allocated {
            memory.insert(
                "allocator_overhead_bytes".to_owned(),
                JsonValue::Signed(resident as i64 - allocated as i64),
            );
        }
    }
    if let Some(rss) = rss {
        memory.insert("rss_bytes".to_owned(), JsonValue::Unsigned(rss.rss));
        memory.insert("rss_anon_bytes".to_owned(), JsonValue::Unsigned(rss.anon));
        memory.insert("rss_file_bytes".to_owned(), JsonValue::Unsigned(rss.file));
        if let Some(resident) = resident {
            memory.insert(
                "rss_minus_resident_bytes".to_owned(),
                JsonValue::Signed(rss.rss as i64 - resident as i64),
            );
        }
    }
    JsonValue::Object(memory)
}

pub fn do_get_counts<S: GetCountsSource>(params: &JsonValue, source: &S) -> JsonValue {
    let mut result = get_counts_json(source, read_min_count(params));
    let held_tree = matches!(params, JsonValue::Object(object)
        if matches!(object.get("held_tree"), Some(JsonValue::Bool(true))));
    if held_tree
        && let JsonValue::Object(object) = &mut result
        && let Some((inner, leaf)) = source.held_tree_reachable()
    {
        object.insert("held_tree_inner_nodes".to_owned(), JsonValue::Unsigned(inner));
        object.insert("held_tree_leaf_nodes".to_owned(), JsonValue::Unsigned(leaf));
    }
    result
}

#[cfg(test)]
mod memory_tests {
    use super::*;

    #[test]
    fn proc_status_rss_fields_are_parsed_in_bytes() {
        let status = "Name:\tquaxar\nVmRSS:\t  4710000 kB\nRssAnon:\t 4600000 kB\nRssFile:\t  110000 kB\n";
        assert_eq!(
            parse_proc_status(status),
            Some(ProcessRss {
                rss: 4_710_000 * 1024,
                anon: 4_600_000 * 1024,
                file: 110_000 * 1024,
            })
        );
        assert_eq!(parse_proc_status("Name:\tx\n"), None);
    }

    /// The layers and gaps add up: charged + untracked = allocated,
    /// allocated + overhead = resident, resident + rss gap = RSS.
    #[test]
    fn memory_section_reports_each_layer_and_gap() {
        let mut counts = BTreeMap::new();
        for (key, value) in [
            ("shamap_structural_bytes", 1_000u64),
            ("shamap_allocated_item_bytes", 200),
            ("fjall_cache_bytes", 300),
            ("fjall_write_buffer_bytes", 100),
            ("jemalloc_allocated_bytes", 2_400),
            ("jemalloc_resident_bytes", 2_600),
        ] {
            counts.insert(key.to_owned(), JsonValue::Unsigned(value));
        }
        let rss = ProcessRss { rss: 2_500, anon: 2_450, file: 50 };
        let JsonValue::Object(memory) = memory_section(&counts, Some(rss)) else {
            panic!("memory is an object");
        };
        let get = |key: &str| memory.get(key).cloned();
        assert_eq!(get("charged_bytes"), Some(JsonValue::Unsigned(1_600)));
        assert_eq!(get("untracked_bytes"), Some(JsonValue::Signed(800)));
        assert_eq!(get("allocated_per_charged"), Some(JsonValue::String("1.50".into())));
        assert_eq!(get("allocator_overhead_bytes"), Some(JsonValue::Signed(200)));
        assert_eq!(get("rss_minus_resident_bytes"), Some(JsonValue::Signed(-100)));
        let Some(JsonValue::Object(charged)) = get("charged") else {
            panic!("charged components");
        };
        assert_eq!(charged.get("fjall_cache"), Some(&JsonValue::Unsigned(300)));
    }

    #[test]
    fn memory_section_skips_layers_without_data() {
        let JsonValue::Object(memory) = memory_section(&BTreeMap::new(), None) else {
            panic!("memory is an object");
        };
        assert_eq!(memory.get("charged_bytes"), Some(&JsonValue::Unsigned(0)));
        assert!(!memory.contains_key("untracked_bytes"));
        assert!(!memory.contains_key("rss_bytes"));
    }
}
