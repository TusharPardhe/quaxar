//! `ledger` with `owner_funds`: rippled LedgerToJson fills `owner_funds` on
//! expanded OfferCreate transactions.

use protocol::{
    Issue, JsonValue, STAmount, STTx, TxType, currency_from_string, get_field_by_symbol,
    lsfDefaultRipple,
};
use rpc_integration_tests::env::*;

fn iou(field: &str, issue: Issue, value: u64) -> STAmount {
    STAmount::new_with_asset(get_field_by_symbol(field), issue, value, 0, false)
}

fn signed(
    account: &mut TestAccount,
    tx_type: TxType,
    fill: impl FnOnce(&mut protocol::STObject),
) -> STTx {
    let sequence = account.next_seq();
    let id = account.id;
    let mut tx = STTx::new(tx_type, |tx| {
        tx.set_account_id(get_field_by_symbol("sfAccount"), id);
        tx.set_field_amount(
            get_field_by_symbol("sfFee"),
            STAmount::new_native(10, false),
        );
        tx.set_field_u32(get_field_by_symbol("sfSequence"), sequence);
        fill(tx);
    });
    sign_tx(&mut tx, account);
    tx
}

/// rippled `fillJsonTx`: an expanded OfferCreate whose TakerGets is not the
/// owner's own IOU carries `owner_funds` = `accountFunds(...).getText()`.
/// Quaxar never filled it in (mainnet, 2026-11-25).
#[test]
fn ledger_expanded_offer_create_carries_owner_funds() {
    let mut alice = TestAccount::new("owner_funds_alice");
    let mut gateway = TestAccount::new("owner_funds_gateway");
    let env = RpcTestEnv::with_flags(
        &[(&alice, 10_000_000_000), (&gateway, 10_000_000_000)],
        &[(&gateway, lsfDefaultRipple)],
    );
    let usd = Issue::new(currency_from_string("USD"), gateway.id);

    let trust = signed(&mut alice, TxType::TRUST_SET, |tx| {
        tx.set_field_amount(
            get_field_by_symbol("sfLimitAmount"),
            iou("sfLimitAmount", usd, 10_000),
        );
    });
    env.submit_and_close(&trust);
    let alice_id = alice.id;
    let fund = signed(&mut gateway, TxType::PAYMENT, |tx| {
        tx.set_account_id(get_field_by_symbol("sfDestination"), alice_id);
        tx.set_field_amount(get_field_by_symbol("sfAmount"), iou("sfAmount", usd, 1_000));
    });
    env.submit_and_close(&fund);
    let offer = signed(&mut alice, TxType::OFFER_CREATE, |tx| {
        tx.set_field_amount(
            get_field_by_symbol("sfTakerPays"),
            STAmount::new_native(400_000_000, false),
        );
        tx.set_field_amount(
            get_field_by_symbol("sfTakerGets"),
            iou("sfTakerGets", usd, 100),
        );
    });
    env.submit_and_close(&offer);

    let source = env.rpc_source();
    let offer_ledger = env
        .app
        .validated_ledger()
        .expect("standalone accept validates")
        .header()
        .seq;
    let JsonValue::Object(result) = rpc::do_ledger(
        &json([
            ("ledger_index", JsonValue::Unsigned(u64::from(offer_ledger))),
            ("transactions", JsonValue::Bool(true)),
            ("expand", JsonValue::Bool(true)),
            ("owner_funds", JsonValue::Bool(true)),
        ]),
        rpc::RpcRole::Admin,
        1,
        &source,
    ) else {
        panic!("ledger must return an object");
    };
    let Some(JsonValue::Object(ledger)) = result.get("ledger") else {
        panic!("ledger must be an object: {result:?}");
    };
    let Some(JsonValue::Array(transactions)) = ledger.get("transactions") else {
        panic!("transactions must be an array: {ledger:?}");
    };
    let offer_entry = transactions
        .iter()
        .find_map(|entry| match entry {
            JsonValue::Object(entry)
                if entry.get("TransactionType")
                    == Some(&JsonValue::String("OfferCreate".to_owned())) =>
            {
                Some(entry)
            }
            _ => None,
        })
        .unwrap_or_else(|| panic!("no OfferCreate in {transactions:?}"));
    assert_eq!(
        offer_entry.get("owner_funds"),
        Some(&JsonValue::String("1000".to_owned())),
        "{offer_entry:?}"
    );
}
