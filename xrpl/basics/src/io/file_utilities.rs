//! Rust port of `xrpl/basics/FileUtilities.h`.
//!
//! The reference API returns a `std::string`, which can hold arbitrary bytes. Rust
//! separates UTF-8 text (`String`) from raw bytes (`Vec<u8>`), so this
//! migration boundary returns `Blob` to preserve the byte-level behavior.

use crate::blob::Blob;
use std::fmt;
use std::fs;
use std::io;
use std::path::Path;

#[derive(Debug)]
pub enum FileUtilitiesError {
    Io(io::Error),
    FileTooLarge { size: u64, max_size: usize },
}

impl FileUtilitiesError {
    pub fn io_kind(&self) -> Option<io::ErrorKind> {
        match self {
            Self::Io(error) => Some(error.kind()),
            Self::FileTooLarge { .. } => None,
        }
    }

    pub fn is_file_too_large(&self) -> bool {
        matches!(self, Self::FileTooLarge { .. })
    }
}

impl fmt::Display for FileUtilitiesError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "{error}"),
            Self::FileTooLarge { size, max_size } => {
                write!(formatter, "file size {size} exceeds maximum {max_size}")
            }
        }
    }
}

impl std::error::Error for FileUtilitiesError {}

impl From<io::Error> for FileUtilitiesError {
    fn from(value: io::Error) -> Self {
        Self::Io(value)
    }
}

pub fn get_file_contents(
    source_path: impl AsRef<Path>,
    max_size: Option<usize>,
) -> Result<Blob, FileUtilitiesError> {
    let full_path = fs::canonicalize(source_path)?;
    let metadata = fs::metadata(&full_path)?;

    if let Some(max_size) = max_size {
        let size = metadata.len();
        if size > max_size as u64 {
            return Err(FileUtilitiesError::FileTooLarge { size, max_size });
        }
    }

    Ok(fs::read(full_path)?)
}

pub fn write_file_contents(
    dest_path: impl AsRef<Path>,
    contents: impl AsRef<[u8]>,
) -> Result<(), FileUtilitiesError> {
    fs::write(dest_path, contents).map_err(Into::into)
}

#[cfg(test)]
mod tests {
    use super::{FileUtilitiesError, get_file_contents, write_file_contents};
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::time::{SystemTime, UNIX_EPOCH};

    struct TempDirGuard {
        path: PathBuf,
    }

    impl TempDirGuard {
        fn new(prefix: &str) -> Self {
            let unique = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock should be after unix epoch")
                .as_nanos();
            let path = std::env::temp_dir().join(format!(
                "xrpl-rust-migration-{prefix}-{}-{unique}",
                std::process::id()
            ));
            fs::create_dir_all(&path).expect("temp test directory should be created");
            Self { path }
        }

        fn join(&self, name: &str) -> PathBuf {
            self.path.join(name)
        }

        fn path(&self) -> &Path {
            &self.path
        }
    }

    impl Drop for TempDirGuard {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    #[test]
    fn get_file_contents_reference_role() {
        let temp = TempDirGuard::new("file-utilities-read");
        let path = temp.join("test.txt");
        let expected = b"This file is very short. That's all we need.";

        write_file_contents(&path, expected).unwrap();

        let no_limit = get_file_contents(&path, None).unwrap();
        assert_eq!(no_limit, expected);

        let with_large_limit = get_file_contents(&path, Some(1024)).unwrap();
        assert_eq!(with_large_limit, expected);

        let too_small = get_file_contents(&path, Some(16)).unwrap_err();
        assert!(matches!(
            too_small,
            FileUtilitiesError::FileTooLarge {
                size: _,
                max_size: 16
            }
        ));
    }

    #[test]
    fn write_file_contents_truncates_existing_file() {
        let temp = TempDirGuard::new("file-utilities-write");
        let path = temp.join("overwrite.txt");

        write_file_contents(&path, b"first value").unwrap();
        write_file_contents(&path, b"next").unwrap();

        assert_eq!(get_file_contents(&path, None).unwrap(), b"next");
    }

    #[test]
    fn missing_source_path_reports_io_error() {
        let temp = TempDirGuard::new("file-utilities-missing");
        let missing = temp.path().join("missing.txt");

        let error = get_file_contents(missing, None).unwrap_err();
        assert_eq!(error.io_kind(), Some(std::io::ErrorKind::NotFound));
    }
}
