use ledger::{Ledger, LedgerFillOptions};
use protocol::{JsonValue, STTx, TxType, get_field_by_symbol};

use crate::AppLedgerFill;

pub(crate) fn insert_owner_funds(tx_json: &mut JsonValue, fill: &AppLedgerFill<'_>, txn: &STTx) {
    if !fill.options.contains(LedgerFillOptions::OWNER_FUNDS)
        || txn.get_txn_type() != TxType::OFFER_CREATE
    {
        return;
    }

    let Some(context) = fill.context else {
        return;
    };

    let account = txn.get_account_id(get_field_by_symbol("sfAccount"));
    let amount = txn.get_field_amount(get_field_by_symbol("sfTakerGets"));
    if account == amount.issue().issuer() {
        return;
    }

    let Some(owner_funds) = context.account_funds(fill.ledger, account, &amount) else {
        return;
    };

    let JsonValue::Object(object) = tx_json else {
        return;
    };
    object.insert("owner_funds".to_owned(), JsonValue::String(owner_funds));
}

#[allow(dead_code)]
fn _keep_types_used(_ledger: &Ledger) {}
