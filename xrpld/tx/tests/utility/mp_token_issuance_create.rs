//! Integration tests that pin the narrowed Rust `MPTokenIssuanceCreate.cpp`
//! shell to the current C++ behavior.

use protocol::{
    Ter, tfMPTCanTransfer, tfMPTRequireAuth, tfMPTokenIssuanceCreateMask, tfUniversal,
    tmfMPTCanMutateCanLock, tmfMPTokenIssuanceCreateMutableMask, trans_token,
};
use tx::utility::mp_token_issuance_create::{
    MAX_MPTOKEN_AMOUNT, MAX_MPTOKEN_METADATA_LENGTH, MAX_TRANSFER_FEE,
};
use tx::{
    MPTokenIssuanceCreateApplyFacts, MPTokenIssuanceCreateApplySink, MPTokenIssuanceCreateMutation,
    MPTokenIssuanceCreatePreflightFacts, get_mp_token_issuance_create_flags_mask,
    mp_token_issuance_create_check_extra_features, run_mp_token_issuance_create_do_apply,
    run_mp_token_issuance_create_preflight,
};

#[derive(Debug, Clone, PartialEq, Eq)]
struct TestSink {
    account_exists: bool,
    reserve_sufficient: bool,
    owner_dir_page: Option<u64>,
    created: Vec<MPTokenIssuanceCreateMutation<&'static str, Vec<u8>, &'static str>>,
    owner_count_deltas: Vec<i32>,
    events: Vec<String>,
}

impl TestSink {
    fn new() -> Self {
        Self {
            account_exists: true,
            reserve_sufficient: true,
            owner_dir_page: Some(33),
            created: Vec::new(),
            owner_count_deltas: Vec::new(),
            events: Vec::new(),
        }
    }
}

impl MPTokenIssuanceCreateApplySink<&'static str, Vec<u8>, &'static str> for TestSink {
    fn account_exists(&mut self) -> bool {
        self.events.push("account_exists".to_string());
        self.account_exists
    }

    fn reserve_sufficient(&mut self) -> bool {
        self.events.push("reserve".to_string());
        self.reserve_sufficient
    }

    fn insert_owner_dir(&mut self) -> Option<u64> {
        self.events.push("owner_dir".to_string());
        self.owner_dir_page
    }

    fn create_issuance(
        &mut self,
        mutation: MPTokenIssuanceCreateMutation<&'static str, Vec<u8>, &'static str>,
    ) {
        self.events.push("create".to_string());
        self.created.push(mutation);
    }

    fn adjust_owner_count(&mut self, delta: i32) {
        self.events.push(format!("owner_count:{delta}"));
        self.owner_count_deltas.push(delta);
    }
}

#[test]
fn mp_token_issuance_create_feature_gate() {
    assert!(!mp_token_issuance_create_check_extra_features(
        true, false, true, false, true
    ));
    assert!(!mp_token_issuance_create_check_extra_features(
        false, true, true, true, false
    ));
    assert!(mp_token_issuance_create_check_extra_features(
        true, true, true, true, true
    ));
}

#[test]
fn mp_token_issuance_create_flags_mask() {
    assert_eq!(
        get_mp_token_issuance_create_flags_mask(),
        tfMPTokenIssuanceCreateMask
    );
}

#[test]
fn mp_token_issuance_create_preflight_guards() {
    let user_reference_holding =
        run_mp_token_issuance_create_preflight(MPTokenIssuanceCreatePreflightFacts {
            fix_cleanup_3_2_0_enabled: true,
            confidential_transfer_enabled: false,
            reference_holding_present: true,
            mutable_flags: None,
            tx_flags: 0,
            transfer_fee: None,
            domain_id_present: false,
            domain_id_is_zero: false,
            metadata_len: None,
            maximum_amount: None,
        });
    let invalid_mutable =
        run_mp_token_issuance_create_preflight(MPTokenIssuanceCreatePreflightFacts {
            fix_cleanup_3_2_0_enabled: false,
            confidential_transfer_enabled: false,
            reference_holding_present: false,
            mutable_flags: Some(0),
            tx_flags: 0,
            transfer_fee: None,
            domain_id_present: false,
            domain_id_is_zero: false,
            metadata_len: None,
            maximum_amount: None,
        });
    let invalid_mutable_mask =
        run_mp_token_issuance_create_preflight(MPTokenIssuanceCreatePreflightFacts {
            fix_cleanup_3_2_0_enabled: false,
            confidential_transfer_enabled: false,
            reference_holding_present: false,
            mutable_flags: Some(tmfMPTokenIssuanceCreateMutableMask),
            tx_flags: 0,
            transfer_fee: None,
            domain_id_present: false,
            domain_id_is_zero: false,
            metadata_len: None,
            maximum_amount: None,
        });
    let bad_fee = run_mp_token_issuance_create_preflight(MPTokenIssuanceCreatePreflightFacts {
        fix_cleanup_3_2_0_enabled: false,
        confidential_transfer_enabled: false,
        reference_holding_present: false,
        mutable_flags: None,
        tx_flags: tfMPTCanTransfer,
        transfer_fee: Some(MAX_TRANSFER_FEE + 1),
        domain_id_present: false,
        domain_id_is_zero: false,
        metadata_len: None,
        maximum_amount: None,
    });
    let missing_transfer_flag =
        run_mp_token_issuance_create_preflight(MPTokenIssuanceCreatePreflightFacts {
            fix_cleanup_3_2_0_enabled: false,
            confidential_transfer_enabled: false,
            reference_holding_present: false,
            mutable_flags: None,
            tx_flags: 0,
            transfer_fee: Some(1),
            domain_id_present: false,
            domain_id_is_zero: false,
            metadata_len: None,
            maximum_amount: None,
        });
    let zero_domain = run_mp_token_issuance_create_preflight(MPTokenIssuanceCreatePreflightFacts {
        fix_cleanup_3_2_0_enabled: false,
        confidential_transfer_enabled: false,
        reference_holding_present: false,
        mutable_flags: None,
        tx_flags: tfMPTRequireAuth,
        transfer_fee: None,
        domain_id_present: true,
        domain_id_is_zero: true,
        metadata_len: None,
        maximum_amount: None,
    });
    let missing_require_auth =
        run_mp_token_issuance_create_preflight(MPTokenIssuanceCreatePreflightFacts {
            fix_cleanup_3_2_0_enabled: false,
            confidential_transfer_enabled: false,
            reference_holding_present: false,
            mutable_flags: None,
            tx_flags: 0,
            transfer_fee: None,
            domain_id_present: true,
            domain_id_is_zero: false,
            metadata_len: None,
            maximum_amount: None,
        });
    let empty_metadata =
        run_mp_token_issuance_create_preflight(MPTokenIssuanceCreatePreflightFacts {
            fix_cleanup_3_2_0_enabled: false,
            confidential_transfer_enabled: false,
            reference_holding_present: false,
            mutable_flags: None,
            tx_flags: 0,
            transfer_fee: None,
            domain_id_present: false,
            domain_id_is_zero: false,
            metadata_len: Some(0),
            maximum_amount: None,
        });
    let oversized_metadata =
        run_mp_token_issuance_create_preflight(MPTokenIssuanceCreatePreflightFacts {
            fix_cleanup_3_2_0_enabled: false,
            confidential_transfer_enabled: false,
            reference_holding_present: false,
            mutable_flags: None,
            tx_flags: 0,
            transfer_fee: None,
            domain_id_present: false,
            domain_id_is_zero: false,
            metadata_len: Some(MAX_MPTOKEN_METADATA_LENGTH + 1),
            maximum_amount: None,
        });
    let zero_max = run_mp_token_issuance_create_preflight(MPTokenIssuanceCreatePreflightFacts {
        fix_cleanup_3_2_0_enabled: false,
        confidential_transfer_enabled: false,
        reference_holding_present: false,
        mutable_flags: None,
        tx_flags: 0,
        transfer_fee: None,
        domain_id_present: false,
        domain_id_is_zero: false,
        metadata_len: None,
        maximum_amount: Some(0),
    });
    let oversized_max =
        run_mp_token_issuance_create_preflight(MPTokenIssuanceCreatePreflightFacts {
            fix_cleanup_3_2_0_enabled: false,
            confidential_transfer_enabled: false,
            reference_holding_present: false,
            mutable_flags: None,
            tx_flags: 0,
            transfer_fee: None,
            domain_id_present: false,
            domain_id_is_zero: false,
            metadata_len: None,
            maximum_amount: Some(MAX_MPTOKEN_AMOUNT + 1),
        });

    assert_eq!(user_reference_holding, Ter::TEM_MALFORMED);
    assert_eq!(invalid_mutable, Ter::TEM_INVALID_FLAG);
    assert_eq!(invalid_mutable_mask, Ter::TEM_INVALID_FLAG);
    assert_eq!(bad_fee, Ter::TEM_BAD_TRANSFER_FEE);
    assert_eq!(missing_transfer_flag, Ter::TEM_MALFORMED);
    assert_eq!(zero_domain, Ter::TEM_MALFORMED);
    assert_eq!(missing_require_auth, Ter::TEM_MALFORMED);
    assert_eq!(empty_metadata, Ter::TEM_MALFORMED);
    assert_eq!(oversized_metadata, Ter::TEM_MALFORMED);
    assert_eq!(zero_max, Ter::TEM_MALFORMED);
    assert_eq!(oversized_max, Ter::TEM_MALFORMED);
}

#[test]
fn mp_token_issuance_create_preflight_accepts_valid_inputs() {
    let result = run_mp_token_issuance_create_preflight(MPTokenIssuanceCreatePreflightFacts {
        fix_cleanup_3_2_0_enabled: true,
        confidential_transfer_enabled: false,
        reference_holding_present: false,
        mutable_flags: Some(tmfMPTCanMutateCanLock),
        tx_flags: tfMPTCanTransfer | tfMPTRequireAuth,
        transfer_fee: Some(MAX_TRANSFER_FEE),
        domain_id_present: true,
        domain_id_is_zero: false,
        metadata_len: Some(MAX_MPTOKEN_METADATA_LENGTH),
        maximum_amount: Some(MAX_MPTOKEN_AMOUNT),
    });

    assert_eq!(result, Ter::TES_SUCCESS);
}

#[test]
fn mp_token_issuance_create_do_apply_preserves_and_masks_universal_flags() {
    let mut sink = TestSink::new();

    let result = run_mp_token_issuance_create_do_apply(
        MPTokenIssuanceCreateApplyFacts {
            account: "issuer",
            flags: tfUniversal | tfMPTCanTransfer | tfMPTRequireAuth,
            sequence: 7,
            maximum_amount: Some(11),
            asset_scale: Some(9),
            transfer_fee: Some(10),
            metadata: Some(vec![1, 2]),
            domain_id: Some("domain"),
            mutable_flags: Some(tmfMPTCanMutateCanLock),
        },
        &mut sink,
    );

    assert_eq!(result, Ter::TES_SUCCESS);
    assert_eq!(
        sink.events,
        [
            "account_exists",
            "reserve",
            "owner_dir",
            "create",
            "owner_count:1"
        ]
    );
    assert_eq!(sink.owner_count_deltas, vec![1]);
    assert_eq!(sink.created.len(), 1);
    assert_eq!(sink.created[0].flags, tfMPTCanTransfer | tfMPTRequireAuth);
    assert_eq!(sink.created[0].owner_node, 33);
    assert_eq!(sink.created[0].outstanding_amount, 0);
}

#[test]
fn mp_token_issuance_create_do_apply_maps_cpp_failures() {
    let facts = MPTokenIssuanceCreateApplyFacts {
        account: "issuer",
        flags: 0,
        sequence: 1,
        maximum_amount: None,
        asset_scale: None,
        transfer_fee: None,
        metadata: None::<Vec<u8>>,
        domain_id: None::<&'static str>,
        mutable_flags: None,
    };

    let mut missing = TestSink::new();
    missing.account_exists = false;
    assert_eq!(
        run_mp_token_issuance_create_do_apply(facts.clone(), &mut missing),
        Ter::TEC_INTERNAL
    );

    let mut reserve = TestSink::new();
    reserve.reserve_sufficient = false;
    let reserve_result = run_mp_token_issuance_create_do_apply(facts.clone(), &mut reserve);
    assert_eq!(reserve_result, Ter::TEC_INSUFFICIENT_RESERVE);
    assert_eq!(trans_token(reserve_result), "tecINSUFFICIENT_RESERVE");

    let mut dir_full = TestSink::new();
    dir_full.owner_dir_page = None;
    assert_eq!(
        run_mp_token_issuance_create_do_apply(facts, &mut dir_full),
        Ter::TEC_DIR_FULL
    );
}
