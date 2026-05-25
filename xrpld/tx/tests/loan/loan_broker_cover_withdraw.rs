//! Integration tests that pin the narrowed Rust
//! `LoanBrokerCoverWithdraw.cpp` wrapper to the current C++ behavior.

use protocol::{Ter, trans_token};
use tx::{
    LoanBrokerCoverWithdrawPreclaimFacts, LoanBrokerCoverWithdrawPreflightFacts,
    run_loan_broker_cover_withdraw_check_extra_features, run_loan_broker_cover_withdraw_preclaim,
    run_loan_broker_cover_withdraw_preflight,
};

fn base() -> LoanBrokerCoverWithdrawPreclaimFacts {
    LoanBrokerCoverWithdrawPreclaimFacts {
        destination_is_pseudo_account: false,
        broker_exists: true,
        submitter_is_broker_owner: true,
        vault_exists: true,
        amount_matches_vault_asset: true,
        destination_is_submitter: true,
        destination_is_vault_asset_issuer: false,
        cover_available_at_least_amount: true,
        cover_after_withdraw_at_least_minimum: true,
        pseudo_balance_at_least_amount: true,
        can_transfer_result: Ter::TES_SUCCESS,
        can_withdraw_result: Ter::TES_SUCCESS,
        require_auth_result: Ter::TES_SUCCESS,
        source_frozen_result: Ter::TES_SUCCESS,
        destination_deep_frozen_result: Ter::TES_SUCCESS,
    }
}

#[test]
fn tx_loan_broker_cover_withdraw_check_extra_features_delegates() {
    let mut called = false;
    assert!(!run_loan_broker_cover_withdraw_check_extra_features(
        false,
        || {
            called = true;
            true
        }
    ));
    assert!(!called);
    assert!(run_loan_broker_cover_withdraw_check_extra_features(
        true,
        || true
    ));
    assert!(!run_loan_broker_cover_withdraw_check_extra_features(
        true,
        || false
    ));
}

#[test]
fn tx_loan_broker_cover_withdraw_preflight_rejects_zero_broker_id() {
    assert_eq!(
        run_loan_broker_cover_withdraw_preflight(LoanBrokerCoverWithdrawPreflightFacts {
            loan_broker_id_is_zero: true,
            amount_is_positive: true,
            amount_is_legal_net: true,
            destination_is_present: false,
            destination_is_zero: false,
        }),
        Ter::TEM_INVALID
    );
}

#[test]
fn tx_loan_broker_cover_withdraw_preflight_rejects_bad_amounts_and_zero_destination() {
    assert_eq!(
        run_loan_broker_cover_withdraw_preflight(LoanBrokerCoverWithdrawPreflightFacts {
            loan_broker_id_is_zero: false,
            amount_is_positive: false,
            amount_is_legal_net: true,
            destination_is_present: false,
            destination_is_zero: false,
        }),
        Ter::TEM_BAD_AMOUNT
    );
    assert_eq!(
        run_loan_broker_cover_withdraw_preflight(LoanBrokerCoverWithdrawPreflightFacts {
            loan_broker_id_is_zero: false,
            amount_is_positive: true,
            amount_is_legal_net: false,
            destination_is_present: false,
            destination_is_zero: false,
        }),
        Ter::TEM_BAD_AMOUNT
    );
    let malformed =
        run_loan_broker_cover_withdraw_preflight(LoanBrokerCoverWithdrawPreflightFacts {
            loan_broker_id_is_zero: false,
            amount_is_positive: true,
            amount_is_legal_net: true,
            destination_is_present: true,
            destination_is_zero: true,
        });
    assert_eq!(malformed, Ter::TEM_MALFORMED);
    assert_eq!(trans_token(malformed), "temMALFORMED");
}

#[test]
fn tx_loan_broker_cover_withdraw_preclaim_rejects_pseudo_destination() {
    assert_eq!(
        run_loan_broker_cover_withdraw_preclaim(LoanBrokerCoverWithdrawPreclaimFacts {
            destination_is_pseudo_account: true,
            ..base()
        }),
        Ter::TEC_PSEUDO_ACCOUNT
    );
}

#[test]
fn tx_loan_broker_cover_withdraw_preclaim_rejects_missing_broker_wrong_owner_missing_vault() {
    assert_eq!(
        run_loan_broker_cover_withdraw_preclaim(LoanBrokerCoverWithdrawPreclaimFacts {
            broker_exists: false,
            ..base()
        }),
        Ter::TEC_NO_ENTRY
    );
    assert_eq!(
        run_loan_broker_cover_withdraw_preclaim(LoanBrokerCoverWithdrawPreclaimFacts {
            submitter_is_broker_owner: false,
            ..base()
        }),
        Ter::TEC_NO_PERMISSION
    );
    assert_eq!(
        run_loan_broker_cover_withdraw_preclaim(LoanBrokerCoverWithdrawPreclaimFacts {
            vault_exists: false,
            ..base()
        }),
        Ter::TEF_BAD_LEDGER
    );
}

#[test]
fn tx_loan_broker_cover_withdraw_preclaim_rejects_wrong_asset() {
    let result = run_loan_broker_cover_withdraw_preclaim(LoanBrokerCoverWithdrawPreclaimFacts {
        amount_matches_vault_asset: false,
        ..base()
    });

    assert_eq!(result, Ter::TEC_WRONG_ASSET);
    assert_eq!(trans_token(result), "tecWRONG_ASSET");
}

#[test]
fn tx_loan_broker_cover_withdraw_preclaim_preserves_transfer_withdraw_auth_and_freeze_failures() {
    assert_eq!(
        run_loan_broker_cover_withdraw_preclaim(LoanBrokerCoverWithdrawPreclaimFacts {
            can_transfer_result: Ter::TER_NO_RIPPLE,
            ..base()
        }),
        Ter::TER_NO_RIPPLE
    );
    assert_eq!(
        run_loan_broker_cover_withdraw_preclaim(LoanBrokerCoverWithdrawPreclaimFacts {
            destination_is_submitter: false,
            can_withdraw_result: Ter::TEC_NO_PERMISSION,
            ..base()
        }),
        Ter::TEC_NO_PERMISSION
    );
    assert_eq!(
        run_loan_broker_cover_withdraw_preclaim(LoanBrokerCoverWithdrawPreclaimFacts {
            require_auth_result: Ter::TER_NO_AUTH,
            ..base()
        }),
        Ter::TER_NO_AUTH
    );
    assert_eq!(
        run_loan_broker_cover_withdraw_preclaim(LoanBrokerCoverWithdrawPreclaimFacts {
            source_frozen_result: Ter::TEC_FROZEN,
            ..base()
        }),
        Ter::TEC_FROZEN
    );
    assert_eq!(
        run_loan_broker_cover_withdraw_preclaim(LoanBrokerCoverWithdrawPreclaimFacts {
            destination_deep_frozen_result: Ter::TEC_FROZEN,
            ..base()
        }),
        Ter::TEC_FROZEN
    );
}

#[test]
fn tx_loan_broker_cover_withdraw_preclaim_skips_self_withdraw_and_issuer_freeze_checks() {
    assert_eq!(
        run_loan_broker_cover_withdraw_preclaim(LoanBrokerCoverWithdrawPreclaimFacts {
            destination_is_submitter: true,
            can_withdraw_result: Ter::TEC_NO_PERMISSION,
            ..base()
        }),
        Ter::TES_SUCCESS
    );
    assert_eq!(
        run_loan_broker_cover_withdraw_preclaim(LoanBrokerCoverWithdrawPreclaimFacts {
            destination_is_vault_asset_issuer: true,
            source_frozen_result: Ter::TEC_FROZEN,
            destination_deep_frozen_result: Ter::TEC_FROZEN,
            ..base()
        }),
        Ter::TES_SUCCESS
    );
}

#[test]
fn tx_loan_broker_cover_withdraw_preclaim_enforces_cover_and_balance_floors() {
    assert_eq!(
        run_loan_broker_cover_withdraw_preclaim(LoanBrokerCoverWithdrawPreclaimFacts {
            cover_available_at_least_amount: false,
            ..base()
        }),
        Ter::TEC_INSUFFICIENT_FUNDS
    );
    assert_eq!(
        run_loan_broker_cover_withdraw_preclaim(LoanBrokerCoverWithdrawPreclaimFacts {
            cover_after_withdraw_at_least_minimum: false,
            ..base()
        }),
        Ter::TEC_INSUFFICIENT_FUNDS
    );
    assert_eq!(
        run_loan_broker_cover_withdraw_preclaim(LoanBrokerCoverWithdrawPreclaimFacts {
            pseudo_balance_at_least_amount: false,
            ..base()
        }),
        Ter::TEC_INSUFFICIENT_FUNDS
    );
}

#[test]
fn tx_loan_broker_cover_withdraw_do_apply_surface_is_exported_slice() {
    let runner =
        tx::loan_broker_cover_withdraw::run_loan_broker_cover_withdraw_do_apply::<SurfaceSmokeSink>;
    let _ = runner;
}

struct SurfaceSmokeSink;

impl tx::loan_broker_cover_withdraw::LoanBrokerCoverWithdrawDoApplySink for SurfaceSmokeSink {
    type Broker = SurfaceSmokeBroker;
    type Vault = SurfaceSmokeVault;
    type AccountId = &'static str;
    type Amount = i64;
    type Asset = &'static str;
    type VaultId = &'static str;

    fn read_broker(&mut self) -> Option<Self::Broker> {
        None
    }

    fn read_vault(&mut self, _vault_id: &Self::VaultId) -> Option<Self::Vault> {
        None
    }

    fn update_broker(&mut self, _broker: &Self::Broker) {}

    fn associate_asset(&mut self, _broker: &Self::Broker, _asset: &Self::Asset) {}

    fn do_withdraw(
        &mut self,
        _destination: &Self::AccountId,
        _pseudo_account_id: &Self::AccountId,
        _pre_fee_balance: &Self::Amount,
        _amount: &Self::Amount,
    ) -> Ter {
        Ter::TES_SUCCESS
    }
}

struct SurfaceSmokeBroker;

impl tx::loan_broker_cover_withdraw::LoanBrokerCoverWithdrawDoApplyBroker for SurfaceSmokeBroker {
    type AccountId = &'static str;
    type Amount = i64;
    type Asset = &'static str;
    type VaultId = &'static str;

    fn vault_id(&self) -> &Self::VaultId {
        &"vault"
    }

    fn pseudo_account_id(&self) -> &Self::AccountId {
        &"pseudo"
    }

    fn subtract_cover_available(&mut self, _amount: &Self::Amount) {}
}

struct SurfaceSmokeVault;

impl tx::loan_broker_cover_withdraw::LoanBrokerCoverWithdrawDoApplyVault for SurfaceSmokeVault {
    type Asset = &'static str;

    fn asset(&self) -> &Self::Asset {
        &"USD"
    }
}
