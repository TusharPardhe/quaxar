//! Pinned-rippled MPTokenIssuanceSet validation and deterministic mutation helpers.

use protocol::{
    NotTec, Ter, lsfMPTCanClawback, lsfMPTCanEscrow, lsfMPTCanHoldConfidentialBalance,
    lsfMPTCanLock, lsfMPTCanTrade, lsfMPTCanTransfer, lsfMPTLocked, lsfMPTRequireAuth,
    lsifMPTCanClawback, lsifMPTCanEscrow, lsifMPTCanHoldConfidentialBalance, lsifMPTCanLock,
    lsifMPTCanTrade, lsifMPTCanTransfer, lsifMPTMetadata, lsifMPTRequireAuth, lsifMPTTransferFee,
    tfMPTLock, tfMPTSetCanClawback, tfMPTSetCanEscrow, tfMPTSetCanHoldConfidentialBalance,
    tfMPTSetCanLock, tfMPTSetCanTrade, tfMPTSetCanTransfer, tfMPTSetRequireAuth, tfMPTUnlock,
    tfMPTokenIssuanceSetEnableFlagMask, tfMPTokenIssuanceSetMask, tifMPTokenIssuanceImmutableMask,
};
use std::collections::BTreeSet;

pub const MAX_TRANSFER_FEE: u16 = 50_000;
pub const MAX_MPTOKEN_METADATA_LENGTH: usize = 1_024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MPTokenIssuanceSetPreflightFacts {
    pub dynamic_mpt_enabled: bool,
    pub single_asset_vault_enabled: bool,
    pub domain_id_present: bool,
    pub holder_present: bool,
    pub account_equals_holder: bool,
    pub tx_flags: u32,
    /// Carries the pinned wire field sfImmutableFlags.
    pub mutable_flags: Option<u32>,
    pub metadata_len: Option<usize>,
    pub transfer_fee: Option<u16>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum MPTokenIssuanceSetGranularPermission {
    Lock,
    Unlock,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MPTokenIssuanceSetPermissionFacts {
    pub delegate_present: bool,
    pub delegate_entry_exists: bool,
    pub broad_permission_granted: bool,
    pub tx_flags: u32,
    pub granular_permissions: BTreeSet<MPTokenIssuanceSetGranularPermission>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MPTokenIssuanceSetPreclaimFacts {
    pub issuance_exists: bool,
    pub issuance_can_lock: bool,
    pub single_asset_vault_enabled: bool,
    pub dynamic_mpt_enabled: bool,
    pub tx_flags: u32,
    pub issuer_matches: bool,
    pub holder_present: bool,
    pub holder_account_exists: bool,
    pub holder_token_exists: bool,
    pub domain_id_present: bool,
    pub domain_id_is_zero: bool,
    pub issuance_requires_auth: bool,
    pub domain_exists: bool,
    pub issuance_domain_present: bool,
    /// Carries the ledger's sfImmutableFlags value.
    pub current_mutable_flags: u32,
    /// Carries the transaction's sfImmutableFlags value.
    pub mutable_flags: Option<u32>,
    pub metadata_present: bool,
    pub transfer_fee: Option<u16>,
    pub issuance_can_transfer: bool,
    pub issuance_has_confidential_balance: bool,
    pub issuance_transfer_fee_nonzero: bool,
    pub issuer_encryption_key_present: bool,
    pub auditor_encryption_key_present: bool,
    pub tx_has_issuer_encryption_key: bool,
    pub tx_has_auditor_encryption_key: bool,
    pub confidential_outstanding_nonzero: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MPTokenIssuanceSetDomainUpdate<DomainId> {
    NoChange,
    Clear,
    Set(DomainId),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MPTokenIssuanceSetApplyFacts<DomainId> {
    pub tx_flags: u32,
    /// Carries sfImmutableFlags; bits are additive and never clear capability flags.
    pub mutable_flags: Option<u32>,
    pub transfer_fee: Option<u16>,
    pub metadata: Option<Vec<u8>>,
    pub domain: MPTokenIssuanceSetDomainUpdate<DomainId>,
}

pub trait MPTokenIssuanceSetApplySink<DomainId> {
    fn target_exists(&mut self) -> bool;
    fn current_flags(&mut self) -> u32;
    fn set_flags(&mut self, flags: u32);
    fn current_immutable_flags(&mut self) -> u32 {
        0
    }
    fn set_immutable_flags(&mut self, _flags: u32) {}
    fn clear_transfer_fee(&mut self);
    fn set_transfer_fee(&mut self, transfer_fee: u16);
    fn clear_metadata(&mut self);
    fn set_metadata(&mut self, metadata: Vec<u8>);
    fn clear_domain(&mut self);
    fn set_domain(&mut self, domain: DomainId);
    fn finish_update(&mut self);
}

#[derive(Clone, Copy)]
struct FlagMapping {
    set: u32,
    immutable: u32,
    ledger: u32,
}
const FLAG_MAPPING: [FlagMapping; 7] = [
    FlagMapping {
        set: tfMPTSetCanLock,
        immutable: lsifMPTCanLock,
        ledger: lsfMPTCanLock,
    },
    FlagMapping {
        set: tfMPTSetRequireAuth,
        immutable: lsifMPTRequireAuth,
        ledger: lsfMPTRequireAuth,
    },
    FlagMapping {
        set: tfMPTSetCanEscrow,
        immutable: lsifMPTCanEscrow,
        ledger: lsfMPTCanEscrow,
    },
    FlagMapping {
        set: tfMPTSetCanTrade,
        immutable: lsifMPTCanTrade,
        ledger: lsfMPTCanTrade,
    },
    FlagMapping {
        set: tfMPTSetCanTransfer,
        immutable: lsifMPTCanTransfer,
        ledger: lsfMPTCanTransfer,
    },
    FlagMapping {
        set: tfMPTSetCanClawback,
        immutable: lsifMPTCanClawback,
        ledger: lsfMPTCanClawback,
    },
    FlagMapping {
        set: tfMPTSetCanHoldConfidentialBalance,
        immutable: lsifMPTCanHoldConfidentialBalance,
        ledger: lsfMPTCanHoldConfidentialBalance,
    },
];

pub fn mp_token_issuance_set_check_extra_features(
    domain: bool,
    permissioned: bool,
    vault: bool,
) -> bool {
    !domain || (permissioned && vault)
}
pub const fn get_mp_token_issuance_set_flags_mask() -> u32 {
    tfMPTokenIssuanceSetMask
}

pub fn run_mp_token_issuance_set_preflight(f: MPTokenIssuanceSetPreflightFacts) -> NotTec {
    let enable = f.tx_flags & tfMPTokenIssuanceSetEnableFlagMask;
    let mutate = enable != 0
        || f.mutable_flags.is_some()
        || f.metadata_len.is_some()
        || f.transfer_fee.is_some();
    if mutate && !f.dynamic_mpt_enabled {
        return Ter::TEM_DISABLED;
    }
    if f.domain_id_present && f.holder_present {
        return Ter::TEM_MALFORMED;
    }
    if (enable & tfMPTSetCanHoldConfidentialBalance) != 0 && f.holder_present {
        return Ter::TEM_MALFORMED;
    }
    if (f.tx_flags & tfMPTLock) != 0 && (f.tx_flags & tfMPTUnlock) != 0 {
        return Ter::TEM_INVALID_FLAG;
    }
    if f.holder_present && f.account_equals_holder {
        return Ter::TEM_MALFORMED;
    }
    if (f.single_asset_vault_enabled || f.dynamic_mpt_enabled)
        && f.tx_flags == 0
        && !f.domain_id_present
        && !mutate
    {
        return Ter::TEM_MALFORMED;
    }
    if f.dynamic_mpt_enabled {
        if mutate && f.holder_present {
            return Ter::TEM_MALFORMED;
        }
        if mutate && (f.tx_flags & (tfMPTLock | tfMPTUnlock)) != 0 {
            return Ter::TEM_MALFORMED;
        }
        if f.transfer_fee.is_some_and(|x| x > MAX_TRANSFER_FEE) {
            return Ter::TEM_BAD_TRANSFER_FEE;
        }
        if f.transfer_fee.is_some_and(|x| x > 0)
            && (enable & tfMPTSetCanHoldConfidentialBalance) != 0
        {
            return Ter::TEM_BAD_TRANSFER_FEE;
        }
        if f.metadata_len
            .is_some_and(|x| x > MAX_MPTOKEN_METADATA_LENGTH)
        {
            return Ter::TEM_MALFORMED;
        }
        if f.mutable_flags
            .is_some_and(|x| x == 0 || (x & tifMPTokenIssuanceImmutableMask) != 0)
        {
            return Ter::TEM_INVALID_FLAG;
        }
    }
    Ter::TES_SUCCESS
}

pub fn run_mp_token_issuance_set_check_permission(f: MPTokenIssuanceSetPermissionFacts) -> NotTec {
    if !f.delegate_present {
        return Ter::TES_SUCCESS;
    }
    if !f.delegate_entry_exists {
        return Ter::TER_NO_DELEGATE_PERMISSION;
    }
    if f.broad_permission_granted {
        return Ter::TES_SUCCESS;
    }
    if (f.tx_flags & tfMPTokenIssuanceSetMask) != 0 {
        return Ter::TER_NO_DELEGATE_PERMISSION;
    }
    if (f.tx_flags & tfMPTLock) != 0
        && !f
            .granular_permissions
            .contains(&MPTokenIssuanceSetGranularPermission::Lock)
    {
        return Ter::TER_NO_DELEGATE_PERMISSION;
    }
    if (f.tx_flags & tfMPTUnlock) != 0
        && !f
            .granular_permissions
            .contains(&MPTokenIssuanceSetGranularPermission::Unlock)
    {
        return Ter::TER_NO_DELEGATE_PERMISSION;
    }
    Ter::TES_SUCCESS
}

pub fn run_mp_token_issuance_set_preclaim(f: MPTokenIssuanceSetPreclaimFacts) -> Ter {
    if !f.issuance_exists {
        return Ter::TEC_OBJECT_NOT_FOUND;
    }
    if !f.issuance_can_lock {
        if !f.single_asset_vault_enabled && !f.dynamic_mpt_enabled {
            return Ter::TEC_NO_PERMISSION;
        }
        if (f.tx_flags & (tfMPTLock | tfMPTUnlock)) != 0 {
            return Ter::TEC_NO_PERMISSION;
        }
    }
    if !f.issuer_matches {
        return Ter::TEC_NO_PERMISSION;
    }
    if f.holder_present {
        if !f.holder_account_exists {
            return Ter::TEC_NO_DST;
        }
        if !f.holder_token_exists {
            return Ter::TEC_OBJECT_NOT_FOUND;
        }
    }
    if f.domain_id_present {
        if !f.issuance_requires_auth {
            return Ter::TEC_NO_PERMISSION;
        }
        if !f.domain_id_is_zero && !f.domain_exists {
            return Ter::TEC_OBJECT_NOT_FOUND;
        }
    }
    let enable = f.tx_flags & tfMPTokenIssuanceSetEnableFlagMask;
    if FLAG_MAPPING
        .iter()
        .any(|m| (enable & m.set) != 0 && (f.current_mutable_flags & m.immutable) != 0)
    {
        return Ter::TEC_NO_PERMISSION;
    }
    if f.metadata_present && (f.current_mutable_flags & lsifMPTMetadata) != 0 {
        return Ter::TEC_NO_PERMISSION;
    }
    if let Some(fee) = f.transfer_fee {
        if fee > 0 && !f.issuance_can_transfer && (enable & tfMPTSetCanTransfer) == 0 {
            return Ter::TEC_NO_PERMISSION;
        }
        if fee > 0 && f.issuance_has_confidential_balance {
            return Ter::TEC_NO_PERMISSION;
        }
        if (f.current_mutable_flags & lsifMPTTransferFee) != 0 {
            return Ter::TEC_NO_PERMISSION;
        }
    }
    // Preserve MPTokenIssuanceSet.cpp's confidential-transfer preclaim order.
    // These checks belong in the shared ledger-aware preclaim rather than only
    // in one dispatcher, otherwise typed execution can accept a transaction
    // which the consensus route rejects later.
    if f.tx_has_issuer_encryption_key && f.issuer_encryption_key_present {
        return Ter::TEC_NO_PERMISSION;
    }
    if f.tx_has_auditor_encryption_key && f.auditor_encryption_key_present {
        return Ter::TEC_NO_PERMISSION;
    }
    let enables_confidential_balance = (enable & tfMPTSetCanHoldConfidentialBalance) != 0;
    if enables_confidential_balance && f.issuance_transfer_fee_nonzero {
        return Ter::TEC_NO_PERMISSION;
    }
    if f.tx_has_issuer_encryption_key
        && !f.issuance_has_confidential_balance
        && !enables_confidential_balance
    {
        return Ter::TEC_NO_PERMISSION;
    }
    if f.tx_has_auditor_encryption_key
        && !f.issuance_has_confidential_balance
        && !enables_confidential_balance
    {
        return Ter::TEC_NO_PERMISSION;
    }
    if (f.tx_has_issuer_encryption_key
        || f.tx_has_auditor_encryption_key
        || enables_confidential_balance)
        && f.confidential_outstanding_nonzero
    {
        return Ter::TEC_NO_PERMISSION;
    }
    Ter::TES_SUCCESS
}

pub fn run_mp_token_issuance_set_do_apply<D: Clone, S: MPTokenIssuanceSetApplySink<D>>(
    f: MPTokenIssuanceSetApplyFacts<D>,
    sink: &mut S,
) -> Ter {
    if !sink.target_exists() {
        return Ter::TEC_INTERNAL;
    }
    let before = sink.current_flags();
    let mut after = before;
    if (f.tx_flags & tfMPTLock) != 0 {
        after |= lsfMPTLocked;
    } else if (f.tx_flags & tfMPTUnlock) != 0 {
        after &= !lsfMPTLocked;
    }
    for m in FLAG_MAPPING {
        if (f.tx_flags & m.set) != 0 {
            after |= m.ledger;
        }
    }
    if before != after {
        sink.set_flags(after);
    }
    if let Some(bits) = f.mutable_flags {
        let old = sink.current_immutable_flags();
        sink.set_immutable_flags(old | bits);
    }
    if let Some(fee) = f.transfer_fee {
        if fee == 0 {
            sink.clear_transfer_fee();
        } else {
            sink.set_transfer_fee(fee);
        }
    }
    if let Some(data) = f.metadata {
        if data.is_empty() {
            sink.clear_metadata();
        } else {
            sink.set_metadata(data);
        }
    }
    match f.domain {
        MPTokenIssuanceSetDomainUpdate::NoChange => {}
        MPTokenIssuanceSetDomainUpdate::Clear => sink.clear_domain(),
        MPTokenIssuanceSetDomainUpdate::Set(d) => sink.set_domain(d),
    }
    sink.finish_update();
    Ter::TES_SUCCESS
}
