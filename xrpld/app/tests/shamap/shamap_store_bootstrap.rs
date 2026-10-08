use app::{
    ApplicationRoot, SHAMapStoreNodeStore, SHAMapStoreSavedState, SHAMapStoreSavedStateDb,
    bootstrap_shamap_store,
};
use basics::basic_config::BasicConfig;
use nodestore::{DummyScheduler, ManagerImp, NullJournal, Scheduler};
use std::fs;
use std::sync::Arc;
use tempfile::TempDir;

fn online_delete_config(
    database_path: &std::path::Path,
    node_db_path: &std::path::Path,
    backend_type: &str,
) -> BasicConfig {
    let mut config = BasicConfig::new();
    config.set_legacy("database_path", database_path.to_string_lossy());
    let node_db = config.section_mut("node_db");
    node_db.set("type", backend_type);
    node_db.set("path", node_db_path.to_string_lossy());
    node_db.set("online_delete", "256");
    config
}

#[test]
fn shamap_store_bootstrap_can_attach_node_store_to_application_root() {
    let dir = TempDir::new().expect("tempdir");
    let mut config = BasicConfig::new();
    config.set_legacy("database_path", dir.path().join("sql").to_string_lossy());
    let node_db = config.section_mut("node_db");
    node_db.set("type", "Memory");
    node_db.set("path", dir.path().join("node").to_string_lossy());

    let bootstrap = bootstrap_shamap_store(
        &config,
        false,
        128,
        1,
        8,
        64,
        2,
        &ManagerImp::new(),
        Arc::new(DummyScheduler) as Arc<dyn Scheduler>,
        Arc::new(NullJournal),
    )
    .expect("bootstrap");

    let mut root = ApplicationRoot::new(0).expect("root");
    let previous = bootstrap.attach_node_store(&mut root);
    assert!(previous.is_none());
    assert!(root.node_store().is_some());
    assert_eq!(
        root.node_store().as_ref().expect("node store").kind(),
        bootstrap.node_store_kind()
    );
}

#[test]
fn shamap_store_bootstrap_injects_large_profile_into_node_object_cache() {
    let dir = TempDir::new().expect("tempdir");
    let mut config = BasicConfig::new();
    config.set_legacy("database_path", dir.path().join("sql").to_string_lossy());
    config.section_mut("node_size").append("large");
    let node_db = config.section_mut("node_db");
    node_db.set("type", "Memory");
    node_db.set("path", dir.path().join("node").to_string_lossy());

    let bootstrap = bootstrap_shamap_store(
        &config,
        false,
        128,
        1,
        8,
        64,
        2,
        &ManagerImp::new(),
        Arc::new(DummyScheduler) as Arc<dyn Scheduler>,
        Arc::new(NullJournal),
    )
    .expect("bootstrap");

    assert_eq!(
        bootstrap
            .effective_node_db_config
            .get::<u64>("node_object_cache_target_nodes")
            .expect("valid target"),
        Some(4_194_304)
    );
    assert_eq!(
        bootstrap
            .effective_node_db_config
            .get::<u64>("cache_idle_seconds")
            .expect("valid age"),
        Some(7_200)
    );
    assert_eq!(
        bootstrap
            .effective_node_db_config
            .get::<u64>("cache_ttl_seconds")
            .expect("valid ttl"),
        Some(0)
    );
}
