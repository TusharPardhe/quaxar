//! NFT synthetic JSON helpers from `xrpl/protocol/NFTSyntheticSerializer.*`.

use std::collections::BTreeMap;

use crate::{JsonValue, STTx, TxMeta, insert_nftoken_id, insert_nftoken_offer_id};

pub fn insert_nft_synthetic_in_json(
    response: &mut JsonValue,
    transaction: Option<&STTx>,
    transaction_meta: &TxMeta,
) {
    let JsonValue::Object(object) = response else {
        *response = JsonValue::Object(BTreeMap::new());
        insert_nft_synthetic_in_json(response, transaction, transaction_meta);
        return;
    };

    let meta = object
        .entry("meta".to_owned())
        .or_insert_with(|| JsonValue::Object(BTreeMap::new()));
    insert_nftoken_id(meta, transaction, transaction_meta);
    insert_nftoken_offer_id(meta, transaction, transaction_meta);
}
