//! Golden vector validation: verify Rust ledger hash computation matches real XRPL mainnet data.
//!
//! These test vectors are real mainnet ledger headers fetched from the XRPL public API.
//! If these pass, the Rust node will produce valid ledger hashes on the network.

use basics::base_uint::Uint256;
use basics::sha_map_hash::SHAMapHash;
use protocol::{LedgerHeader, calculate_ledger_hash};

fn hash_from_hex(hex: &str) -> SHAMapHash {
    let mut bytes = [0u8; 32];
    for i in 0..32 {
        bytes[i] = u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16).unwrap();
    }
    SHAMapHash::new(Uint256::from(bytes))
}

struct TestVector {
    seq: u32,
    drops: u64,
    parent_hash: &'static str,
    tx_hash: &'static str,
    account_hash: &'static str,
    parent_close_time: u32,
    close_time: u32,
    close_time_resolution: u8,
    close_flags: u8,
    expected_hash: &'static str,
}

/// Real XRPL mainnet ledger headers — the ground truth.
const VECTORS: &[TestVector] = &[
    // Ledger #90,000,000 — Aug 12, 2024
    TestVector {
        seq: 90_000_000,
        drops: 99_987_314_818_963_207,
        parent_hash: "BDFC7450C4A23EDA4914C610EF9377326C3EB43336E4B61F26765AC848F0B3B0",
        tx_hash: "AA41663B2572325D04C844928E21AE221071B510274F151F1D5C9E39A89AE22E",
        account_hash: "B90C97F3FC402B0F72782BBDC900CDA15AEBD3D50DFDD8EFF2C65FD542D30170",
        parent_close_time: 776_753_590,
        close_time: 776_753_591,
        close_time_resolution: 10,
        close_flags: 0,
        expected_hash: "1A681F8BB1341C25B5FA89C0BCAA69E5CC44360AF3BB613483190909A299E23B",
    },
    // Ledger #85,000,000 — Dec 2023
    TestVector {
        seq: 85_000_000,
        drops: 99_988_056_755_485_958,
        parent_hash: "7D6031A585EBD1F2B3E22596078994F71D8CC5139FF9E9C606846387F57E0B83",
        tx_hash: "512BB29000F9A878E39EF4FE3EF71AD3BC48C4416BCD2CF7386E257DF48D295D",
        account_hash: "D7299E64F5984217BFE4A08B4E7512971C0DCFD881104B91FE6DD3BA86224422",
        parent_close_time: 757_482_891,
        close_time: 757_482_900,
        close_time_resolution: 10,
        close_flags: 0,
        expected_hash: "9005D563EC676ADE18EE030926DC0BC9DD963E1090D4A8B3D679F98033B013E6",
    },
    // Ledger #80,000,000 — Jun 2023
    TestVector {
        seq: 80_000_000,
        drops: 99_988_894_618_845_389,
        parent_hash: "E70235CEAA964202C74CD81132C69F8F44FE539F9E347F82AA3131410B7CDBEF",
        tx_hash: "CDA48280EF5F88BC86164A77E1072296899B384619D552445D71C922026A3E8C",
        account_hash: "AE7B283387566579CD73E4E720F29E81DC11218536BDBB7C1D83A427AC041D79",
        parent_close_time: 738_268_730,
        close_time: 738_268_731,
        close_time_resolution: 10,
        close_flags: 0,
        expected_hash: "DB978F031BB14734213998060E077D5F813358222DAB07CA8148588D852A55DF",
    },
    // Ledger #75,000,000 — Dec 2022
    TestVector {
        seq: 75_000_000,
        drops: 99_989_256_070_974_321,
        parent_hash: "6C8C25B25BD3F5062BDBDFF4703405810689326DEF1E3E7F49C2AEF025925C01",
        tx_hash: "E86DF89254922F80E7AFA183D967EB7605D76B45EC3D5C96736D2CBA4E212442",
        account_hash: "C00F8E216CE9901D4BA535FE759A872D02256EE031CBCCA750FCAC37A6317A0F",
        parent_close_time: 718_851_971,
        close_time: 718_851_980,
        close_time_resolution: 10,
        close_flags: 0,
        expected_hash: "20D7A4EA1ED9A088AB604DB17F6204693E0BA9F9456628862EDCB6B0670B7A43",
    },
    // Ledger #70,000,000 — Jun 2022
    TestVector {
        seq: 70_000_000,
        drops: 99_989_677_625_083_371,
        parent_hash: "98E2337799919AE78E4A627CA014BA14B0B9E188B14AFC600402C94D5512FA4C",
        tx_hash: "FDB6FDF142CB36E1FF4B31BB5C6906D3A9D00369744A69D5A3F1CD1AFF5E832F",
        account_hash: "2BACA2F4879C39CD11D49D80233AF27F7EFEC796B2F43BE65A8B4C0FFEC6CBB0",
        parent_close_time: 699_397_401,
        close_time: 699_397_402,
        close_time_resolution: 10,
        close_flags: 0,
        expected_hash: "A6D1B3DAFF5F255C5DBB34307FA275A9B33A7457EFFFEF8113968B9A709D8C48",
    },
];

#[test]
fn ledger_hash_matches_mainnet_for_all_vectors() {
    for (i, v) in VECTORS.iter().enumerate() {
        let header = LedgerHeader {
            seq: v.seq,
            drops: v.drops,
            parent_hash: hash_from_hex(v.parent_hash),
            tx_hash: hash_from_hex(v.tx_hash),
            account_hash: hash_from_hex(v.account_hash),
            parent_close_time: v.parent_close_time,
            close_time: v.close_time,
            close_time_resolution: v.close_time_resolution,
            close_flags: v.close_flags,
            ..LedgerHeader::default()
        };

        let computed = calculate_ledger_hash(&header);
        let expected = hash_from_hex(v.expected_hash);

        assert_eq!(
            computed,
            expected,
            "LEDGER #{} (seq={}): hash mismatch!\n  computed: {}\n  expected: {}",
            i + 1,
            v.seq,
            computed.as_uint256(),
            expected.as_uint256(),
        );
    }
}

#[test]
fn ledger_hash_changes_if_any_field_differs() {
    let v = &VECTORS[0];
    let base = LedgerHeader {
        seq: v.seq,
        drops: v.drops,
        parent_hash: hash_from_hex(v.parent_hash),
        tx_hash: hash_from_hex(v.tx_hash),
        account_hash: hash_from_hex(v.account_hash),
        parent_close_time: v.parent_close_time,
        close_time: v.close_time,
        close_time_resolution: v.close_time_resolution,
        close_flags: v.close_flags,
        ..LedgerHeader::default()
    };

    let correct = calculate_ledger_hash(&base);

    // Changing any single field must produce a different hash
    let mut modified = base;
    modified.seq += 1;
    assert_ne!(
        calculate_ledger_hash(&modified),
        correct,
        "seq change not detected"
    );

    let mut modified = base;
    modified.drops += 1;
    assert_ne!(
        calculate_ledger_hash(&modified),
        correct,
        "drops change not detected"
    );

    let mut modified = base;
    modified.close_time += 1;
    assert_ne!(
        calculate_ledger_hash(&modified),
        correct,
        "close_time change not detected"
    );

    let mut modified = base;
    modified.close_flags = 1;
    assert_ne!(
        calculate_ledger_hash(&modified),
        correct,
        "close_flags change not detected"
    );
}
