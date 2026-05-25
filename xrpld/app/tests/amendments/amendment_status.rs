use app::{AmendmentLastVote, AmendmentStatus, AmendmentVote, KnownAmendment};
use basics::base_uint::Uint256;
use basics::chrono::{NetClockTimePoint, weeks};
use protocol::{
    JsonValue, KeyType, STValidation, STVector256, SecretKey, VF_FULL_VALIDATION, calc_node_id,
    derive_public_key, feature_id, get_field_by_symbol,
};
use std::collections::{BTreeMap, BTreeSet};

fn signed_validation(
    seed: u8,
    ledger_id: Uint256,
    seq: u32,
    amendments: &[Uint256],
) -> STValidation {
    let secret = SecretKey::from_bytes([seed; 32]);
    let public = derive_public_key(KeyType::Secp256k1, &secret).expect("validator public key");
    let mut validation = STValidation::new_signed(
        1_000,
        &public,
        calc_node_id(&public),
        &secret,
        |validation| {
            validation.set_field_h256(get_field_by_symbol("sfLedgerHash"), ledger_id);
            validation.set_field_u32(get_field_by_symbol("sfLedgerSequence"), seq);
            validation.set_flag(VF_FULL_VALIDATION);
            if !amendments.is_empty() {
                validation.set_field_v256(
                    get_field_by_symbol("sfAmendments"),
                    STVector256::from_values(
                        get_field_by_symbol("sfAmendments"),
                        amendments.to_vec(),
                    ),
                );
            }
        },
    )
    .expect("signed validation");
    validation.set_trusted();
    validation
}

#[test]
fn amendment_table_validation_only_votes_for_upvoted_supported_unenabled_features() {
    let alpha = Uint256::from_u64(1);
    let beta = Uint256::from_u64(2);
    let gamma = Uint256::from_u64(3);
    let table = AmendmentStatus::with_known_amendments(
        weeks(2),
        [
            KnownAmendment::new("alpha", alpha, true, AmendmentVote::Up),
            KnownAmendment::new("beta", beta, true, AmendmentVote::Down),
            KnownAmendment::new("gamma", gamma, false, AmendmentVote::Up),
        ],
    );

    let enabled = BTreeSet::from([beta]);
    assert_eq!(table.do_validation(&enabled), vec![alpha]);
    assert_eq!(table.get_desired(), vec![alpha]);
}

#[test]
fn amendment_table_voting_emits_got_lost_and_enable_actions() {
    let got = Uint256::from_u64(10);
    let lost = Uint256::from_u64(11);
    let held = Uint256::from_u64(12);
    let table = AmendmentStatus::with_known_amendments(
        weeks(2),
        [
            KnownAmendment::new("got", got, true, AmendmentVote::Up),
            KnownAmendment::new("lost", lost, true, AmendmentVote::Up),
            KnownAmendment::new("held", held, true, AmendmentVote::Up),
        ],
    );
    let trusted = derive_public_key(KeyType::Secp256k1, &SecretKey::from_bytes([1; 32]))
        .expect("trusted key");
    table.set_trusted_validators([trusted]);

    let close_time = NetClockTimePoint::new(2_000_000);
    let parent_ledger = Uint256::from_u64(99);
    let validations = vec![signed_validation(1, parent_ledger, 256, &[got, held])];
    let majority_time = NetClockTimePoint::new(
        close_time.as_seconds() - u32::try_from(weeks(2).whole_seconds()).expect("weeks fits u32"),
    );

    let actions = table.do_voting(
        close_time,
        &BTreeSet::new(),
        &BTreeMap::from([(lost, close_time), (held, majority_time)]),
        &validations,
    );

    assert_eq!(
        actions,
        BTreeMap::from([
            (got, protocol::ENABLE_AMENDMENT_GOT_MAJORITY_FLAG),
            (held, 0),
            (lost, protocol::ENABLE_AMENDMENT_LOST_MAJORITY_FLAG),
        ])
    );
    assert_eq!(
        table.last_vote(),
        Some(AmendmentLastVote {
            trusted_validations: 1,
            threshold: 1,
            votes: BTreeMap::from([(got, 1), (held, 1)]),
        })
    );
}

#[test]
fn amendment_table_retains_recent_trusted_votes_across_flag_rounds() {
    let amendment = Uint256::from_u64(21);
    let table = AmendmentStatus::with_known_amendments(
        weeks(2),
        [KnownAmendment::new(
            "retained",
            amendment,
            true,
            AmendmentVote::Up,
        )],
    );
    let trusted = derive_public_key(KeyType::Secp256k1, &SecretKey::from_bytes([7; 32]))
        .expect("trusted key");
    table.set_trusted_validators([trusted]);

    let parent_ledger = Uint256::from_u64(101);
    let first_round = table.do_voting(
        NetClockTimePoint::new(500),
        &BTreeSet::new(),
        &BTreeMap::new(),
        &[signed_validation(7, parent_ledger, 256, &[amendment])],
    );
    let second_round = table.do_voting(
        NetClockTimePoint::new(900),
        &BTreeSet::new(),
        &BTreeMap::new(),
        &[],
    );

    assert_eq!(
        first_round.get(&amendment),
        Some(&protocol::ENABLE_AMENDMENT_GOT_MAJORITY_FLAG)
    );
    assert_eq!(
        second_round.get(&amendment),
        Some(&protocol::ENABLE_AMENDMENT_GOT_MAJORITY_FLAG)
    );
}

#[test]
fn amendment_table_validated_ledger_tracks_first_unsupported_expected_time() {
    let unsupported = Uint256::from_u64(404);
    let table = AmendmentStatus::new();

    table.do_validated_ledger_with_sets(
        512,
        &BTreeSet::new(),
        &BTreeMap::from([(unsupported, NetClockTimePoint::new(8_000))]),
    );

    assert_eq!(
        table.first_unsupported_expected(),
        Some(NetClockTimePoint::new(
            8_000 + u32::try_from(weeks(2).whole_seconds()).expect("weeks fits u32")
        ))
    );
    assert!(!table.has_unsupported_enabled());
}

#[test]
fn default_amendment_registry_marks_current_mainnet_amendments_supported() {
    let table = AmendmentStatus::new();
    let enabled = BTreeSet::from([
        feature_id("XRPFees"),
        feature_id("AMM"),
        feature_id("XChainBridge"),
        feature_id("Clawback"),
        feature_id("fixUniversalNumber"),
        feature_id("PriceOracle"),
        feature_id("MPTokensV1"),
        feature_id("DynamicNFT"),
        feature_id("Credentials"),
        feature_id("PermissionedDomains"),
    ]);

    table.do_validated_ledger_with_sets(900_000, &enabled, &BTreeMap::new());

    assert!(!table.has_unsupported_enabled());
}

#[test]
fn feature_json_surfaces_supported_enabled_and_majority_fields() {
    let amendment = feature_id("XRPFees");
    let table = AmendmentStatus::new();
    table.do_validated_ledger_with_sets(
        910_000,
        &BTreeSet::from([amendment]),
        &BTreeMap::from([(amendment, NetClockTimePoint::new(12_345))]),
    );

    let JsonValue::Object(outer) = table
        .feature_json(amendment, true)
        .expect("registered feature should render")
    else {
        panic!("feature json must be an object");
    };

    let JsonValue::Object(feature) = outer
        .values()
        .next()
        .expect("feature json must have one entry")
    else {
        panic!("inner feature json must be an object");
    };

    assert_eq!(
        feature.get("name"),
        Some(&JsonValue::String("XRPFees".to_owned()))
    );
    assert_eq!(feature.get("supported"), Some(&JsonValue::Bool(true)));
    assert_eq!(feature.get("enabled"), Some(&JsonValue::Bool(true)));
    assert_eq!(feature.get("majority"), Some(&JsonValue::Signed(12_345)));
}
