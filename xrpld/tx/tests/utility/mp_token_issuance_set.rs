//! Pinned-rippled MPTokenIssuanceSet behavior fixtures.

use protocol::{
    Ter, lsfMPTCanLock, lsfMPTCanTransfer, lsfMPTLocked, lsifMPTCanTransfer, lsifMPTMetadata,
    lsifMPTTransferFee, tfMPTLock, tfMPTSetCanHoldConfidentialBalance, tfMPTSetCanTransfer,
    tfMPTUnlock, tfMPTokenIssuanceSetMask, tifMPTokenIssuanceImmutableMask,
};
use std::collections::BTreeSet;
use tx::utility::mp_token_issuance_set::{MAX_MPTOKEN_METADATA_LENGTH, MAX_TRANSFER_FEE};
use tx::{
    MPTokenIssuanceSetApplyFacts, MPTokenIssuanceSetApplySink, MPTokenIssuanceSetDomainUpdate,
    MPTokenIssuanceSetGranularPermission, MPTokenIssuanceSetPermissionFacts,
    MPTokenIssuanceSetPreclaimFacts, MPTokenIssuanceSetPreflightFacts,
    get_mp_token_issuance_set_flags_mask, mp_token_issuance_set_check_extra_features,
    run_mp_token_issuance_set_check_permission, run_mp_token_issuance_set_do_apply,
    run_mp_token_issuance_set_preclaim, run_mp_token_issuance_set_preflight,
};

#[derive(Default)]
struct Sink {
    exists: bool,
    flags: u32,
    immutable: u32,
    fee: Option<u16>,
    metadata: Option<Vec<u8>>,
    domain: Option<&'static str>,
    finished: usize,
}
impl MPTokenIssuanceSetApplySink<&'static str> for Sink {
    fn target_exists(&mut self) -> bool {
        self.exists
    }
    fn current_flags(&mut self) -> u32 {
        self.flags
    }
    fn set_flags(&mut self, value: u32) {
        self.flags = value;
    }
    fn current_immutable_flags(&mut self) -> u32 {
        self.immutable
    }
    fn set_immutable_flags(&mut self, value: u32) {
        self.immutable = value;
    }
    fn clear_transfer_fee(&mut self) {
        self.fee = None;
    }
    fn set_transfer_fee(&mut self, value: u16) {
        self.fee = Some(value);
    }
    fn clear_metadata(&mut self) {
        self.metadata = None;
    }
    fn set_metadata(&mut self, value: Vec<u8>) {
        self.metadata = Some(value);
    }
    fn clear_domain(&mut self) {
        self.domain = None;
    }
    fn set_domain(&mut self, value: &'static str) {
        self.domain = Some(value);
    }
    fn finish_update(&mut self) {
        self.finished += 1;
    }
}

fn preflight(flags: u32, immutable: Option<u32>) -> MPTokenIssuanceSetPreflightFacts {
    MPTokenIssuanceSetPreflightFacts {
        dynamic_mpt_enabled: true,
        single_asset_vault_enabled: true,
        domain_id_present: false,
        holder_present: false,
        account_equals_holder: false,
        tx_flags: flags,
        mutable_flags: immutable,
        metadata_len: None,
        transfer_fee: None,
    }
}

fn preclaim(flags: u32, immutable: u32) -> MPTokenIssuanceSetPreclaimFacts {
    MPTokenIssuanceSetPreclaimFacts {
        issuance_exists: true,
        issuance_can_lock: true,
        single_asset_vault_enabled: true,
        dynamic_mpt_enabled: true,
        tx_flags: flags,
        issuer_matches: true,
        holder_present: false,
        holder_account_exists: true,
        holder_token_exists: true,
        domain_id_present: false,
        domain_id_is_zero: false,
        issuance_requires_auth: true,
        domain_exists: true,
        issuance_domain_present: false,
        current_mutable_flags: immutable,
        mutable_flags: None,
        metadata_present: false,
        transfer_fee: None,
        issuance_can_transfer: false,
        issuance_has_confidential_balance: false,
        issuance_transfer_fee_nonzero: false,
        issuer_encryption_key_present: false,
        auditor_encryption_key_present: false,
        tx_has_issuer_encryption_key: false,
        tx_has_auditor_encryption_key: false,
        confidential_outstanding_nonzero: false,
    }
}

#[test]
fn feature_gate_and_mask_match_pinned_rippled() {
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
fn preflight_uses_enable_flags_and_rejects_invalid_immutable_bits() {
    let mut disabled = preflight(tfMPTSetCanTransfer, None);
    disabled.dynamic_mpt_enabled = false;
    assert_eq!(
        run_mp_token_issuance_set_preflight(disabled),
        Ter::TEM_DISABLED
    );
    assert_eq!(
        run_mp_token_issuance_set_preflight(preflight(0, Some(0))),
        Ter::TEM_INVALID_FLAG
    );
    assert_eq!(
        run_mp_token_issuance_set_preflight(preflight(0, Some(tifMPTokenIssuanceImmutableMask))),
        Ter::TEM_INVALID_FLAG
    );
    let mut fee = preflight(tfMPTSetCanTransfer, None);
    fee.transfer_fee = Some(MAX_TRANSFER_FEE + 1);
    assert_eq!(
        run_mp_token_issuance_set_preflight(fee),
        Ter::TEM_BAD_TRANSFER_FEE
    );
    let mut metadata = preflight(0, None);
    metadata.metadata_len = Some(MAX_MPTOKEN_METADATA_LENGTH + 1);
    assert_eq!(
        run_mp_token_issuance_set_preflight(metadata),
        Ter::TEM_MALFORMED
    );
    let mut mixed = preflight(tfMPTLock | tfMPTSetCanTransfer, None);
    assert_eq!(
        run_mp_token_issuance_set_preflight(mixed),
        Ter::TEM_MALFORMED
    );
    mixed.tx_flags = tfMPTSetCanTransfer;
    assert_eq!(run_mp_token_issuance_set_preflight(mixed), Ter::TES_SUCCESS);
}

#[test]
fn preclaim_enforces_one_way_immutability_and_same_tx_transfer_enable() {
    assert_eq!(
        run_mp_token_issuance_set_preclaim(preclaim(tfMPTSetCanTransfer, lsifMPTCanTransfer)),
        Ter::TEC_NO_PERMISSION
    );
    let mut same_tx_fee = preclaim(tfMPTSetCanTransfer, 0);
    same_tx_fee.transfer_fee = Some(10);
    assert_eq!(
        run_mp_token_issuance_set_preclaim(same_tx_fee),
        Ter::TES_SUCCESS
    );
    let mut immutable_metadata = preclaim(0, lsifMPTMetadata);
    immutable_metadata.metadata_present = true;
    assert_eq!(
        run_mp_token_issuance_set_preclaim(immutable_metadata),
        Ter::TEC_NO_PERMISSION
    );
    let mut immutable_fee = preclaim(0, lsifMPTTransferFee);
    immutable_fee.transfer_fee = Some(0);
    assert_eq!(
        run_mp_token_issuance_set_preclaim(immutable_fee),
        Ter::TEC_NO_PERMISSION
    );
}

#[test]
fn confidential_preclaim_checks_match_pinned_precedence_and_constraints() {
    let mut immutable_fee_precedes_confidential = preclaim(0, lsifMPTTransferFee);
    immutable_fee_precedes_confidential.transfer_fee = Some(10);
    immutable_fee_precedes_confidential.issuance_has_confidential_balance = true;
    assert_eq!(
        run_mp_token_issuance_set_preclaim(immutable_fee_precedes_confidential),
        Ter::TEC_NO_PERMISSION
    );

    let mut fee_on_confidential = preclaim(0, 0);
    fee_on_confidential.transfer_fee = Some(10);
    fee_on_confidential.issuance_can_transfer = true;
    fee_on_confidential.issuance_has_confidential_balance = true;
    assert_eq!(
        run_mp_token_issuance_set_preclaim(fee_on_confidential),
        Ter::TEC_NO_PERMISSION
    );

    let mut duplicate_issuer_key = preclaim(0, 0);
    duplicate_issuer_key.tx_has_issuer_encryption_key = true;
    duplicate_issuer_key.issuer_encryption_key_present = true;
    duplicate_issuer_key.issuance_has_confidential_balance = true;
    assert_eq!(
        run_mp_token_issuance_set_preclaim(duplicate_issuer_key),
        Ter::TEC_NO_PERMISSION
    );

    let mut enable_with_fee = preclaim(tfMPTSetCanHoldConfidentialBalance, 0);
    enable_with_fee.issuance_transfer_fee_nonzero = true;
    assert_eq!(
        run_mp_token_issuance_set_preclaim(enable_with_fee),
        Ter::TEC_NO_PERMISSION
    );

    let mut key_without_confidential = preclaim(0, 0);
    key_without_confidential.tx_has_issuer_encryption_key = true;
    assert_eq!(
        run_mp_token_issuance_set_preclaim(key_without_confidential),
        Ter::TEC_NO_PERMISSION
    );

    let mut same_tx_enable_and_key = key_without_confidential;
    same_tx_enable_and_key.tx_flags = tfMPTSetCanHoldConfidentialBalance;
    assert_eq!(
        run_mp_token_issuance_set_preclaim(same_tx_enable_and_key),
        Ter::TES_SUCCESS
    );

    let mut outstanding = same_tx_enable_and_key;
    outstanding.confidential_outstanding_nonzero = true;
    assert_eq!(
        run_mp_token_issuance_set_preclaim(outstanding),
        Ter::TEC_NO_PERMISSION
    );
}

#[test]
fn apply_only_enables_capabilities_and_ors_immutable_bits() {
    let mut sink = Sink {
        exists: true,
        flags: lsfMPTCanLock,
        immutable: lsifMPTMetadata,
        ..Sink::default()
    };
    assert_eq!(
        run_mp_token_issuance_set_do_apply(
            MPTokenIssuanceSetApplyFacts {
                tx_flags: tfMPTSetCanTransfer,
                mutable_flags: Some(lsifMPTCanTransfer),
                transfer_fee: Some(5),
                metadata: Some(vec![1]),
                domain: MPTokenIssuanceSetDomainUpdate::Set("domain")
            },
            &mut sink
        ),
        Ter::TES_SUCCESS
    );
    assert_eq!(sink.flags, lsfMPTCanLock | lsfMPTCanTransfer);
    assert_eq!(sink.immutable, lsifMPTMetadata | lsifMPTCanTransfer);
    assert_eq!(sink.fee, Some(5));
    assert_eq!(sink.metadata, Some(vec![1]));
    assert_eq!(sink.domain, Some("domain"));
    assert_eq!(sink.finished, 1);
}

#[test]
fn apply_lock_unlock_and_default_field_clears_match_pinned_order() {
    let mut sink = Sink {
        exists: true,
        flags: lsfMPTLocked,
        fee: Some(5),
        metadata: Some(vec![1]),
        domain: Some("old"),
        ..Sink::default()
    };
    assert_eq!(
        run_mp_token_issuance_set_do_apply(
            MPTokenIssuanceSetApplyFacts {
                tx_flags: tfMPTUnlock,
                mutable_flags: None,
                transfer_fee: Some(0),
                metadata: Some(Vec::new()),
                domain: MPTokenIssuanceSetDomainUpdate::Clear
            },
            &mut sink
        ),
        Ter::TES_SUCCESS
    );
    assert_eq!(sink.flags, 0);
    assert_eq!(sink.fee, None);
    assert_eq!(sink.metadata, None);
    assert_eq!(sink.domain, None);
}

#[test]
fn delegated_lock_permission_remains_granular() {
    let denied = run_mp_token_issuance_set_check_permission(MPTokenIssuanceSetPermissionFacts {
        delegate_present: true,
        delegate_entry_exists: true,
        broad_permission_granted: false,
        tx_flags: tfMPTLock,
        granular_permissions: BTreeSet::new(),
    });
    let allowed = run_mp_token_issuance_set_check_permission(MPTokenIssuanceSetPermissionFacts {
        delegate_present: true,
        delegate_entry_exists: true,
        broad_permission_granted: false,
        tx_flags: tfMPTLock,
        granular_permissions: BTreeSet::from([MPTokenIssuanceSetGranularPermission::Lock]),
    });
    assert_eq!(denied, Ter::TER_NO_DELEGATE_PERMISSION);
    assert_eq!(allowed, Ter::TES_SUCCESS);
}
