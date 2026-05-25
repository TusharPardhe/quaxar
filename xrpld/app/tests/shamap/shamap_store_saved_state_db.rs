use app::{SHAMapStoreSavedState, SHAMapStoreSavedStateDb};
use basics::basic_config::BasicConfig;
use tempfile::TempDir;

#[test]
fn shamap_store_saved_state_db_bootstraps_and_round_trips_state() {
    let dir = TempDir::new().expect("tempdir");
    let mut config = BasicConfig::new();
    config.set_legacy("database_path", dir.path().to_string_lossy());

    let db = SHAMapStoreSavedStateDb::open(&config, "state").expect("state db");
    assert_eq!(db.get_can_delete().expect("can delete"), 0);
    assert_eq!(
        db.get_state().expect("state"),
        SHAMapStoreSavedState::default()
    );

    db.set_can_delete(600).expect("can delete");
    db.set_state(&SHAMapStoreSavedState {
        writable_db: "writable".to_owned(),
        archive_db: "archive".to_owned(),
        last_rotated: 700,
    })
    .expect("saved state");
    db.set_last_rotated(701).expect("last rotated");

    assert_eq!(db.get_can_delete().expect("can delete"), 600);
    assert_eq!(
        db.get_state().expect("state"),
        SHAMapStoreSavedState {
            writable_db: "writable".to_owned(),
            archive_db: "archive".to_owned(),
            last_rotated: 701,
        }
    );
}
