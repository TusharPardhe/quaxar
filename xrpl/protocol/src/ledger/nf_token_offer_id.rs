//! NFToken synthetic offer-id helpers from `xrpl/protocol/NFTokenOfferID.*`.

use std::collections::BTreeMap;

use basics::base_uint::Uint256;

use crate::{JsonValue, STTx, StBase, TxMeta, TxType, get_field_by_symbol, is_tes_success};

pub fn can_have_nf_token_offer_id(transaction: Option<&STTx>, transaction_meta: &TxMeta) -> bool {
    let Some(transaction) = transaction else {
        return false;
    };

    let tx_type = transaction.get_txn_type();
    if !((tx_type == TxType::NFTOKEN_MINT
        && transaction.is_field_present(get_field_by_symbol("sfAmount")))
        || tx_type == TxType::NFTOKEN_CREATE_OFFER)
    {
        return false;
    }

    is_tes_success(transaction_meta.get_result_ter())
}

pub fn get_offer_id_from_created_offer(transaction_meta: &TxMeta) -> Option<Uint256> {
    transaction_meta.get_nodes().iter().find_map(|node| {
        (node.get_field_u16(get_field_by_symbol("sfLedgerEntryType"))
            == crate::LedgerEntryType::NFTokenOffer as u16
            && node.fname() == get_field_by_symbol("sfCreatedNode"))
        .then(|| node.get_field_h256(get_field_by_symbol("sfLedgerIndex")))
    })
}

pub fn insert_nftoken_offer_id(
    response: &mut JsonValue,
    transaction: Option<&STTx>,
    transaction_meta: &TxMeta,
) {
    if !can_have_nf_token_offer_id(transaction, transaction_meta) {
        return;
    }

    let JsonValue::Object(response) = response else {
        *response = JsonValue::Object(BTreeMap::new());
        insert_nftoken_offer_id(response, transaction, transaction_meta);
        return;
    };

    if let Some(result) = get_offer_id_from_created_offer(transaction_meta) {
        response.insert("offer_id".to_owned(), JsonValue::String(result.to_string()));
    }
}
