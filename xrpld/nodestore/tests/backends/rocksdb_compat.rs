use basics::{base_uint::Uint256, basic_config::Section};
use nodestore::{
    Factory, NodeObject, NodeObjectType, NuDbCompatibilityFactory, NuDbContext, NuDbFactory,
};
use std::sync::Arc;
use tempfile::TempDir;

fn nudb_section(path: &str) -> Section {
    let mut section = Section::new("node_db");
    section.set("type", "NuDB");
    section.set("path", path);
    section
}

fn sample_object(fill: u8, payload: &[u8]) -> Arc<NodeObject> {
    NodeObject::create_object(
        NodeObjectType::Ledger,
        payload.to_vec(),
        Uint256::from_array([fill; 32]),
    )
}

#[test]
fn nudb_factory_alias_builds_the_native_backend() {
    let dir = TempDir::new().expect("tempdir");
    let path = dir.path().join("nudb-alias");
    let section = nudb_section(&path.to_string_lossy());

    let backend = NuDbFactory::new()
        .create_instance(
            NodeObject::KEY_BYTES,
            &section,
            0,
            Arc::new(nodestore::DummyScheduler),
            Arc::new(nodestore::NullJournal),
        )
        .expect("public NuDbFactory alias should stay usable");

    assert_eq!(backend.get_block_size(), Some(4096));
    backend.open(true).expect("open");
    let object = sample_object(0xA1, &[1, 2, 3]);
    backend.store(Arc::clone(&object));
    let (fetched, status) = backend.fetch(object.hash());
    assert_eq!(status, nodestore::Status::Ok);
    assert_eq!(fetched.expect("stored object").data(), object.data());
    backend.close().expect("close");
    assert!(path.join("nudb.dat").exists());
    assert!(path.join("nudb.key").exists());
    assert!(path.join("nudb.log").exists());
}

#[test]
fn nudb_typed_context_entrypoint_preserves_deterministic_headers() {
    let dir = TempDir::new().expect("tempdir");
    let path = dir.path().join("nudb-context");
    let section = nudb_section(&path.to_string_lossy());
    let mut context = NuDbContext::new(nodestore::NUDB_APPNUM, 8, 9);

    let backend = NuDbFactory::new()
        .create_instance_with_nudb_context(
            NodeObject::KEY_BYTES,
            &section,
            0,
            Arc::new(nodestore::DummyScheduler),
            &mut context,
            Arc::new(nodestore::NullJournal),
        )
        .expect("NuDB factory should handle the typed context entrypoint")
        .expect("NuDB context path should build a native NuDB backend");

    backend.open(true).expect("open");
    let object = sample_object(0xB2, &[9, 8, 7, 6]);
    backend.store(Arc::clone(&object));
    let (fetched, status) = backend.fetch(object.hash());
    assert_eq!(status, nodestore::Status::Ok);
    assert_eq!(fetched.expect("stored object").data(), object.data());
    backend.close().expect("close");
}

#[test]
fn nudb_aliases_share_the_same_native_factory_type() {
    let factory = NuDbFactory::new();
    let compat = NuDbCompatibilityFactory::new();

    assert_eq!(factory.get_name(), "NuDB");
    assert_eq!(compat.get_name(), "NuDB");
}
