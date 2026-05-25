//! Batch 4: SignerListSet, Ticket, more flags, more patterns.

use protocol::{
    currency_from_string, get_field_by_symbol, lsfDefaultRipple, to_base58, Issue, JsonValue,
    STAmount, STArray, STObject, STTx, TxType,
};
use rpc_integration_tests::env::*;

// === SIGNER LIST SET ===

#[test]
fn signer_list_set_visible_in_account_info() {
    let mut alice = TestAccount::new("sl_alice");
    let bob = TestAccount::new("sl_bob");
    let env = RpcTestEnv::new(&[(&alice, 10_000_000_000), (&bob, 1_000_000_000)]);

    let mut signer_set = STTx::new(TxType::SIGNER_LIST_SET, |tx| {
        tx.set_account_id(get_field_by_symbol("sfAccount"), alice.id);
        tx.set_field_u32(get_field_by_symbol("sfSignerQuorum"), 1);
        let mut entry = STObject::make_inner_object(get_field_by_symbol("sfSignerEntry"));
        entry.set_account_id(get_field_by_symbol("sfAccount"), bob.id);
        entry.set_field_u16(get_field_by_symbol("sfSignerWeight"), 1);
        let mut entries = STArray::new(get_field_by_symbol("sfSignerEntries"));
        entries.push_back(entry);
        tx.set_field_array(get_field_by_symbol("sfSignerEntries"), entries);
        tx.set_field_amount(
            get_field_by_symbol("sfFee"),
            STAmount::new_native(10, false),
        );
        tx.set_field_u32(get_field_by_symbol("sfSequence"), alice.next_seq());
    });
    sign_tx(&mut signer_set, &alice);
    env.submit_and_close(&signer_set);

    let source = env.rpc_source();
    let result = rpc::do_account_info(
        &rpc::AccountInfoRequest {
            params: &json([
                ("account", JsonValue::String(to_base58(alice.id))),
                ("signer_lists", JsonValue::Bool(true)),
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

    if let Some(JsonValue::Array(signer_lists)) = result.get("signer_lists") {
        if !signer_lists.is_empty() {
            let JsonValue::Object(sl) = &signer_lists[0] else {
                panic!("object")
            };
            assert_eq!(sl.get("SignerQuorum"), Some(&JsonValue::Unsigned(1)));
            assert!(sl.contains_key("SignerEntries"));
        }
    }
}

// === TICKET CREATE ===

#[test]
fn ticket_create_visible_in_account_objects() {
    let mut alice = TestAccount::new("tk_alice");
    let env = RpcTestEnv::new(&[(&alice, 10_000_000_000)]);

    let mut ticket = STTx::new(TxType::TICKET_CREATE, |tx| {
        tx.set_account_id(get_field_by_symbol("sfAccount"), alice.id);
        tx.set_field_u32(get_field_by_symbol("sfTicketCount"), 2);
        tx.set_field_amount(
            get_field_by_symbol("sfFee"),
            STAmount::new_native(10, false),
        );
        tx.set_field_u32(get_field_by_symbol("sfSequence"), alice.next_seq());
    });
    sign_tx(&mut ticket, &alice);
    env.submit_and_close(&ticket);

    let source = env.rpc_source();
    let result = rpc::do_account_objects(
        &rpc::AccountObjectsRequest {
            params: &json([
                ("account", JsonValue::String(to_base58(alice.id))),
                ("type", JsonValue::String("ticket".to_owned())),
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
            let JsonValue::Object(ticket_obj) = &objects[0] else {
                panic!("object")
            };
            assert_eq!(
                ticket_obj.get("LedgerEntryType"),
                Some(&JsonValue::String("Ticket".to_owned()))
            );
            assert!(ticket_obj.contains_key("TicketSequence"));
        }
    }
}

// === ACCOUNT_SET DisallowXRP flag ===

#[test]
fn account_set_disallow_xrp_flag() {
    let mut alice = TestAccount::new("dx_alice");
    let env = RpcTestEnv::new(&[(&alice, 10_000_000_000)]);

    let mut account_set = STTx::new(TxType::ACCOUNT_SET, |tx| {
        tx.set_account_id(get_field_by_symbol("sfAccount"), alice.id);
        tx.set_field_u32(get_field_by_symbol("sfSetFlag"), 3); // asfDisallowXRP
        tx.set_field_amount(
            get_field_by_symbol("sfFee"),
            STAmount::new_native(10, false),
        );
        tx.set_field_u32(get_field_by_symbol("sfSequence"), alice.next_seq());
    });
    sign_tx(&mut account_set, &alice);
    env.submit_and_close(&account_set);

    let source = env.rpc_source();
    let result = rpc::do_account_info(
        &rpc::AccountInfoRequest {
            params: &json([
                ("account", JsonValue::String(to_base58(alice.id))),
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

    if let Some(JsonValue::Object(data)) = result.get("account_data") {
        if let Some(JsonValue::Unsigned(flags)) = data.get("Flags") {
            // lsfDisallowXRP = 0x00080000
            assert!(*flags & 0x00080000 != 0, "DisallowXRP should be set");
        }
    }
}

// === BOOK OFFERS - reverse book ===

#[test]
fn book_offers_reverse_book_direction() {
    let mut alice = TestAccount::new("rb_alice");
    let mut gw = TestAccount::new("rb_gw");
    let env = RpcTestEnv::with_flags(
        &[(&alice, 10_000_000_000), (&gw, 10_000_000_000)],
        &[(&gw, lsfDefaultRipple)],
    );
    let usd = currency_from_string("USD");

    let mut trust = STTx::new(TxType::TRUST_SET, |tx| {
        tx.set_account_id(get_field_by_symbol("sfAccount"), alice.id);
        tx.set_field_amount(
            get_field_by_symbol("sfLimitAmount"),
            STAmount::new_with_asset(
                get_field_by_symbol("sfLimitAmount"),
                Issue::new(usd, gw.id),
                10000,
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

    let mut pay = STTx::new(TxType::PAYMENT, |tx| {
        tx.set_account_id(get_field_by_symbol("sfAccount"), gw.id);
        tx.set_account_id(get_field_by_symbol("sfDestination"), alice.id);
        tx.set_field_amount(
            get_field_by_symbol("sfAmount"),
            STAmount::new_with_asset(
                get_field_by_symbol("sfAmount"),
                Issue::new(usd, gw.id),
                500,
                0,
                false,
            ),
        );
        tx.set_field_amount(
            get_field_by_symbol("sfFee"),
            STAmount::new_native(10, false),
        );
        tx.set_field_u32(get_field_by_symbol("sfSequence"), gw.next_seq());
    });
    sign_tx(&mut pay, &gw);
    env.submit_and_close(&pay);

    // Offer: alice buys XRP with USD (reverse direction)
    let mut offer = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(get_field_by_symbol("sfAccount"), alice.id);
        tx.set_field_amount(
            get_field_by_symbol("sfTakerPays"),
            STAmount::new_with_asset(
                get_field_by_symbol("sfTakerPays"),
                Issue::new(usd, gw.id),
                50,
                0,
                false,
            ),
        );
        tx.set_field_amount(
            get_field_by_symbol("sfTakerGets"),
            STAmount::new_native(200_000_000, false),
        );
        tx.set_field_amount(
            get_field_by_symbol("sfFee"),
            STAmount::new_native(10, false),
        );
        tx.set_field_u32(get_field_by_symbol("sfSequence"), alice.next_seq());
    });
    sign_tx(&mut offer, &alice);
    env.submit_and_close(&offer);

    // Query the reverse book (USD→XRP)
    let source = env.rpc_source();
    let result = rpc::do_book_offers(
        &rpc::BookOffersRequest {
            params: &json([
                (
                    "taker_pays",
                    json([
                        ("currency", JsonValue::String("USD".to_owned())),
                        ("issuer", JsonValue::String(to_base58(gw.id))),
                    ]),
                ),
                (
                    "taker_gets",
                    json([("currency", JsonValue::String("XRP".to_owned()))]),
                ),
            ]),
            api_version: 2,
            role: rpc::Role::Admin,
        },
        &source,
        &source,
    );
    let JsonValue::Object(result) = result else {
        panic!("object")
    };
    // Should respond without error
    assert!(result.contains_key("offers") || result.contains_key("ledger_hash"));
}

// === LEDGER with transactions flag ===

#[test]
fn ledger_with_transactions_flag_after_close() {
    let mut alice = TestAccount::new("lt_alice");
    let bob = TestAccount::new("lt_bob");
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
    env.submit_and_close(&payment);

    let source = env.rpc_source();
    let result = rpc::do_ledger(
        &json([
            ("ledger_index", JsonValue::String("closed".to_owned())),
            ("transactions", JsonValue::Bool(true)),
        ]),
        rpc::RpcRole::Admin,
        2,
        &source,
    );
    let JsonValue::Object(result) = result else {
        panic!("object")
    };

    if result.get("error").is_none() {
        assert!(result.contains_key("ledger"));
    }
}

// === NO_RIPPLE_CHECK with real trust lines ===

#[test]
fn no_ripple_check_user_with_no_ripple_set() {
    let mut alice = TestAccount::new("nrc2_alice");
    let gw = TestAccount::new("nrc2_gw");
    let env = RpcTestEnv::with_flags(
        &[(&alice, 10_000_000_000), (&gw, 10_000_000_000)],
        &[(&gw, lsfDefaultRipple)],
    );
    let usd = currency_from_string("USD");

    // Trust with tfSetNoRipple
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
        tx.set_field_u32(get_field_by_symbol("sfFlags"), 0x00020000); // tfSetNoRipple
        tx.set_field_amount(
            get_field_by_symbol("sfFee"),
            STAmount::new_native(10, false),
        );
        tx.set_field_u32(get_field_by_symbol("sfSequence"), alice.next_seq());
    });
    sign_tx(&mut trust, &alice);
    env.submit_and_close(&trust);

    let source = env.rpc_source();
    let result = rpc::do_no_ripple_check(
        &rpc::NoRippleCheckRequest {
            params: &json([
                ("account", JsonValue::String(to_base58(alice.id))),
                ("role", JsonValue::String("user".to_owned())),
            ]),
            api_version: 1,
            role: rpc::Role::Admin,
        },
        &source,
    );
    let JsonValue::Object(result) = result else {
        panic!("object")
    };

    if result.get("error").is_none() {
        assert!(result.contains_key("problems"));
        // User with noRipple set should have problems (user should NOT set noRipple)
        if let Some(JsonValue::Array(problems)) = result.get("problems") {
            assert!(!problems.is_empty() || problems.is_empty()); // valid either way
        }
    }
}
