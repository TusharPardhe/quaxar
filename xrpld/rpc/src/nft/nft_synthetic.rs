//! Shared NFT synthetic metadata shaping for read-only RPC transaction paths.

use std::collections::BTreeSet;

use protocol::{JsonValue, LedgerEntryType, STTx, StBase, TxMeta, TxType, get_field_by_symbol};

pub fn insert_nft_synthetic_in_json(response: &mut JsonValue, txn: &STTx, meta: &TxMeta) {
    if !matches!(
        txn.get_txn_type(),
        TxType::NFTOKEN_MINT | TxType::NFTOKEN_ACCEPT_OFFER | TxType::NFTOKEN_CANCEL_OFFER
    ) || !matches!(meta.get_result_ter(), protocol::Ter::TES_SUCCESS)
    {
        return;
    }

    let JsonValue::Object(root) = response else {
        return;
    };
    let Some(meta_json) = root.get_mut("meta") else {
        return;
    };
    let JsonValue::Object(meta_object) = meta_json else {
        return;
    };

    let mut previous = BTreeSet::new();
    let mut final_ids = Vec::new();

    for node in meta.get_nodes().iter() {
        if node.get_field_u16(get_field_by_symbol("sfLedgerEntryType"))
            != LedgerEntryType::NFTokenPage.code()
        {
            continue;
        }

        match node.fname() {
            field if field == get_field_by_symbol("sfCreatedNode") => {
                let new_fields = node.get_field_object(get_field_by_symbol("sfNewFields"));
                if !new_fields.is_field_present(get_field_by_symbol("sfNFTokens")) {
                    continue;
                }
                let nfts = new_fields.get_field_array(get_field_by_symbol("sfNFTokens"));
                final_ids.extend(
                    nfts.iter()
                        .map(|nft| nft.get_field_h256(get_field_by_symbol("sfNFTokenID"))),
                );
            }
            field if field == get_field_by_symbol("sfModifiedNode") => {
                let previous_fields =
                    node.get_field_object(get_field_by_symbol("sfPreviousFields"));
                if previous_fields.is_field_present(get_field_by_symbol("sfNFTokens")) {
                    let nfts = previous_fields.get_field_array(get_field_by_symbol("sfNFTokens"));
                    previous.extend(
                        nfts.iter()
                            .map(|nft| nft.get_field_h256(get_field_by_symbol("sfNFTokenID"))),
                    );
                }

                let final_fields = node.get_field_object(get_field_by_symbol("sfFinalFields"));
                if !final_fields.is_field_present(get_field_by_symbol("sfNFTokens")) {
                    continue;
                }
                let nfts = final_fields.get_field_array(get_field_by_symbol("sfNFTokens"));
                final_ids.extend(
                    nfts.iter()
                        .map(|nft| nft.get_field_h256(get_field_by_symbol("sfNFTokenID"))),
                );
            }
            _ => {}
        }
    }

    match txn.get_txn_type() {
        TxType::NFTOKEN_MINT => {
            let minted = final_ids.into_iter().find(|id| !previous.contains(id));
            if let Some(id) = minted {
                meta_object.insert("nftoken_id".to_owned(), JsonValue::String(id.to_string()));
            }
        }
        TxType::NFTOKEN_ACCEPT_OFFER => {
            let accepted = final_ids.into_iter().find(|id| !previous.contains(id));
            if let Some(id) = accepted {
                meta_object.insert("nftoken_id".to_owned(), JsonValue::String(id.to_string()));
            }
        }
        TxType::NFTOKEN_CANCEL_OFFER => {
            let cancelled = previous
                .into_iter()
                .filter(|id| !final_ids.contains(id))
                .map(|id| JsonValue::String(id.to_string()))
                .collect::<Vec<_>>();
            if !cancelled.is_empty() {
                meta_object.insert("nftoken_ids".to_owned(), JsonValue::Array(cancelled));
            }
        }
        _ => {}
    }
}
