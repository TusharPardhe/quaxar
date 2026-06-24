//! Parallel transaction pre-flight verification.
//!
//! Signature verification (Ed25519 / secp256k1) is CPU-bound and stateless.
//! This module provides a parallel batch verifier using Rayon that checks
//! signatures across all available cores before the sequential apply path.

use protocol::{Rules, STTx};
use rayon::prelude::*;
use std::sync::Arc;

/// Result of parallel signature pre-verification for one transaction.
#[derive(Debug, Clone)]
pub struct PreVerifyResult {
    pub tx: Arc<STTx>,
    pub signature_valid: bool,
}

/// Verify signatures for a batch of transactions in parallel.
///
/// This is safe because `check_sign` is a pure function that only reads
/// from the transaction object and the rules — no shared mutable state.
///
/// Returns results in the same order as input.
pub fn parallel_verify_signatures(txns: &[Arc<STTx>], rules: &Rules) -> Vec<PreVerifyResult> {
    txns.par_iter()
        .map(|tx| {
            let valid = tx.check_sign(rules).is_ok();
            PreVerifyResult {
                tx: Arc::clone(tx),
                signature_valid: valid,
            }
        })
        .collect()
}

/// Filter a batch of transactions, rejecting those with invalid signatures.
/// Returns only transactions that passed signature verification.
pub fn filter_valid_signatures(txns: &[Arc<STTx>], rules: &Rules) -> Vec<Arc<STTx>> {
    parallel_verify_signatures(txns, rules)
        .into_iter()
        .filter(|r| r.signature_valid)
        .map(|r| r.tx)
        .collect()
}
