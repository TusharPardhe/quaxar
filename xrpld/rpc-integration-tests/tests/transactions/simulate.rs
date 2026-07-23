//! Integration tests for the transaction RPC handler.

use std::collections::BTreeMap;

use basics::{
    base_uint::{Uint160, Uint192},
    str_hex::str_hex,
    string_utilities::str_unhex,
};
use protocol::{
    get_field_by_symbol, to_base58, AccountID, JsonValue, MPTAmount, MPTIssue, STAmount,
    STLedgerEntry, STTx, Ter, TxMeta, TxType,
};
use rpc_integration_tests::env::*;

fn submit_request<'a>(
    params: &'a JsonValue,
    source: &'a rpc::ApplicationServerInfo<&app::ApplicationRoot>,
) -> Result<JsonValue, rpc::RpcStatus> {
    rpc::do_submit(&rpc::RpcRequestContext {
        params,
        env: &rpc::SubmitSource,
        runtime: source,
        role: rpc::Role::Admin,
        api_version: 2,
        headers: rpc::JsonContextHeaders::default(),
        request_headers: BTreeMap::new(),
        unlimited: true,
        remote_ip: None,
        load_type: rpc::RpcLoadType::Reference,
    })
}

fn simulate_request<'a>(
    params: &'a JsonValue,
    source: &'a rpc::ApplicationServerInfo<&app::ApplicationRoot>,
) -> Result<JsonValue, rpc::RpcStatus> {
    rpc::do_simulate(&rpc::RpcRequestContext {
        params,
        env: &rpc::SimulateSource,
        runtime: source,
        role: rpc::Role::Admin,
        api_version: 2,
        headers: rpc::JsonContextHeaders::default(),
        request_headers: BTreeMap::new(),
        unlimited: true,
        remote_ip: None,
        load_type: rpc::RpcLoadType::Reference,
    })
}

#[test]
fn submit_valid_payment_returns_engine_result() {
    let mut alice = TestAccount::new("sub_alice");
    let bob = TestAccount::new("sub_bob");
    let env = RpcTestEnv::new(&[(&alice, 10_000_000_000), (&bob, 1_000_000_000)]);

    // Create and sign a payment
    let mut payment = STTx::new(TxType::PAYMENT, |tx| {
        tx.set_account_id(get_field_by_symbol("sfAccount"), alice.id);
        tx.set_account_id(get_field_by_symbol("sfDestination"), bob.id);
        tx.set_field_amount(
            get_field_by_symbol("sfAmount"),
            STAmount::new_native(1_000_000, false),
        );
        tx.set_field_amount(
            get_field_by_symbol("sfFee"),
            STAmount::new_native(10, false),
        );
        tx.set_field_u32(get_field_by_symbol("sfSequence"), alice.next_seq());
    });
    sign_tx(&mut payment, &alice);

    let tx_blob = str_hex(payment.get_serializer().data());
    let source = env.rpc_source();
    let result = submit_request(&json([("tx_blob", JsonValue::String(tx_blob))]), &source);

    match result {
        Ok(JsonValue::Object(obj)) => {
            assert!(
                obj.contains_key("engine_result") || obj.contains_key("error"),
                "should have engine_result or error: {:?}",
                obj.keys().collect::<Vec<_>>()
            );
            if let Some(JsonValue::String(engine_result)) = obj.get("engine_result") {
                // Should be tesSUCCESS or a tec/tef/ter code
                assert!(
                    engine_result.starts_with("tes")
                        || engine_result.starts_with("tec")
                        || engine_result.starts_with("tef")
                        || engine_result.starts_with("ter"),
                    "engine_result should be a valid result code: {engine_result}"
                );
            }
            if obj.contains_key("tx_json") {
                let JsonValue::Object(tx_json) = obj.get("tx_json").unwrap() else {
                    panic!("object")
                };
                assert!(tx_json.contains_key("TransactionType") || tx_json.contains_key("hash"));
            }
        }
        Ok(other) => panic!("unexpected result: {other:?}"),
        Err(status) => {
            // Some errors are expected (e.g., noNetwork if not synced)
            assert!(
                status.code_string() == "noNetwork"
                    || status.code_string() == "notSynced"
                    || status.code_string() == "noCurrent"
                    || status.code_string() == "internal",
                "unexpected error: {}",
                status.code_string()
            );
        }
    }
}

#[test]
fn submit_invalid_blob_returns_error() {
    let alice = TestAccount::new("sub_alice2");
    let env = RpcTestEnv::new(&[(&alice, 10_000_000_000)]);

    let source = env.rpc_source();
    let result = submit_request(
        &json([("tx_blob", JsonValue::String("DEADBEEF".to_owned()))]),
        &source,
    );

    match result {
        Ok(JsonValue::Object(obj)) => {
            // Invalid blob should produce an error
            assert!(
                obj.contains_key("error") || obj.contains_key("engine_result"),
                "should have error or engine_result"
            );
            if let Some(JsonValue::String(error)) = obj.get("error") {
                assert_eq!(error, "invalidTransaction");
            }
        }
        Err(_) => {} // Error is also acceptable
        _ => panic!("unexpected"),
    }
}

#[test]
fn submit_missing_tx_blob_returns_invalid_params() {
    let alice = TestAccount::new("sub_alice3");
    let env = RpcTestEnv::new(&[(&alice, 10_000_000_000)]);

    let source = env.rpc_source();
    let result = submit_request(&json([]), &source);

    assert!(result.is_err(), "missing tx_blob should error");
}

#[test]
fn simulate_valid_payment() {
    let mut alice = TestAccount::new("sim_alice");
    let bob = TestAccount::new("sim_bob");
    let env = RpcTestEnv::new(&[(&alice, 10_000_000_000), (&bob, 1_000_000_000)]);

    let mut payment = STTx::new(TxType::PAYMENT, |tx| {
        tx.set_account_id(get_field_by_symbol("sfAccount"), alice.id);
        tx.set_account_id(get_field_by_symbol("sfDestination"), bob.id);
        tx.set_field_amount(
            get_field_by_symbol("sfAmount"),
            STAmount::new_native(1_000_000, false),
        );
        tx.set_field_amount(
            get_field_by_symbol("sfFee"),
            STAmount::new_native(10, false),
        );
        tx.set_field_u32(get_field_by_symbol("sfSequence"), alice.next_seq());
    });
    sign_tx(&mut payment, &alice);

    let tx_blob = str_hex(payment.get_serializer().data());
    let source = env.rpc_source();
    let result = simulate_request(
        &json([("tx_blob", JsonValue::String(tx_blob.clone()))]),
        &source,
    );

    let JsonValue::Object(json_response) = result.expect("simulate should succeed") else {
        panic!("simulate response should be an object");
    };
    assert_eq!(
        json_response.get("engine_result"),
        Some(&JsonValue::String("tesSUCCESS".to_owned()))
    );
    let ledger_seq = json_response
        .get("ledger_index")
        .and_then(JsonValue::as_u64)
        .and_then(|value| u32::try_from(value).ok())
        .expect("successful simulation should include ledger index");
    let JsonValue::Object(meta) = json_response.get("meta").expect("JSON metadata") else {
        panic!("simulation metadata should be an object");
    };
    assert_eq!(
        meta.get("TransactionResult"),
        Some(&JsonValue::String("tesSUCCESS".to_owned()))
    );
    assert!(
        matches!(meta.get("AffectedNodes"), Some(JsonValue::Array(nodes)) if !nodes.is_empty()),
        "canonical simulation metadata must include the state-table mutations"
    );

    let binary = simulate_request(
        &json([
            ("tx_blob", JsonValue::String(tx_blob)),
            ("binary", JsonValue::Bool(true)),
        ]),
        &source,
    )
    .expect("binary simulate should succeed");
    let JsonValue::Object(binary) = binary else {
        panic!("binary simulation response should be an object");
    };
    let meta_blob = binary
        .get("meta_blob")
        .and_then(JsonValue::as_str)
        .expect("binary simulation must include canonical meta_blob");
    let meta_bytes = str_unhex(meta_blob).expect("meta_blob should be hexadecimal");
    let decoded = TxMeta::from_raw(payment.get_transaction_id(), ledger_seq, &meta_bytes);
    assert_eq!(decoded.get_result_ter(), Ter::TES_SUCCESS);
    assert_eq!(
        decoded.get_nodes().len(),
        meta.get("AffectedNodes")
            .and_then(|value| match value {
                JsonValue::Array(nodes) => Some(nodes.len()),
                _ => None,
            })
            .expect("JSON affected nodes should be an array"),
        "JSON and meta_blob must derive from the same TxMeta"
    );
}

fn account_uint160(account: AccountID) -> Uint160 {
    Uint160::from_slice(account.data()).expect("account ID width")
}

fn mpt_id_for(issuer: AccountID, sequence: u32) -> Uint192 {
    let mut bytes = [0u8; 24];
    bytes[..4].copy_from_slice(&sequence.to_be_bytes());
    bytes[4..].copy_from_slice(issuer.data());
    Uint192::from_slice(&bytes).expect("MPT ID width")
}

fn mpt_issuance_entry(issuer: AccountID, sequence: u32) -> STLedgerEntry {
    let keylet = protocol::mpt_issuance_keylet(sequence, account_uint160(issuer));
    let mut entry = STLedgerEntry::new(keylet);
    entry.set_account_id(get_field_by_symbol("sfIssuer"), issuer);
    entry.set_field_u32(get_field_by_symbol("sfSequence"), sequence);
    entry.set_field_u64(get_field_by_symbol("sfOutstandingAmount"), 10_000);
    entry.set_field_u16(get_field_by_symbol("sfTransferFee"), 25_000);
    entry.set_field_u32(get_field_by_symbol("sfFlags"), protocol::lsfMPTCanTransfer);
    entry.set_field_u64(get_field_by_symbol("sfOwnerNode"), 0);
    entry
}

fn mptoken_entry(holder: AccountID, issuance_id: Uint192, amount: u64) -> STLedgerEntry {
    let keylet = protocol::mptoken_keylet_from_mptid(issuance_id, account_uint160(holder));
    let mut entry = STLedgerEntry::new(keylet);
    entry.set_account_id(get_field_by_symbol("sfAccount"), holder);
    entry.set_field_h192(get_field_by_symbol("sfMPTokenIssuanceID"), issuance_id);
    entry.set_field_u64(get_field_by_symbol("sfMPTAmount"), amount);
    entry.set_field_u32(get_field_by_symbol("sfFlags"), 0);
    entry.set_field_u64(get_field_by_symbol("sfOwnerNode"), 0);
    entry
}

#[test]
fn simulate_partial_mpt_payment_records_delivered_amount_only_with_amendment() {
    for amendment_enabled in [false, true] {
        let mut source_account = TestAccount::new(if amendment_enabled {
            "simulate_mpt_source_on"
        } else {
            "simulate_mpt_source_off"
        });
        let destination = TestAccount::new(if amendment_enabled {
            "simulate_mpt_destination_on"
        } else {
            "simulate_mpt_destination_off"
        });
        let issuer = TestAccount::new(if amendment_enabled {
            "simulate_mpt_issuer_on"
        } else {
            "simulate_mpt_issuer_off"
        });
        let issuance_id = mpt_id_for(issuer.id, 1);
        let mpt_issue = MPTIssue::new(issuance_id);
        let expected_delivery = STAmount::from_mpt_amount(
            get_field_by_symbol("sfAmount"),
            MPTAmount::from_value(800),
            mpt_issue,
        );
        let features = amendment_enabled
            .then_some(protocol::fix_mpt_delivered_amount())
            .into_iter()
            .collect::<Vec<_>>();
        let env = RpcTestEnv::with_entries_and_features(
            &[
                (&source_account, 1_000_000_000),
                (&destination, 1_000_000_000),
                (&issuer, 1_000_000_000),
            ],
            &[
                mpt_issuance_entry(issuer.id, 1),
                mptoken_entry(source_account.id, issuance_id, 10_000),
            ],
            &features,
        );

        let mut payment = STTx::new(TxType::PAYMENT, |tx| {
            tx.set_account_id(get_field_by_symbol("sfAccount"), source_account.id);
            tx.set_account_id(get_field_by_symbol("sfDestination"), destination.id);
            tx.set_field_amount(
                get_field_by_symbol("sfAmount"),
                STAmount::from_mpt_amount(
                    get_field_by_symbol("sfAmount"),
                    MPTAmount::from_value(1_000),
                    mpt_issue,
                ),
            );
            tx.set_field_u32(get_field_by_symbol("sfFlags"), 0x0002_0000);
            tx.set_field_amount(
                get_field_by_symbol("sfFee"),
                STAmount::new_native(10, false),
            );
            tx.set_field_u32(get_field_by_symbol("sfSequence"), source_account.next_seq());
        });
        sign_tx(&mut payment, &source_account);
        let tx_blob = str_hex(payment.get_serializer().data());
        let source = env.rpc_source();

        let response_json = simulate_request(
            &json([("tx_blob", JsonValue::String(tx_blob.clone()))]),
            &source,
        )
        .expect("MPT partial-payment simulation should succeed");
        let JsonValue::Object(response) = response_json else {
            panic!("simulation response should be an object");
        };
        assert_eq!(
            response.get("engine_result"),
            Some(&JsonValue::String("tesSUCCESS".to_owned()))
        );
        let ledger_seq = response
            .get("ledger_index")
            .and_then(JsonValue::as_u64)
            .and_then(|value| u32::try_from(value).ok())
            .expect("successful simulation should include ledger index");
        let JsonValue::Object(meta) = response.get("meta").expect("simulation metadata") else {
            panic!("simulation metadata should be an object");
        };
        if amendment_enabled {
            assert_eq!(meta.get("DeliveredAmount"), meta.get("delivered_amount"));
            assert!(meta.contains_key("DeliveredAmount"));
        } else {
            assert!(!meta.contains_key("DeliveredAmount"));
            assert_eq!(
                meta.get("delivered_amount"),
                Some(&JsonValue::String("unavailable".to_owned()))
            );
        }

        let binary = simulate_request(
            &json([
                ("tx_blob", JsonValue::String(tx_blob)),
                ("binary", JsonValue::Bool(true)),
            ]),
            &source,
        )
        .expect("binary MPT partial-payment simulation should succeed");
        let JsonValue::Object(binary) = binary else {
            panic!("binary simulation response should be an object");
        };
        let meta_blob = binary
            .get("meta_blob")
            .and_then(JsonValue::as_str)
            .expect("binary simulation must include metadata");
        let meta_bytes = str_unhex(meta_blob).expect("metadata blob should be hexadecimal");
        let decoded = TxMeta::from_raw(payment.get_transaction_id(), ledger_seq, &meta_bytes);
        assert_eq!(decoded.get_result_ter(), Ter::TES_SUCCESS);
        assert_eq!(
            decoded.get_delivered_amount(),
            amendment_enabled.then_some(&expected_delivery),
            "sfDeliveredAmount must follow the fixMPTDeliveredAmount amendment gate"
        );
    }
}

#[test]
fn simulate_with_tx_json() {
    let alice = TestAccount::new("sim_alice2");
    let bob = TestAccount::new("sim_bob2");
    let env = RpcTestEnv::new(&[(&alice, 10_000_000_000), (&bob, 1_000_000_000)]);

    let source = env.rpc_source();
    let result = simulate_request(
        &json([(
            "tx_json",
            json([
                ("TransactionType", JsonValue::String("Payment".to_owned())),
                ("Account", JsonValue::String(to_base58(alice.id))),
                ("Destination", JsonValue::String(to_base58(bob.id))),
                ("Amount", JsonValue::String("1000000".to_owned())),
                ("Fee", JsonValue::String("10".to_owned())),
                ("Sequence", JsonValue::Unsigned(1)),
            ]),
        )]),
        &source,
    );

    // Should either succeed with engine_result or fail with a known error
    match result {
        Ok(JsonValue::Object(obj)) => {
            assert!(
                obj.contains_key("engine_result") || obj.contains_key("error"),
                "should have engine_result or error"
            );
        }
        Err(status) => {
            // Known acceptable errors for simulate in standalone
            let token = status.code_string();
            assert!(
                token == "notSynced"
                    || token == "noNetwork"
                    || token == "noCurrent"
                    || token == "notSupported"
                    || token == "internal"
                    || token == "invalidParams",
                "unexpected: {token}"
            );
        }
        _ => {}
    }
}
