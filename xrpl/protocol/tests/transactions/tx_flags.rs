use protocol::{
    ACCOUNT_SET_FLAGS, ACCOUNT_SET_FLAGS_MASK, AMM_DEPOSIT_FLAGS, AMM_DEPOSIT_FLAGS_MASK,
    BATCH_FLAGS, BATCH_FLAGS_MASK, BatchTransactionFlags, FULLY_CANONICAL_SIGNATURE_FLAG,
    INNER_BATCH_TRANSACTION_FLAG, LOAN_PAY_FLAGS, LOAN_PAY_FLAGS_MASK, MPT_ISSUANCE_CREATE_FLAGS,
    MPT_ISSUANCE_CREATE_FLAGS_MASK, NF_TOKEN_BURNABLE_FLAG, NF_TOKEN_MINT_FLAGS_WITHOUT_MUTABLE,
    NF_TOKEN_MINT_OLD_FLAGS, NF_TOKEN_MINT_OLD_FLAGS_WITH_MUTABLE,
    PAYMENT_CHANNEL_CLAIM_CLOSE_FLAG, PAYMENT_CHANNEL_CLAIM_FLAGS_MASK,
    PAYMENT_CHANNEL_CLAIM_RENEW_FLAG, PAYMENT_FLAGS, PAYMENT_FLAGS_MASK,
    PAYMENT_PARTIAL_PAYMENT_FLAG, VAULT_CREATE_FLAGS, VAULT_CREATE_FLAGS_MASK,
    asfAllowTrustLineClawback, getAMMClawbackFlags, getAMMDepositFlags, getAMMWithdrawFlags,
    getAccountSetFlags, getAllTxFlags, getAsfFlagMap, getBatchFlags, getEnableAmendmentFlags,
    getLoanManageFlags, getLoanPayFlags, getLoanSetFlags, getMPTokenAuthorizeFlags,
    getMPTokenIssuanceCreateFlags, getMPTokenIssuanceSetFlags, getNFTokenCreateOfferFlags,
    getNFTokenMintFlags, getOfferCreateFlags, getPaymentChannelClaimFlags, getPaymentFlags,
    getTrustSetFlags, getUniversalFlags, getVaultCreateFlags, getXChainModifyBridgeFlags,
    tfAMMDepositMask, tfAccountSetMask, tfBatchMask, tfInnerBatchTxn, tfLoanPayMask,
    tfMPTPaymentMask, tfPaymentMask, tfTrustLine, tfTrustSetPermissionMask, tfUniversal,
    tfUniversalMask, transaction_flags_mask,
};

fn map_keys(map: &protocol::FlagMap) -> Vec<&str> {
    map.keys().map(String::as_str).collect()
}

#[test]
fn tx_flag_catalog_is_reexported_from_protocol_root() {
    assert_eq!(FULLY_CANONICAL_SIGNATURE_FLAG, 0x8000_0000);
    assert_eq!(INNER_BATCH_TRANSACTION_FLAG, 0x4000_0000);
    assert_eq!(
        tfUniversal,
        FULLY_CANONICAL_SIGNATURE_FLAG | INNER_BATCH_TRANSACTION_FLAG
    );
    assert_eq!(tfUniversalMask, !tfUniversal);

    assert_eq!(ACCOUNT_SET_FLAGS, 0x003f_0000);
    assert_eq!(
        ACCOUNT_SET_FLAGS_MASK,
        transaction_flags_mask(ACCOUNT_SET_FLAGS)
    );
    assert_eq!(tfAccountSetMask, ACCOUNT_SET_FLAGS_MASK);

    assert_eq!(PAYMENT_PARTIAL_PAYMENT_FLAG, 0x0002_0000);
    assert_eq!(PAYMENT_FLAGS, 0x0007_0000);
    assert_eq!(PAYMENT_FLAGS_MASK, transaction_flags_mask(PAYMENT_FLAGS));
    assert_eq!(tfPaymentMask, PAYMENT_FLAGS_MASK);

    assert_eq!(PAYMENT_CHANNEL_CLAIM_RENEW_FLAG, 0x0001_0000);
    assert_eq!(PAYMENT_CHANNEL_CLAIM_CLOSE_FLAG, 0x0002_0000);
    assert_eq!(PAYMENT_CHANNEL_CLAIM_FLAGS_MASK, 0x3ffc_ffff);
    assert_eq!(tfTrustLine, 0x0000_0004);

    assert_eq!(AMM_DEPOSIT_FLAGS, 0x00f9_0000);
    assert_eq!(
        AMM_DEPOSIT_FLAGS_MASK,
        transaction_flags_mask(AMM_DEPOSIT_FLAGS)
    );
    assert_eq!(tfAMMDepositMask, AMM_DEPOSIT_FLAGS_MASK);

    assert_eq!(VAULT_CREATE_FLAGS, 0x0003_0000);
    assert_eq!(
        VAULT_CREATE_FLAGS_MASK,
        transaction_flags_mask(VAULT_CREATE_FLAGS)
    );

    assert_eq!(LOAN_PAY_FLAGS, 0x0007_0000);
    assert_eq!(LOAN_PAY_FLAGS_MASK, transaction_flags_mask(LOAN_PAY_FLAGS));
    assert_eq!(tfLoanPayMask, LOAN_PAY_FLAGS_MASK);

    assert_eq!(BATCH_FLAGS, 0x000f_0000);
    assert_eq!(BATCH_FLAGS_MASK, 0x7ff0_ffff);
    assert_eq!(BatchTransactionFlags::MASK.bits(), 0x000f_0000);
    assert_eq!(tfBatchMask, BATCH_FLAGS_MASK);
}

#[test]
fn tx_flag_catalog_map_getters_match_cpp_shape() {
    let universal = getUniversalFlags();
    assert_eq!(
        map_keys(universal),
        vec!["tfFullyCanonicalSig", "tfInnerBatchTxn"]
    );

    let account_set = getAccountSetFlags();
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

    let asf = getAsfFlagMap();
    assert_eq!(asf.len(), 16);
    assert_eq!(
        asf.get("asfAllowTrustLineClawback"),
        Some(&asfAllowTrustLineClawback)
    );

    let all = getAllTxFlags();
    assert_eq!(all.len(), 21);
    assert_eq!(all.first().expect("universal entry").0, "universal");
    assert_eq!(all.last().expect("loan manage entry").0, "LoanManage");
    assert_eq!(
        all[1].1.get("tfRequireDestTag"),
        Some(&(ACCOUNT_SET_FLAGS & 0x0001_0000))
    );
    assert_eq!(
        all[3].1.get("tfPartialPayment"),
        Some(&PAYMENT_PARTIAL_PAYMENT_FLAG)
    );
    assert_eq!(
        all[17].1.get("tfAllOrNothing"),
        Some(&BatchTransactionFlags::ALL_OR_NOTHING.bits())
    );
}

#[test]
fn tx_flag_catalog_getters_cover_remaining_cpp_maps() {
    assert_eq!(
        map_keys(getOfferCreateFlags()),
        vec![
            "tfFillOrKill",
            "tfHybrid",
            "tfImmediateOrCancel",
            "tfPassive",
            "tfSell",
        ]
    );
    assert_eq!(
        map_keys(getPaymentFlags()),
        vec!["tfLimitQuality", "tfNoRippleDirect", "tfPartialPayment"]
    );
    assert_eq!(
        map_keys(getTrustSetFlags()),
        vec![
            "tfClearDeepFreeze",
            "tfClearFreeze",
            "tfClearNoRipple",
            "tfSetDeepFreeze",
            "tfSetFreeze",
            "tfSetNoRipple",
            "tfSetfAuth",
        ]
    );
    assert_eq!(
        map_keys(getEnableAmendmentFlags()),
        vec!["tfGotMajority", "tfLostMajority"]
    );
    assert_eq!(
        map_keys(getPaymentChannelClaimFlags()),
        vec!["tfClose", "tfRenew"]
    );
    assert_eq!(
        map_keys(getNFTokenMintFlags()),
        vec![
            "tfBurnable",
            "tfMutable",
            "tfOnlyXRP",
            "tfTransferable",
            "tfTrustLine",
        ]
    );
    assert_eq!(
        map_keys(getMPTokenIssuanceCreateFlags()),
        vec![
            "tfMPTCanClawback",
            "tfMPTCanEscrow",
            "tfMPTCanLock",
            "tfMPTCanTrade",
            "tfMPTCanTransfer",
            "tfMPTRequireAuth",
        ]
    );
    assert_eq!(
        map_keys(getMPTokenAuthorizeFlags()),
        vec!["tfMPTUnauthorize"]
    );
    assert_eq!(
        map_keys(getMPTokenIssuanceSetFlags()),
        vec!["tfMPTLock", "tfMPTUnlock"]
    );
    assert_eq!(
        map_keys(getNFTokenCreateOfferFlags()),
        vec!["tfSellNFToken"]
    );
    assert_eq!(
        map_keys(getAMMDepositFlags()),
        vec![
            "tfLPToken",
            "tfLimitLPToken",
            "tfOneAssetLPToken",
            "tfSingleAsset",
            "tfTwoAsset",
            "tfTwoAssetIfEmpty",
        ]
    );
    assert_eq!(
        map_keys(getAMMWithdrawFlags()),
        vec![
            "tfLPToken",
            "tfLimitLPToken",
            "tfOneAssetLPToken",
            "tfOneAssetWithdrawAll",
            "tfSingleAsset",
            "tfTwoAsset",
            "tfWithdrawAll",
        ]
    );
    assert_eq!(map_keys(getAMMClawbackFlags()), vec!["tfClawTwoAssets"]);
    assert_eq!(
        map_keys(getXChainModifyBridgeFlags()),
        vec!["tfClearAccountCreateAmount"]
    );
    assert_eq!(
        map_keys(getVaultCreateFlags()),
        vec!["tfVaultPrivate", "tfVaultShareNonTransferable"]
    );
    assert_eq!(
        map_keys(getBatchFlags()),
        vec![
            "tfAllOrNothing",
            "tfIndependent",
            "tfOnlyOne",
            "tfUntilFailure"
        ]
    );
    assert_eq!(map_keys(getLoanSetFlags()), vec!["tfLoanOverpayment"]);
    assert_eq!(
        map_keys(getLoanPayFlags()),
        vec![
            "tfLoanFullPayment",
            "tfLoanLatePayment",
            "tfLoanOverpayment"
        ]
    );
    assert_eq!(
        map_keys(getLoanManageFlags()),
        vec!["tfLoanDefault", "tfLoanImpair", "tfLoanUnimpair"]
    );
}

#[test]
fn tx_flag_masks_match_current_cpp_inner_batch_and_legacy_rules() {
    assert_eq!(tfBatchMask & tfInnerBatchTxn, tfInnerBatchTxn);
    assert_eq!(tfPaymentMask & tfInnerBatchTxn, 0);
    assert_eq!(tfAccountSetMask & tfInnerBatchTxn, 0);

    assert_eq!(
        tfMPTPaymentMask,
        !(tfUniversal | PAYMENT_PARTIAL_PAYMENT_FLAG)
    );
    assert_eq!(
        tfTrustSetPermissionMask,
        !(tfUniversal | 0x0001_0000 | 0x0010_0000 | 0x0020_0000)
    );

    assert_eq!(NF_TOKEN_BURNABLE_FLAG, 0x0000_0001);
    assert_eq!(tfTrustLine, 0x0000_0004);
    assert_eq!(
        NF_TOKEN_MINT_FLAGS_WITHOUT_MUTABLE,
        !(tfUniversal | 0x0000_0001 | 0x0000_0002 | 0x0000_0008)
    );
    assert_eq!(
        NF_TOKEN_MINT_OLD_FLAGS,
        !(tfUniversal | 0x0000_0001 | 0x0000_0002 | 0x0000_0008 | tfTrustLine)
    );
    assert_eq!(
        NF_TOKEN_MINT_OLD_FLAGS_WITH_MUTABLE,
        !(tfUniversal | 0x0000_0001 | 0x0000_0002 | 0x0000_0008 | tfTrustLine | 0x0000_0010)
    );
}

#[test]
fn tx_flag_catalog_keeps_current_mpt_and_batch_masks() {
    assert_eq!(MPT_ISSUANCE_CREATE_FLAGS, 0x0000_007e);
    assert_eq!(
        MPT_ISSUANCE_CREATE_FLAGS_MASK,
        transaction_flags_mask(MPT_ISSUANCE_CREATE_FLAGS)
    );
    assert_eq!(getBatchFlags().get("tfIndependent"), Some(&0x0008_0000));
    assert_eq!(getAllTxFlags()[11].0, "NFTokenCreateOffer");
    assert_eq!(getAllTxFlags()[16].0, "VaultCreate");
}
