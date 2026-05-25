use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JournalLevel {
    Trace,
    Debug,
    Info,
    Warn,
    Error,
    Fatal,
}

impl fmt::Display for JournalLevel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let level = match self {
            JournalLevel::Trace => "trace",
            JournalLevel::Debug => "debug",
            JournalLevel::Info => "info",
            JournalLevel::Warn => "warn",
            JournalLevel::Error => "error",
            JournalLevel::Fatal => "fatal",
        };
        f.write_str(level)
    }
}

pub trait PerfLogJournal: Send + Sync + 'static {
    fn log(&self, level: JournalLevel, message: &str);
}

pub struct NullJournal;

impl PerfLogJournal for NullJournal {
    fn log(&self, _level: JournalLevel, _message: &str) {}
}
