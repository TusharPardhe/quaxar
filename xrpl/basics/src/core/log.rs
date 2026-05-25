//! Compatibility-oriented log hook surface for `xrpl/basics/Log.h`.

use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum LogSeverity {
    Invalid = -1,
    Trace = 0,
    Debug = 1,
    Info = 2,
    Warning = 3,
    Error = 4,
    Fatal = 5,
}

impl LogSeverity {
    pub fn from_string(value: &str) -> Self {
        match value.to_ascii_lowercase().as_str() {
            "trace" => Self::Trace,
            "debug" => Self::Debug,
            "info" => Self::Info,
            "warning" | "warn" => Self::Warning,
            "error" => Self::Error,
            "fatal" => Self::Fatal,
            _ => Self::Invalid,
        }
    }
}

impl fmt::Display for LogSeverity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let text = match self {
            Self::Invalid => "invalid",
            Self::Trace => "trace",
            Self::Debug => "debug",
            Self::Info => "info",
            Self::Warning => "warning",
            Self::Error => "error",
            Self::Fatal => "fatal",
        };
        f.write_str(text)
    }
}

pub trait LogSink: Send + Sync + 'static {
    fn write(&self, level: LogSeverity, partition: &str, text: &str);
}

#[derive(Debug, Default)]
pub struct NullLogSink;

impl LogSink for NullLogSink {
    fn write(&self, _level: LogSeverity, _partition: &str, _text: &str) {}
}

#[derive(Debug, Default)]
pub struct RecordingLogSink {
    entries: Mutex<Vec<(LogSeverity, String, String)>>,
}

impl RecordingLogSink {
    pub fn entries(&self) -> Vec<(LogSeverity, String, String)> {
        self.entries
            .lock()
            .expect("log sink mutex poisoned")
            .clone()
    }
}

impl LogSink for RecordingLogSink {
    fn write(&self, level: LogSeverity, partition: &str, text: &str) {
        self.entries.lock().expect("log sink mutex poisoned").push((
            level,
            partition.to_owned(),
            text.to_owned(),
        ));
    }
}

#[derive(Clone)]
pub struct Logs {
    state: Arc<Mutex<LogsState>>,
}

struct LogsState {
    threshold: LogSeverity,
    partitions: BTreeMap<String, LogSeverity>,
    sink: Arc<dyn LogSink>,
    file: Option<PathBuf>,
    silent: bool,
}

impl fmt::Debug for Logs {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let state = self.state.lock().expect("logs mutex poisoned");
        f.debug_struct("Logs")
            .field("threshold", &state.threshold)
            .field("partition_count", &state.partitions.len())
            .field("file", &state.file)
            .field("silent", &state.silent)
            .finish()
    }
}

impl Logs {
    pub fn new(threshold: LogSeverity) -> Self {
        Self::with_sink(threshold, Arc::new(NullLogSink))
    }

    pub fn with_sink(threshold: LogSeverity, sink: Arc<dyn LogSink>) -> Self {
        Self {
            state: Arc::new(Mutex::new(LogsState {
                threshold,
                partitions: BTreeMap::new(),
                sink,
                file: None,
                silent: false,
            })),
        }
    }

    pub fn open(&self, path: impl AsRef<Path>) -> bool {
        let path = path.as_ref();
        if let Some(parent) = path.parent()
            && std::fs::create_dir_all(parent).is_err()
        {
            return false;
        }
        if std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .is_err()
        {
            return false;
        }

        let mut state = self.state.lock().expect("logs mutex poisoned");
        state.file = Some(path.to_path_buf());
        true
    }

    pub fn threshold(&self) -> LogSeverity {
        self.state.lock().expect("logs mutex poisoned").threshold
    }

    pub fn set_threshold(&self, threshold: LogSeverity) {
        self.state.lock().expect("logs mutex poisoned").threshold = threshold;
    }

    pub fn partition_severities(&self) -> Vec<(String, String)> {
        self.state
            .lock()
            .expect("logs mutex poisoned")
            .partitions
            .iter()
            .map(|(name, severity)| (name.clone(), severity.to_string()))
            .collect()
    }

    pub fn silent(&self, silent: bool) {
        self.state.lock().expect("logs mutex poisoned").silent = silent;
    }

    pub fn write(&self, level: LogSeverity, partition: &str, text: &str, console: bool) {
        let mut state = self.state.lock().expect("logs mutex poisoned");
        let default_threshold = state.threshold;
        let partition_threshold = *state
            .partitions
            .entry(partition.to_owned())
            .or_insert(default_threshold);
        if level < partition_threshold {
            return;
        }

        state.sink.write(level, partition, text);
        if let Some(path) = &state.file {
            let _ = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)
                .and_then(|mut file| {
                    use std::io::Write;
                    writeln!(file, "[{level}] {partition}: {text}")
                });
        }
        if console && !state.silent {
            match level {
                LogSeverity::Trace => tracing::trace!(target: "basics", partition, "{text}"),
                LogSeverity::Debug => tracing::debug!(target: "basics", partition, "{text}"),
                LogSeverity::Info => tracing::info!(target: "basics", partition, "{text}"),
                LogSeverity::Warning => tracing::warn!(target: "basics", partition, "{text}"),
                LogSeverity::Error | LogSeverity::Fatal => {
                    tracing::error!(target: "basics", partition, "{text}")
                }
                LogSeverity::Invalid => {}
            }
        }
    }

    pub fn rotate(&self) -> String {
        let path = self.state.lock().expect("logs mutex poisoned").file.clone();
        if let Some(path) = path
            && std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)
                .is_ok()
        {
            return "The log file was closed and reopened.".to_owned();
        }
        "The log file could not be closed and reopened.".to_owned()
    }

    pub fn journal(&self, partition: impl Into<String>) -> Journal {
        Journal {
            logs: self.clone(),
            partition: partition.into(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Journal {
    logs: Logs,
    partition: String,
}

impl Journal {
    pub fn write(&self, level: LogSeverity, text: &str) {
        self.logs.write(level, &self.partition, text, false);
    }

    pub fn trace(&self, text: &str) {
        self.write(LogSeverity::Trace, text);
    }

    pub fn debug(&self, text: &str) {
        self.write(LogSeverity::Debug, text);
    }

    pub fn info(&self, text: &str) {
        self.write(LogSeverity::Info, text);
    }

    pub fn warning(&self, text: &str) {
        self.write(LogSeverity::Warning, text);
    }

    pub fn error(&self, text: &str) {
        self.write(LogSeverity::Error, text);
    }

    pub fn fatal(&self, text: &str) {
        self.write(LogSeverity::Fatal, text);
    }
}

#[cfg(test)]
mod tests {
    use super::{LogSeverity, Logs, RecordingLogSink};
    use std::path::PathBuf;
    use std::sync::Arc;

    fn unique_path(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "{name}-{}-{}.log",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("time")
                .as_nanos()
        ))
    }

    #[test]
    fn severity_round_trip_strings() {
        assert_eq!(LogSeverity::from_string("trace"), LogSeverity::Trace);
        assert_eq!(LogSeverity::from_string("warn"), LogSeverity::Warning);
        assert_eq!(LogSeverity::Error.to_string(), "error");
        assert_eq!(LogSeverity::Invalid.to_string(), "invalid");
    }

    #[test]
    fn logs_write_to_sink_and_file() {
        let sink = Arc::new(RecordingLogSink::default());
        let logs = Logs::with_sink(LogSeverity::Info, sink.clone());
        let path = unique_path("logs");
        assert!(logs.open(&path));

        logs.write(LogSeverity::Debug, "peer", "hidden", false);
        logs.write(LogSeverity::Info, "peer", "visible", false);

        let entries = sink.entries();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].2, "visible");

        let file = std::fs::read_to_string(&path).expect("log file");
        assert!(file.contains("visible"));
        let _ = std::fs::remove_file(path);
    }
}
