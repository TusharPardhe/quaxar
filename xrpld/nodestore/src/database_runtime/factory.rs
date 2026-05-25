use crate::{Backend, NodeStoreJournal, Scheduler};
use basics::basic_config::Section;
use std::{any::Any, sync::Arc};

pub type BackendResult = Result<Box<dyn Backend>, String>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NuDbContext {
    app_type: u64,
    uid: u64,
    salt: u64,
}

impl NuDbContext {
    pub const fn new(app_type: u64, uid: u64, salt: u64) -> Self {
        Self {
            app_type,
            uid,
            salt,
        }
    }

    pub const fn app_type(&self) -> u64 {
        self.app_type
    }

    pub const fn uid(&self) -> u64 {
        self.uid
    }

    pub const fn salt(&self) -> u64 {
        self.salt
    }
}

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

    fn create_instance_with_nudb_context(
        &self,
        key_bytes: usize,
        parameters: &Section,
        burst_size: usize,
        scheduler: Arc<dyn Scheduler>,
        context: &mut NuDbContext,
        journal: Arc<dyn NodeStoreJournal>,
    ) -> Option<BackendResult> {
        let _ = (
            key_bytes, parameters, burst_size, scheduler, context, journal,
        );
        None
    }

    fn create_instance_with_context(
        &self,
        key_bytes: usize,
        parameters: &Section,
        burst_size: usize,
        scheduler: Arc<dyn Scheduler>,
        context: &mut dyn Any,
        journal: Arc<dyn NodeStoreJournal>,
    ) -> Option<BackendResult> {
        if let Some(context) = context.downcast_mut::<NuDbContext>() {
            return self.create_instance_with_nudb_context(
                key_bytes, parameters, burst_size, scheduler, context, journal,
            );
        }
        let _ = (
            key_bytes, parameters, burst_size, scheduler, context, journal,
        );
        None
    }
}

#[cfg(test)]
mod tests {
    use super::{Factory, NuDbContext};
    use crate::{Backend, DummyScheduler, NodeStoreJournal, NullJournal, Scheduler};
    use basics::base_uint::Uint256;
    use basics::basic_config::Section;
    use std::sync::Arc;

    struct DummyFactory;

    struct NuDbContextFactory;

    struct DummyBackend;

    impl Backend for DummyBackend {
        fn get_name(&self) -> String {
            "dummy".to_owned()
        }

        fn open(&self, _create_if_missing: bool) -> Result<(), String> {
            Ok(())
        }

        fn is_open(&self) -> bool {
            true
        }

        fn close(&self) -> Result<(), String> {
            Ok(())
        }

        fn fetch(&self, _hash: &Uint256) -> (Option<Arc<crate::NodeObject>>, crate::Status) {
            (None, crate::Status::NotFound)
        }

        fn fetch_batch(
            &self,
            _hashes: &[Uint256],
        ) -> (Vec<Option<Arc<crate::NodeObject>>>, crate::Status) {
            (Vec::new(), crate::Status::Ok)
        }

        fn store(&self, _object: Arc<crate::NodeObject>) {}

        fn store_batch(&self, _batch: &crate::Batch) {}

        fn sync(&self) {}

        fn for_each(&self, _callback: &mut dyn FnMut(Arc<crate::NodeObject>)) {}

        fn get_write_load(&self) -> i32 {
            0
        }

        fn set_delete_path(&self) {}

        fn fd_required(&self) -> i32 {
            0
        }
    }

    impl Factory for DummyFactory {
        fn get_name(&self) -> String {
            "Dummy".to_owned()
        }

        fn create_instance(
            &self,
            _key_bytes: usize,
            _parameters: &Section,
            _burst_size: usize,
            _scheduler: Arc<dyn Scheduler>,
            _journal: Arc<dyn NodeStoreJournal>,
        ) -> super::BackendResult {
            Ok(Box::new(DummyBackend))
        }
    }

    impl Factory for NuDbContextFactory {
        fn get_name(&self) -> String {
            "NuDB".to_owned()
        }

        fn create_instance(
            &self,
            _key_bytes: usize,
            _parameters: &Section,
            _burst_size: usize,
            _scheduler: Arc<dyn Scheduler>,
            _journal: Arc<dyn NodeStoreJournal>,
        ) -> super::BackendResult {
            Ok(Box::new(DummyBackend))
        }

        fn create_instance_with_nudb_context(
            &self,
            _key_bytes: usize,
            _parameters: &Section,
            _burst_size: usize,
            _scheduler: Arc<dyn Scheduler>,
            context: &mut NuDbContext,
            _journal: Arc<dyn NodeStoreJournal>,
        ) -> Option<super::BackendResult> {
            *context = NuDbContext::new(7, 11, 13);
            Some(Ok(Box::new(DummyBackend)))
        }
    }

    #[test]
    fn factory_context_overload_defaults_to_none() {
        let factory = DummyFactory;
        let mut section = Section::new("node_db");
        section.set("type", "dummy");
        let mut context = ();

        assert!(
            factory
                .create_instance_with_context(
                    crate::NodeObject::KEY_BYTES,
                    &section,
                    0,
                    Arc::new(DummyScheduler),
                    &mut context,
                    Arc::new(NullJournal),
                )
                .is_none()
        );
    }

    #[test]
    fn typed_nudb_context_overload_stays_object_safe() {
        let factory = NuDbContextFactory;
        let mut section = Section::new("node_db");
        section.set("type", "NuDB");
        let mut context = NuDbContext::new(1, 2, 3);

        let backend = factory
            .create_instance_with_context(
                crate::NodeObject::KEY_BYTES,
                &section,
                0,
                Arc::new(DummyScheduler),
                &mut context,
                Arc::new(NullJournal),
            )
            .expect("typed context should route through the NuDB-specific overload")
            .expect("backend should construct");

        assert_eq!(backend.get_name(), "dummy");
        assert_eq!(context, NuDbContext::new(7, 11, 13));
    }
}
