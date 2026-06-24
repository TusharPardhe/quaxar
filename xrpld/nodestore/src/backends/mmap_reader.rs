//! Memory-mapped reader for zero-copy access to NuDB data files.
//! Only available when the `mmap-store` feature is enabled.

use memmap2::Mmap;
use std::fs::File;
use std::io;
use std::path::{Path, PathBuf};

/// Read-only memory-mapped view over a NuDB data file.
pub struct MmapReader {
    _path: PathBuf,
    mmap: Mmap,
}

impl MmapReader {
    /// Open the file at `path` as a read-only memory map.
    pub fn open(path: impl AsRef<Path>) -> io::Result<Self> {
        let file = File::open(path.as_ref())?;
        // SAFETY: The file is opened read-only and we do not modify it.
        let mmap = unsafe { Mmap::map(&file)? };
        Ok(Self {
            _path: path.as_ref().to_path_buf(),
            mmap,
        })
    }

    /// Return a zero-copy slice into the mapped data at the given offset and length.
    pub fn read_at(&self, offset: usize, len: usize) -> Option<&[u8]> {
        self.mmap.get(offset..offset + len)
    }

    /// Total size of the mapped file.
    pub fn len(&self) -> usize {
        self.mmap.len()
    }

    /// Whether the mapped region is empty.
    pub fn is_empty(&self) -> bool {
        self.mmap.is_empty()
    }
}
