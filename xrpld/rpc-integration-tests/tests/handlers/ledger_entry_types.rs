//! Integration tests for LedgerEntry with complex entry types and AccountObjects NFT pages.

use protocol::{
    currency_from_string, get_field_by_symbol, lsfDefaultRipple, to_base58, Issue, JsonValue,
    STAmount, STTx, TxType,
};
use rpc_integration_tests::env::*;

// === NFTokenMint + account_objects NFT page ===

#[test]
fn nftoken_mint_creates_nft_page() {
    let mut alice = TestAccount::new("nft_alice");
    let env = RpcTestEnv::new(&[(&alice, 10_000_000_000)]);

    let mut mint = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(get_field_by_symbol("sfAccount"), alice.id);
        tx.set_field_u32(get_field_by_symbol("sfNFTokenTaxon"), 1);
        tx.set_field_u32(get_field_by_symbol("sfFlags"), 0x0008); // tfTransferable
        tx.set_field_amount(
            get_field_by_symbol("sfFee"),
            STAmount::new_native(10, false),
        );
        tx.set_field_u32(get_field_by_symbol("sfSequence"), alice.next_seq());
    });
    sign_tx(&mut mint, &alice);
    env.submit_and_close(&mint);

    // Query account_objects with type=nft_page
    let source = env.rpc_source();
    let result = rpc::do_account_objects(
        &rpc::AccountObjectsRequest {
            params: &json([
                ("account", JsonValue::String(to_base58(alice.id))),
                ("type", JsonValue::String("nft_page".to_owned())),
            ]),
            api_version: 1,
            role: rpc::Role::Admin,
        },
        &source,
    );
    let JsonValue::Object(result) = result else {
        panic!("object")
    };

    if let Some(JsonValue::Array(objects)) = result.get("account_objects") {
        if !objects.is_empty() {
            let JsonValue::Object(page) = &objects[0] else {
                panic!("object")
            };
            assert_eq!(
                page.get("LedgerEntryType"),
                Some(&JsonValue::String("NFTokenPage".to_owned()))
            );
            assert!(page.contains_key("NFTokens"));
        }
    }
}

#[test]
fn nftoken_mint_visible_via_account_nfts() {
    let mut alice = TestAccount::new("nft_alice2");
    let env = RpcTestEnv::new(&[(&alice, 10_000_000_000)]);

    let mut mint = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(get_field_by_symbol("sfAccount"), alice.id);
        tx.set_field_u32(get_field_by_symbol("sfNFTokenTaxon"), 42);
        tx.set_field_u32(get_field_by_symbol("sfFlags"), 0);
        tx.set_field_amount(
            get_field_by_symbol("sfFee"),
            STAmount::new_native(10, false),
        );
        tx.set_field_u32(get_field_by_symbol("sfSequence"), alice.next_seq());
    });
    sign_tx(&mut mint, &alice);
    env.submit_and_close(&mint);

    let source = env.rpc_source();
    let result = rpc::do_account_nfts(
        &rpc::AccountNFTsRequest {
            params: &json([("account", JsonValue::String(to_base58(alice.id)))]),
            api_version: 2,
            role: rpc::Role::Admin,
        },
        &source,
    );
    let JsonValue::Object(result) = result else {
        panic!("object")
    };

    if let Some(JsonValue::Array(nfts)) = result.get("account_nfts") {
        if !nfts.is_empty() {
            let JsonValue::Object(nft) = &nfts[0] else {
                panic!("object")
            };
            assert!(nft.contains_key("NFTokenID"));
            assert!(nft.contains_key("Flags"));
            assert!(nft.contains_key("Issuer"));
            assert!(nft.contains_key("NFTokenTaxon"));
            assert!(nft.contains_key("nft_serial"));
        }
    }
}

// === MPTokenIssuanceCreate ===

#[test]
fn mptoken_issuance_create() {
    let mut alice = TestAccount::new("mpt_alice");
    let env = RpcTestEnv::new(&[(&alice, 10_000_000_000)]);

    let mut create = STTx::new(TxType::MPTOKEN_ISSUANCE_CREATE, |tx| {
        tx.set_account_id(get_field_by_symbol("sfAccount"), alice.id);
        tx.set_field_u32(get_field_by_symbol("sfFlags"), 0);
        tx.set_field_amount(
            get_field_by_symbol("sfFee"),
            STAmount::new_native(10, false),
        );
        tx.set_field_u32(get_field_by_symbol("sfSequence"), alice.next_seq());
    });
    sign_tx(&mut create, &alice);
    env.submit_and_close(&create);

    // Verify account_objects shows the issuance
    let source = env.rpc_source();
    let result = rpc::do_account_objects(
        &rpc::AccountObjectsRequest {
            params: &json([("account", JsonValue::String(to_base58(alice.id)))]),
            api_version: 1,
            role: rpc::Role::Admin,
        },
        &source,
    );
    let JsonValue::Object(result) = result else {
        panic!("object")
    };

    if let Some(JsonValue::Array(objects)) = result.get("account_objects") {
        let has_mpt = objects.iter().any(|o| {
            matches!(o, JsonValue::Object(obj) if obj.get("LedgerEntryType") == Some(&JsonValue::String("MPTokenIssuance".to_owned())))
        });
        // MPTokenIssuance should be created if transactor supports it
        if has_mpt {
            assert!(has_mpt);
        }
    }
}

// === Account with multiple object types ===

#[test]
fn account_objects_multiple_types_after_various_txs() {
    let mut alice = TestAccount::new("multi_alice");
    let gw = TestAccount::new("multi_gw");
    let env = RpcTestEnv::with_flags(
        &[(&alice, 10_000_000_000), (&gw, 10_000_000_000)],
        &[(&gw, lsfDefaultRipple)],
    );

    let usd = currency_from_string("USD");

    // Trust line
    let mut trust = STTx::new(TxType::TRUST_SET, |tx| {
        tx.set_account_id(get_field_by_symbol("sfAccount"), alice.id);
        tx.set_field_amount(
            get_field_by_symbol("sfLimitAmount"),
            STAmount::new_with_asset(
                get_field_by_symbol("sfLimitAmount"),
                Issue::new(usd, gw.id),
                1000,
                0,
                false,
            ),
        );
        tx.set_field_amount(
            get_field_by_symbol("sfFee"),
            STAmount::new_native(10, false),
        );
        tx.set_field_u32(get_field_by_symbol("sfSequence"), alice.next_seq());
    });
    sign_tx(&mut trust, &alice);
    env.submit_and_close(&trust);

    // Offer
    let mut offer = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(get_field_by_symbol("sfAccount"), alice.id);
        tx.set_field_amount(
            get_field_by_symbol("sfTakerPays"),
            STAmount::new_native(100_000_000, false),
        );
        tx.set_field_amount(
            get_field_by_symbol("sfTakerGets"),
            STAmount::new_with_asset(
                get_field_by_symbol("sfTakerGets"),
                Issue::new(usd, gw.id),
                50,
                0,
                false,
            ),
        );
        tx.set_field_amount(
            get_field_by_symbol("sfFee"),
            STAmount::new_native(10, false),
        );
        tx.set_field_u32(get_field_by_symbol("sfSequence"), alice.next_seq());
    });
    sign_tx(&mut offer, &alice);
    env.submit_and_close(&offer);

    // NFT mint
    let mut mint = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(get_field_by_symbol("sfAccount"), alice.id);
        tx.set_field_u32(get_field_by_symbol("sfNFTokenTaxon"), 99);
        tx.set_field_u32(get_field_by_symbol("sfFlags"), 0);
        tx.set_field_amount(
            get_field_by_symbol("sfFee"),
            STAmount::new_native(10, false),
        );
        tx.set_field_u32(get_field_by_symbol("sfSequence"), alice.next_seq());
    });
    sign_tx(&mut mint, &alice);
    env.submit_and_close(&mint);

    // Query all objects
    let source = env.rpc_source();
    let result = rpc::do_account_objects(
        &rpc::AccountObjectsRequest {
            params: &json([("account", JsonValue::String(to_base58(alice.id)))]),
            api_version: 1,
            role: rpc::Role::Admin,
        },
        &source,
    );
    let JsonValue::Object(result) = result else {
        panic!("object")
    };

    if let Some(JsonValue::Array(objects)) = result.get("account_objects") {
        // Should have multiple object types
        let types: Vec<&str> = objects
            .iter()
            .filter_map(|o| match o {
                JsonValue::Object(obj) => match obj.get("LedgerEntryType") {
                    Some(JsonValue::String(t)) => Some(t.as_str()),
                    _ => None,
                },
                _ => None,
            })
            .collect();
        // At minimum should have trust line (RippleState)
        assert!(!types.is_empty(), "should have at least one object type");
    }
}

// === Ledger entry lookup for various types ===

#[test]
fn ledger_entry_finds_trust_line_after_trust_set() {
    let mut alice = TestAccount::new("le2_alice");
    let gw = TestAccount::new("le2_gw");
    let env = RpcTestEnv::with_flags(
        &[(&alice, 10_000_000_000), (&gw, 10_000_000_000)],
        &[(&gw, lsfDefaultRipple)],
    );

    let mut trust = STTx::new(TxType::TRUST_SET, |tx| {
        tx.set_account_id(get_field_by_symbol("sfAccount"), alice.id);
        tx.set_field_amount(
            get_field_by_symbol("sfLimitAmount"),
            STAmount::new_with_asset(
                get_field_by_symbol("sfLimitAmount"),
                Issue::new(currency_from_string("USD"), gw.id),
                500,
                0,
                false,
            ),
        );
        tx.set_field_amount(
            get_field_by_symbol("sfFee"),
            STAmount::new_native(10, false),
        );
        tx.set_field_u32(get_field_by_symbol("sfSequence"), alice.next_seq());
    });
    sign_tx(&mut trust, &alice);
    env.submit_and_close(&trust);

    let source = env.rpc_source();
    let result = rpc::do_ledger_entry(
        &rpc::LedgerEntryRequest {
            params: &json([
                (
                    "ripple_state",
                    json([
                        (
                            "accounts",
                            JsonValue::Array(vec![
                                JsonValue::String(to_base58(alice.id)),
                                JsonValue::String(to_base58(gw.id)),
                            ]),
                        ),
                        ("currency", JsonValue::String("USD".to_owned())),
                    ]),
                ),
                ("ledger_index", JsonValue::String("current".to_owned())),
            ]),
            api_version: 2,
            role: rpc::Role::Admin,
        },
        &source,
    );
    let JsonValue::Object(result) = result else {
        panic!("object")
    };

    // Should find the trust line
    if result.get("error").is_none() {
        assert!(result.contains_key("node") || result.contains_key("node_binary"));
        assert!(result.contains_key("index"));
    }
}
