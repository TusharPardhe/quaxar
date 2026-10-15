//! Disk-full ballast (design Case 10).
//!
//! A ballast is a file whose blocks are really allocated on disk, so deleting
//! it hands that space back when the disk fills up. Setting a file's length is
//! not enough: `set_len` creates a sparse file that occupies no blocks, and
//! deleting it frees nothing.
//!
//! Allocation, fastest first:
//! - Linux/Android: `fallocate(2)` mode 0, a metadata-only reservation.
//! - macOS/iOS: `fcntl(F_PREALLOCATE)` (contiguous, then any), then `ftruncate`.
//! - Other Unix: `posix_fallocate(3)`.
//! - Anything else, or a filesystem that refuses or fakes preallocation
//!   (some ZFS/btrfs/NFS setups): write the file out. The data is pseudo-random
//!   so compressing or deduplicating filesystems cannot store it as nothing.
//!
//! On Unix the result is verified from the allocated block count; on Windows
//! the file is always written, which NTFS/ReFS back with real clusters.

use std::fs::File;
use std::io::Write;
use std::path::Path;

/// Chunk used by the write fallback.
const WRITE_CHUNK: usize = 1 << 20;

/// How a ballast was allocated, for logging.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BallastMethod {
    Preallocated,
    Written,
}

/// Bytes the filesystem has really allocated to `path`. `None` where the
/// platform does not report it.
pub fn allocated_bytes(path: &Path) -> Option<u64> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        // st_blocks is always in 512-byte units, whatever the block size.
        std::fs::metadata(path).ok().map(|m| m.blocks().saturating_mul(512))
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        None
    }
}

/// Whether `path` is a ballast of at least `len` bytes that really occupies
/// disk. Where allocation cannot be observed, the length must match.
pub fn is_allocated(path: &Path, len: u64) -> bool {
    let Ok(metadata) = std::fs::metadata(path) else {
        return false;
    };
    if metadata.len() < len {
        return false;
    }
    allocated_bytes(path).is_none_or(|allocated| allocated >= len)
}

/// Create (or replace) the ballast at `path` with `len` allocated bytes.
pub fn create(path: &Path, len: u64) -> std::io::Result<BallastMethod> {
    let file = File::create(path)?;
    if preallocate(&file, len).is_ok() {
        file.sync_all()?;
        if is_allocated(path, len) {
            return Ok(BallastMethod::Preallocated);
        }
    }
    drop(file);
    // Fallback: really write it, so even a filesystem that ignored the
    // preallocation has to back it with blocks.
    let mut file = File::create(path)?;
    write_out(&mut file, len)?;
    file.sync_all()?;
    if is_allocated(path, len) {
        Ok(BallastMethod::Written)
    } else {
        Err(std::io::Error::other(format!(
            "ballast {} is not fully allocated after writing {len} bytes",
            path.display()
        )))
    }
}

fn write_out(file: &mut File, len: u64) -> std::io::Result<()> {
    // xorshift64*: cheap, and incompressible enough for lz4/zstd and dedup.
    let mut state: u64 = 0x9E37_79B9_7F4A_7C15;
    let mut chunk = vec![0u8; WRITE_CHUNK];
    let mut remaining = len;
    while remaining > 0 {
        for word in chunk.chunks_exact_mut(8) {
            state ^= state >> 12;
            state ^= state << 25;
            state ^= state >> 27;
            word.copy_from_slice(&state.wrapping_mul(0x2545_F491_4F6C_DD1D).to_le_bytes());
        }
        let n = remaining.min(WRITE_CHUNK as u64) as usize;
        file.write_all(&chunk[..n])?;
        remaining -= n as u64;
    }
    Ok(())
}

#[cfg(any(target_os = "linux", target_os = "android"))]
fn preallocate(file: &File, len: u64) -> std::io::Result<()> {
    use std::os::fd::AsRawFd;
    let len = libc::off_t::try_from(len).map_err(std::io::Error::other)?;
    // SAFETY: valid open fd; mode 0 allocates blocks and extends the size.
    let rc = unsafe { libc::fallocate(file.as_raw_fd(), 0, 0, len) };
    if rc == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

#[cfg(any(target_os = "macos", target_os = "ios"))]
fn preallocate(file: &File, len: u64) -> std::io::Result<()> {
    use std::os::fd::AsRawFd;
    let length = libc::off_t::try_from(len).map_err(std::io::Error::other)?;
    let fd = file.as_raw_fd();
    let mut store = libc::fstore_t {
        fst_flags: libc::F_ALLOCATECONTIG | libc::F_ALLOCATEALL,
        fst_posmode: libc::F_PEOFPOSMODE,
        fst_offset: 0,
        fst_length: length,
        fst_bytesalloc: 0,
    };
    // SAFETY: valid fd and a properly initialised fstore_t.
    let mut rc = unsafe { libc::fcntl(fd, libc::F_PREALLOCATE, &mut store) };
    if rc == -1 {
        store.fst_flags = libc::F_ALLOCATEALL;
        // SAFETY: as above, retrying without the contiguity requirement.
        rc = unsafe { libc::fcntl(fd, libc::F_PREALLOCATE, &mut store) };
    }
    if rc == -1 {
        return Err(std::io::Error::last_os_error());
    }
    // F_PREALLOCATE reserves blocks past EOF; extending the size makes them
    // part of the file.
    file.set_len(len)
}

#[cfg(all(
    unix,
    not(any(
        target_os = "linux",
        target_os = "android",
        target_os = "macos",
        target_os = "ios"
    ))
))]
fn preallocate(file: &File, len: u64) -> std::io::Result<()> {
    use std::os::fd::AsRawFd;
    let len = libc::off_t::try_from(len).map_err(std::io::Error::other)?;
    // SAFETY: valid open fd.
    let rc = unsafe { libc::posix_fallocate(file.as_raw_fd(), 0, len) };
    if rc == 0 {
        Ok(())
    } else {
        Err(std::io::Error::from_raw_os_error(rc))
    }
}

#[cfg(not(unix))]
fn preallocate(_file: &File, _len: u64) -> std::io::Result<()> {
    // Windows: an NTFS allocation hint is not a guarantee on every volume
    // type, so write the file out instead.
    Err(std::io::Error::from(std::io::ErrorKind::Unsupported))
}

/// Bytes available to this (unprivileged) process on the filesystem holding
/// `path`. `None` if the platform query fails.
pub fn available_bytes(path: &Path) -> Option<u64> {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        let c_path = std::ffi::CString::new(path.as_os_str().as_bytes()).ok()?;
        // SAFETY: zeroed statvfs is a valid out-parameter for statvfs(3).
        let mut stat: libc::statvfs = unsafe { std::mem::zeroed() };
        // SAFETY: c_path is NUL-terminated and stat is writable.
        if unsafe { libc::statvfs(c_path.as_ptr(), &mut stat) } != 0 {
            return None;
        }
        #[allow(clippy::useless_conversion)]
        Some(u64::from(stat.f_bavail).saturating_mul(u64::from(stat.f_frsize)))
    }
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
        let mut available: u64 = 0;
        // SAFETY: wide is NUL-terminated; out-pointers are valid or null.
        let ok = unsafe {
            windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW(
                wide.as_ptr(),
                &mut available,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            )
        };
        (ok != 0).then_some(available)
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = path;
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "quaxar-ballast-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(&dir).expect("temp dir");
        dir.join("ballast")
    }

    #[test]
    fn created_ballast_really_occupies_disk() {
        let path = temp("alloc");
        let len = 8 << 20;
        create(&path, len).expect("create ballast");
        assert!(is_allocated(&path, len));
        if let Some(allocated) = allocated_bytes(&path) {
            assert!(allocated >= len, "allocated {allocated} < {len}");
        }
        let _ = std::fs::remove_dir_all(path.parent().expect("parent"));
    }

    /// The old ballast: `set_len` alone makes a sparse file. It must be seen as
    /// not allocated so it is replaced.
    #[cfg(unix)]
    #[test]
    fn sparse_file_is_not_a_ballast() {
        let path = temp("sparse");
        let len = 8 << 20;
        File::create(&path).expect("create").set_len(len).expect("set_len");
        assert!(
            !is_allocated(&path, len),
            "a sparse file must not count as a reserve"
        );
        create(&path, len).expect("replace");
        assert!(is_allocated(&path, len));
        let _ = std::fs::remove_dir_all(path.parent().expect("parent"));
    }

    #[test]
    fn write_fallback_allocates_and_is_not_zero_filled() {
        let path = temp("write");
        let len = (3 << 20) + 123;
        let mut file = File::create(&path).expect("create");
        write_out(&mut file, len).expect("write");
        file.sync_all().expect("sync");
        assert!(is_allocated(&path, len));
        let bytes = std::fs::read(&path).expect("read");
        assert_eq!(bytes.len() as u64, len);
        assert!(bytes.iter().any(|b| *b != 0), "fallback data must not be zeros");
        let _ = std::fs::remove_dir_all(path.parent().expect("parent"));
    }

    #[test]
    fn available_bytes_reports_the_filesystem() {
        let path = temp("avail");
        std::fs::write(&path, b"x").expect("write");
        let available = available_bytes(path.parent().expect("parent"));
        #[cfg(any(unix, windows))]
        assert!(available.is_some_and(|bytes| bytes > 0));
        let _ = available;
        let _ = std::fs::remove_dir_all(path.parent().expect("parent"));
    }
}
