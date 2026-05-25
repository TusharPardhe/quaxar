//! Validates that our amendment feature IDs match the live XRPL network.
//!
//! Amendment IDs are SHA-512-half of the amendment name string.
//! If these don't match, the node will fork from the network.

use basics::base_uint::Uint256;
use protocol::feature_id;

fn hex_to_uint256(hex: &str) -> Uint256 {
    let bytes: Vec<u8> = (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap())
        .collect();
    Uint256::from_slice(&bytes).unwrap()
}

/// Ground truth: amendment IDs from the live XRPL mainnet (fetched via `feature` RPC).
/// These are the canonical IDs that all C++ nodes agree on.
const MAINNET_AMENDMENTS: &[(&str, &str)] = &[
    (
        "AMM",
        "8CC0774A3BF66D1D22E76BBDA8E8A232E6B6313834301B3B23E8601196AE6455",
    ),
    (
        "Batch",
        "894646DD5284E97DECFE6674A6D6152686791C4A95F8C132CCA9BAF9E5812FB6",
    ),
    (
        "Checks",
        "157D2D480E006395B76F948E3E07A45A05FE10230D88A7993C71F97AE4B1F2D1",
    ),
    (
        "Clawback",
        "56B241D7A43D40354D02A9DC4C8DF5C7A1F930D92A9035C4E12291B3CA3E1C2B",
    ),
    (
        "CryptoConditions",
        "1562511F573A19AE9BD103B5D6B9E01B3B46805AEC5D3C4805C902B514399146",
    ),
    (
        "DID",
        "DB432C3A09D9D5DFC7859F39AE5FF767ABC59AED0A9FB441E83B814D8946C109",
    ),
    (
        "DeepFreeze",
        "DAF3A6EB04FA5DC51E8E4F23E9B7022B693EFA636F23F22664746C77B5786B23",
    ),
    (
        "DeletableAccounts",
        "30CD365592B8EE40489BA01AE2F7555CAC9C983145871DC82A42A31CF5BAE7D9",
    ),
    (
        "DepositAuth",
        "F64E1EABBE79D55B3BB82020516CEC2C582A98A6BFE20FBE9BB6A0D233418064",
    ),
    (
        "DepositPreauth",
        "3CBC5C4E630A1B82380295CDA84B32B49DD066602E74E39B85EF64137FA65194",
    ),
    (
        "DisallowIncoming",
        "47C3002ABA31628447E8E9A8B315FAA935CE30183F9A9B86845E469CA2CDC3DF",
    ),
    (
        "EnforceInvariants",
        "DC9CA96AEA1DCF83E527D1AFC916EFAF5D27388ECA4060A88817C1238CAEE0BF",
    ),
    (
        "Escrow",
        "07D43DCE529B15A10827E5E04943B496762F9A88E3268269D69C44BE49E21104",
    ),
    (
        "ExpandedSignerList",
        "B2A4DB846F0891BF2C76AB2F2ACC8F5B4EC64437135C6E56F3F859DE5FFD5856",
    ),
    (
        "FeeEscalation",
        "42426C4D4F1009EE67080A9B7965B44656D7714D104A72F9B4369F97ABF044EE",
    ),
    (
        "Flow",
        "740352F2412A9909880C23A559FCECEDA3BE2126FED62FC7660D628A06927F11",
    ),
    (
        "FlowCross",
        "3012E8230864E95A58C60FD61430D7E1B4D3353195F2981DC12B0C7C0950FFAC",
    ),
    (
        "HardenedValidations",
        "1F4AFA8FA1BC8827AD4C0F682C03A8B671DCDF6B5C4DE36D44243A684103EF88",
    ),
    (
        "LendingProtocol",
        "565B90CA1AB2B9D42208ED10884188C64F9E19083DECB9634AAF06EB03299509",
    ),
    (
        "MPTokensV1",
        "950AE2EA4654E47F04AA8739C0B214E242097E802FD372D24047A89AB1F5EC38",
    ),
    (
        "MultiSign",
        "4C97EBA926031A7CF7D7B36FDE3ED66DDA5421192D63DE53FFB46E43B9DC8373",
    ),
    (
        "NegativeUNL",
        "B4E4F5D2D6FB84DF7399960A732309C9FD530EAE5941838160042833625A6076",
    ),
    (
        "NonFungibleTokensV1_1",
        "32A122F1352A4C7B3A6D790362CC34749C5E57FCE896377BFDC6CCD14F6CD627",
    ),
    (
        "PayChan",
        "08DE7D96082187F6E6578530258C77FAABABE4C20474BDB82F04B021F1A68647",
    ),
    (
        "PermissionedDomains",
        "A730EB18A9D4BB52502C898589558B4CCEB4BE10044500EE5581137A2E80E849",
    ),
    (
        "PriceOracle",
        "96FD2F293A519AE1DB6F8BED23E4AD9119342DA7CB6BAFD00953D16C54205D8B",
    ),
    (
        "RequireFullyCanonicalSig",
        "00C1FC4A53E60AB02C864641002B3172F38677E29C26C5406685179B37E1EDAC",
    ),
    (
        "SingleAssetVault",
        "81BD2619B6B3C8625AC5D0BC01DE17F06C3F0AB95C7C87C93715B87A4FD240D8",
    ),
    (
        "TickSize",
        "532651B4FD58DF8922A49BA101AB3E996E5BFBF95A913B3E392504863E63B164",
    ),
    (
        "TicketBatch",
        "955DF3FA5891195A9DAEFA1DDC6BB244B545DDE1BAA84CBB25D5F12A8DA68A0C",
    ),
    (
        "TokenEscrow",
        "138B968F25822EFBF54C00F97031221C47B1EAB8321D93C7C2AEAF85F04EC5DF",
    ),
    (
        "XChainBridge",
        "C98D98EE9616ACD36E81FDEB8D41D349BF5F1B41DD64A0ABC1FE9AA5EA267E9C",
    ),
    (
        "XRPFees",
        "93E516234E35E08CA689FA33A6D38E103881F8DCB53023F728C307AA89D515A7",
    ),
    (
        "fixUniversalNumber",
        "2E2FB9CF8A44EB80F4694D38AADAE9B8B7ADAFD2F092E10068E61C98C4F092B0",
    ),
    (
        "fixInnerObjTemplate",
        "C393B3AEEBF575E475F0C60D5E4241B2070CC4D0EB6C4846B1A07508FAEFC485",
    ),
    (
        "fixInnerObjTemplate2",
        "9196110C23EA879B4229E51C286180C7D02166DA712559F634372F5264D0EC59",
    ),
    (
        "fixPreviousTxnID",
        "7BB62DC13EC72B775091E9C71BF8CF97E122647693B50C5E87A80DFD6FCFAC50",
    ),
    (
        "fixAMMv1_1",
        "35291ADD2D79EB6991343BDA0912269C817D0F094B02226C1C14AD2858962ED4",
    ),
    (
        "fixAMMv1_3",
        "7CA70A7674A26FA517412858659EBC7EDEEF7D2D608824464E6FDEFD06854E14",
    ),
    (
        "fixNFTokenPageLinks",
        "C7981B764EC4439123A86CC7CCBA436E9B3FF73B3F10A0AE51882E404522FC41",
    ),
    (
        "fixEnforceNFTokenTrustlineV2",
        "B32752F7DCC41FB86534118FC4EEC8F56E7BD0A7DB60FD73F93F257233C08E3A",
    ),
    (
        "fixTokenEscrowV1",
        "32B8614321F7E070419115ABEAB1742EA20F3E3AF34432B5E2F474F8083260DC",
    ),
    (
        "NFTokenMintOffer",
        "EE3CF852F0506782D05E65D49E5DCC3D16D50898CD1B646BAE274863401CC3CE",
    ),
];

/// Core test: verify our feature_id() produces the same hash as the live network.
#[test]
fn amendment_ids_match_mainnet() {
    let mut mismatches = Vec::new();

    for (name, expected_hex) in MAINNET_AMENDMENTS {
        let computed = feature_id(name);
        let expected = hex_to_uint256(expected_hex);

        if computed != expected {
            mismatches.push(format!(
                "  {name}:\n    computed: {computed}\n    expected: {expected}"
            ));
        }
    }

    assert!(
        mismatches.is_empty(),
        "Amendment ID mismatches ({}/{}):\n{}",
        mismatches.len(),
        MAINNET_AMENDMENTS.len(),
        mismatches.join("\n")
    );
}

/// Test: feature_id is deterministic.
#[test]
fn feature_id_is_deterministic() {
    for (name, _) in MAINNET_AMENDMENTS.iter().take(10) {
        let id1 = feature_id(name);
        let id2 = feature_id(name);
        assert_eq!(id1, id2, "feature_id must be deterministic for '{name}'");
    }
}

/// Test: different names produce different IDs.
#[test]
fn different_names_produce_different_ids() {
    let id_amm = feature_id("AMM");
    let id_xrp = feature_id("XRPFees");
    let id_batch = feature_id("Batch");

    assert_ne!(id_amm, id_xrp);
    assert_ne!(id_amm, id_batch);
    assert_ne!(id_xrp, id_batch);
}
