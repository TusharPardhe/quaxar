//! Integration tests that pin the narrowed Rust `MPTokenAuthorize.cpp` shell
//! to the current C++ behavior.

use protocol::{Ter, tfMPTUnauthorize, tfMPTUnauthorizeMask, trans_token};
use tx::{
    MPTokenAuthorizeApplySink, MPTokenAuthorizeCreateMutation, MPTokenAuthorizeCreateSink,
    MPTokenAuthorizePreclaimFacts, MPTokenAuthorizePreflightFacts,
    get_mp_token_authorize_flags_mask, run_mp_token_authorize_create_mptoken,
    run_mp_token_authorize_do_apply, run_mp_token_authorize_preclaim,
    run_mp_token_authorize_preflight,
};

#[derive(Debug, Clone, PartialEq, Eq)]
struct TestCreateSink {
    owner_dir_page: Option<u64>,
    inserted: Vec<MPTokenAuthorizeCreateMutation<&'static str, &'static str>>,
}

impl MPTokenAuthorizeCreateSink<&'static str, &'static str> for TestCreateSink {
    fn insert_owner_dir(&mut self) -> Option<u64> {
        self.owner_dir_page
    }

    fn insert_mptoken(
        &mut self,
        mutation: MPTokenAuthorizeCreateMutation<&'static str, &'static str>,
    ) {
        self.inserted.push(mutation);
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct TestApplySink {
    result: Ter,
    calls: Vec<(&'static str, &'static str, u32, Option<&'static str>)>,
}

impl MPTokenAuthorizeApplySink<&'static str, &'static str> for TestApplySink {
    fn authorize_mptoken(
        &mut self,
        mpt_issuance_id: &'static str,
        account: &'static str,
        tx_flags: u32,
        holder: Option<&'static str>,
    ) -> Ter {
        self.calls
            .push((mpt_issuance_id, account, tx_flags, holder));
        self.result
    }
}

#[test]
fn mp_token_authorize_mask_and_preflight_match_cpp() {
    assert_eq!(get_mp_token_authorize_flags_mask(), tfMPTUnauthorizeMask);
    assert_eq!(
        run_mp_token_authorize_preflight(MPTokenAuthorizePreflightFacts {
            account_equals_holder: true,
        }),
        Ter::TEM_MALFORMED
    );
    assert_eq!(
        run_mp_token_authorize_preflight(MPTokenAuthorizePreflightFacts {
            account_equals_holder: false,
        }),
        Ter::TES_SUCCESS
    );
}

#[test]
fn mp_token_authorize_preclaim_matches_holder_flow_guards() {
    let missing = run_mp_token_authorize_preclaim(MPTokenAuthorizePreclaimFacts {
        holder_present: false,
        account_token_exists: false,
        tx_flags: tfMPTUnauthorize,
        token_balance_is_zero: true,
        token_locked_amount_is_zero: true,
        issuance_exists: true,
        single_asset_vault_enabled: false,
        token_locked: false,
        account_is_issuer: false,
        holder_account_exists: true,
        issuance_requires_auth: true,
        holder_token_exists: true,
        holder_is_pseudo_account: false,
    });
    let obligations = run_mp_token_authorize_preclaim(MPTokenAuthorizePreclaimFacts {
        holder_present: false,
        account_token_exists: true,
        tx_flags: tfMPTUnauthorize,
        token_balance_is_zero: false,
        token_locked_amount_is_zero: true,
        issuance_exists: true,
        single_asset_vault_enabled: false,
        token_locked: false,
        account_is_issuer: false,
        holder_account_exists: true,
        issuance_requires_auth: true,
        holder_token_exists: true,
        holder_is_pseudo_account: false,
    });
    let locked = run_mp_token_authorize_preclaim(MPTokenAuthorizePreclaimFacts {
        holder_present: false,
        account_token_exists: true,
        tx_flags: tfMPTUnauthorize,
        token_balance_is_zero: true,
        token_locked_amount_is_zero: true,
        issuance_exists: true,
        single_asset_vault_enabled: true,
        token_locked: true,
        account_is_issuer: false,
        holder_account_exists: true,
        issuance_requires_auth: true,
        holder_token_exists: true,
        holder_is_pseudo_account: false,
    });
    let duplicate = run_mp_token_authorize_preclaim(MPTokenAuthorizePreclaimFacts {
        holder_present: false,
        account_token_exists: true,
        tx_flags: 0,
        token_balance_is_zero: true,
        token_locked_amount_is_zero: true,
        issuance_exists: true,
        single_asset_vault_enabled: false,
        token_locked: false,
        account_is_issuer: false,
        holder_account_exists: true,
        issuance_requires_auth: true,
        holder_token_exists: true,
        holder_is_pseudo_account: false,
    });

    assert_eq!(missing, Ter::TEC_OBJECT_NOT_FOUND);
    assert_eq!(obligations, Ter::TEC_HAS_OBLIGATIONS);
    assert_eq!(locked, Ter::TEC_NO_PERMISSION);
    assert_eq!(duplicate, Ter::TEC_DUPLICATE);
}

#[test]
fn mp_token_authorize_preclaim_matches_issuer_flow_guards() {
    let no_dst = run_mp_token_authorize_preclaim(MPTokenAuthorizePreclaimFacts {
        holder_present: true,
        account_token_exists: false,
        tx_flags: 0,
        token_balance_is_zero: true,
        token_locked_amount_is_zero: true,
        issuance_exists: true,
        single_asset_vault_enabled: false,
        token_locked: false,
        account_is_issuer: true,
        holder_account_exists: false,
        issuance_requires_auth: true,
        holder_token_exists: true,
        holder_is_pseudo_account: false,
    });
    let no_auth = run_mp_token_authorize_preclaim(MPTokenAuthorizePreclaimFacts {
        holder_present: true,
        account_token_exists: false,
        tx_flags: 0,
        token_balance_is_zero: true,
        token_locked_amount_is_zero: true,
        issuance_exists: true,
        single_asset_vault_enabled: false,
        token_locked: false,
        account_is_issuer: true,
        holder_account_exists: true,
        issuance_requires_auth: false,
        holder_token_exists: true,
        holder_is_pseudo_account: false,
    });
    let pseudo = run_mp_token_authorize_preclaim(MPTokenAuthorizePreclaimFacts {
        holder_present: true,
        account_token_exists: false,
        tx_flags: 0,
        token_balance_is_zero: true,
        token_locked_amount_is_zero: true,
        issuance_exists: true,
        single_asset_vault_enabled: false,
        token_locked: false,
        account_is_issuer: true,
        holder_account_exists: true,
        issuance_requires_auth: true,
        holder_token_exists: true,
        holder_is_pseudo_account: true,
    });

    assert_eq!(no_dst, Ter::TEC_NO_DST);
    assert_eq!(no_auth, Ter::TEC_NO_AUTH);
    assert_eq!(pseudo, Ter::TEC_NO_PERMISSION);
}

#[test]
fn mp_token_authorize_create_and_do_apply_match_cpp() {
    let mut create_sink = TestCreateSink {
        owner_dir_page: Some(17),
        inserted: Vec::new(),
    };

    let create_result = run_mp_token_authorize_create_mptoken("mpt", "alice", 9, &mut create_sink);

    assert_eq!(create_result, Ter::TES_SUCCESS);
    assert_eq!(
        create_sink.inserted,
        vec![MPTokenAuthorizeCreateMutation {
            account: "alice",
            mpt_issuance_id: "mpt",
            flags: 9,
            owner_node: 17,
        }]
    );

    let mut dir_full_sink = TestCreateSink {
        owner_dir_page: None,
        inserted: Vec::new(),
    };
    let dir_full = run_mp_token_authorize_create_mptoken("mpt", "alice", 9, &mut dir_full_sink);
    assert_eq!(dir_full, Ter::TEC_DIR_FULL);
    assert_eq!(trans_token(dir_full), "tecDIR_FULL");

    let mut apply_sink = TestApplySink {
        result: Ter::TES_SUCCESS,
        calls: Vec::new(),
    };
    let do_apply = run_mp_token_authorize_do_apply(
        "mpt",
        "issuer",
        tfMPTUnauthorize,
        Some("holder"),
        &mut apply_sink,
    );

    assert_eq!(do_apply, Ter::TES_SUCCESS);
    assert_eq!(
        apply_sink.calls,
        vec![("mpt", "issuer", tfMPTUnauthorize, Some("holder"))]
    );
}
