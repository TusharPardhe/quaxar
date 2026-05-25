//! Deterministic
//! the reference implementation metadata, `preflight(...)`, `preclaim(...)`, and bounded
//! `doApply()` carrier shells.
//!
//! This ports the current top-level branch ordering around:
//!
//! - the zero loan-id malformed guard,
//! - the mutually-exclusive tx-specific flag check in `preflight(...)`,
//! - missing-loan rejection in `preclaim(...)`,
//! - default, impairment, and fully-paid permission gates,
//! - the "too soon to default" timing rule,
//! - the impossible missing-broker fallback to `tecINTERNAL`,
//! - the final broker-owner permission gate,
//! - the `doApply()` loan/broker/vault load order,
//! - the top-level default/impair/unimpair/noop branch ordering,
//! - the shared `owedToVault(...)` amount helper used by those branches,
//! - and the post-branch `associateAsset(...)` amendment gate.

use protocol::{NotTec, Ter, is_tes_success};
use std::ops::Sub;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct LoanManagePreflightFacts {
    pub loan_id_is_zero: bool,
    pub tx_specific_flags: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct LoanManagePreclaimFacts {
    pub loan_exists: bool,
    pub loan_is_defaulted: bool,
    pub loan_is_impaired: bool,
    pub tx_requests_impair: bool,
    pub tx_requests_unimpair: bool,
    pub tx_requests_default: bool,
    pub payment_remaining_is_zero: bool,
    pub default_is_too_soon: bool,
    pub broker_exists: bool,
    pub submitter_is_broker_owner: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct LoanManageDoApplyFacts {
    pub tx_requests_default: bool,
    pub tx_requests_impair: bool,
    pub tx_requests_unimpair: bool,
    pub security_fix_3_1_3_enabled: bool,
}

pub trait LoanManageDoApplyLoan {
    type BrokerId;
    type Asset;

    fn broker_id(&self) -> &Self::BrokerId;
    fn associate_asset(&mut self, asset: &Self::Asset);
}

pub trait LoanManageDoApplyBroker {
    type VaultId;
    type Asset;

    fn vault_id(&self) -> &Self::VaultId;
    fn associate_asset(&mut self, asset: &Self::Asset);
}

pub trait LoanManageDoApplyVault {
    type Asset;

    fn asset(&self) -> &Self::Asset;
    fn associate_asset(&mut self, asset: &Self::Asset);
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoanManageOwedToVaultFacts<Amount> {
    pub total_value_outstanding: Amount,
    pub management_fee_outstanding: Amount,
}

pub fn run_loan_manage_preflight(facts: LoanManagePreflightFacts) -> NotTec {
    if facts.loan_id_is_zero {
        return Ter::TEM_INVALID;
    }

    if facts.tx_specific_flags.count_ones() > 1 {
        return Ter::TEM_INVALID_FLAG;
    }

    Ter::TES_SUCCESS
}

pub fn run_loan_manage_preclaim(facts: LoanManagePreclaimFacts) -> Ter {
    if !facts.loan_exists {
        return Ter::TEC_NO_ENTRY;
    }

    if facts.loan_is_defaulted {
        return Ter::TEC_NO_PERMISSION;
    }

    if facts.loan_is_impaired && facts.tx_requests_impair {
        return Ter::TEC_NO_PERMISSION;
    }

    if !facts.loan_is_impaired && !facts.loan_is_defaulted && facts.tx_requests_unimpair {
        return Ter::TEC_NO_PERMISSION;
    }

    if facts.payment_remaining_is_zero {
        return Ter::TEC_NO_PERMISSION;
    }

    if facts.tx_requests_default && facts.default_is_too_soon {
        return Ter::TEC_TOO_SOON;
    }

    if !facts.broker_exists {
        return Ter::TEC_INTERNAL;
    }

    if !facts.submitter_is_broker_owner {
        return Ter::TEC_NO_PERMISSION;
    }

    Ter::TES_SUCCESS
}

pub fn run_loan_manage_owed_to_vault<Amount>(facts: LoanManageOwedToVaultFacts<Amount>) -> Amount
where
    Amount: Sub<Output = Amount>,
{
    facts.total_value_outstanding - facts.management_fee_outstanding
}

#[allow(clippy::too_many_arguments)]
pub fn run_loan_manage_do_apply<
    Loan,
    Broker,
    Vault,
    LoanId,
    BrokerId,
    VaultId,
    Asset,
    ReadLoan,
    ReadBroker,
    ReadVault,
    DefaultLoan,
    ImpairLoan,
    UnimpairLoan,
    UpdateLoan,
    UpdateBroker,
    UpdateVault,
>(
    loan_id: &LoanId,
    facts: LoanManageDoApplyFacts,
    read_loan: ReadLoan,
    read_broker: ReadBroker,
    read_vault: ReadVault,
    default_loan: DefaultLoan,
    impair_loan: ImpairLoan,
    unimpair_loan: UnimpairLoan,
    update_loan: UpdateLoan,
    update_broker: UpdateBroker,
    update_vault: UpdateVault,
) -> Ter
where
    Loan: LoanManageDoApplyLoan<BrokerId = BrokerId, Asset = Asset>,
    Broker: LoanManageDoApplyBroker<VaultId = VaultId, Asset = Asset>,
    Vault: LoanManageDoApplyVault<Asset = Asset>,
    Asset: Clone,
    ReadLoan: FnOnce(&LoanId) -> Option<Loan>,
    ReadBroker: FnOnce(&BrokerId) -> Option<Broker>,
    ReadVault: FnOnce(&VaultId) -> Option<Vault>,
    DefaultLoan: FnOnce(&mut Loan, &mut Broker, &mut Vault, &Asset) -> Ter,
    ImpairLoan: FnOnce(&mut Loan, &mut Vault, &Asset) -> Ter,
    UnimpairLoan: FnOnce(&mut Loan, &mut Vault, &Asset) -> Ter,
    UpdateLoan: FnOnce(&Loan),
    UpdateBroker: FnOnce(&Broker),
    UpdateVault: FnOnce(&Vault),
{
    let mut loan = match read_loan(loan_id) {
        Some(loan) => loan,
        None => return Ter::TEF_BAD_LEDGER,
    };

    let mut broker = match read_broker(loan.broker_id()) {
        Some(broker) => broker,
        None => return Ter::TEF_BAD_LEDGER,
    };

    let mut vault = match read_vault(broker.vault_id()) {
        Some(vault) => vault,
        None => return Ter::TEF_BAD_LEDGER,
    };
    let vault_asset = vault.asset().clone();

    let result = if facts.tx_requests_default {
        default_loan(&mut loan, &mut broker, &mut vault, &vault_asset)
    } else if facts.tx_requests_impair {
        impair_loan(&mut loan, &mut vault, &vault_asset)
    } else if facts.tx_requests_unimpair {
        unimpair_loan(&mut loan, &mut vault, &vault_asset)
    } else {
        Ter::TES_SUCCESS
    };

    if facts.security_fix_3_1_3_enabled && is_tes_success(result) {
        loan.associate_asset(&vault_asset);
        broker.associate_asset(&vault_asset);
        vault.associate_asset(&vault_asset);
        update_loan(&loan);
        update_broker(&broker);
        update_vault(&vault);
    }

    result
}

#[cfg(test)]
mod tests {
    use protocol::{Ter, trans_token};

    use super::{
        LoanManageOwedToVaultFacts, LoanManagePreclaimFacts, LoanManagePreflightFacts,
        run_loan_manage_owed_to_vault, run_loan_manage_preclaim, run_loan_manage_preflight,
    };

    fn base() -> LoanManagePreclaimFacts {
        LoanManagePreclaimFacts {
            loan_exists: true,
            loan_is_defaulted: false,
            loan_is_impaired: false,
            tx_requests_impair: false,
            tx_requests_unimpair: false,
            tx_requests_default: false,
            payment_remaining_is_zero: false,
            default_is_too_soon: false,
            broker_exists: true,
            submitter_is_broker_owner: true,
        }
    }

    #[test]
    fn loan_manage_preflight_rejects_zero_loan_id() {
        assert_eq!(
            run_loan_manage_preflight(LoanManagePreflightFacts {
                loan_id_is_zero: true,
                tx_specific_flags: 0,
            }),
            Ter::TEM_INVALID
        );
    }

    #[test]
    fn loan_manage_preflight_rejects_multiple_tx_specific_flags() {
        let result = run_loan_manage_preflight(LoanManagePreflightFacts {
            loan_id_is_zero: false,
            tx_specific_flags: 0x0001_0000 | 0x0002_0000,
        });

        assert_eq!(result, Ter::TEM_INVALID_FLAG);
        assert_eq!(trans_token(result), "temINVALID_FLAG");
    }

    #[test]
    fn loan_manage_preclaim_rejects_missing_loan() {
        let result = run_loan_manage_preclaim(LoanManagePreclaimFacts::default());

        assert_eq!(result, Ter::TEC_NO_ENTRY);
    }

    #[test]
    fn loan_manage_preclaim_rejects_defaulted_loan() {
        let result = run_loan_manage_preclaim(LoanManagePreclaimFacts {
            loan_is_defaulted: true,
            ..base()
        });

        assert_eq!(result, Ter::TEC_NO_PERMISSION);
    }

    #[test]
    fn loan_manage_preclaim_rejects_duplicate_impair() {
        let result = run_loan_manage_preclaim(LoanManagePreclaimFacts {
            loan_is_impaired: true,
            tx_requests_impair: true,
            ..base()
        });

        assert_eq!(result, Ter::TEC_NO_PERMISSION);
    }

    #[test]
    fn loan_manage_preclaim_rejects_unimpairing_unimpaired_loan() {
        let result = run_loan_manage_preclaim(LoanManagePreclaimFacts {
            tx_requests_unimpair: true,
            ..base()
        });

        assert_eq!(result, Ter::TEC_NO_PERMISSION);
    }

    #[test]
    fn loan_manage_preclaim_rejects_paid_off_loan() {
        let result = run_loan_manage_preclaim(LoanManagePreclaimFacts {
            payment_remaining_is_zero: true,
            ..base()
        });

        assert_eq!(result, Ter::TEC_NO_PERMISSION);
    }

    #[test]
    fn loan_manage_preclaim_rejects_early_default() {
        let result = run_loan_manage_preclaim(LoanManagePreclaimFacts {
            tx_requests_default: true,
            default_is_too_soon: true,
            ..base()
        });

        assert_eq!(result, Ter::TEC_TOO_SOON);
    }

    #[test]
    fn loan_manage_preclaim_maps_missing_broker_to_internal() {
        let result = run_loan_manage_preclaim(LoanManagePreclaimFacts {
            broker_exists: false,
            ..base()
        });

        assert_eq!(result, Ter::TEC_INTERNAL);
    }

    #[test]
    fn loan_manage_preclaim_rejects_non_owner_submitter() {
        let result = run_loan_manage_preclaim(LoanManagePreclaimFacts {
            submitter_is_broker_owner: false,
            ..base()
        });

        assert_eq!(result, Ter::TEC_NO_PERMISSION);
    }

    #[test]
    fn loan_manage_preclaim_accepts_allowed_transition() {
        assert_eq!(run_loan_manage_preclaim(base()), Ter::TES_SUCCESS);
    }

    #[test]
    fn loan_manage_owed_to_vault_matches_the_cpp_formula() {
        let amount = run_loan_manage_owed_to_vault(LoanManageOwedToVaultFacts {
            total_value_outstanding: 125_i64,
            management_fee_outstanding: 25_i64,
        });

        assert_eq!(amount, 100_i64);
    }

    #[test]
    fn loan_manage_owed_to_vault_preserves_zero_management_fee() {
        let amount = run_loan_manage_owed_to_vault(LoanManageOwedToVaultFacts {
            total_value_outstanding: 125_i64,
            management_fee_outstanding: 0_i64,
        });

        assert_eq!(amount, 125_i64);
    }
}
