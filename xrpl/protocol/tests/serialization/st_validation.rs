use basics::base_uint::Uint256;
use protocol::{
    HashPrefix, KeyType, NodeId, STValidation, STVar, SecretKey, SerializedTypeId, Serializer,
    StBase, VF_FULL_VALIDATION, VF_FULLY_CANONICAL_SIG, calc_node_id, derive_public_key,
    get_field_by_symbol,
};

fn hash(fill: u8) -> Uint256 {
    Uint256::from_array([fill; 32])
}

#[test]
fn protocol_stvalidation_signed_constructor_sets_trusted_signature_and_flags() {
    let secret = SecretKey::from_bytes([0x44; 32]);
    let public = derive_public_key(KeyType::Secp256k1, &secret).expect("public key");
    let node_id = calc_node_id(&public);

    let validation = STValidation::new_signed(700, &public, node_id, &secret, |validation| {
        validation.set_field_h256(get_field_by_symbol("sfLedgerHash"), hash(0xA1));
        validation.set_field_h256(get_field_by_symbol("sfConsensusHash"), hash(0xB2));
        validation.set_field_u32(get_field_by_symbol("sfLedgerSequence"), 55);
        validation.set_flag(VF_FULL_VALIDATION);
    })
    .expect("signed validation should build");

    assert_eq!(validation.fname(), get_field_by_symbol("sfValidation"));
    assert_eq!(validation.stype(), SerializedTypeId::Object);
    assert_eq!(validation.get_signer_public(), &public);
    assert_eq!(validation.get_node_id(), node_id);
    assert_eq!(validation.get_sign_time(), 700);
    assert_eq!(validation.get_seen_time(), 700);
    assert_eq!(validation.get_ledger_hash(), hash(0xA1));
    assert_eq!(validation.get_consensus_hash(), hash(0xB2));
    assert!(validation.is_full());
    assert!(validation.is_trusted());
    assert!(validation.is_valid());
    assert!((validation.get_flags() & VF_FULLY_CANONICAL_SIG) != 0);
    assert!(!validation.get_signature().is_empty());
    assert!(validation.render().contains("base58: "));
}

#[test]
fn protocol_stvalidation_from_serialized_uses_lookup_node_id_and_checks_signature() {
    let secret = SecretKey::from_bytes([0x55; 32]);
    let public = derive_public_key(KeyType::Secp256k1, &secret).expect("public key");

    let validation =
        STValidation::new_signed(900, &public, calc_node_id(&public), &secret, |validation| {
            validation.set_field_h256(get_field_by_symbol("sfLedgerHash"), hash(0xC1));
            validation.set_field_u32(get_field_by_symbol("sfLedgerSequence"), 88);
        })
        .expect("signed validation should build");

    let custom_node_id =
        NodeId::from_hex("1111111111111111111111111111111111111111").expect("node id hex");
    let serialized = validation.get_serialized();
    let mut iter = protocol::SerialIter::new(&serialized);
    let reparsed = STValidation::from_serial_iter(
        &mut iter,
        |lookup_public| {
            assert_eq!(lookup_public, &public);
            custom_node_id
        },
        true,
    )
    .expect("serialized validation should verify");

    assert_eq!(reparsed.get_node_id(), custom_node_id);
    assert_eq!(reparsed.get_signer_public(), &public);
    assert_eq!(reparsed.get_ledger_hash(), hash(0xC1));
    assert_eq!(
        reparsed.get_field_u32(get_field_by_symbol("sfLedgerSequence")),
        88
    );
    assert!(reparsed.is_valid());
    assert!(!reparsed.is_trusted());
}

#[test]
fn protocol_stvalidation_rejects_ed25519_validation_keys() {
    let secret = SecretKey::from_bytes([0x66; 32]);
    let public = derive_public_key(KeyType::Ed25519, &secret).expect("public key");

    let error = STValidation::new_signed(
        1000,
        &public,
        NodeId::from_hex("2222222222222222222222222222222222222222").expect("node id"),
        &secret,
        |_| {},
    )
    .expect_err("ed25519 validations should be rejected");

    assert_eq!(error, protocol::StValidationError::KeyTypeMismatch);
}

#[test]
fn protocol_stvalidation_rejects_bad_serialized_signature() {
    let secret = SecretKey::from_bytes([0x77; 32]);
    let public = derive_public_key(KeyType::Secp256k1, &secret).expect("public key");
    let mut validation = STValidation::new_signed(
        1234,
        &public,
        calc_node_id(&public),
        &secret,
        |validation| {
            validation.set_field_h256(get_field_by_symbol("sfLedgerHash"), hash(0xD1));
            validation.set_field_u32(get_field_by_symbol("sfLedgerSequence"), 99);
        },
    )
    .expect("signed validation should build");

    validation.set_field_vl(get_field_by_symbol("sfSignature"), &[0x00, 0x01]);
    let serialized = validation.get_serialized();
    let mut iter = protocol::SerialIter::new(&serialized);

    let error = STValidation::from_serial_iter_default_node_id(&mut iter, true)
        .expect_err("bad validation signature should fail");
    assert_eq!(error, protocol::StValidationError::InvalidSignature);
}

#[test]
fn protocol_stvar_rejects_validation_transaction_and_ledger_entry_root_types() {
    let pseudo_root_types = [
        (
            SerializedTypeId::Validation,
            get_field_by_symbol("sfValidation"),
        ),
        (
            SerializedTypeId::Transaction,
            get_field_by_symbol("sfTransaction"),
        ),
        (
            SerializedTypeId::LedgerEntry,
            get_field_by_symbol("sfLedgerEntry"),
        ),
    ];

    for (type_id, field) in pseudo_root_types {
        let panic = std::panic::catch_unwind(|| STVar::from_serialized_type(type_id, field))
            .expect_err("pseudo-root STVar type should stay unsupported");
        let message = panic
            .downcast_ref::<String>()
            .map(String::as_str)
            .or_else(|| panic.downcast_ref::<&'static str>().copied())
            .expect("panic payload should be string");
        assert_eq!(message, "Unknown object type");
    }
}

#[test]
fn protocol_stvalidation_signing_hash_uses_validation_prefix() {
    let secret = SecretKey::from_bytes([0x88; 32]);
    let public = derive_public_key(KeyType::Secp256k1, &secret).expect("public key");

    let validation =
        STValidation::new_signed(333, &public, calc_node_id(&public), &secret, |validation| {
            validation.set_field_h256(get_field_by_symbol("sfLedgerHash"), hash(0xE1));
            validation.set_field_u32(get_field_by_symbol("sfLedgerSequence"), 144);
        })
        .expect("signed validation should build");

    let mut manual = Serializer::default();
    manual.add32_prefix(HashPrefix::Validation);
    validation.add_without_signing_fields(&mut manual);

    assert_eq!(validation.get_signing_hash(), manual.get_sha512_half());
}
