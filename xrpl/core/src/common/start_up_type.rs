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
}

impl fmt::Display for StartUpType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", *self as u8)
    }
}
