//! Deterministic the reference implementation shells.
//!
//! This ports the current compatibility-safe surface for:
//!
//! - `checkExtraFeatures(...)`,
//! - `getFlagsMask(...)`,
//! - the ordered `preflight(...)` validation,
//! - and the owner-dir / owner-count `create(...)` mutation shell used by
//!   `doApply()`.

use protocol::{
    NotTec, Ter, tfMPTCanHoldConfidentialBalance, tfMPTCanTransfer, tfMPTRequireAuth,
    tfMPTokenIssuanceCreateMask, tfUniversal, tmfMPTokenIssuanceCreateMutableMask,
};

pub const MAX_TRANSFER_FEE: u16 = 50_000;
pub const MAX_MPTOKEN_METADATA_LENGTH: usize = 1_024;
pub const MAX_MPTOKEN_AMOUNT: u64 = 0x7fff_ffff_ffff_ffff;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MPTokenIssuanceCreatePreflightFacts {
    pub fix_cleanup_3_2_0_enabled: bool,
    pub confidential_transfer_enabled: bool,
    pub reference_holding_present: bool,
    pub mutable_flags: Option<u32>,
    pub tx_flags: u32,
    pub transfer_fee: Option<u16>,
    pub domain_id_present: bool,
    pub domain_id_is_zero: bool,
    pub metadata_len: Option<usize>,
    pub maximum_amount: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MPTokenIssuanceCreateApplyFacts<AccountId, Metadata, DomainId> {
    pub account: AccountId,
    pub flags: u32,
    pub sequence: u32,
    pub maximum_amount: Option<u64>,
    pub asset_scale: Option<u8>,
    pub transfer_fee: Option<u16>,
    pub metadata: Option<Metadata>,
    pub domain_id: Option<DomainId>,
    pub mutable_flags: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MPTokenIssuanceCreateMutation<AccountId, Metadata, DomainId> {
    pub account: AccountId,
    pub flags: u32,
    pub sequence: u32,
    pub outstanding_amount: u64,
    pub owner_node: u64,
    pub maximum_amount: Option<u64>,
    pub asset_scale: Option<u8>,
    pub transfer_fee: Option<u16>,
    pub metadata: Option<Metadata>,
    pub domain_id: Option<DomainId>,
    pub mutable_flags: Option<u32>,
}

pub trait MPTokenIssuanceCreateApplySink<AccountId, Metadata, DomainId> {
    fn account_exists(&mut self) -> bool;
    fn reserve_sufficient(&mut self) -> bool;
    fn insert_owner_dir(&mut self) -> Option<u64>;
    fn create_issuance(
        &mut self,
        mutation: MPTokenIssuanceCreateMutation<AccountId, Metadata, DomainId>,
    );
    fn adjust_owner_count(&mut self, delta: i32);
}

pub fn mp_token_issuance_create_check_extra_features(
    domain_id_present: bool,
    permissioned_domains_enabled: bool,
    single_asset_vault_enabled: bool,
    mutable_flags_present: bool,
    dynamic_mpt_enabled: bool,
) -> bool {
    if domain_id_present && !(permissioned_domains_enabled && single_asset_vault_enabled) {
        return false;
    }

    if mutable_flags_present && !dynamic_mpt_enabled {
        return false;
    }

    true
}

pub const fn get_mp_token_issuance_create_flags_mask() -> u32 {
    tfMPTokenIssuanceCreateMask
}

pub fn run_mp_token_issuance_create_preflight(
    facts: MPTokenIssuanceCreatePreflightFacts,
) -> NotTec {
    if facts.fix_cleanup_3_2_0_enabled && facts.reference_holding_present {
        return Ter::TEM_MALFORMED;
    }

    if let Some(mutable_flags) = facts.mutable_flags
        && (mutable_flags == 0 || (mutable_flags & tmfMPTokenIssuanceCreateMutableMask) != 0)
    {
        return Ter::TEM_INVALID_FLAG;
    }

    if let Some(fee) = facts.transfer_fee {
        if fee > MAX_TRANSFER_FEE {
            return Ter::TEM_BAD_TRANSFER_FEE;
        }

        if fee > 0 && (facts.tx_flags & tfMPTCanTransfer) == 0 {
            return Ter::TEM_MALFORMED;
        }

        if fee > 0
            && facts.confidential_transfer_enabled
            && (facts.tx_flags & tfMPTCanHoldConfidentialBalance) != 0
        {
            return Ter::TEM_MALFORMED;
        }
    }

    if facts.domain_id_present {
        if facts.domain_id_is_zero {
            return Ter::TEM_MALFORMED;
        }

        if (facts.tx_flags & tfMPTRequireAuth) == 0 {
            return Ter::TEM_MALFORMED;
        }
    }

    if let Some(metadata_len) = facts.metadata_len
        && (metadata_len == 0 || metadata_len > MAX_MPTOKEN_METADATA_LENGTH)
    {
        return Ter::TEM_MALFORMED;
    }

    if let Some(maximum_amount) = facts.maximum_amount
        && (maximum_amount == 0 || maximum_amount > MAX_MPTOKEN_AMOUNT)
    {
        return Ter::TEM_MALFORMED;
    }

    Ter::TES_SUCCESS
}

pub fn run_mp_token_issuance_create_do_apply<AccountId, Metadata, DomainId, S>(
    facts: MPTokenIssuanceCreateApplyFacts<AccountId, Metadata, DomainId>,
    sink: &mut S,
) -> Ter
where
    AccountId: Clone,
    Metadata: Clone,
    DomainId: Clone,
    S: MPTokenIssuanceCreateApplySink<AccountId, Metadata, DomainId>,
{
    if !sink.account_exists() {
        return Ter::TEC_INTERNAL;
    }

    if !sink.reserve_sufficient() {
        return Ter::TEC_INSUFFICIENT_RESERVE;
    }

    let Some(owner_node) = sink.insert_owner_dir() else {
        return Ter::TEC_DIR_FULL;
    };

    sink.create_issuance(MPTokenIssuanceCreateMutation {
        account: facts.account,
        flags: facts.flags & !tfUniversal,
        sequence: facts.sequence,
        outstanding_amount: 0,
        owner_node,
        maximum_amount: facts.maximum_amount,
        asset_scale: facts.asset_scale,
        transfer_fee: facts.transfer_fee,
        metadata: facts.metadata,
        domain_id: facts.domain_id,
        mutable_flags: facts.mutable_flags,
    });
    sink.adjust_owner_count(1);
    Ter::TES_SUCCESS
}
