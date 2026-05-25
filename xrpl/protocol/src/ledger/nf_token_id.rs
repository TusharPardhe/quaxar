//! NFToken synthetic-id helpers from `xrpl/protocol/NFTokenID.*`.

use std::collections::BTreeMap;

use basics::base_uint::Uint256;

use crate::{JsonValue, STTx, StBase, TxMeta, TxType, get_field_by_symbol, is_tes_success};

pub fn can_have_nf_token_id(transaction: Option<&STTx>, transaction_meta: &TxMeta) -> bool {
    let Some(transaction) = transaction else {
        return false;
    };

    match transaction.get_txn_type() {
        value
            if value == TxType::NFTOKEN_MINT
                || value == TxType::NFTOKEN_ACCEPT_OFFER
                || value == TxType::NFTOKEN_CANCEL_OFFER => {}
        _ => return false,
    }

    is_tes_success(transaction_meta.get_result_ter())
}

pub fn get_nftoken_id_from_page(transaction_meta: &TxMeta) -> Option<Uint256> {
    let mut prev_ids = Vec::new();
    let mut final_ids = Vec::new();

    for node in transaction_meta.get_nodes().iter() {
        if node.get_field_u16(get_field_by_symbol("sfLedgerEntryType"))
            != crate::LedgerEntryType::NFTokenPage as u16
        {
            continue;
        }

        if node.fname() == get_field_by_symbol("sfCreatedNode") {
            let added = node
                .get_field_object(get_field_by_symbol("sfNewFields"))
                .get_field_array(get_field_by_symbol("sfNFTokens"));
            final_ids.extend(
                added
                    .iter()
                    .map(|nft| nft.get_field_h256(get_field_by_symbol("sfNFTokenID"))),
            );
        } else if node.fname() == get_field_by_symbol("sfModifiedNode") {
            let previous_fields = node.get_field_object(get_field_by_symbol("sfPreviousFields"));
            if !previous_fields.is_field_present(get_field_by_symbol("sfNFTokens")) {
                continue;
            }

            prev_ids.extend(
                previous_fields
                    .get_field_array(get_field_by_symbol("sfNFTokens"))
                    .iter()
                    .map(|nft| nft.get_field_h256(get_field_by_symbol("sfNFTokenID"))),
            );
            final_ids.extend(
                node.get_field_object(get_field_by_symbol("sfFinalFields"))
                    .get_field_array(get_field_by_symbol("sfNFTokens"))
                    .iter()
                    .map(|nft| nft.get_field_h256(get_field_by_symbol("sfNFTokenID"))),
            );
        }
    }

    (final_ids.len() == prev_ids.len() + 1)
        .then(|| {
            final_ids
                .iter()
                .zip(prev_ids.iter().chain(std::iter::repeat(&Uint256::zero())))
                .find_map(|(final_id, prev_id)| (final_id != prev_id).then_some(*final_id))
        })
        .flatten()
}

pub fn get_nftoken_id_from_deleted_offer(transaction_meta: &TxMeta) -> Vec<Uint256> {
    let mut result = Vec::new();

    for node in transaction_meta.get_nodes().iter() {
        if node.get_field_u16(get_field_by_symbol("sfLedgerEntryType"))
            != crate::LedgerEntryType::NFTokenOffer as u16
            || node.fname() != get_field_by_symbol("sfDeletedNode")
        {
            continue;
        }

        result.push(
            node.get_field_object(get_field_by_symbol("sfFinalFields"))
                .get_field_h256(get_field_by_symbol("sfNFTokenID")),
        );
    }

    result.sort();
    result.dedup();
    result
}

pub fn insert_nftoken_id(
    response: &mut JsonValue,
    transaction: Option<&STTx>,
    transaction_meta: &TxMeta,
) {
    if !can_have_nf_token_id(transaction, transaction_meta) {
        return;
    }

    let Some(transaction) = transaction else {
        return;
    };

    let JsonValue::Object(response) = response else {
        *response = JsonValue::Object(BTreeMap::new());
        insert_nftoken_id(response, Some(transaction), transaction_meta);
        return;
    };

    match transaction.get_txn_type() {
        value if value == TxType::NFTOKEN_MINT => {
            if let Some(result) = get_nftoken_id_from_page(transaction_meta) {
                response.insert(
                    "nftoken_id".to_owned(),
                    JsonValue::String(result.to_string()),
                );
            }
        }
        value if value == TxType::NFTOKEN_ACCEPT_OFFER => {
            if let Some(result) = get_nftoken_id_from_deleted_offer(transaction_meta).first() {
                response.insert(
                    "nftoken_id".to_owned(),
                    JsonValue::String(result.to_string()),
                );
            }
        }
        value if value == TxType::NFTOKEN_CANCEL_OFFER => {
            response.insert(
                "nftoken_ids".to_owned(),
                JsonValue::Array(
                    get_nftoken_id_from_deleted_offer(transaction_meta)
                        .into_iter()
                        .map(|value| JsonValue::String(value.to_string()))
                        .collect(),
                ),
            );
        }
        _ => {}
    }
}
