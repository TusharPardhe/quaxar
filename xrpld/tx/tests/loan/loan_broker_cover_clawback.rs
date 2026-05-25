//! Integration tests that pin the narrowed Rust
//! `LoanBrokerCoverClawback.cpp` wrapper to the current C++ behavior.

use protocol::{Ter, trans_token};
use tx::{
    LoanBrokerCoverClawbackAmountKind, LoanBrokerCoverClawbackPreclaimFacts,
    LoanBrokerCoverClawbackPreflightFacts, LoanBrokerCoverClawbackResolveBrokerIdFacts,
    run_loan_broker_cover_clawback_check_extra_features,
    run_loan_broker_cover_clawback_determine_amount, run_loan_broker_cover_clawback_preclaim,
    run_loan_broker_cover_clawback_preflight, run_loan_broker_cover_clawback_resolve_broker_id,
};

fn base() -> LoanBrokerCoverClawbackPreclaimFacts {
    LoanBrokerCoverClawbackPreclaimFacts {
        broker_id_resolution_result: Ter::TES_SUCCESS,
        broker_exists: true,
        vault_exists: true,
        vault_asset_is_native: false,
        submitter_is_vault_asset_issuer: true,
        amount_is_present: false,
        amount_asset_matches_vault_asset: true,
        claw_amount_can_be_determined: true,
        pseudo_balance_at_least_claw_amount: true,
        issuer_account_exists: true,
        amount_kind: LoanBrokerCoverClawbackAmountKind::Issue,
        mpt_issuance_exists: true,
        mpt_can_clawback: true,
        mpt_issuer_matches_submitter: true,
        issuer_allows_trustline_clawback: true,
        issuer_has_no_freeze: false,
    }
}

#[test]
fn tx_loan_broker_cover_clawback_check_extra_features_delegates() {
    let mut called = false;
    assert!(!run_loan_broker_cover_clawback_check_extra_features(
        false,
        || {
            called = true;
            true
        }
    ));
    assert!(!called);
    assert!(run_loan_broker_cover_clawback_check_extra_features(
        true,
        || true
    ));
    assert!(!run_loan_broker_cover_clawback_check_extra_features(
        true,
        || false
    ));
}

#[test]
fn tx_loan_broker_cover_clawback_preflight_rejects_missing_inputs() {
    assert_eq!(
        run_loan_broker_cover_clawback_preflight(LoanBrokerCoverClawbackPreflightFacts::default()),
        Ter::TEM_INVALID
    );
}

#[test]
fn tx_loan_broker_cover_clawback_preflight_rejects_bad_ids_and_amounts() {
    assert_eq!(
        run_loan_broker_cover_clawback_preflight(LoanBrokerCoverClawbackPreflightFacts {
            broker_id_is_present: true,
            broker_id_is_zero: true,
            ..LoanBrokerCoverClawbackPreflightFacts::default()
        }),
        Ter::TEM_INVALID
    );
    assert_eq!(
        run_loan_broker_cover_clawback_preflight(LoanBrokerCoverClawbackPreflightFacts {
            amount_is_present: true,
            amount_is_native: true,
            amount_is_legal_net: true,
            ..LoanBrokerCoverClawbackPreflightFacts::default()
        }),
        Ter::TEM_BAD_AMOUNT
    );
    assert_eq!(
        run_loan_broker_cover_clawback_preflight(LoanBrokerCoverClawbackPreflightFacts {
            amount_is_present: true,
            amount_is_negative: true,
            amount_is_legal_net: true,
            ..LoanBrokerCoverClawbackPreflightFacts::default()
        }),
        Ter::TEM_BAD_AMOUNT
    );
    assert_eq!(
        run_loan_broker_cover_clawback_preflight(LoanBrokerCoverClawbackPreflightFacts {
            amount_is_present: true,
            amount_is_legal_net: false,
            ..LoanBrokerCoverClawbackPreflightFacts::default()
        }),
        Ter::TEM_BAD_AMOUNT
    );
}

#[test]
fn tx_loan_broker_cover_clawback_preflight_rejects_missing_id_mpt_and_bad_holders() {
    assert_eq!(
        run_loan_broker_cover_clawback_preflight(LoanBrokerCoverClawbackPreflightFacts {
            amount_is_present: true,
            amount_is_legal_net: true,
            broker_id_missing_amount_is_mpt: true,
            ..LoanBrokerCoverClawbackPreflightFacts::default()
        }),
        Ter::TEM_INVALID
    );
    assert_eq!(
        run_loan_broker_cover_clawback_preflight(LoanBrokerCoverClawbackPreflightFacts {
            amount_is_present: true,
            amount_is_legal_net: true,
            broker_id_missing_amount_holder_is_account: true,
            ..LoanBrokerCoverClawbackPreflightFacts::default()
        }),
        Ter::TEM_INVALID
    );
    assert_eq!(
        run_loan_broker_cover_clawback_preflight(LoanBrokerCoverClawbackPreflightFacts {
            amount_is_present: true,
            amount_is_legal_net: true,
            broker_id_missing_amount_holder_is_zero: true,
            ..LoanBrokerCoverClawbackPreflightFacts::default()
        }),
        Ter::TEM_INVALID
    );
}

#[test]
fn tx_loan_broker_cover_clawback_preclaim_returns_broker_resolution_failure() {
    let result = run_loan_broker_cover_clawback_preclaim(LoanBrokerCoverClawbackPreclaimFacts {
        broker_id_resolution_result: Ter::TEC_OBJECT_NOT_FOUND,
        ..base()
    });

    assert_eq!(result, Ter::TEC_OBJECT_NOT_FOUND);
    assert_eq!(trans_token(result), "tecOBJECT_NOT_FOUND");
}

#[test]
fn tx_loan_broker_cover_clawback_resolve_broker_id_matches_current_cpp_routes() {
    assert_eq!(
        run_loan_broker_cover_clawback_resolve_broker_id(
            LoanBrokerCoverClawbackResolveBrokerIdFacts {
                broker_id_from_tx: Some("broker-1"),
                amount_is_present: false,
                amount_holds_issue: false,
                holder_account_exists: false,
                broker_id_from_holder_account: None,
            }
        ),
        Ok("broker-1")
    );
    assert_eq!(
        run_loan_broker_cover_clawback_resolve_broker_id(
            LoanBrokerCoverClawbackResolveBrokerIdFacts::<&str> {
                broker_id_from_tx: None,
                amount_is_present: false,
                amount_holds_issue: false,
                holder_account_exists: false,
                broker_id_from_holder_account: None,
            }
        ),
        Err(Ter::TEC_INTERNAL)
    );
    assert_eq!(
        run_loan_broker_cover_clawback_resolve_broker_id(
            LoanBrokerCoverClawbackResolveBrokerIdFacts::<&str> {
                broker_id_from_tx: None,
                amount_is_present: true,
                amount_holds_issue: false,
                holder_account_exists: false,
                broker_id_from_holder_account: None,
            }
        ),
        Err(Ter::TEC_INTERNAL)
    );
    assert_eq!(
        run_loan_broker_cover_clawback_resolve_broker_id(
            LoanBrokerCoverClawbackResolveBrokerIdFacts::<&str> {
                broker_id_from_tx: None,
                amount_is_present: true,
                amount_holds_issue: true,
                holder_account_exists: false,
                broker_id_from_holder_account: None,
            }
        ),
        Err(Ter::TEC_NO_ENTRY)
    );
    assert_eq!(
        run_loan_broker_cover_clawback_resolve_broker_id(
            LoanBrokerCoverClawbackResolveBrokerIdFacts::<&str> {
                broker_id_from_tx: None,
                amount_is_present: true,
                amount_holds_issue: true,
                holder_account_exists: true,
                broker_id_from_holder_account: None,
            }
        ),
        Err(Ter::TEC_OBJECT_NOT_FOUND)
    );
    assert_eq!(
        run_loan_broker_cover_clawback_resolve_broker_id(
            LoanBrokerCoverClawbackResolveBrokerIdFacts {
                broker_id_from_tx: None,
                amount_is_present: true,
                amount_holds_issue: true,
                holder_account_exists: true,
                broker_id_from_holder_account: Some("broker-2"),
            }
        ),
        Ok("broker-2")
    );
}

#[test]
fn tx_loan_broker_cover_clawback_preclaim_rejects_missing_broker_vault_native_and_wrong_issuer() {
    assert_eq!(
        run_loan_broker_cover_clawback_preclaim(LoanBrokerCoverClawbackPreclaimFacts {
            broker_exists: false,
            ..base()
        }),
        Ter::TEC_NO_ENTRY
    );
    assert_eq!(
        run_loan_broker_cover_clawback_preclaim(LoanBrokerCoverClawbackPreclaimFacts {
            vault_exists: false,
            ..base()
        }),
        Ter::TEF_BAD_LEDGER
    );
    assert_eq!(
        run_loan_broker_cover_clawback_preclaim(LoanBrokerCoverClawbackPreclaimFacts {
            vault_asset_is_native: true,
            ..base()
        }),
        Ter::TEC_NO_PERMISSION
    );
    assert_eq!(
        run_loan_broker_cover_clawback_preclaim(LoanBrokerCoverClawbackPreclaimFacts {
            submitter_is_vault_asset_issuer: false,
            ..base()
        }),
        Ter::TEC_NO_PERMISSION
    );
}

#[test]
fn tx_loan_broker_cover_clawback_preclaim_rejects_wrong_asset_and_minimum_cover() {
    let wrong_asset =
        run_loan_broker_cover_clawback_preclaim(LoanBrokerCoverClawbackPreclaimFacts {
            amount_is_present: true,
            amount_asset_matches_vault_asset: false,
            ..base()
        });
    let minimum = run_loan_broker_cover_clawback_preclaim(LoanBrokerCoverClawbackPreclaimFacts {
        claw_amount_can_be_determined: false,
        ..base()
    });

    assert_eq!(wrong_asset, Ter::TEC_WRONG_ASSET);
    assert_eq!(trans_token(wrong_asset), "tecWRONG_ASSET");
    assert_eq!(minimum, Ter::TEC_INSUFFICIENT_FUNDS);
}

#[test]
fn tx_loan_broker_cover_clawback_preclaim_rejects_balance_and_missing_issuer() {
    assert_eq!(
        run_loan_broker_cover_clawback_preclaim(LoanBrokerCoverClawbackPreclaimFacts {
            pseudo_balance_at_least_claw_amount: false,
            ..base()
        }),
        Ter::TEC_INTERNAL
    );
    assert_eq!(
        run_loan_broker_cover_clawback_preclaim(LoanBrokerCoverClawbackPreclaimFacts {
            issuer_account_exists: false,
            ..base()
        }),
        Ter::TEF_BAD_LEDGER
    );
}

#[test]
fn tx_loan_broker_cover_clawback_preclaim_enforces_iou_issuer_flags() {
    assert_eq!(
        run_loan_broker_cover_clawback_preclaim(LoanBrokerCoverClawbackPreclaimFacts {
            issuer_allows_trustline_clawback: false,
            ..base()
        }),
        Ter::TEC_NO_PERMISSION
    );
    assert_eq!(
        run_loan_broker_cover_clawback_preclaim(LoanBrokerCoverClawbackPreclaimFacts {
            issuer_has_no_freeze: true,
            ..base()
        }),
        Ter::TEC_NO_PERMISSION
    );
}

#[test]
fn tx_loan_broker_cover_clawback_preclaim_enforces_mpt_rules() {
    assert_eq!(
        run_loan_broker_cover_clawback_preclaim(LoanBrokerCoverClawbackPreclaimFacts {
            amount_kind: LoanBrokerCoverClawbackAmountKind::Mpt,
            mpt_issuance_exists: false,
            ..base()
        }),
        Ter::TEC_OBJECT_NOT_FOUND
    );
    assert_eq!(
        run_loan_broker_cover_clawback_preclaim(LoanBrokerCoverClawbackPreclaimFacts {
            amount_kind: LoanBrokerCoverClawbackAmountKind::Mpt,
            mpt_can_clawback: false,
            ..base()
        }),
        Ter::TEC_NO_PERMISSION
    );
    assert_eq!(
        run_loan_broker_cover_clawback_preclaim(LoanBrokerCoverClawbackPreclaimFacts {
            amount_kind: LoanBrokerCoverClawbackAmountKind::Mpt,
            mpt_issuer_matches_submitter: false,
            ..base()
        }),
        Ter::TEC_INTERNAL
    );
}

#[test]
fn tx_loan_broker_cover_clawback_preclaim_accepts_valid_iou_and_mpt_cases() {
    assert_eq!(
        run_loan_broker_cover_clawback_preclaim(base()),
        Ter::TES_SUCCESS
    );
    assert_eq!(
        run_loan_broker_cover_clawback_preclaim(LoanBrokerCoverClawbackPreclaimFacts {
            amount_kind: LoanBrokerCoverClawbackAmountKind::Mpt,
            ..base()
        }),
        Ter::TES_SUCCESS
    );
}

#[test]
fn tx_loan_broker_cover_clawback_determine_amount_matches_current_cpp_clamp_rules() {
    assert_eq!(
        run_loan_broker_cover_clawback_determine_amount(50_i64, None),
        Ok(50_i64)
    );
    assert_eq!(
        run_loan_broker_cover_clawback_determine_amount(50_i64, Some(0_i64)),
        Ok(50_i64)
    );
    assert_eq!(
        run_loan_broker_cover_clawback_determine_amount(50_i64, Some(75_i64)),
        Ok(50_i64)
    );
    assert_eq!(
        run_loan_broker_cover_clawback_determine_amount(50_i64, Some(25_i64)),
        Ok(25_i64)
    );
    assert_eq!(
        run_loan_broker_cover_clawback_determine_amount(0_i64, None),
        Err(Ter::TEC_INSUFFICIENT_FUNDS)
    );
}

#[test]
fn tx_loan_broker_cover_clawback_do_apply_surface_is_exported_slice() {
    let result = tx::loan_broker_cover_clawback::run_loan_broker_cover_clawback_do_apply(
        &mut SurfaceSmokeSink,
        &"issuer",
        || Ok("broker"),
        |_, _| Ok(SurfaceSmokeAmount { native: false }),
    );

    assert_eq!(result, Ter::TEC_INTERNAL);
}

struct SurfaceSmokeSink;

impl tx::loan_broker_cover_clawback::LoanBrokerCoverClawbackDoApplySink for SurfaceSmokeSink {
    type Broker = SurfaceSmokeBroker;
    type Vault = SurfaceSmokeVault;
    type BrokerId = &'static str;
    type AccountId = &'static str;
    type Amount = SurfaceSmokeAmount;
    type Asset = &'static str;
    type VaultId = &'static str;

    fn read_broker(&mut self, _broker_id: &Self::BrokerId) -> Option<Self::Broker> {
        None
    }

    fn read_vault(&mut self, _vault_id: &Self::VaultId) -> Option<Self::Vault> {
        None
    }

    fn update_broker(&mut self, _broker: &Self::Broker) {}

    fn associate_asset(&mut self, _broker: &Self::Broker, _asset: &Self::Asset) {}

    fn send_asset(
        &mut self,
        _pseudo_account_id: &Self::AccountId,
        _destination: &Self::AccountId,
        _amount: &Self::Amount,
    ) -> Ter {
        Ter::TES_SUCCESS
    }
}

struct SurfaceSmokeBroker;

impl tx::loan_broker_cover_clawback::LoanBrokerCoverClawbackDoApplyBroker for SurfaceSmokeBroker {
    type AccountId = &'static str;
    type Amount = SurfaceSmokeAmount;
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

impl tx::loan_broker_cover_clawback::LoanBrokerCoverClawbackDoApplyVault for SurfaceSmokeVault {
    type Asset = &'static str;

    fn asset(&self) -> &Self::Asset {
        &"USD"
    }
}

struct SurfaceSmokeAmount {
    native: bool,
}

impl tx::loan_broker_cover_clawback::LoanBrokerCoverClawbackDoApplyAmount for SurfaceSmokeAmount {
    fn is_native(&self) -> bool {
        self.native
    }
}
