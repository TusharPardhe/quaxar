use crate::SHAMapStoreSavedState;
use basics::basic_config::Section;
use std::fs;
use std::path::{Path, PathBuf};

pub const SHAMAP_STORE_DB_PREFIX: &str = "xrpldb";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SHAMapStorePathPlan {
    pub state: SHAMapStoreSavedState,
    pub stale_paths: Vec<PathBuf>,
}

impl SHAMapStorePathPlan {
    pub fn cleanup_stale_paths(&self) -> Result<(), String> {
        cleanup_shamap_store_stale_paths(&self.stale_paths)
    }
}

pub fn reconcile_shamap_store_paths(
    node_db: &Section,
    state: &SHAMapStoreSavedState,
) -> Result<SHAMapStorePathPlan, String> {
    let base = PathBuf::from(
        node_db
            .get::<String>("path")
            .ok()
            .flatten()
            .unwrap_or_default(),
    );

    ensure_directory(&base)?;

    let mut next_state = state.clone();
    if rewrite_saved_path_parent(&base, &mut next_state.writable_db) {
        rewrite_saved_path_parent(&base, &mut next_state.archive_db);
    }

    let mut writable_exists = false;
    let mut archive_exists = false;
    let mut stale_paths = Vec::new();
    for entry in fs::read_dir(&base).map_err(|error| error.to_string())? {
        let path = entry.map_err(|error| error.to_string())?.path();
        let as_string = path.to_string_lossy();
        if !next_state.writable_db.is_empty() && next_state.writable_db == as_string {
            writable_exists = true;
        } else if !next_state.archive_db.is_empty() && next_state.archive_db == as_string {
            archive_exists = true;
        } else if path.file_stem().and_then(|stem| stem.to_str()) == Some(SHAMAP_STORE_DB_PREFIX) {
            stale_paths.push(path);
        }
    }

    let missing_stored_path = (!writable_exists && !next_state.writable_db.is_empty())
        || (!archive_exists && !next_state.archive_db.is_empty());
    let mismatched_existence = writable_exists != archive_exists;
    let mismatched_empty = next_state.writable_db.is_empty() != next_state.archive_db.is_empty();
    if missing_stored_path || mismatched_existence || mismatched_empty {
        return Err("state db error".to_owned());
    }

    stale_paths.sort();
    Ok(SHAMapStorePathPlan {
        state: next_state,
        stale_paths,
    })
}

pub fn cleanup_shamap_store_stale_paths(paths: &[PathBuf]) -> Result<(), String> {
    for path in paths {
        if !path.exists() {
            continue;
        }

        if path.is_dir() {
            fs::remove_dir_all(path).map_err(|error| error.to_string())?;
        } else {
            fs::remove_file(path).map_err(|error| error.to_string())?;
        }
    }

    Ok(())
}

fn ensure_directory(path: &Path) -> Result<(), String> {
    if path.exists() {
        if !path.is_dir() {
            return Err("node db path must be a directory.".to_owned());
        }
        return Ok(());
    }

    fs::create_dir_all(path).map_err(|error| error.to_string())
}

fn rewrite_saved_path_parent(base: &Path, saved_path: &mut String) -> bool {
    if saved_path.is_empty() {
        return false;
    }

    let stored = PathBuf::from(saved_path.clone());
    if stored.parent() == Some(base) {
        return false;
    }

    if let Some(filename) = stored.file_name() {
        *saved_path = base.join(filename).to_string_lossy().into_owned();
        return true;
    }

    false
}

#[cfg(test)]
mod tests {
    use super::{
        SHAMAP_STORE_DB_PREFIX, cleanup_shamap_store_stale_paths, reconcile_shamap_store_paths,
    };
    use crate::SHAMapStoreSavedState;
    use basics::basic_config::Section;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn paths_rewrite_stored_backend_names_when_node_db_path_moves() {
        let new_dir = TempDir::new().expect("tempdir");
        let mut section = Section::new("node_db");
        section.set("path", new_dir.path().to_string_lossy());
        let state = SHAMapStoreSavedState {
            writable_db: "/old/xrpldb.0000".to_owned(),
            archive_db: "/old/xrpldb.0001".to_owned(),
            last_rotated: 10,
        };

        fs::create_dir(new_dir.path().join("xrpldb.0000")).expect("writable");
        fs::create_dir(new_dir.path().join("xrpldb.0001")).expect("archive");
        let plan = reconcile_shamap_store_paths(&section, &state).expect("plan");

        assert!(plan.state.writable_db.ends_with("xrpldb.0000"));
        assert!(plan.state.archive_db.ends_with("xrpldb.0001"));
    }

    #[test]
    fn paths_collect_stale_xrpldb_directories_for_cleanup() {
        let dir = TempDir::new().expect("tempdir");
        let mut section = Section::new("node_db");
        section.set("path", dir.path().to_string_lossy());
        fs::create_dir(dir.path().join("xrpldb.0000")).expect("stale");
        fs::create_dir(dir.path().join("xrpldb.0001")).expect("stale");
        fs::create_dir(dir.path().join("other.0002")).expect("other");

        let plan = reconcile_shamap_store_paths(&section, &SHAMapStoreSavedState::default())
            .expect("plan");
        assert_eq!(plan.stale_paths.len(), 2);
        assert!(plan.stale_paths.iter().all(|path| {
            path.file_stem().and_then(|stem| stem.to_str()) == Some(SHAMAP_STORE_DB_PREFIX)
        }));
    }

    #[test]
    fn cleanup_shamap_store_stale_paths_removes_files_and_directories_remove_all() {
        let dir = TempDir::new().expect("tempdir");
        let stale_dir = dir.path().join("xrpldb.0000");
        let stale_file = dir.path().join("xrpldb.0001");
        fs::create_dir(&stale_dir).expect("stale dir");
        fs::write(&stale_file, b"stale").expect("stale file");

        cleanup_shamap_store_stale_paths(&[stale_dir.clone(), stale_file.clone()])
            .expect("cleanup should succeed");

        assert!(!stale_dir.exists());
        assert!(!stale_file.exists());
    }

    #[test]
    fn path_plan_cleanup_removes_collected_stale_paths() {
        let dir = TempDir::new().expect("tempdir");
        let mut section = Section::new("node_db");
        section.set("path", dir.path().to_string_lossy());
        let stale_dir = dir.path().join("xrpldb.0000");
        let stale_file = dir.path().join("xrpldb.0001");
        fs::create_dir(&stale_dir).expect("stale dir");
        fs::write(&stale_file, b"stale").expect("stale file");

        let plan = reconcile_shamap_store_paths(&section, &SHAMapStoreSavedState::default())
            .expect("plan");
        assert_eq!(
            plan.stale_paths,
            vec![stale_dir.clone(), stale_file.clone()]
        );

        plan.cleanup_stale_paths().expect("cleanup");

        assert!(!stale_dir.exists());
        assert!(!stale_file.exists());
    }

    #[test]
    fn paths_detect_cpp_corruption_cases() {
        let dir = TempDir::new().expect("tempdir");
        let mut section = Section::new("node_db");
        section.set("path", dir.path().to_string_lossy());
        fs::create_dir(dir.path().join("xrpldb.0000")).expect("writable");
        let state = SHAMapStoreSavedState {
            writable_db: dir
                .path()
                .join("xrpldb.0000")
                .to_string_lossy()
                .into_owned(),
            archive_db: dir
                .path()
                .join("xrpldb.0001")
                .to_string_lossy()
                .into_owned(),
            last_rotated: 1,
        };

        let error = reconcile_shamap_store_paths(&section, &state).expect_err("must fail");
        assert_eq!(error, "state db error");
    }
}
