//! Integration tests that pin the narrowed Rust `Clawback.cpp`
//! preflight/preclaim shells to the current C++ behavior.

use std::cmp::Ordering;

use protocol::{Ter, trans_token};
use tx::{
    ClawbackAssetKind, ClawbackIssuePreclaimFacts, ClawbackMptPreclaimFacts,
    ClawbackPreclaimAssetFacts, ClawbackPreclaimFacts, ClawbackPreflightFacts,
    ClawbackTrustlineBalanceSign, run_clawback_preclaim, run_clawback_preflight,
};

#[test]
fn clawback_issue_preflight_rejects_holder_field() {
    let result = run_clawback_preflight(ClawbackPreflightFacts {
        asset_kind: ClawbackAssetKind::Issue,
        holder_field_present: true,
        mptokens_v1_enabled: false,
        issuer_equals_holder: false,
        amount_is_xrp: false,
        amount_positive: true,
        mpt_amount_exceeds_max: false,
    });

    assert_eq!(result, Ter::TEM_MALFORMED);
    assert_eq!(trans_token(result), "temMALFORMED");
}

#[test]
fn clawback_issue_preflight_rejects_self_xrp_or_non_positive_amount() {
    let self_holder = run_clawback_preflight(ClawbackPreflightFacts {
        asset_kind: ClawbackAssetKind::Issue,
        holder_field_present: false,
        mptokens_v1_enabled: false,
        issuer_equals_holder: true,
        amount_is_xrp: false,
        amount_positive: true,
        mpt_amount_exceeds_max: false,
    });
    let xrp = run_clawback_preflight(ClawbackPreflightFacts {
        asset_kind: ClawbackAssetKind::Issue,
        holder_field_present: false,
        mptokens_v1_enabled: false,
        issuer_equals_holder: false,
        amount_is_xrp: true,
        amount_positive: true,
        mpt_amount_exceeds_max: false,
    });
    let non_positive = run_clawback_preflight(ClawbackPreflightFacts {
        asset_kind: ClawbackAssetKind::Issue,
        holder_field_present: false,
        mptokens_v1_enabled: false,
        issuer_equals_holder: false,
        amount_is_xrp: false,
        amount_positive: false,
        mpt_amount_exceeds_max: false,
    });

    assert_eq!(self_holder, Ter::TEM_BAD_AMOUNT);
    assert_eq!(xrp, Ter::TEM_BAD_AMOUNT);
    assert_eq!(non_positive, Ter::TEM_BAD_AMOUNT);
}

#[test]
fn clawback_mpt_preflight_honors_feature_holder_and_amount_guards() {
    let disabled = run_clawback_preflight(ClawbackPreflightFacts {
        asset_kind: ClawbackAssetKind::Mpt,
        holder_field_present: true,
        mptokens_v1_enabled: false,
        issuer_equals_holder: false,
        amount_is_xrp: false,
        amount_positive: true,
        mpt_amount_exceeds_max: false,
    });
    let missing_holder = run_clawback_preflight(ClawbackPreflightFacts {
        asset_kind: ClawbackAssetKind::Mpt,
        holder_field_present: false,
        mptokens_v1_enabled: true,
        issuer_equals_holder: false,
        amount_is_xrp: false,
        amount_positive: true,
        mpt_amount_exceeds_max: false,
    });
    let self_holder = run_clawback_preflight(ClawbackPreflightFacts {
        asset_kind: ClawbackAssetKind::Mpt,
        holder_field_present: true,
        mptokens_v1_enabled: true,
        issuer_equals_holder: true,
        amount_is_xrp: false,
        amount_positive: true,
        mpt_amount_exceeds_max: false,
    });
    let too_large = run_clawback_preflight(ClawbackPreflightFacts {
        asset_kind: ClawbackAssetKind::Mpt,
        holder_field_present: true,
        mptokens_v1_enabled: true,
        issuer_equals_holder: false,
        amount_is_xrp: false,
        amount_positive: true,
        mpt_amount_exceeds_max: true,
    });

    assert_eq!(disabled, Ter::TEM_DISABLED);
    assert_eq!(missing_holder, Ter::TEM_MALFORMED);
    assert_eq!(self_holder, Ter::TEM_MALFORMED);
    assert_eq!(too_large, Ter::TEM_BAD_AMOUNT);
}

#[test]
fn clawback_preclaim_checks_account_and_holder_type_guards_in() {
    let missing_account = run_clawback_preclaim(ClawbackPreclaimFacts {
        issuer_exists: false,
        holder_exists: true,
        single_asset_vault_enabled: true,
        holder_is_pseudo_account: true,
        holder_is_amm_account: true,
        asset: ClawbackPreclaimAssetFacts::Issue(ClawbackIssuePreclaimFacts {
            allow_trustline_clawback: true,
            issuer_no_freeze: false,
            ripple_state_exists: true,
            trustline_balance_sign: ClawbackTrustlineBalanceSign::Positive,
            issuer_holder_ordering: Ordering::Greater,
            account_holds_positive: true,
        }),
    });
    let pseudo = run_clawback_preclaim(ClawbackPreclaimFacts {
        issuer_exists: true,
        holder_exists: true,
        single_asset_vault_enabled: true,
        holder_is_pseudo_account: true,
        holder_is_amm_account: true,
        asset: ClawbackPreclaimAssetFacts::Issue(ClawbackIssuePreclaimFacts {
            allow_trustline_clawback: true,
            issuer_no_freeze: false,
            ripple_state_exists: true,
            trustline_balance_sign: ClawbackTrustlineBalanceSign::Positive,
            issuer_holder_ordering: Ordering::Greater,
            account_holds_positive: true,
        }),
    });
    let amm = run_clawback_preclaim(ClawbackPreclaimFacts {
        issuer_exists: true,
        holder_exists: true,
        single_asset_vault_enabled: false,
        holder_is_pseudo_account: true,
        holder_is_amm_account: true,
        asset: ClawbackPreclaimAssetFacts::Issue(ClawbackIssuePreclaimFacts {
            allow_trustline_clawback: true,
            issuer_no_freeze: false,
            ripple_state_exists: true,
            trustline_balance_sign: ClawbackTrustlineBalanceSign::Positive,
            issuer_holder_ordering: Ordering::Greater,
            account_holds_positive: true,
        }),
    });

    assert_eq!(missing_account, Ter::TER_NO_ACCOUNT);
    assert_eq!(pseudo, Ter::TEC_PSEUDO_ACCOUNT);
    assert_eq!(amm, Ter::TEC_AMM_ACCOUNT);
}

#[test]
fn clawback_issue_preclaim_matches_permission_line_ordering_and_funds_guards() {
    let no_permission = run_clawback_preclaim(ClawbackPreclaimFacts {
        issuer_exists: true,
        holder_exists: true,
        single_asset_vault_enabled: false,
        holder_is_pseudo_account: false,
        holder_is_amm_account: false,
        asset: ClawbackPreclaimAssetFacts::Issue(ClawbackIssuePreclaimFacts {
            allow_trustline_clawback: false,
            issuer_no_freeze: false,
            ripple_state_exists: true,
            trustline_balance_sign: ClawbackTrustlineBalanceSign::Positive,
            issuer_holder_ordering: Ordering::Greater,
            account_holds_positive: true,
        }),
    });
    let no_line = run_clawback_preclaim(ClawbackPreclaimFacts {
        issuer_exists: true,
        holder_exists: true,
        single_asset_vault_enabled: false,
        holder_is_pseudo_account: false,
        holder_is_amm_account: false,
        asset: ClawbackPreclaimAssetFacts::Issue(ClawbackIssuePreclaimFacts {
            allow_trustline_clawback: true,
            issuer_no_freeze: false,
            ripple_state_exists: false,
            trustline_balance_sign: ClawbackTrustlineBalanceSign::Positive,
            issuer_holder_ordering: Ordering::Greater,
            account_holds_positive: true,
        }),
    });
    let positive_wrong_side = run_clawback_preclaim(ClawbackPreclaimFacts {
        issuer_exists: true,
        holder_exists: true,
        single_asset_vault_enabled: false,
        holder_is_pseudo_account: false,
        holder_is_amm_account: false,
        asset: ClawbackPreclaimAssetFacts::Issue(ClawbackIssuePreclaimFacts {
            allow_trustline_clawback: true,
            issuer_no_freeze: false,
            ripple_state_exists: true,
            trustline_balance_sign: ClawbackTrustlineBalanceSign::Positive,
            issuer_holder_ordering: Ordering::Less,
            account_holds_positive: true,
        }),
    });
    let negative_wrong_side = run_clawback_preclaim(ClawbackPreclaimFacts {
        issuer_exists: true,
        holder_exists: true,
        single_asset_vault_enabled: false,
        holder_is_pseudo_account: false,
        holder_is_amm_account: false,
        asset: ClawbackPreclaimAssetFacts::Issue(ClawbackIssuePreclaimFacts {
            allow_trustline_clawback: true,
            issuer_no_freeze: false,
            ripple_state_exists: true,
            trustline_balance_sign: ClawbackTrustlineBalanceSign::Negative,
            issuer_holder_ordering: Ordering::Greater,
            account_holds_positive: true,
        }),
    });
    let insufficient = run_clawback_preclaim(ClawbackPreclaimFacts {
        issuer_exists: true,
        holder_exists: true,
        single_asset_vault_enabled: false,
        holder_is_pseudo_account: false,
        holder_is_amm_account: false,
        asset: ClawbackPreclaimAssetFacts::Issue(ClawbackIssuePreclaimFacts {
            allow_trustline_clawback: true,
            issuer_no_freeze: false,
            ripple_state_exists: true,
            trustline_balance_sign: ClawbackTrustlineBalanceSign::Zero,
            issuer_holder_ordering: Ordering::Equal,
            account_holds_positive: false,
        }),
    });
    let success = run_clawback_preclaim(ClawbackPreclaimFacts {
        issuer_exists: true,
        holder_exists: true,
        single_asset_vault_enabled: false,
        holder_is_pseudo_account: false,
        holder_is_amm_account: false,
        asset: ClawbackPreclaimAssetFacts::Issue(ClawbackIssuePreclaimFacts {
            allow_trustline_clawback: true,
            issuer_no_freeze: false,
            ripple_state_exists: true,
            trustline_balance_sign: ClawbackTrustlineBalanceSign::Positive,
            issuer_holder_ordering: Ordering::Greater,
            account_holds_positive: true,
        }),
    });

    assert_eq!(no_permission, Ter::TEC_NO_PERMISSION);
    assert_eq!(no_line, Ter::TEC_NO_LINE);
    assert_eq!(positive_wrong_side, Ter::TEC_NO_PERMISSION);
    assert_eq!(negative_wrong_side, Ter::TEC_NO_PERMISSION);
    assert_eq!(insufficient, Ter::TEC_INSUFFICIENT_FUNDS);
    assert_eq!(success, Ter::TES_SUCCESS);
}

#[test]
fn clawback_mpt_preclaim_object_permission_and_funds_guards() {
    let missing_issuance = run_clawback_preclaim(ClawbackPreclaimFacts {
        issuer_exists: true,
        holder_exists: true,
        single_asset_vault_enabled: false,
        holder_is_pseudo_account: false,
        holder_is_amm_account: false,
        asset: ClawbackPreclaimAssetFacts::Mpt(ClawbackMptPreclaimFacts {
            issuance_exists: false,
            issuance_can_clawback: true,
            issuance_issuer_matches: true,
            holder_token_exists: true,
            account_holds_positive: true,
        }),
    });
    let no_permission = run_clawback_preclaim(ClawbackPreclaimFacts {
        issuer_exists: true,
        holder_exists: true,
        single_asset_vault_enabled: false,
        holder_is_pseudo_account: false,
        holder_is_amm_account: false,
        asset: ClawbackPreclaimAssetFacts::Mpt(ClawbackMptPreclaimFacts {
            issuance_exists: true,
            issuance_can_clawback: false,
            issuance_issuer_matches: true,
            holder_token_exists: true,
            account_holds_positive: true,
        }),
    });
    let missing_token = run_clawback_preclaim(ClawbackPreclaimFacts {
        issuer_exists: true,
        holder_exists: true,
        single_asset_vault_enabled: false,
        holder_is_pseudo_account: false,
        holder_is_amm_account: false,
        asset: ClawbackPreclaimAssetFacts::Mpt(ClawbackMptPreclaimFacts {
            issuance_exists: true,
            issuance_can_clawback: true,
            issuance_issuer_matches: true,
            holder_token_exists: false,
            account_holds_positive: true,
        }),
    });
    let insufficient = run_clawback_preclaim(ClawbackPreclaimFacts {
        issuer_exists: true,
        holder_exists: true,
        single_asset_vault_enabled: false,
        holder_is_pseudo_account: false,
        holder_is_amm_account: false,
        asset: ClawbackPreclaimAssetFacts::Mpt(ClawbackMptPreclaimFacts {
            issuance_exists: true,
            issuance_can_clawback: true,
            issuance_issuer_matches: true,
            holder_token_exists: true,
            account_holds_positive: false,
        }),
    });
    let success = run_clawback_preclaim(ClawbackPreclaimFacts {
        issuer_exists: true,
        holder_exists: true,
        single_asset_vault_enabled: false,
        holder_is_pseudo_account: false,
        holder_is_amm_account: false,
        asset: ClawbackPreclaimAssetFacts::Mpt(ClawbackMptPreclaimFacts {
            issuance_exists: true,
            issuance_can_clawback: true,
            issuance_issuer_matches: true,
            holder_token_exists: true,
            account_holds_positive: true,
        }),
    });

    assert_eq!(missing_issuance, Ter::TEC_OBJECT_NOT_FOUND);
    assert_eq!(no_permission, Ter::TEC_NO_PERMISSION);
    assert_eq!(missing_token, Ter::TEC_OBJECT_NOT_FOUND);
    assert_eq!(insufficient, Ter::TEC_INSUFFICIENT_FUNDS);
    assert_eq!(success, Ter::TES_SUCCESS);
}
