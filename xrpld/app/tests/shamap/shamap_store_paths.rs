use app::{SHAMapStoreSavedState, reconcile_shamap_store_paths};
use basics::basic_config::Section;
use std::fs;
use tempfile::TempDir;

#[test]
fn shamap_store_paths_rewrite_and_cleanup_match_dbpaths_boundary() {
    let new_dir = TempDir::new().expect("tempdir");
    fs::create_dir(new_dir.path().join("xrpldb.0000")).expect("writable");
    fs::create_dir(new_dir.path().join("xrpldb.0001")).expect("archive");
    fs::create_dir(new_dir.path().join("xrpldb.0002")).expect("stale");

    let mut section = Section::new("node_db");
    section.set("path", new_dir.path().to_string_lossy());
    let state = SHAMapStoreSavedState {
        writable_db: "/old/path/xrpldb.0000".to_owned(),
        archive_db: "/old/path/xrpldb.0001".to_owned(),
        last_rotated: 10,
    };

    let plan = reconcile_shamap_store_paths(&section, &state).expect("plan");
    assert!(plan.state.writable_db.ends_with("xrpldb.0000"));
    assert!(plan.state.archive_db.ends_with("xrpldb.0001"));
    assert_eq!(plan.stale_paths.len(), 1);
    assert!(plan.stale_paths[0].ends_with("xrpldb.0002"));
}
