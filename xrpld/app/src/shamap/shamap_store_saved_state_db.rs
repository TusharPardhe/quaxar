use crate::SHAMapStoreSavedState;
use basics::basic_config::BasicConfig;
use xrpld_core::StateDb;

#[derive(Debug)]
pub struct SHAMapStoreSavedStateDb {
    inner: StateDb,
}

impl SHAMapStoreSavedStateDb {
    pub fn open(config: &BasicConfig, db_name: &str) -> Result<Self, String> {
        Ok(Self {
            inner: StateDb::open(config, db_name)?,
        })
    }

    pub fn from_state_db(inner: StateDb) -> Self {
        Self { inner }
    }

    pub fn get_can_delete(&self) -> Result<u32, String> {
        self.inner.get_can_delete()
    }

    pub fn set_can_delete(&self, can_delete: u32) -> Result<u32, String> {
        self.inner.set_can_delete(can_delete)
    }

    pub fn get_state(&self) -> Result<SHAMapStoreSavedState, String> {
        self.inner.get_state()
    }

    pub fn set_state(&self, state: &SHAMapStoreSavedState) -> Result<(), String> {
        self.inner.set_state(state)
    }

    pub fn set_last_rotated(&self, seq: u32) -> Result<(), String> {
        self.inner.set_last_rotated(seq)
    }
}
