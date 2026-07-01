//! Static ledger-format flag catalogs from `xrpl/protocol/LedgerFormats.h`.
//!
//! This ports the protocol-owned ledger flag values and the static getter maps
//! used by the reference server-definitions catalog.

use std::{collections::BTreeMap, sync::OnceLock};

pub type LedgerFlagValue = u32;
pub type LedgerFlagMap = BTreeMap<String, LedgerFlagValue>;
pub type LedgerFlagMapPairList = Vec<(String, LedgerFlagMap)>;

pub const PASSWORD_SPENT_LEDGER_FLAG: LedgerFlagValue = 0x0001_0000;
pub const REQUIRE_DEST_TAG_LEDGER_FLAG: LedgerFlagValue = 0x0002_0000;
pub const REQUIRE_AUTH_LEDGER_FLAG: LedgerFlagValue = 0x0004_0000;
pub const DISALLOW_XRP_LEDGER_FLAG: LedgerFlagValue = 0x0008_0000;
pub const DISABLE_MASTER_LEDGER_FLAG: LedgerFlagValue = 0x0010_0000;
pub const NO_FREEZE_LEDGER_FLAG: LedgerFlagValue = 0x0020_0000;
pub const GLOBAL_FREEZE_LEDGER_FLAG: LedgerFlagValue = 0x0040_0000;
pub const DEFAULT_RIPPLE_LEDGER_FLAG: LedgerFlagValue = 0x0080_0000;
pub const DEPOSIT_AUTH_LEDGER_FLAG: LedgerFlagValue = 0x0100_0000;
pub const DISALLOW_INCOMING_NF_TOKEN_OFFER_LEDGER_FLAG: LedgerFlagValue = 0x0400_0000;
pub const DISALLOW_INCOMING_CHECK_LEDGER_FLAG: LedgerFlagValue = 0x0800_0000;
pub const DISALLOW_INCOMING_PAY_CHAN_LEDGER_FLAG: LedgerFlagValue = 0x1000_0000;
pub const DISALLOW_INCOMING_TRUSTLINE_LEDGER_FLAG: LedgerFlagValue = 0x2000_0000;
pub const ALLOW_TRUST_LINE_LOCKING_LEDGER_FLAG: LedgerFlagValue = 0x4000_0000;
pub const ALLOW_TRUST_LINE_CLAWBACK_LEDGER_FLAG: LedgerFlagValue = 0x8000_0000;

pub const PASSIVE_OFFER_LEDGER_FLAG: LedgerFlagValue = 0x0001_0000;
pub const SELL_OFFER_LEDGER_FLAG: LedgerFlagValue = 0x0002_0000;
pub const HYBRID_OFFER_LEDGER_FLAG: LedgerFlagValue = 0x0004_0000;

pub const LOW_RESERVE_LEDGER_FLAG: LedgerFlagValue = 0x0001_0000;
pub const HIGH_RESERVE_LEDGER_FLAG: LedgerFlagValue = 0x0002_0000;
pub const LOW_AUTH_LEDGER_FLAG: LedgerFlagValue = 0x0004_0000;
pub const HIGH_AUTH_LEDGER_FLAG: LedgerFlagValue = 0x0008_0000;
pub const LOW_NO_RIPPLE_LEDGER_FLAG: LedgerFlagValue = 0x0010_0000;
pub const HIGH_NO_RIPPLE_LEDGER_FLAG: LedgerFlagValue = 0x0020_0000;
pub const LOW_FREEZE_LEDGER_FLAG: LedgerFlagValue = 0x0040_0000;
pub const HIGH_FREEZE_LEDGER_FLAG: LedgerFlagValue = 0x0080_0000;
pub const AMM_NODE_LEDGER_FLAG: LedgerFlagValue = 0x0100_0000;
pub const LOW_DEEP_FREEZE_LEDGER_FLAG: LedgerFlagValue = 0x0200_0000;
pub const HIGH_DEEP_FREEZE_LEDGER_FLAG: LedgerFlagValue = 0x0400_0000;

pub const ONE_OWNER_COUNT_LEDGER_FLAG: LedgerFlagValue = 0x0001_0000;

pub const NF_TOKEN_BUY_OFFERS_LEDGER_FLAG: LedgerFlagValue = 0x0000_0001;
pub const NF_TOKEN_SELL_OFFERS_LEDGER_FLAG: LedgerFlagValue = 0x0000_0002;

pub const SELL_NF_TOKEN_LEDGER_FLAG: LedgerFlagValue = 0x0000_0001;

pub const MPT_LOCKED_LEDGER_FLAG: LedgerFlagValue = 0x0000_0001;
pub const MPT_CAN_LOCK_LEDGER_FLAG: LedgerFlagValue = 0x0000_0002;
pub const MPT_REQUIRE_AUTH_LEDGER_FLAG: LedgerFlagValue = 0x0000_0004;
pub const MPT_CAN_ESCROW_LEDGER_FLAG: LedgerFlagValue = 0x0000_0008;
pub const MPT_CAN_TRADE_LEDGER_FLAG: LedgerFlagValue = 0x0000_0010;
pub const MPT_CAN_TRANSFER_LEDGER_FLAG: LedgerFlagValue = 0x0000_0020;
pub const MPT_CAN_CLAWBACK_LEDGER_FLAG: LedgerFlagValue = 0x0000_0040;
pub const MPT_CAN_HOLD_CONFIDENTIAL_BALANCE_LEDGER_FLAG: LedgerFlagValue = 0x0000_0080;

pub const MPT_CAN_MUTATE_CAN_LOCK_LEDGER_FLAG: LedgerFlagValue = 0x0000_0002;
pub const MPT_CAN_MUTATE_REQUIRE_AUTH_LEDGER_FLAG: LedgerFlagValue = 0x0000_0004;
pub const MPT_CAN_MUTATE_CAN_ESCROW_LEDGER_FLAG: LedgerFlagValue = 0x0000_0008;
pub const MPT_CAN_MUTATE_CAN_TRADE_LEDGER_FLAG: LedgerFlagValue = 0x0000_0010;
pub const MPT_CAN_MUTATE_CAN_TRANSFER_LEDGER_FLAG: LedgerFlagValue = 0x0000_0020;
pub const MPT_CAN_MUTATE_CAN_CLAWBACK_LEDGER_FLAG: LedgerFlagValue = 0x0000_0040;
pub const MPT_CAN_MUTATE_METADATA_LEDGER_FLAG: LedgerFlagValue = 0x0001_0000;
pub const MPT_CAN_MUTATE_TRANSFER_FEE_LEDGER_FLAG: LedgerFlagValue = 0x0002_0000;

pub const MPT_AUTHORIZED_LEDGER_FLAG: LedgerFlagValue = 0x0000_0002;
pub const MPT_AMM_LEDGER_FLAG: LedgerFlagValue = 0x0000_0004;

pub const ACCEPTED_LEDGER_FLAG: LedgerFlagValue = 0x0001_0000;

pub const VAULT_PRIVATE_LEDGER_FLAG: LedgerFlagValue = 0x0001_0000;

pub const LOAN_DEFAULT_LEDGER_FLAG: LedgerFlagValue = 0x0001_0000;
pub const LOAN_IMPAIRED_LEDGER_FLAG: LedgerFlagValue = 0x0002_0000;
pub const LOAN_OVERPAYMENT_LEDGER_FLAG: LedgerFlagValue = 0x0004_0000;

macro_rules! alias_consts {
    ($(($source:ident => $alias:ident)),* $(,)?) => {
        $(pub use self::$source as $alias;)*
    };
}

alias_consts!(
    (PASSWORD_SPENT_LEDGER_FLAG => lsfPasswordSpent),
    (REQUIRE_DEST_TAG_LEDGER_FLAG => lsfRequireDestTag),
    (REQUIRE_AUTH_LEDGER_FLAG => lsfRequireAuth),
    (DISALLOW_XRP_LEDGER_FLAG => lsfDisallowXRP),
    (DISABLE_MASTER_LEDGER_FLAG => lsfDisableMaster),
    (NO_FREEZE_LEDGER_FLAG => lsfNoFreeze),
    (GLOBAL_FREEZE_LEDGER_FLAG => lsfGlobalFreeze),
    (DEFAULT_RIPPLE_LEDGER_FLAG => lsfDefaultRipple),
    (DEPOSIT_AUTH_LEDGER_FLAG => lsfDepositAuth),
    (DISALLOW_INCOMING_NF_TOKEN_OFFER_LEDGER_FLAG => lsfDisallowIncomingNFTokenOffer),
    (DISALLOW_INCOMING_CHECK_LEDGER_FLAG => lsfDisallowIncomingCheck),
    (DISALLOW_INCOMING_PAY_CHAN_LEDGER_FLAG => lsfDisallowIncomingPayChan),
    (DISALLOW_INCOMING_TRUSTLINE_LEDGER_FLAG => lsfDisallowIncomingTrustline),
    (ALLOW_TRUST_LINE_LOCKING_LEDGER_FLAG => lsfAllowTrustLineLocking),
    (ALLOW_TRUST_LINE_CLAWBACK_LEDGER_FLAG => lsfAllowTrustLineClawback),
    (PASSIVE_OFFER_LEDGER_FLAG => lsfPassive),
    (SELL_OFFER_LEDGER_FLAG => lsfSell),
    (HYBRID_OFFER_LEDGER_FLAG => lsfHybrid),
    (LOW_RESERVE_LEDGER_FLAG => lsfLowReserve),
    (HIGH_RESERVE_LEDGER_FLAG => lsfHighReserve),
    (LOW_AUTH_LEDGER_FLAG => lsfLowAuth),
    (HIGH_AUTH_LEDGER_FLAG => lsfHighAuth),
    (LOW_NO_RIPPLE_LEDGER_FLAG => lsfLowNoRipple),
    (HIGH_NO_RIPPLE_LEDGER_FLAG => lsfHighNoRipple),
    (LOW_FREEZE_LEDGER_FLAG => lsfLowFreeze),
    (HIGH_FREEZE_LEDGER_FLAG => lsfHighFreeze),
    (AMM_NODE_LEDGER_FLAG => lsfAMMNode),
    (LOW_DEEP_FREEZE_LEDGER_FLAG => lsfLowDeepFreeze),
    (HIGH_DEEP_FREEZE_LEDGER_FLAG => lsfHighDeepFreeze),
    (ONE_OWNER_COUNT_LEDGER_FLAG => lsfOneOwnerCount),
    (NF_TOKEN_BUY_OFFERS_LEDGER_FLAG => lsfNFTokenBuyOffers),
    (NF_TOKEN_SELL_OFFERS_LEDGER_FLAG => lsfNFTokenSellOffers),
    (SELL_NF_TOKEN_LEDGER_FLAG => lsfSellNFToken),
    (MPT_LOCKED_LEDGER_FLAG => lsfMPTLocked),
    (MPT_CAN_LOCK_LEDGER_FLAG => lsfMPTCanLock),
    (MPT_REQUIRE_AUTH_LEDGER_FLAG => lsfMPTRequireAuth),
    (MPT_CAN_ESCROW_LEDGER_FLAG => lsfMPTCanEscrow),
    (MPT_CAN_TRADE_LEDGER_FLAG => lsfMPTCanTrade),
    (MPT_CAN_TRANSFER_LEDGER_FLAG => lsfMPTCanTransfer),
    (MPT_CAN_CLAWBACK_LEDGER_FLAG => lsfMPTCanClawback),
    (MPT_CAN_HOLD_CONFIDENTIAL_BALANCE_LEDGER_FLAG => lsfMPTCanHoldConfidentialBalance),
    (MPT_CAN_MUTATE_CAN_LOCK_LEDGER_FLAG => lsmfMPTCanMutateCanLock),
    (MPT_CAN_MUTATE_REQUIRE_AUTH_LEDGER_FLAG => lsmfMPTCanMutateRequireAuth),
    (MPT_CAN_MUTATE_CAN_ESCROW_LEDGER_FLAG => lsmfMPTCanMutateCanEscrow),
    (MPT_CAN_MUTATE_CAN_TRADE_LEDGER_FLAG => lsmfMPTCanMutateCanTrade),
    (MPT_CAN_MUTATE_CAN_TRANSFER_LEDGER_FLAG => lsmfMPTCanMutateCanTransfer),
    (MPT_CAN_MUTATE_CAN_CLAWBACK_LEDGER_FLAG => lsmfMPTCanMutateCanClawback),
    (MPT_CAN_MUTATE_METADATA_LEDGER_FLAG => lsmfMPTCanMutateMetadata),
    (MPT_CAN_MUTATE_TRANSFER_FEE_LEDGER_FLAG => lsmfMPTCanMutateTransferFee),
    (MPT_AUTHORIZED_LEDGER_FLAG => lsfMPTAuthorized),
    (MPT_AMM_LEDGER_FLAG => lsfMPTAMM),
    (ACCEPTED_LEDGER_FLAG => lsfAccepted),
    (VAULT_PRIVATE_LEDGER_FLAG => lsfVaultPrivate),
    (LOAN_DEFAULT_LEDGER_FLAG => lsfLoanDefault),
    (LOAN_IMPAIRED_LEDGER_FLAG => lsfLoanImpaired),
    (LOAN_OVERPAYMENT_LEDGER_FLAG => lsfLoanOverpayment),
);

fn build_map(entries: &[(&str, LedgerFlagValue)]) -> LedgerFlagMap {
    entries
        .iter()
        .map(|(name, value)| ((*name).to_owned(), *value))
        .collect()
}

macro_rules! ledger_flag_getter {
    ($fn_name:ident, $static_name:ident, [$(($name:literal, $value:expr)),* $(,)?]) => {
        #[allow(non_snake_case)]
        pub fn $fn_name() -> &'static LedgerFlagMap {
            static $static_name: OnceLock<LedgerFlagMap> = OnceLock::new();
            $static_name.get_or_init(|| build_map(&[$(($name, $value)),*]))
        }
    };
}

ledger_flag_getter!(
    getAccountRootFlags,
    ACCOUNT_ROOT_FLAGS,
    [
        ("lsfPasswordSpent", lsfPasswordSpent),
        ("lsfRequireDestTag", lsfRequireDestTag),
        ("lsfRequireAuth", lsfRequireAuth),
        ("lsfDisallowXRP", lsfDisallowXRP),
        ("lsfDisableMaster", lsfDisableMaster),
        ("lsfNoFreeze", lsfNoFreeze),
        ("lsfGlobalFreeze", lsfGlobalFreeze),
        ("lsfDefaultRipple", lsfDefaultRipple),
        ("lsfDepositAuth", lsfDepositAuth),
        (
            "lsfDisallowIncomingNFTokenOffer",
            lsfDisallowIncomingNFTokenOffer
        ),
        ("lsfDisallowIncomingCheck", lsfDisallowIncomingCheck),
        ("lsfDisallowIncomingPayChan", lsfDisallowIncomingPayChan),
        ("lsfDisallowIncomingTrustline", lsfDisallowIncomingTrustline),
        ("lsfAllowTrustLineLocking", lsfAllowTrustLineLocking),
        ("lsfAllowTrustLineClawback", lsfAllowTrustLineClawback),
    ]
);
ledger_flag_getter!(
    getOfferFlags,
    OFFER_FLAGS,
    [
        ("lsfPassive", lsfPassive),
        ("lsfSell", lsfSell),
        ("lsfHybrid", lsfHybrid),
    ]
);
ledger_flag_getter!(
    getRippleStateFlags,
    RIPPLE_STATE_FLAGS,
    [
        ("lsfLowReserve", lsfLowReserve),
        ("lsfHighReserve", lsfHighReserve),
        ("lsfLowAuth", lsfLowAuth),
        ("lsfHighAuth", lsfHighAuth),
        ("lsfLowNoRipple", lsfLowNoRipple),
        ("lsfHighNoRipple", lsfHighNoRipple),
        ("lsfLowFreeze", lsfLowFreeze),
        ("lsfHighFreeze", lsfHighFreeze),
        ("lsfAMMNode", lsfAMMNode),
        ("lsfLowDeepFreeze", lsfLowDeepFreeze),
        ("lsfHighDeepFreeze", lsfHighDeepFreeze),
    ]
);
ledger_flag_getter!(
    getSignerListFlags,
    SIGNER_LIST_FLAGS,
    [("lsfOneOwnerCount", lsfOneOwnerCount),]
);
ledger_flag_getter!(
    getDirNodeFlags,
    DIR_NODE_FLAGS,
    [
        ("lsfNFTokenBuyOffers", lsfNFTokenBuyOffers),
        ("lsfNFTokenSellOffers", lsfNFTokenSellOffers),
    ]
);
ledger_flag_getter!(
    getNFTokenOfferFlags,
    NFTOKEN_OFFER_FLAGS,
    [("lsfSellNFToken", lsfSellNFToken),]
);
ledger_flag_getter!(
    getMPTokenIssuanceFlags,
    MPTOKEN_ISSUANCE_FLAGS,
    [
        ("lsfMPTLocked", lsfMPTLocked),
        ("lsfMPTCanLock", lsfMPTCanLock),
        ("lsfMPTRequireAuth", lsfMPTRequireAuth),
        ("lsfMPTCanEscrow", lsfMPTCanEscrow),
        ("lsfMPTCanTrade", lsfMPTCanTrade),
        ("lsfMPTCanTransfer", lsfMPTCanTransfer),
        ("lsfMPTCanClawback", lsfMPTCanClawback),
    ]
);
ledger_flag_getter!(
    getMPTokenIssuanceMutableFlags,
    MPTOKEN_ISSUANCE_MUTABLE_FLAGS,
    [
        ("lsmfMPTCanMutateCanLock", lsmfMPTCanMutateCanLock),
        ("lsmfMPTCanMutateRequireAuth", lsmfMPTCanMutateRequireAuth),
        ("lsmfMPTCanMutateCanEscrow", lsmfMPTCanMutateCanEscrow),
        ("lsmfMPTCanMutateCanTrade", lsmfMPTCanMutateCanTrade),
        ("lsmfMPTCanMutateCanTransfer", lsmfMPTCanMutateCanTransfer),
        ("lsmfMPTCanMutateCanClawback", lsmfMPTCanMutateCanClawback),
        ("lsmfMPTCanMutateMetadata", lsmfMPTCanMutateMetadata),
        ("lsmfMPTCanMutateTransferFee", lsmfMPTCanMutateTransferFee),
    ]
);
ledger_flag_getter!(
    getMPTokenFlags,
    MPTOKEN_FLAGS,
    [
        ("lsfMPTLocked", lsfMPTLocked),
        ("lsfMPTAuthorized", lsfMPTAuthorized),
        ("lsfMPTAMM", lsfMPTAMM),
    ]
);
ledger_flag_getter!(
    getCredentialFlags,
    CREDENTIAL_FLAGS,
    [("lsfAccepted", lsfAccepted),]
);
ledger_flag_getter!(
    getVaultFlags,
    VAULT_FLAGS,
    [("lsfVaultPrivate", lsfVaultPrivate),]
);
ledger_flag_getter!(
    getLoanFlags,
    LOAN_FLAGS,
    [
        ("lsfLoanDefault", lsfLoanDefault),
        ("lsfLoanImpaired", lsfLoanImpaired),
        ("lsfLoanOverpayment", lsfLoanOverpayment),
    ]
);

#[allow(non_snake_case)]
pub fn getAllLedgerFlags() -> &'static LedgerFlagMapPairList {
    static ALL_LEDGER_FLAGS: OnceLock<LedgerFlagMapPairList> = OnceLock::new();
    ALL_LEDGER_FLAGS.get_or_init(|| {
        vec![
            ("AccountRoot".to_owned(), getAccountRootFlags().clone()),
            ("Offer".to_owned(), getOfferFlags().clone()),
            ("RippleState".to_owned(), getRippleStateFlags().clone()),
            ("SignerList".to_owned(), getSignerListFlags().clone()),
            ("DirNode".to_owned(), getDirNodeFlags().clone()),
            ("NFTokenOffer".to_owned(), getNFTokenOfferFlags().clone()),
            (
                "MPTokenIssuance".to_owned(),
                getMPTokenIssuanceFlags().clone(),
            ),
            (
                "MPTokenIssuanceMutable".to_owned(),
                getMPTokenIssuanceMutableFlags().clone(),
            ),
            ("MPToken".to_owned(), getMPTokenFlags().clone()),
            ("Credential".to_owned(), getCredentialFlags().clone()),
            ("Vault".to_owned(), getVaultFlags().clone()),
            ("Loan".to_owned(), getLoanFlags().clone()),
        ]
    })
}
