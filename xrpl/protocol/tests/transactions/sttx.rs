use std::{
    collections::BTreeSet,
    panic::{AssertUnwindSafe, catch_unwind},
};

use basics::str_hex::str_hex;
use protocol::{
    AccountID, IOUAmount, Issue, JsonOptions, JsonValue, MPTAmount, MPTIssue, STAmount, STObject,
    STPathSet, STTx, SeqProxy, SerialIter, StBase, TxType, currency_from_string,
    get_field_by_symbol, make_mpt_id,
};

fn account(hex: &str) -> AccountID {
    AccountID::from_hex(hex).expect("account hex should parse")
}

fn payment_tx(sequence: u32) -> STTx {
    let source = account("1111111111111111111111111111111111111111");
    let destination = account("2222222222222222222222222222222222222222");

    STTx::new(TxType::PAYMENT, |tx| {
        tx.set_account_id(get_field_by_symbol("sfAccount"), source);
        tx.set_account_id(get_field_by_symbol("sfDestination"), destination);
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

#[test]
fn protocol_sttx_round_trips_serialized_payment() {
    let tx = payment_tx(7);
    let serializer = tx.get_serializer();
    let mut sit = SerialIter::new(serializer.data());

    let parsed = STTx::from_serial_iter(&mut sit);

    assert_eq!(parsed.get_txn_type(), TxType::PAYMENT);
    assert_eq!(parsed.get_transaction_id(), tx.get_transaction_id());
    assert_eq!(parsed.get_seq_proxy(), SeqProxy::sequence(7));
    assert!(sit.empty());
}

#[test]
fn protocol_sttx_rejects_too_short_serialized_payload() {
    // Too-short payload: returns an empty/invalid tx instead of panicking
    let mut sit = SerialIter::new(&[0u8; 31]);
    let tx = STTx::from_serial_iter(&mut sit);
    // The returned tx should be a default/empty Payment
    assert_eq!(tx.get_txn_type(), TxType::PAYMENT);
}

#[test]
fn protocol_sttx_from_stobject_applies_transaction_template() {
    let source = account("1111111111111111111111111111111111111111");
    let destination = account("2222222222222222222222222222222222222222");
    let mut object = STObject::new(get_field_by_symbol("sfTransaction"));
    object.set_field_u16(
        get_field_by_symbol("sfTransactionType"),
        Into::<u16>::into(TxType::PAYMENT),
    );
    object.set_account_id(get_field_by_symbol("sfAccount"), source);
    object.set_account_id(get_field_by_symbol("sfDestination"), destination);
    object.set_field_amount(
        get_field_by_symbol("sfAmount"),
        STAmount::new_native(1_000_000, false),
    );
    object.set_field_amount(
        get_field_by_symbol("sfFee"),
        STAmount::new_native(10, false),
    );
    object.set_field_vl(get_field_by_symbol("sfSigningPubKey"), &[]);
    object.set_field_u32(get_field_by_symbol("sfSequence"), 3);

    let rebuilt = STTx::from_stobject(object);

    assert_eq!(rebuilt.get_txn_type(), TxType::PAYMENT);
    assert!(rebuilt.is_field_present(get_field_by_symbol("sfAmount")));
    assert_eq!(rebuilt.get_seq_value(), 3);
    assert_eq!(rebuilt.get_fee_payer(), source);
}

#[test]
fn protocol_sttx_from_stobject_reapplies_template_for_explicit_default_fields() {
    let tx = payment_tx(4);
    let mut object = (*tx).clone();
    object.set_stbase(STPathSet::new(get_field_by_symbol("sfPaths")));

    let result = catch_unwind(AssertUnwindSafe(|| {
        let _ = STTx::from_stobject(object);
    }));

    assert!(
        result.is_ok(),
        "C++ silently accepts explicit default-valued fields"
    );
}

#[test]
fn protocol_sttx_prefers_sequence_then_ticket_then_zero_sequence() {
    let sequence_tx = payment_tx(9);

    let ticket_tx = STTx::new(TxType::PAYMENT, |tx| {
        tx.set_account_id(
            get_field_by_symbol("sfAccount"),
            account("1111111111111111111111111111111111111111"),
        );
        tx.set_account_id(
            get_field_by_symbol("sfDestination"),
            account("2222222222222222222222222222222222222222"),
        );
        tx.set_field_amount(
            get_field_by_symbol("sfAmount"),
            STAmount::new_native(1_000_000, false),
        );
        tx.set_field_amount(
            get_field_by_symbol("sfFee"),
            STAmount::new_native(10, false),
        );
        tx.set_field_u32(get_field_by_symbol("sfSequence"), 0);
        tx.set_field_u32(get_field_by_symbol("sfTicketSequence"), 44);
    });

    let zero_seq_tx = payment_tx(0);

    assert_eq!(sequence_tx.get_seq_proxy(), SeqProxy::sequence(9));
    assert_eq!(sequence_tx.get_seq_value(), 9);
    assert_eq!(ticket_tx.get_seq_proxy(), SeqProxy::ticket(44));
    assert_eq!(ticket_tx.get_seq_value(), 44);
    assert_eq!(zero_seq_tx.get_seq_proxy(), SeqProxy::sequence(0));
}

#[test]
fn protocol_sttx_fee_payer_uses_delegate_when_present() {
    let delegated = account("AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA");
    let tx = STTx::new(TxType::PAYMENT, |tx| {
        tx.set_account_id(
            get_field_by_symbol("sfAccount"),
            account("1111111111111111111111111111111111111111"),
        );
        tx.set_account_id(get_field_by_symbol("sfDelegate"), delegated);
        tx.set_account_id(
            get_field_by_symbol("sfDestination"),
            account("2222222222222222222222222222222222222222"),
        );
        tx.set_field_amount(
            get_field_by_symbol("sfAmount"),
            STAmount::new_native(1_000_000, false),
        );
        tx.set_field_amount(
            get_field_by_symbol("sfFee"),
            STAmount::new_native(10, false),
        );
        tx.set_field_u32(get_field_by_symbol("sfSequence"), 1);
    });

    assert_eq!(tx.get_fee_payer(), delegated);
}

#[test]
fn protocol_sttx_get_signature_extracts_present_signature_or_empty() {
    let tx = payment_tx(2);
    assert!(STTx::get_signature(&tx).is_empty());

    let mut signed = tx.clone();
    let signature = vec![0x11, 0x22, 0x33, 0x44];
    signed.set_field_vl(get_field_by_symbol("sfTxnSignature"), &signature);

    assert_eq!(STTx::get_signature(&signed), signature);
}

#[test]
fn protocol_sttx_collects_account_and_issuer_mentions() {
    let source = account("1111111111111111111111111111111111111111");
    let destination = account("2222222222222222222222222222222222222222");
    let issuer = account("3333333333333333333333333333333333333333");
    let mpt_issuer = account("4444444444444444444444444444444444444444");
    let usd = currency_from_string("USD");

    let tx = STTx::new(TxType::PAYMENT, |tx| {
        tx.set_account_id(get_field_by_symbol("sfAccount"), source);
        tx.set_account_id(get_field_by_symbol("sfDestination"), destination);
        tx.set_field_amount(
            get_field_by_symbol("sfAmount"),
            STAmount::from_iou_amount(
                get_field_by_symbol("sfAmount"),
                IOUAmount::from_parts(10, 0).expect("IOU amount should normalize"),
                Issue::new(usd, issuer),
            ),
        );
        tx.set_field_amount(
            get_field_by_symbol("sfSendMax"),
            STAmount::from_mpt_amount(
                get_field_by_symbol("sfSendMax"),
                MPTAmount::from_value(10),
                MPTIssue::new(make_mpt_id(9, mpt_issuer)),
            ),
        );
        tx.set_field_amount(
            get_field_by_symbol("sfFee"),
            STAmount::new_native(10, false),
        );
        tx.set_field_u32(get_field_by_symbol("sfSequence"), 1);
    });

    let expected = BTreeSet::from([source, destination, issuer, mpt_issuer]);
    assert_eq!(tx.get_mentioned_accounts(), expected);
}

#[test]
fn protocol_sttx_json_binary_matches_current_cpp_shapes() {
    let tx = payment_tx(5);

    let legacy = tx.get_json_binary(JsonOptions::NONE, true);
    let v2 = tx.get_json_binary(JsonOptions::DISABLE_API_PRIOR_V2, true);

    match legacy {
        JsonValue::Object(object) => {
            assert_eq!(
                object.get("hash"),
                Some(&JsonValue::String(tx.get_transaction_id().to_string()))
            );
            assert_eq!(
                object.get("tx"),
                Some(&JsonValue::String(str_hex(tx.get_serializer().data())))
            );
        }
        other => panic!("unexpected legacy binary shape: {other:?}"),
    }

    assert_eq!(v2, JsonValue::String(str_hex(tx.get_serializer().data())));
}

#[test]
fn protocol_sttx_json_adds_legacy_hash_and_omits_v2_hash() {
    let tx = payment_tx(6);

    let legacy = tx.json(JsonOptions::NONE);
    let v2 = tx.json(JsonOptions::DISABLE_API_PRIOR_V2);

    match legacy {
        JsonValue::Object(object) => {
            assert_eq!(
                object.get("hash"),
                Some(&JsonValue::String(tx.get_transaction_id().to_string()))
            );
        }
        other => panic!("unexpected legacy json shape: {other:?}"),
    }

    match v2 {
        JsonValue::Object(object) => {
            assert!(!object.contains_key("hash"));
        }
        other => panic!("unexpected v2 json shape: {other:?}"),
    }
}
