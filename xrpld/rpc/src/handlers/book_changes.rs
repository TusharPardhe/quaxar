//! `book_changes` RPC handler slice.
//!
//! This ports the the reference implementation `book_changes` handler over the already-landed
//! ledger lookup and protocol amount helpers. The orderbook aggregation itself
//! stays local and deterministic so the handler does not need any fake
//! `Application` or network runtime graph.

use std::collections::BTreeMap;

use basics::base_uint::Uint256;
use protocol::{
    Asset, JsonValue, LedgerEntryType, STAmount, STTx, StBase, TxMeta, TxType, asset_to_string,
    get_field_by_symbol, no_issue, sf_generic,
};

use crate::handlers::ledger_lookup::{
    LedgerLookupContext, LedgerLookupLedger, LedgerLookupSource, RpcRole, lookup_ledger_with_result,
};
use crate::{RpcErrorCode, RpcStatus};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BookChangesTransaction {
    pub txn: STTx,
    pub meta: TxMeta,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BookChangesLedger {
    pub ledger_time: u32,
    pub transactions: Vec<BookChangesTransaction>,
}

pub trait BookChangesSource: LedgerLookupSource {
    fn book_changes_ledger(&self, ledger: LedgerLookupLedger) -> Option<BookChangesLedger>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BookChangesRequest<'a> {
    pub params: &'a JsonValue,
    pub api_version: u32,
    pub role: RpcRole,
}

#[derive(Debug, Clone)]
struct BookChangeAggregate {
    volume_a: STAmount,
    volume_b: STAmount,
    high: STAmount,
    low: STAmount,
    open: STAmount,
    close: STAmount,
    domain: Option<Uint256>,
}

fn ensure_object(value: &mut JsonValue) -> &mut BTreeMap<String, JsonValue> {
    if !matches!(value, JsonValue::Object(_)) {
        *value = JsonValue::Object(BTreeMap::new());
    }

    let JsonValue::Object(object) = value else {
        unreachable!("json value should be an object");
    };
    object
}

fn issue_label(amount: &STAmount) -> String {
    if amount.native() {
        "XRP_drops".to_owned()
    } else {
        asset_to_string(amount.asset())
    }
}

fn amount_text(amount: &STAmount) -> String {
    if amount.native() {
        amount.xrp().drops().to_string()
    } else if amount.holds_mpt_issue() {
        amount.mpt().value().to_string()
    } else {
        amount.iou().to_string()
    }
}

fn delta_amount(final_amount: &STAmount, previous_amount: &STAmount) -> STAmount {
    match final_amount.asset() {
        Asset::Issue(_) if final_amount.native() => {
            STAmount::from(final_amount.xrp() - previous_amount.xrp())
        }
        Asset::Issue(issue) => {
            let delta = final_amount
                .iou()
                .checked_sub(previous_amount.iou())
                .expect("book_changes IOU delta should stay representable");
            STAmount::from_iou_amount(sf_generic(), delta, issue)
        }
        Asset::MPTIssue(issue) => {
            let delta = final_amount.mpt() - previous_amount.mpt();
            STAmount::from_mpt_amount(sf_generic(), delta, issue)
        }
    }
}

fn positive_amount(amount: STAmount) -> STAmount {
    match amount.asset() {
        Asset::Issue(_) if amount.native() => STAmount::from(protocol::XRPAmount::from_drops(
            i64::try_from(amount.xrp().drops().unsigned_abs())
                .expect("book_changes XRP volume should stay representable"),
        )),
        Asset::Issue(issue) => {
            let absolute = i64::try_from(amount.iou().mantissa().unsigned_abs())
                .expect("book_changes IOU volume should stay representable");
            STAmount::from_iou_amount(
                sf_generic(),
                protocol::IOUAmount::from_parts(absolute, amount.iou().exponent())
                    .expect("book_changes IOU volume should stay representable"),
                issue,
            )
        }
        Asset::MPTIssue(issue) => {
            let absolute = i64::try_from(amount.mpt().value().unsigned_abs())
                .expect("book_changes MPT volume should stay representable");
            STAmount::from_mpt_amount(
                sf_generic(),
                protocol::MPTAmount::from_value(absolute),
                issue,
            )
        }
    }
}

fn add_amount(target: &mut STAmount, value: STAmount) {
    let updated = match target.asset() {
        Asset::Issue(_) if target.native() => STAmount::from(target.xrp() + value.xrp()),
        Asset::Issue(issue) => {
            let sum = target
                .iou()
                .checked_add(value.iou())
                .expect("book_changes IOU volume should stay representable");
            STAmount::from_iou_amount(sf_generic(), sum, issue)
        }
        Asset::MPTIssue(issue) => {
            STAmount::from_mpt_amount(sf_generic(), target.mpt() + value.mpt(), issue)
        }
    };
    *target = updated;
}

fn scale_up(value: &mut u128, exponent: &mut i32) {
    while *value < 1_000_000_000_000_000u128 {
        *value = value
            .checked_mul(10)
            .expect("STAmount mantissa scaling should not overflow");
        *exponent -= 1;
    }
}

fn divide_st_amount(num: &STAmount, den: &STAmount) -> STAmount {
    if den.signum() == 0 || num.signum() == 0 {
        return STAmount::new_with_asset(sf_generic(), no_issue(), 0, 0, false);
    }

    let mut num_val = u128::from(num.mantissa());
    let mut den_val = u128::from(den.mantissa());
    let mut num_offset = num.exponent();
    let mut den_offset = den.exponent();

    if num.native() || num.holds_mpt_issue() {
        scale_up(&mut num_val, &mut num_offset);
    }

    if den.native() || den.holds_mpt_issue() {
        scale_up(&mut den_val, &mut den_offset);
    }

    let mantissa = num_val
        .checked_mul(100_000_000_000_000_000u128)
        .expect("book_changes rate multiplication should not overflow")
        / den_val
        + 5;
    let negative = num.negative() != den.negative();
    let exponent = num_offset - den_offset - 17;

    STAmount::new_with_asset(
        sf_generic(),
        no_issue(),
        u64::try_from(mantissa).expect("book_changes rate mantissa should fit in u64"),
        exponent,
        negative,
    )
}

fn compute_book_changes(transactions: &[BookChangesTransaction]) -> JsonValue {
    let mut tally: BTreeMap<String, BookChangeAggregate> = BTreeMap::new();

    for tx in transactions {
        if !tx
            .txn
            .is_field_present(get_field_by_symbol("sfTransactionType"))
        {
            continue;
        }

        let mut offer_cancel = None;
        let tx_type = TxType::from_u16(
            tx.txn
                .get_field_u16(get_field_by_symbol("sfTransactionType")),
        );
        if matches!(tx_type, TxType::OFFER_CANCEL | TxType::OFFER_CREATE)
            && tx
                .txn
                .is_field_present(get_field_by_symbol("sfOfferSequence"))
        {
            offer_cancel = Some(tx.txn.get_field_u32(get_field_by_symbol("sfOfferSequence")));
        }

        for node in tx.meta.get_nodes().iter() {
            let meta_type = node.fname();
            let node_type = node.get_field_u16(get_field_by_symbol("sfLedgerEntryType"));

            if node_type != LedgerEntryType::Offer.code()
                || meta_type == get_field_by_symbol("sfCreatedNode")
            {
                continue;
            }

            if !node.is_field_present(get_field_by_symbol("sfFinalFields"))
                || !node.is_field_present(get_field_by_symbol("sfPreviousFields"))
            {
                continue;
            }

            let final_fields = node.get_field_object(get_field_by_symbol("sfFinalFields"));
            let previous_fields = node.get_field_object(get_field_by_symbol("sfPreviousFields"));

            if !final_fields.is_field_present(get_field_by_symbol("sfTakerGets"))
                || !final_fields.is_field_present(get_field_by_symbol("sfTakerPays"))
                || !previous_fields.is_field_present(get_field_by_symbol("sfTakerGets"))
                || !previous_fields.is_field_present(get_field_by_symbol("sfTakerPays"))
            {
                continue;
            }

            if meta_type == get_field_by_symbol("sfDeletedNode")
                && offer_cancel.is_some()
                && final_fields.get_field_u32(get_field_by_symbol("sfSequence"))
                    == offer_cancel.expect("offer_cancel presence already checked")
            {
                continue;
            }

            let delta_gets = delta_amount(
                &final_fields.get_field_amount(get_field_by_symbol("sfTakerGets")),
                &previous_fields.get_field_amount(get_field_by_symbol("sfTakerGets")),
            );
            let delta_pays = delta_amount(
                &final_fields.get_field_amount(get_field_by_symbol("sfTakerPays")),
                &previous_fields.get_field_amount(get_field_by_symbol("sfTakerPays")),
            );

            let get_issue = asset_to_string(delta_gets.asset());
            let pay_issue = asset_to_string(delta_pays.asset());
            let noswap = if delta_gets.native() {
                true
            } else if delta_pays.native() {
                false
            } else {
                get_issue < pay_issue
            };

            let mut first = if noswap {
                delta_gets.clone()
            } else {
                delta_pays.clone()
            };
            let mut second = if noswap {
                delta_pays.clone()
            } else {
                delta_gets.clone()
            };

            if second.signum() == 0 {
                continue;
            }

            let rate = divide_st_amount(&first, &second);

            first = positive_amount(first);
            second = positive_amount(second);

            let key = if noswap {
                format!("{get_issue}|{pay_issue}")
            } else {
                format!("{pay_issue}|{get_issue}")
            };
            let domain = final_fields
                .is_field_present(get_field_by_symbol("sfDomainID"))
                .then(|| final_fields.get_field_h256(get_field_by_symbol("sfDomainID")));

            match tally.get_mut(&key) {
                Some(entry) => {
                    add_amount(&mut entry.volume_a, first);
                    add_amount(&mut entry.volume_b, second);
                    if entry.high < rate {
                        entry.high = rate.clone();
                    }
                    if entry.low > rate {
                        entry.low = rate.clone();
                    }
                    entry.close = rate.clone();
                    entry.domain = domain;
                }
                None => {
                    tally.insert(
                        key,
                        BookChangeAggregate {
                            volume_a: first,
                            volume_b: second,
                            high: rate.clone(),
                            low: rate.clone(),
                            open: rate.clone(),
                            close: rate,
                            domain,
                        },
                    );
                }
            }
        }
    }

    let mut changes = Vec::with_capacity(tally.len());
    for aggregate in tally.into_values() {
        let mut change = BTreeMap::new();
        change.insert(
            "currency_a".to_owned(),
            JsonValue::String(issue_label(&aggregate.volume_a)),
        );
        change.insert(
            "currency_b".to_owned(),
            JsonValue::String(issue_label(&aggregate.volume_b)),
        );
        change.insert(
            "volume_a".to_owned(),
            JsonValue::String(amount_text(&aggregate.volume_a)),
        );
        change.insert(
            "volume_b".to_owned(),
            JsonValue::String(amount_text(&aggregate.volume_b)),
        );
        change.insert(
            "high".to_owned(),
            JsonValue::String(amount_text(&aggregate.high)),
        );
        change.insert(
            "low".to_owned(),
            JsonValue::String(amount_text(&aggregate.low)),
        );
        change.insert(
            "open".to_owned(),
            JsonValue::String(amount_text(&aggregate.open)),
        );
        change.insert(
            "close".to_owned(),
            JsonValue::String(amount_text(&aggregate.close)),
        );
        if let Some(domain) = aggregate.domain {
            change.insert("domain".to_owned(), JsonValue::String(domain.to_string()));
        }
        changes.push(JsonValue::Object(change));
    }

    JsonValue::Array(changes)
}

pub fn do_book_changes<S: BookChangesSource>(
    request: &BookChangesRequest<'_>,
    source: &S,
) -> JsonValue {
    tracing::trace!(target: "rpc", method = "book_changes", "book_changes query");
    let context = LedgerLookupContext {
        params: request.params,
        source,
        api_version: request.api_version,
        role: request.role,
    };

    let (ledger, mut result) = match lookup_ledger_with_result(&context) {
        Ok(result) => result,
        Err(status) => {
            let mut error = JsonValue::Object(BTreeMap::new());
            status.inject(&mut error);
            return error;
        }
    };

    let Some(snapshot) = source.book_changes_ledger(ledger) else {
        let mut error = JsonValue::Object(BTreeMap::new());
        RpcStatus::new(RpcErrorCode::LedgerNotFound).inject(&mut error);
        return error;
    };

    let object = ensure_object(&mut result);
    object.insert(
        "type".to_owned(),
        JsonValue::String("bookChanges".to_owned()),
    );
    object.insert(
        "ledger_time".to_owned(),
        JsonValue::Unsigned(u64::from(snapshot.ledger_time)),
    );
    object.insert(
        "changes".to_owned(),
        compute_book_changes(&snapshot.transactions),
    );

    result
}
