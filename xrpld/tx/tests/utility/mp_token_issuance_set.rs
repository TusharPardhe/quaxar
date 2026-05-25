//! Integration tests that pin the narrowed Rust `MPTokenIssuanceSet.cpp`
//! shell to the current C++ behavior.

use std::collections::BTreeSet;

use protocol::{
    Ter, lsfMPTCanLock, lsfMPTCanTransfer, lsfMPTLocked, lsfMPTRequireAuth,
    lsmfMPTCanMutateMetadata, lsmfMPTCanMutateTransferFee, tfMPTLock, tfMPTUnlock,
    tfMPTokenIssuanceSetMask, tfUniversalMask, tmfMPTClearCanTransfer, tmfMPTSetCanLock,
    tmfMPTokenIssuanceSetMutableMask, trans_token,
};
use tx::utility::mp_token_issuance_set::{MAX_MPTOKEN_METADATA_LENGTH, MAX_TRANSFER_FEE};
use tx::{
    MPTokenIssuanceSetApplyFacts, MPTokenIssuanceSetApplySink, MPTokenIssuanceSetDomainUpdate,
    MPTokenIssuanceSetGranularPermission, MPTokenIssuanceSetPermissionFacts,
    MPTokenIssuanceSetPreclaimFacts, MPTokenIssuanceSetPreflightFacts,
    get_mp_token_issuance_set_flags_mask, mp_token_issuance_set_check_extra_features,
    run_mp_token_issuance_set_check_permission, run_mp_token_issuance_set_do_apply,
    run_mp_token_issuance_set_preclaim, run_mp_token_issuance_set_preflight,
};

#[derive(Debug, Clone, PartialEq, Eq)]
struct TestSink {
    target_exists: bool,
    current_flags: u32,
    flags_set: Vec<u32>,
    transfer_fee_clears: usize,
    transfer_fees: Vec<u16>,
    metadata_clears: usize,
    metadatas: Vec<Vec<u8>>,
    domain_clears: usize,
    domains: Vec<&'static str>,
    finished: usize,
    events: Vec<String>,
}

impl TestSink {
    fn new() -> Self {
        Self {
            target_exists: true,
            current_flags: 0,
            flags_set: Vec::new(),
            transfer_fee_clears: 0,
            transfer_fees: Vec::new(),
            metadata_clears: 0,
            metadatas: Vec::new(),
            domain_clears: 0,
            domains: Vec::new(),
            finished: 0,
            events: Vec::new(),
        }
    }
}

impl MPTokenIssuanceSetApplySink<&'static str> for TestSink {
    fn target_exists(&mut self) -> bool {
        self.events.push("target_exists".to_string());
        self.target_exists
    }

    fn current_flags(&mut self) -> u32 {
        self.events.push("current_flags".to_string());
        self.current_flags
    }

    fn set_flags(&mut self, flags: u32) {
        self.events.push(format!("set_flags:{flags:#x}"));
        self.flags_set.push(flags);
    }

    fn clear_transfer_fee(&mut self) {
        self.events.push("clear_transfer_fee".to_string());
        self.transfer_fee_clears += 1;
    }

    fn set_transfer_fee(&mut self, transfer_fee: u16) {
        self.events.push(format!("set_transfer_fee:{transfer_fee}"));
        self.transfer_fees.push(transfer_fee);
    }

    fn clear_metadata(&mut self) {
        self.events.push("clear_metadata".to_string());
        self.metadata_clears += 1;
    }

    fn set_metadata(&mut self, metadata: Vec<u8>) {
        self.events.push("set_metadata".to_string());
        self.metadatas.push(metadata);
    }

    fn clear_domain(&mut self) {
        self.events.push("clear_domain".to_string());
        self.domain_clears += 1;
    }

    fn set_domain(&mut self, domain: &'static str) {
        self.events.push(format!("set_domain:{domain}"));
        self.domains.push(domain);
    }

    fn finish_update(&mut self) {
        self.events.push("finish".to_string());
        self.finished += 1;
    }
}

#[test]
fn mp_token_issuance_set_feature_gate_and_mask_match_cpp() {
    assert!(!mp_token_issuance_set_check_extra_features(
        true, false, true
    ));
    assert!(mp_token_issuance_set_check_extra_features(
        false, false, false
    ));
    assert_eq!(
        get_mp_token_issuance_set_flags_mask(),
        tfMPTokenIssuanceSetMask
    );
}

#[test]
fn mp_token_issuance_set_preflight_guards() {
    let disabled = run_mp_token_issuance_set_preflight(MPTokenIssuanceSetPreflightFacts {
        dynamic_mpt_enabled: false,
        single_asset_vault_enabled: false,
        domain_id_present: false,
        holder_present: false,
        account_equals_holder: false,
        tx_flags: 0,
        mutable_flags: Some(tmfMPTSetCanLock),
        metadata_len: None,
        transfer_fee: None,
    });
    let domain_and_holder = run_mp_token_issuance_set_preflight(MPTokenIssuanceSetPreflightFacts {
        dynamic_mpt_enabled: true,
        single_asset_vault_enabled: true,
        domain_id_present: true,
        holder_present: true,
        account_equals_holder: false,
        tx_flags: 0,
        mutable_flags: None,
        metadata_len: None,
        transfer_fee: None,
    });
    let lock_and_unlock = run_mp_token_issuance_set_preflight(MPTokenIssuanceSetPreflightFacts {
        dynamic_mpt_enabled: true,
        single_asset_vault_enabled: true,
        domain_id_present: false,
        holder_present: false,
        account_equals_holder: false,
        tx_flags: tfMPTLock | tfMPTUnlock,
        mutable_flags: None,
        metadata_len: None,
        transfer_fee: None,
    });
    let self_holder = run_mp_token_issuance_set_preflight(MPTokenIssuanceSetPreflightFacts {
        dynamic_mpt_enabled: true,
        single_asset_vault_enabled: true,
        domain_id_present: false,
        holder_present: true,
        account_equals_holder: true,
        tx_flags: 0,
        mutable_flags: None,
        metadata_len: None,
        transfer_fee: None,
    });
    let no_change = run_mp_token_issuance_set_preflight(MPTokenIssuanceSetPreflightFacts {
        dynamic_mpt_enabled: false,
        single_asset_vault_enabled: true,
        domain_id_present: false,
        holder_present: false,
        account_equals_holder: false,
        tx_flags: 0,
        mutable_flags: None,
        metadata_len: None,
        transfer_fee: None,
    });
    let mutate_with_holder =
        run_mp_token_issuance_set_preflight(MPTokenIssuanceSetPreflightFacts {
            dynamic_mpt_enabled: true,
            single_asset_vault_enabled: true,
            domain_id_present: false,
            holder_present: true,
            account_equals_holder: false,
            tx_flags: 0,
            mutable_flags: Some(tmfMPTSetCanLock),
            metadata_len: None,
            transfer_fee: None,
        });
    let mutate_with_non_universal_flags =
        run_mp_token_issuance_set_preflight(MPTokenIssuanceSetPreflightFacts {
            dynamic_mpt_enabled: true,
            single_asset_vault_enabled: true,
            domain_id_present: false,
            holder_present: false,
            account_equals_holder: false,
            tx_flags: tfUniversalMask,
            mutable_flags: Some(tmfMPTSetCanLock),
            metadata_len: None,
            transfer_fee: None,
        });
    let bad_fee = run_mp_token_issuance_set_preflight(MPTokenIssuanceSetPreflightFacts {
        dynamic_mpt_enabled: true,
        single_asset_vault_enabled: true,
        domain_id_present: false,
        holder_present: false,
        account_equals_holder: false,
        tx_flags: 0,
        mutable_flags: None,
        metadata_len: None,
        transfer_fee: Some(MAX_TRANSFER_FEE + 1),
    });
    let bad_metadata = run_mp_token_issuance_set_preflight(MPTokenIssuanceSetPreflightFacts {
        dynamic_mpt_enabled: true,
        single_asset_vault_enabled: true,
        domain_id_present: false,
        holder_present: false,
        account_equals_holder: false,
        tx_flags: 0,
        mutable_flags: None,
        metadata_len: Some(MAX_MPTOKEN_METADATA_LENGTH + 1),
        transfer_fee: None,
    });
    let invalid_mutable = run_mp_token_issuance_set_preflight(MPTokenIssuanceSetPreflightFacts {
        dynamic_mpt_enabled: true,
        single_asset_vault_enabled: true,
        domain_id_present: false,
        holder_present: false,
        account_equals_holder: false,
        tx_flags: 0,
        mutable_flags: Some(tmfMPTokenIssuanceSetMutableMask),
        metadata_len: None,
        transfer_fee: None,
    });
    let nonzero_fee_clear_transfer =
        run_mp_token_issuance_set_preflight(MPTokenIssuanceSetPreflightFacts {
            dynamic_mpt_enabled: true,
            single_asset_vault_enabled: true,
            domain_id_present: false,
            holder_present: false,
            account_equals_holder: false,
            tx_flags: 0,
            mutable_flags: Some(tmfMPTClearCanTransfer),
            metadata_len: None,
            transfer_fee: Some(1),
        });

    assert_eq!(disabled, Ter::TEM_DISABLED);
    assert_eq!(domain_and_holder, Ter::TEM_MALFORMED);
    assert_eq!(lock_and_unlock, Ter::TEM_INVALID_FLAG);
    assert_eq!(self_holder, Ter::TEM_MALFORMED);
    assert_eq!(no_change, Ter::TEM_MALFORMED);
    assert_eq!(mutate_with_holder, Ter::TEM_MALFORMED);
    assert_eq!(mutate_with_non_universal_flags, Ter::TEM_INVALID_FLAG);
    assert_eq!(bad_fee, Ter::TEM_BAD_TRANSFER_FEE);
    assert_eq!(bad_metadata, Ter::TEM_MALFORMED);
    assert_eq!(invalid_mutable, Ter::TEM_INVALID_FLAG);
    assert_eq!(nonzero_fee_clear_transfer, Ter::TEM_MALFORMED);
}

#[test]
fn mp_token_issuance_set_check_permission() {
    let missing_delegate =
        run_mp_token_issuance_set_check_permission(MPTokenIssuanceSetPermissionFacts {
            delegate_present: true,
            delegate_entry_exists: false,
            broad_permission_granted: false,
            tx_flags: 0,
            granular_permissions: BTreeSet::new(),
        });
    let broad = run_mp_token_issuance_set_check_permission(MPTokenIssuanceSetPermissionFacts {
        delegate_present: true,
        delegate_entry_exists: true,
        broad_permission_granted: true,
        tx_flags: 0,
        granular_permissions: BTreeSet::new(),
    });
    let no_lock_permission =
        run_mp_token_issuance_set_check_permission(MPTokenIssuanceSetPermissionFacts {
            delegate_present: true,
            delegate_entry_exists: true,
            broad_permission_granted: false,
            tx_flags: tfMPTLock,
            granular_permissions: BTreeSet::new(),
        });
    let mut permissions = BTreeSet::new();
    permissions.insert(MPTokenIssuanceSetGranularPermission::Lock);
    let granular = run_mp_token_issuance_set_check_permission(MPTokenIssuanceSetPermissionFacts {
        delegate_present: true,
        delegate_entry_exists: true,
        broad_permission_granted: false,
        tx_flags: tfMPTLock,
        granular_permissions: permissions,
    });

    assert_eq!(missing_delegate, Ter::TER_NO_DELEGATE_PERMISSION);
    assert_eq!(broad, Ter::TES_SUCCESS);
    assert_eq!(no_lock_permission, Ter::TER_NO_DELEGATE_PERMISSION);
    assert_eq!(granular, Ter::TES_SUCCESS);
}

#[test]
fn mp_token_issuance_set_preclaim_ordered_guards() {
    let missing = run_mp_token_issuance_set_preclaim(MPTokenIssuanceSetPreclaimFacts {
        issuance_exists: false,
        issuance_can_lock: true,
        single_asset_vault_enabled: false,
        dynamic_mpt_enabled: false,
        tx_flags: 0,
        issuer_matches: true,
        holder_present: false,
        holder_account_exists: true,
        holder_token_exists: true,
        domain_id_present: false,
        domain_id_is_zero: false,
        issuance_requires_auth: true,
        domain_exists: true,
        current_mutable_flags: 0,
        mutable_flags: None,
        metadata_present: false,
        transfer_fee: None,
        issuance_can_transfer: true,
    });
    let no_lock_permission = run_mp_token_issuance_set_preclaim(MPTokenIssuanceSetPreclaimFacts {
        issuance_exists: true,
        issuance_can_lock: false,
        single_asset_vault_enabled: false,
        dynamic_mpt_enabled: false,
        tx_flags: tfMPTLock,
        issuer_matches: true,
        holder_present: false,
        holder_account_exists: true,
        holder_token_exists: true,
        domain_id_present: false,
        domain_id_is_zero: false,
        issuance_requires_auth: true,
        domain_exists: true,
        current_mutable_flags: 0,
        mutable_flags: None,
        metadata_present: false,
        transfer_fee: None,
        issuance_can_transfer: true,
    });
    let no_dst = run_mp_token_issuance_set_preclaim(MPTokenIssuanceSetPreclaimFacts {
        issuance_exists: true,
        issuance_can_lock: true,
        single_asset_vault_enabled: true,
        dynamic_mpt_enabled: true,
        tx_flags: 0,
        issuer_matches: true,
        holder_present: true,
        holder_account_exists: false,
        holder_token_exists: true,
        domain_id_present: false,
        domain_id_is_zero: false,
        issuance_requires_auth: true,
        domain_exists: true,
        current_mutable_flags: 0,
        mutable_flags: None,
        metadata_present: false,
        transfer_fee: None,
        issuance_can_transfer: true,
    });
    let no_domain_permission =
        run_mp_token_issuance_set_preclaim(MPTokenIssuanceSetPreclaimFacts {
            issuance_exists: true,
            issuance_can_lock: true,
            single_asset_vault_enabled: true,
            dynamic_mpt_enabled: true,
            tx_flags: 0,
            issuer_matches: true,
            holder_present: false,
            holder_account_exists: true,
            holder_token_exists: true,
            domain_id_present: true,
            domain_id_is_zero: false,
            issuance_requires_auth: false,
            domain_exists: true,
            current_mutable_flags: 0,
            mutable_flags: None,
            metadata_present: false,
            transfer_fee: None,
            issuance_can_transfer: true,
        });
    let nonmutable_metadata = run_mp_token_issuance_set_preclaim(MPTokenIssuanceSetPreclaimFacts {
        issuance_exists: true,
        issuance_can_lock: true,
        single_asset_vault_enabled: true,
        dynamic_mpt_enabled: true,
        tx_flags: 0,
        issuer_matches: true,
        holder_present: false,
        holder_account_exists: true,
        holder_token_exists: true,
        domain_id_present: false,
        domain_id_is_zero: false,
        issuance_requires_auth: true,
        domain_exists: true,
        current_mutable_flags: 0,
        mutable_flags: None,
        metadata_present: true,
        transfer_fee: None,
        issuance_can_transfer: true,
    });
    let nonmutable_transfer_fee =
        run_mp_token_issuance_set_preclaim(MPTokenIssuanceSetPreclaimFacts {
            issuance_exists: true,
            issuance_can_lock: true,
            single_asset_vault_enabled: true,
            dynamic_mpt_enabled: true,
            tx_flags: 0,
            issuer_matches: true,
            holder_present: false,
            holder_account_exists: true,
            holder_token_exists: true,
            domain_id_present: false,
            domain_id_is_zero: false,
            issuance_requires_auth: true,
            domain_exists: true,
            current_mutable_flags: lsfMPTCanTransfer,
            mutable_flags: None,
            metadata_present: false,
            transfer_fee: Some(10),
            issuance_can_transfer: true,
        });
    let valid = run_mp_token_issuance_set_preclaim(MPTokenIssuanceSetPreclaimFacts {
        issuance_exists: true,
        issuance_can_lock: true,
        single_asset_vault_enabled: true,
        dynamic_mpt_enabled: true,
        tx_flags: 0,
        issuer_matches: true,
        holder_present: false,
        holder_account_exists: true,
        holder_token_exists: true,
        domain_id_present: true,
        domain_id_is_zero: false,
        issuance_requires_auth: true,
        domain_exists: true,
        current_mutable_flags: lsmfMPTCanMutateMetadata | lsmfMPTCanMutateTransferFee,
        mutable_flags: None,
        metadata_present: true,
        transfer_fee: Some(10),
        issuance_can_transfer: true,
    });

    assert_eq!(missing, Ter::TEC_OBJECT_NOT_FOUND);
    assert_eq!(no_lock_permission, Ter::TEC_NO_PERMISSION);
    assert_eq!(no_dst, Ter::TEC_NO_DST);
    assert_eq!(no_domain_permission, Ter::TEC_NO_PERMISSION);
    assert_eq!(nonmutable_metadata, Ter::TEC_NO_PERMISSION);
    assert_eq!(nonmutable_transfer_fee, Ter::TEC_NO_PERMISSION);
    assert_eq!(valid, Ter::TES_SUCCESS);
}

#[test]
fn mp_token_issuance_set_do_apply_preserves_cpp_mutation_order() {
    let mut sink = TestSink::new();
    sink.current_flags = lsfMPTCanLock | lsfMPTRequireAuth;

    let result = run_mp_token_issuance_set_do_apply(
        MPTokenIssuanceSetApplyFacts {
            tx_flags: tfMPTLock,
            mutable_flags: Some(tmfMPTClearCanTransfer),
            transfer_fee: Some(0),
            metadata: Some(vec![1, 2, 3]),
            domain: MPTokenIssuanceSetDomainUpdate::Set("domain"),
        },
        &mut sink,
    );

    assert_eq!(result, Ter::TES_SUCCESS);
    assert_eq!(
        sink.events,
        [
            "target_exists",
            "current_flags",
            "clear_transfer_fee",
            "set_flags:0x7",
            "clear_transfer_fee",
            "set_metadata",
            "set_domain:domain",
            "finish",
        ]
    );
    assert_eq!(
        sink.flags_set,
        vec![lsfMPTCanLock | lsfMPTRequireAuth | lsfMPTLocked]
    );
    assert_eq!(sink.transfer_fee_clears, 2);
    assert_eq!(sink.metadatas, vec![vec![1, 2, 3]]);
    assert_eq!(sink.domains, vec!["domain"]);
    assert_eq!(sink.finished, 1);
}

#[test]
fn mp_token_issuance_set_do_apply_handles_clear_and_missing_target() {
    let mut missing = TestSink::new();
    missing.target_exists = false;
    assert_eq!(
        run_mp_token_issuance_set_do_apply(
            MPTokenIssuanceSetApplyFacts {
                tx_flags: 0,
                mutable_flags: None,
                transfer_fee: None,
                metadata: None,
                domain: MPTokenIssuanceSetDomainUpdate::NoChange,
            },
            &mut missing,
        ),
        Ter::TEC_INTERNAL
    );

    let mut clear = TestSink::new();
    clear.current_flags = lsfMPTLocked;
    let result = run_mp_token_issuance_set_do_apply(
        MPTokenIssuanceSetApplyFacts {
            tx_flags: tfMPTUnlock,
            mutable_flags: None,
            transfer_fee: Some(5),
            metadata: Some(Vec::new()),
            domain: MPTokenIssuanceSetDomainUpdate::Clear,
        },
        &mut clear,
    );

    assert_eq!(result, Ter::TES_SUCCESS);
    assert_eq!(clear.flags_set, vec![0]);
    assert_eq!(clear.transfer_fees, vec![5]);
    assert_eq!(clear.metadata_clears, 1);
    assert_eq!(clear.domain_clears, 1);
    assert_eq!(trans_token(result), "tesSUCCESS");
}
