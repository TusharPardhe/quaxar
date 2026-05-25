use basics::hardened_hash::HardenedHashBuilder;
use basics::unordered_containers::{
    HardenedHashMultimap, HardenedHashMultiset, HashMultimap, HashMultiset,
};

#[test]
fn plain_multimap_keeps_duplicate_values_per_key() {
    let mut map = HashMultimap::<String, usize>::new();

    map.insert(String::from("ledger"), 1);
    map.insert(String::from("ledger"), 2);
    map.insert(String::from("tx"), 3);

    assert_eq!(map.len(), 3);
    assert_eq!(map.count("ledger"), 2);
    assert_eq!(map.get("ledger"), Some(&[1, 2][..]));

    let mut seen: Vec<_> = map
        .iter()
        .map(|(key, value)| (key.as_str(), *value))
        .collect();
    seen.sort_unstable();
    assert_eq!(seen, vec![("ledger", 1), ("ledger", 2), ("tx", 3)]);

    assert_eq!(map.remove_one("ledger"), Some(2));
    assert_eq!(map.count("ledger"), 1);
    assert_eq!(map.remove_all("ledger"), 1);
    assert_eq!(map.count("ledger"), 0);
}

#[test]
fn hardened_multimap_accepts_explicit_seeded_hashers() {
    let mut map =
        HardenedHashMultimap::<String, usize>::with_hasher(HardenedHashBuilder::from_seed(17));

    map.insert(String::from("owner"), 9);
    map.insert(String::from("owner"), 11);

    assert_eq!(map.count("owner"), 2);
    assert!(map.contains_key("owner"));
    assert_eq!(map.remove_all("owner"), 2);
    assert!(map.is_empty());
}

#[test]
fn plain_multiset_counts_and_repeats_duplicates() {
    let mut set = HashMultiset::<String>::default();

    set.insert(String::from("ledger"));
    set.insert(String::from("ledger"));
    set.insert(String::from("tx"));

    assert_eq!(set.len(), 3);
    assert_eq!(set.count("ledger"), 2);
    assert!(set.contains_key("ledger"));

    let mut seen: Vec<_> = set.iter().cloned().collect();
    seen.sort_unstable();
    assert_eq!(seen, vec!["ledger", "ledger", "tx"]);

    assert!(set.remove_one("ledger"));
    assert_eq!(set.count("ledger"), 1);
    assert_eq!(set.remove_all("ledger"), 1);
    assert!(!set.remove_one("ledger"));
    assert_eq!(set.count("ledger"), 0);
}

#[test]
fn hardened_multiset_accepts_explicit_seeded_hashers() {
    let mut set = HardenedHashMultiset::<String>::with_hasher(HardenedHashBuilder::from_seed(29));

    set.insert(String::from("fee"));
    set.insert(String::from("fee"));

    assert_eq!(set.count("fee"), 2);
    assert!(set.remove_one("fee"));
    assert_eq!(set.remove_all("fee"), 1);
    assert!(set.is_empty());
}
