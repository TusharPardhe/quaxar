use std::collections::BTreeMap;

use basics::chrono::to_string_iso;
use basics::str_hex::str_hex;
use basics::tagged_cache::CacheClock;
use ledger::Ledger;
use protocol::{
    JsonOptions, JsonValue, LedgerEntryType, STTx, StBase, TxMeta, TxType, get_field_by_symbol,
    make_mpt_id,
};
use shamap::family::{FullBelowCache, MissingNodeReporter, SHAMapFamily, SHAMapNodeFetcher};
use shamap::traversal::TraversalError;
use std::hash::BuildHasher;

use crate::ledger_to_json::ledger_to_json_owner_funds::insert_owner_funds;
use crate::{AppLedgerFill, LedgerTxEntry, copy_from, insert_deliver_max};

const DELIVERED_AMOUNT_SWITCH_LEDGER: u32 = 4_594_095;
const DELIVERED_AMOUNT_SWITCH_CLOSE_TIME: u32 = 446_000_000;

pub(crate) fn fill_json_transactions(json: &mut JsonValue, fill: &AppLedgerFill<'_>) {
    let JsonValue::Object(root) = json else {
        panic!("ledger json root must be an object");
    };

    let expanded = fill.is_expanded();
    let binary = fill.is_binary();
    let txs = fill
        .transactions
        .iter()
        .map(|entry| fill_json_tx_entry(fill, binary, expanded, entry.txn, entry.meta))
        .collect();
    root.insert("transactions".to_owned(), JsonValue::Array(txs));
}

pub(crate) fn fill_json_transactions_with_family<CLOCK, S, C, F, MR, NS>(
    json: &mut JsonValue,
    fill: &AppLedgerFill<'_>,
    family: &SHAMapFamily<CLOCK, S, C, F, MR, NS>,
) -> Result<(), TraversalError>
where
    CLOCK: CacheClock,
    S: BuildHasher + Clone,
    C: FullBelowCache,
    F: SHAMapNodeFetcher,
    MR: MissingNodeReporter,
{
    let JsonValue::Object(root) = json else {
        panic!("ledger json root must be an object");
    };

    let expanded = fill.is_expanded();
    let binary = fill.is_binary();
    let mut txs = Vec::new();
    fill.ledger
        .tx_map()
        .visit_leaves_with_family(family, &mut |item| {
            if let Ok((txn, meta)) =
                ledger::decode_transaction_md_item(fill.ledger.header().seq, item)
            {
                txs.push(fill_json_tx_entry(
                    fill,
                    binary,
                    expanded,
                    txn.as_ref(),
                    Some(&meta),
                ));
            }
        })?;
    root.insert("transactions".to_owned(), JsonValue::Array(txs));
    Ok(())
}

pub(crate) fn fill_json_tx_entry(
    fill: &AppLedgerFill<'_>,
    binary: bool,
    expanded: bool,
    txn: &STTx,
    meta: Option<&TxMeta>,
) -> JsonValue {
    if !expanded {
        return JsonValue::String(txn.get_transaction_id().to_string());
    }

    let mut tx_json = JsonValue::Object(BTreeMap::new());
    if binary {
        fill_json_tx_binary(&mut tx_json, fill, txn, meta);
    } else if fill.api_version() > 1 {
        fill_json_tx_v2(&mut tx_json, fill, txn, meta);
    } else {
        fill_json_tx_v1(&mut tx_json, fill, txn, meta);
    }

    insert_owner_funds(&mut tx_json, fill, txn);
    tx_json
}

fn fill_json_tx_binary(
    tx_json: &mut JsonValue,
    fill: &AppLedgerFill<'_>,
    txn: &STTx,
    meta: Option<&TxMeta>,
) {
    let JsonValue::Object(object) = tx_json else {
        unreachable!("transaction JSON shell should be an object");
    };

    object.insert(
        "tx_blob".to_owned(),
        JsonValue::String(str_hex(txn.get_serializer().data())),
    );
    if fill.api_version() > 1 {
        object.insert(
            "hash".to_owned(),
            JsonValue::String(txn.get_transaction_id().to_string()),
        );
    }

    if let Some(meta) = meta {
        let meta_key = if fill.api_version() > 1 {
            "meta_blob"
        } else {
            "meta"
        };
        object.insert(
            meta_key.to_owned(),
            JsonValue::String(str_hex(meta.get_as_object().get_serializer().data())),
        );
    }
}

fn fill_json_tx_v2(
    tx_json: &mut JsonValue,
    fill: &AppLedgerFill<'_>,
    txn: &STTx,
    meta: Option<&TxMeta>,
) {
    let JsonValue::Object(object) = tx_json else {
        unreachable!("transaction JSON shell should be an object");
    };

    let nested = object
        .entry("tx_json".to_owned())
        .or_insert(JsonValue::Object(BTreeMap::new()));
    copy_from(nested, &txn.json(JsonOptions::DISABLE_API_PRIOR_V2));
    insert_deliver_max(nested, txn.get_txn_type(), fill.api_version());

    object.insert(
        "hash".to_owned(),
        JsonValue::String(txn.get_transaction_id().to_string()),
    );

    if let Some(meta) = meta {
        let mut meta_json = meta.get_json(JsonOptions::NONE);
        insert_delivered_amount(&mut meta_json, fill.ledger, txn, meta);
        insert_mp_token_issuance_id(&mut meta_json, txn, meta);
        object.insert("meta".to_owned(), meta_json);
    }

    if fill.ledger.is_immutable() {
        object.insert(
            "ledger_hash".to_owned(),
            JsonValue::String(fill.ledger.header().hash.to_string()),
        );
    }

    let validated = fill
        .context
        .is_some_and(|context| context.is_validated(fill.ledger));
    object.insert("validated".to_owned(), JsonValue::Bool(validated));
    if validated {
        object.insert(
            "ledger_index".to_owned(),
            JsonValue::Unsigned(u64::from(fill.ledger.header().seq)),
        );
        if let Some(close_time) = fill.close_time {
            object.insert(
                "close_time_iso".to_owned(),
                JsonValue::String(to_string_iso(close_time)),
            );
        }
    }
}

fn fill_json_tx_v1(
    tx_json: &mut JsonValue,
    fill: &AppLedgerFill<'_>,
    txn: &STTx,
    meta: Option<&TxMeta>,
) {
    copy_from(tx_json, &txn.json(JsonOptions::NONE));
    insert_deliver_max(tx_json, txn.get_txn_type(), fill.api_version());

    let JsonValue::Object(object) = tx_json else {
        unreachable!("transaction JSON shell should be an object");
    };

    if let Some(meta) = meta {
        let mut meta_json = meta.get_json(JsonOptions::NONE);
        insert_delivered_amount(&mut meta_json, fill.ledger, txn, meta);
        insert_mp_token_issuance_id(&mut meta_json, txn, meta);
        object.insert("metaData".to_owned(), meta_json);
    }
}

pub(crate) fn transaction_subscription_event(
    ledger: &Ledger,
    txn: &STTx,
    meta: &TxMeta,
) -> JsonValue {
    let mut meta_json = meta.get_json(JsonOptions::NONE);
    insert_delivered_amount(&mut meta_json, ledger, txn, meta);
    JsonValue::Object(BTreeMap::from([
        (
            "type".to_owned(),
            JsonValue::String("transaction".to_owned()),
        ),
        ("transaction".to_owned(), txn.json(JsonOptions::NONE)),
        ("meta".to_owned(), meta_json),
        (
            "ledger_index".to_owned(),
            JsonValue::Unsigned(u64::from(ledger.header().seq)),
        ),
        (
            "ledger_hash".to_owned(),
            JsonValue::String(ledger.header().hash.to_string()),
        ),
        ("validated".to_owned(), JsonValue::Bool(true)),
    ]))
}

fn insert_delivered_amount(meta_json: &mut JsonValue, ledger: &Ledger, txn: &STTx, meta: &TxMeta) {
    if !can_have_delivered_amount(txn, meta) {
        return;
    }

    let delivered = get_delivered_amount(ledger, txn, meta)
        .map(|amount| amount.json(JsonOptions::INCLUDE_DATE))
        .unwrap_or_else(|| JsonValue::String("unavailable".to_owned()));

    let JsonValue::Object(object) = meta_json else {
        return;
    };
    object.insert("delivered_amount".to_owned(), delivered);
}

fn can_have_delivered_amount(txn: &STTx, meta: &TxMeta) -> bool {
    matches!(txn.get_txn_type(), TxType::PAYMENT | TxType::CHECK_CASH)
        && meta.get_result_ter().to_int() == 0
}

fn get_delivered_amount(ledger: &Ledger, txn: &STTx, meta: &TxMeta) -> Option<protocol::STAmount> {
    if let Some(amount) = meta.get_delivered_amount() {
        return Some(amount.clone());
    }

    let amount_field = get_field_by_symbol("sfAmount");
    if !txn.is_field_present(amount_field) {
        return None;
    }

    let header = ledger.header();
    if header.seq >= DELIVERED_AMOUNT_SWITCH_LEDGER
        || header.close_time > DELIVERED_AMOUNT_SWITCH_CLOSE_TIME
    {
        return Some(txn.get_field_amount(amount_field));
    }

    None
}

fn insert_mp_token_issuance_id(meta_json: &mut JsonValue, txn: &STTx, meta: &TxMeta) {
    if txn.get_txn_type() != TxType::MPTOKEN_ISSUANCE_CREATE || meta.get_result_ter().to_int() != 0
    {
        return;
    }

    let Some(issuance_id) = get_created_mp_token_issuance_id(meta) else {
        return;
    };

    let JsonValue::Object(object) = meta_json else {
        return;
    };
    object.insert(
        "mpt_issuance_id".to_owned(),
        JsonValue::String(issuance_id.to_string()),
    );
}

fn get_created_mp_token_issuance_id(meta: &TxMeta) -> Option<protocol::MPTID> {
    for node in meta.get_nodes().iter() {
        if node.fname() != get_field_by_symbol("sfCreatedNode")
            || node.get_field_u16(get_field_by_symbol("sfLedgerEntryType"))
                != LedgerEntryType::MPTokenIssuance.code()
        {
            continue;
        }

        let issuance = node.get_field_object(get_field_by_symbol("sfNewFields"));
        return Some(make_mpt_id(
            issuance.get_field_u32(get_field_by_symbol("sfSequence")),
            issuance.get_account_id(get_field_by_symbol("sfIssuer")),
        ));
    }

    None
}

pub(crate) fn fill_json_queue_tx(fill: &AppLedgerFill<'_>, txn: &STTx) -> JsonValue {
    fill_json_tx_entry(fill, fill.is_binary(), fill.is_expanded(), txn, None)
}

#[allow(dead_code)]
fn _keep_items_used(_entry: LedgerTxEntry<'_>) {}
