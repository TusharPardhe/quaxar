//! Public transaction-flag catalog for `xrpl/protocol`.
//!
//! This mirrors the reference `xrpl/protocol/TxFlags.h` bit values, adds the
//! compatibility aliases the Rust tree still expects to see, and exposes the
//! static flag-map getters used by the reference catalog API.

use std::{collections::BTreeMap, sync::OnceLock};

pub type FlagValue = u32;
pub type FlagMap = BTreeMap<String, FlagValue>;
pub type FlagMapPairList = Vec<(String, FlagMap)>;

pub const FULLY_CANONICAL_SIGNATURE_FLAG: FlagValue = 0x8000_0000;
pub const INNER_BATCH_TRANSACTION_FLAG: FlagValue = 0x4000_0000;
pub const UNIVERSAL_TRANSACTION_FLAGS: FlagValue =
    FULLY_CANONICAL_SIGNATURE_FLAG | INNER_BATCH_TRANSACTION_FLAG;
pub const UNIVERSAL_TRANSACTION_FLAGS_MASK: FlagValue = !UNIVERSAL_TRANSACTION_FLAGS;

pub const ACCOUNT_SET_REQUIRE_DEST_TAG_FLAG: FlagValue = 0x0001_0000;
pub const ACCOUNT_SET_OPTIONAL_DEST_TAG_FLAG: FlagValue = 0x0002_0000;
pub const ACCOUNT_SET_REQUIRE_AUTH_FLAG: FlagValue = 0x0004_0000;
pub const ACCOUNT_SET_OPTIONAL_AUTH_FLAG: FlagValue = 0x0008_0000;
pub const ACCOUNT_SET_DISALLOW_XRP_FLAG: FlagValue = 0x0010_0000;
pub const ACCOUNT_SET_ALLOW_XRP_FLAG: FlagValue = 0x0020_0000;
pub const ACCOUNT_SET_FLAGS: FlagValue = ACCOUNT_SET_REQUIRE_DEST_TAG_FLAG
    | ACCOUNT_SET_OPTIONAL_DEST_TAG_FLAG
    | ACCOUNT_SET_REQUIRE_AUTH_FLAG
    | ACCOUNT_SET_OPTIONAL_AUTH_FLAG
    | ACCOUNT_SET_DISALLOW_XRP_FLAG
    | ACCOUNT_SET_ALLOW_XRP_FLAG;
pub const ACCOUNT_SET_FLAGS_MASK: FlagValue = transaction_flags_mask(ACCOUNT_SET_FLAGS);

pub const OFFER_CREATE_PASSIVE_FLAG: FlagValue = 0x0001_0000;
pub const OFFER_CREATE_IMMEDIATE_OR_CANCEL_FLAG: FlagValue = 0x0002_0000;
pub const OFFER_CREATE_FILL_OR_KILL_FLAG: FlagValue = 0x0004_0000;
pub const OFFER_CREATE_SELL_FLAG: FlagValue = 0x0008_0000;
pub const OFFER_CREATE_HYBRID_FLAG: FlagValue = 0x0010_0000;
pub const OFFER_CREATE_FLAGS: FlagValue = OFFER_CREATE_PASSIVE_FLAG
    | OFFER_CREATE_IMMEDIATE_OR_CANCEL_FLAG
    | OFFER_CREATE_FILL_OR_KILL_FLAG
    | OFFER_CREATE_SELL_FLAG
    | OFFER_CREATE_HYBRID_FLAG;
pub const OFFER_CREATE_FLAGS_MASK: FlagValue = transaction_flags_mask(OFFER_CREATE_FLAGS);

pub const PAYMENT_NO_RIPPLE_DIRECT_FLAG: FlagValue = 0x0001_0000;
pub const PAYMENT_PARTIAL_PAYMENT_FLAG: FlagValue = 0x0002_0000;
pub const PAYMENT_LIMIT_QUALITY_FLAG: FlagValue = 0x0004_0000;
pub const PAYMENT_FLAGS: FlagValue =
    PAYMENT_NO_RIPPLE_DIRECT_FLAG | PAYMENT_PARTIAL_PAYMENT_FLAG | PAYMENT_LIMIT_QUALITY_FLAG;
pub const PAYMENT_FLAGS_MASK: FlagValue = transaction_flags_mask(PAYMENT_FLAGS);

pub const TRUST_SET_SET_AUTH_FLAG: FlagValue = 0x0001_0000;
pub const TRUST_SET_SET_NO_RIPPLE_FLAG: FlagValue = 0x0002_0000;
pub const TRUST_SET_CLEAR_NO_RIPPLE_FLAG: FlagValue = 0x0004_0000;
pub const TRUST_SET_SET_FREEZE_FLAG: FlagValue = 0x0010_0000;
pub const TRUST_SET_CLEAR_FREEZE_FLAG: FlagValue = 0x0020_0000;
pub const TRUST_SET_SET_DEEP_FREEZE_FLAG: FlagValue = 0x0040_0000;
pub const TRUST_SET_CLEAR_DEEP_FREEZE_FLAG: FlagValue = 0x0080_0000;
pub const TRUST_SET_FLAGS: FlagValue = TRUST_SET_SET_AUTH_FLAG
    | TRUST_SET_SET_NO_RIPPLE_FLAG
    | TRUST_SET_CLEAR_NO_RIPPLE_FLAG
    | TRUST_SET_SET_FREEZE_FLAG
    | TRUST_SET_CLEAR_FREEZE_FLAG
    | TRUST_SET_SET_DEEP_FREEZE_FLAG
    | TRUST_SET_CLEAR_DEEP_FREEZE_FLAG;
pub const TRUST_SET_FLAGS_MASK: FlagValue = transaction_flags_mask(TRUST_SET_FLAGS);

pub const ENABLE_AMENDMENT_GOT_MAJORITY_FLAG: FlagValue = 0x0001_0000;
pub const ENABLE_AMENDMENT_LOST_MAJORITY_FLAG: FlagValue = 0x0002_0000;
pub const ENABLE_AMENDMENT_FLAGS: FlagValue =
    ENABLE_AMENDMENT_GOT_MAJORITY_FLAG | ENABLE_AMENDMENT_LOST_MAJORITY_FLAG;
pub const ENABLE_AMENDMENT_FLAGS_MASK: FlagValue = transaction_flags_mask(ENABLE_AMENDMENT_FLAGS);

pub const PAYMENT_CHANNEL_CLAIM_RENEW_FLAG: FlagValue = 0x0001_0000;
pub const PAYMENT_CHANNEL_CLAIM_CLOSE_FLAG: FlagValue = 0x0002_0000;
pub const PAYMENT_CHANNEL_CLAIM_FLAGS: FlagValue =
    PAYMENT_CHANNEL_CLAIM_RENEW_FLAG | PAYMENT_CHANNEL_CLAIM_CLOSE_FLAG;
pub const PAYMENT_CHANNEL_CLAIM_FLAGS_MASK: FlagValue =
    transaction_flags_mask(PAYMENT_CHANNEL_CLAIM_FLAGS);

pub const NF_TOKEN_BURNABLE_FLAG: FlagValue = 0x0000_0001;
pub const NF_TOKEN_ONLY_XRP_FLAG: FlagValue = 0x0000_0002;
pub const NF_TOKEN_TRUST_LINE_FLAG: FlagValue = 0x0000_0004;
pub const NF_TOKEN_TRANSFERABLE_FLAG: FlagValue = 0x0000_0008;
pub const NF_TOKEN_MUTABLE_FLAG: FlagValue = 0x0000_0010;
pub const NF_TOKEN_CREATE_OFFER_SELL_FLAG: FlagValue = 0x0000_0001;
pub const NF_TOKEN_MINT_FLAGS_WITHOUT_MUTABLE: FlagValue = !(UNIVERSAL_TRANSACTION_FLAGS
    | NF_TOKEN_BURNABLE_FLAG
    | NF_TOKEN_ONLY_XRP_FLAG
    | NF_TOKEN_TRANSFERABLE_FLAG);
pub const NF_TOKEN_MINT_OLD_FLAGS: FlagValue = !(UNIVERSAL_TRANSACTION_FLAGS
    | NF_TOKEN_BURNABLE_FLAG
    | NF_TOKEN_ONLY_XRP_FLAG
    | NF_TOKEN_TRANSFERABLE_FLAG
    | NF_TOKEN_TRUST_LINE_FLAG);
pub const NF_TOKEN_MINT_OLD_FLAGS_WITH_MUTABLE: FlagValue = !(UNIVERSAL_TRANSACTION_FLAGS
    | NF_TOKEN_BURNABLE_FLAG
    | NF_TOKEN_ONLY_XRP_FLAG
    | NF_TOKEN_TRANSFERABLE_FLAG
    | NF_TOKEN_TRUST_LINE_FLAG
    | NF_TOKEN_MUTABLE_FLAG);

pub const MPT_CAN_LOCK_FLAG: FlagValue = 0x0000_0002;
pub const MPT_REQUIRE_AUTH_FLAG: FlagValue = 0x0000_0004;
pub const MPT_CAN_ESCROW_FLAG: FlagValue = 0x0000_0008;
pub const MPT_CAN_TRADE_FLAG: FlagValue = 0x0000_0010;
pub const MPT_CAN_TRANSFER_FLAG: FlagValue = 0x0000_0020;
pub const MPT_CAN_CLAWBACK_FLAG: FlagValue = 0x0000_0040;
pub const MPT_CAN_HOLD_CONFIDENTIAL_BALANCE_FLAG: FlagValue = 0x0000_0080;
pub const MPT_ISSUANCE_CREATE_FLAGS: FlagValue = MPT_CAN_LOCK_FLAG
    | MPT_REQUIRE_AUTH_FLAG
    | MPT_CAN_ESCROW_FLAG
    | MPT_CAN_TRADE_FLAG
    | MPT_CAN_TRANSFER_FLAG
    | MPT_CAN_CLAWBACK_FLAG
    | MPT_CAN_HOLD_CONFIDENTIAL_BALANCE_FLAG;
pub const MPT_ISSUANCE_CREATE_FLAGS_MASK: FlagValue =
    transaction_flags_mask(MPT_ISSUANCE_CREATE_FLAGS);

pub const MPT_UNAUTHORIZE_FLAG: FlagValue = 0x0000_0001;
pub const MPT_UNAUTHORIZE_FLAGS_MASK: FlagValue = transaction_flags_mask(MPT_UNAUTHORIZE_FLAG);

pub const MPT_LOCK_FLAG: FlagValue = 0x0000_0001;
pub const MPT_UNLOCK_FLAG: FlagValue = 0x0000_0002;
pub const MPT_ISSUANCE_SET_FLAGS: FlagValue = MPT_LOCK_FLAG | MPT_UNLOCK_FLAG;
pub const MPT_ISSUANCE_SET_FLAGS_MASK: FlagValue = transaction_flags_mask(MPT_ISSUANCE_SET_FLAGS);

pub const AMM_LP_TOKEN_FLAG: FlagValue = 0x0001_0000;
pub const AMM_WITHDRAW_ALL_FLAG: FlagValue = 0x0002_0000;
pub const AMM_ONE_ASSET_WITHDRAW_ALL_FLAG: FlagValue = 0x0004_0000;
pub const AMM_SINGLE_ASSET_FLAG: FlagValue = 0x0008_0000;
pub const AMM_TWO_ASSET_FLAG: FlagValue = 0x0010_0000;
pub const AMM_ONE_ASSET_LP_TOKEN_FLAG: FlagValue = 0x0020_0000;
pub const AMM_LIMIT_LP_TOKEN_FLAG: FlagValue = 0x0040_0000;
pub const AMM_TWO_ASSET_IF_EMPTY_FLAG: FlagValue = 0x0080_0000;
pub const AMM_DEPOSIT_FLAGS: FlagValue = AMM_LP_TOKEN_FLAG
    | AMM_SINGLE_ASSET_FLAG
    | AMM_TWO_ASSET_FLAG
    | AMM_ONE_ASSET_LP_TOKEN_FLAG
    | AMM_LIMIT_LP_TOKEN_FLAG
    | AMM_TWO_ASSET_IF_EMPTY_FLAG;
pub const AMM_DEPOSIT_FLAGS_MASK: FlagValue = transaction_flags_mask(AMM_DEPOSIT_FLAGS);
pub const AMM_WITHDRAW_FLAGS: FlagValue = AMM_LP_TOKEN_FLAG
    | AMM_WITHDRAW_ALL_FLAG
    | AMM_ONE_ASSET_WITHDRAW_ALL_FLAG
    | AMM_SINGLE_ASSET_FLAG
    | AMM_TWO_ASSET_FLAG
    | AMM_ONE_ASSET_LP_TOKEN_FLAG
    | AMM_LIMIT_LP_TOKEN_FLAG;
pub const AMM_WITHDRAW_FLAGS_MASK: FlagValue = transaction_flags_mask(AMM_WITHDRAW_FLAGS);

pub const AMM_CLAWBACK_TWO_ASSETS_FLAG: FlagValue = 0x0000_0001;
pub const AMM_CLAWBACK_FLAGS_MASK: FlagValue = transaction_flags_mask(AMM_CLAWBACK_TWO_ASSETS_FLAG);

pub const XCHAIN_MODIFY_BRIDGE_CLEAR_ACCOUNT_CREATE_AMOUNT_FLAG: FlagValue = 0x0001_0000;
pub const XCHAIN_MODIFY_BRIDGE_FLAGS_MASK: FlagValue =
    transaction_flags_mask(XCHAIN_MODIFY_BRIDGE_CLEAR_ACCOUNT_CREATE_AMOUNT_FLAG);

pub const VAULT_PRIVATE_FLAG: FlagValue = 0x0001_0000;
pub const VAULT_SHARE_NON_TRANSFERABLE_FLAG: FlagValue = 0x0002_0000;
pub const VAULT_CREATE_FLAGS: FlagValue = VAULT_PRIVATE_FLAG | VAULT_SHARE_NON_TRANSFERABLE_FLAG;
pub const VAULT_CREATE_FLAGS_MASK: FlagValue = transaction_flags_mask(VAULT_CREATE_FLAGS);

pub const BATCH_ALL_OR_NOTHING_FLAG: FlagValue = 0x0001_0000;
pub const BATCH_ONLY_ONE_FLAG: FlagValue = 0x0002_0000;
pub const BATCH_UNTIL_FAILURE_FLAG: FlagValue = 0x0004_0000;
pub const BATCH_INDEPENDENT_FLAG: FlagValue = 0x0008_0000;
pub const BATCH_FLAGS: FlagValue = BATCH_ALL_OR_NOTHING_FLAG
    | BATCH_ONLY_ONE_FLAG
    | BATCH_UNTIL_FAILURE_FLAG
    | BATCH_INDEPENDENT_FLAG;
pub const BATCH_FLAGS_MASK: FlagValue =
    transaction_flags_mask_with_adjustment(BATCH_FLAGS, INNER_BATCH_TRANSACTION_FLAG);

pub const LOAN_OVERPAYMENT_FLAG: FlagValue = 0x0001_0000;
pub const LOAN_FULL_PAYMENT_FLAG: FlagValue = 0x0002_0000;
pub const LOAN_LATE_PAYMENT_FLAG: FlagValue = 0x0004_0000;
pub const LOAN_SET_FLAGS: FlagValue = LOAN_OVERPAYMENT_FLAG;
pub const LOAN_SET_FLAGS_MASK: FlagValue = transaction_flags_mask(LOAN_SET_FLAGS);
pub const LOAN_PAY_FLAGS: FlagValue =
    LOAN_OVERPAYMENT_FLAG | LOAN_FULL_PAYMENT_FLAG | LOAN_LATE_PAYMENT_FLAG;
pub const LOAN_PAY_FLAGS_MASK: FlagValue = transaction_flags_mask(LOAN_PAY_FLAGS);
pub const LOAN_MANAGE_DEFAULT_FLAG: FlagValue = 0x0001_0000;
pub const LOAN_MANAGE_IMPAIR_FLAG: FlagValue = 0x0002_0000;
pub const LOAN_MANAGE_UNIMPAIR_FLAG: FlagValue = 0x0004_0000;
pub const LOAN_MANAGE_FLAGS: FlagValue =
    LOAN_MANAGE_DEFAULT_FLAG | LOAN_MANAGE_IMPAIR_FLAG | LOAN_MANAGE_UNIMPAIR_FLAG;
pub const LOAN_MANAGE_FLAGS_MASK: FlagValue = transaction_flags_mask(LOAN_MANAGE_FLAGS);

pub const MPT_PAYMENT_MASK: FlagValue =
    !(UNIVERSAL_TRANSACTION_FLAGS | PAYMENT_PARTIAL_PAYMENT_FLAG);
pub const TRUST_SET_PERMISSION_MASK: FlagValue = !(UNIVERSAL_TRANSACTION_FLAGS
    | TRUST_SET_SET_AUTH_FLAG
    | TRUST_SET_SET_FREEZE_FLAG
    | TRUST_SET_CLEAR_FREEZE_FLAG);
pub const WITHDRAW_SUB_TX_FLAGS: FlagValue = AMM_LP_TOKEN_FLAG
    | AMM_SINGLE_ASSET_FLAG
    | AMM_TWO_ASSET_FLAG
    | AMM_ONE_ASSET_LP_TOKEN_FLAG
    | AMM_LIMIT_LP_TOKEN_FLAG
    | AMM_WITHDRAW_ALL_FLAG
    | AMM_ONE_ASSET_WITHDRAW_ALL_FLAG;
pub const DEPOSIT_SUB_TX_FLAGS: FlagValue = AMM_LP_TOKEN_FLAG
    | AMM_SINGLE_ASSET_FLAG
    | AMM_TWO_ASSET_FLAG
    | AMM_ONE_ASSET_LP_TOKEN_FLAG
    | AMM_LIMIT_LP_TOKEN_FLAG
    | AMM_TWO_ASSET_IF_EMPTY_FLAG;

pub const MPT_CAN_MUTATE_CAN_LOCK_FLAG: FlagValue = 0x0000_0002;
pub const MPT_CAN_MUTATE_REQUIRE_AUTH_FLAG: FlagValue = 0x0000_0004;
pub const MPT_CAN_MUTATE_CAN_ESCROW_FLAG: FlagValue = 0x0000_0008;
pub const MPT_CAN_MUTATE_CAN_TRADE_FLAG: FlagValue = 0x0000_0010;
pub const MPT_CAN_MUTATE_CAN_TRANSFER_FLAG: FlagValue = 0x0000_0020;
pub const MPT_CAN_MUTATE_CAN_CLAWBACK_FLAG: FlagValue = 0x0000_0040;
pub const MPT_CAN_MUTATE_METADATA_FLAG: FlagValue = 0x0001_0000;
pub const MPT_CAN_MUTATE_TRANSFER_FEE_FLAG: FlagValue = 0x0002_0000;
pub const MPT_ISSUANCE_CREATE_MUTABLE_MASK: FlagValue = !(MPT_CAN_MUTATE_CAN_LOCK_FLAG
    | MPT_CAN_MUTATE_REQUIRE_AUTH_FLAG
    | MPT_CAN_MUTATE_CAN_ESCROW_FLAG
    | MPT_CAN_MUTATE_CAN_TRADE_FLAG
    | MPT_CAN_MUTATE_CAN_TRANSFER_FLAG
    | MPT_CAN_MUTATE_CAN_CLAWBACK_FLAG
    | MPT_CAN_MUTATE_METADATA_FLAG
    | MPT_CAN_MUTATE_TRANSFER_FEE_FLAG);

pub const MPT_SET_CAN_LOCK_FLAG: FlagValue = 0x0000_0001;
pub const MPT_CLEAR_CAN_LOCK_FLAG: FlagValue = 0x0000_0002;
pub const MPT_SET_REQUIRE_AUTH_FLAG: FlagValue = 0x0000_0004;
pub const MPT_CLEAR_REQUIRE_AUTH_FLAG: FlagValue = 0x0000_0008;
pub const MPT_SET_CAN_ESCROW_FLAG: FlagValue = 0x0000_0010;
pub const MPT_CLEAR_CAN_ESCROW_FLAG: FlagValue = 0x0000_0020;
pub const MPT_SET_CAN_TRADE_FLAG: FlagValue = 0x0000_0040;
pub const MPT_CLEAR_CAN_TRADE_FLAG: FlagValue = 0x0000_0080;
pub const MPT_SET_CAN_TRANSFER_FLAG: FlagValue = 0x0000_0100;
pub const MPT_CLEAR_CAN_TRANSFER_FLAG: FlagValue = 0x0000_0200;
pub const MPT_SET_CAN_CLAWBACK_FLAG: FlagValue = 0x0000_0400;
pub const MPT_CLEAR_CAN_CLAWBACK_FLAG: FlagValue = 0x0000_0800;
pub const MPT_ISSUANCE_SET_MUTABLE_MASK: FlagValue = !(MPT_SET_CAN_LOCK_FLAG
    | MPT_CLEAR_CAN_LOCK_FLAG
    | MPT_SET_REQUIRE_AUTH_FLAG
    | MPT_CLEAR_REQUIRE_AUTH_FLAG
    | MPT_SET_CAN_ESCROW_FLAG
    | MPT_CLEAR_CAN_ESCROW_FLAG
    | MPT_SET_CAN_TRADE_FLAG
    | MPT_CLEAR_CAN_TRADE_FLAG
    | MPT_SET_CAN_TRANSFER_FLAG
    | MPT_CLEAR_CAN_TRANSFER_FLAG
    | MPT_SET_CAN_CLAWBACK_FLAG
    | MPT_CLEAR_CAN_CLAWBACK_FLAG);

pub const ASF_REQUIRE_DEST_FLAG: FlagValue = 1;
pub const ASF_REQUIRE_AUTH_FLAG: FlagValue = 2;
pub const ASF_DISALLOW_XRP_FLAG: FlagValue = 3;
pub const ASF_DISABLE_MASTER_FLAG: FlagValue = 4;
pub const ASF_ACCOUNT_TXN_ID_FLAG: FlagValue = 5;
pub const ASF_NO_FREEZE_FLAG: FlagValue = 6;
pub const ASF_GLOBAL_FREEZE_FLAG: FlagValue = 7;
pub const ASF_DEFAULT_RIPPLE_FLAG: FlagValue = 8;
pub const ASF_DEPOSIT_AUTH_FLAG: FlagValue = 9;
pub const ASF_AUTHORIZED_NF_TOKEN_MINTER_FLAG: FlagValue = 10;
pub const ASF_DISALLOW_INCOMING_NF_TOKEN_OFFER_FLAG: FlagValue = 12;
pub const ASF_DISALLOW_INCOMING_CHECK_FLAG: FlagValue = 13;
pub const ASF_DISALLOW_INCOMING_PAY_CHAN_FLAG: FlagValue = 14;
pub const ASF_DISALLOW_INCOMING_TRUSTLINE_FLAG: FlagValue = 15;
pub const ASF_ALLOW_TRUST_LINE_CLAWBACK_FLAG: FlagValue = 16;
pub const ASF_ALLOW_TRUST_LINE_LOCKING_FLAG: FlagValue = 17;

macro_rules! alias_consts {
    ($(($source:ident => $alias:ident)),* $(,)?) => {
        $(pub use self::$source as $alias;)*
    };
}

alias_consts!(
    (FULLY_CANONICAL_SIGNATURE_FLAG => tfFullyCanonicalSig),
    (INNER_BATCH_TRANSACTION_FLAG => tfInnerBatchTxn),
    (UNIVERSAL_TRANSACTION_FLAGS => tfUniversal),
    (UNIVERSAL_TRANSACTION_FLAGS_MASK => tfUniversalMask),
    (ACCOUNT_SET_REQUIRE_DEST_TAG_FLAG => tfRequireDestTag),
    (ACCOUNT_SET_OPTIONAL_DEST_TAG_FLAG => tfOptionalDestTag),
    (ACCOUNT_SET_REQUIRE_AUTH_FLAG => tfRequireAuth),
    (ACCOUNT_SET_OPTIONAL_AUTH_FLAG => tfOptionalAuth),
    (ACCOUNT_SET_DISALLOW_XRP_FLAG => tfDisallowXRP),
    (ACCOUNT_SET_ALLOW_XRP_FLAG => tfAllowXRP),
    (ACCOUNT_SET_FLAGS_MASK => tfAccountSetMask),
    (OFFER_CREATE_PASSIVE_FLAG => tfPassive),
    (OFFER_CREATE_IMMEDIATE_OR_CANCEL_FLAG => tfImmediateOrCancel),
    (OFFER_CREATE_FILL_OR_KILL_FLAG => tfFillOrKill),
    (OFFER_CREATE_SELL_FLAG => tfSell),
    (OFFER_CREATE_HYBRID_FLAG => tfHybrid),
    (OFFER_CREATE_FLAGS_MASK => tfOfferCreateMask),
    (PAYMENT_NO_RIPPLE_DIRECT_FLAG => tfNoRippleDirect),
    (PAYMENT_PARTIAL_PAYMENT_FLAG => tfPartialPayment),
    (PAYMENT_LIMIT_QUALITY_FLAG => tfLimitQuality),
    (PAYMENT_FLAGS_MASK => tfPaymentMask),
    (TRUST_SET_SET_AUTH_FLAG => tfSetfAuth),
    (TRUST_SET_SET_NO_RIPPLE_FLAG => tfSetNoRipple),
    (TRUST_SET_CLEAR_NO_RIPPLE_FLAG => tfClearNoRipple),
    (TRUST_SET_SET_FREEZE_FLAG => tfSetFreeze),
    (TRUST_SET_CLEAR_FREEZE_FLAG => tfClearFreeze),
    (TRUST_SET_SET_DEEP_FREEZE_FLAG => tfSetDeepFreeze),
    (TRUST_SET_CLEAR_DEEP_FREEZE_FLAG => tfClearDeepFreeze),
    (TRUST_SET_FLAGS_MASK => tfTrustSetMask),
    (ENABLE_AMENDMENT_GOT_MAJORITY_FLAG => tfGotMajority),
    (ENABLE_AMENDMENT_LOST_MAJORITY_FLAG => tfLostMajority),
    (ENABLE_AMENDMENT_FLAGS_MASK => tfEnableAmendmentMask),
    (PAYMENT_CHANNEL_CLAIM_RENEW_FLAG => tfRenew),
    (PAYMENT_CHANNEL_CLAIM_CLOSE_FLAG => tfClose),
    (PAYMENT_CHANNEL_CLAIM_FLAGS_MASK => tfPaymentChannelClaimMask),
    (NF_TOKEN_BURNABLE_FLAG => tfBurnable),
    (NF_TOKEN_ONLY_XRP_FLAG => tfOnlyXRP),
    (NF_TOKEN_TRUST_LINE_FLAG => tfTrustLine),
    (NF_TOKEN_TRANSFERABLE_FLAG => tfTransferable),
    (NF_TOKEN_MUTABLE_FLAG => tfMutable),
    (NF_TOKEN_CREATE_OFFER_SELL_FLAG => tfSellNFToken),
    (NF_TOKEN_MINT_FLAGS_WITHOUT_MUTABLE => tfNFTokenMintMaskWithoutMutable),
    (NF_TOKEN_MINT_OLD_FLAGS => tfNFTokenMintOldMask),
    (NF_TOKEN_MINT_OLD_FLAGS_WITH_MUTABLE => tfNFTokenMintOldMaskWithMutable),
    (MPT_CAN_LOCK_FLAG => tfMPTCanLock),
    (MPT_REQUIRE_AUTH_FLAG => tfMPTRequireAuth),
    (MPT_CAN_ESCROW_FLAG => tfMPTCanEscrow),
    (MPT_CAN_TRADE_FLAG => tfMPTCanTrade),
    (MPT_CAN_TRANSFER_FLAG => tfMPTCanTransfer),
    (MPT_CAN_CLAWBACK_FLAG => tfMPTCanClawback),
    (MPT_CAN_HOLD_CONFIDENTIAL_BALANCE_FLAG => tfMPTCanHoldConfidentialBalance),
    (MPT_ISSUANCE_CREATE_FLAGS_MASK => tfMPTokenIssuanceCreateMask),
    (MPT_UNAUTHORIZE_FLAG => tfMPTUnauthorize),
    (MPT_UNAUTHORIZE_FLAGS_MASK => tfMPTUnauthorizeMask),
    (MPT_LOCK_FLAG => tfMPTLock),
    (MPT_UNLOCK_FLAG => tfMPTUnlock),
    (MPT_ISSUANCE_SET_FLAGS_MASK => tfMPTokenIssuanceSetMask),
    (AMM_LP_TOKEN_FLAG => tfLPToken),
    (AMM_WITHDRAW_ALL_FLAG => tfWithdrawAll),
    (AMM_ONE_ASSET_WITHDRAW_ALL_FLAG => tfOneAssetWithdrawAll),
    (AMM_SINGLE_ASSET_FLAG => tfSingleAsset),
    (AMM_TWO_ASSET_FLAG => tfTwoAsset),
    (AMM_ONE_ASSET_LP_TOKEN_FLAG => tfOneAssetLPToken),
    (AMM_LIMIT_LP_TOKEN_FLAG => tfLimitLPToken),
    (AMM_TWO_ASSET_IF_EMPTY_FLAG => tfTwoAssetIfEmpty),
    (AMM_DEPOSIT_FLAGS_MASK => tfAMMDepositMask),
    (AMM_WITHDRAW_FLAGS_MASK => tfAMMWithdrawMask),
    (AMM_CLAWBACK_TWO_ASSETS_FLAG => tfClawTwoAssets),
    (AMM_CLAWBACK_FLAGS_MASK => tfAMMClawbackMask),
    (XCHAIN_MODIFY_BRIDGE_CLEAR_ACCOUNT_CREATE_AMOUNT_FLAG => tfClearAccountCreateAmount),
    (XCHAIN_MODIFY_BRIDGE_FLAGS_MASK => tfXChainModifyBridgeMask),
    (VAULT_PRIVATE_FLAG => tfVaultPrivate),
    (VAULT_SHARE_NON_TRANSFERABLE_FLAG => tfVaultShareNonTransferable),
    (VAULT_CREATE_FLAGS_MASK => tfVaultCreateMask),
    (BATCH_ALL_OR_NOTHING_FLAG => tfAllOrNothing),
    (BATCH_ONLY_ONE_FLAG => tfOnlyOne),
    (BATCH_UNTIL_FAILURE_FLAG => tfUntilFailure),
    (BATCH_INDEPENDENT_FLAG => tfIndependent),
    (BATCH_FLAGS_MASK => tfBatchMask),
    (LOAN_OVERPAYMENT_FLAG => tfLoanOverpayment),
    (LOAN_FULL_PAYMENT_FLAG => tfLoanFullPayment),
    (LOAN_LATE_PAYMENT_FLAG => tfLoanLatePayment),
    (LOAN_SET_FLAGS_MASK => tfLoanSetMask),
    (LOAN_PAY_FLAGS_MASK => tfLoanPayMask),
    (LOAN_MANAGE_DEFAULT_FLAG => tfLoanDefault),
    (LOAN_MANAGE_IMPAIR_FLAG => tfLoanImpair),
    (LOAN_MANAGE_UNIMPAIR_FLAG => tfLoanUnimpair),
    (LOAN_MANAGE_FLAGS_MASK => tfLoanManageMask),
    (MPT_PAYMENT_MASK => tfMPTPaymentMask),
    (TRUST_SET_PERMISSION_MASK => tfTrustSetPermissionMask),
    (WITHDRAW_SUB_TX_FLAGS => tfWithdrawSubTx),
    (DEPOSIT_SUB_TX_FLAGS => tfDepositSubTx),
    (MPT_CAN_MUTATE_CAN_LOCK_FLAG => tmfMPTCanMutateCanLock),
    (MPT_CAN_MUTATE_REQUIRE_AUTH_FLAG => tmfMPTCanMutateRequireAuth),
    (MPT_CAN_MUTATE_CAN_ESCROW_FLAG => tmfMPTCanMutateCanEscrow),
    (MPT_CAN_MUTATE_CAN_TRADE_FLAG => tmfMPTCanMutateCanTrade),
    (MPT_CAN_MUTATE_CAN_TRANSFER_FLAG => tmfMPTCanMutateCanTransfer),
    (MPT_CAN_MUTATE_CAN_CLAWBACK_FLAG => tmfMPTCanMutateCanClawback),
    (MPT_CAN_MUTATE_METADATA_FLAG => tmfMPTCanMutateMetadata),
    (MPT_CAN_MUTATE_TRANSFER_FEE_FLAG => tmfMPTCanMutateTransferFee),
    (MPT_ISSUANCE_CREATE_MUTABLE_MASK => tmfMPTokenIssuanceCreateMutableMask),
    (MPT_SET_CAN_LOCK_FLAG => tmfMPTSetCanLock),
    (MPT_CLEAR_CAN_LOCK_FLAG => tmfMPTClearCanLock),
    (MPT_SET_REQUIRE_AUTH_FLAG => tmfMPTSetRequireAuth),
    (MPT_CLEAR_REQUIRE_AUTH_FLAG => tmfMPTClearRequireAuth),
    (MPT_SET_CAN_ESCROW_FLAG => tmfMPTSetCanEscrow),
    (MPT_CLEAR_CAN_ESCROW_FLAG => tmfMPTClearCanEscrow),
    (MPT_SET_CAN_TRADE_FLAG => tmfMPTSetCanTrade),
    (MPT_CLEAR_CAN_TRADE_FLAG => tmfMPTClearCanTrade),
    (MPT_SET_CAN_TRANSFER_FLAG => tmfMPTSetCanTransfer),
    (MPT_CLEAR_CAN_TRANSFER_FLAG => tmfMPTClearCanTransfer),
    (MPT_SET_CAN_CLAWBACK_FLAG => tmfMPTSetCanClawback),
    (MPT_CLEAR_CAN_CLAWBACK_FLAG => tmfMPTClearCanClawback),
    (MPT_ISSUANCE_SET_MUTABLE_MASK => tmfMPTokenIssuanceSetMutableMask),
    (ASF_REQUIRE_DEST_FLAG => asfRequireDest),
    (ASF_REQUIRE_AUTH_FLAG => asfRequireAuth),
    (ASF_DISALLOW_XRP_FLAG => asfDisallowXRP),
    (ASF_DISABLE_MASTER_FLAG => asfDisableMaster),
    (ASF_ACCOUNT_TXN_ID_FLAG => asfAccountTxnID),
    (ASF_NO_FREEZE_FLAG => asfNoFreeze),
    (ASF_GLOBAL_FREEZE_FLAG => asfGlobalFreeze),
    (ASF_DEFAULT_RIPPLE_FLAG => asfDefaultRipple),
    (ASF_DEPOSIT_AUTH_FLAG => asfDepositAuth),
    (ASF_AUTHORIZED_NF_TOKEN_MINTER_FLAG => asfAuthorizedNFTokenMinter),
    (ASF_DISALLOW_INCOMING_NF_TOKEN_OFFER_FLAG => asfDisallowIncomingNFTokenOffer),
    (ASF_DISALLOW_INCOMING_CHECK_FLAG => asfDisallowIncomingCheck),
    (ASF_DISALLOW_INCOMING_PAY_CHAN_FLAG => asfDisallowIncomingPayChan),
    (ASF_DISALLOW_INCOMING_TRUSTLINE_FLAG => asfDisallowIncomingTrustline),
    (ASF_ALLOW_TRUST_LINE_CLAWBACK_FLAG => asfAllowTrustLineClawback),
    (ASF_ALLOW_TRUST_LINE_LOCKING_FLAG => asfAllowTrustLineLocking),
);

fn make_flag_map(entries: &[(&'static str, FlagValue)]) -> FlagMap {
    entries
        .iter()
        .map(|(name, value)| ((*name).to_string(), *value))
        .collect()
}

pub fn get_universal_flags() -> &'static FlagMap {
    static FLAGS: OnceLock<FlagMap> = OnceLock::new();
    FLAGS.get_or_init(|| {
        make_flag_map(&[
            ("tfFullyCanonicalSig", tfFullyCanonicalSig),
            ("tfInnerBatchTxn", tfInnerBatchTxn),
        ])
    })
}

pub fn get_account_set_flags() -> &'static FlagMap {
    static FLAGS: OnceLock<FlagMap> = OnceLock::new();
    FLAGS.get_or_init(|| {
        make_flag_map(&[
            ("tfRequireDestTag", tfRequireDestTag),
            ("tfOptionalDestTag", tfOptionalDestTag),
            ("tfRequireAuth", tfRequireAuth),
            ("tfOptionalAuth", tfOptionalAuth),
            ("tfDisallowXRP", tfDisallowXRP),
            ("tfAllowXRP", tfAllowXRP),
        ])
    })
}

pub fn get_offer_create_flags() -> &'static FlagMap {
    static FLAGS: OnceLock<FlagMap> = OnceLock::new();
    FLAGS.get_or_init(|| {
        make_flag_map(&[
            ("tfPassive", tfPassive),
            ("tfImmediateOrCancel", tfImmediateOrCancel),
            ("tfFillOrKill", tfFillOrKill),
            ("tfSell", tfSell),
            ("tfHybrid", tfHybrid),
        ])
    })
}

pub fn get_payment_flags() -> &'static FlagMap {
    static FLAGS: OnceLock<FlagMap> = OnceLock::new();
    FLAGS.get_or_init(|| {
        make_flag_map(&[
            ("tfNoRippleDirect", tfNoRippleDirect),
            ("tfPartialPayment", tfPartialPayment),
            ("tfLimitQuality", tfLimitQuality),
        ])
    })
}

pub fn get_trust_set_flags() -> &'static FlagMap {
    static FLAGS: OnceLock<FlagMap> = OnceLock::new();
    FLAGS.get_or_init(|| {
        make_flag_map(&[
            ("tfSetfAuth", tfSetfAuth),
            ("tfSetNoRipple", tfSetNoRipple),
            ("tfClearNoRipple", tfClearNoRipple),
            ("tfSetFreeze", tfSetFreeze),
            ("tfClearFreeze", tfClearFreeze),
            ("tfSetDeepFreeze", tfSetDeepFreeze),
            ("tfClearDeepFreeze", tfClearDeepFreeze),
        ])
    })
}

pub fn get_enable_amendment_flags() -> &'static FlagMap {
    static FLAGS: OnceLock<FlagMap> = OnceLock::new();
    FLAGS.get_or_init(|| {
        make_flag_map(&[
            ("tfGotMajority", tfGotMajority),
            ("tfLostMajority", tfLostMajority),
        ])
    })
}

pub fn get_payment_channel_claim_flags() -> &'static FlagMap {
    static FLAGS: OnceLock<FlagMap> = OnceLock::new();
    FLAGS.get_or_init(|| make_flag_map(&[("tfRenew", tfRenew), ("tfClose", tfClose)]))
}

pub fn get_nft_mint_flags() -> &'static FlagMap {
    static FLAGS: OnceLock<FlagMap> = OnceLock::new();
    FLAGS.get_or_init(|| {
        make_flag_map(&[
            ("tfBurnable", tfBurnable),
            ("tfOnlyXRP", tfOnlyXRP),
            ("tfTrustLine", tfTrustLine),
            ("tfTransferable", tfTransferable),
            ("tfMutable", tfMutable),
        ])
    })
}

pub fn get_mpt_issuance_create_flags() -> &'static FlagMap {
    static FLAGS: OnceLock<FlagMap> = OnceLock::new();
    FLAGS.get_or_init(|| {
        make_flag_map(&[
            ("tfMPTCanLock", tfMPTCanLock),
            ("tfMPTRequireAuth", tfMPTRequireAuth),
            ("tfMPTCanEscrow", tfMPTCanEscrow),
            ("tfMPTCanTrade", tfMPTCanTrade),
            ("tfMPTCanTransfer", tfMPTCanTransfer),
            ("tfMPTCanClawback", tfMPTCanClawback),
            ("tfMPTCanHoldConfidentialBalance", tfMPTCanHoldConfidentialBalance),
        ])
    })
}

pub fn get_mpt_authorize_flags() -> &'static FlagMap {
    static FLAGS: OnceLock<FlagMap> = OnceLock::new();
    FLAGS.get_or_init(|| make_flag_map(&[("tfMPTUnauthorize", tfMPTUnauthorize)]))
}

pub fn get_mpt_issuance_set_flags() -> &'static FlagMap {
    static FLAGS: OnceLock<FlagMap> = OnceLock::new();
    FLAGS.get_or_init(|| make_flag_map(&[("tfMPTLock", tfMPTLock), ("tfMPTUnlock", tfMPTUnlock)]))
}

pub fn get_nft_create_offer_flags() -> &'static FlagMap {
    static FLAGS: OnceLock<FlagMap> = OnceLock::new();
    FLAGS.get_or_init(|| make_flag_map(&[("tfSellNFToken", tfSellNFToken)]))
}

pub fn get_amm_deposit_flags() -> &'static FlagMap {
    static FLAGS: OnceLock<FlagMap> = OnceLock::new();
    FLAGS.get_or_init(|| {
        make_flag_map(&[
            ("tfLPToken", tfLPToken),
            ("tfSingleAsset", tfSingleAsset),
            ("tfTwoAsset", tfTwoAsset),
            ("tfOneAssetLPToken", tfOneAssetLPToken),
            ("tfLimitLPToken", tfLimitLPToken),
            ("tfTwoAssetIfEmpty", tfTwoAssetIfEmpty),
        ])
    })
}

pub fn get_amm_withdraw_flags() -> &'static FlagMap {
    static FLAGS: OnceLock<FlagMap> = OnceLock::new();
    FLAGS.get_or_init(|| {
        make_flag_map(&[
            ("tfLPToken", tfLPToken),
            ("tfWithdrawAll", tfWithdrawAll),
            ("tfOneAssetWithdrawAll", tfOneAssetWithdrawAll),
            ("tfSingleAsset", tfSingleAsset),
            ("tfTwoAsset", tfTwoAsset),
            ("tfOneAssetLPToken", tfOneAssetLPToken),
            ("tfLimitLPToken", tfLimitLPToken),
        ])
    })
}

pub fn get_amm_clawback_flags() -> &'static FlagMap {
    static FLAGS: OnceLock<FlagMap> = OnceLock::new();
    FLAGS.get_or_init(|| make_flag_map(&[("tfClawTwoAssets", tfClawTwoAssets)]))
}

pub fn get_xchain_modify_bridge_flags() -> &'static FlagMap {
    static FLAGS: OnceLock<FlagMap> = OnceLock::new();
    FLAGS.get_or_init(|| {
        make_flag_map(&[("tfClearAccountCreateAmount", tfClearAccountCreateAmount)])
    })
}

pub fn get_vault_create_flags() -> &'static FlagMap {
    static FLAGS: OnceLock<FlagMap> = OnceLock::new();
    FLAGS.get_or_init(|| {
        make_flag_map(&[
            ("tfVaultPrivate", tfVaultPrivate),
            ("tfVaultShareNonTransferable", tfVaultShareNonTransferable),
        ])
    })
}

pub fn get_batch_flags() -> &'static FlagMap {
    static FLAGS: OnceLock<FlagMap> = OnceLock::new();
    FLAGS.get_or_init(|| {
        make_flag_map(&[
            ("tfAllOrNothing", tfAllOrNothing),
            ("tfOnlyOne", tfOnlyOne),
            ("tfUntilFailure", tfUntilFailure),
            ("tfIndependent", tfIndependent),
        ])
    })
}

pub fn get_loan_set_flags() -> &'static FlagMap {
    static FLAGS: OnceLock<FlagMap> = OnceLock::new();
    FLAGS.get_or_init(|| make_flag_map(&[("tfLoanOverpayment", tfLoanOverpayment)]))
}

pub fn get_loan_pay_flags() -> &'static FlagMap {
    static FLAGS: OnceLock<FlagMap> = OnceLock::new();
    FLAGS.get_or_init(|| {
        make_flag_map(&[
            ("tfLoanOverpayment", tfLoanOverpayment),
            ("tfLoanFullPayment", tfLoanFullPayment),
            ("tfLoanLatePayment", tfLoanLatePayment),
        ])
    })
}

pub fn get_loan_manage_flags() -> &'static FlagMap {
    static FLAGS: OnceLock<FlagMap> = OnceLock::new();
    FLAGS.get_or_init(|| {
        make_flag_map(&[
            ("tfLoanDefault", tfLoanDefault),
            ("tfLoanImpair", tfLoanImpair),
            ("tfLoanUnimpair", tfLoanUnimpair),
        ])
    })
}

pub fn get_asf_flag_map() -> &'static FlagMap {
    static FLAGS: OnceLock<FlagMap> = OnceLock::new();
    FLAGS.get_or_init(|| {
        make_flag_map(&[
            ("asfRequireDest", asfRequireDest),
            ("asfRequireAuth", asfRequireAuth),
            ("asfDisallowXRP", asfDisallowXRP),
            ("asfDisableMaster", asfDisableMaster),
            ("asfAccountTxnID", asfAccountTxnID),
            ("asfNoFreeze", asfNoFreeze),
            ("asfGlobalFreeze", asfGlobalFreeze),
            ("asfDefaultRipple", asfDefaultRipple),
            ("asfDepositAuth", asfDepositAuth),
            ("asfAuthorizedNFTokenMinter", asfAuthorizedNFTokenMinter),
            (
                "asfDisallowIncomingNFTokenOffer",
                asfDisallowIncomingNFTokenOffer,
            ),
            ("asfDisallowIncomingCheck", asfDisallowIncomingCheck),
            ("asfDisallowIncomingPayChan", asfDisallowIncomingPayChan),
            ("asfDisallowIncomingTrustline", asfDisallowIncomingTrustline),
            ("asfAllowTrustLineClawback", asfAllowTrustLineClawback),
            ("asfAllowTrustLineLocking", asfAllowTrustLineLocking),
        ])
    })
}

pub fn get_all_tx_flags() -> &'static FlagMapPairList {
    static FLAGS: OnceLock<FlagMapPairList> = OnceLock::new();
    FLAGS.get_or_init(|| {
        vec![
            ("universal".to_string(), get_universal_flags().clone()),
            ("AccountSet".to_string(), get_account_set_flags().clone()),
            ("OfferCreate".to_string(), get_offer_create_flags().clone()),
            ("Payment".to_string(), get_payment_flags().clone()),
            ("TrustSet".to_string(), get_trust_set_flags().clone()),
            (
                "EnableAmendment".to_string(),
                get_enable_amendment_flags().clone(),
            ),
            (
                "PaymentChannelClaim".to_string(),
                get_payment_channel_claim_flags().clone(),
            ),
            ("NFTokenMint".to_string(), get_nft_mint_flags().clone()),
            (
                "MPTokenIssuanceCreate".to_string(),
                get_mpt_issuance_create_flags().clone(),
            ),
            (
                "MPTokenAuthorize".to_string(),
                get_mpt_authorize_flags().clone(),
            ),
            (
                "MPTokenIssuanceSet".to_string(),
                get_mpt_issuance_set_flags().clone(),
            ),
            (
                "NFTokenCreateOffer".to_string(),
                get_nft_create_offer_flags().clone(),
            ),
            ("AMMDeposit".to_string(), get_amm_deposit_flags().clone()),
            ("AMMWithdraw".to_string(), get_amm_withdraw_flags().clone()),
            ("AMMClawback".to_string(), get_amm_clawback_flags().clone()),
            (
                "XChainModifyBridge".to_string(),
                get_xchain_modify_bridge_flags().clone(),
            ),
            ("VaultCreate".to_string(), get_vault_create_flags().clone()),
            ("Batch".to_string(), get_batch_flags().clone()),
            ("LoanSet".to_string(), get_loan_set_flags().clone()),
            ("LoanPay".to_string(), get_loan_pay_flags().clone()),
            ("LoanManage".to_string(), get_loan_manage_flags().clone()),
        ]
    })
}

pub use self::get_account_set_flags as getAccountSetFlags;
pub use self::get_all_tx_flags as getAllTxFlags;
pub use self::get_amm_clawback_flags as getAMMClawbackFlags;
pub use self::get_amm_deposit_flags as getAMMDepositFlags;
pub use self::get_amm_withdraw_flags as getAMMWithdrawFlags;
pub use self::get_asf_flag_map as getAsfFlagMap;
pub use self::get_batch_flags as getBatchFlags;
pub use self::get_enable_amendment_flags as getEnableAmendmentFlags;
pub use self::get_loan_manage_flags as getLoanManageFlags;
pub use self::get_loan_pay_flags as getLoanPayFlags;
pub use self::get_loan_set_flags as getLoanSetFlags;
pub use self::get_mpt_authorize_flags as getMPTokenAuthorizeFlags;
pub use self::get_mpt_issuance_create_flags as getMPTokenIssuanceCreateFlags;
pub use self::get_mpt_issuance_set_flags as getMPTokenIssuanceSetFlags;
pub use self::get_nft_create_offer_flags as getNFTokenCreateOfferFlags;
pub use self::get_nft_mint_flags as getNFTokenMintFlags;
pub use self::get_offer_create_flags as getOfferCreateFlags;
pub use self::get_payment_channel_claim_flags as getPaymentChannelClaimFlags;
pub use self::get_payment_flags as getPaymentFlags;
pub use self::get_trust_set_flags as getTrustSetFlags;
pub use self::get_universal_flags as getUniversalFlags;
pub use self::get_vault_create_flags as getVaultCreateFlags;
pub use self::get_xchain_modify_bridge_flags as getXChainModifyBridgeFlags;

pub const fn transaction_flags_mask(transaction_flags: FlagValue) -> FlagValue {
    !(UNIVERSAL_TRANSACTION_FLAGS | transaction_flags)
}

pub const fn transaction_flags_mask_with_adjustment(
    transaction_flags: FlagValue,
    mask_adjustment: FlagValue,
) -> FlagValue {
    transaction_flags_mask(transaction_flags) | mask_adjustment
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct BatchTransactionFlags(u32);

impl BatchTransactionFlags {
    pub const NONE: Self = Self(0);
    pub const ALL_OR_NOTHING: Self = Self(0x0001_0000);
    pub const ONLY_ONE: Self = Self(0x0002_0000);
    pub const UNTIL_FAILURE: Self = Self(0x0004_0000);
    pub const INDEPENDENT: Self = Self(0x0008_0000);
    pub const MASK: Self = Self(
        Self::ALL_OR_NOTHING.0 | Self::ONLY_ONE.0 | Self::UNTIL_FAILURE.0 | Self::INDEPENDENT.0,
    );

    pub const fn from_bits(bits: u32) -> Self {
        Self(bits & Self::MASK.0)
    }

    pub const fn bits(self) -> u32 {
        self.0
    }

    pub const fn contains(self, other: Self) -> bool {
        (self.0 & other.0) == other.0
    }
}

impl std::ops::BitOr for BatchTransactionFlags {
    type Output = Self;

    fn bitor(self, rhs: Self) -> Self::Output {
        Self(self.0 | rhs.0)
    }
}

impl std::ops::BitOrAssign for BatchTransactionFlags {
    fn bitor_assign(&mut self, rhs: Self) {
        *self = *self | rhs;
    }
}

impl std::ops::BitAnd for BatchTransactionFlags {
    type Output = Self;

    fn bitand(self, rhs: Self) -> Self::Output {
        Self(self.0 & rhs.0)
    }
}

impl std::ops::BitAndAssign for BatchTransactionFlags {
    fn bitand_assign(&mut self, rhs: Self) {
        *self = *self & rhs;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map_keys(map: &FlagMap) -> Vec<&str> {
        map.keys().map(String::as_str).collect()
    }

    #[test]
    fn tx_flag_catalog_constants_match_cpp_txflags() {
        assert_eq!(FULLY_CANONICAL_SIGNATURE_FLAG, 0x8000_0000);
        assert_eq!(INNER_BATCH_TRANSACTION_FLAG, 0x4000_0000);
        assert_eq!(UNIVERSAL_TRANSACTION_FLAGS, 0xc000_0000);
        assert_eq!(UNIVERSAL_TRANSACTION_FLAGS_MASK, 0x3fff_ffff);
        assert_eq!(tfFullyCanonicalSig, FULLY_CANONICAL_SIGNATURE_FLAG);
        assert_eq!(tfInnerBatchTxn, INNER_BATCH_TRANSACTION_FLAG);
        assert_eq!(tfUniversal, UNIVERSAL_TRANSACTION_FLAGS);
        assert_eq!(tfUniversalMask, UNIVERSAL_TRANSACTION_FLAGS_MASK);

        assert_eq!(ACCOUNT_SET_REQUIRE_DEST_TAG_FLAG, 0x0001_0000);
        assert_eq!(ACCOUNT_SET_OPTIONAL_DEST_TAG_FLAG, 0x0002_0000);
        assert_eq!(ACCOUNT_SET_REQUIRE_AUTH_FLAG, 0x0004_0000);
        assert_eq!(ACCOUNT_SET_OPTIONAL_AUTH_FLAG, 0x0008_0000);
        assert_eq!(ACCOUNT_SET_DISALLOW_XRP_FLAG, 0x0010_0000);
        assert_eq!(ACCOUNT_SET_ALLOW_XRP_FLAG, 0x0020_0000);
        assert_eq!(ACCOUNT_SET_FLAGS, 0x003f_0000);
        assert_eq!(ACCOUNT_SET_FLAGS_MASK, 0x3fc0_ffff);
        assert_eq!(tfRequireDestTag, ACCOUNT_SET_REQUIRE_DEST_TAG_FLAG);
        assert_eq!(tfAllowXRP, ACCOUNT_SET_ALLOW_XRP_FLAG);
        assert_eq!(tfAccountSetMask, ACCOUNT_SET_FLAGS_MASK);

        assert_eq!(PAYMENT_FLAGS, 0x0007_0000);
        assert_eq!(PAYMENT_FLAGS_MASK, 0x3ff8_ffff);
        assert_eq!(tfPaymentMask, PAYMENT_FLAGS_MASK);

        assert_eq!(TRUST_SET_FLAGS, 0x00f7_0000);
        assert_eq!(TRUST_SET_FLAGS_MASK, 0x3f08_ffff);
        assert_eq!(tfTrustSetMask, TRUST_SET_FLAGS_MASK);

        assert_eq!(NF_TOKEN_MINT_FLAGS_WITHOUT_MUTABLE, 0x3fff_fff4);
        assert_eq!(NF_TOKEN_MINT_OLD_FLAGS, 0x3fff_fff0);
        assert_eq!(NF_TOKEN_MINT_OLD_FLAGS_WITH_MUTABLE, 0x3fff_ffe0);
        assert_eq!(tfTrustLine, NF_TOKEN_TRUST_LINE_FLAG);
        assert_eq!(tfNFTokenMintOldMask, NF_TOKEN_MINT_OLD_FLAGS);

        assert_eq!(MPT_ISSUANCE_CREATE_FLAGS, 0x0000_00fe);
        assert_eq!(MPT_ISSUANCE_CREATE_FLAGS_MASK, 0x3fff_ff01);
        assert_eq!(tfMPTokenIssuanceCreateMask, MPT_ISSUANCE_CREATE_FLAGS_MASK);
        assert_eq!(MPT_ISSUANCE_SET_FLAGS, 0x0000_0003);
        assert_eq!(MPT_ISSUANCE_SET_FLAGS_MASK, 0x3fff_fffc);
        assert_eq!(MPT_ISSUANCE_CREATE_MUTABLE_MASK, 0xfffc_ff81);
        assert_eq!(MPT_ISSUANCE_SET_MUTABLE_MASK, 0xffff_f000);

        assert_eq!(AMM_DEPOSIT_FLAGS, 0x00f9_0000);
        assert_eq!(AMM_DEPOSIT_FLAGS_MASK, 0x3f06_ffff);
        assert_eq!(AMM_WITHDRAW_FLAGS, 0x007f_0000);
        assert_eq!(AMM_WITHDRAW_FLAGS_MASK, 0x3f80_ffff);
        assert_eq!(tfAMMDepositMask, AMM_DEPOSIT_FLAGS_MASK);
        assert_eq!(tfAMMWithdrawMask, AMM_WITHDRAW_FLAGS_MASK);

        assert_eq!(BATCH_FLAGS, 0x000f_0000);
        assert_eq!(BATCH_FLAGS_MASK, 0x7ff0_ffff);
        assert_eq!(tfBatchMask, BATCH_FLAGS_MASK);

        assert_eq!(LOAN_SET_FLAGS, 0x0001_0000);
        assert_eq!(LOAN_SET_FLAGS_MASK, 0x3ffe_ffff);
        assert_eq!(LOAN_PAY_FLAGS, 0x0007_0000);
        assert_eq!(LOAN_PAY_FLAGS_MASK, 0x3ff8_ffff);
        assert_eq!(LOAN_MANAGE_FLAGS, 0x0007_0000);
        assert_eq!(LOAN_MANAGE_FLAGS_MASK, 0x3ff8_ffff);
        assert_eq!(tfLoanSetMask, LOAN_SET_FLAGS_MASK);
        assert_eq!(tfLoanPayMask, LOAN_PAY_FLAGS_MASK);
        assert_eq!(tfLoanManageMask, LOAN_MANAGE_FLAGS_MASK);

        assert_eq!(MPT_PAYMENT_MASK, 0x3ffd_ffff);
        assert_eq!(TRUST_SET_PERMISSION_MASK, 0x3fce_ffff);
        assert_eq!(WITHDRAW_SUB_TX_FLAGS, 0x007f_0000);
        assert_eq!(DEPOSIT_SUB_TX_FLAGS, 0x00f9_0000);
        assert_eq!(tfMPTPaymentMask, MPT_PAYMENT_MASK);
        assert_eq!(tfTrustSetPermissionMask, TRUST_SET_PERMISSION_MASK);
        assert_eq!(tfWithdrawSubTx, WITHDRAW_SUB_TX_FLAGS);
        assert_eq!(tfDepositSubTx, DEPOSIT_SUB_TX_FLAGS);

        assert_eq!(
            asfAllowTrustLineClawback,
            ASF_ALLOW_TRUST_LINE_CLAWBACK_FLAG
        );
        assert_eq!(asfAllowTrustLineLocking, ASF_ALLOW_TRUST_LINE_LOCKING_FLAG);
    }

    #[test]
    fn transaction_flag_map_getters_match_cpp_catalog_shapes() {
        let universal = get_universal_flags();
        let universal_alias = getUniversalFlags();
        assert!(std::ptr::eq(universal, universal_alias));
        assert_eq!(
            map_keys(universal),
            vec!["tfFullyCanonicalSig", "tfInnerBatchTxn"]
        );
        assert_eq!(
            universal.get("tfFullyCanonicalSig"),
            Some(&tfFullyCanonicalSig)
        );
        assert_eq!(universal.get("tfInnerBatchTxn"), Some(&tfInnerBatchTxn));

        let account_set = get_account_set_flags();
        assert!(std::ptr::eq(account_set, getAccountSetFlags()));
        assert_eq!(
            map_keys(account_set),
            vec![
                "tfAllowXRP",
                "tfDisallowXRP",
                "tfOptionalAuth",
                "tfOptionalDestTag",
                "tfRequireAuth",
                "tfRequireDestTag",
            ]
        );

        let asf = get_asf_flag_map();
        assert!(std::ptr::eq(asf, getAsfFlagMap()));
        assert_eq!(asf.get("asfRequireDest"), Some(&ASF_REQUIRE_DEST_FLAG));
        assert_eq!(
            asf.get("asfAllowTrustLineClawback"),
            Some(&ASF_ALLOW_TRUST_LINE_CLAWBACK_FLAG)
        );
        assert_eq!(asf.len(), 16);

        let all = get_all_tx_flags();
        assert!(std::ptr::eq(all, getAllTxFlags()));
        let names: Vec<&str> = all.iter().map(|(name, _)| name.as_str()).collect();
        assert_eq!(
            names,
            vec![
                "universal",
                "AccountSet",
                "OfferCreate",
                "Payment",
                "TrustSet",
                "EnableAmendment",
                "PaymentChannelClaim",
                "NFTokenMint",
                "MPTokenIssuanceCreate",
                "MPTokenAuthorize",
                "MPTokenIssuanceSet",
                "NFTokenCreateOffer",
                "AMMDeposit",
                "AMMWithdraw",
                "AMMClawback",
                "XChainModifyBridge",
                "VaultCreate",
                "Batch",
                "LoanSet",
                "LoanPay",
                "LoanManage",
            ]
        );
        assert_eq!(all.len(), 21);
        assert_eq!(
            all[1].1.get("tfRequireDestTag"),
            Some(&ACCOUNT_SET_REQUIRE_DEST_TAG_FLAG)
        );
        assert_eq!(
            all[3].1.get("tfPartialPayment"),
            Some(&PAYMENT_PARTIAL_PAYMENT_FLAG)
        );
        assert_eq!(
            all[17].1.get("tfAllOrNothing"),
            Some(&BATCH_ALL_OR_NOTHING_FLAG)
        );
    }

    #[test]
    fn batch_transaction_flags_mask_out_unrelated_bits_batch_mode_reads() {
        let flags = BatchTransactionFlags::from_bits(0x4001_0000);

        assert_eq!(flags, BatchTransactionFlags::ALL_OR_NOTHING);
        assert!(flags.contains(BatchTransactionFlags::ALL_OR_NOTHING));
        assert!(!flags.contains(BatchTransactionFlags::ONLY_ONE));
    }

    #[test]
    fn batch_transaction_flags_mask_batch_mask_shape() {
        assert_eq!(
            BATCH_FLAGS_MASK & INNER_BATCH_TRANSACTION_FLAG,
            INNER_BATCH_TRANSACTION_FLAG
        );
        assert_eq!(BATCH_FLAGS_MASK & BATCH_ALL_OR_NOTHING_FLAG, 0);
        assert_eq!(BATCH_FLAGS_MASK & BATCH_ONLY_ONE_FLAG, 0);
        assert_eq!(BATCH_FLAGS_MASK & BATCH_UNTIL_FAILURE_FLAG, 0);
        assert_eq!(BATCH_FLAGS_MASK & BATCH_INDEPENDENT_FLAG, 0);
    }
}
