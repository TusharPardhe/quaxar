//! WorkFile: async local file read.
//!
//! Reads file contents up to 1MB, calls callback with contents or error.

use std::sync::{Arc, Mutex};

use tokio::runtime::Handle;

use super::work::{FileWorkCallback, Work, WorkError};

/// Max file size: 1 MB (matches reference `megabytes(1)`).
const MAX_FILE_SIZE: u64 = 1_048_576;

/// Async file reader.
pub struct WorkFile {
    path: String,
    callback: Arc<Mutex<Option<FileWorkCallback>>>,
    handle: Handle,
}

impl WorkFile {
    pub fn new(path: String, handle: Handle, cb: FileWorkCallback) -> Arc<Self> {
        Arc::new(Self {
            path,
            callback: Arc::new(Mutex::new(Some(cb))),
            handle,
        })
    }

    fn fire_callback(&self, result: Result<String, WorkError>) {
        if let Some(cb) = self.callback.lock().unwrap().take() {
            cb(result);
        }
    }
}

impl Work for WorkFile {
    fn run(&self) {
        let path = self.path.clone();
        let callback = self.callback.clone();

        self.handle.spawn(async move {
            let result = async {
                let metadata = tokio::fs::metadata(&path)
                    .await
                    .map_err(|e| WorkError::Io(e.to_string()))?;

                if metadata.len() > MAX_FILE_SIZE {
                    return Err(WorkError::Io(format!(
                        "file too large: {} bytes (max {})",
                        metadata.len(),
                        MAX_FILE_SIZE
                    )));
                }

                tokio::fs::read_to_string(&path)
                    .await
                    .map_err(|e| WorkError::Io(e.to_string()))
            }
            .await;

            if let Some(cb) = callback.lock().unwrap().take() {
                cb(result);
            }
        });
    }

    fn cancel(&self) {
        // Nothing to do - either it finished in run, or it didn't start.
    }
}

impl Drop for WorkFile {
    fn drop(&mut self) {
        self.fire_callback(Err(WorkError::Dropped));
    }
}
