//! Current Rust helper mirroring `Transactor::minimumFee(...)` and the
//! immediate `checkFee(...)` bridge that consumes it.
//!
//! This module preserves the current `tapUNLIMITED` handling and the direct
//! bridge into the landed `checkFee(...)` shell.

use protocol::Ter;

use crate::{ApplyFlags, any_apply_flags, run_transactor_check_fee};

pub fn run_transactor_minimum_fee<Registry: ?Sized, Fees: ?Sized, Fee>(
    registry: &Registry,
    base_fee: Fee,
    fees: &Fees,
    flags: ApplyFlags,
    scale_fee_load: impl FnOnce(Fee, &Registry, &Fees, bool) -> Fee,
) -> Fee {
    scale_fee_load(
        base_fee,
        registry,
        fees,
        any_apply_flags(flags & ApplyFlags::UNLIMITED),
    )
}

#[allow(clippy::too_many_arguments)]
pub fn run_transactor_check_fee_with_minimum_fee<
    Tx,
    AccountState,
    Registry: ?Sized,
    Fees: ?Sized,
    IsLegalAmount,
    ScaleFeeLoad,
    ReadAccount,
    AccountBalance,
>(
    flags: ApplyFlags,
    ledger_open: bool,
    tx: &Tx,
    base_fee: Tx::Amount,
    zero: Tx::Amount,
    registry: &Registry,
    fees: &Fees,
    is_legal_amount: IsLegalAmount,
    scale_fee_load: ScaleFeeLoad,
    read_account: ReadAccount,
    account_balance: AccountBalance,
) -> Ter
where
    Tx: crate::TransactorCheckFeeTx,
    IsLegalAmount: FnMut(Tx::Amount) -> bool,
    ScaleFeeLoad: FnOnce(Tx::Amount, &Registry, &Fees, bool) -> Tx::Amount,
    ReadAccount: FnMut(&Tx::AccountId) -> Option<AccountState>,
    AccountBalance: FnMut(&AccountState) -> Tx::Amount,
{
    run_transactor_check_fee(
        flags,
        ledger_open,
        tx,
        base_fee,
        zero,
        is_legal_amount,
        |base_fee| run_transactor_minimum_fee(registry, base_fee, fees, flags, scale_fee_load),
        read_account,
        account_balance,
    )
}

#[cfg(test)]
mod tests {
    use protocol::Ter;

    use super::{run_transactor_check_fee_with_minimum_fee, run_transactor_minimum_fee};
    use crate::{ApplyFlags, TransactorCheckFeeTx};

    #[test]
    fn transactor_minimum_fee_passes_unlimited_flag() {
        let observed = run_transactor_minimum_fee(
            "registry",
            10_u64,
            "fees",
            ApplyFlags::UNLIMITED,
            |base_fee, registry, fees, unlimited| {
                assert_eq!(base_fee, 10);
                assert_eq!(registry, "registry");
                assert_eq!(fees, "fees");
                assert!(unlimited);
                22_u64
            },
        );

        assert_eq!(observed, 22);
    }

    #[test]
    fn transactor_minimum_fee_clears_unlimited_flag() {
        let observed = run_transactor_minimum_fee(
            "registry",
            10_u64,
            "fees",
            ApplyFlags::NONE,
            |_, _, _, unlimited| {
                assert!(!unlimited);
                33_u64
            },
        );

        assert_eq!(observed, 33);
    }

    struct FeeTx {
        fee_is_native: bool,
        fee_paid: i64,
        fee_payer: &'static str,
    }

    impl TransactorCheckFeeTx for FeeTx {
        type AccountId = &'static str;
        type Amount = i64;

        fn fee_is_native(&self) -> bool {
            self.fee_is_native
        }

        fn fee_paid(&self) -> Self::Amount {
            self.fee_paid
        }

        fn fee_payer(&self) -> Self::AccountId {
            self.fee_payer
        }
    }

    #[test]
    fn transactor_check_fee_bridge_uses_minimum_fee_helper() {
        let result = run_transactor_check_fee_with_minimum_fee(
            ApplyFlags::UNLIMITED,
            true,
            &FeeTx {
                fee_is_native: true,
                fee_paid: 19,
                fee_payer: "alice",
            },
            10,
            0,
            &"registry",
            &"fees",
            |_| true,
            |base_fee, registry, fees, unlimited| {
                assert_eq!(base_fee, 10);
                assert_eq!(registry, &"registry");
                assert_eq!(fees, &"fees");
                assert!(unlimited);
                20
            },
            |_| Some(100_i64),
            |balance| *balance,
        );

        assert_eq!(result, Ter::TEL_INSUF_FEE_P);
    }
}
