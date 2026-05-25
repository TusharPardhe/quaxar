use protocol::{
    getAccountRootFlags, getAllLedgerFlags, getLoanFlags, getMPTokenFlags, getOfferFlags,
    getRippleStateFlags, getVaultFlags, lsfAMMNode, lsfAllowTrustLineClawback,
    lsfDisallowIncomingCheck, lsfLoanOverpayment, lsfMPTAuthorized, lsfPasswordSpent, lsfSell,
    lsfVaultPrivate,
};

fn map_keys(map: &protocol::LedgerFlagMap) -> Vec<&str> {
    map.keys().map(String::as_str).collect()
}

#[test]
fn ledger_flag_maps_match_current_cpp_catalog_shape() {
    let account_root = getAccountRootFlags();
    assert_eq!(
        account_root.get("lsfPasswordSpent"),
        Some(&lsfPasswordSpent)
    );
    assert_eq!(
        account_root.get("lsfAllowTrustLineClawback"),
        Some(&lsfAllowTrustLineClawback)
    );
    assert_eq!(
        account_root.get("lsfDisallowIncomingCheck"),
        Some(&lsfDisallowIncomingCheck)
    );

    let offer = getOfferFlags();
    assert_eq!(offer.get("lsfSell"), Some(&lsfSell));

    let ripple_state = getRippleStateFlags();
    assert_eq!(ripple_state.get("lsfAMMNode"), Some(&lsfAMMNode));
    assert_eq!(ripple_state.get("lsfDefaultRipple"), None);

    let mp_token = getMPTokenFlags();
    assert_eq!(mp_token.get("lsfMPTAuthorized"), Some(&lsfMPTAuthorized));

    let vault = getVaultFlags();
    assert_eq!(vault.get("lsfVaultPrivate"), Some(&lsfVaultPrivate));

    let loan = getLoanFlags();
    assert_eq!(loan.get("lsfLoanOverpayment"), Some(&lsfLoanOverpayment));
}

#[test]
fn all_ledger_flag_maps_preserve_cpp_group_order() {
    let all = getAllLedgerFlags();
    assert_eq!(all.len(), 12);
    assert_eq!(all.first().expect("first group").0, "AccountRoot");
    assert_eq!(all.last().expect("last group").0, "Loan");
    assert_eq!(
        map_keys(&all[0].1),
        vec![
            "lsfAllowTrustLineClawback",
            "lsfAllowTrustLineLocking",
            "lsfDefaultRipple",
            "lsfDepositAuth",
            "lsfDisableMaster",
            "lsfDisallowIncomingCheck",
            "lsfDisallowIncomingNFTokenOffer",
            "lsfDisallowIncomingPayChan",
            "lsfDisallowIncomingTrustline",
            "lsfDisallowXRP",
            "lsfGlobalFreeze",
            "lsfNoFreeze",
            "lsfPasswordSpent",
            "lsfRequireAuth",
            "lsfRequireDestTag",
        ]
    );
}
