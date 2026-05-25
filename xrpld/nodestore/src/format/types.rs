use crate::NodeObject;
use std::sync::Arc;

#[allow(non_upper_case_globals)]
pub const batch_write_preallocation_size: usize = 256;
#[allow(non_upper_case_globals)]
pub const batch_write_limit_size: usize = 65_536;
pub const BATCH_WRITE_PREALLOCATION_SIZE: usize = batch_write_preallocation_size;
pub const BATCH_WRITE_LIMIT_SIZE: usize = batch_write_limit_size;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Ok,
    NotFound,
    DataCorrupt,
    Unknown,
    BackendError,
    CustomCode(i32),
}

impl Status {
    pub fn custom_code(code: i32) -> Self {
        Self::CustomCode(code)
    }

    pub fn code(self) -> Option<i32> {
        match self {
            Self::CustomCode(code) => Some(code),
            _ => None,
        }
    }
}

pub type Batch = Vec<Arc<NodeObject>>;
