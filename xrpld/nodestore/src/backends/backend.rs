use crate::{Batch, NodeObject, Status};
use basics::base_uint::Uint256;
use std::sync::Arc;

pub trait Backend: Send + Sync + 'static {
    fn get_name(&self) -> String;

    fn get_block_size(&self) -> Option<usize> {
        None
    }

    fn open(&self, create_if_missing: bool) -> Result<(), String>;

    fn open_deterministic(
        &self,
        _create_if_missing: bool,
        _app_type: u64,
        _uid: u64,
        _salt: u64,
    ) -> Result<(), String> {
        Err(format!(
            "Deterministic appType/uid/salt not supported by backend {}",
            self.get_name()
        ))
    }

    fn is_open(&self) -> bool;

    fn close(&self) -> Result<(), String>;

    fn fetch(&self, hash: &Uint256) -> (Option<Arc<NodeObject>>, Status);

    fn fetch_batch(&self, hashes: &[Uint256]) -> (Vec<Option<Arc<NodeObject>>>, Status);

    fn store(&self, object: Arc<NodeObject>);

    fn store_batch(&self, batch: &Batch);

    fn sync(&self);

    fn for_each(&self, callback: &mut dyn FnMut(Arc<NodeObject>));

    fn get_write_load(&self) -> i32;

    fn set_delete_path(&self);

    fn verify(&self) {}

    fn fd_required(&self) -> i32;
}
