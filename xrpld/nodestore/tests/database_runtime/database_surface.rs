use basics::{base_uint::Uint256, basic_config::Section};
use nodestore::{
    Database, DatabaseImporter, DatabaseRotating, DatabaseSource, DummyScheduler, Manager,
    ManagerImp, NodeObject, NodeObjectType, NullJournal,
};
use std::sync::Arc;

fn memory_section(path: &str) -> Section {
    let mut section = Section::new("node_db");
    section.set("type", "Memory");
    section.set("path", path);
    section
}

fn sample_object(fill: u8) -> Arc<NodeObject> {
    NodeObject::create_object(
        NodeObjectType::Ledger,
        vec![fill, fill + 1, fill + 2],
        Uint256::from_array([fill; 32]),
    )
}

struct VectorSource {
    objects: Vec<Arc<NodeObject>>,
}

impl DatabaseSource for VectorSource {
    fn for_each(&self, callback: &mut dyn FnMut(Arc<NodeObject>)) {
        for object in &self.objects {
            callback(Arc::clone(object));
        }
    }
}

fn import_all(importer: &dyn DatabaseImporter, source: &dyn DatabaseSource) {
    importer.import_database(source);
}

fn collect_hashes(source: &dyn DatabaseSource) -> Vec<Uint256> {
    let mut hashes = Vec::new();
    source.for_each(&mut |object| hashes.push(*object.hash()));
    hashes
}

#[test]
fn node_and_rotating_owners_share_the_same_database_source_and_importer_surface() {
    let manager = ManagerImp::new();
    let scheduler: Arc<dyn nodestore::Scheduler> = Arc::new(DummyScheduler);
    let journal: Arc<dyn nodestore::NodeStoreJournal> = Arc::new(NullJournal);

    let source = VectorSource {
        objects: vec![sample_object(0x11), sample_object(0x22)],
    };

    let node_section = memory_section("surface-node");
    let node_db: Arc<dyn Database> = manager
        .make_database(
            0,
            Arc::clone(&scheduler),
            1,
            &node_section,
            Arc::clone(&journal),
        )
        .expect("node database");
    import_all(node_db.as_ref(), &source);

    assert_eq!(
        collect_hashes(node_db.as_ref()),
        vec![*source.objects[0].hash(), *source.objects[1].hash()]
    );
    for object in &source.objects {
        let fetched = node_db
            .fetch_node_object(object.hash(), 1, nodestore::FetchType::Synchronous, false)
            .expect("node database should have imported object");
        assert_eq!(fetched.data(), object.data());
    }

    let writable_section = memory_section("surface-rotating-writable");
    let archive_section = memory_section("surface-rotating-archive");
    let rotating_db: Arc<dyn DatabaseRotating> = manager
        .make_rotating_database(
            0,
            Arc::clone(&scheduler),
            1,
            &writable_section,
            &archive_section,
            &writable_section,
            Arc::clone(&journal),
        )
        .expect("rotating database");
    import_all(rotating_db.as_ref(), &source);

    assert_eq!(
        collect_hashes(rotating_db.as_ref()),
        vec![*source.objects[0].hash(), *source.objects[1].hash()]
    );
    for object in &source.objects {
        let fetched = rotating_db
            .fetch_node_object(object.hash(), 1, nodestore::FetchType::Synchronous, false)
            .expect("rotating database should have imported object");
        assert_eq!(fetched.data(), object.data());
    }

    let fanout_section = memory_section("surface-fanout");
    let fanout_db: Arc<dyn Database> = manager
        .make_database(0, scheduler, 1, &fanout_section, journal)
        .expect("fanout database");
    import_all(fanout_db.as_ref(), rotating_db.as_ref());

    assert_eq!(
        collect_hashes(fanout_db.as_ref()),
        vec![*source.objects[0].hash(), *source.objects[1].hash()]
    );
}
