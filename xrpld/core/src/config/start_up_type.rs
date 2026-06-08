use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum StartUpType {
    Fresh = 0,
    Normal = 1,
    Load = 2,
    LoadFile = 3,
    Replay = 4,
    Network = 5,
    Snapshot = 6,
}

impl fmt::Display for StartUpType {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&(*self as u8).to_string())
    }
}
