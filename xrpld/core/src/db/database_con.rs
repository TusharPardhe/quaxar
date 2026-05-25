use crate::start_up_type::StartUpType;
use rusqlite::Connection;
use std::ops::{Deref, DerefMut};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

pub struct LockedConnection<'a> {
    guard: MutexGuard<'a, Connection>,
}

impl<'a> LockedConnection<'a> {
    pub fn get(&mut self) -> &mut Connection {
        &mut self.guard
    }
}

impl Deref for LockedConnection<'_> {
    type Target = Connection;

    fn deref(&self) -> &Self::Target {
        &self.guard
    }
}

impl DerefMut for LockedConnection<'_> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.guard
    }
}

#[derive(Debug, Clone)]
pub struct DatabaseConSetup {
    pub start_up: StartUpType,
    pub stand_alone: bool,
    pub data_dir: PathBuf,
    pub common_pragma: Option<Vec<String>>,
    pub tx_pragma: Vec<String>,
    pub lgr_pragma: Vec<String>,
}

impl Default for DatabaseConSetup {
    fn default() -> Self {
        Self {
            start_up: StartUpType::Normal,
            stand_alone: false,
            data_dir: PathBuf::new(),
            common_pragma: None,
            tx_pragma: Vec::new(),
            lgr_pragma: Vec::new(),
        }
    }
}

impl DatabaseConSetup {
    pub fn common_pragma(&self) -> Option<&[String]> {
        self.common_pragma.as_deref()
    }
}

pub struct DatabaseCon {
    connection: Mutex<Connection>,
}

impl DatabaseCon {
    pub fn new_from_setup(
        setup: &DatabaseConSetup,
        db_name: &str,
        pragma: &[String],
        init_sql: &[&str],
    ) -> Result<Self, String> {
        let path = if setup.stand_alone
            && !matches!(
                setup.start_up,
                StartUpType::Load | StartUpType::LoadFile | StartUpType::Replay
            ) {
            PathBuf::new()
        } else {
            setup.data_dir.join(db_name)
        };

        Self::new_internal(&path, setup.common_pragma(), pragma, init_sql)
    }

    pub fn new_at_path(
        data_dir: &Path,
        db_name: &str,
        pragma: &[String],
        init_sql: &[&str],
    ) -> Result<Self, String> {
        Self::new_internal(&data_dir.join(db_name), None, pragma, init_sql)
    }

    pub fn get_session(&self) -> MutexGuard<'_, Connection> {
        self.connection
            .lock()
            .expect("database connection mutex must not be poisoned")
    }

    pub fn checkout_db(&self) -> LockedConnection<'_> {
        LockedConnection {
            guard: self
                .connection
                .lock()
                .expect("database connection mutex must not be poisoned"),
        }
    }

    fn new_internal(
        path: &Path,
        common_pragma: Option<&[String]>,
        pragma: &[String],
        init_sql: &[&str],
    ) -> Result<Self, String> {
        let connection = crate::open_sqlite_connection("sqlite", &path.to_string_lossy())?;

        for statement in pragma {
            connection
                .execute_batch(statement)
                .map_err(|error| error.to_string())?;
        }

        if let Some(common_pragma) = common_pragma {
            for statement in common_pragma {
                connection
                    .execute_batch(statement)
                    .map_err(|error| error.to_string())?;
            }
        }

        for sql in init_sql {
            connection
                .execute_batch(sql)
                .map_err(|error| error.to_string())?;
        }

        Ok(Self {
            connection: Mutex::new(connection),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{DatabaseCon, DatabaseConSetup};
    use crate::{LEDGER_DB_INIT, LEDGER_DB_NAME, StartUpType, TRANSACTION_DB_INIT};
    use tempfile::TempDir;

    #[test]
    fn database_con_creates_ledger_schema() {
        let dir = TempDir::new().expect("tempdir");
        let setup = DatabaseConSetup {
            data_dir: dir.path().to_path_buf(),
            ..Default::default()
        };
        let db = DatabaseCon::new_from_setup(&setup, LEDGER_DB_NAME, &[], LEDGER_DB_INIT)
            .expect("database must initialize");

        let connection = db.get_session();
        let exists: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='Ledgers'",
                [],
                |row| row.get(0),
            )
            .expect("query");
        assert_eq!(exists, 1);
    }

    #[test]
    fn standalone_non_load_uses_temporary_sqlite_target() {
        let setup = DatabaseConSetup {
            start_up: StartUpType::Normal,
            stand_alone: true,
            ..Default::default()
        };
        let db = DatabaseCon::new_from_setup(&setup, "temp.db", &[], TRANSACTION_DB_INIT)
            .expect("database must initialize");
        let connection = db.get_session();
        let exists: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='Transactions'",
                [],
                |row| row.get(0),
            )
            .expect("query");
        assert_eq!(exists, 1);
    }
}
