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
        let value = match self {
            Self::Trace => "trace",
            Self::Debug => "debug",
            Self::Info => "info",
            Self::Warn => "warn",
            Self::Error => "error",
            Self::Fatal => "fatal",
        };
        f.write_str(value)
    }
}

pub trait NodeStoreJournal: Send + Sync + 'static {
    fn log(&self, level: JournalLevel, message: &str);
}

#[derive(Debug, Default)]
pub struct NullJournal;

impl NodeStoreJournal for NullJournal {
    fn log(&self, _level: JournalLevel, _message: &str) {}
}
