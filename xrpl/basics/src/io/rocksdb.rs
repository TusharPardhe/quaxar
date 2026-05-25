//! Rust compatibility surface for `xrpl/basics/rocksdb.h`.
//!
//! The reference header conditionally re-exports RocksDB's public types when the
//! build enables RocksDB support. In Rust we mirror that role by re-exporting
//! the `rocksdb` crate surface directly from `basics`.

pub use ::rocksdb::*;

pub const ROCKSDB_AVAILABLE: bool = true;

pub const fn rocksdb_available() -> bool {
    ROCKSDB_AVAILABLE
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RocksDbSupport;

impl RocksDbSupport {
    pub const fn available(&self) -> bool {
        ROCKSDB_AVAILABLE
    }
}

#[cfg(test)]
mod tests {
    use super::{Env, Options, ROCKSDB_AVAILABLE, RocksDbSupport, rocksdb_available};

    #[test]
    fn rocksdb_compatibility_surface_exposes_real_runtime_types() {
        let mut env = Env::new().expect("create env");
        env.set_background_threads(1);

        let mut options = Options::default();
        options.set_env(&env);

        assert_eq!(ROCKSDB_AVAILABLE, rocksdb_available());
        assert!(rocksdb_available());
        assert!(RocksDbSupport.available());
    }
}
