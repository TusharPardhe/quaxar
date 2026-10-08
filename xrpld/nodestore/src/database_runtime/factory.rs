use crate::{Backend, NodeStoreJournal, Scheduler};
use basics::basic_config::Section;
use std::sync::Arc;

pub type BackendResult = Result<Box<dyn Backend>, String>;

pub trait Factory: Send + Sync + 'static {
    fn get_name(&self) -> String;

    fn create_instance(
        &self,
        key_bytes: usize,
        parameters: &Section,
        burst_size: usize,
        scheduler: Arc<dyn Scheduler>,
        journal: Arc<dyn NodeStoreJournal>,
    ) -> BackendResult;
}
