//! Current the reference implementation metadata, preflight, and delegated-permission
//! shells.
//!
//! This ports the deterministic behavior around:
//!
//! - the current blocker-versus-normal `makeTxConsequences(...)` category,
//! - the literal `tfAccountSetMask` flags mask,
//! - the ordered `preflight(...)` validation branches,
//! - the narrow `preclaim(...)` owner-dir and clawback gates,
//! - the front, tail, and outer `doApply()` mutation shells,
//! - and the delegated `checkPermission(...)` granular-permission gates.

use protocol::{NotTec, SeqProxy, Ter};

use crate::ApplyFlags;

use crate::consequences::{TxConsequencesShape, build_tx_consequences};
use crate::{TxConsequences, TxConsequencesCategory};

pub const FULLY_CANONICAL_SIGNATURE_FLAG: u32 = 0x8000_0000;
pub const INNER_BATCH_TRANSACTION_FLAG: u32 = 0x4000_0000;
pub const UNIVERSAL_FLAGS: u32 = FULLY_CANONICAL_SIGNATURE_FLAG | INNER_BATCH_TRANSACTION_FLAG;
pub const UNIVERSAL_FLAGS_MASK: u32 = !UNIVERSAL_FLAGS;

pub const ACCOUNT_SET_REQUIRE_DEST_TAG_FLAG: u32 = 0x0001_0000;
pub const ACCOUNT_SET_OPTIONAL_DEST_TAG_FLAG: u32 = 0x0002_0000;
pub const ACCOUNT_SET_REQUIRE_AUTH_FLAG: u32 = 0x0004_0000;
pub const ACCOUNT_SET_OPTIONAL_AUTH_FLAG: u32 = 0x0008_0000;
pub const ACCOUNT_SET_DISALLOW_XRP_FLAG: u32 = 0x0010_0000;
pub const ACCOUNT_SET_ALLOW_XRP_FLAG: u32 = 0x0020_0000;

pub const ACCOUNT_SET_FLAGS_MASK: u32 = !(UNIVERSAL_FLAGS
    | ACCOUNT_SET_REQUIRE_DEST_TAG_FLAG
    | ACCOUNT_SET_OPTIONAL_DEST_TAG_FLAG
    | ACCOUNT_SET_REQUIRE_AUTH_FLAG
    | ACCOUNT_SET_OPTIONAL_AUTH_FLAG
    | ACCOUNT_SET_DISALLOW_XRP_FLAG
    | ACCOUNT_SET_ALLOW_XRP_FLAG);

pub const ASF_REQUIRE_DEST: u32 = 1;
pub const ASF_REQUIRE_AUTH: u32 = 2;
pub const ASF_DISALLOW_XRP: u32 = 3;
pub const ASF_DISABLE_MASTER: u32 = 4;
pub const ASF_ACCOUNT_TXN_ID: u32 = 5;
pub const ASF_NO_FREEZE: u32 = 6;
pub const ASF_GLOBAL_FREEZE: u32 = 7;
pub const ASF_DEFAULT_RIPPLE: u32 = 8;
pub const ASF_DEPOSIT_AUTH: u32 = 9;
pub const ASF_AUTHORIZED_NFTOKEN_MINTER: u32 = 10;
pub const ASF_DISALLOW_INCOMING_NFTOKEN_OFFER: u32 = 12;
pub const ASF_DISALLOW_INCOMING_CHECK: u32 = 13;
pub const ASF_DISALLOW_INCOMING_PAY_CHAN: u32 = 14;
pub const ASF_DISALLOW_INCOMING_TRUSTLINE: u32 = 15;
pub const ASF_ALLOW_TRUST_LINE_CLAWBACK: u32 = 16;
pub const ASF_ALLOW_TRUST_LINE_LOCKING: u32 = 17;

pub const LSF_REQUIRE_DEST_TAG: u32 = 0x0002_0000;
pub const LSF_REQUIRE_AUTH: u32 = 0x0004_0000;
pub const LSF_DISALLOW_XRP: u32 = 0x0008_0000;
pub const LSF_DISABLE_MASTER: u32 = 0x0010_0000;
pub const LSF_NO_FREEZE: u32 = 0x0020_0000;
pub const LSF_GLOBAL_FREEZE: u32 = 0x0040_0000;
pub const LSF_DISALLOW_INCOMING_NFTOKEN_OFFER: u32 = 0x0400_0000;
pub const LSF_DISALLOW_INCOMING_CHECK: u32 = 0x0800_0000;
pub const LSF_DISALLOW_INCOMING_PAY_CHAN: u32 = 0x1000_0000;
pub const LSF_DISALLOW_INCOMING_TRUSTLINE: u32 = 0x2000_0000;
pub const LSF_DEFAULT_RIPPLE: u32 = 0x0080_0000;
pub const LSF_DEPOSIT_AUTH: u32 = 0x0100_0000;
pub const LSF_ALLOW_TRUST_LINE_LOCKING: u32 = 0x4000_0000;
pub const LSF_ALLOW_TRUST_LINE_CLAWBACK: u32 = 0x8000_0000;

pub const TT_ACCOUNT_SET_PERMISSION: u32 = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccountSetGranularPermission {
    DomainSet = 65540,
    EmailHashSet = 65541,
    MessageKeySet = 65542,
    TransferRateSet = 65543,
    TickSizeSet = 65544,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AccountSetPreflightFacts {
    pub tx_flags: u32,
    pub set_flag: u32,
    pub clear_flag: u32,
    pub transfer_rate: Option<u32>,
    pub quality_one: u32,
    pub tick_size: Option<u8>,
    pub min_tick_size: u8,
    pub max_tick_size: u8,
    pub message_key_present: bool,
    pub message_key_is_valid: bool,
    pub domain_len: Option<usize>,
    pub max_domain_length: usize,
    pub nftoken_minter_present: bool,
}

impl Default for AccountSetPreflightFacts {
    fn default() -> Self {
        Self {
            tx_flags: 0,
            set_flag: 0,
            clear_flag: 0,
            transfer_rate: None,
            quality_one: 1_000_000_000,
            tick_size: None,
            min_tick_size: 3,
            max_tick_size: 15,
            message_key_present: false,
            message_key_is_valid: true,
            domain_len: None,
            max_domain_length: 256,
            nftoken_minter_present: false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct AccountSetPreclaimFacts {
    pub tx_flags: u32,
    pub set_flag: u32,
    pub apply_flags: ApplyFlags,
    pub account_exists: bool,
    pub account_flags: u32,
    pub owner_dir_empty: bool,
    pub feature_clawback_enabled: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct AccountSetDoApplyFlagFacts {
    pub tx_flags: u32,
    pub set_flag: u32,
    pub clear_flag: u32,
    pub account_exists: bool,
    pub account_flags: u32,
    pub signed_with_master: bool,
    pub has_regular_key: bool,
    pub has_signer_list: bool,
    pub account_txn_id_present: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccountSetTxnIdAction {
    None,
    Set,
    Clear,
}

impl Default for AccountSetTxnIdAction {
    fn default() -> Self {
        Self::None
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct AccountSetDoApplyFlagState {
    pub account_flags: u32,
    pub account_txn_id_action: AccountSetTxnIdAction,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AccountSetFieldMutation<T> {
    NoChange,
    Clear,
    Set(T),
}

impl<T> Default for AccountSetFieldMutation<T> {
    fn default() -> Self {
        Self::NoChange
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct AccountSetDoApplyTailFacts<AccountId> {
    pub set_flag: u32,
    pub clear_flag: u32,
    pub account_flags: u32,
    pub quality_one: u32,
    pub max_tick_size: u8,
    pub email_hash: Option<u128>,
    pub wallet_locator: Option<Vec<u8>>,
    pub message_key: Option<Vec<u8>>,
    pub domain: Option<Vec<u8>>,
    pub transfer_rate: Option<u32>,
    pub tick_size: Option<u8>,
    pub nftoken_minter: Option<AccountId>,
    pub nftoken_minter_present_on_account: bool,
    pub feature_token_escrow_enabled: bool,
    pub feature_clawback_enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountSetDoApplyFacts<AccountId> {
    pub flag_facts: AccountSetDoApplyFlagFacts,
    pub tail_facts: AccountSetDoApplyTailFacts<AccountId>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountSetDoApplyTailState<AccountId> {
    pub account_flags: u32,
    pub email_hash_action: AccountSetFieldMutation<u128>,
    pub wallet_locator_action: AccountSetFieldMutation<Vec<u8>>,
    pub message_key_action: AccountSetFieldMutation<Vec<u8>>,
    pub domain_action: AccountSetFieldMutation<Vec<u8>>,
    pub transfer_rate_action: AccountSetFieldMutation<u32>,
    pub tick_size_action: AccountSetFieldMutation<u8>,
    pub nftoken_minter_action: AccountSetFieldMutation<AccountId>,
}

impl<AccountId> Default for AccountSetDoApplyTailState<AccountId> {
    fn default() -> Self {
        Self {
            account_flags: 0,
            email_hash_action: AccountSetFieldMutation::NoChange,
            wallet_locator_action: AccountSetFieldMutation::NoChange,
            message_key_action: AccountSetFieldMutation::NoChange,
            domain_action: AccountSetFieldMutation::NoChange,
            transfer_rate_action: AccountSetFieldMutation::NoChange,
            tick_size_action: AccountSetFieldMutation::NoChange,
            nftoken_minter_action: AccountSetFieldMutation::NoChange,
        }
    }
}

pub trait AccountSetDoApplySink {
    type AccountId: Clone;

    fn set_account_txn_id(&mut self);
    fn clear_account_txn_id(&mut self);

    fn set_email_hash(&mut self, value: u128);
    fn clear_email_hash(&mut self);

    fn set_wallet_locator(&mut self, value: Vec<u8>);
    fn clear_wallet_locator(&mut self);

    fn set_message_key(&mut self, value: Vec<u8>);
    fn clear_message_key(&mut self);

    fn set_domain(&mut self, value: Vec<u8>);
    fn clear_domain(&mut self);

    fn set_transfer_rate(&mut self, value: u32);
    fn clear_transfer_rate(&mut self);

    fn set_tick_size(&mut self, value: u8);
    fn clear_tick_size(&mut self);

    fn set_nftoken_minter(&mut self, value: Self::AccountId);
    fn clear_nftoken_minter(&mut self);

    fn set_account_flags(&mut self, value: u32);
    fn update_account(&mut self);
}

pub trait AccountSetPermissionTx {
    type AccountId: Clone;

    fn account_id(&self) -> Self::AccountId;
    fn delegate(&self) -> Option<Self::AccountId>;
    fn set_flag(&self) -> u32;
    fn clear_flag(&self) -> u32;
    fn flags(&self) -> u32;
    fn email_hash_present(&self) -> bool;
    fn wallet_locator_present(&self) -> bool;
    fn nftoken_minter_present(&self) -> bool;
    fn message_key_present(&self) -> bool;
    fn domain_present(&self) -> bool;
    fn transfer_rate_present(&self) -> bool;
    fn tick_size_present(&self) -> bool;
}

pub const fn account_set_tx_consequences_category(
    tx_flags: u32,
    set_flag: u32,
    clear_flag: u32,
) -> TxConsequencesCategory {
    let blocker = (tx_flags & (ACCOUNT_SET_REQUIRE_AUTH_FLAG | ACCOUNT_SET_OPTIONAL_AUTH_FLAG)
        != 0)
        || matches!(
            set_flag,
            ASF_REQUIRE_AUTH | ASF_DISABLE_MASTER | ASF_ACCOUNT_TXN_ID
        )
        || matches!(
            clear_flag,
            ASF_REQUIRE_AUTH | ASF_DISABLE_MASTER | ASF_ACCOUNT_TXN_ID
        );

    if blocker {
        TxConsequencesCategory::Blocker
    } else {
        TxConsequencesCategory::Normal
    }
}

pub const fn account_set_tx_consequences_shape(
    tx_flags: u32,
    set_flag: u32,
    clear_flag: u32,
) -> TxConsequencesShape {
    match account_set_tx_consequences_category(tx_flags, set_flag, clear_flag) {
        TxConsequencesCategory::Normal => TxConsequencesShape::Normal,
        TxConsequencesCategory::Blocker => TxConsequencesShape::Blocker,
    }
}

pub fn run_account_set_make_tx_consequences(
    fee_drops: u64,
    seq_proxy: SeqProxy,
    tx_flags: u32,
    set_flag: u32,
    clear_flag: u32,
) -> TxConsequences {
    build_tx_consequences(
        fee_drops,
        seq_proxy,
        account_set_tx_consequences_shape(tx_flags, set_flag, clear_flag),
    )
}

pub const fn get_account_set_flags_mask() -> u32 {
    ACCOUNT_SET_FLAGS_MASK
}

pub fn run_account_set_preflight(facts: AccountSetPreflightFacts) -> NotTec {
    if facts.set_flag != 0 && facts.set_flag == facts.clear_flag {
        return Ter::TEM_INVALID_FLAG;
    }

    let set_require_auth =
        (facts.tx_flags & ACCOUNT_SET_REQUIRE_AUTH_FLAG != 0) || facts.set_flag == ASF_REQUIRE_AUTH;
    let clear_require_auth = (facts.tx_flags & ACCOUNT_SET_OPTIONAL_AUTH_FLAG != 0)
        || facts.clear_flag == ASF_REQUIRE_AUTH;
    if set_require_auth && clear_require_auth {
        return Ter::TEM_INVALID_FLAG;
    }

    let set_require_dest = (facts.tx_flags & ACCOUNT_SET_REQUIRE_DEST_TAG_FLAG != 0)
        || facts.set_flag == ASF_REQUIRE_DEST;
    let clear_require_dest = (facts.tx_flags & ACCOUNT_SET_OPTIONAL_DEST_TAG_FLAG != 0)
        || facts.clear_flag == ASF_REQUIRE_DEST;
    if set_require_dest && clear_require_dest {
        return Ter::TEM_INVALID_FLAG;
    }

    let set_disallow_xrp =
        (facts.tx_flags & ACCOUNT_SET_DISALLOW_XRP_FLAG != 0) || facts.set_flag == ASF_DISALLOW_XRP;
    let clear_disallow_xrp =
        (facts.tx_flags & ACCOUNT_SET_ALLOW_XRP_FLAG != 0) || facts.clear_flag == ASF_DISALLOW_XRP;
    if set_disallow_xrp && clear_disallow_xrp {
        return Ter::TEM_INVALID_FLAG;
    }

    if let Some(rate) = facts.transfer_rate {
        if rate != 0 && rate < facts.quality_one {
            return Ter::TEM_BAD_TRANSFER_RATE;
        }
        if rate > 2 * facts.quality_one {
            return Ter::TEM_BAD_TRANSFER_RATE;
        }
    }

    if let Some(tick_size) = facts.tick_size
        && tick_size != 0
        && (tick_size < facts.min_tick_size || tick_size > facts.max_tick_size)
    {
        return Ter::TEM_BAD_TICK_SIZE;
    }

    if facts.message_key_present && !facts.message_key_is_valid {
        return Ter::TEL_BAD_PUBLIC_KEY;
    }

    if let Some(domain_len) = facts.domain_len
        && domain_len > facts.max_domain_length
    {
        return Ter::TEL_BAD_DOMAIN;
    }

    if facts.set_flag == ASF_AUTHORIZED_NFTOKEN_MINTER && !facts.nftoken_minter_present {
        return Ter::TEM_MALFORMED;
    }

    if facts.clear_flag == ASF_AUTHORIZED_NFTOKEN_MINTER && facts.nftoken_minter_present {
        return Ter::TEM_MALFORMED;
    }

    Ter::TES_SUCCESS
}

pub fn run_account_set_preclaim(facts: AccountSetPreclaimFacts) -> Ter {
    if !facts.account_exists {
        return Ter::TER_NO_ACCOUNT;
    }

    let set_require_auth =
        (facts.tx_flags & ACCOUNT_SET_REQUIRE_AUTH_FLAG != 0) || facts.set_flag == ASF_REQUIRE_AUTH;

    if set_require_auth && (facts.account_flags & LSF_REQUIRE_AUTH == 0) && !facts.owner_dir_empty {
        return if facts.apply_flags.bits() & ApplyFlags::RETRY.bits() != 0 {
            Ter::TER_OWNERS
        } else {
            Ter::TEC_OWNERS
        };
    }

    if facts.feature_clawback_enabled {
        if facts.set_flag == ASF_ALLOW_TRUST_LINE_CLAWBACK {
            if facts.account_flags & LSF_NO_FREEZE != 0 {
                return Ter::TEC_NO_PERMISSION;
            }

            if !facts.owner_dir_empty {
                return Ter::TEC_OWNERS;
            }
        } else if facts.set_flag == ASF_NO_FREEZE
            && facts.account_flags & LSF_ALLOW_TRUST_LINE_CLAWBACK != 0
        {
            return Ter::TEC_NO_PERMISSION;
        }
    }

    Ter::TES_SUCCESS
}

pub fn run_account_set_do_apply_flags(
    facts: AccountSetDoApplyFlagFacts,
) -> Result<AccountSetDoApplyFlagState, Ter> {
    if !facts.account_exists {
        return Err(Ter::TEF_INTERNAL);
    }

    let mut account_flags = facts.account_flags;
    let mut account_txn_id_action = AccountSetTxnIdAction::None;

    let set_require_dest = (facts.tx_flags & ACCOUNT_SET_REQUIRE_DEST_TAG_FLAG != 0)
        || facts.set_flag == ASF_REQUIRE_DEST;
    let clear_require_dest = (facts.tx_flags & ACCOUNT_SET_OPTIONAL_DEST_TAG_FLAG != 0)
        || facts.clear_flag == ASF_REQUIRE_DEST;
    let set_require_auth =
        (facts.tx_flags & ACCOUNT_SET_REQUIRE_AUTH_FLAG != 0) || facts.set_flag == ASF_REQUIRE_AUTH;
    let clear_require_auth = (facts.tx_flags & ACCOUNT_SET_OPTIONAL_AUTH_FLAG != 0)
        || facts.clear_flag == ASF_REQUIRE_AUTH;
    let set_disallow_xrp =
        (facts.tx_flags & ACCOUNT_SET_DISALLOW_XRP_FLAG != 0) || facts.set_flag == ASF_DISALLOW_XRP;
    let clear_disallow_xrp =
        (facts.tx_flags & ACCOUNT_SET_ALLOW_XRP_FLAG != 0) || facts.clear_flag == ASF_DISALLOW_XRP;

    if set_require_auth && (facts.account_flags & LSF_REQUIRE_AUTH == 0) {
        account_flags |= LSF_REQUIRE_AUTH;
    }

    if clear_require_auth && (facts.account_flags & LSF_REQUIRE_AUTH != 0) {
        account_flags &= !LSF_REQUIRE_AUTH;
    }

    if set_require_dest && (facts.account_flags & LSF_REQUIRE_DEST_TAG == 0) {
        account_flags |= LSF_REQUIRE_DEST_TAG;
    }

    if clear_require_dest && (facts.account_flags & LSF_REQUIRE_DEST_TAG != 0) {
        account_flags &= !LSF_REQUIRE_DEST_TAG;
    }

    if set_disallow_xrp && (facts.account_flags & LSF_DISALLOW_XRP == 0) {
        account_flags |= LSF_DISALLOW_XRP;
    }

    if clear_disallow_xrp && (facts.account_flags & LSF_DISALLOW_XRP != 0) {
        account_flags &= !LSF_DISALLOW_XRP;
    }

    if facts.set_flag == ASF_DISABLE_MASTER && (facts.account_flags & LSF_DISABLE_MASTER == 0) {
        if !facts.signed_with_master {
            return Err(Ter::TEC_NEED_MASTER_KEY);
        }

        if !facts.has_regular_key && !facts.has_signer_list {
            return Err(Ter::TEC_NO_ALTERNATIVE_KEY);
        }

        account_flags |= LSF_DISABLE_MASTER;
    }

    if facts.clear_flag == ASF_DISABLE_MASTER && (facts.account_flags & LSF_DISABLE_MASTER != 0) {
        account_flags &= !LSF_DISABLE_MASTER;
    }

    if facts.set_flag == ASF_DEFAULT_RIPPLE {
        account_flags |= LSF_DEFAULT_RIPPLE;
    } else if facts.clear_flag == ASF_DEFAULT_RIPPLE {
        account_flags &= !LSF_DEFAULT_RIPPLE;
    }

    if facts.set_flag == ASF_NO_FREEZE {
        if !facts.signed_with_master && (facts.account_flags & LSF_DISABLE_MASTER == 0) {
            return Err(Ter::TEC_NEED_MASTER_KEY);
        }

        account_flags |= LSF_NO_FREEZE;
    }

    if facts.set_flag == ASF_GLOBAL_FREEZE {
        account_flags |= LSF_GLOBAL_FREEZE;
    }

    if facts.set_flag != ASF_GLOBAL_FREEZE
        && facts.clear_flag == ASF_GLOBAL_FREEZE
        && (account_flags & LSF_NO_FREEZE == 0)
    {
        account_flags &= !LSF_GLOBAL_FREEZE;
    }

    if facts.set_flag == ASF_ACCOUNT_TXN_ID && !facts.account_txn_id_present {
        account_txn_id_action = AccountSetTxnIdAction::Set;
    }

    if facts.clear_flag == ASF_ACCOUNT_TXN_ID && facts.account_txn_id_present {
        account_txn_id_action = AccountSetTxnIdAction::Clear;
    }

    if facts.set_flag == ASF_DEPOSIT_AUTH {
        account_flags |= LSF_DEPOSIT_AUTH;
    } else if facts.clear_flag == ASF_DEPOSIT_AUTH {
        account_flags &= !LSF_DEPOSIT_AUTH;
    }

    Ok(AccountSetDoApplyFlagState {
        account_flags,
        account_txn_id_action,
    })
}

pub fn run_account_set_do_apply_tail<AccountId: Clone>(
    facts: AccountSetDoApplyTailFacts<AccountId>,
) -> AccountSetDoApplyTailState<AccountId> {
    let mut state = AccountSetDoApplyTailState {
        account_flags: facts.account_flags,
        ..AccountSetDoApplyTailState::default()
    };

    if let Some(email_hash) = facts.email_hash {
        if email_hash == 0 {
            state.email_hash_action = AccountSetFieldMutation::Clear;
        } else {
            state.email_hash_action = AccountSetFieldMutation::Set(email_hash);
        }
    }

    if let Some(wallet_locator) = facts.wallet_locator {
        if wallet_locator.is_empty() {
            state.wallet_locator_action = AccountSetFieldMutation::Clear;
        } else {
            state.wallet_locator_action = AccountSetFieldMutation::Set(wallet_locator);
        }
    }

    if let Some(message_key) = facts.message_key {
        if message_key.is_empty() {
            state.message_key_action = AccountSetFieldMutation::Clear;
        } else {
            state.message_key_action = AccountSetFieldMutation::Set(message_key);
        }
    }

    if let Some(domain) = facts.domain {
        if domain.is_empty() {
            state.domain_action = AccountSetFieldMutation::Clear;
        } else {
            state.domain_action = AccountSetFieldMutation::Set(domain);
        }
    }

    if let Some(transfer_rate) = facts.transfer_rate {
        if transfer_rate == 0 || transfer_rate == facts.quality_one {
            state.transfer_rate_action = AccountSetFieldMutation::Clear;
        } else {
            state.transfer_rate_action = AccountSetFieldMutation::Set(transfer_rate);
        }
    }

    if let Some(tick_size) = facts.tick_size {
        if tick_size == 0 || tick_size == facts.max_tick_size {
            state.tick_size_action = AccountSetFieldMutation::Clear;
        } else {
            state.tick_size_action = AccountSetFieldMutation::Set(tick_size);
        }
    }

    if facts.set_flag == ASF_AUTHORIZED_NFTOKEN_MINTER
        && let Some(nftoken_minter) = facts.nftoken_minter
    {
        state.nftoken_minter_action = AccountSetFieldMutation::Set(nftoken_minter);
    }

    if facts.clear_flag == ASF_AUTHORIZED_NFTOKEN_MINTER && facts.nftoken_minter_present_on_account
    {
        state.nftoken_minter_action = AccountSetFieldMutation::Clear;
    }

    if facts.set_flag == ASF_DISALLOW_INCOMING_NFTOKEN_OFFER {
        state.account_flags |= LSF_DISALLOW_INCOMING_NFTOKEN_OFFER;
    } else if facts.clear_flag == ASF_DISALLOW_INCOMING_NFTOKEN_OFFER {
        state.account_flags &= !LSF_DISALLOW_INCOMING_NFTOKEN_OFFER;
    }

    if facts.set_flag == ASF_DISALLOW_INCOMING_CHECK {
        state.account_flags |= LSF_DISALLOW_INCOMING_CHECK;
    } else if facts.clear_flag == ASF_DISALLOW_INCOMING_CHECK {
        state.account_flags &= !LSF_DISALLOW_INCOMING_CHECK;
    }

    if facts.set_flag == ASF_DISALLOW_INCOMING_PAY_CHAN {
        state.account_flags |= LSF_DISALLOW_INCOMING_PAY_CHAN;
    } else if facts.clear_flag == ASF_DISALLOW_INCOMING_PAY_CHAN {
        state.account_flags &= !LSF_DISALLOW_INCOMING_PAY_CHAN;
    }

    if facts.set_flag == ASF_DISALLOW_INCOMING_TRUSTLINE {
        state.account_flags |= LSF_DISALLOW_INCOMING_TRUSTLINE;
    } else if facts.clear_flag == ASF_DISALLOW_INCOMING_TRUSTLINE {
        state.account_flags &= !LSF_DISALLOW_INCOMING_TRUSTLINE;
    }

    if facts.feature_token_escrow_enabled {
        if facts.set_flag == ASF_ALLOW_TRUST_LINE_LOCKING {
            state.account_flags |= LSF_ALLOW_TRUST_LINE_LOCKING;
        } else if facts.clear_flag == ASF_ALLOW_TRUST_LINE_LOCKING {
            state.account_flags &= !LSF_ALLOW_TRUST_LINE_LOCKING;
        }
    }

    if facts.feature_clawback_enabled && facts.set_flag == ASF_ALLOW_TRUST_LINE_CLAWBACK {
        state.account_flags |= LSF_ALLOW_TRUST_LINE_CLAWBACK;
    }

    state
}

pub fn run_account_set_do_apply<Sink>(
    sink: &mut Sink,
    facts: AccountSetDoApplyFacts<Sink::AccountId>,
) -> Ter
where
    Sink: AccountSetDoApplySink,
{
    let AccountSetDoApplyFacts {
        flag_facts,
        mut tail_facts,
    } = facts;

    let flag_state = match run_account_set_do_apply_flags(flag_facts) {
        Ok(state) => state,
        Err(err) => return err,
    };

    match flag_state.account_txn_id_action {
        AccountSetTxnIdAction::None => {}
        AccountSetTxnIdAction::Set => sink.set_account_txn_id(),
        AccountSetTxnIdAction::Clear => sink.clear_account_txn_id(),
    }

    tail_facts.account_flags = flag_state.account_flags;
    let tail_state = run_account_set_do_apply_tail(tail_facts);

    match tail_state.email_hash_action {
        AccountSetFieldMutation::NoChange => {}
        AccountSetFieldMutation::Clear => sink.clear_email_hash(),
        AccountSetFieldMutation::Set(value) => sink.set_email_hash(value),
    }

    match tail_state.wallet_locator_action {
        AccountSetFieldMutation::NoChange => {}
        AccountSetFieldMutation::Clear => sink.clear_wallet_locator(),
        AccountSetFieldMutation::Set(value) => sink.set_wallet_locator(value),
    }

    match tail_state.message_key_action {
        AccountSetFieldMutation::NoChange => {}
        AccountSetFieldMutation::Clear => sink.clear_message_key(),
        AccountSetFieldMutation::Set(value) => sink.set_message_key(value),
    }

    match tail_state.domain_action {
        AccountSetFieldMutation::NoChange => {}
        AccountSetFieldMutation::Clear => sink.clear_domain(),
        AccountSetFieldMutation::Set(value) => sink.set_domain(value),
    }

    match tail_state.transfer_rate_action {
        AccountSetFieldMutation::NoChange => {}
        AccountSetFieldMutation::Clear => sink.clear_transfer_rate(),
        AccountSetFieldMutation::Set(value) => sink.set_transfer_rate(value),
    }

    match tail_state.tick_size_action {
        AccountSetFieldMutation::NoChange => {}
        AccountSetFieldMutation::Clear => sink.clear_tick_size(),
        AccountSetFieldMutation::Set(value) => sink.set_tick_size(value),
    }

    match tail_state.nftoken_minter_action {
        AccountSetFieldMutation::NoChange => {}
        AccountSetFieldMutation::Clear => sink.clear_nftoken_minter(),
        AccountSetFieldMutation::Set(value) => sink.set_nftoken_minter(value),
    }

    if flag_facts.account_flags != tail_state.account_flags {
        sink.set_account_flags(tail_state.account_flags);
    }

    sink.update_account();
    Ter::TES_SUCCESS
}

pub fn run_account_set_check_permission<Tx, DelegateState, ReadDelegate, HasPermission>(
    tx: &Tx,
    read_delegate: ReadDelegate,
    mut has_permission: HasPermission,
) -> NotTec
where
    Tx: AccountSetPermissionTx,
    ReadDelegate: FnOnce(&Tx::AccountId, &Tx::AccountId) -> Option<DelegateState>,
    HasPermission: FnMut(&DelegateState, AccountSetGranularPermission) -> bool,
{
    let Some(delegate) = tx.delegate() else {
        return Ter::TES_SUCCESS;
    };

    let Some(delegate_state) = read_delegate(&tx.account_id(), &delegate) else {
        return Ter::TER_NO_DELEGATE_PERMISSION;
    };

    if tx.set_flag() != 0 || tx.clear_flag() != 0 || (tx.flags() & UNIVERSAL_FLAGS_MASK) != 0 {
        return Ter::TER_NO_DELEGATE_PERMISSION;
    }

    if tx.email_hash_present()
        && !has_permission(&delegate_state, AccountSetGranularPermission::EmailHashSet)
    {
        return Ter::TER_NO_DELEGATE_PERMISSION;
    }

    if tx.wallet_locator_present() || tx.nftoken_minter_present() {
        return Ter::TER_NO_DELEGATE_PERMISSION;
    }

    if tx.message_key_present()
        && !has_permission(&delegate_state, AccountSetGranularPermission::MessageKeySet)
    {
        return Ter::TER_NO_DELEGATE_PERMISSION;
    }

    if tx.domain_present()
        && !has_permission(&delegate_state, AccountSetGranularPermission::DomainSet)
    {
        return Ter::TER_NO_DELEGATE_PERMISSION;
    }

    if tx.transfer_rate_present()
        && !has_permission(
            &delegate_state,
            AccountSetGranularPermission::TransferRateSet,
        )
    {
        return Ter::TER_NO_DELEGATE_PERMISSION;
    }

    if tx.tick_size_present()
        && !has_permission(&delegate_state, AccountSetGranularPermission::TickSizeSet)
    {
        return Ter::TER_NO_DELEGATE_PERMISSION;
    }

    Ter::TES_SUCCESS
}
