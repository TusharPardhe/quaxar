use basics::{base_uint::Uint256, basic_config::Section};
use nodestore::{
    DummyScheduler, Manager, ManagerImp, NodeObject, NodeObjectType, NodeStoreJournal, NuDbContext,
    NullJournal, Status,
};
use std::sync::Arc;
use tempfile::TempDir;

fn section(path: &str) -> Section {
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
fn nudb_factory_round_trips_and_keeps_deterministic_context() {
    let dir = TempDir::new().expect("tempdir");
    let path = dir.path().join("nudb");
    let mut section = section(&path.to_string_lossy());
    section.set("nudb_block_size", "8192");

    let manager = ManagerImp::new();
    let scheduler: Arc<dyn nodestore::Scheduler> = Arc::new(DummyScheduler);
    let journal: Arc<dyn NodeStoreJournal> = Arc::new(NullJournal);
    let mut context = NuDbContext::new(nodestore::NUDB_APPNUM, 8, 9);

    let backend = manager
        .make_backend_with_nudb_context(
            &section,
            0,
            Arc::clone(&scheduler),
            &mut context,
            Arc::clone(&journal),
        )
        .expect("typed NuDB context backend");

    assert_eq!(backend.get_block_size(), Some(8192));
    assert_eq!(backend.fd_required(), 3);
    backend.open(true).expect("open");

    let object = sample_object(0x44, &[1, 2, 3, 4]);
    backend.store(Arc::clone(&object));
    let (fetched, status) = backend.fetch(object.hash());
    assert_eq!(status, Status::Ok);
    assert_eq!(fetched.expect("stored object").data(), object.data());
    backend.verify();
    backend.close().expect("close");

    let mut reopen_context = NuDbContext::new(nodestore::NUDB_APPNUM, 8, 9);
    let reopened = manager
        .make_backend_with_nudb_context(&section, 0, scheduler, &mut reopen_context, journal)
        .expect("typed NuDB context backend");
    reopened.open(false).expect("reopen");
    let (reopened_object, status) = reopened.fetch(object.hash());
    assert_eq!(status, Status::Ok);
    assert_eq!(
        reopened_object.expect("reopened object").data(),
        object.data()
    );
}
