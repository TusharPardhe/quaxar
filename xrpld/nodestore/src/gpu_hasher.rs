//! GPU-accelerated batch hashing scaffold.
//!
//! Gated behind the `gpu-hash` feature. Provides a [`BatchHasher`] trait with
//! a CPU fallback and a stub GPU implementation for future wgpu integration.

use sha2::{Digest, Sha256};

/// Batch-hash multiple inputs in one call.
pub trait BatchHasher {
    fn hash_batch(&self, inputs: &[&[u8]]) -> Vec<[u8; 32]>;
}

/// CPU fallback using sha2.
pub struct CpuBatchHasher;

impl BatchHasher for CpuBatchHasher {
    fn hash_batch(&self, inputs: &[&[u8]]) -> Vec<[u8; 32]> {
        inputs
            .iter()
            .map(|data| {
                let mut hasher = Sha256::new();
                hasher.update(data);
                hasher.finalize().into()
            })
            .collect()
    }
}

/// Stub GPU hasher — delegates to [`CpuBatchHasher`] until wgpu integration.
pub struct GpuBatchHasher {
    fallback: CpuBatchHasher,
}

impl GpuBatchHasher {
    pub fn new() -> Self {
        Self {
            fallback: CpuBatchHasher,
        }
    }
}

impl BatchHasher for GpuBatchHasher {
    fn hash_batch(&self, inputs: &[&[u8]]) -> Vec<[u8; 32]> {
        self.fallback.hash_batch(inputs)
    }
}
