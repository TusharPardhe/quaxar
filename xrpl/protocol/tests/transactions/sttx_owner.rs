use basics::base_uint::Uint256;
use protocol::{
    AccountID, HashPrefix, KeyType, LoanSetBuilder, NumberJsonInput, Rules, STAmount, STArray,
    STNumber, STObject, STTx, STUInt16, STUInt32, STVar, SecretKey, Serializer, SignatureRole,
    StBase, TxType, build_multi_signing_data_with_prefix, calc_account_id, derive_public_key,
    fix_cleanup_3_4_0, get_field_by_symbol, passes_local_checks, sf_generic, sign, signing_prefix,
    sterilize,
};

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
fn protocol_sttx_sponsor_signature_is_checked_with_upstream_error_context() {
    let payer_secret = SecretKey::from_bytes([0x43; 32]);
    let payer_public =
        derive_public_key(KeyType::Secp256k1, &payer_secret).expect("payer public key");
    let sponsor_secret = SecretKey::from_bytes([0x44; 32]);
    let sponsor_public =
        derive_public_key(KeyType::Secp256k1, &sponsor_secret).expect("sponsor public key");

    let mut tx = payment_tx(11);
    tx.set_field_vl(
        get_field_by_symbol("sfSigningPubKey"),
        payer_public.as_bytes(),
    );
    tx.set_account_id(
        get_field_by_symbol("sfSponsor"),
        calc_account_id(sponsor_public.as_bytes()),
    );
    tx.peek_field_object(get_field_by_symbol("sfSponsorSignature"))
        .set_field_vl(
            get_field_by_symbol("sfSigningPubKey"),
            sponsor_public.as_bytes(),
        );
    tx.sign(&payer_public, &payer_secret, None)
        .expect("payer signature should succeed");
    tx.sign(
        &sponsor_public,
        &sponsor_secret,
        Some(get_field_by_symbol("sfSponsorSignature")),
    )
    .expect("sponsor signature should succeed");

    assert_eq!(tx.check_sign(&Rules::default()), Ok(()));

    tx.peek_field_object(get_field_by_symbol("sfSponsorSignature"))
        .set_field_vl(get_field_by_symbol("sfTxnSignature"), &[0x00, 0x01]);
    assert_eq!(
        tx.check_sign(&Rules::default()),
        Err("Sponsor: Invalid signature.".to_owned())
    );
}

#[test]
fn protocol_sttx_batch_ids_and_signature_checks_match_current_cpp() {
    let signer_secret = SecretKey::from_bytes([0x51; 32]);
    let signer_public =
        derive_public_key(KeyType::Secp256k1, &signer_secret).expect("signer public key");
    let signer_account = calc_account_id(signer_public.as_bytes());
    let outer_account = account(0x50);

    let (raw_transactions, expected_ids) = raw_transaction_array();
    let mut batch = STTx::new(TxType::BATCH, |tx| {
        tx.set_account_id(get_field_by_symbol("sfAccount"), outer_account);
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
    batch_message.add_bit_string(outer_account);
    batch_message.add32(batch.get_seq_value());
    batch_message.add32(batch.get_flags());
    batch_message.add32(tx_ids.len() as u32);
    for tx_id in &tx_ids {
        batch_message.add_bit_string(*tx_id);
    }
    batch_message.add_bit_string(signer_account);

    let signature = sign(&signer_public, &signer_secret, batch_message.data()).expect("signature");
    let mut batch_signer = STObject::make_inner_object(get_field_by_symbol("sfBatchSigner"));
    batch_signer.set_account_id(get_field_by_symbol("sfAccount"), signer_account);
    batch_signer.set_field_vl(
        get_field_by_symbol("sfSigningPubKey"),
        signer_public.as_bytes(),
    );
    batch_signer.set_field_vl(get_field_by_symbol("sfTxnSignature"), &signature);

    let mut batch_signers = STArray::new(get_field_by_symbol("sfBatchSigners"));
    batch_signers.push_back(batch_signer.clone());
    batch.set_field_array(get_field_by_symbol("sfBatchSigners"), batch_signers);
    batch.set_field_vl(
        get_field_by_symbol("sfSigningPubKey"),
        signer_public.as_bytes(),
    );
    batch
        .sign(&signer_public, &signer_secret, None)
        .expect("outer batch signature should succeed");

    assert_eq!(batch.check_batch_sign(&Rules::default()), Ok(()));
    assert_eq!(batch.check_sign(&Rules::default()), Ok(()));

    batch.set_field_u32(get_field_by_symbol("sfSequence"), 2);
    assert!(batch.check_batch_sign(&Rules::default()).is_err());
    batch.set_field_u32(get_field_by_symbol("sfSequence"), 1);
    batch.set_account_id(get_field_by_symbol("sfAccount"), account(0x52));
    assert!(batch.check_batch_sign(&Rules::default()).is_err());
    batch.set_account_id(get_field_by_symbol("sfAccount"), outer_account);

    let mut invalid_batch_signer = batch_signer;
    invalid_batch_signer.set_field_vl(get_field_by_symbol("sfTxnSignature"), &[0x00, 0x01]);
    let mut invalid_batch_signers = STArray::new(get_field_by_symbol("sfBatchSigners"));
    invalid_batch_signers.push_back(invalid_batch_signer);
    batch.set_field_array(get_field_by_symbol("sfBatchSigners"), invalid_batch_signers);
    batch
        .sign(&signer_public, &signer_secret, None)
        .expect("outer batch signature should remain valid");

    assert!(batch.check_sign(&Rules::default()).is_err());
}

#[test]
fn protocol_sttx_batch_multisignatures_bind_batch_signer_account() {
    let multisigner_secret = SecretKey::from_bytes([0x52; 32]);
    let multisigner_public =
        derive_public_key(KeyType::Secp256k1, &multisigner_secret).expect("multisigner public key");
    let multisigner_account = calc_account_id(multisigner_public.as_bytes());
    let outer_account = account(0x53);
    let batch_signer_account = account(0x54);

    let (raw_transactions, expected_ids) = raw_transaction_array();
    let batch = STTx::new(TxType::BATCH, |tx| {
        tx.set_account_id(get_field_by_symbol("sfAccount"), outer_account);
        tx.set_field_amount(
            get_field_by_symbol("sfFee"),
            STAmount::new_native(10, false),
        );
        tx.set_field_u32(get_field_by_symbol("sfSequence"), 1);
        tx.set_field_array(get_field_by_symbol("sfRawTransactions"), raw_transactions);
    });

    let mut batch_message = Serializer::default();
    batch_message.add32_prefix(HashPrefix::Batch);
    batch_message.add_bit_string(outer_account);
    batch_message.add32(batch.get_seq_value());
    batch_message.add32(batch.get_flags());
    batch_message.add32(expected_ids.len() as u32);
    for tx_id in &expected_ids {
        batch_message.add_bit_string(*tx_id);
    }
    batch_message.add_bit_string(batch_signer_account);
    batch_message.add_bit_string(multisigner_account);

    let signature = sign(
        &multisigner_public,
        &multisigner_secret,
        batch_message.data(),
    )
    .expect("signature");
    let mut inner_signer = STObject::make_inner_object(get_field_by_symbol("sfSigner"));
    inner_signer.set_account_id(get_field_by_symbol("sfAccount"), multisigner_account);
    inner_signer.set_field_vl(
        get_field_by_symbol("sfSigningPubKey"),
        multisigner_public.as_bytes(),
    );
    inner_signer.set_field_vl(get_field_by_symbol("sfTxnSignature"), &signature);
    let mut signers = STArray::new(get_field_by_symbol("sfSigners"));
    signers.push_back(inner_signer);

    let mut batch_signer = STObject::make_inner_object(get_field_by_symbol("sfBatchSigner"));
    batch_signer.set_account_id(get_field_by_symbol("sfAccount"), batch_signer_account);
    batch_signer.set_field_vl(get_field_by_symbol("sfSigningPubKey"), &[]);
    batch_signer.set_field_array(get_field_by_symbol("sfSigners"), signers);
    let mut batch_signers = STArray::new(get_field_by_symbol("sfBatchSigners"));
    batch_signers.push_back(batch_signer);

    let mut batch = batch;
    batch.set_field_array(get_field_by_symbol("sfBatchSigners"), batch_signers);
    assert_eq!(batch.check_batch_sign(&Rules::default()), Ok(()));
}

#[test]
fn protocol_sttx_local_checks_reject_invalid_raw_transaction_owners_and_types() {
    let (raw_transactions, _) = raw_transaction_array();
    let mut non_batch = STObject::new(get_field_by_symbol("sfTransaction"));
    non_batch.emplace_back(STVar::new(STUInt16::with_field(
        get_field_by_symbol("sfTransactionType"),
        TxType::PAYMENT.into(),
    )));
    non_batch.emplace_back(STVar::new(raw_transactions));
    assert_eq!(
        passes_local_checks(&non_batch),
        Err("Only Batch transactions may contain raw transactions.".to_owned())
    );

    let mut unknown_raw = STObject::new(get_field_by_symbol("sfRawTransaction"));
    unknown_raw.set_field_u16(get_field_by_symbol("sfTransactionType"), u16::MAX);
    let mut raw_transactions = STArray::new(get_field_by_symbol("sfRawTransactions"));
    raw_transactions.push_back(unknown_raw);
    let batch = STTx::new(TxType::BATCH, |tx| {
        tx.set_account_id(get_field_by_symbol("sfAccount"), account(0x55));
        tx.set_field_amount(
            get_field_by_symbol("sfFee"),
            STAmount::new_native(10, false),
        );
        tx.set_field_u32(get_field_by_symbol("sfSequence"), 1);
        tx.set_field_array(get_field_by_symbol("sfRawTransactions"), raw_transactions);
    });
    assert_eq!(
        passes_local_checks(&batch),
        Err("Invalid raw transaction type.".to_owned())
    );
}

#[test]
fn protocol_sttx_local_checks_reject_invalid_raw_inner_transactions() {
    let batch_with_raw = |raw: STObject| {
        let mut raw_transactions = STArray::new(get_field_by_symbol("sfRawTransactions"));
        raw_transactions.push_back(raw);
        STTx::new(TxType::BATCH, |tx| {
            tx.set_account_id(get_field_by_symbol("sfAccount"), account(0x56));
            tx.set_field_amount(
                get_field_by_symbol("sfFee"),
                STAmount::new_native(10, false),
            );
            tx.set_field_u32(get_field_by_symbol("sfSequence"), 1);
            tx.set_field_array(get_field_by_symbol("sfRawTransactions"), raw_transactions);
        })
    };

    let mut invalid_memo = payment_tx(16).clone_as_object();
    invalid_memo.set_fname(get_field_by_symbol("sfRawTransaction"));
    let mut memo = STObject::make_inner_object(get_field_by_symbol("sfMemo"));
    memo.set_field_vl(get_field_by_symbol("sfMemoType"), &[0x00]);
    let mut memos = STArray::new(get_field_by_symbol("sfMemos"));
    memos.push_back(memo);
    invalid_memo.set_field_array(get_field_by_symbol("sfMemos"), memos);
    assert_eq!(
        passes_local_checks(&batch_with_raw(invalid_memo)),
        Err(
            "The MemoType and MemoFormat fields may only contain characters that are allowed in URLs under RFC 3986."
                .to_owned()
        )
    );

    let mut invalid_account = payment_tx(17).clone_as_object();
    invalid_account.set_fname(get_field_by_symbol("sfRawTransaction"));
    invalid_account.make_field_present(get_field_by_symbol("sfDelegate"));
    assert_eq!(
        passes_local_checks(&batch_with_raw(invalid_account)),
        Err("An account field is invalid.".to_owned())
    );

    let mut pseudo = STObject::new(get_field_by_symbol("sfRawTransaction"));
    pseudo.set_field_u16(
        get_field_by_symbol("sfTransactionType"),
        TxType::AMENDMENT.into(),
    );
    pseudo.set_account_id(get_field_by_symbol("sfAccount"), account(0x57));
    assert_eq!(
        passes_local_checks(&batch_with_raw(pseudo)),
        Err("Cannot submit pseudo transactions.".to_owned())
    );
}

#[test]
fn protocol_sttx_batch_signature_checks_reject_preconditions_before_hashing_or_verifying() {
    let payment = payment_tx(12);
    assert_eq!(
        payment.check_batch_sign(&Rules::default()),
        Err("Not a batch transaction.".to_owned())
    );

    let mut missing_inner_transactions = STTx::new(TxType::BATCH, |tx| {
        tx.set_account_id(get_field_by_symbol("sfAccount"), account(0x41));
        tx.set_field_amount(
            get_field_by_symbol("sfFee"),
            STAmount::new_native(10, false),
        );
        tx.set_field_u32(get_field_by_symbol("sfSequence"), 1);
    });
    missing_inner_transactions.set_field_array(
        get_field_by_symbol("sfBatchSigners"),
        STArray::new(get_field_by_symbol("sfBatchSigners")),
    );
    assert_eq!(
        missing_inner_transactions.check_batch_sign(&Rules::default()),
        Err("Missing inner transactions.".to_owned())
    );

    let (raw_transactions, _) = raw_transaction_array();
    let mut oversized_signers = STArray::new(get_field_by_symbol("sfBatchSigners"));
    for _ in 0..=protocol::MAX_BATCH_SIGNER_COUNT {
        oversized_signers.push_back(STObject::make_inner_object(get_field_by_symbol(
            "sfBatchSigner",
        )));
    }
    let oversized = STTx::new(TxType::BATCH, |tx| {
        tx.set_account_id(get_field_by_symbol("sfAccount"), account(0x42));
        tx.set_field_amount(
            get_field_by_symbol("sfFee"),
            STAmount::new_native(10, false),
        );
        tx.set_field_u32(get_field_by_symbol("sfSequence"), 1);
        tx.set_field_array(get_field_by_symbol("sfRawTransactions"), raw_transactions);
        tx.set_field_array(get_field_by_symbol("sfBatchSigners"), oversized_signers);
    });
    assert_eq!(protocol::MAX_BATCH_SIGNER_COUNT, 24);
    assert_eq!(
        oversized.check_batch_sign(&Rules::default()),
        Err("BatchSigners array exceeds max entries.".to_owned())
    );

    let mut nested_raw = STObject::new(get_field_by_symbol("sfRawTransaction"));
    nested_raw.set_field_u16(
        get_field_by_symbol("sfTransactionType"),
        TxType::BATCH.into(),
    );
    let mut nested_raw_transactions = STArray::new(get_field_by_symbol("sfRawTransactions"));
    nested_raw_transactions.push_back(nested_raw);
    let nested = STTx::new(TxType::BATCH, |tx| {
        tx.set_account_id(get_field_by_symbol("sfAccount"), account(0x43));
        tx.set_field_amount(
            get_field_by_symbol("sfFee"),
            STAmount::new_native(10, false),
        );
        tx.set_field_u32(get_field_by_symbol("sfSequence"), 1);
        tx.set_field_array(
            get_field_by_symbol("sfRawTransactions"),
            nested_raw_transactions,
        );
        tx.set_field_array(
            get_field_by_symbol("sfBatchSigners"),
            STArray::new(get_field_by_symbol("sfBatchSigners")),
        );
    });
    assert_eq!(
        nested.check_batch_sign(&Rules::default()),
        Err("Batch inner transaction cannot be a Batch.".to_owned())
    );

    let mut oversized_raw_transactions = STArray::new(get_field_by_symbol("sfRawTransactions"));
    for sequence in 1..=u32::try_from(protocol::MAX_BATCH_TX_COUNT + 1).expect("count fits u32") {
        let mut raw = payment_tx(sequence).clone_as_object();
        raw.set_fname(get_field_by_symbol("sfRawTransaction"));
        oversized_raw_transactions.push_back(raw);
    }
    let oversized_raw = STTx::new(TxType::BATCH, |tx| {
        tx.set_account_id(get_field_by_symbol("sfAccount"), account(0x44));
        tx.set_field_amount(
            get_field_by_symbol("sfFee"),
            STAmount::new_native(10, false),
        );
        tx.set_field_u32(get_field_by_symbol("sfSequence"), 1);
        tx.set_field_array(
            get_field_by_symbol("sfRawTransactions"),
            oversized_raw_transactions,
        );
        tx.set_field_array(
            get_field_by_symbol("sfBatchSigners"),
            STArray::new(get_field_by_symbol("sfBatchSigners")),
        );
    });
    assert_eq!(
        oversized_raw.check_batch_sign(&Rules::default()),
        Err("Raw Transactions array exceeds max entries.".to_owned())
    );
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
fn protocol_sttx_batch_ids_recompute_canonically_after_raw_transaction_changes() {
    let (raw_transactions, mut expected_ids) = raw_transaction_array();
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

    let extra_tx = payment_tx(5);
    let mut extra = extra_tx.clone_as_object();
    extra.set_fname(get_field_by_symbol("sfRawTransaction"));
    expected_ids.push(extra_tx.get_transaction_id());
    batch
        .peek_field_array(get_field_by_symbol("sfRawTransactions"))
        .push_back(extra);

    assert_eq!(batch.get_batch_transaction_ids(), expected_ids);
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

#[test]
fn protocol_role_signature_domains_activate_with_fix_cleanup_3_4_0() {
    let legacy = Rules::default();
    let fixed = Rules::new([fix_cleanup_3_4_0()]);

    assert_eq!(HashPrefix::CounterpartyTxSign.as_u32(), 0x4350_5400);
    assert_eq!(HashPrefix::CounterpartyTxMultiSign.as_u32(), 0x4350_4D00);
    assert_eq!(HashPrefix::SponsorTxSign.as_u32(), 0x5350_4E00);
    assert_eq!(HashPrefix::SponsorTxMultiSign.as_u32(), 0x5350_4D00);
    for role in [
        SignatureRole::Transaction,
        SignatureRole::Counterparty,
        SignatureRole::Sponsor,
    ] {
        assert_eq!(signing_prefix(role, false, &legacy), HashPrefix::TxSign);
        assert_eq!(signing_prefix(role, true, &legacy), HashPrefix::TxMultiSign);
    }
    assert_eq!(
        signing_prefix(SignatureRole::Transaction, false, &fixed),
        HashPrefix::TxSign
    );
    assert_eq!(
        signing_prefix(SignatureRole::Transaction, true, &fixed),
        HashPrefix::TxMultiSign
    );
    assert_eq!(
        signing_prefix(SignatureRole::Counterparty, false, &fixed),
        HashPrefix::CounterpartyTxSign
    );
    assert_eq!(
        signing_prefix(SignatureRole::Counterparty, true, &fixed),
        HashPrefix::CounterpartyTxMultiSign
    );
    assert_eq!(
        signing_prefix(SignatureRole::Sponsor, false, &fixed),
        HashPrefix::SponsorTxSign
    );
    assert_eq!(
        signing_prefix(SignatureRole::Sponsor, true, &fixed),
        HashPrefix::SponsorTxMultiSign
    );
}

#[test]
fn protocol_role_signatures_are_cryptographically_partitioned_by_cleanup_3_4_0() {
    let legacy = Rules::default();
    let fixed = Rules::new([fix_cleanup_3_4_0()]);
    let payer_secret = SecretKey::from_bytes([0x71; 32]);
    let payer_public = derive_public_key(KeyType::Secp256k1, &payer_secret).expect("payer key");
    let role_secret = SecretKey::from_bytes([0x72; 32]);
    let role_public = derive_public_key(KeyType::Secp256k1, &role_secret).expect("role key");
    let payer = calc_account_id(payer_public.as_bytes());
    let role_account = calc_account_id(role_public.as_bytes());

    let make_loan = |sequence| {
        let mut tx = LoanSetBuilder::new(
            payer,
            Uint256::from_array([0xC1; 32]),
            STNumber::from_json_input(NumberJsonInput::UInt(100)).expect("number"),
            Some(sequence),
            Some(STAmount::new_native(10, false)),
        )
        .set_counterparty(role_account)
        .build(&payer_public, &payer_secret)
        .expect("loan set")
        .tx()
        .as_ref()
        .clone();
        tx.set_account_id(get_field_by_symbol("sfSponsor"), role_account);
        tx.sign_with_role(
            &payer_public,
            &payer_secret,
            SignatureRole::Transaction,
            &legacy,
        )
        .expect("primary signature after sponsor assignment");
        tx
    };
    let add_single = |tx: &mut STTx, role, rules: &Rules| {
        role_target(tx, role).set_field_vl(
            get_field_by_symbol("sfSigningPubKey"),
            role_public.as_bytes(),
        );
        tx.sign_with_role(&role_public, &role_secret, role, rules)
            .expect("role single signature");
    };
    let add_multi = |tx: &mut STTx, role, rules: &Rules| {
        let message = build_multi_signing_data_with_prefix(
            &tx.clone_as_object(),
            role_account,
            signing_prefix(role, true, rules),
        );
        let signature = sign(&role_public, &role_secret, message.data()).expect("multi signature");
        let mut signer = STObject::make_inner_object(get_field_by_symbol("sfSigner"));
        signer.set_account_id(get_field_by_symbol("sfAccount"), role_account);
        signer.set_field_vl(
            get_field_by_symbol("sfSigningPubKey"),
            role_public.as_bytes(),
        );
        signer.set_field_vl(get_field_by_symbol("sfTxnSignature"), &signature);
        let mut signers = STArray::new(get_field_by_symbol("sfSigners"));
        signers.push_back(signer);
        let target = role_target(tx, role);
        target.set_field_vl(get_field_by_symbol("sfSigningPubKey"), &[]);
        target.set_field_array(get_field_by_symbol("sfSigners"), signers);
    };

    for (offset, role) in [SignatureRole::Counterparty, SignatureRole::Sponsor]
        .into_iter()
        .enumerate()
    {
        let mut legacy_single = make_loan(71 + offset as u32);
        add_single(&mut legacy_single, role, &legacy);
        assert_eq!(legacy_single.check_sign(&legacy), Ok(()));
        assert!(legacy_single.check_sign(&fixed).is_err());

        let mut fixed_single = make_loan(81 + offset as u32);
        add_single(&mut fixed_single, role, &fixed);
        assert_eq!(fixed_single.check_sign(&fixed), Ok(()));
        assert!(fixed_single.check_sign(&legacy).is_err());

        let mut legacy_multi = make_loan(91 + offset as u32);
        add_multi(&mut legacy_multi, role, &legacy);
        assert_eq!(legacy_multi.check_sign(&legacy), Ok(()));
        assert!(legacy_multi.check_sign(&fixed).is_err());

        let mut fixed_multi = make_loan(101 + offset as u32);
        add_multi(&mut fixed_multi, role, &fixed);
        assert_eq!(fixed_multi.check_sign(&fixed), Ok(()));
        assert!(fixed_multi.check_sign(&legacy).is_err());
    }

    let mut counterparty_single = make_loan(111);
    add_single(
        &mut counterparty_single,
        SignatureRole::Counterparty,
        &fixed,
    );
    let counterparty_signature = role_target(&mut counterparty_single, SignatureRole::Counterparty)
        .get_field_vl(get_field_by_symbol("sfTxnSignature"));
    let sponsor_target = role_target(&mut counterparty_single, SignatureRole::Sponsor);
    sponsor_target.set_field_vl(
        get_field_by_symbol("sfSigningPubKey"),
        role_public.as_bytes(),
    );
    sponsor_target.set_field_vl(
        get_field_by_symbol("sfTxnSignature"),
        &counterparty_signature,
    );
    assert_eq!(
        counterparty_single.check_sign(&fixed),
        Err("Sponsor: Invalid signature.".to_owned())
    );

    let mut sponsor_single = make_loan(112);
    add_single(&mut sponsor_single, SignatureRole::Sponsor, &fixed);
    let sponsor_signature = role_target(&mut sponsor_single, SignatureRole::Sponsor)
        .get_field_vl(get_field_by_symbol("sfTxnSignature"));
    let counterparty_target = role_target(&mut sponsor_single, SignatureRole::Counterparty);
    counterparty_target.set_field_vl(
        get_field_by_symbol("sfSigningPubKey"),
        role_public.as_bytes(),
    );
    counterparty_target.set_field_vl(get_field_by_symbol("sfTxnSignature"), &sponsor_signature);
    assert_eq!(
        sponsor_single.check_sign(&fixed),
        Err("Counterparty: Invalid signature.".to_owned())
    );

    let mut counterparty_multi = make_loan(113);
    add_multi(&mut counterparty_multi, SignatureRole::Counterparty, &fixed);
    let counterparty_signers = role_target(&mut counterparty_multi, SignatureRole::Counterparty)
        .get_field_array(get_field_by_symbol("sfSigners"))
        .clone();
    let sponsor_target = role_target(&mut counterparty_multi, SignatureRole::Sponsor);
    sponsor_target.set_field_vl(get_field_by_symbol("sfSigningPubKey"), &[]);
    sponsor_target.set_field_array(get_field_by_symbol("sfSigners"), counterparty_signers);
    assert_eq!(
        counterparty_multi.check_sign(&fixed),
        Err(format!(
            "Sponsor: Invalid signature on account {}.",
            protocol::to_base58(role_account)
        ))
    );

    let mut sponsor_multi = make_loan(114);
    add_multi(&mut sponsor_multi, SignatureRole::Sponsor, &fixed);
    let sponsor_signers = role_target(&mut sponsor_multi, SignatureRole::Sponsor)
        .get_field_array(get_field_by_symbol("sfSigners"))
        .clone();
    let counterparty_target = role_target(&mut sponsor_multi, SignatureRole::Counterparty);
    counterparty_target.set_field_vl(get_field_by_symbol("sfSigningPubKey"), &[]);
    counterparty_target.set_field_array(get_field_by_symbol("sfSigners"), sponsor_signers);
    assert_eq!(
        sponsor_multi.check_sign(&fixed),
        Err(format!(
            "Counterparty: Invalid signature on account {}.",
            protocol::to_base58(role_account)
        ))
    );
}

fn role_target(tx: &mut STTx, role: SignatureRole) -> &mut STObject {
    tx.peek_field_object(
        protocol::signature_field(role).expect("alternate role has a signature target"),
    )
}
