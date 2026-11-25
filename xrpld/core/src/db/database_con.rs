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
    /// A second, read-only connection to a WAL database. WAL readers see the
    /// last committed state and never wait on the writer, so RPC reads use
    /// it instead of queueing behind a ledger save that holds `connection`.
    /// `None` for in-memory or temporary databases and for non-WAL journals.
    read_connection: Option<Mutex<Connection>>,
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

    /// A session for read-only queries. On a WAL database this is a
    /// separate read-only connection that does not wait for the writer;
    /// otherwise it is the main session.
    pub fn get_read_session(&self) -> MutexGuard<'_, Connection> {
        match &self.read_connection {
            Some(read_connection) => read_connection
                .lock()
                .expect("database read connection mutex must not be poisoned"),
            None => self.get_session(),
        }
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

        let read_connection = open_wal_reader(&connection, path).map(Mutex::new);
        Ok(Self {
            connection: Mutex::new(connection),
            read_connection,
        })
    }
}

/// A read-only connection to `path` when the database there is a file in
/// WAL mode, so readers run beside the writer.
fn open_wal_reader(connection: &Connection, path: &Path) -> Option<Connection> {
    if path.as_os_str().is_empty() {
        return None;
    }
    let journal_mode: String = connection
        .query_row("PRAGMA journal_mode", [], |row| row.get(0))
        .ok()?;
    if !journal_mode.eq_ignore_ascii_case("wal") {
        return None;
    }
    Connection::open_with_flags(
        path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .ok()
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

    /// RPC reads must not queue behind a writer holding the main session (a
    /// ledger save): on mainnet `tx_history` p99 was 14 ms against rippled's
    /// 2.7 ms (2026-11-25). In WAL mode the read session runs beside it and
    /// sees the last committed rows.
    #[test]
    fn wal_read_session_does_not_wait_for_the_writer() {
        let dir = TempDir::new().expect("tempdir");
        let db = DatabaseCon::new_at_path(
            dir.path(),
            "transaction.db",
            &["PRAGMA journal_mode=WAL;".to_owned()],
            TRANSACTION_DB_INIT,
        )
        .expect("database must initialize");
        let count = |connection: &rusqlite::Connection| -> i64 {
            connection
                .query_row("SELECT COUNT(*) FROM Transactions", [], |row| row.get(0))
                .expect("count")
        };

        let writer = db.get_session();
        writer
            .execute_batch(
                "BEGIN; INSERT INTO Transactions (TransID, TransType) VALUES ('A', 'Payment');",
            )
            .expect("open write transaction");
        let (sender, receiver) = std::sync::mpsc::channel();
        let seen = std::thread::scope(|scope| {
            scope.spawn(|| {
                let _ = sender.send(count(&db.get_read_session()));
            });
            let seen = receiver.recv_timeout(std::time::Duration::from_secs(5));
            // Release the writer before the scope joins the reader, so a
            // reader stuck on the main session finishes and the test fails
            // instead of hanging.
            writer.execute_batch("COMMIT;").expect("commit");
            drop(writer);
            seen
        });
        assert_eq!(
            seen.expect("the read session must not wait for the writer"),
            0,
            "uncommitted rows are not visible"
        );
        assert_eq!(
            count(&db.get_read_session()),
            1,
            "committed rows are visible"
        );
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
