use crate::DBConfig;
use basics::basic_config::BasicConfig;
use rusqlite::{Connection, OptionalExtension, params};
use std::sync::Mutex;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SavedState {
    pub writable_db: String,
    pub archive_db: String,
    pub last_rotated: u32,
}

#[derive(Debug)]
pub struct StateDb {
    connection: Mutex<Connection>,
}

impl StateDb {
    pub fn open(config: &BasicConfig, db_name: &str) -> Result<Self, String> {
        let connection = DBConfig::from_config(config, db_name)?.open()?;
        Self::from_connection(connection)
    }

    pub fn from_connection(connection: Connection) -> Result<Self, String> {
        init_state_db(&connection)?;
        Ok(Self {
            connection: Mutex::new(connection),
        })
    }

    pub fn get_can_delete(&self) -> Result<u32, String> {
        let connection = self.connection.lock().map_err(|error| error.to_string())?;
        connection
            .query_row(
                "SELECT CanDeleteSeq FROM CanDelete WHERE Key = 1;",
                [],
                |row| row.get(0),
            )
            .map_err(|error| error.to_string())
    }

    pub fn set_can_delete(&self, can_delete: u32) -> Result<u32, String> {
        let connection = self.connection.lock().map_err(|error| error.to_string())?;
        connection
            .execute(
                "UPDATE CanDelete SET CanDeleteSeq = ?1 WHERE Key = 1;",
                params![can_delete],
            )
            .map_err(|error| error.to_string())?;
        Ok(can_delete)
    }

    pub fn get_state(&self) -> Result<SavedState, String> {
        let connection = self.connection.lock().map_err(|error| error.to_string())?;
        connection
            .query_row(
                "SELECT WritableDb, ArchiveDb, LastRotatedLedger FROM DbState WHERE Key = 1;",
                [],
                |row| {
                    Ok(SavedState {
                        writable_db: row.get(0)?,
                        archive_db: row.get(1)?,
                        last_rotated: row.get(2)?,
                    })
                },
            )
            .map_err(|error| error.to_string())
    }

    pub fn set_state(&self, state: &SavedState) -> Result<(), String> {
        let connection = self.connection.lock().map_err(|error| error.to_string())?;
        connection
            .execute(
                "UPDATE DbState \
                 SET WritableDb = ?1, ArchiveDb = ?2, LastRotatedLedger = ?3 \
                 WHERE Key = 1;",
                params![state.writable_db, state.archive_db, state.last_rotated],
            )
            .map_err(|error| error.to_string())?;
        Ok(())
    }

    pub fn set_last_rotated(&self, seq: u32) -> Result<(), String> {
        let connection = self.connection.lock().map_err(|error| error.to_string())?;
        connection
            .execute(
                "UPDATE DbState SET LastRotatedLedger = ?1 WHERE Key = 1;",
                params![seq],
            )
            .map_err(|error| error.to_string())?;
        Ok(())
    }
}

fn init_state_db(connection: &Connection) -> Result<(), String> {
    connection
        .execute_batch(
            "PRAGMA synchronous=FULL;
             CREATE TABLE IF NOT EXISTS DbState (
               Key INTEGER PRIMARY KEY,
               WritableDb TEXT,
               ArchiveDb TEXT,
               LastRotatedLedger INTEGER
             );
             CREATE TABLE IF NOT EXISTS CanDelete (
               Key INTEGER PRIMARY KEY,
               CanDeleteSeq INTEGER
             );",
        )
        .map_err(|error| error.to_string())?;

    let state_count: i64 = connection
        .query_row("SELECT COUNT(Key) FROM DbState WHERE Key = 1;", [], |row| {
            row.get(0)
        })
        .map_err(|error| error.to_string())?;
    if state_count == 0 {
        connection
            .execute("INSERT INTO DbState VALUES (1, '', '', 0);", [])
            .map_err(|error| error.to_string())?;
    }

    let can_delete_count: Option<i64> = connection
        .query_row(
            "SELECT COUNT(Key) FROM CanDelete WHERE Key = 1;",
            [],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| error.to_string())?;
    if can_delete_count.unwrap_or_default() == 0 {
        connection
            .execute("INSERT INTO CanDelete VALUES (1, 0);", [])
            .map_err(|error| error.to_string())?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{SavedState, StateDb};
    use basics::basic_config::BasicConfig;
    use rusqlite::Connection;
    use tempfile::TempDir;

    #[test]
    fn state_db_bootstraps_default_rows_state_tables() {
        let connection = Connection::open_in_memory().expect("sqlite");
        let db = StateDb::from_connection(connection).expect("state db");

        assert_eq!(db.get_can_delete().expect("can delete"), 0);
        assert_eq!(db.get_state().expect("state"), SavedState::default());
    }

    #[test]
    fn state_db_round_trips_saved_state_and_last_rotated() {
        let connection = Connection::open_in_memory().expect("sqlite");
        let db = StateDb::from_connection(connection).expect("state db");
        let saved = SavedState {
            writable_db: "writable".to_owned(),
            archive_db: "archive".to_owned(),
            last_rotated: 900,
        };

        db.set_state(&saved).expect("save state");
        assert_eq!(db.get_state().expect("saved state"), saved);

        db.set_last_rotated(901).expect("last rotated");
        assert_eq!(db.get_state().expect("saved state").last_rotated, 901);
    }

    #[test]
    fn state_db_uses_cpp_database_path_rules_for_state_db() {
        let dir = TempDir::new().expect("tempdir");
        let mut config = BasicConfig::new();
        config.set_legacy("database_path", dir.path().to_string_lossy());

        let db = StateDb::open(&config, "state").expect("state db");
        db.set_can_delete(123).expect("can delete");
        assert_eq!(db.get_can_delete().expect("can delete"), 123);
        assert!(dir.path().join("state.db").exists());
    }
}
