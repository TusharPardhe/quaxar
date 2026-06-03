//! Deterministic the reference implementation shells.
//!
//! This ports the current compatibility-safe surface for:
//!
//! - `checkExtraFeatures(...)`,
//! - `getFlagsMask(...)`,
//! - `preflight(...)`,
//! - `checkPermission(...)`,
//! - `preclaim(...)`,
//! - and the loaded `doApply()` mutation ordering.

use std::collections::BTreeSet;

use protocol::{
    NotTec, Ter, lsfMPTLocked, lsmfMPTCanMutateCanClawback, lsmfMPTCanMutateCanEscrow,
    lsmfMPTCanMutateCanLock, lsmfMPTCanMutateCanTrade, lsmfMPTCanMutateCanTransfer,
    lsmfMPTCanMutateMetadata, lsmfMPTCanMutateRequireAuth, lsmfMPTCanMutateTransferFee, tfMPTLock,
    tfMPTUnlock, tfMPTokenIssuanceSetMask, tfUniversalMask, tmfMPTClearCanClawback,
    tmfMPTClearCanEscrow, tmfMPTClearCanLock, tmfMPTClearCanTrade, tmfMPTClearCanTransfer,
    tmfMPTClearRequireAuth, tmfMPTSetCanClawback, tmfMPTSetCanEscrow, tmfMPTSetCanLock,
    tmfMPTSetCanTrade, tmfMPTSetCanTransfer, tmfMPTSetRequireAuth,
    tmfMPTokenIssuanceSetMutableMask,
};

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
    pub current_mutable_flags: u32,
    pub mutable_flags: Option<u32>,
    pub metadata_present: bool,
    pub transfer_fee: Option<u16>,
    pub issuance_can_transfer: bool,
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
    pub mutable_flags: Option<u32>,
    pub transfer_fee: Option<u16>,
    pub metadata: Option<Vec<u8>>,
    pub domain: MPTokenIssuanceSetDomainUpdate<DomainId>,
}

pub trait MPTokenIssuanceSetApplySink<DomainId> {
    fn target_exists(&mut self) -> bool;
    fn current_flags(&mut self) -> u32;
    fn set_flags(&mut self, flags: u32);
    fn clear_transfer_fee(&mut self);
    fn set_transfer_fee(&mut self, transfer_fee: u16);
    fn clear_metadata(&mut self);
    fn set_metadata(&mut self, metadata: Vec<u8>);
    fn clear_domain(&mut self);
    fn set_domain(&mut self, domain: DomainId);
    fn finish_update(&mut self);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct MPTMutabilityFlags {
    set_flag: u32,
    clear_flag: u32,
    can_mutate_flag: u32,
}

const MPT_MUTABILITY_FLAGS: [MPTMutabilityFlags; 6] = [
    MPTMutabilityFlags {
        set_flag: tmfMPTSetCanLock,
        clear_flag: tmfMPTClearCanLock,
        can_mutate_flag: lsmfMPTCanMutateCanLock,
    },
    MPTMutabilityFlags {
        set_flag: tmfMPTSetRequireAuth,
        clear_flag: tmfMPTClearRequireAuth,
        can_mutate_flag: lsmfMPTCanMutateRequireAuth,
    },
    MPTMutabilityFlags {
        set_flag: tmfMPTSetCanEscrow,
        clear_flag: tmfMPTClearCanEscrow,
        can_mutate_flag: lsmfMPTCanMutateCanEscrow,
    },
    MPTMutabilityFlags {
        set_flag: tmfMPTSetCanTrade,
        clear_flag: tmfMPTClearCanTrade,
        can_mutate_flag: lsmfMPTCanMutateCanTrade,
    },
    MPTMutabilityFlags {
        set_flag: tmfMPTSetCanTransfer,
        clear_flag: tmfMPTClearCanTransfer,
        can_mutate_flag: lsmfMPTCanMutateCanTransfer,
    },
    MPTMutabilityFlags {
        set_flag: tmfMPTSetCanClawback,
        clear_flag: tmfMPTClearCanClawback,
        can_mutate_flag: lsmfMPTCanMutateCanClawback,
    },
];

pub fn mp_token_issuance_set_check_extra_features(
    domain_id_present: bool,
    permissioned_domains_enabled: bool,
    single_asset_vault_enabled: bool,
) -> bool {
    !domain_id_present || (permissioned_domains_enabled && single_asset_vault_enabled)
}

pub const fn get_mp_token_issuance_set_flags_mask() -> u32 {
    tfMPTokenIssuanceSetMask
}

pub fn run_mp_token_issuance_set_preflight(facts: MPTokenIssuanceSetPreflightFacts) -> NotTec {
    let is_mutate = facts.mutable_flags.is_some()
        || facts.metadata_len.is_some()
        || facts.transfer_fee.is_some();

    if is_mutate && !facts.dynamic_mpt_enabled {
        return Ter::TEM_DISABLED;
    }

    if facts.domain_id_present && facts.holder_present {
        return Ter::TEM_MALFORMED;
    }

    if (facts.tx_flags & tfMPTLock) != 0 && (facts.tx_flags & tfMPTUnlock) != 0 {
        return Ter::TEM_INVALID_FLAG;
    }

    if facts.holder_present && facts.account_equals_holder {
        return Ter::TEM_MALFORMED;
    }

    if (facts.single_asset_vault_enabled || facts.dynamic_mpt_enabled)
        && facts.tx_flags == 0
        && !facts.domain_id_present
        && !is_mutate
    {
        return Ter::TEM_MALFORMED;
    }

    if facts.dynamic_mpt_enabled {
        if is_mutate && facts.holder_present {
            return Ter::TEM_MALFORMED;
        }

        if is_mutate && (facts.tx_flags & tfUniversalMask) != 0 {
            return Ter::TEM_MALFORMED;
        }

        if facts.transfer_fee.is_some_and(|fee| fee > MAX_TRANSFER_FEE) {
            return Ter::TEM_BAD_TRANSFER_FEE;
        }

        if facts
            .metadata_len
            .is_some_and(|metadata_len| metadata_len > MAX_MPTOKEN_METADATA_LENGTH)
        {
            return Ter::TEM_MALFORMED;
        }

        if let Some(mutable_flags) = facts.mutable_flags {
            if mutable_flags == 0 || (mutable_flags & tmfMPTokenIssuanceSetMutableMask) != 0 {
                return Ter::TEM_INVALID_FLAG;
            }

            if MPT_MUTABILITY_FLAGS.iter().any(|flag| {
                (mutable_flags & flag.set_flag) != 0 && (mutable_flags & flag.clear_flag) != 0
            }) {
                return Ter::TEM_INVALID_FLAG;
            }

            if facts.transfer_fee.unwrap_or(0) != 0 && (mutable_flags & tmfMPTClearCanTransfer) != 0
            {
                return Ter::TEM_MALFORMED;
            }
        }
    }

    Ter::TES_SUCCESS
}

pub fn run_mp_token_issuance_set_check_permission(
    facts: MPTokenIssuanceSetPermissionFacts,
) -> NotTec {
    if !facts.delegate_present {
        return Ter::TES_SUCCESS;
    }

    if !facts.delegate_entry_exists {
        return Ter::TER_NO_DELEGATE_PERMISSION;
    }

    if facts.broad_permission_granted {
        return Ter::TES_SUCCESS;
    }

    if (facts.tx_flags & tfMPTokenIssuanceSetMask) != 0 {
        return Ter::TER_NO_DELEGATE_PERMISSION;
    }

    if (facts.tx_flags & tfMPTLock) != 0
        && !facts
            .granular_permissions
            .contains(&MPTokenIssuanceSetGranularPermission::Lock)
    {
        return Ter::TER_NO_DELEGATE_PERMISSION;
    }

    if (facts.tx_flags & tfMPTUnlock) != 0
        && !facts
            .granular_permissions
            .contains(&MPTokenIssuanceSetGranularPermission::Unlock)
    {
        return Ter::TER_NO_DELEGATE_PERMISSION;
    }

    Ter::TES_SUCCESS
}

pub fn run_mp_token_issuance_set_preclaim(facts: MPTokenIssuanceSetPreclaimFacts) -> Ter {
    if !facts.issuance_exists {
        return Ter::TEC_OBJECT_NOT_FOUND;
    }

    if !facts.issuance_can_lock {
        if !facts.single_asset_vault_enabled && !facts.dynamic_mpt_enabled {
            return Ter::TEC_NO_PERMISSION;
        }
        if (facts.tx_flags & tfMPTLock) != 0 || (facts.tx_flags & tfMPTUnlock) != 0 {
            return Ter::TEC_NO_PERMISSION;
        }
    }

    if !facts.issuer_matches {
        return Ter::TEC_NO_PERMISSION;
    }

    if facts.holder_present {
        if !facts.holder_account_exists {
            return Ter::TEC_NO_DST;
        }

        if !facts.holder_token_exists {
            return Ter::TEC_OBJECT_NOT_FOUND;
        }
    }

    if facts.domain_id_present {
        if !facts.issuance_requires_auth {
            return Ter::TEC_NO_PERMISSION;
        }

        if !facts.domain_id_is_zero && !facts.domain_exists {
            return Ter::TEC_OBJECT_NOT_FOUND;
        }
    }

    if let Some(mutable_flags) = facts.mutable_flags
        && MPT_MUTABILITY_FLAGS.iter().any(|flag| {
            (facts.current_mutable_flags & flag.can_mutate_flag) == 0
                && (mutable_flags & (flag.set_flag | flag.clear_flag)) != 0
        })
    {
        return Ter::TEC_NO_PERMISSION;
    }

    if facts
        .mutable_flags
        .is_some_and(|flags| (flags & tmfMPTClearRequireAuth) != 0)
        && facts.issuance_domain_present
    {
        return Ter::TEC_NO_PERMISSION;
    }

    if facts.metadata_present && (facts.current_mutable_flags & lsmfMPTCanMutateMetadata) == 0 {
        return Ter::TEC_NO_PERMISSION;
    }

    if let Some(transfer_fee) = facts.transfer_fee {
        if transfer_fee > 0 && !facts.issuance_can_transfer {
            return Ter::TEC_NO_PERMISSION;
        }

        if (facts.current_mutable_flags & lsmfMPTCanMutateTransferFee) == 0 {
            return Ter::TEC_NO_PERMISSION;
        }
    }

    Ter::TES_SUCCESS
}

pub fn run_mp_token_issuance_set_do_apply<DomainId, S>(
    facts: MPTokenIssuanceSetApplyFacts<DomainId>,
    sink: &mut S,
) -> Ter
where
    DomainId: Clone,
    S: MPTokenIssuanceSetApplySink<DomainId>,
{
    if !sink.target_exists() {
        return Ter::TEC_INTERNAL;
    }

    let flags_in = sink.current_flags();
    let mut flags_out = flags_in;

    if (facts.tx_flags & tfMPTLock) != 0 {
        flags_out |= lsfMPTLocked;
    } else if (facts.tx_flags & tfMPTUnlock) != 0 {
        flags_out &= !lsfMPTLocked;
    }

    if let Some(mutable_flags) = facts.mutable_flags {
        for flag in MPT_MUTABILITY_FLAGS {
            if (mutable_flags & flag.set_flag) != 0 {
                flags_out |= flag.can_mutate_flag;
            } else if (mutable_flags & flag.clear_flag) != 0 {
                flags_out &= !flag.can_mutate_flag;
            }
        }

        if (mutable_flags & tmfMPTClearCanTransfer) != 0 {
            sink.clear_transfer_fee();
        }
    }

    if flags_in != flags_out {
        sink.set_flags(flags_out);
    }

    if let Some(transfer_fee) = facts.transfer_fee {
        if transfer_fee == 0 {
            sink.clear_transfer_fee();
        } else {
            sink.set_transfer_fee(transfer_fee);
        }
    }

    if let Some(metadata) = facts.metadata {
        if metadata.is_empty() {
            sink.clear_metadata();
        } else {
            sink.set_metadata(metadata);
        }
    }

    match facts.domain {
        MPTokenIssuanceSetDomainUpdate::NoChange => {}
        MPTokenIssuanceSetDomainUpdate::Clear => sink.clear_domain(),
        MPTokenIssuanceSetDomainUpdate::Set(domain) => sink.set_domain(domain),
    }

    sink.finish_update();
    Ter::TES_SUCCESS
}
