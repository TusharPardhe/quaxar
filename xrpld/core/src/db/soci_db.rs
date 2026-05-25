use basics::basic_config::BasicConfig;
use rusqlite::{Connection, OpenFlags, ffi};
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DBConfig {
    connection_string: String,
}

impl DBConfig {
    pub fn from_db_path(db_path: impl Into<String>) -> Self {
        Self {
            connection_string: db_path.into(),
        }
    }

    pub fn from_config(config: &BasicConfig, db_name: &str) -> Result<Self, String> {
        Ok(Self::from_db_path(get_soci_init(config, db_name)?))
    }

    pub fn connection_string(&self) -> &str {
        &self.connection_string
    }

    pub fn open(&self) -> Result<Connection, String> {
        open_sqlite_connection("sqlite", &self.connection_string)
    }
}

pub fn open_sqlite_connection_from_config(
    config: &BasicConfig,
    db_name: &str,
) -> Result<Connection, String> {
    DBConfig::from_config(config, db_name)?.open()
}

pub fn open_sqlite_connection(
    backend_name: &str,
    connection_string: &str,
) -> Result<Connection, String> {
    if backend_name != "sqlite" {
        return Err(format!("Unsupported soci backend: {backend_name}"));
    }

    Connection::open_with_flags(
        connection_string,
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_CREATE,
    )
    .map_err(|error| error.to_string())
}

pub fn get_kb_used_all(_connection: &Connection) -> u32 {
    let used = unsafe { ffi::sqlite3_memory_used() };
    (used / 1024) as u32
}

pub fn get_kb_used_db(connection: &Connection) -> Result<u32, String> {
    let mut current = 0;
    let mut high_water = 0;
    let result = unsafe {
        ffi::sqlite3_db_status(
            connection.handle(),
            ffi::SQLITE_DBSTATUS_CACHE_USED,
            &mut current,
            &mut high_water,
            0,
        )
    };

    if result != ffi::SQLITE_OK {
        return Err(format!("sqlite3_db_status failed with code {result}"));
    }

    Ok((current / 1024) as u32)
}

pub fn vec_from_blob(blob: &[u8]) -> Vec<u8> {
    blob.to_vec()
}

pub fn string_from_blob(blob: &[u8]) -> String {
    String::from_utf8_lossy(blob).into_owned()
}

pub fn blob_from_bytes(bytes: &[u8]) -> Vec<u8> {
    bytes.to_vec()
}

pub fn blob_from_string(value: &str) -> Vec<u8> {
    value.as_bytes().to_vec()
}

fn get_soci_sqlite_init(name: &str, dir: &str, ext: &str) -> Result<String, String> {
    if name.is_empty() {
        return Err(format!(
            "Sqlite databases must specify a dir and a name. Name: {name} Dir: {dir}"
        ));
    }

    let mut path = PathBuf::from(dir);
    if path.is_dir() {
        path.push(format!("{name}{ext}"));
    }
    Ok(path.to_string_lossy().into_owned())
}

fn get_soci_init(config: &BasicConfig, db_name: &str) -> Result<String, String> {
    let sqdb = config.section("sqdb");
    let backend_name = sqdb
        .get::<String>("backend")
        .map_err(|error| error.to_string())?
        .unwrap_or_else(|| "sqlite".to_owned());

    if backend_name != "sqlite" {
        return Err(format!("Unsupported soci backend: {backend_name}"));
    }

    let path = config
        .legacy("database_path")
        .map_err(|error| error.to_string())?;
    let ext = if db_name == "validators" || db_name == "peerfinder" {
        ".sqlite"
    } else {
        ".db"
    };
    get_soci_sqlite_init(db_name, &path, ext)
}

#[cfg(test)]
mod tests {
    use super::{
        DBConfig, blob_from_bytes, blob_from_string, open_sqlite_connection, string_from_blob,
        vec_from_blob,
    };
    use basics::basic_config::BasicConfig;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn db_config_uses_cpp_sqlite_path_rules() {
        let dir = TempDir::new().expect("tempdir");
        let mut config = BasicConfig::new();
        config.set_legacy("database_path", dir.path().to_string_lossy());

        let tx = DBConfig::from_config(&config, "transactions").expect("dbconfig");
        let validators = DBConfig::from_config(&config, "validators").expect("dbconfig");

        assert!(tx.connection_string().ends_with("transactions.db"));
        assert!(
            validators
                .connection_string()
                .ends_with("validators.sqlite")
        );
    }

    #[test]
    fn sqlite_open_rejects_unsupported_backends() {
        let error = open_sqlite_connection("postgresql", "ignored").expect_err("backend must fail");
        assert_eq!(error, "Unsupported soci backend: postgresql");
    }

    #[test]
    fn blob_helpers_round_trip() {
        let bytes = vec![1, 2, 3, 4];
        assert_eq!(vec_from_blob(&bytes), bytes);
        assert_eq!(blob_from_bytes(&bytes), bytes);
        assert_eq!(string_from_blob(b"abc"), "abc");
        assert_eq!(blob_from_string("abc"), b"abc");
    }

    #[test]
    fn sqlite_dbconfig_can_open_real_file() {
        let dir = TempDir::new().expect("tempdir");
        let path = dir.path().join("example.db");
        let connection = DBConfig::from_db_path(path.to_string_lossy())
            .open()
            .expect("open");
        connection
            .execute("CREATE TABLE example (id INTEGER)", [])
            .expect("create");
        drop(connection);
        assert!(fs::metadata(path).is_ok());
    }
}
