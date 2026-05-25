//! Ambient transaction-runtime guards used by the the reference implementation tx callers.
//!
//! This ports only the rules-driven runtime state setup around transaction
//! work.

use basics::number::{MantissaScale, NumberMantissaScaleGuard};

use crate::{
    CurrentTransactionRulesGuard, NumberSo, Rules, feature_lending_protocol,
    feature_single_asset_vault, feature_universal_number,
};

/// Mirrors callers like `apply(...)`, `TxQ::apply(...)`, and
/// `Transactor::operator()`, which set both the STNumber switchover and the
/// current transaction rules for the duration of a transaction operation.
#[derive(Debug)]
pub struct TransactionApplyRuntimeGuard {
    _st_number: NumberSo,
    _rules: CurrentTransactionRulesGuard,
}

impl TransactionApplyRuntimeGuard {
    pub fn new(rules: &Rules) -> Self {
        Self {
            _st_number: NumberSo::new(rules.enabled(&feature_universal_number())),
            _rules: CurrentTransactionRulesGuard::new(rules.clone()),
        }
    }
}

/// Mirrors `with_txn_type(...)` in the transaction dispatch layer.
///
/// When SingleAssetVault or LendingProtocol is enabled, the current rules and
/// STNumber switchover are both set explicitly. Otherwise, the legacy path
/// forces the old small-mantissa policy without changing the current rules
/// context or the STNumber switchover.
#[derive(Debug)]
pub struct TransactionStepRuntimeGuard {
    _st_number: Option<NumberSo>,
    _rules: Option<CurrentTransactionRulesGuard>,
    _mantissa_scale: Option<NumberMantissaScaleGuard>,
}

impl TransactionStepRuntimeGuard {
    pub fn new(rules: &Rules) -> Self {
        if rules.enabled(&feature_single_asset_vault())
            || rules.enabled(&feature_lending_protocol())
        {
            Self {
                _st_number: Some(NumberSo::new(rules.enabled(&feature_universal_number()))),
                _rules: Some(CurrentTransactionRulesGuard::new(rules.clone())),
                _mantissa_scale: None,
            }
        } else {
            Self {
                _st_number: None,
                _rules: None,
                _mantissa_scale: Some(NumberMantissaScaleGuard::new(MantissaScale::Small)),
            }
        }
    }
}
