use basics::{base_uint::Uint256, basic_config::Section};
use nodestore::{
    DatabaseSource, DummyScheduler, Manager, ManagerImp, NodeObject, NodeObjectType, NullJournal,
};
use std::collections::BTreeMap;
use std::sync::Arc;
use tempfile::TempDir;

fn memory_section(path: &str) -> Section {
    let mut section = Section::new("node_db");
    section.set("type", "Memory");
    section.set("path", path);
    section
}

fn rocksdb_section(path: &str) -> Section {
    let mut section = Section::new("node_db");
    section.set("type", "RocksDB");
    section.set("path", path);
    section
}

fn sample_objects() -> Vec<NodeObject> {
    vec![
        NodeObject::new(
            NodeObjectType::Ledger,
            vec![1, 2, 3, 4],
            Uint256::from_array([0x11; 32]),
        ),
        NodeObject::new(
            NodeObjectType::AccountNode,
            vec![5, 6, 7],
            Uint256::from_array([0x22; 32]),
        ),
        NodeObject::new(
            NodeObjectType::TransactionNode,
            vec![8, 9, 10, 11, 12],
            Uint256::from_array([0x33; 32]),
        ),
    ]
}

fn export_map(objects: Vec<Arc<NodeObject>>) -> BTreeMap<Uint256, (NodeObjectType, Vec<u8>)> {
    objects
        .into_iter()
        .map(|object| {
            (
                *object.hash(),
                (object.object_type(), object.data().clone()),
            )
        })
        .collect()
}

struct ExportedObjects(Vec<Arc<NodeObject>>);

impl DatabaseSource for ExportedObjects {
    fn for_each(&self, callback: &mut dyn FnMut(Arc<NodeObject>)) {
        for object in &self.0 {
            callback(Arc::clone(object));
        }
    }
}

#[test]
fn manager_import_export_round_trips_between_memory_and_rocksdb() {
    let manager = ManagerImp::new();
    let scheduler: Arc<dyn nodestore::Scheduler> = Arc::new(DummyScheduler);
    let journal: Arc<dyn nodestore::NodeStoreJournal> = Arc::new(NullJournal);
    let rocks_dir = TempDir::new().expect("tempdir");

    let source = manager
        .make_database(
            0,
            Arc::clone(&scheduler),
            1,
            &memory_section("parity-source"),
            Arc::clone(&journal),
        )
        .expect("source database");
    let rocks = manager
        .make_database(
            0,
            Arc::clone(&scheduler),
            1,
            &rocksdb_section(&rocks_dir.path().join("parity-rocks").to_string_lossy()),
            Arc::clone(&journal),
        )
        .expect("rocksdb database");
    let restored = manager
        .make_database(0, scheduler, 1, &memory_section("parity-restored"), journal)
        .expect("restored database");

    for object in sample_objects() {
        source.store(
            object.object_type(),
            object.data().clone(),
            *object.hash(),
            1,
        );
    }

    let source_export = manager.export(source.as_ref());
    assert_eq!(source_export.len(), 3);

    manager.import(rocks.as_ref(), &ExportedObjects(source_export));

    let mut visited_hashes = Vec::new();
    manager.visit(rocks.as_ref(), &mut |object| {
        visited_hashes.push(*object.hash())
    });
    visited_hashes.sort();
    assert_eq!(
        visited_hashes,
        vec![
            Uint256::from_array([0x11; 32]),
            Uint256::from_array([0x22; 32]),
            Uint256::from_array([0x33; 32]),
        ]
    );

    manager.import(
        restored.as_ref(),
        &ExportedObjects(manager.export(rocks.as_ref())),
    );

    let source_map = export_map(manager.export(source.as_ref()));
    let restored_map = export_map(manager.export(restored.as_ref()));
    assert_eq!(restored_map, source_map);

    source.stop();
    rocks.stop();
    restored.stop();
}
