//! `xrpl/protocol/Feature.h` compatibility surface.
//!
//! The Rust registry mirrors the the reference implementation `features.macro` list so callers
//! can reason about amendment names, support state, and default vote behavior
//! without inventing a second source of truth.

use basics::base_uint::Uint256;
use sha2::{Digest, Sha512};
use std::collections::HashSet;

pub const FEATURE_XRP_FEES_NAME: &str = "XRPFees";
pub const FEATURE_BATCH_NAME: &str = "Batch";
pub const FEATURE_BATCH_V1_1_NAME: &str = "BatchV1_1";
pub const FEATURE_AMM_NAME: &str = "AMM";
pub const FEATURE_XCHAIN_BRIDGE_NAME: &str = "XChainBridge";
pub const FEATURE_CLAWBACK_NAME: &str = "Clawback";
pub const FEATURE_TOKEN_ESCROW_NAME: &str = "TokenEscrow";
pub const FEATURE_CONFIDENTIAL_TRANSFER_NAME: &str = "ConfidentialTransfer";
pub const FIX_BATCH_INNER_SIGS_NAME: &str = "fixBatchInnerSigs";
pub const FIX_INNER_OBJ_TEMPLATE_NAME: &str = "fixInnerObjTemplate";
pub const FIX_INNER_OBJ_TEMPLATE2_NAME: &str = "fixInnerObjTemplate2";
pub const FIX_PREVIOUS_TXN_ID_NAME: &str = "fixPreviousTxnID";
pub const FEATURE_LENDING_PROTOCOL_NAME: &str = "LendingProtocol";
pub const FEATURE_LENDING_PROTOCOL_V1_1_NAME: &str = "LendingProtocolV1_1";
pub const FEATURE_SINGLE_ASSET_VAULT_NAME: &str = "SingleAssetVault";
pub const FEATURE_UNIVERSAL_NUMBER_NAME: &str = "fixUniversalNumber";
pub const FIX_AMMV1_1_NAME: &str = "fixAMMv1_1";
pub const FIX_AMMV1_3_NAME: &str = "fixAMMv1_3";
pub const FIX_CLEANUP_3_2_0_NAME: &str = "fixCleanup3_2_0";
pub const FIX_CLEANUP_3_3_0_NAME: &str = "fixCleanup3_3_0";
pub const FEATURE_SPONSOR_NAME: &str = "Sponsor";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegisteredFeatureVote {
    DefaultYes,
    DefaultNo,
    Obsolete,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RegisteredFeature {
    pub name: &'static str,
    pub supported: bool,
    pub vote: RegisteredFeatureVote,
}

impl RegisteredFeature {
    pub const fn new(name: &'static str, supported: bool, vote: RegisteredFeatureVote) -> Self {
        Self {
            name,
            supported,
            vote,
        }
    }
}

pub const REGISTERED_FEATURES: &[RegisteredFeature] = &[
    RegisteredFeature::new(
        FEATURE_BATCH_V1_1_NAME,
        true,
        RegisteredFeatureVote::DefaultNo,
    ),
    RegisteredFeature::new(
        "ConfidentialTransfer",
        false,
        RegisteredFeatureVote::DefaultNo,
    ),
    // release-3.1: cleanup for expired NFT offers, MPToken locked amount,
    // credential deletion errors, PermissionedDEX hybrid validation.
    // Enabled on testnet; mark supported so node is not amendment-blocked.
    RegisteredFeature::new("fixCleanup3_1_3", false, RegisteredFeatureVote::DefaultYes),
    RegisteredFeature::new(
        FIX_CLEANUP_3_2_0_NAME,
        true,
        RegisteredFeatureVote::DefaultYes,
    ),
    RegisteredFeature::new(
        FIX_CLEANUP_3_3_0_NAME,
        true,
        RegisteredFeatureVote::DefaultNo,
    ),
    RegisteredFeature::new("MPTokensV2", false, RegisteredFeatureVote::DefaultNo),
    RegisteredFeature::new("fixSecurity3_1_3", false, RegisteredFeatureVote::DefaultNo),
    RegisteredFeature::new(
        "fixPermissionedDomainInvariant",
        false,
        RegisteredFeatureVote::DefaultNo,
    ),
    RegisteredFeature::new("fixBatchInnerSigs", false, RegisteredFeatureVote::DefaultNo),
    RegisteredFeature::new("LendingProtocol", false, RegisteredFeatureVote::DefaultNo),
    RegisteredFeature::new(
        "LendingProtocolV1_1",
        false,
        RegisteredFeatureVote::DefaultNo,
    ),
    RegisteredFeature::new(
        "PermissionDelegationV1_1",
        false,
        RegisteredFeatureVote::DefaultNo,
    ),
    RegisteredFeature::new("fixDirectoryLimit", true, RegisteredFeatureVote::DefaultNo),
    RegisteredFeature::new(
        "fixIncludeKeyletFields",
        false,
        RegisteredFeatureVote::DefaultNo,
    ),
    RegisteredFeature::new("DynamicMPT", false, RegisteredFeatureVote::DefaultNo),
    RegisteredFeature::new("fixTokenEscrowV1", false, RegisteredFeatureVote::DefaultNo),
    RegisteredFeature::new(
        "fixPriceOracleOrder",
        false,
        RegisteredFeatureVote::DefaultNo,
    ),
    RegisteredFeature::new(
        "fixMPTDeliveredAmount",
        false,
        RegisteredFeatureVote::DefaultNo,
    ),
    RegisteredFeature::new(
        "fixAMMClawbackRounding",
        false,
        RegisteredFeatureVote::DefaultNo,
    ),
    RegisteredFeature::new("TokenEscrow", true, RegisteredFeatureVote::DefaultNo),
    RegisteredFeature::new(
        "fixEnforceNFTokenTrustlineV2",
        true,
        RegisteredFeatureVote::DefaultNo,
    ),
    RegisteredFeature::new("fixAMMv1_3", true, RegisteredFeatureVote::DefaultNo),
    RegisteredFeature::new("PermissionedDEX", true, RegisteredFeatureVote::DefaultNo),
    RegisteredFeature::new("Batch", true, RegisteredFeatureVote::DefaultNo),
    RegisteredFeature::new("SingleAssetVault", false, RegisteredFeatureVote::DefaultNo),
    RegisteredFeature::new(
        "fixPayChanCancelAfter",
        true,
        RegisteredFeatureVote::DefaultNo,
    ),
    RegisteredFeature::new("fixInvalidTxFlags", true, RegisteredFeatureVote::DefaultNo),
    RegisteredFeature::new(
        "fixFrozenLPTokenTransfer",
        true,
        RegisteredFeatureVote::DefaultNo,
    ),
    RegisteredFeature::new("DeepFreeze", true, RegisteredFeatureVote::DefaultNo),
    RegisteredFeature::new(
        "PermissionedDomains",
        true,
        RegisteredFeatureVote::DefaultNo,
    ),
    RegisteredFeature::new("DynamicNFT", true, RegisteredFeatureVote::DefaultNo),
    RegisteredFeature::new("Credentials", true, RegisteredFeatureVote::DefaultNo),
    RegisteredFeature::new("AMMClawback", true, RegisteredFeatureVote::DefaultNo),
    RegisteredFeature::new("fixAMMv1_2", true, RegisteredFeatureVote::DefaultNo),
    RegisteredFeature::new("MPTokensV1", true, RegisteredFeatureVote::DefaultNo),
    RegisteredFeature::new("InvariantsV1_1", false, RegisteredFeatureVote::DefaultNo),
    RegisteredFeature::new(
        "fixNFTokenPageLinks",
        true,
        RegisteredFeatureVote::DefaultNo,
    ),
    RegisteredFeature::new(
        "fixInnerObjTemplate2",
        true,
        RegisteredFeatureVote::DefaultNo,
    ),
    RegisteredFeature::new(
        "fixEnforceNFTokenTrustline",
        true,
        RegisteredFeatureVote::DefaultNo,
    ),
    RegisteredFeature::new("fixReducedOffersV2", true, RegisteredFeatureVote::DefaultNo),
    RegisteredFeature::new("NFTokenMintOffer", true, RegisteredFeatureVote::DefaultNo),
    RegisteredFeature::new("fixAMMv1_1", true, RegisteredFeatureVote::DefaultNo),
    RegisteredFeature::new("fixPreviousTxnID", true, RegisteredFeatureVote::DefaultNo),
    RegisteredFeature::new(
        "fixXChainRewardRounding",
        true,
        RegisteredFeatureVote::DefaultNo,
    ),
    RegisteredFeature::new("fixEmptyDID", true, RegisteredFeatureVote::DefaultNo),
    RegisteredFeature::new("PriceOracle", true, RegisteredFeatureVote::DefaultNo),
    RegisteredFeature::new(
        "fixAMMOverflowOffer",
        true,
        RegisteredFeatureVote::DefaultYes,
    ),
    RegisteredFeature::new(
        "fixInnerObjTemplate",
        true,
        RegisteredFeatureVote::DefaultNo,
    ),
    RegisteredFeature::new("fixNFTokenReserve", true, RegisteredFeatureVote::DefaultNo),
    RegisteredFeature::new("fixFillOrKill", true, RegisteredFeatureVote::DefaultNo),
    RegisteredFeature::new("DID", true, RegisteredFeatureVote::DefaultNo),
    RegisteredFeature::new(
        "fixDisallowIncomingV1",
        true,
        RegisteredFeatureVote::DefaultNo,
    ),
    RegisteredFeature::new("XChainBridge", true, RegisteredFeatureVote::DefaultNo),
    RegisteredFeature::new("AMM", true, RegisteredFeatureVote::DefaultNo),
    RegisteredFeature::new("Clawback", true, RegisteredFeatureVote::DefaultNo),
    RegisteredFeature::new("fixUniversalNumber", true, RegisteredFeatureVote::DefaultNo),
    RegisteredFeature::new("XRPFees", true, RegisteredFeatureVote::DefaultNo),
    RegisteredFeature::new(
        "fixRemoveNFTokenAutoTrustLine",
        true,
        RegisteredFeatureVote::DefaultYes,
    ),
    RegisteredFeature::new("Sponsor", false, RegisteredFeatureVote::DefaultNo),
    RegisteredFeature::new("fix1201", true, RegisteredFeatureVote::Obsolete),
    RegisteredFeature::new("fix1368", true, RegisteredFeatureVote::Obsolete),
    RegisteredFeature::new("fix1373", true, RegisteredFeatureVote::Obsolete),
    RegisteredFeature::new("fix1512", true, RegisteredFeatureVote::Obsolete),
    RegisteredFeature::new("fix1513", true, RegisteredFeatureVote::Obsolete),
    RegisteredFeature::new("fix1515", true, RegisteredFeatureVote::Obsolete),
    RegisteredFeature::new("fix1523", true, RegisteredFeatureVote::Obsolete),
    RegisteredFeature::new("fix1528", true, RegisteredFeatureVote::Obsolete),
    RegisteredFeature::new("fix1543", true, RegisteredFeatureVote::Obsolete),
    RegisteredFeature::new("fix1571", true, RegisteredFeatureVote::Obsolete),
    RegisteredFeature::new("fix1578", true, RegisteredFeatureVote::Obsolete),
    RegisteredFeature::new("fix1623", true, RegisteredFeatureVote::Obsolete),
    RegisteredFeature::new("fix1781", true, RegisteredFeatureVote::Obsolete),
    RegisteredFeature::new(
        "fixAmendmentMajorityCalc",
        true,
        RegisteredFeatureVote::Obsolete,
    ),
    RegisteredFeature::new("fixCheckThreading", true, RegisteredFeatureVote::Obsolete),
    RegisteredFeature::new(
        "fixMasterKeyAsRegularKey",
        true,
        RegisteredFeatureVote::Obsolete,
    ),
    RegisteredFeature::new(
        "fixNonFungibleTokensV1_2",
        true,
        RegisteredFeatureVote::Obsolete,
    ),
    RegisteredFeature::new("fixNFTokenRemint", true, RegisteredFeatureVote::Obsolete),
    RegisteredFeature::new(
        "fixPayChanRecipientOwnerDir",
        true,
        RegisteredFeatureVote::Obsolete,
    ),
    RegisteredFeature::new(
        "fixQualityUpperBound",
        true,
        RegisteredFeatureVote::Obsolete,
    ),
    RegisteredFeature::new("fixReducedOffersV1", true, RegisteredFeatureVote::Obsolete),
    RegisteredFeature::new(
        "fixRmSmallIncreasedQOffers",
        true,
        RegisteredFeatureVote::Obsolete,
    ),
    RegisteredFeature::new(
        "fixSTAmountCanonicalize",
        true,
        RegisteredFeatureVote::Obsolete,
    ),
    RegisteredFeature::new(
        "fixTakerDryOfferRemoval",
        true,
        RegisteredFeatureVote::Obsolete,
    ),
    RegisteredFeature::new("fixTrustLinesToSelf", true, RegisteredFeatureVote::Obsolete),
    RegisteredFeature::new("Checks", true, RegisteredFeatureVote::Obsolete),
    RegisteredFeature::new(
        "CheckCashMakesTrustLine",
        true,
        RegisteredFeatureVote::Obsolete,
    ),
    RegisteredFeature::new("CryptoConditions", true, RegisteredFeatureVote::Obsolete),
    RegisteredFeature::new(
        "CryptoConditionsSuite",
        true,
        RegisteredFeatureVote::Obsolete,
    ),
    RegisteredFeature::new("DeletableAccounts", true, RegisteredFeatureVote::Obsolete),
    RegisteredFeature::new("DepositAuth", true, RegisteredFeatureVote::Obsolete),
    RegisteredFeature::new("DepositPreauth", true, RegisteredFeatureVote::Obsolete),
    RegisteredFeature::new("DisallowIncoming", true, RegisteredFeatureVote::Obsolete),
    RegisteredFeature::new("Escrow", true, RegisteredFeatureVote::Obsolete),
    RegisteredFeature::new("EnforceInvariants", true, RegisteredFeatureVote::Obsolete),
    RegisteredFeature::new("ExpandedSignerList", true, RegisteredFeatureVote::Obsolete),
    RegisteredFeature::new("FeeEscalation", true, RegisteredFeatureVote::Obsolete),
    RegisteredFeature::new("Flow", true, RegisteredFeatureVote::Obsolete),
    RegisteredFeature::new("FlowCross", true, RegisteredFeatureVote::Obsolete),
    RegisteredFeature::new("FlowSortStrands", true, RegisteredFeatureVote::Obsolete),
    RegisteredFeature::new("HardenedValidations", true, RegisteredFeatureVote::Obsolete),
    RegisteredFeature::new(
        "ImmediateOfferKilled",
        true,
        RegisteredFeatureVote::Obsolete,
    ),
    RegisteredFeature::new("MultiSign", true, RegisteredFeatureVote::Obsolete),
    RegisteredFeature::new("MultiSignReserve", true, RegisteredFeatureVote::Obsolete),
    RegisteredFeature::new("NegativeUNL", true, RegisteredFeatureVote::Obsolete),
    RegisteredFeature::new("NonFungibleTokensV1", true, RegisteredFeatureVote::Obsolete),
    RegisteredFeature::new("fixNFTokenDirV1", true, RegisteredFeatureVote::Obsolete),
    RegisteredFeature::new("fixNFTokenNegOffer", true, RegisteredFeatureVote::Obsolete),
    RegisteredFeature::new(
        "NonFungibleTokensV1_1",
        true,
        RegisteredFeatureVote::Obsolete,
    ),
    RegisteredFeature::new("PayChan", true, RegisteredFeatureVote::Obsolete),
    RegisteredFeature::new(
        "RequireFullyCanonicalSig",
        true,
        RegisteredFeatureVote::Obsolete,
    ),
    RegisteredFeature::new("SortedDirectories", true, RegisteredFeatureVote::Obsolete),
    RegisteredFeature::new("TicketBatch", true, RegisteredFeatureVote::Obsolete),
    RegisteredFeature::new("TickSize", true, RegisteredFeatureVote::Obsolete),
    RegisteredFeature::new("TrustSetAuth", true, RegisteredFeatureVote::Obsolete),
];

pub fn feature_id(name: &str) -> Uint256 {
    let mut hasher = Sha512::new();
    hasher.update(name.as_bytes());
    let digest = hasher.finalize();
    Uint256::from_slice(&digest[..32]).expect("SHA-512 half output must contain 32 bytes")
}

pub fn feature_xrp_fees() -> Uint256 {
    feature_id(FEATURE_XRP_FEES_NAME)
}

pub fn feature_batch() -> Uint256 {
    feature_id(FEATURE_BATCH_NAME)
}

pub fn feature_batch_v1_1() -> Uint256 {
    feature_id(FEATURE_BATCH_V1_1_NAME)
}

pub fn feature_amm() -> Uint256 {
    feature_id(FEATURE_AMM_NAME)
}

pub fn feature_xchain_bridge() -> Uint256 {
    feature_id(FEATURE_XCHAIN_BRIDGE_NAME)
}

pub fn feature_clawback() -> Uint256 {
    feature_id(FEATURE_CLAWBACK_NAME)
}

pub fn feature_token_escrow() -> Uint256 {
    feature_id(FEATURE_TOKEN_ESCROW_NAME)
}

pub fn feature_confidential_transfer() -> Uint256 {
    feature_id(FEATURE_CONFIDENTIAL_TRANSFER_NAME)
}

pub fn fix_batch_inner_sigs() -> Uint256 {
    feature_id(FIX_BATCH_INNER_SIGS_NAME)
}

pub fn fix_inner_obj_template() -> Uint256 {
    feature_id(FIX_INNER_OBJ_TEMPLATE_NAME)
}

pub fn fix_inner_obj_template2() -> Uint256 {
    feature_id(FIX_INNER_OBJ_TEMPLATE2_NAME)
}

pub fn fix_previous_txn_id() -> Uint256 {
    feature_id(FIX_PREVIOUS_TXN_ID_NAME)
}

pub fn feature_lending_protocol() -> Uint256 {
    feature_id(FEATURE_LENDING_PROTOCOL_NAME)
}

pub fn feature_lending_protocol_v1_1() -> Uint256 {
    feature_id(FEATURE_LENDING_PROTOCOL_V1_1_NAME)
}

pub fn feature_single_asset_vault() -> Uint256 {
    feature_id(FEATURE_SINGLE_ASSET_VAULT_NAME)
}

pub fn feature_universal_number() -> Uint256 {
    feature_id(FEATURE_UNIVERSAL_NUMBER_NAME)
}

pub fn feature_mp_tokens_v1() -> Uint256 {
    feature_id("MPTokensV1")
}

pub fn feature_permissioned_domains() -> Uint256 {
    feature_id("PermissionedDomains")
}

pub fn feature_deep_freeze() -> Uint256 {
    feature_id("DeepFreeze")
}

pub fn feature_nftoken_mint_offer() -> Uint256 {
    feature_id("NFTokenMintOffer")
}

pub fn fix_nftoken_page_links() -> Uint256 {
    feature_id("fixNFTokenPageLinks")
}

pub fn fix_enforce_nftoken_trustline_v2() -> Uint256 {
    feature_id("fixEnforceNFTokenTrustlineV2")
}

pub fn fix_cleanup_3_1_3() -> Uint256 {
    feature_id("fixCleanup3_1_3")
}

pub fn fix_cleanup_3_2_0() -> Uint256 {
    feature_id(FIX_CLEANUP_3_2_0_NAME)
}

pub fn fix_cleanup_3_3_0() -> Uint256 {
    feature_id(FIX_CLEANUP_3_3_0_NAME)
}

pub fn fix_token_escrow_v1() -> Uint256 {
    feature_id("fixTokenEscrowV1")
}

pub fn fix_ammv1_1() -> Uint256 {
    feature_id(FIX_AMMV1_1_NAME)
}

pub fn fix_ammv1_3() -> Uint256 {
    feature_id(FIX_AMMV1_3_NAME)
}

pub fn feature_sponsor() -> Uint256 {
    feature_id(FEATURE_SPONSOR_NAME)
}

pub fn registered_feature(feature: &Uint256) -> Option<&'static RegisteredFeature> {
    REGISTERED_FEATURES
        .iter()
        .find(|registered| feature_id(registered.name) == *feature)
}

pub fn feature_name(feature: &Uint256) -> Option<&'static str> {
    registered_feature(feature).map(|registered| registered.name)
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FeatureSet {
    features: HashSet<Uint256>,
}

impl FeatureSet {
    pub fn new<I>(features: I) -> Self
    where
        I: IntoIterator<Item = Uint256>,
    {
        Self {
            features: features.into_iter().collect(),
        }
    }

    pub fn contains(&self, feature: &Uint256) -> bool {
        self.features.contains(feature)
    }

    pub fn iter(&self) -> impl Iterator<Item = Uint256> + '_ {
        self.features.iter().copied()
    }

    pub fn len(&self) -> usize {
        self.features.len()
    }

    pub fn is_empty(&self) -> bool {
        self.features.is_empty()
    }
}

impl FromIterator<Uint256> for FeatureSet {
    fn from_iter<T: IntoIterator<Item = Uint256>>(iter: T) -> Self {
        Self::new(iter)
    }
}

impl IntoIterator for FeatureSet {
    type Item = Uint256;
    type IntoIter = std::collections::hash_set::IntoIter<Uint256>;

    fn into_iter(self) -> Self::IntoIter {
        self.features.into_iter()
    }
}

#[cfg(test)]
mod tests {
    use super::{
        FEATURE_AMM_NAME, FEATURE_BATCH_NAME, FEATURE_CLAWBACK_NAME, FEATURE_LENDING_PROTOCOL_NAME,
        FEATURE_SINGLE_ASSET_VAULT_NAME, FEATURE_TOKEN_ESCROW_NAME, FEATURE_UNIVERSAL_NUMBER_NAME,
        FEATURE_XCHAIN_BRIDGE_NAME, FEATURE_XRP_FEES_NAME, FIX_AMMV1_1_NAME, FIX_AMMV1_3_NAME,
        FIX_BATCH_INNER_SIGS_NAME, FIX_INNER_OBJ_TEMPLATE_NAME, FIX_INNER_OBJ_TEMPLATE2_NAME,
        FIX_PREVIOUS_TXN_ID_NAME, FeatureSet, REGISTERED_FEATURES, feature_amm, feature_batch,
        feature_clawback, feature_id, feature_lending_protocol, feature_name,
        feature_single_asset_vault, feature_token_escrow, feature_universal_number,
        feature_xchain_bridge, feature_xrp_fees, fix_ammv1_1, fix_ammv1_3, fix_batch_inner_sigs,
        fix_inner_obj_template, fix_inner_obj_template2, fix_previous_txn_id,
    };
    use basics::base_uint::Uint256;

    #[test]
    fn feature_xrp_fees_matches_current_cpp_identifier() {
        let expected =
            Uint256::from_hex("93E516234E35E08CA689FA33A6D38E103881F8DCB53023F728C307AA89D515A7")
                .expect("expected feature hex should parse");

        assert_eq!(feature_xrp_fees(), expected);
        assert_eq!(feature_name(&expected), Some(FEATURE_XRP_FEES_NAME));
    }

    #[test]
    fn feature_id_sha512_half_registration_rule() {
        assert_eq!(feature_id(FEATURE_BATCH_NAME), feature_batch());
        assert_eq!(feature_id(FEATURE_CLAWBACK_NAME), feature_clawback());
        assert_eq!(
            feature_id(FIX_BATCH_INNER_SIGS_NAME),
            fix_batch_inner_sigs()
        );
        assert_eq!(
            feature_id(FEATURE_TOKEN_ESCROW_NAME),
            feature_token_escrow()
        );
        assert_eq!(feature_id(FEATURE_XRP_FEES_NAME), feature_xrp_fees());
        assert_eq!(
            feature_id(FEATURE_LENDING_PROTOCOL_NAME),
            feature_lending_protocol()
        );
        assert_eq!(feature_id(FIX_PREVIOUS_TXN_ID_NAME), fix_previous_txn_id());
        assert_eq!(
            feature_id(FEATURE_SINGLE_ASSET_VAULT_NAME),
            feature_single_asset_vault()
        );
        assert_eq!(
            feature_id(FEATURE_UNIVERSAL_NUMBER_NAME),
            feature_universal_number()
        );
        assert_eq!(feature_id(FIX_AMMV1_1_NAME), fix_ammv1_1());
        assert_eq!(feature_id(FIX_AMMV1_3_NAME), fix_ammv1_3());
    }

    #[test]
    fn feature_name_returns_none_for_unknown_ids() {
        assert_eq!(feature_name(&Uint256::from_array([0xFF; 32])), None);
    }

    #[test]
    fn registered_feature_table_stays_in_sync_with_lookup_helpers() {
        for registered in REGISTERED_FEATURES {
            let expected = feature_id(registered.name);
            assert_eq!(feature_name(&expected), Some(registered.name));
        }
    }

    #[test]
    fn batch_features_match_current_cpp_identifiers() {
        let batch =
            Uint256::from_hex("894646DD5284E97DECFE6674A6D6152686791C4A95F8C132CCA9BAF9E5812FB6")
                .expect("expected batch hex should parse");
        let amm =
            Uint256::from_hex("8CC0774A3BF66D1D22E76BBDA8E8A232E6B6313834301B3B23E8601196AE6455")
                .expect("expected amm hex should parse");
        let xchain_bridge =
            Uint256::from_hex("C98D98EE9616ACD36E81FDEB8D41D349BF5F1B41DD64A0ABC1FE9AA5EA267E9C")
                .expect("expected xchain bridge hex should parse");
        let clawback =
            Uint256::from_hex("56B241D7A43D40354D02A9DC4C8DF5C7A1F930D92A9035C4E12291B3CA3E1C2B")
                .expect("expected clawback hex should parse");
        let token_escrow =
            Uint256::from_hex("138B968F25822EFBF54C00F97031221C47B1EAB8321D93C7C2AEAF85F04EC5DF")
                .expect("expected token escrow hex should parse");
        let fix_inner_sigs =
            Uint256::from_hex("267624F8F744C4A4F1B5821A7D54410BCEBABE987F0172EE89E5FC4B6EDBC18A")
                .expect("expected inner-batch signature fix hex should parse");
        let fix_inner_obj =
            Uint256::from_hex("C393B3AEEBF575E475F0C60D5E4241B2070CC4D0EB6C4846B1A07508FAEFC485")
                .expect("expected inner object template hex should parse");
        let fix_inner_obj_2 =
            Uint256::from_hex("9196110C23EA879B4229E51C286180C7D02166DA712559F634372F5264D0EC59")
                .expect("expected inner object template2 hex should parse");
        let previous_txn_fix =
            Uint256::from_hex("7BB62DC13EC72B775091E9C71BF8CF97E122647693B50C5E87A80DFD6FCFAC50")
                .expect("expected previous txn fix hex should parse");

        assert_eq!(feature_batch(), batch);
        assert_eq!(feature_name(&batch), Some(FEATURE_BATCH_NAME));
        assert_eq!(feature_amm(), amm);
        assert_eq!(feature_name(&amm), Some(FEATURE_AMM_NAME));
        assert_eq!(feature_xchain_bridge(), xchain_bridge);
        assert_eq!(
            feature_name(&xchain_bridge),
            Some(FEATURE_XCHAIN_BRIDGE_NAME)
        );
        assert_eq!(feature_clawback(), clawback);
        assert_eq!(feature_name(&clawback), Some(FEATURE_CLAWBACK_NAME));
        assert_eq!(feature_token_escrow(), token_escrow);
        assert_eq!(feature_name(&token_escrow), Some(FEATURE_TOKEN_ESCROW_NAME));
        assert_eq!(fix_batch_inner_sigs(), fix_inner_sigs);
        assert_eq!(
            feature_name(&fix_inner_sigs),
            Some(FIX_BATCH_INNER_SIGS_NAME)
        );
        assert_eq!(fix_inner_obj_template(), fix_inner_obj);
        assert_eq!(
            feature_name(&fix_inner_obj),
            Some(FIX_INNER_OBJ_TEMPLATE_NAME)
        );
        assert_eq!(fix_inner_obj_template2(), fix_inner_obj_2);
        assert_eq!(
            feature_name(&fix_inner_obj_2),
            Some(FIX_INNER_OBJ_TEMPLATE2_NAME)
        );
        assert_eq!(fix_previous_txn_id(), previous_txn_fix);
        assert_eq!(
            feature_name(&previous_txn_fix),
            Some(FIX_PREVIOUS_TXN_ID_NAME)
        );
    }

    #[test]
    fn lending_and_single_asset_vault_features_match_current_cpp_identifiers() {
        let lending =
            Uint256::from_hex("565B90CA1AB2B9D42208ED10884188C64F9E19083DECB9634AAF06EB03299509")
                .expect("expected lending protocol hex should parse");
        let single_asset_vault =
            Uint256::from_hex("81BD2619B6B3C8625AC5D0BC01DE17F06C3F0AB95C7C87C93715B87A4FD240D8")
                .expect("expected single asset vault hex should parse");
        let universal_number =
            Uint256::from_hex("2E2FB9CF8A44EB80F4694D38AADAE9B8B7ADAFD2F092E10068E61C98C4F092B0")
                .expect("expected universal number hex should parse");

        assert_eq!(feature_lending_protocol(), lending);
        assert_eq!(feature_name(&lending), Some(FEATURE_LENDING_PROTOCOL_NAME));
        assert_eq!(feature_single_asset_vault(), single_asset_vault);
        assert_eq!(
            feature_name(&single_asset_vault),
            Some(FEATURE_SINGLE_ASSET_VAULT_NAME)
        );
        assert_eq!(feature_universal_number(), universal_number);
        assert_eq!(
            feature_name(&universal_number),
            Some(FEATURE_UNIVERSAL_NUMBER_NAME)
        );
    }

    #[test]
    fn feature_set_behaves_like_a_deduped_feature_collection() {
        let unknown = Uint256::from_array([0x11; 32]);
        let features = FeatureSet::new([feature_xrp_fees(), feature_xrp_fees(), unknown]);

        assert_eq!(features.len(), 2);
        assert!(features.contains(&feature_xrp_fees()));
        assert!(features.contains(&unknown));
        assert!(!features.is_empty());
    }
}
