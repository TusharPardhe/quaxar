//! Integration tests that pin the narrowed Rust vault runtime preflight shells
//! to the current C++ behavior.

use protocol::{Ter, trans_token};
use tx::{
    VaultClawbackPreflightFacts, VaultDepositPreflightFacts, VaultWithdrawPreflightFacts,
    run_vault_clawback_preflight, run_vault_deposit_preflight, run_vault_withdraw_preflight,
};

#[test]
fn vault_clawback_preflight_rejects_zero_vault_id() {
    let result = run_vault_clawback_preflight(VaultClawbackPreflightFacts {
        vault_id_is_zero: true,
        ..VaultClawbackPreflightFacts::default()
    });

    assert_eq!(result, Ter::TEM_MALFORMED);
    assert_eq!(trans_token(result), "temMALFORMED");
}

#[test]
fn vault_clawback_preflight_rejects_negative_amount() {
    let result = run_vault_clawback_preflight(VaultClawbackPreflightFacts {
        amount_present: true,
        amount_is_negative: true,
        ..VaultClawbackPreflightFacts::default()
    });

    assert_eq!(result, Ter::TEM_BAD_AMOUNT);
}

#[test]
fn vault_clawback_preflight_rejects_xrp_amount_asset() {
    let result = run_vault_clawback_preflight(VaultClawbackPreflightFacts {
        amount_present: true,
        amount_asset_is_xrp: true,
        ..VaultClawbackPreflightFacts::default()
    });

    assert_eq!(result, Ter::TEM_MALFORMED);
}

#[test]
fn vault_clawback_preflight_accepts_missing_or_zero_amount() {
    let missing = run_vault_clawback_preflight(VaultClawbackPreflightFacts::default());
    let zero = run_vault_clawback_preflight(VaultClawbackPreflightFacts {
        amount_present: true,
        ..VaultClawbackPreflightFacts::default()
    });

    assert_eq!(missing, Ter::TES_SUCCESS);
    assert_eq!(zero, Ter::TES_SUCCESS);
}

#[test]
fn vault_deposit_preflight_rejects_zero_vault_id() {
    let result = run_vault_deposit_preflight(VaultDepositPreflightFacts {
        vault_id_is_zero: true,
        amount_is_positive: true,
    });

    assert_eq!(result, Ter::TEM_MALFORMED);
}

#[test]
fn vault_deposit_preflight_rejects_non_positive_amount() {
    let result = run_vault_deposit_preflight(VaultDepositPreflightFacts::default());

    assert_eq!(result, Ter::TEM_BAD_AMOUNT);
}

#[test]
fn vault_deposit_preflight_accepts_positive_amount() {
    let result = run_vault_deposit_preflight(VaultDepositPreflightFacts {
        amount_is_positive: true,
        ..VaultDepositPreflightFacts::default()
    });

    assert_eq!(result, Ter::TES_SUCCESS);
}

#[test]
fn vault_withdraw_preflight_rejects_zero_vault_id() {
    let result = run_vault_withdraw_preflight(VaultWithdrawPreflightFacts {
        vault_id_is_zero: true,
        amount_is_positive: true,
        ..VaultWithdrawPreflightFacts::default()
    });

    assert_eq!(result, Ter::TEM_MALFORMED);
}

#[test]
fn vault_withdraw_preflight_rejects_non_positive_amount() {
    let result = run_vault_withdraw_preflight(VaultWithdrawPreflightFacts::default());

    assert_eq!(result, Ter::TEM_BAD_AMOUNT);
}

#[test]
fn vault_withdraw_preflight_rejects_zero_destination() {
    let result = run_vault_withdraw_preflight(VaultWithdrawPreflightFacts {
        amount_is_positive: true,
        destination_present: true,
        destination_is_zero: true,
        ..VaultWithdrawPreflightFacts::default()
    });

    assert_eq!(result, Ter::TEM_MALFORMED);
}

#[test]
fn vault_withdraw_preflight_accepts_missing_or_nonzero_destination() {
    let missing = run_vault_withdraw_preflight(VaultWithdrawPreflightFacts {
        amount_is_positive: true,
        ..VaultWithdrawPreflightFacts::default()
    });
    let present = run_vault_withdraw_preflight(VaultWithdrawPreflightFacts {
        amount_is_positive: true,
        destination_present: true,
        ..VaultWithdrawPreflightFacts::default()
    });

    assert_eq!(missing, Ter::TES_SUCCESS);
    assert_eq!(present, Ter::TES_SUCCESS);
}
