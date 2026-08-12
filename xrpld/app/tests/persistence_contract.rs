use app::SHAMapStoreNodeStore;
use basics::basic_config::Section;
use nodestore::{DummyScheduler, Manager, ManagerImp, NullJournal};
use std::sync::Arc;

#[test]
fn shamap_store_facade_publishes_a_nonzero_storage_generation() {
    let manager = ManagerImp::new();
    let mut config = Section::new("node_db");
    config.set("type", "Memory");
    config.set("path", "persistence-generation-contract");
    let database = manager
        .make_database(
            0,
            Arc::new(DummyScheduler),
            1,
            &config,
            Arc::new(NullJournal),
        )
        .expect("database");
    let node_store = SHAMapStoreNodeStore::Single(database);

    assert!(
        node_store.store_generation() > 0,
        "ReadKey admission must never receive the legacy generation zero"
    );
}
