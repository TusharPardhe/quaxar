//! Integration tests for book offers operations.

use protocol::{
    currency_from_string, get_field_by_symbol, lsfDefaultRipple, to_base58, Issue, JsonValue,
    STAmount, STTx, TxType,
};
use rpc_integration_tests::env::*;
use rpc_integration_tests::helpers::*;

#[test]
fn defs_book_offers_bad_market() {
    let a = TestAccount::new("t47");
    let e = RpcTestEnv::new(&[(&a, 5_000_000_000)]);
    let s = e.rpc_source();
    let r = rpc::do_book_offers(
        &rpc::BookOffersRequest {
            params: &json([
                (
                    "taker_pays",
                    json([("currency", JsonValue::String("XRP".to_owned()))]),
                ),
                (
                    "taker_gets",
                    json([("currency", JsonValue::String("XRP".to_owned()))]),
                ),
            ]),
            api_version: 2,
            role: rpc::Role::Admin,
        },
        &s,
        &s,
    );
    let JsonValue::Object(r) = r else {
        panic!("obj")
    };
    assert_eq!(
        r.get("error"),
        Some(&JsonValue::String("badMarket".to_owned()))
    );
}

#[test]
fn defs_book_offers_missing_pays() {
    let a = TestAccount::new("t48");
    let e = RpcTestEnv::new(&[(&a, 5_000_000_000)]);
    let s = e.rpc_source();
    let r = rpc::do_book_offers(
        &rpc::BookOffersRequest {
            params: &json([(
                "taker_gets",
                json([("currency", JsonValue::String("USD".to_owned()))]),
            )]),
            api_version: 2,
            role: rpc::Role::Admin,
        },
        &s,
        &s,
    );
    let JsonValue::Object(r) = r else {
        panic!("obj")
    };
    assert!(r.contains_key("error"));
}

#[test]
fn defs_ext_62() {
    let a = TestAccount::new("d62");
    let e = RpcTestEnv::new(&[(&a, 5_000_000_000)]);
    let s = e.rpc_source();
    let JsonValue::Object(r) = rpc::do_book_offers(
        &rpc::BookOffersRequest {
            params: &json([
                ("taker_pays", json([("currency", sv("XRP"))])),
                ("taker_gets", json([("currency", sv("XRP"))])),
            ]),
            api_version: 2,
            role: rpc::Role::Admin,
        },
        &s,
        &s,
    ) else {
        panic!("")
    };
    assert_eq!(r.get("error"), Some(&sv("badMarket")));
}

#[test]
fn defs_ext_63() {
    let a = TestAccount::new("d63");
    let e = RpcTestEnv::new(&[(&a, 5_000_000_000)]);
    let s = e.rpc_source();
    let JsonValue::Object(r) = rpc::do_book_offers(
        &rpc::BookOffersRequest {
            params: &obj(),
            api_version: 2,
            role: rpc::Role::Admin,
        },
        &s,
        &s,
    ) else {
        panic!("")
    };
    assert!(r.contains_key("error"));
}

#[test]
fn defs_fmt_97() {
    let a = TestAccount::new("c97");
    let e = RpcTestEnv::new(&[(&a, 5_000_000_000)]);
    let s = e.rpc_source();
    let JsonValue::Object(r) = rpc::do_book_offers(
        &rpc::BookOffersRequest {
            params: &JsonValue::Object(Default::default()),
            api_version: 2,
            role: rpc::Role::Admin,
        },
        &s,
        &s,
    ) else {
        panic!("")
    };
    assert!(r.contains_key("error"));
}

#[test]
fn defs_types_61_book_offer_flags_zero() {
    let mut a = TestAccount::new("b61a");
    let mut g = TestAccount::new("b61g");
    let e = RpcTestEnv::with_flags(
        &[(&a, 10_000_000_000), (&g, 10_000_000_000)],
        &[(&g, lsfDefaultRipple)],
    );
    let usd = currency_from_string("USD");
    let mut t = STTx::new(TxType::TRUST_SET, |tx| {
        tx.set_account_id(get_field_by_symbol("sfAccount"), a.id);
        tx.set_field_amount(
            get_field_by_symbol("sfLimitAmount"),
            STAmount::new_with_asset(
                get_field_by_symbol("sfLimitAmount"),
                Issue::new(usd, g.id),
                10000,
                0,
                false,
            ),
        );
        tx.set_field_amount(
            get_field_by_symbol("sfFee"),
            STAmount::new_native(10, false),
        );
        tx.set_field_u32(get_field_by_symbol("sfSequence"), a.next_seq());
    });
    sign_tx(&mut t, &a);
    e.submit_and_close(&t);
    let mut p = STTx::new(TxType::PAYMENT, |tx| {
        tx.set_account_id(get_field_by_symbol("sfAccount"), g.id);
        tx.set_account_id(get_field_by_symbol("sfDestination"), a.id);
        tx.set_field_amount(
            get_field_by_symbol("sfAmount"),
            STAmount::new_with_asset(
                get_field_by_symbol("sfAmount"),
                Issue::new(usd, g.id),
                500,
                0,
                false,
            ),
        );
        tx.set_field_amount(
            get_field_by_symbol("sfFee"),
            STAmount::new_native(10, false),
        );
        tx.set_field_u32(get_field_by_symbol("sfSequence"), g.next_seq());
    });
    sign_tx(&mut p, &g);
    e.submit_and_close(&p);
    let mut o = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(get_field_by_symbol("sfAccount"), a.id);
        tx.set_field_amount(
            get_field_by_symbol("sfTakerPays"),
            STAmount::new_native(100_000_000, false),
        );
        tx.set_field_amount(
            get_field_by_symbol("sfTakerGets"),
            STAmount::new_with_asset(
                get_field_by_symbol("sfTakerGets"),
                Issue::new(usd, g.id),
                50,
                0,
                false,
            ),
        );
        tx.set_field_amount(
            get_field_by_symbol("sfFee"),
            STAmount::new_native(10, false),
        );
        tx.set_field_u32(get_field_by_symbol("sfSequence"), a.next_seq());
    });
    sign_tx(&mut o, &a);
    e.submit_and_close(&o);
    let s = e.rpc_source();
    let r = rpc::do_book_offers(
        &rpc::BookOffersRequest {
            params: &json([
                ("taker_pays", json([("currency", sv("XRP"))])),
                (
                    "taker_gets",
                    json([
                        ("currency", sv("USD")),
                        ("issuer", JsonValue::String(to_base58(g.id))),
                    ]),
                ),
            ]),
            api_version: 2,
            role: rpc::Role::Admin,
        },
        &s,
        &s,
    );
    let JsonValue::Object(r) = r else {
        panic!("obj")
    };
    if let Some(JsonValue::Array(offers)) = r.get("offers") {
        if !offers.is_empty() {
            let JsonValue::Object(of) = &offers[0] else {
                panic!("obj")
            };
            assert!(of.contains_key("Account"));
            assert!(of.contains_key("TakerPays"));
            assert!(of.contains_key("TakerGets"));
            assert!(of.contains_key("BookDirectory"));
            assert!(of.contains_key("quality"));
            assert!(of.contains_key("owner_funds"));
            assert!(of.contains_key("Sequence"));
            assert!(of.contains_key("index"));
        }
    }
}
// AccountLines: limit field value
