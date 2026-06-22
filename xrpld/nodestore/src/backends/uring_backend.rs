//! io_uring-based batch reader for Linux with the `io-uring` feature enabled.
//! Falls back to a no-op stub on non-Linux platforms or when the feature is disabled.

#[cfg(all(target_os = "linux", feature = "io-uring"))]
mod inner {
    use std::io;
    use std::path::{Path, PathBuf};
    use tokio_uring::fs::File;

    /// Batch reader that uses io_uring for concurrent read submissions.
    pub struct UringBatchReader {
        path: PathBuf,
    }

    impl UringBatchReader {
        pub fn new(path: impl AsRef<Path>) -> Self {
            Self {
                path: path.as_ref().to_path_buf(),
            }
        }

        /// Submit batch reads at the given offsets and lengths via io_uring.
        /// Returns the data for each (offset, len) pair.
        pub fn batch_fetch(&self, requests: &[(u64, u32)]) -> io::Result<Vec<Vec<u8>>> {
            tokio_uring::start(async {
                let file = File::open(&self.path).await?;
                let mut results = Vec::with_capacity(requests.len());
                for &(offset, len) in requests {
                    let buf = vec![0u8; len as usize];
                    let (res, buf) = file.read_at(buf, offset).await;
                    res?;
                    results.push(buf);
                }
                Ok(results)
            })
        }
    }
}

#[cfg(not(all(target_os = "linux", feature = "io-uring")))]
mod inner {
    use std::io;
    use std::path::{Path, PathBuf};

    /// Fallback stub when io_uring is unavailable.
    pub struct UringBatchReader {
        _path: PathBuf,
    }

    impl UringBatchReader {
        pub fn new(path: impl AsRef<Path>) -> Self {
            Self {
                _path: path.as_ref().to_path_buf(),
            }
        }

        pub fn batch_fetch(&self, _requests: &[(u64, u32)]) -> io::Result<Vec<Vec<u8>>> {
            Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "io_uring not available on this platform",
            ))
        }
    }
}

pub use inner::UringBatchReader;
