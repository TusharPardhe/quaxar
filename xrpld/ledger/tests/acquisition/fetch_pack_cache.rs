use basics::base_uint::Uint256;
use basics::tagged_cache::ManualClock;
use ledger::{FetchPackCache, sweep_ledger_master_like};
use std::sync::Arc;
use time::Duration;

fn sha512_half(data: &[u8]) -> Uint256 {
    use sha2::{Digest, Sha512};

    let digest = Sha512::digest(data);
    let mut bytes = [0u8; 32];
    bytes.copy_from_slice(&digest[..32]);
    Uint256::from_array(bytes)
}

#[test]
fn fetch_pack_cache_returns_matching_blob_and_erases_entry() {
    let clock = ManualClock::new(0);
    let cache = FetchPackCache::new(8, Duration::seconds(60), clock);
    let blob = vec![1u8, 2, 3, 4];
    let hash = sha512_half(&blob);

    cache.add_fetch_pack(hash, blob.clone());

    assert_eq!(cache.get_cache_size(), 1);
    assert_eq!(cache.get_fetch_pack(hash), Some(blob));
    assert_eq!(cache.get_cache_size(), 0);
    assert_eq!(cache.get_fetch_pack(hash), None);
}

#[test]
fn fetch_pack_cache_discards_invalid_blob_after_retrieval() {
    let clock = ManualClock::new(0);
    let cache = FetchPackCache::new(8, Duration::seconds(60), clock);
    let blob = vec![9u8, 8, 7, 6];

    cache.add_fetch_pack(Uint256::from_array([0x55; 32]), blob);

    assert_eq!(cache.get_fetch_pack(Uint256::from_array([0x55; 32])), None);
    assert_eq!(cache.get_cache_size(), 0);
}

#[test]
fn ledger_master_sweep_calls_history_and_fetch_pack() {
    let history_clock = Arc::new(ManualClock::new(0));
    let history = ledger::LedgerHistory::new(8, Duration::seconds(1), history_clock.clone());

    let fetch_pack_clock = Arc::new(ManualClock::new(0));
    let fetch_pack = FetchPackCache::new(8, Duration::seconds(1), fetch_pack_clock.clone());
    let blob = vec![4u8, 3, 2, 1];
    let hash = sha512_half(&blob);
    fetch_pack.add_fetch_pack(hash, blob);

    assert_eq!(fetch_pack.get_cache_size(), 1);
    history_clock.advance_seconds(5);
    fetch_pack_clock.advance_seconds(5);
    sweep_ledger_master_like(&history, &fetch_pack);
    assert_eq!(fetch_pack.get_cache_size(), 0);
}
