//! Node log file: `[debug_logfile]`, written off the logging threads and
//! rotated by size.
//!
//! IMPORTANT: logging must never stall consensus. Each log line is handed to
//! a bounded queue; a dedicated `quaxar-log-writer` thread writes it. If the
//! queue is full (the disk is stuck), lines are dropped and counted instead of
//! blocking the caller, and the writer reports how many it dropped.
//!
//! Rotation: when `debug.log` passes `max_bytes`, it becomes `debug.log.1`,
//! `.1` becomes `.2`, and so on; the oldest beyond `keep` is deleted. Disk use
//! is bounded by about `max_bytes × (keep + 1)`.
//!
//! As rippled `Config::getDebugLogFile`, a relative path is relative to the
//! config file's directory, and the directory is created if missing.

use std::fs::{File, OpenOptions};
use std::io::{self, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, SyncSender, TrySendError, sync_channel};
use std::time::Duration;

/// Bytes per file before rotating.
pub const DEFAULT_MAX_BYTES: u64 = 256 * 1024 * 1024;
/// Rotated files kept (`debug.log.1` … `debug.log.N`).
pub const DEFAULT_KEEP: usize = 8;
/// Lines queued before new lines are dropped.
const QUEUE_LINES: usize = 65_536;
/// The writer flushes at least this often.
const FLUSH_EVERY: Duration = Duration::from_millis(250);

/// Resolve `[debug_logfile]` like rippled: relative to the config directory.
pub fn resolve_path(configured: &str, config_path: &Path) -> PathBuf {
    let path = PathBuf::from(configured.trim());
    if path.is_absolute() {
        return path;
    }
    config_path
        .parent()
        .map_or_else(|| path.clone(), |dir| dir.join(&path))
}

struct Rotating {
    path: PathBuf,
    max_bytes: u64,
    keep: usize,
    out: BufWriter<File>,
    size: u64,
}

fn open_append(path: &Path) -> io::Result<(BufWriter<File>, u64)> {
    let file = OpenOptions::new().create(true).append(true).open(path)?;
    let size = file.metadata()?.len();
    Ok((BufWriter::with_capacity(256 * 1024, file), size))
}

fn rotated(path: &Path, n: usize) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(format!(".{n}"));
    PathBuf::from(name)
}

impl Rotating {
    fn open(path: PathBuf, max_bytes: u64, keep: usize) -> io::Result<Self> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let (out, size) = open_append(&path)?;
        Ok(Self {
            path,
            max_bytes,
            keep,
            out,
            size,
        })
    }

    fn rotate(&mut self) -> io::Result<()> {
        self.out.flush()?;
        let _ = std::fs::remove_file(rotated(&self.path, self.keep));
        for n in (1..self.keep).rev() {
            let from = rotated(&self.path, n);
            if from.exists() {
                std::fs::rename(&from, rotated(&self.path, n + 1))?;
            }
        }
        if self.keep > 0 {
            std::fs::rename(&self.path, rotated(&self.path, 1))?;
        } else {
            std::fs::remove_file(&self.path)?;
        }
        let (out, size) = open_append(&self.path)?;
        self.out = out;
        self.size = size;
        Ok(())
    }

    fn write_line(&mut self, line: &[u8]) -> io::Result<()> {
        if self.size > 0 && self.size + line.len() as u64 > self.max_bytes {
            self.rotate()?;
        }
        self.out.write_all(line)?;
        self.size += line.len() as u64;
        Ok(())
    }
}

/// The log sink handed to `tracing_subscriber::fmt::layer().with_writer`.
#[derive(Clone)]
pub struct FileLog {
    sender: SyncSender<Vec<u8>>,
    dropped: Arc<AtomicU64>,
}

impl FileLog {
    /// Open (or create) the log file and start the writer thread.
    pub fn start(path: PathBuf, max_bytes: u64, keep: usize) -> io::Result<Self> {
        let mut file = Rotating::open(path, max_bytes, keep)?;
        let (sender, receiver) = sync_channel::<Vec<u8>>(QUEUE_LINES);
        let dropped = Arc::new(AtomicU64::new(0));
        let writer_dropped = Arc::clone(&dropped);
        std::thread::Builder::new()
            .name("quaxar-log-writer".to_owned())
            .spawn(move || run(&mut file, receiver, &writer_dropped))?;
        Ok(Self { sender, dropped })
    }

    /// Lines dropped so far because the queue was full.
    pub fn dropped(&self) -> u64 {
        self.dropped.load(Ordering::Relaxed)
    }
}

fn run(file: &mut Rotating, receiver: Receiver<Vec<u8>>, dropped: &AtomicU64) {
    let mut reported = 0u64;
    loop {
        match receiver.recv_timeout(FLUSH_EVERY) {
            Ok(line) => {
                let _ = file.write_line(&line);
                // Drain what is queued before flushing.
                while let Ok(line) = receiver.try_recv() {
                    let _ = file.write_line(&line);
                }
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => {
                let _ = file.out.flush();
                return;
            }
        }
        let now = dropped.load(Ordering::Relaxed);
        if now > reported {
            let note = format!(
                "{} WARN quaxar-log-writer: log queue full, dropped {} lines\n",
                chrono_now(),
                now - reported
            );
            let _ = file.write_line(note.as_bytes());
            reported = now;
        }
        let _ = file.out.flush();
    }
}

fn chrono_now() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Micros, true)
}

/// One event's bytes; sent to the writer when the formatter is done with it.
pub struct LineWriter {
    buf: Vec<u8>,
    log: FileLog,
}

impl Write for LineWriter {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        self.buf.extend_from_slice(data);
        Ok(data.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Drop for LineWriter {
    fn drop(&mut self) {
        if self.buf.is_empty() {
            return;
        }
        let line = std::mem::take(&mut self.buf);
        if let Err(TrySendError::Full(_) | TrySendError::Disconnected(_)) =
            self.log.sender.try_send(line)
        {
            self.log.dropped.fetch_add(1, Ordering::Relaxed);
        }
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for FileLog {
    type Writer = LineWriter;
    fn make_writer(&'a self) -> LineWriter {
        LineWriter {
            buf: Vec::with_capacity(256),
            log: self.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("qx-log-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn relative_log_path_is_relative_to_the_config_directory() {
        let config = Path::new("/etc/opt/quaxar/quaxar.cfg");
        assert_eq!(
            resolve_path("log/debug.log", config),
            PathBuf::from("/etc/opt/quaxar/log/debug.log")
        );
        assert_eq!(
            resolve_path("/srv/log/debug.log", config),
            PathBuf::from("/srv/log/debug.log")
        );
    }

    #[test]
    fn rotation_keeps_at_most_keep_files_and_bounds_each_one() {
        let dir = temp_dir("rotate");
        let path = dir.join("debug.log");
        let mut file = Rotating::open(path.clone(), 100, 2).unwrap();
        for i in 0..40 {
            file.write_line(format!("line {i:04} xxxxxxxxxx\n").as_bytes()).unwrap();
        }
        file.out.flush().unwrap();
        assert!(path.exists());
        assert!(rotated(&path, 1).exists());
        assert!(rotated(&path, 2).exists());
        assert!(!rotated(&path, 3).exists(), "only `keep` rotated files");
        for p in [path.clone(), rotated(&path, 1), rotated(&path, 2)] {
            assert!(std::fs::metadata(&p).unwrap().len() <= 100, "{}", p.display());
        }
        let newest = std::fs::read_to_string(&path).unwrap();
        assert!(newest.contains("line 0039"));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn lines_reach_the_file_through_the_writer_thread() {
        let dir = temp_dir("writer");
        let path = dir.join("sub/debug.log");
        let log = FileLog::start(path.clone(), DEFAULT_MAX_BYTES, 2).unwrap();
        for i in 0..100 {
            let mut w = tracing_subscriber::fmt::MakeWriter::make_writer(&log);
            w.write_all(format!("event {i}\n").as_bytes()).unwrap();
        }
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        loop {
            let text = std::fs::read_to_string(&path).unwrap_or_default();
            if text.lines().count() == 100 {
                assert!(text.ends_with("event 99\n"));
                break;
            }
            assert!(std::time::Instant::now() < deadline, "lines not written");
            std::thread::sleep(Duration::from_millis(20));
        }
        assert_eq!(log.dropped(), 0);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
