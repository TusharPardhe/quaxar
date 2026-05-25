use crate::{Backend, Factory, NodeObject, NodeStoreJournal, Scheduler, Status};
use basics::{base_uint::Uint256, basic_config::Section};
use std::sync::Arc;

#[derive(Debug, Default)]
pub struct NullBackend;

impl Backend for NullBackend {
    fn get_name(&self) -> String {
        String::new()
    }

    fn open(&self, _create_if_missing: bool) -> Result<(), String> {
        Ok(())
    }

    fn is_open(&self) -> bool {
        false
    }

    fn close(&self) -> Result<(), String> {
        Ok(())
    }

    fn fetch(&self, _hash: &Uint256) -> (Option<Arc<NodeObject>>, Status) {
        (None, Status::NotFound)
    }

    fn fetch_batch(&self, _hashes: &[Uint256]) -> (Vec<Option<Arc<NodeObject>>>, Status) {
        (Vec::new(), Status::Ok)
    }

    fn store(&self, _object: Arc<NodeObject>) {}

    fn store_batch(&self, _batch: &crate::Batch) {}

    fn sync(&self) {}

    fn for_each(&self, _callback: &mut dyn FnMut(Arc<NodeObject>)) {}

    fn get_write_load(&self) -> i32 {
        0
    }

    fn set_delete_path(&self) {}

    fn fd_required(&self) -> i32 {
        0
    }
}

#[derive(Debug, Default)]
pub struct NullFactory;

impl NullFactory {
    pub fn new() -> Self {
        Self
    }
}

impl Factory for NullFactory {
    fn get_name(&self) -> String {
        "none".to_owned()
    }

    fn create_instance(
        &self,
        _key_bytes: usize,
        _parameters: &Section,
        _burst_size: usize,
        _scheduler: Arc<dyn Scheduler>,
        _journal: Arc<dyn NodeStoreJournal>,
    ) -> crate::factory::BackendResult {
        Ok(Box::new(NullBackend))
    }
}

#[cfg(test)]
mod tests {
    use super::{NullBackend, NullFactory};
    use crate::{Backend, Factory, NullJournal};
    use basics::{base_uint::Uint256, basic_config::Section};
    use std::sync::Arc;

    #[test]
    fn null_backend_empty_behavior() {
        let backend = NullBackend;
        assert_eq!(backend.get_name(), "");
        assert!(!backend.is_open());
        assert_eq!(
            backend.fetch(&Uint256::from_array([1; 32])).1,
            crate::Status::NotFound
        );
        assert!(backend.fetch_batch(&[]).0.is_empty());
        assert_eq!(backend.get_write_load(), 0);
    }

    #[test]
    fn null_factory_registers_cpp_backend_name() {
        let backend = NullFactory::new()
            .create_instance(
                crate::NodeObject::KEY_BYTES,
                &Section::new("node_db"),
                0,
                Arc::new(crate::DummyScheduler),
                Arc::new(NullJournal),
            )
            .expect("null backend");

        assert_eq!(NullFactory::new().get_name(), "none");
        assert_eq!(backend.get_name(), "");
    }
}
