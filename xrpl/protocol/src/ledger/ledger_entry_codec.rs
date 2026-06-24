//! Typed ledger-entry serialization and decode helpers for ported protocol
//! entry types.
//!
//! This module covers the serialized state objects used by the current
//! genesis/setup and ledger-entry parity callers and exposes a shared typed
//! enum over those entry shapes.

use crate::feature_xrp_fees;
use crate::keylet::{LedgerEntryType, account_keylet, amendments_keylet, fee_settings_keylet};
use basics::base_uint::{Uint160, Uint256};

const STI_UINT16: u8 = 1;
const STI_UINT32: u8 = 2;
const STI_UINT64: u8 = 3;
const STI_UINT128: u8 = 4;
const STI_UINT256: u8 = 5;
const STI_AMOUNT: u8 = 6;
const STI_VL: u8 = 7;
const STI_ACCOUNT: u8 = 8;
const STI_NUMBER: u8 = 9;
const STI_INT32: u8 = 10;
const STI_INT64: u8 = 11;
const STI_OBJECT: u8 = 14;
const STI_ARRAY: u8 = 15;
const STI_UINT8: u8 = 16;
const STI_UINT160: u8 = 17;
const STI_PATHSET: u8 = 18;
const STI_VECTOR256: u8 = 19;
const STI_UINT96: u8 = 20;
const STI_UINT192: u8 = 21;
const STI_UINT384: u8 = 22;
const STI_UINT512: u8 = 23;
const STI_ISSUE: u8 = 24;
const STI_XCHAIN_BRIDGE: u8 = 25;
const STI_CURRENCY: u8 = 26;

const SF_LEDGER_ENTRY_TYPE: u8 = 1;
const SF_BALANCE: u8 = 2;
const SF_AMENDMENTS: u8 = 3;
const SF_FLAGS: u8 = 2;
const SF_SEQUENCE: u8 = 4;
const SF_BASE_FEE: u8 = 5;
const SF_CLOSE_TIME: u8 = 7;
const SF_ACCOUNT_TXN_ID: u8 = 9;
const SF_TRANSFER_RATE: u8 = 11;
const SF_OWNER_COUNT: u8 = 13;
const SF_TICK_SIZE: u8 = 16;
const SF_ACCOUNT: u8 = 1;
const SF_MAJORITIES: u8 = 16;
const SF_MAJORITY: u8 = 18;
const SF_MAJORITY_AMENDMENT: u8 = 19;
const SF_BASE_FEE_DROPS: u8 = 22;
const SF_RESERVE_BASE_DROPS: u8 = 23;
const SF_RESERVE_INCREMENT_DROPS: u8 = 24;
const SF_REFERENCE_FEE_UNITS: u8 = 30;
const SF_RESERVE_BASE: u8 = 31;
const SF_RESERVE_INCREMENT: u8 = 32;
const SF_PREVIOUS_TXN_ID: u8 = 5;
const SF_PREVIOUS_TXN_LGR_SEQ: u8 = 5;
const SF_HASHES: u8 = 2;
const SF_LAST_LEDGER_SEQUENCE: u8 = 27;
const SF_PUBLIC_KEY: u8 = 1;
const SF_FIRST_LEDGER_SEQUENCE: u8 = 26;
const SF_VALIDATOR_TO_DISABLE: u8 = 20;
const SF_VALIDATOR_TO_RE_ENABLE: u8 = 21;
const SF_DISABLED_VALIDATOR: u8 = 19;
const SF_DISABLED_VALIDATORS: u8 = 17;

const LT_ACCOUNT_ROOT: u16 = 0x0061;
const LT_AMENDMENTS: u16 = 0x0066;
const LT_LEDGER_HASHES: u16 = 0x0068;
const LT_NEGATIVE_UNL: u16 = 0x004E;
const LT_FEE_SETTINGS: u16 = 0x0073;

const OBJECT_END: u8 = 0xE1;
const ARRAY_END: u8 = 0xF1;
const STAMOUNT_ISSUED_CURRENCY: u64 = 0x8000_0000_0000_0000;
const STAMOUNT_POSITIVE: u64 = 0x4000_0000_0000_0000;
const STAMOUNT_MPTOKEN: u64 = 0x2000_0000_0000_0000;
const STAMOUNT_VALUE_MASK: u64 = !(STAMOUNT_POSITIVE | STAMOUNT_MPTOKEN);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DecodedAmountField {
    pub drops: u64,
    pub native: bool,
    pub negative: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DecodedFeeSettingsEntry {
    pub base_fee: Option<u64>,
    pub reference_fee_units: Option<u32>,
    pub reserve_base: Option<u32>,
    pub reserve_increment: Option<u32>,
    pub base_fee_drops: Option<DecodedAmountField>,
    pub reserve_base_drops: Option<DecodedAmountField>,
    pub reserve_increment_drops: Option<DecodedAmountField>,
    pub previous_txn_id: Option<Uint256>,
    pub previous_txn_lgr_seq: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DecodedAmendmentsEntry {
    pub amendments: Vec<Uint256>,
    pub majorities: Vec<DecodedMajorityEntry>,
    pub previous_txn_id: Option<Uint256>,
    pub previous_txn_lgr_seq: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DecodedMajorityEntry {
    pub close_time: u32,
    pub amendment: Uint256,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DecodedLedgerHashesEntry {
    pub hashes: Vec<Uint256>,
    pub last_ledger_sequence: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodedDisabledValidator {
    pub public_key: Vec<u8>,
    pub first_ledger_sequence: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DecodedNegativeUnlEntry {
    pub disabled_validators: Vec<DecodedDisabledValidator>,
    pub validator_to_disable: Option<Vec<u8>>,
    pub validator_to_re_enable: Option<Vec<u8>>,
    pub previous_txn_id: Option<Uint256>,
    pub previous_txn_lgr_seq: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DecodedLedgerEntry {
    AccountRoot(DecodedAccountRootEntry),
    Amendments(DecodedAmendmentsEntry),
    FeeSettings(DecodedFeeSettingsEntry),
    LedgerHashes(DecodedLedgerHashesEntry),
    NegativeUnl(DecodedNegativeUnlEntry),
}

impl DecodedLedgerEntry {
    pub const fn entry_type(&self) -> LedgerEntryType {
        match self {
            Self::AccountRoot(_) => LedgerEntryType::AccountRoot,
            Self::Amendments(_) => LedgerEntryType::Amendments,
            Self::FeeSettings(_) => LedgerEntryType::FeeSettings,
            Self::LedgerHashes(_) => LedgerEntryType::LedgerHashes,
            Self::NegativeUnl(_) => LedgerEntryType::NegativeUnl,
        }
    }
}

/// Fixed amendments field shape used by the current constructor/setup callers
/// we have ported so far.
///
/// This stays narrower than a general `Amendments` / `SLE` port. It preserves
/// the the reference implementation rule that the field may be absent, in which case the typed
/// shape simply carries an empty amendment list.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ConstructorAmendmentsEntry {
    pub amendments: Vec<Uint256>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DecodedAccountRootEntry {
    pub flags: Option<u32>,
    pub sequence: Option<u32>,
    pub owner_count: Option<u32>,
    pub account_txn_id: Option<Uint256>,
    pub previous_txn_id: Option<Uint256>,
    pub previous_txn_lgr_seq: Option<u32>,
    pub transfer_rate: Option<u32>,
    pub balance: Option<DecodedAmountField>,
    pub tick_size: Option<u8>,
    pub account_id: Option<Uint160>,
}

/// Fixed `AccountRoot` field set written by the current constructor-time
/// callers we have ported so far.
///
/// This is intentionally narrower than a general `AccountRoot` / `SLE` port.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConstructorAccountRootEntry {
    pub sequence: u32,
    pub balance_drops: u64,
    pub account_id: Uint160,
}

/// Fixed fee-settings shapes written by the current constructor-time callers we
/// have ported so far.
///
/// This is intentionally narrower than a general fee-settings / `SLE` port.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConstructorFeeSettingsEntry {
    Legacy {
        base_fee: u64,
        reference_fee_units: u32,
        reserve_base: Option<u32>,
        reserve_increment: Option<u32>,
    },
    XrpDrops {
        base_fee_drops: u64,
        reserve_base_drops: u64,
        reserve_increment_drops: u64,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConstructorLedgerEntry {
    AccountRoot(ConstructorAccountRootEntry),
    Amendments(ConstructorAmendmentsEntry),
    FeeSettings(ConstructorFeeSettingsEntry),
}

impl ConstructorLedgerEntry {
    pub const fn entry_type(&self) -> LedgerEntryType {
        match self {
            Self::AccountRoot(_) => LedgerEntryType::AccountRoot,
            Self::Amendments(_) => LedgerEntryType::Amendments,
            Self::FeeSettings(_) => LedgerEntryType::FeeSettings,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LedgerEntryDecodeError {
    MissingLedgerEntryType,
    UnexpectedLedgerEntryType { expected: u16, actual: u16 },
    MissingField(&'static str),
    TruncatedField,
    InvalidVariableLengthPrefix(u8),
    InvalidVector256Length(usize),
    UnsupportedFieldType(u8),
    DuplicateField(&'static str),
    UnexpectedObjectEnd,
    UnexpectedArrayEnd,
    MissingObjectEnd,
    MissingArrayEnd,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConstructorAccountRootDecodeError {
    LedgerEntry(LedgerEntryDecodeError),
    MissingField(&'static str),
    NonNativeBalance,
    NegativeBalance,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConstructorFeeSettingsDecodeError {
    LedgerEntry(LedgerEntryDecodeError),
    MissingField(&'static str),
    MixedFeeFormats,
    NonNativeAmount(&'static str),
    NegativeAmount(&'static str),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PortedLedgerEntryDecodeError {
    LedgerEntry(LedgerEntryDecodeError),
    UnsupportedLedgerEntryType(u16),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConstructorLedgerEntryDecodeError {
    LedgerEntry(LedgerEntryDecodeError),
    AccountRoot(ConstructorAccountRootDecodeError),
    FeeSettings(ConstructorFeeSettingsDecodeError),
    UnsupportedLedgerEntryType(u16),
}

pub const REFERENCE_FEE_UNITS_DEPRECATED: u32 = 10;

pub fn encode_account_root_entry(
    account_id: Uint160,
    sequence: u32,
    balance_drops: u64,
) -> Vec<u8> {
    let mut bytes = Vec::new();
    append_u16_field(&mut bytes, SF_LEDGER_ENTRY_TYPE, LT_ACCOUNT_ROOT);
    append_u32_field(&mut bytes, SF_FLAGS, 0);
    append_u32_field(&mut bytes, SF_SEQUENCE, sequence);
    append_u32_field(&mut bytes, SF_PREVIOUS_TXN_LGR_SEQ, 0);
    append_u32_field(&mut bytes, SF_OWNER_COUNT, 0);
    append_u256_field(&mut bytes, SF_PREVIOUS_TXN_ID, Uint256::default());
    append_native_amount_field(&mut bytes, SF_BALANCE, balance_drops);
    append_account_field(&mut bytes, SF_ACCOUNT, account_id);
    bytes
}

pub fn encode_constructor_account_root_entry(entry: ConstructorAccountRootEntry) -> Vec<u8> {
    encode_account_root_entry(entry.account_id, entry.sequence, entry.balance_drops)
}

pub fn encode_constructor_ledger_entry(entry: &ConstructorLedgerEntry) -> Vec<u8> {
    match entry {
        ConstructorLedgerEntry::AccountRoot(entry) => encode_constructor_account_root_entry(*entry),
        ConstructorLedgerEntry::Amendments(entry) => encode_constructor_amendments_entry(entry),
        ConstructorLedgerEntry::FeeSettings(entry) => encode_constructor_fee_settings_entry(*entry),
    }
}

pub fn constructor_ledger_entry_key(entry: &ConstructorLedgerEntry) -> Uint256 {
    match entry {
        ConstructorLedgerEntry::AccountRoot(entry) => account_keylet(entry.account_id).key,
        ConstructorLedgerEntry::Amendments(_) => amendments_keylet().key,
        ConstructorLedgerEntry::FeeSettings(_) => fee_settings_keylet().key,
    }
}

pub fn constructor_ledger_item(entry: &ConstructorLedgerEntry) -> (Uint256, Vec<u8>) {
    (
        constructor_ledger_entry_key(entry),
        encode_constructor_ledger_entry(entry),
    )
}

pub fn constructor_ledger_items(entries: &[ConstructorLedgerEntry]) -> Vec<(Uint256, Vec<u8>)> {
    entries.iter().map(constructor_ledger_item).collect()
}

pub fn encode_amendments_entry(amendments: &[Uint256]) -> Vec<u8> {
    let mut bytes = Vec::new();
    append_u16_field(&mut bytes, SF_LEDGER_ENTRY_TYPE, LT_AMENDMENTS);
    append_u32_field(&mut bytes, SF_FLAGS, 0);
    append_vector256_field(&mut bytes, SF_AMENDMENTS, amendments);
    bytes
}

pub fn encode_ledger_hashes_entry(entry: &DecodedLedgerHashesEntry) -> Vec<u8> {
    let mut bytes = Vec::new();
    append_u16_field(&mut bytes, SF_LEDGER_ENTRY_TYPE, LT_LEDGER_HASHES);
    append_u32_field(&mut bytes, SF_FLAGS, 0);
    if let Some(last_ledger_sequence) = entry.last_ledger_sequence {
        append_u32_field(&mut bytes, SF_LAST_LEDGER_SEQUENCE, last_ledger_sequence);
    }
    append_vector256_field(&mut bytes, SF_HASHES, &entry.hashes);
    bytes
}

pub fn encode_negative_unl_entry(entry: &DecodedNegativeUnlEntry) -> Vec<u8> {
    let mut bytes = Vec::new();
    append_u16_field(&mut bytes, SF_LEDGER_ENTRY_TYPE, LT_NEGATIVE_UNL);
    append_u32_field(&mut bytes, SF_FLAGS, 0);

    if !entry.disabled_validators.is_empty() {
        append_field_id(&mut bytes, STI_ARRAY, SF_DISABLED_VALIDATORS);
        for validator in &entry.disabled_validators {
            append_field_id(&mut bytes, STI_OBJECT, SF_DISABLED_VALIDATOR);
            append_vl_field(&mut bytes, SF_PUBLIC_KEY, &validator.public_key);
            append_u32_field(
                &mut bytes,
                SF_FIRST_LEDGER_SEQUENCE,
                validator.first_ledger_sequence,
            );
            bytes.push(OBJECT_END);
        }
        bytes.push(ARRAY_END);
    }

    if let Some(validator) = &entry.validator_to_disable {
        append_vl_field(&mut bytes, SF_VALIDATOR_TO_DISABLE, validator);
    }
    if let Some(validator) = &entry.validator_to_re_enable {
        append_vl_field(&mut bytes, SF_VALIDATOR_TO_RE_ENABLE, validator);
    }
    if let Some(previous_txn_id) = entry.previous_txn_id {
        append_u256_field(&mut bytes, SF_PREVIOUS_TXN_ID, previous_txn_id);
    }
    if let Some(previous_txn_lgr_seq) = entry.previous_txn_lgr_seq {
        append_u32_field(&mut bytes, SF_PREVIOUS_TXN_LGR_SEQ, previous_txn_lgr_seq);
    }

    bytes
}

pub fn encode_constructor_amendments_entry(entry: &ConstructorAmendmentsEntry) -> Vec<u8> {
    encode_amendments_entry(&entry.amendments)
}

pub fn encode_constructor_fee_settings_entry(entry: ConstructorFeeSettingsEntry) -> Vec<u8> {
    let mut bytes = Vec::new();
    append_u16_field(&mut bytes, SF_LEDGER_ENTRY_TYPE, LT_FEE_SETTINGS);
    append_u32_field(&mut bytes, SF_FLAGS, 0);

    match entry {
        ConstructorFeeSettingsEntry::Legacy {
            base_fee,
            reference_fee_units,
            reserve_base,
            reserve_increment,
        } => {
            // Canonical order: UINT32 fields (type 2) before UINT64 fields (type 6)
            append_u32_field(&mut bytes, SF_REFERENCE_FEE_UNITS, reference_fee_units);
            if let Some(reserve_base) = reserve_base {
                append_u32_field(&mut bytes, SF_RESERVE_BASE, reserve_base);
            }
            if let Some(reserve_increment) = reserve_increment {
                append_u32_field(&mut bytes, SF_RESERVE_INCREMENT, reserve_increment);
            }
            append_u64_field(&mut bytes, SF_BASE_FEE, base_fee);
        }
        ConstructorFeeSettingsEntry::XrpDrops {
            base_fee_drops,
            reserve_base_drops,
            reserve_increment_drops,
        } => {
            append_native_amount_field(&mut bytes, SF_BASE_FEE_DROPS, base_fee_drops);
            append_native_amount_field(&mut bytes, SF_RESERVE_BASE_DROPS, reserve_base_drops);
            append_native_amount_field(
                &mut bytes,
                SF_RESERVE_INCREMENT_DROPS,
                reserve_increment_drops,
            );
        }
    }

    bytes
}

pub fn make_constructor_fee_settings_entry(
    base_drops: u64,
    reserve_drops: u64,
    increment_drops: u64,
    _amendments: &[Uint256],
) -> ConstructorFeeSettingsEntry {
    // Rippled quirk: Rippled ALWAYS builds the genesis FeeSettings entry in the
    // Legacy format, even if the XRPFees amendment is enabled at genesis!
    // It only upgrades to XrpDrops format later when the amendment is processed.
    // To ensure byte-for-byte genesis ledger parity, we must do the same.
    ConstructorFeeSettingsEntry::Legacy {
        base_fee: base_drops,
        reference_fee_units: REFERENCE_FEE_UNITS_DEPRECATED,
        reserve_base: u32::try_from(reserve_drops).ok(),
        reserve_increment: u32::try_from(increment_drops).ok(),
    }
}

pub fn build_genesis_setup_constructor_entries(
    base_drops: u64,
    reserve_drops: u64,
    increment_drops: u64,
    amendments: &[Uint256],
) -> Vec<ConstructorLedgerEntry> {
    let mut entries = Vec::with_capacity(1 + usize::from(!amendments.is_empty()));

    if !amendments.is_empty() {
        entries.push(ConstructorLedgerEntry::Amendments(
            ConstructorAmendmentsEntry {
                amendments: amendments.to_vec(),
            },
        ));
    }

    entries.push(ConstructorLedgerEntry::FeeSettings(
        make_constructor_fee_settings_entry(base_drops, reserve_drops, increment_drops, amendments),
    ));

    entries
}

pub fn build_genesis_state_constructor_entries(
    total_drops: u64,
    base_drops: u64,
    reserve_drops: u64,
    increment_drops: u64,
    amendments: &[Uint256],
) -> Vec<ConstructorLedgerEntry> {
    let mut entries = Vec::with_capacity(2 + usize::from(!amendments.is_empty()));
    entries.push(ConstructorLedgerEntry::AccountRoot(
        ConstructorAccountRootEntry {
            sequence: 1,
            balance_drops: total_drops,
            account_id: crate::genesis_account_id(),
        },
    ));
    entries.extend(build_genesis_setup_constructor_entries(
        base_drops,
        reserve_drops,
        increment_drops,
        amendments,
    ));
    entries
}

pub fn encode_fee_settings_entry(
    base_drops: u64,
    reserve_drops: u64,
    increment_drops: u64,
    xrp_fees_enabled: bool,
) -> Vec<u8> {
    if xrp_fees_enabled {
        return encode_constructor_fee_settings_entry(ConstructorFeeSettingsEntry::XrpDrops {
            base_fee_drops: base_drops,
            reserve_base_drops: reserve_drops,
            reserve_increment_drops: increment_drops,
        });
    }

    encode_constructor_fee_settings_entry(ConstructorFeeSettingsEntry::Legacy {
        base_fee: base_drops,
        reference_fee_units: REFERENCE_FEE_UNITS_DEPRECATED,
        reserve_base: u32::try_from(reserve_drops).ok(),
        reserve_increment: u32::try_from(increment_drops).ok(),
    })
}

pub fn decode_amendments_entry(
    payload: &[u8],
) -> Result<DecodedAmendmentsEntry, LedgerEntryDecodeError> {
    let mut cursor = Cursor::new(payload);
    let mut entry_type = None;
    let mut amendments = None;
    let mut majorities = None;
    let mut previous_txn_id = None;
    let mut previous_txn_lgr_seq = None;

    parse_object(&mut cursor, |field_type, field_name, cursor| {
        match (field_type, field_name) {
            (STI_UINT16, SF_LEDGER_ENTRY_TYPE) => set_once(
                &mut entry_type,
                Some(cursor.read_u16()?),
                "sfLedgerEntryType",
            ),
            (STI_VECTOR256, SF_AMENDMENTS) => set_once(
                &mut amendments,
                Some(read_vector256(cursor)?),
                "sfAmendments",
            ),
            (STI_ARRAY, SF_MAJORITIES) => set_once(
                &mut majorities,
                Some(read_majorities_array(cursor)?),
                "sfMajorities",
            ),
            (STI_UINT256, SF_PREVIOUS_TXN_ID) => set_once(
                &mut previous_txn_id,
                Some(cursor.read_u256()?),
                "sfPreviousTxnID",
            ),
            (STI_UINT32, SF_PREVIOUS_TXN_LGR_SEQ) => set_once(
                &mut previous_txn_lgr_seq,
                Some(cursor.read_u32()?),
                "sfPreviousTxnLgrSeq",
            ),
            (STI_OBJECT, SF_MAJORITY) | (STI_UINT32, SF_CLOSE_TIME) => {
                skip_field_value(field_type, cursor)
            }
            _ => skip_field_value(field_type, cursor),
        }
    })?;

    expect_entry_type(entry_type, LT_AMENDMENTS)?;

    Ok(DecodedAmendmentsEntry {
        amendments: amendments.unwrap_or_default(),
        majorities: majorities.unwrap_or_default(),
        previous_txn_id,
        previous_txn_lgr_seq,
    })
}

pub fn decode_ledger_entry_type_code(payload: &[u8]) -> Result<u16, LedgerEntryDecodeError> {
    let mut cursor = Cursor::new(payload);
    let mut entry_type = None;

    parse_object(&mut cursor, |field_type, field_name, cursor| {
        match (field_type, field_name) {
            (STI_UINT16, SF_LEDGER_ENTRY_TYPE) => set_once(
                &mut entry_type,
                Some(cursor.read_u16()?),
                "sfLedgerEntryType",
            ),
            _ => skip_field_value(field_type, cursor),
        }
    })?;

    entry_type.ok_or(LedgerEntryDecodeError::MissingLedgerEntryType)
}

pub fn decode_ported_ledger_entry(
    payload: &[u8],
) -> Result<DecodedLedgerEntry, PortedLedgerEntryDecodeError> {
    match decode_ledger_entry_type_code(payload)
        .map_err(PortedLedgerEntryDecodeError::LedgerEntry)?
    {
        LT_ACCOUNT_ROOT => decode_account_root_entry(payload)
            .map(DecodedLedgerEntry::AccountRoot)
            .map_err(PortedLedgerEntryDecodeError::LedgerEntry),
        LT_AMENDMENTS => decode_amendments_entry(payload)
            .map(DecodedLedgerEntry::Amendments)
            .map_err(PortedLedgerEntryDecodeError::LedgerEntry),
        LT_FEE_SETTINGS => decode_fee_settings_entry(payload)
            .map(DecodedLedgerEntry::FeeSettings)
            .map_err(PortedLedgerEntryDecodeError::LedgerEntry),
        LT_LEDGER_HASHES => decode_ledger_hashes_entry(payload)
            .map(DecodedLedgerEntry::LedgerHashes)
            .map_err(PortedLedgerEntryDecodeError::LedgerEntry),
        LT_NEGATIVE_UNL => decode_negative_unl_entry(payload)
            .map(DecodedLedgerEntry::NegativeUnl)
            .map_err(PortedLedgerEntryDecodeError::LedgerEntry),
        other => Err(PortedLedgerEntryDecodeError::UnsupportedLedgerEntryType(
            other,
        )),
    }
}

pub fn decode_ledger_hashes_entry(
    payload: &[u8],
) -> Result<DecodedLedgerHashesEntry, LedgerEntryDecodeError> {
    let mut cursor = Cursor::new(payload);
    let mut entry_type = None;
    let mut hashes = None;
    let mut last_ledger_sequence = None;

    parse_object(&mut cursor, |field_type, field_name, cursor| {
        match (field_type, field_name) {
            (STI_UINT16, SF_LEDGER_ENTRY_TYPE) => set_once(
                &mut entry_type,
                Some(cursor.read_u16()?),
                "sfLedgerEntryType",
            ),
            (STI_UINT32, SF_LAST_LEDGER_SEQUENCE) => set_once(
                &mut last_ledger_sequence,
                Some(cursor.read_u32()?),
                "sfLastLedgerSequence",
            ),
            (STI_VECTOR256, SF_HASHES) => {
                set_once(&mut hashes, Some(read_vector256(cursor)?), "sfHashes")
            }
            _ => skip_field_value(field_type, cursor),
        }
    })?;

    expect_entry_type(entry_type, LT_LEDGER_HASHES)?;

    Ok(DecodedLedgerHashesEntry {
        hashes: hashes.ok_or(LedgerEntryDecodeError::MissingField("sfHashes"))?,
        last_ledger_sequence,
    })
}

pub fn decode_negative_unl_entry(
    payload: &[u8],
) -> Result<DecodedNegativeUnlEntry, LedgerEntryDecodeError> {
    let mut cursor = Cursor::new(payload);
    let mut entry_type = None;
    let mut disabled_validators = None;
    let mut validator_to_disable = None;
    let mut validator_to_re_enable = None;
    let mut previous_txn_id = None;
    let mut previous_txn_lgr_seq = None;

    parse_object(&mut cursor, |field_type, field_name, cursor| {
        match (field_type, field_name) {
            (STI_UINT16, SF_LEDGER_ENTRY_TYPE) => set_once(
                &mut entry_type,
                Some(cursor.read_u16()?),
                "sfLedgerEntryType",
            ),
            (STI_ARRAY, SF_DISABLED_VALIDATORS) => set_once(
                &mut disabled_validators,
                Some(read_disabled_validators_array(cursor)?),
                "sfDisabledValidators",
            ),
            (STI_VL, SF_VALIDATOR_TO_DISABLE) => set_once(
                &mut validator_to_disable,
                Some(read_variable_length_bytes(cursor)?.to_vec()),
                "sfValidatorToDisable",
            ),
            (STI_VL, SF_VALIDATOR_TO_RE_ENABLE) => set_once(
                &mut validator_to_re_enable,
                Some(read_variable_length_bytes(cursor)?.to_vec()),
                "sfValidatorToReEnable",
            ),
            (STI_UINT256, SF_PREVIOUS_TXN_ID) => set_once(
                &mut previous_txn_id,
                Some(cursor.read_u256()?),
                "sfPreviousTxnID",
            ),
            (STI_UINT32, SF_PREVIOUS_TXN_LGR_SEQ) => set_once(
                &mut previous_txn_lgr_seq,
                Some(cursor.read_u32()?),
                "sfPreviousTxnLgrSeq",
            ),
            _ => skip_field_value(field_type, cursor),
        }
    })?;

    expect_entry_type(entry_type, LT_NEGATIVE_UNL)?;

    Ok(DecodedNegativeUnlEntry {
        disabled_validators: disabled_validators.unwrap_or_default(),
        validator_to_disable,
        validator_to_re_enable,
        previous_txn_id,
        previous_txn_lgr_seq,
    })
}

pub fn decode_account_root_entry(
    payload: &[u8],
) -> Result<DecodedAccountRootEntry, LedgerEntryDecodeError> {
    let mut cursor = Cursor::new(payload);
    let mut entry_type = None;
    let mut flags = None;
    let mut sequence = None;
    let mut owner_count = None;
    let mut account_txn_id = None;
    let mut previous_txn_id = None;
    let mut previous_txn_lgr_seq = None;
    let mut transfer_rate = None;
    let mut balance = None;
    let mut tick_size = None;
    let mut account_id = None;

    parse_object(&mut cursor, |field_type, field_name, cursor| {
        match (field_type, field_name) {
            (STI_UINT16, SF_LEDGER_ENTRY_TYPE) => set_once(
                &mut entry_type,
                Some(cursor.read_u16()?),
                "sfLedgerEntryType",
            ),
            (STI_UINT32, SF_FLAGS) => set_once(&mut flags, Some(cursor.read_u32()?), "sfFlags"),
            (STI_UINT32, SF_SEQUENCE) => {
                set_once(&mut sequence, Some(cursor.read_u32()?), "sfSequence")
            }
            (STI_UINT32, SF_OWNER_COUNT) => {
                set_once(&mut owner_count, Some(cursor.read_u32()?), "sfOwnerCount")
            }
            (STI_UINT256, SF_ACCOUNT_TXN_ID) => set_once(
                &mut account_txn_id,
                Some(cursor.read_u256()?),
                "sfAccountTxnID",
            ),
            (STI_UINT256, SF_PREVIOUS_TXN_ID) => set_once(
                &mut previous_txn_id,
                Some(cursor.read_u256()?),
                "sfPreviousTxnID",
            ),
            (STI_UINT32, SF_PREVIOUS_TXN_LGR_SEQ) => set_once(
                &mut previous_txn_lgr_seq,
                Some(cursor.read_u32()?),
                "sfPreviousTxnLgrSeq",
            ),
            (STI_UINT32, SF_TRANSFER_RATE) => set_once(
                &mut transfer_rate,
                Some(cursor.read_u32()?),
                "sfTransferRate",
            ),
            (STI_AMOUNT, SF_BALANCE) => {
                set_once(&mut balance, Some(read_amount_field(cursor)?), "sfBalance")
            }
            (STI_UINT8, SF_TICK_SIZE) => {
                set_once(&mut tick_size, Some(cursor.read_u8()?), "sfTickSize")
            }
            (STI_ACCOUNT, SF_ACCOUNT) => {
                set_once(&mut account_id, Some(read_account_id(cursor)?), "sfAccount")
            }
            _ => skip_field_value(field_type, cursor),
        }
    })?;

    expect_entry_type(entry_type, LT_ACCOUNT_ROOT)?;

    Ok(DecodedAccountRootEntry {
        flags,
        sequence,
        owner_count,
        account_txn_id,
        previous_txn_id,
        previous_txn_lgr_seq,
        transfer_rate,
        balance,
        tick_size,
        account_id,
    })
}

pub fn decode_constructor_amendments_entry(
    payload: &[u8],
) -> Result<ConstructorAmendmentsEntry, LedgerEntryDecodeError> {
    let decoded = decode_amendments_entry(payload)?;
    Ok(ConstructorAmendmentsEntry {
        amendments: decoded.amendments,
    })
}

pub fn decode_constructor_account_root_entry(
    payload: &[u8],
) -> Result<ConstructorAccountRootEntry, ConstructorAccountRootDecodeError> {
    let decoded = decode_account_root_entry(payload)
        .map_err(ConstructorAccountRootDecodeError::LedgerEntry)?;

    let Some(sequence) = decoded.sequence else {
        return Err(ConstructorAccountRootDecodeError::MissingField(
            "sfSequence",
        ));
    };
    let Some(balance) = decoded.balance else {
        return Err(ConstructorAccountRootDecodeError::MissingField("sfBalance"));
    };
    let Some(account_id) = decoded.account_id else {
        return Err(ConstructorAccountRootDecodeError::MissingField("sfAccount"));
    };

    if !balance.native {
        return Err(ConstructorAccountRootDecodeError::NonNativeBalance);
    }
    if balance.negative {
        return Err(ConstructorAccountRootDecodeError::NegativeBalance);
    }

    Ok(ConstructorAccountRootEntry {
        sequence,
        balance_drops: balance.drops,
        account_id,
    })
}

pub fn decode_constructor_fee_settings_entry(
    payload: &[u8],
) -> Result<ConstructorFeeSettingsEntry, ConstructorFeeSettingsDecodeError> {
    let decoded = decode_fee_settings_entry(payload)
        .map_err(ConstructorFeeSettingsDecodeError::LedgerEntry)?;

    let has_legacy = decoded.base_fee.is_some()
        || decoded.reference_fee_units.is_some()
        || decoded.reserve_base.is_some()
        || decoded.reserve_increment.is_some();
    let has_xrp = decoded.base_fee_drops.is_some()
        || decoded.reserve_base_drops.is_some()
        || decoded.reserve_increment_drops.is_some();

    if has_legacy && has_xrp {
        return Err(ConstructorFeeSettingsDecodeError::MixedFeeFormats);
    }

    if has_xrp {
        return Ok(ConstructorFeeSettingsEntry::XrpDrops {
            base_fee_drops: require_native_non_negative_amount(
                decoded.base_fee_drops,
                "sfBaseFeeDrops",
            )?,
            reserve_base_drops: require_native_non_negative_amount(
                decoded.reserve_base_drops,
                "sfReserveBaseDrops",
            )?,
            reserve_increment_drops: require_native_non_negative_amount(
                decoded.reserve_increment_drops,
                "sfReserveIncrementDrops",
            )?,
        });
    }

    if has_legacy {
        return Ok(ConstructorFeeSettingsEntry::Legacy {
            base_fee: decoded
                .base_fee
                .ok_or(ConstructorFeeSettingsDecodeError::MissingField("sfBaseFee"))?,
            reference_fee_units: decoded.reference_fee_units.ok_or(
                ConstructorFeeSettingsDecodeError::MissingField("sfReferenceFeeUnits"),
            )?,
            reserve_base: decoded.reserve_base,
            reserve_increment: decoded.reserve_increment,
        });
    }

    Err(ConstructorFeeSettingsDecodeError::MissingField(
        "fee settings fields",
    ))
}

pub fn decode_constructor_ledger_entry(
    payload: &[u8],
) -> Result<ConstructorLedgerEntry, ConstructorLedgerEntryDecodeError> {
    match decode_ledger_entry_type_code(payload)
        .map_err(ConstructorLedgerEntryDecodeError::LedgerEntry)?
    {
        LT_ACCOUNT_ROOT => decode_constructor_account_root_entry(payload)
            .map(ConstructorLedgerEntry::AccountRoot)
            .map_err(ConstructorLedgerEntryDecodeError::AccountRoot),
        LT_AMENDMENTS => decode_constructor_amendments_entry(payload)
            .map(ConstructorLedgerEntry::Amendments)
            .map_err(ConstructorLedgerEntryDecodeError::LedgerEntry),
        LT_FEE_SETTINGS => decode_constructor_fee_settings_entry(payload)
            .map(ConstructorLedgerEntry::FeeSettings)
            .map_err(ConstructorLedgerEntryDecodeError::FeeSettings),
        other => Err(ConstructorLedgerEntryDecodeError::UnsupportedLedgerEntryType(other)),
    }
}

pub fn decode_fee_settings_entry(
    payload: &[u8],
) -> Result<DecodedFeeSettingsEntry, LedgerEntryDecodeError> {
    let mut cursor = Cursor::new(payload);
    let mut entry_type = None;
    let mut fields = DecodedFeeSettingsEntry::default();

    parse_object(&mut cursor, |field_type, field_name, cursor| {
        match (field_type, field_name) {
            (STI_UINT16, SF_LEDGER_ENTRY_TYPE) => set_once(
                &mut entry_type,
                Some(cursor.read_u16()?),
                "sfLedgerEntryType",
            ),
            (STI_UINT64, SF_BASE_FEE) => {
                set_once(&mut fields.base_fee, Some(cursor.read_u64()?), "sfBaseFee")
            }
            (STI_UINT32, SF_REFERENCE_FEE_UNITS) => set_once(
                &mut fields.reference_fee_units,
                Some(cursor.read_u32()?),
                "sfReferenceFeeUnits",
            ),
            (STI_UINT32, SF_RESERVE_BASE) => set_once(
                &mut fields.reserve_base,
                Some(cursor.read_u32()?),
                "sfReserveBase",
            ),
            (STI_UINT32, SF_RESERVE_INCREMENT) => set_once(
                &mut fields.reserve_increment,
                Some(cursor.read_u32()?),
                "sfReserveIncrement",
            ),
            (STI_AMOUNT, SF_BASE_FEE_DROPS) => set_once(
                &mut fields.base_fee_drops,
                Some(read_amount_field(cursor)?),
                "sfBaseFeeDrops",
            ),
            (STI_AMOUNT, SF_RESERVE_BASE_DROPS) => set_once(
                &mut fields.reserve_base_drops,
                Some(read_amount_field(cursor)?),
                "sfReserveBaseDrops",
            ),
            (STI_AMOUNT, SF_RESERVE_INCREMENT_DROPS) => set_once(
                &mut fields.reserve_increment_drops,
                Some(read_amount_field(cursor)?),
                "sfReserveIncrementDrops",
            ),
            (STI_UINT32, SF_CLOSE_TIME) => skip_field_value(field_type, cursor),
            (STI_UINT256, SF_PREVIOUS_TXN_ID) => set_once(
                &mut fields.previous_txn_id,
                Some(cursor.read_u256()?),
                "sfPreviousTxnID",
            ),
            (STI_UINT32, SF_PREVIOUS_TXN_LGR_SEQ) => set_once(
                &mut fields.previous_txn_lgr_seq,
                Some(cursor.read_u32()?),
                "sfPreviousTxnLgrSeq",
            ),
            _ => skip_field_value(field_type, cursor),
        }
    })?;

    expect_entry_type(entry_type, LT_FEE_SETTINGS)?;
    Ok(fields)
}

fn append_field_id(bytes: &mut Vec<u8>, field_type: u8, field_name: u8) {
    if field_type < 16 && field_name < 16 {
        bytes.push((field_type << 4) | field_name);
    } else if field_type < 16 {
        bytes.push(field_type << 4);
        bytes.push(field_name);
    } else if field_name < 16 {
        bytes.push(field_name);
        bytes.push(field_type);
    } else {
        bytes.push(0);
        bytes.push(field_type);
        bytes.push(field_name);
    }
}

fn append_u16_field(bytes: &mut Vec<u8>, field_name: u8, value: u16) {
    append_field_id(bytes, STI_UINT16, field_name);
    bytes.extend_from_slice(&value.to_be_bytes());
}

fn append_u32_field(bytes: &mut Vec<u8>, field_name: u8, value: u32) {
    append_field_id(bytes, STI_UINT32, field_name);
    bytes.extend_from_slice(&value.to_be_bytes());
}

fn append_u64_field(bytes: &mut Vec<u8>, field_name: u8, value: u64) {
    append_field_id(bytes, STI_UINT64, field_name);
    bytes.extend_from_slice(&value.to_be_bytes());
}

fn append_u256_field(bytes: &mut Vec<u8>, field_name: u8, value: Uint256) {
    append_field_id(bytes, STI_UINT256, field_name);
    bytes.extend_from_slice(value.data());
}

fn append_native_amount_field(bytes: &mut Vec<u8>, field_name: u8, value: u64) {
    append_field_id(bytes, STI_AMOUNT, field_name);
    bytes.extend_from_slice(&(value | STAMOUNT_POSITIVE).to_be_bytes());
}

fn append_account_field(bytes: &mut Vec<u8>, field_name: u8, value: Uint160) {
    append_field_id(bytes, STI_ACCOUNT, field_name);
    append_variable_length(bytes, Uint160::BYTES);
    bytes.extend_from_slice(value.data());
}

fn append_vl_field(bytes: &mut Vec<u8>, field_name: u8, value: &[u8]) {
    append_field_id(bytes, STI_VL, field_name);
    append_variable_length(bytes, value.len());
    bytes.extend_from_slice(value);
}

fn append_vector256_field(bytes: &mut Vec<u8>, field_name: u8, values: &[Uint256]) {
    append_field_id(bytes, STI_VECTOR256, field_name);
    let payload_len = values.len() * Uint256::BYTES;
    append_variable_length(bytes, payload_len);
    for value in values {
        bytes.extend_from_slice(value.data());
    }
}

fn append_variable_length(bytes: &mut Vec<u8>, length: usize) {
    if length <= 192 {
        bytes.push(length as u8);
        return;
    }

    if length <= 12_480 {
        let adjusted = length - 193;
        bytes.push(193 + ((adjusted >> 8) as u8));
        bytes.push((adjusted & 0xff) as u8);
        return;
    }

    if length <= 918_744 {
        let adjusted = length - 12_481;
        bytes.push(241 + ((adjusted >> 16) as u8));
        bytes.push(((adjusted >> 8) & 0xff) as u8);
        bytes.push((adjusted & 0xff) as u8);
        return;
    }

    panic!("variable-length field payload exceeds current XRPL encoding limits");
}

fn expect_entry_type(entry_type: Option<u16>, expected: u16) -> Result<(), LedgerEntryDecodeError> {
    let Some(entry_type) = entry_type else {
        return Err(LedgerEntryDecodeError::MissingLedgerEntryType);
    };

    if entry_type != expected {
        return Err(LedgerEntryDecodeError::UnexpectedLedgerEntryType {
            expected,
            actual: entry_type,
        });
    }

    Ok(())
}

fn set_once<T>(
    slot: &mut Option<T>,
    value: Option<T>,
    name: &'static str,
) -> Result<(), LedgerEntryDecodeError> {
    if value.is_none() {
        return Ok(());
    }
    if slot.is_some() {
        return Err(LedgerEntryDecodeError::DuplicateField(name));
    }
    *slot = value;
    Ok(())
}

fn read_vector256(cursor: &mut Cursor<'_>) -> Result<Vec<Uint256>, LedgerEntryDecodeError> {
    let length = cursor.read_variable_length()?;
    if length % Uint256::BYTES != 0 {
        return Err(LedgerEntryDecodeError::InvalidVector256Length(length));
    }

    let slice = cursor.read_bytes(length)?;
    Ok(slice
        .chunks_exact(Uint256::BYTES)
        .map(|chunk| Uint256::from_slice(chunk).expect("vector256 chunk must be 32 bytes"))
        .collect())
}

fn read_majorities_array(
    cursor: &mut Cursor<'_>,
) -> Result<Vec<DecodedMajorityEntry>, LedgerEntryDecodeError> {
    let mut majorities = Vec::new();

    while !cursor.is_empty() {
        let (field_type, field_name) = cursor.read_field_id()?;

        if field_type == STI_ARRAY && field_name == 1 {
            return Ok(majorities);
        }

        if field_type == STI_OBJECT && field_name == SF_MAJORITY {
            majorities.push(read_majority_object(cursor)?);
            continue;
        }

        skip_field_value(field_type, cursor)?;
    }

    Err(LedgerEntryDecodeError::MissingArrayEnd)
}

fn read_disabled_validators_array(
    cursor: &mut Cursor<'_>,
) -> Result<Vec<DecodedDisabledValidator>, LedgerEntryDecodeError> {
    let mut validators = Vec::new();

    while !cursor.is_empty() {
        let (field_type, field_name) = cursor.read_field_id()?;

        if field_type == STI_ARRAY && field_name == 1 {
            return Ok(validators);
        }

        if field_type == STI_OBJECT && field_name == SF_DISABLED_VALIDATOR {
            validators.push(read_disabled_validator_object(cursor)?);
            continue;
        }

        skip_field_value(field_type, cursor)?;
    }

    Err(LedgerEntryDecodeError::MissingArrayEnd)
}

fn read_disabled_validator_object(
    cursor: &mut Cursor<'_>,
) -> Result<DecodedDisabledValidator, LedgerEntryDecodeError> {
    let mut public_key = None;
    let mut first_ledger_sequence = None;

    while !cursor.is_empty() {
        let (field_type, field_name) = cursor.read_field_id()?;

        if field_type == STI_OBJECT && field_name == 1 {
            return Ok(DecodedDisabledValidator {
                public_key: public_key
                    .ok_or(LedgerEntryDecodeError::MissingField("sfPublicKey"))?,
                first_ledger_sequence: first_ledger_sequence.ok_or(
                    LedgerEntryDecodeError::MissingField("sfFirstLedgerSequence"),
                )?,
            });
        }
        if field_type == STI_ARRAY && field_name == 1 {
            return Err(LedgerEntryDecodeError::UnexpectedArrayEnd);
        }

        match (field_type, field_name) {
            (STI_VL, SF_PUBLIC_KEY) => set_once(
                &mut public_key,
                Some(read_variable_length_bytes(cursor)?.to_vec()),
                "sfPublicKey",
            )?,
            (STI_UINT32, SF_FIRST_LEDGER_SEQUENCE) => set_once(
                &mut first_ledger_sequence,
                Some(cursor.read_u32()?),
                "sfFirstLedgerSequence",
            )?,
            _ => skip_field_value(field_type, cursor)?,
        }
    }

    Err(LedgerEntryDecodeError::MissingObjectEnd)
}

fn read_majority_object(
    cursor: &mut Cursor<'_>,
) -> Result<DecodedMajorityEntry, LedgerEntryDecodeError> {
    let mut close_time = None;
    let mut amendment = None;

    while !cursor.is_empty() {
        let (field_type, field_name) = cursor.read_field_id()?;

        if field_type == STI_OBJECT && field_name == 1 {
            return Ok(DecodedMajorityEntry {
                close_time: close_time
                    .ok_or(LedgerEntryDecodeError::MissingField("sfCloseTime"))?,
                amendment: amendment
                    .ok_or(LedgerEntryDecodeError::MissingField("sfMajorityAmendment"))?,
            });
        }
        if field_type == STI_ARRAY && field_name == 1 {
            return Err(LedgerEntryDecodeError::UnexpectedArrayEnd);
        }

        match (field_type, field_name) {
            (STI_UINT32, SF_CLOSE_TIME) => {
                set_once(&mut close_time, Some(cursor.read_u32()?), "sfCloseTime")?
            }
            (STI_UINT256, SF_MAJORITY_AMENDMENT) => set_once(
                &mut amendment,
                Some(cursor.read_u256()?),
                "sfMajorityAmendment",
            )?,
            _ => skip_field_value(field_type, cursor)?,
        }
    }

    Err(LedgerEntryDecodeError::MissingObjectEnd)
}

fn read_amount_field(
    cursor: &mut Cursor<'_>,
) -> Result<DecodedAmountField, LedgerEntryDecodeError> {
    let value = cursor.read_u64()?;

    if (value & STAMOUNT_ISSUED_CURRENCY) == 0 {
        if (value & STAMOUNT_MPTOKEN) != 0 {
            cursor.skip(25)?;
            return Ok(DecodedAmountField {
                drops: 0,
                native: false,
                negative: false,
            });
        }

        let negative = (value & STAMOUNT_POSITIVE) == 0 && value != 0;
        return Ok(DecodedAmountField {
            drops: value & STAMOUNT_VALUE_MASK,
            native: true,
            negative,
        });
    }

    cursor.skip(40)?;
    Ok(DecodedAmountField {
        drops: 0,
        native: false,
        negative: false,
    })
}

fn read_account_id(cursor: &mut Cursor<'_>) -> Result<Uint160, LedgerEntryDecodeError> {
    let length = cursor.read_variable_length()?;
    if length != Uint160::BYTES {
        return Err(LedgerEntryDecodeError::TruncatedField);
    }
    let bytes = cursor.read_bytes(length)?;
    Ok(Uint160::from_slice(bytes).expect("account id bytes must be 20 bytes"))
}

fn read_variable_length_bytes<'a>(
    cursor: &mut Cursor<'a>,
) -> Result<&'a [u8], LedgerEntryDecodeError> {
    let length = cursor.read_variable_length()?;
    cursor.read_bytes(length)
}

fn require_native_non_negative_amount(
    amount: Option<DecodedAmountField>,
    field_name: &'static str,
) -> Result<u64, ConstructorFeeSettingsDecodeError> {
    let amount = amount.ok_or(ConstructorFeeSettingsDecodeError::MissingField(field_name))?;

    if !amount.native {
        return Err(ConstructorFeeSettingsDecodeError::NonNativeAmount(
            field_name,
        ));
    }
    if amount.negative {
        return Err(ConstructorFeeSettingsDecodeError::NegativeAmount(
            field_name,
        ));
    }

    Ok(amount.drops)
}

fn parse_object<F>(
    cursor: &mut Cursor<'_>,
    mut field_handler: F,
) -> Result<(), LedgerEntryDecodeError>
where
    F: FnMut(u8, u8, &mut Cursor<'_>) -> Result<(), LedgerEntryDecodeError>,
{
    while !cursor.is_empty() {
        let (field_type, field_name) = cursor.read_field_id()?;

        if field_type == STI_OBJECT && field_name == 1 {
            return Ok(());
        }
        if field_type == STI_ARRAY && field_name == 1 {
            return Err(LedgerEntryDecodeError::UnexpectedArrayEnd);
        }

        field_handler(field_type, field_name, cursor)?;
    }

    // Top-level SLEs in the state map don't have an end marker —
    // Reaching end of data is valid for top-level objects.
    Ok(())
}

fn skip_field_value(field_type: u8, cursor: &mut Cursor<'_>) -> Result<(), LedgerEntryDecodeError> {
    match field_type {
        STI_UINT8 => cursor.skip(1),
        STI_UINT16 => cursor.skip(2),
        STI_UINT32 | STI_INT32 => cursor.skip(4),
        STI_UINT64 | STI_INT64 => cursor.skip(8),
        STI_UINT96 => cursor.skip(12),
        STI_UINT128 => cursor.skip(16),
        STI_UINT160 | STI_CURRENCY => cursor.skip(20),
        STI_UINT192 => cursor.skip(24),
        STI_UINT256 => cursor.skip(32),
        STI_ISSUE => cursor.skip(40),
        STI_UINT384 | STI_XCHAIN_BRIDGE => cursor.skip(48),
        STI_UINT512 => cursor.skip(64),
        STI_AMOUNT => {
            let _ = read_amount_field(cursor)?;
            Ok(())
        }
        STI_VL | STI_ACCOUNT | STI_PATHSET | STI_NUMBER | STI_VECTOR256 => {
            let length = cursor.read_variable_length()?;
            cursor.skip(length)
        }
        STI_OBJECT => skip_object(cursor),
        STI_ARRAY => skip_array(cursor),
        _ => Err(LedgerEntryDecodeError::UnsupportedFieldType(field_type)),
    }
}

fn skip_object(cursor: &mut Cursor<'_>) -> Result<(), LedgerEntryDecodeError> {
    while !cursor.is_empty() {
        let (field_type, field_name) = cursor.read_field_id()?;

        if field_type == STI_OBJECT && field_name == 1 {
            return Ok(());
        }
        if field_type == STI_ARRAY && field_name == 1 {
            return Err(LedgerEntryDecodeError::UnexpectedArrayEnd);
        }

        skip_field_value(field_type, cursor)?;
    }

    Err(LedgerEntryDecodeError::MissingObjectEnd)
}

fn skip_array(cursor: &mut Cursor<'_>) -> Result<(), LedgerEntryDecodeError> {
    while !cursor.is_empty() {
        let (field_type, field_name) = cursor.read_field_id()?;

        if field_type == STI_ARRAY && field_name == 1 {
            return Ok(());
        }
        if field_type == STI_OBJECT && field_name == 1 {
            return Err(LedgerEntryDecodeError::UnexpectedObjectEnd);
        }

        skip_field_value(field_type, cursor)?;
    }

    Err(LedgerEntryDecodeError::MissingArrayEnd)
}

struct Cursor<'a> {
    bytes: &'a [u8],
    offset: usize,
}

impl<'a> Cursor<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, offset: 0 }
    }

    fn is_empty(&self) -> bool {
        self.offset == self.bytes.len()
    }

    fn skip(&mut self, length: usize) -> Result<(), LedgerEntryDecodeError> {
        let _ = self.read_bytes(length)?;
        Ok(())
    }

    fn read_bytes(&mut self, length: usize) -> Result<&'a [u8], LedgerEntryDecodeError> {
        let end = self
            .offset
            .checked_add(length)
            .ok_or(LedgerEntryDecodeError::TruncatedField)?;
        let slice = self
            .bytes
            .get(self.offset..end)
            .ok_or(LedgerEntryDecodeError::TruncatedField)?;
        self.offset = end;
        Ok(slice)
    }

    fn read_u8(&mut self) -> Result<u8, LedgerEntryDecodeError> {
        Ok(self.read_bytes(1)?[0])
    }

    fn read_u16(&mut self) -> Result<u16, LedgerEntryDecodeError> {
        Ok(u16::from_be_bytes(
            self.read_bytes(2)?
                .try_into()
                .expect("u16 slice must have 2 bytes"),
        ))
    }

    fn read_u32(&mut self) -> Result<u32, LedgerEntryDecodeError> {
        Ok(u32::from_be_bytes(
            self.read_bytes(4)?
                .try_into()
                .expect("u32 slice must have 4 bytes"),
        ))
    }

    fn read_u64(&mut self) -> Result<u64, LedgerEntryDecodeError> {
        Ok(u64::from_be_bytes(
            self.read_bytes(8)?
                .try_into()
                .expect("u64 slice must have 8 bytes"),
        ))
    }

    fn read_u256(&mut self) -> Result<Uint256, LedgerEntryDecodeError> {
        Ok(Uint256::from_slice(self.read_bytes(Uint256::BYTES)?)
            .expect("uint256 slice must have 32 bytes"))
    }

    fn read_field_id(&mut self) -> Result<(u8, u8), LedgerEntryDecodeError> {
        let first = self.read_u8()?;
        let mut field_type = first >> 4;
        let mut field_name = first & 0x0F;

        if field_type == 0 {
            field_type = self.read_u8()?;
        }
        if field_name == 0 {
            field_name = self.read_u8()?;
        }

        Ok((field_type, field_name))
    }

    fn read_variable_length(&mut self) -> Result<usize, LedgerEntryDecodeError> {
        let b1 = self.read_u8()?;

        if b1 <= 192 {
            return Ok(usize::from(b1));
        }
        if b1 <= 240 {
            let b2 = usize::from(self.read_u8()?);
            return Ok(193 + ((usize::from(b1) - 193) << 8) + b2);
        }
        if b1 <= 254 {
            let b2 = usize::from(self.read_u8()?);
            let b3 = usize::from(self.read_u8()?);
            return Ok(12_481 + ((usize::from(b1) - 241) << 16) + (b2 << 8) + b3);
        }

        Err(LedgerEntryDecodeError::InvalidVariableLengthPrefix(b1))
    }
}

#[cfg(test)]
mod tests {
    use super::{
        ConstructorAccountRootDecodeError, ConstructorAccountRootEntry, ConstructorAmendmentsEntry,
        ConstructorFeeSettingsDecodeError, ConstructorFeeSettingsEntry, DecodedAmountField,
        DecodedMajorityEntry, LT_AMENDMENTS, LT_FEE_SETTINGS, REFERENCE_FEE_UNITS_DEPRECATED,
        STI_ACCOUNT, STI_AMOUNT, STI_ARRAY, STI_OBJECT, STI_UINT8, STI_UINT16, STI_UINT32,
        STI_UINT64, STI_UINT256, STI_VECTOR256, decode_account_root_entry, decode_amendments_entry,
        decode_constructor_account_root_entry, decode_constructor_amendments_entry,
        decode_constructor_fee_settings_entry, decode_fee_settings_entry,
        encode_account_root_entry, encode_amendments_entry, encode_constructor_account_root_entry,
        encode_constructor_amendments_entry, encode_constructor_fee_settings_entry,
        encode_fee_settings_entry, make_constructor_fee_settings_entry,
    };
    use crate::feature_xrp_fees;
    use basics::base_uint::{Uint160, Uint256};

    fn bytes_to_hex(bytes: &[u8]) -> String {
        bytes.iter().map(|byte| format!("{byte:02X}")).collect()
    }

    #[test]
    fn account_root_entry_matches_current_cpp_genesis_vector() {
        let account_id = Uint160::from_hex("B5F762798A53D543A014CAF8B297CFF8F2F937E8")
            .expect("expected account id should parse");

        assert_eq!(
            bytes_to_hex(&encode_account_root_entry(
                account_id,
                1,
                100_000_000_000_000_000,
            )),
            "1100612200000000240000000162416345785D8A00008114B5F762798A53D543A014CAF8B297CFF8F2F937E8"
        );
    }

    #[test]
    fn amendments_entry_matches_current_field_layout() {
        let amendment = Uint256::from_array([0xAB; 32]);

        assert_eq!(
            bytes_to_hex(&encode_amendments_entry(&[amendment])),
            format!("1100662200000000031320{}", "AB".repeat(32))
        );
    }

    #[test]
    fn constructor_amendments_entry_round_trips_through_typed_helpers() {
        let entry = ConstructorAmendmentsEntry {
            amendments: vec![
                Uint256::from_array([0xAB; 32]),
                Uint256::from_array([0xAC; 32]),
            ],
        };

        let decoded =
            decode_constructor_amendments_entry(&encode_constructor_amendments_entry(&entry))
                .expect("encoded constructor amendments entry should decode");

        assert_eq!(decoded, entry);
    }

    #[test]
    fn constructor_amendments_entry_defaults_to_empty_when_field_is_absent() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&encode_u16(1, LT_AMENDMENTS));
        bytes.push(super::OBJECT_END);

        let decoded = decode_constructor_amendments_entry(&bytes)
            .expect("typed constructor amendments decode should preserve empty default");

        assert_eq!(decoded, ConstructorAmendmentsEntry::default());
    }

    #[test]
    fn legacy_fee_settings_entry_matches_current_field_layout() {
        assert_eq!(
            bytes_to_hex(&encode_fee_settings_entry(10, 20, 30, false)),
            "11007335000000000000000A201E0000000A201F0000001420200000001EE1"
        );
    }

    #[test]
    fn xrp_fee_settings_entry_matches_current_field_layout() {
        assert_eq!(
            bytes_to_hex(&encode_fee_settings_entry(11, 22, 33, true)),
            "1100736016400000000000000B6017400000000000001660184000000000000021E1"
        );
    }

    fn field_id(field_type: u8, field_name: u8) -> Vec<u8> {
        if field_type < 16 && field_name < 16 {
            vec![(field_type << 4) | field_name]
        } else if field_type < 16 {
            vec![field_type << 4, field_name]
        } else if field_name < 16 {
            vec![field_name, field_type]
        } else {
            vec![0, field_type, field_name]
        }
    }

    fn encode_u16(field_name: u8, value: u16) -> Vec<u8> {
        let mut bytes = field_id(STI_UINT16, field_name);
        bytes.extend_from_slice(&value.to_be_bytes());
        bytes
    }

    fn encode_u8(field_name: u8, value: u8) -> Vec<u8> {
        let mut bytes = field_id(STI_UINT8, field_name);
        bytes.push(value);
        bytes
    }

    fn encode_u32(field_name: u8, value: u32) -> Vec<u8> {
        let mut bytes = field_id(STI_UINT32, field_name);
        bytes.extend_from_slice(&value.to_be_bytes());
        bytes
    }

    fn encode_u64(field_name: u8, value: u64) -> Vec<u8> {
        let mut bytes = field_id(STI_UINT64, field_name);
        bytes.extend_from_slice(&value.to_be_bytes());
        bytes
    }

    fn encode_native_amount(field_name: u8, value: u64) -> Vec<u8> {
        let mut bytes = field_id(STI_AMOUNT, field_name);
        bytes.extend_from_slice(&(value | super::STAMOUNT_POSITIVE).to_be_bytes());
        bytes
    }

    fn encode_vector256(field_name: u8, values: &[Uint256]) -> Vec<u8> {
        let mut bytes = field_id(STI_VECTOR256, field_name);
        let payload_len = values.len() * Uint256::BYTES;
        bytes
            .push(u8::try_from(payload_len).expect("small vector256 payload must fit in one byte"));
        for value in values {
            bytes.extend_from_slice(value.data());
        }
        bytes
    }

    fn encode_uint256(field_name: u8, value: Uint256) -> Vec<u8> {
        let mut bytes = field_id(STI_UINT256, field_name);
        bytes.extend_from_slice(value.data());
        bytes
    }

    fn encode_majorities_array() -> Vec<u8> {
        let amendment = Uint256::from_array([0xAB; 32]);
        let mut bytes = field_id(STI_ARRAY, 16);
        bytes.extend_from_slice(&field_id(STI_OBJECT, 18));
        bytes.extend_from_slice(&encode_u32(7, 22));
        bytes.extend_from_slice(&encode_uint256(19, amendment));
        bytes.push(super::OBJECT_END);
        bytes.push(0xF1);
        bytes
    }

    #[test]
    fn decoders_read_target_fields_and_skip_unneeded_ones() {
        let amendment = Uint256::from_array([0x11; 32]);
        let mut amendments_bytes = Vec::new();
        amendments_bytes.extend_from_slice(&encode_u16(1, LT_AMENDMENTS));
        amendments_bytes.extend_from_slice(&encode_vector256(3, &[amendment]));
        amendments_bytes.extend_from_slice(&encode_majorities_array());
        amendments_bytes.push(super::OBJECT_END);

        let amendments =
            decode_amendments_entry(&amendments_bytes).expect("amendments entry should decode");
        assert_eq!(amendments.amendments, vec![amendment]);
        assert_eq!(
            amendments.majorities,
            vec![DecodedMajorityEntry {
                close_time: 22,
                amendment: Uint256::from_array([0xAB; 32]),
            }]
        );
        assert_eq!(amendments.previous_txn_id, None);
        assert_eq!(amendments.previous_txn_lgr_seq, None);

        let mut fee_bytes = Vec::new();
        fee_bytes.extend_from_slice(&encode_u16(1, LT_FEE_SETTINGS));
        fee_bytes.extend_from_slice(&encode_u64(5, 10));
        fee_bytes.extend_from_slice(&encode_u32(30, 256));
        fee_bytes.extend_from_slice(&encode_u32(31, 20));
        fee_bytes.extend_from_slice(&encode_native_amount(24, 30));
        fee_bytes.push(super::OBJECT_END);

        let fees = decode_fee_settings_entry(&fee_bytes).expect("fees entry should decode");
        assert_eq!(fees.base_fee, Some(10));
        assert_eq!(fees.reference_fee_units, Some(256));
        assert_eq!(fees.reserve_base, Some(20));
        assert_eq!(
            fees.reserve_increment_drops,
            Some(DecodedAmountField {
                drops: 30,
                native: true,
                negative: false,
            })
        );
        assert_eq!(fees.previous_txn_id, None);
        assert_eq!(fees.previous_txn_lgr_seq, None);
    }

    #[test]
    fn encoders_round_trip_through_decoders() {
        let amendments = vec![
            Uint256::from_array([0x31; 32]),
            Uint256::from_array([0x32; 32]),
        ];
        let decoded_amendments = decode_amendments_entry(&encode_amendments_entry(&amendments))
            .expect("encoded amendments entry should decode");
        assert_eq!(decoded_amendments.amendments, amendments);
        assert!(decoded_amendments.majorities.is_empty());
        assert_eq!(decoded_amendments.previous_txn_id, None);
        assert_eq!(decoded_amendments.previous_txn_lgr_seq, None);

        let decoded_fees = decode_fee_settings_entry(&encode_fee_settings_entry(11, 22, 33, true))
            .expect("encoded fee settings entry should decode");
        assert_eq!(
            decoded_fees.base_fee_drops,
            Some(DecodedAmountField {
                drops: 11,
                native: true,
                negative: false,
            })
        );
        assert_eq!(
            decoded_fees.reserve_base_drops,
            Some(DecodedAmountField {
                drops: 22,
                native: true,
                negative: false,
            })
        );
        assert_eq!(
            decoded_fees.reserve_increment_drops,
            Some(DecodedAmountField {
                drops: 33,
                native: true,
                negative: false,
            })
        );
    }

    #[test]
    fn account_root_entry_round_trips_through_decoder() {
        let account_id = Uint160::from_hex("B5F762798A53D543A014CAF8B297CFF8F2F937E8")
            .expect("expected account id should parse");

        let decoded = decode_account_root_entry(&encode_account_root_entry(
            account_id,
            1,
            100_000_000_000_000_000,
        ))
        .expect("encoded account root should decode");

        assert_eq!(decoded.sequence, Some(1));
        assert_eq!(
            decoded.balance,
            Some(DecodedAmountField {
                drops: 100_000_000_000_000_000,
                native: true,
                negative: false,
            })
        );
        assert_eq!(decoded.account_id, Some(account_id));
        assert_eq!(decoded.flags, Some(0));
        assert_eq!(decoded.owner_count, Some(0));
        assert_eq!(decoded.account_txn_id, None);
        assert_eq!(decoded.previous_txn_id, Some(Uint256::zero()));
        assert_eq!(decoded.previous_txn_lgr_seq, Some(0));
        assert_eq!(decoded.transfer_rate, None);
        assert_eq!(decoded.tick_size, None);
    }

    #[test]
    fn amendments_decoder_reads_previous_txn_and_majority_objects() {
        let amendment = Uint256::from_array([0x51; 32]);
        let previous_txn_id = Uint256::from_array([0x61; 32]);
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&encode_u16(1, LT_AMENDMENTS));
        bytes.extend_from_slice(&encode_vector256(3, &[amendment]));
        bytes.extend_from_slice(&encode_majorities_array());
        bytes.extend_from_slice(&encode_uint256(5, previous_txn_id));
        bytes.extend_from_slice(&encode_u32(5, 91_442_944));
        bytes.push(super::OBJECT_END);

        let decoded = decode_amendments_entry(&bytes)
            .expect("amendments decoder should preserve current singleton tail fields");

        assert_eq!(decoded.amendments, vec![amendment]);
        assert_eq!(
            decoded.majorities,
            vec![DecodedMajorityEntry {
                close_time: 22,
                amendment: Uint256::from_array([0xAB; 32]),
            }]
        );
        assert_eq!(decoded.previous_txn_id, Some(previous_txn_id));
        assert_eq!(decoded.previous_txn_lgr_seq, Some(91_442_944));
    }

    #[test]
    fn account_root_decoder_reads_common_tail_fields() {
        let account_id = Uint160::from_hex("B5F762798A53D543A014CAF8B297CFF8F2F937E8")
            .expect("expected account id should parse");
        let previous_txn_id = Uint256::from_array([0x71; 32]);
        let account_txn_id = Uint256::from_array([0x72; 32]);
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&encode_u16(1, 0x0061));
        bytes.extend_from_slice(&encode_u32(2, 0x0012_0000));
        bytes.extend_from_slice(&encode_u32(4, 7));
        bytes.extend_from_slice(&encode_u32(13, 9));
        bytes.extend_from_slice(&encode_uint256(9, account_txn_id));
        bytes.extend_from_slice(&encode_uint256(5, previous_txn_id));
        bytes.extend_from_slice(&encode_u32(5, 91_442_945));
        bytes.extend_from_slice(&encode_u32(11, 1_000_000_000));
        bytes.extend_from_slice(&encode_native_amount(2, 123_456_789));
        bytes.extend_from_slice(&encode_u8(16, 12));
        bytes.extend_from_slice(&field_id(STI_ACCOUNT, 1));
        bytes.push(
            u8::try_from(Uint160::BYTES).expect("account ids should fit in one-byte VL prefixes"),
        );
        bytes.extend_from_slice(account_id.data());
        bytes.push(super::OBJECT_END);

        let decoded =
            decode_account_root_entry(&bytes).expect("account root tail fields should decode");

        assert_eq!(decoded.flags, Some(0x0012_0000));
        assert_eq!(decoded.sequence, Some(7));
        assert_eq!(decoded.owner_count, Some(9));
        assert_eq!(decoded.account_txn_id, Some(account_txn_id));
        assert_eq!(decoded.previous_txn_id, Some(previous_txn_id));
        assert_eq!(decoded.previous_txn_lgr_seq, Some(91_442_945));
        assert_eq!(decoded.transfer_rate, Some(1_000_000_000));
        assert_eq!(
            decoded.balance,
            Some(DecodedAmountField {
                drops: 123_456_789,
                native: true,
                negative: false,
            })
        );
        assert_eq!(decoded.tick_size, Some(12));
        assert_eq!(decoded.account_id, Some(account_id));
    }

    #[test]
    fn fee_settings_decoder_reads_previous_txn_tail_fields() {
        let previous_txn_id = Uint256::from_array([0x81; 32]);
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&encode_u16(1, LT_FEE_SETTINGS));
        bytes.extend_from_slice(&encode_u64(5, 10));
        bytes.extend_from_slice(&encode_u32(30, 256));
        bytes.extend_from_slice(&encode_uint256(5, previous_txn_id));
        bytes.extend_from_slice(&encode_u32(5, 91_442_946));
        bytes.push(super::OBJECT_END);

        let decoded =
            decode_fee_settings_entry(&bytes).expect("fee settings tail fields should decode");

        assert_eq!(decoded.base_fee, Some(10));
        assert_eq!(decoded.reference_fee_units, Some(256));
        assert_eq!(decoded.previous_txn_id, Some(previous_txn_id));
        assert_eq!(decoded.previous_txn_lgr_seq, Some(91_442_946));
    }

    #[test]
    fn constructor_account_root_entry_round_trips_through_typed_helpers() {
        let entry = ConstructorAccountRootEntry {
            sequence: 1,
            balance_drops: 100_000_000_000_000_000,
            account_id: Uint160::from_hex("B5F762798A53D543A014CAF8B297CFF8F2F937E8")
                .expect("expected account id should parse"),
        };

        let decoded =
            decode_constructor_account_root_entry(&encode_constructor_account_root_entry(entry))
                .expect("encoded constructor account root should decode");

        assert_eq!(decoded, entry);
    }

    #[test]
    fn constructor_account_root_entry_requires_sequence_balance_and_account() {
        let account_id = Uint160::from_hex("B5F762798A53D543A014CAF8B297CFF8F2F937E8")
            .expect("expected account id should parse");

        let mut bytes = Vec::new();
        bytes.extend_from_slice(&encode_u16(1, 0x0061));
        bytes.extend_from_slice(&encode_native_amount(2, 100_000_000_000_000_000));
        bytes.extend_from_slice(&field_id(STI_ACCOUNT, 1));
        bytes.push(
            u8::try_from(Uint160::BYTES).expect("account ids should fit in one-byte VL prefixes"),
        );
        bytes.extend_from_slice(account_id.data());
        bytes.push(super::OBJECT_END);

        let error = decode_constructor_account_root_entry(&bytes)
            .expect_err("constructor account root without sequence must fail");
        assert_eq!(
            error,
            ConstructorAccountRootDecodeError::MissingField("sfSequence")
        );
    }

    #[test]
    fn constructor_account_root_entry_rejects_non_native_balance() {
        let account_id = Uint160::from_hex("B5F762798A53D543A014CAF8B297CFF8F2F937E8")
            .expect("expected account id should parse");

        let mut bytes = Vec::new();
        bytes.extend_from_slice(&encode_u16(1, 0x0061));
        bytes.extend_from_slice(&encode_u32(4, 1));
        bytes.extend_from_slice(&field_id(STI_AMOUNT, 2));
        bytes.extend_from_slice(&0x8000_0000_0000_0001u64.to_be_bytes());
        bytes.extend_from_slice(&[0u8; 40]);
        bytes.extend_from_slice(&field_id(STI_ACCOUNT, 1));
        bytes.push(
            u8::try_from(Uint160::BYTES).expect("account ids should fit in one-byte VL prefixes"),
        );
        bytes.extend_from_slice(account_id.data());
        bytes.push(super::OBJECT_END);

        let error = decode_constructor_account_root_entry(&bytes)
            .expect_err("constructor account root with issued balance must fail");
        assert_eq!(error, ConstructorAccountRootDecodeError::NonNativeBalance);
    }

    #[test]
    fn constructor_account_root_entry_rejects_negative_balance() {
        let account_id = Uint160::from_hex("B5F762798A53D543A014CAF8B297CFF8F2F937E8")
            .expect("expected account id should parse");

        let mut bytes = Vec::new();
        bytes.extend_from_slice(&encode_u16(1, 0x0061));
        bytes.extend_from_slice(&encode_u32(4, 1));
        bytes.extend_from_slice(&field_id(STI_AMOUNT, 2));
        bytes.extend_from_slice(&42u64.to_be_bytes());
        bytes.extend_from_slice(&field_id(STI_ACCOUNT, 1));
        bytes.push(
            u8::try_from(Uint160::BYTES).expect("account ids should fit in one-byte VL prefixes"),
        );
        bytes.extend_from_slice(account_id.data());
        bytes.push(super::OBJECT_END);

        let error = decode_constructor_account_root_entry(&bytes)
            .expect_err("constructor account root with negative balance must fail");
        assert_eq!(error, ConstructorAccountRootDecodeError::NegativeBalance);
    }

    #[test]
    fn constructor_fee_settings_entry_round_trips_through_typed_legacy_helpers() {
        let entry = ConstructorFeeSettingsEntry::Legacy {
            base_fee: 10,
            reference_fee_units: REFERENCE_FEE_UNITS_DEPRECATED,
            reserve_base: Some(20),
            reserve_increment: Some(30),
        };

        let decoded =
            decode_constructor_fee_settings_entry(&encode_constructor_fee_settings_entry(entry))
                .expect("encoded constructor legacy fee settings should decode");

        assert_eq!(decoded, entry);
    }

    #[test]
    fn constructor_fee_settings_entry_round_trips_through_typed_xrp_helpers() {
        let entry = ConstructorFeeSettingsEntry::XrpDrops {
            base_fee_drops: 11,
            reserve_base_drops: 22,
            reserve_increment_drops: 33,
        };

        let decoded =
            decode_constructor_fee_settings_entry(&encode_constructor_fee_settings_entry(entry))
                .expect("encoded constructor xrp fee settings should decode");

        assert_eq!(decoded, entry);
    }

    #[test]
    fn constructor_fee_settings_shape_follows_xrp_fees_amendment() {
        assert_eq!(
            make_constructor_fee_settings_entry(10, 20, 30, &[feature_xrp_fees()]),
            ConstructorFeeSettingsEntry::XrpDrops {
                base_fee_drops: 10,
                reserve_base_drops: 20,
                reserve_increment_drops: 30,
            }
        );
        assert_eq!(
            make_constructor_fee_settings_entry(10, 20, 30, &[Uint256::from_array([0xAB; 32])]),
            ConstructorFeeSettingsEntry::Legacy {
                base_fee: 10,
                reference_fee_units: REFERENCE_FEE_UNITS_DEPRECATED,
                reserve_base: Some(20),
                reserve_increment: Some(30),
            }
        );
    }

    #[test]
    fn constructor_fee_settings_entry_rejects_mixed_legacy_and_xrp_shapes() {
        let mut bytes =
            encode_constructor_fee_settings_entry(ConstructorFeeSettingsEntry::Legacy {
                base_fee: 10,
                reference_fee_units: REFERENCE_FEE_UNITS_DEPRECATED,
                reserve_base: Some(20),
                reserve_increment: Some(30),
            });
        // Remove trailing OBJECT_END, insert XRP field, then re-add OBJECT_END
        bytes.pop(); // remove 0xE1
        bytes.extend_from_slice(&encode_native_amount(22, 11));
        bytes.push(0xE1); // re-add OBJECT_END

        let error = decode_constructor_fee_settings_entry(&bytes)
            .expect_err("mixed constructor fee settings should fail");
        assert_eq!(error, ConstructorFeeSettingsDecodeError::MixedFeeFormats);
    }

    #[test]
    fn constructor_fee_settings_entry_requires_reference_fee_units_for_legacy_shape() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&encode_u16(1, LT_FEE_SETTINGS));
        bytes.extend_from_slice(&encode_u64(5, 10));
        bytes.extend_from_slice(&encode_u32(31, 20));
        bytes.extend_from_slice(&encode_u32(32, 30));
        bytes.push(super::OBJECT_END);

        let error = decode_constructor_fee_settings_entry(&bytes)
            .expect_err("legacy constructor fee settings without reference fee units must fail");
        assert_eq!(
            error,
            ConstructorFeeSettingsDecodeError::MissingField("sfReferenceFeeUnits")
        );
    }

    #[test]
    fn constructor_fee_settings_entry_rejects_non_native_xrp_amounts() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&encode_u16(1, LT_FEE_SETTINGS));
        bytes.extend_from_slice(&field_id(STI_AMOUNT, 22));
        bytes.extend_from_slice(&0x8000_0000_0000_0001u64.to_be_bytes());
        bytes.extend_from_slice(&[0u8; 40]);
        bytes.extend_from_slice(&encode_native_amount(23, 22));
        bytes.extend_from_slice(&encode_native_amount(24, 33));
        bytes.push(super::OBJECT_END);

        let error = decode_constructor_fee_settings_entry(&bytes)
            .expect_err("issued constructor xrp fee amount must fail");
        assert_eq!(
            error,
            ConstructorFeeSettingsDecodeError::NonNativeAmount("sfBaseFeeDrops")
        );
    }

    #[test]
    fn constructor_fee_settings_entry_rejects_negative_xrp_amounts() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&encode_u16(1, LT_FEE_SETTINGS));
        bytes.extend_from_slice(&field_id(STI_AMOUNT, 22));
        bytes.extend_from_slice(&11u64.to_be_bytes());
        bytes.extend_from_slice(&encode_native_amount(23, 22));
        bytes.extend_from_slice(&encode_native_amount(24, 33));
        bytes.push(super::OBJECT_END);

        let error = decode_constructor_fee_settings_entry(&bytes)
            .expect_err("negative constructor xrp fee amount must fail");
        assert_eq!(
            error,
            ConstructorFeeSettingsDecodeError::NegativeAmount("sfBaseFeeDrops")
        );
    }
}
