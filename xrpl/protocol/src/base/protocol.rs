//! Protocol-wide constants from `xrpl/protocol/Protocol.h`.

use basics::base_uint::Uint256;

use crate::{Bips, Bips32, TenthBips, TenthBips32, ValueUnit};

pub const TX_MIN_SIZE_BYTES: usize = 32;
pub const TX_MAX_SIZE_BYTES: usize = 1024 * 1024;
pub const UNFUNDED_OFFER_REMOVE_LIMIT: usize = 1000;
pub const EXPIRED_OFFER_REMOVE_LIMIT: usize = 256;
pub const OVERSIZE_METADATA_CAP: usize = 5200;
pub const DIR_NODE_MAX_ENTRIES: usize = 32;
pub const DIR_NODE_MAX_PAGES: u64 = 262_144;
pub const DIR_MAX_TOKENS_PER_PAGE: usize = 32;
pub const MAX_DELETABLE_DIR_ENTRIES: usize = 1000;
pub const MAX_TOKEN_OFFER_CANCEL_COUNT: usize = 500;
pub const MAX_DELETABLE_TOKEN_OFFER_ENTRIES: usize = 500;
pub const MAX_TRANSFER_FEE: u16 = 50_000;
pub const BIPS_PER_UNITY: Bips32 = ValueUnit::new(10_000);
pub const TENTH_BIPS_PER_UNITY: TenthBips32 = ValueUnit::new(100_000);
pub const MAX_TOKEN_URI_LENGTH: usize = 256;
pub const MAX_DID_DOCUMENT_LENGTH: usize = 256;
pub const MAX_DID_URI_LENGTH: usize = 256;
pub const MAX_DID_DATA_LENGTH: usize = 256;
pub const MAX_DOMAIN_LENGTH: usize = 256;
pub const MAX_CREDENTIAL_URI_LENGTH: usize = 256;
pub const MAX_CREDENTIAL_TYPE_LENGTH: usize = 64;
pub const MAX_CREDENTIALS_ARRAY_SIZE: usize = 8;
pub const MAX_PERMISSIONED_DOMAIN_CREDENTIALS_ARRAY_SIZE: usize = 10;
pub const MAX_MP_TOKEN_METADATA_LENGTH: usize = 1024;
pub const MAX_MP_TOKEN_AMOUNT: u64 = 0x7FFF_FFFF_FFFF_FFFF;
pub const MAX_DATA_PAYLOAD_LENGTH: usize = 256;
pub const VAULT_STRATEGY_FIRST_COME_FIRST_SERVE: u8 = 1;
pub const VAULT_DEFAULT_IOU_SCALE: u8 = 6;
pub const VAULT_MAXIMUM_IOU_SCALE: u8 = 18;
pub const MAX_ASSET_CHECK_DEPTH: u8 = 5;
pub type LedgerIndex = u32;
pub type TxID = Uint256;
pub const FLAG_LEDGER_INTERVAL: u32 = 256;
pub const MAX_DELETABLE_AMM_TRUST_LINES: u16 = 512;
pub const MAX_ORACLE_URI: usize = 256;
pub const MAX_ORACLE_PROVIDER: usize = 256;
pub const MAX_ORACLE_DATA_SERIES: usize = 10;
pub const MAX_ORACLE_SYMBOL_CLASS: usize = 16;
pub const MAX_LAST_UPDATE_TIME_DELTA: usize = 300;
pub const MAX_PRICE_SCALE: usize = 20;
pub const MAX_TRIM: usize = 25;
pub const PERMISSION_MAX_SIZE: usize = 10;
pub const MAX_BATCH_TX_COUNT: usize = 8;

pub mod lending {
    use crate::{TenthBips16, TenthBips32, ValueUnit, percentage_to_tenth_bips};

    pub const MAX_MANAGEMENT_FEE_RATE: TenthBips16 = ValueUnit::new(10_000);
    pub const MAX_COVER_RATE: TenthBips32 = percentage_to_tenth_bips(100);
    pub const MAX_OVERPAYMENT_FEE: TenthBips32 = percentage_to_tenth_bips(100);
    pub const MAX_INTEREST_RATE: TenthBips32 = percentage_to_tenth_bips(100);
    pub const MAX_LATE_INTEREST_RATE: TenthBips32 = percentage_to_tenth_bips(100);
    pub const MAX_CLOSE_INTEREST_RATE: TenthBips32 = percentage_to_tenth_bips(100);
    pub const MAX_OVERPAYMENT_INTEREST_RATE: TenthBips32 = percentage_to_tenth_bips(100);
    pub const LOAN_PAYMENTS_PER_FEE_INCREMENT: i32 = 5;
    pub const LOAN_MAXIMUM_PAYMENTS_PER_TRANSACTION: i32 = 100;
}

pub const fn percentage_to_bips(percentage: u32) -> Bips32 {
    ValueUnit::new(percentage * BIPS_PER_UNITY.value() / 100)
}

pub const fn percentage_to_tenth_bips(percentage: u32) -> TenthBips32 {
    ValueUnit::new(percentage * TENTH_BIPS_PER_UNITY.value() / 100)
}

pub fn bips_of_value<T, TBips>(value: T, bips: Bips<TBips>) -> T
where
    T: Copy + std::ops::Mul<TBips, Output = T> + std::ops::Div<TBips, Output = T>,
    TBips: Copy + From<u16>,
{
    value * bips.value() / TBips::from(BIPS_PER_UNITY.value() as u16)
}

pub fn tenth_bips_of_value<T, TBips>(value: T, bips: TenthBips<TBips>) -> T
where
    T: Copy + std::ops::Mul<TBips, Output = T> + std::ops::Div<TBips, Output = T>,
    TBips: Copy + From<u32>,
{
    value * bips.value() / TBips::from(TENTH_BIPS_PER_UNITY.value())
}

pub fn is_voting_ledger(seq: LedgerIndex) -> bool {
    seq.is_multiple_of(FLAG_LEDGER_INTERVAL)
}

pub fn is_flag_ledger(seq: LedgerIndex) -> bool {
    seq.is_multiple_of(FLAG_LEDGER_INTERVAL)
}
