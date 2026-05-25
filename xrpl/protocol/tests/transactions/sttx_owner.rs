use basics::base_uint::Uint256;
use protocol::{
    AccountID, HashPrefix, KeyType, LoanSetBuilder, NumberJsonInput, Rules, STAmount, STArray,
    STNumber, STObject, STTx, STUInt32, STVar, SecretKey, Serializer, StBase, TxType,
    calc_account_id, derive_public_key, get_field_by_symbol, passes_local_checks, sf_generic, sign,
    sterilize,
};
use std::panic::{AssertUnwindSafe, catch_unwind};

fn account(fill: u8) -> AccountID {
    AccountID::from_array([fill; 20])
}

fn payment_tx(sequence: u32) -> STTx {
    STTx::new(TxType::PAYMENT, |tx| {
        tx.set_account_id(get_field_by_symbol("sfAccount"), account(0x11));
        tx.set_account_id(get_field_by_symbol("sfDestination"), account(0x22));
        tx.set_field_amount(
            get_field_by_symbol("sfAmount"),
            STAmount::new_native(1_000_000, false),
        );
        tx.set_field_amount(
            get_field_by_symbol("sfFee"),
            STAmount::new_native(10, false),
        );
        tx.set_field_u32(get_field_by_symbol("sfSequence"), sequence);
    })
}

fn raw_transaction_array() -> (STArray, Vec<Uint256>) {
    let first = payment_tx(3);
    let second = payment_tx(4);

    let mut first_raw = first.clone_as_object();
    first_raw.set_fname(get_field_by_symbol("sfRawTransaction"));

    let mut second_raw = second.clone_as_object();
    second_raw.set_fname(get_field_by_symbol("sfRawTransaction"));

    let mut array = STArray::new(get_field_by_symbol("sfRawTransactions"));
    array.push_back(first_raw);
    array.push_back(second_raw);

    (
        array,
        vec![first.get_transaction_id(), second.get_transaction_id()],
    )
}

#[test]
fn protocol_sttx_get_signature_returns_empty_on_wrong_field_type() {
    let mut object = STObject::new(sf_generic());
    object.emplace_back(STVar::new(STUInt32::with_field(
        get_field_by_symbol("sfTxnSignature"),
        7,
    )));

    assert!(STTx::get_signature(&object).is_empty());
}

#[test]
fn protocol_sttx_sign_refreshes_hash_without_rewriting_signing_pub_key() {
    let secret = SecretKey::from_bytes([0x31; 32]);
    let public = derive_public_key(KeyType::Secp256k1, &secret).expect("public key");

    let mut tx = payment_tx(10);
    tx.set_field_vl(get_field_by_symbol("sfSigningPubKey"), public.as_bytes());
    let before = tx.get_transaction_id();

    tx.sign(&public, &secret, None)
        .expect("signature should succeed");

    assert_eq!(
        tx.get_field_vl(get_field_by_symbol("sfSigningPubKey")),
        public.as_bytes().to_vec()
    );
    assert!(!STTx::get_signature(&tx).is_empty());
    assert_ne!(tx.get_transaction_id(), before);
    assert_eq!(tx.check_sign(&Rules::default()), Ok(()));
}

#[test]
fn protocol_sttx_counterparty_signature_target_and_error_prefix_match_cpp() {
    let borrower_secret = SecretKey::from_bytes([0x41; 32]);
    let borrower_public =
        derive_public_key(KeyType::Secp256k1, &borrower_secret).expect("borrower public key");
    let counterparty_secret = SecretKey::from_bytes([0x42; 32]);
    let counterparty_public = derive_public_key(KeyType::Secp256k1, &counterparty_secret)
        .expect("counterparty public key");

    let borrower = calc_account_id(borrower_public.as_bytes());
    let counterparty = calc_account_id(counterparty_public.as_bytes());

    let loan = LoanSetBuilder::new(
        borrower,
        Uint256::from_array([0xAB; 32]),
        STNumber::from_json_input(NumberJsonInput::UInt(100)).expect("number"),
        Some(1),
        Some(STAmount::new_native(10, false)),
    )
    .set_counterparty(counterparty)
    .build(&borrower_public, &borrower_secret)
    .expect("loan set should build");

    let mut tx = loan.tx().as_ref().clone();
    tx.peek_field_object(get_field_by_symbol("sfCounterpartySignature"))
        .set_field_vl(
            get_field_by_symbol("sfSigningPubKey"),
            counterparty_public.as_bytes(),
        );
    tx.sign(
        &counterparty_public,
        &counterparty_secret,
        Some(get_field_by_symbol("sfCounterpartySignature")),
    )
    .expect("counterparty signature should succeed");

    assert_eq!(tx.check_sign(&Rules::default()), Ok(()));

    tx.peek_field_object(get_field_by_symbol("sfCounterpartySignature"))
        .set_field_vl(get_field_by_symbol("sfTxnSignature"), &[0x00, 0x01]);

    assert_eq!(
        tx.check_sign(&Rules::default()),
        Err("Counterparty: Invalid signature.".to_owned())
    );
}

#[test]
fn protocol_sttx_batch_ids_and_signature_checks_match_current_cpp() {
    let signer_secret = SecretKey::from_bytes([0x51; 32]);
    let signer_public =
        derive_public_key(KeyType::Secp256k1, &signer_secret).expect("signer public key");
    let signer_account = calc_account_id(signer_public.as_bytes());

    let (raw_transactions, expected_ids) = raw_transaction_array();
    let mut batch = STTx::new(TxType::BATCH, |tx| {
        tx.set_account_id(get_field_by_symbol("sfAccount"), signer_account);
        tx.set_field_amount(
            get_field_by_symbol("sfFee"),
            STAmount::new_native(10, false),
        );
        tx.set_field_u32(get_field_by_symbol("sfSequence"), 1);
        tx.set_field_array(get_field_by_symbol("sfRawTransactions"), raw_transactions);
    });

    let tx_ids = batch.get_batch_transaction_ids();
    assert_eq!(tx_ids, expected_ids);

    let mut batch_message = Serializer::default();
    batch_message.add32_prefix(HashPrefix::Batch);
    batch_message.add32(batch.get_flags());
    batch_message.add32(tx_ids.len() as u32);
    for tx_id in &tx_ids {
        batch_message.add_bit_string(*tx_id);
    }

    let signature = sign(&signer_public, &signer_secret, batch_message.data()).expect("signature");
    let mut batch_signer = STObject::make_inner_object(get_field_by_symbol("sfBatchSigner"));
    batch_signer.set_account_id(get_field_by_symbol("sfAccount"), signer_account);
    batch_signer.set_field_vl(
        get_field_by_symbol("sfSigningPubKey"),
        signer_public.as_bytes(),
    );
    batch_signer.set_field_vl(get_field_by_symbol("sfTxnSignature"), &signature);

    let mut batch_signers = STArray::new(get_field_by_symbol("sfBatchSigners"));
    batch_signers.push_back(batch_signer);
    batch.set_field_array(get_field_by_symbol("sfBatchSigners"), batch_signers);

    assert_eq!(batch.check_batch_sign(&Rules::default()), Ok(()));
}

#[test]
fn protocol_sttx_batch_ids_remain_stable_on_repeated_access() {
    let (raw_transactions, expected_ids) = raw_transaction_array();
    let batch = STTx::new(TxType::BATCH, |tx| {
        tx.set_account_id(get_field_by_symbol("sfAccount"), account(0x31));
        tx.set_field_amount(
            get_field_by_symbol("sfFee"),
            STAmount::new_native(10, false),
        );
        tx.set_field_u32(get_field_by_symbol("sfSequence"), 2);
        tx.set_field_array(get_field_by_symbol("sfRawTransactions"), raw_transactions);
    });

    let first = batch.get_batch_transaction_ids();
    let second = batch.get_batch_transaction_ids();

    assert_eq!(first, expected_ids);
    assert_eq!(second, expected_ids);
}

#[test]
fn protocol_sttx_batch_ids_reject_late_raw_transaction_count_changes() {
    let (raw_transactions, expected_ids) = raw_transaction_array();
    let mut batch = STTx::new(TxType::BATCH, |tx| {
        tx.set_account_id(get_field_by_symbol("sfAccount"), account(0x32));
        tx.set_field_amount(
            get_field_by_symbol("sfFee"),
            STAmount::new_native(10, false),
        );
        tx.set_field_u32(get_field_by_symbol("sfSequence"), 3);
        tx.set_field_array(get_field_by_symbol("sfRawTransactions"), raw_transactions);
    });

    assert_eq!(batch.get_batch_transaction_ids(), expected_ids);

    let mut extra = payment_tx(5).clone_as_object();
    extra.set_fname(get_field_by_symbol("sfRawTransaction"));
    batch
        .peek_field_array(get_field_by_symbol("sfRawTransactions"))
        .push_back(extra);

    let panic = catch_unwind(AssertUnwindSafe(|| batch.get_batch_transaction_ids()))
        .expect_err("changed raw transaction count should panic");
    let message = panic
        .downcast_ref::<String>()
        .map(String::as_str)
        .or_else(|| panic.downcast_ref::<&'static str>().copied())
        .expect("panic message");
    assert!(
        message.contains("STTx::getBatchTransactionIDs : batch transaction IDs size mismatch"),
        "unexpected panic message: {message}"
    );
}

#[test]
fn protocol_sttx_local_checks_reject_pseudo_transactions_and_invalid_accounts() {
    let secret = SecretKey::from_bytes([0x61; 32]);
    let public = derive_public_key(KeyType::Secp256k1, &secret).expect("public key");

    let pseudo = protocol::EnableAmendmentBuilder::new(
        account(0x10),
        7,
        Uint256::from_array([0xAA; 32]),
        Some(1),
        Some(STAmount::new_native(10, false)),
    )
    .build(&public, &secret)
    .expect("enable amendment");

    assert_eq!(
        passes_local_checks(pseudo.as_sttx()),
        Err("Cannot submit pseudo transactions.".to_owned())
    );

    let mut bad_account = payment_tx(8);
    bad_account.make_field_present(get_field_by_symbol("sfDelegate"));

    assert_eq!(
        passes_local_checks(&bad_account),
        Err("An account field is invalid.".to_owned())
    );
}

#[test]
fn protocol_sttx_sterilize_round_trips_canonical_bytes() {
    let tx = payment_tx(13);
    let sterilized = sterilize(&tx);

    assert_eq!(sterilized.get_transaction_id(), tx.get_transaction_id());
    assert_eq!(
        sterilized.get_serializer().data(),
        tx.get_serializer().data()
    );
}
