//! Public fee-entry wrappers in the transaction dispatch layer.
//!
//! This ports the exact public delegation shape of `calculateBaseFee(...)`
//! and `calculateDefaultBaseFee(...)`.

use crate::run_calculate_default_base_fee_with_context;

pub fn run_calculate_base_fee_entrypoint<View: ?Sized, Tx: ?Sized, Fee>(
    view: &View,
    tx: &Tx,
    invoke_calculate_base_fee: impl FnOnce(&View, &Tx) -> Fee,
) -> Fee {
    invoke_calculate_base_fee(view, tx)
}

pub fn run_calculate_default_base_fee_entrypoint<View: ?Sized, Tx: ?Sized, Fee>(
    view: &View,
    tx: &Tx,
    calculate_default_base_fee: impl FnOnce(&View, &Tx) -> Fee,
) -> Fee {
    run_calculate_default_base_fee_with_context(view, tx, calculate_default_base_fee)
}

#[cfg(test)]
mod tests {
    use super::{run_calculate_base_fee_entrypoint, run_calculate_default_base_fee_entrypoint};

    #[test]
    fn calculate_base_fee_entrypoint_delegates_to_invoke_shell() {
        let fee = run_calculate_base_fee_entrypoint("view", "tx", |view, tx| {
            assert_eq!(view, "view");
            assert_eq!(tx, "tx");
            12_u64
        });

        assert_eq!(fee, 12_u64);
    }

    #[test]
    fn calculate_default_base_fee_entrypoint_delegates_to_default_shell() {
        let fee = run_calculate_default_base_fee_entrypoint("view", "tx", |view, tx| {
            assert_eq!(view, "view");
            assert_eq!(tx, "tx");
            9_u64
        });

        assert_eq!(fee, 9_u64);
    }
}
