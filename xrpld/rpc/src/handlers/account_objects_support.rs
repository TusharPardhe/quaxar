//! Shared read-only traversal helpers for account-owned objects and NFT pages.
//!
//! This stays narrow on purpose: explicit read/successor traits over the
//! current ledger surface, no hidden application/runtime owner, and direct reference
//! parity for the account object and NFT page traversal rules that can be
//! expressed with the current Rust protocol/ledger/keylet stack.

#![allow(clippy::collapsible_if, clippy::manual_contains, dead_code)]

use basics::base_uint::{Uint160, Uint256};
use ledger::Ledger;
use protocol::{
    AccountID, JsonOptions, JsonValue, Keylet, LedgerEntryType, NFTokenPage, STLedgerEntry,
    STObject, StBase, child_keylet, get_field_by_symbol, nft_page_keylet, nft_page_max_keylet,
    nft_page_min_keylet, owner_dir_keylet, page_keylet, to_base58,
};
use shamap::traversal::TraversalError;
use std::sync::Arc;

const NFT_PAGE_MASK_HEX: &str = "0000000000000000000000000000000000000000ffffffffffffffffffffffff";

fn raw_account_id(account: AccountID) -> Uint160 {
    Uint160::from_slice(account.data()).expect("account width")
}

fn sf_indexes() -> &'static protocol::SField {
    get_field_by_symbol("sfIndexes")
}

fn sf_index_next() -> &'static protocol::SField {
    get_field_by_symbol("sfIndexNext")
}

fn nft_next_page_min(page: &STLedgerEntry) -> Option<Uint256> {
    NFTokenPage::new(Arc::new(page.clone()))
        .ok()
        .and_then(|page| page.get_next_page_min())
}

fn sf_nftokens() -> &'static protocol::SField {
    get_field_by_symbol("sfNFTokens")
}

fn sf_nftoken_id() -> &'static protocol::SField {
    get_field_by_symbol("sfNFTokenID")
}

fn nft_page_mask() -> Uint256 {
    Uint256::from_hex(NFT_PAGE_MASK_HEX).expect("expected nft page mask should parse")
}

fn nft_flags(id: Uint256) -> u16 {
    let mut flags = [0u8; 2];
    flags.copy_from_slice(&id.data()[0..2]);
    u16::from_be_bytes(flags)
}

fn nft_transfer_fee(id: Uint256) -> u16 {
    let mut fee = [0u8; 2];
    fee.copy_from_slice(&id.data()[2..4]);
    u16::from_be_bytes(fee)
}

fn nft_serial(id: Uint256) -> u32 {
    let mut serial = [0u8; 4];
    serial.copy_from_slice(&id.data()[28..32]);
    u32::from_be_bytes(serial)
}

fn nft_taxon(id: Uint256) -> u32 {
    let mut taxon = [0u8; 4];
    taxon.copy_from_slice(&id.data()[24..28]);
    let taxon = u32::from_be_bytes(taxon);
    let mixed = 384_160_001u32
        .wrapping_mul(nft_serial(id))
        .wrapping_add(2_459);
    taxon ^ mixed
}

fn nft_issuer(id: Uint256) -> AccountID {
    AccountID::from_slice(&id.data()[4..24]).expect("nft issuer width should match")
}

fn is_nft_marker_before_candidate(marker: Uint256, candidate: Uint256) -> bool {
    let masked_marker = marker & nft_page_mask();
    let masked_candidate = candidate & nft_page_mask();

    if masked_candidate < masked_marker {
        return true;
    }

    if masked_candidate == masked_marker && candidate < marker {
        return true;
    }

    false
}

fn nft_page_json(page: &STLedgerEntry) -> JsonValue {
    NFTokenPage::new(Arc::new(page.clone()))
        .map(|page| page.as_st_ledger_entry().json(JsonOptions::NONE))
        .unwrap_or_else(|_| page.json(JsonOptions::NONE))
}

fn nft_token_json(token: &STObject) -> JsonValue {
    let mut json = token.json(JsonOptions::NONE);
    let JsonValue::Object(fields) = &mut json else {
        return json;
    };

    let nftoken_id = token.get_field_h256(sf_nftoken_id());
    fields.insert(
        "Flags".to_owned(),
        JsonValue::Unsigned(u64::from(nft_flags(nftoken_id))),
    );
    fields.insert(
        "Issuer".to_owned(),
        JsonValue::String(to_base58(nft_issuer(nftoken_id))),
    );
    fields.insert(
        "NFTokenTaxon".to_owned(),
        JsonValue::Unsigned(u64::from(nft_taxon(nftoken_id))),
    );
    fields.insert(
        "nft_serial".to_owned(),
        JsonValue::Unsigned(u64::from(nft_serial(nftoken_id))),
    );

    let transfer_fee = nft_transfer_fee(nftoken_id);
    if transfer_fee != 0 {
        fields.insert(
            "TransferFee".to_owned(),
            JsonValue::Unsigned(u64::from(transfer_fee)),
        );
    }

    json
}

pub trait AccountObjectsView {
    fn read_entry(&self, keylet: Keylet) -> Result<Option<STLedgerEntry>, TraversalError>;
    fn exists_entry(&self, keylet: Keylet) -> Result<bool, TraversalError> {
        Ok(self.read_entry(keylet)?.is_some())
    }
    fn succ_key(
        &self,
        key: Uint256,
        last: Option<Uint256>,
    ) -> Result<Option<Uint256>, TraversalError>;
}

impl AccountObjectsView for Ledger {
    fn read_entry(&self, keylet: Keylet) -> Result<Option<STLedgerEntry>, TraversalError> {
        Ledger::read(self, keylet)
    }

    fn succ_key(
        &self,
        key: Uint256,
        last: Option<Uint256>,
    ) -> Result<Option<Uint256>, TraversalError> {
        Ledger::succ(self, key, last)
    }
}

pub fn account_objects_valid_type(entry_type: LedgerEntryType) -> bool {
    !matches!(
        entry_type,
        LedgerEntryType::Amendments
            | LedgerEntryType::DirectoryNode
            | LedgerEntryType::FeeSettings
            | LedgerEntryType::LedgerHashes
            | LedgerEntryType::NegativeUnl
    )
}

pub fn account_objects_owner_root(account: AccountID) -> Keylet {
    owner_dir_keylet(raw_account_id(account))
}

pub fn account_objects_nft_min(account: AccountID) -> Keylet {
    nft_page_min_keylet(raw_account_id(account))
}

pub fn account_objects_nft_max(account: AccountID) -> Keylet {
    nft_page_max_keylet(raw_account_id(account))
}

#[derive(Debug, Clone)]
pub enum AccountTraversalError {
    InvalidMarker,
    Traversal(TraversalError),
}

impl From<TraversalError> for AccountTraversalError {
    fn from(value: TraversalError) -> Self {
        Self::Traversal(value)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AccountObjectsMarker {
    NftPage {
        page: Uint256,
    },
    Directory {
        dir_index: Uint256,
        entry_index: Uint256,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct AccountObjectsTraversal {
    pub items: Vec<JsonValue>,
    pub marker: Option<AccountObjectsMarker>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct AccountNftsTraversal {
    pub items: Vec<JsonValue>,
    pub marker: Option<Uint256>,
}

pub fn collect_account_nfts<V: AccountObjectsView>(
    view: &V,
    account: AccountID,
    marker: Uint256,
    limit: u32,
) -> Result<AccountNftsTraversal, AccountTraversalError> {
    let first = nft_page_keylet(account_objects_nft_min(account), marker);
    let last = account_objects_nft_max(account);
    let mut current_key = view
        .succ_key(first.key, Some(last.key.next()))?
        .unwrap_or(last.key);
    let mut current_page =
        view.read_entry(Keylet::new(LedgerEntryType::NFTokenPage, current_key))?;

    let mut items = Vec::new();
    let mut count = 0u32;
    let mut past_marker = marker.is_zero();
    let mut marker_found = false;

    while let Some(page) = current_page {
        let tokens = page.get_field_array(sf_nftokens());
        for token in tokens.iter() {
            let nftoken_id = token.get_field_h256(sf_nftoken_id());

            if !past_marker {
                if is_nft_marker_before_candidate(marker, nftoken_id) {
                    continue;
                }

                if nftoken_id == marker {
                    marker_found = true;
                    continue;
                }
            }

            if marker != Uint256::zero() && !marker_found {
                return Err(AccountTraversalError::InvalidMarker);
            }

            past_marker = true;
            items.push(nft_token_json(token));

            count = count.saturating_add(1);
            if count == limit {
                if let Some(next_page_min) = nft_next_page_min(&page) {
                    return Ok(AccountNftsTraversal {
                        items,
                        marker: Some(next_page_min),
                    });
                }

                return Ok(AccountNftsTraversal {
                    items,
                    marker: None,
                });
            }
        }

        if let Some(next_page_min) = nft_next_page_min(&page) {
            current_key = next_page_min;
            current_page =
                view.read_entry(Keylet::new(LedgerEntryType::NFTokenPage, current_key))?;
        } else {
            current_page = None;
        }
    }

    if marker != Uint256::zero() && !marker_found {
        return Err(AccountTraversalError::InvalidMarker);
    }

    Ok(AccountNftsTraversal {
        items,
        marker: None,
    })
}

pub fn collect_account_objects<V: AccountObjectsView>(
    view: &V,
    account: AccountID,
    type_filter: Option<&[LedgerEntryType]>,
    dir_index: Uint256,
    entry_index: Uint256,
    limit: u32,
) -> Result<AccountObjectsTraversal, AccountTraversalError> {
    if dir_index.is_non_zero()
        && !view.exists_entry(Keylet::new(LedgerEntryType::DirectoryNode, dir_index))?
    {
        return Err(AccountTraversalError::InvalidMarker);
    }

    let type_matches = |ledger_type: LedgerEntryType| {
        type_filter
            .map(|filter| filter.iter().any(|candidate| *candidate == ledger_type))
            .unwrap_or(true)
    };

    let mut items = Vec::new();
    let mut mlimit = limit;
    let mut iterate_nft_pages = (type_filter.is_none()
        || type_matches(LedgerEntryType::NFTokenPage))
        && dir_index.is_zero();
    let first_nft_page = account_objects_nft_min(account);

    if iterate_nft_pages && entry_index != Uint256::zero() {
        if first_nft_page.key != (entry_index & !nft_page_mask()) {
            iterate_nft_pages = false;
        }
    }

    if iterate_nft_pages {
        let first = if entry_index.is_zero() {
            first_nft_page
        } else {
            Keylet::new(LedgerEntryType::NFTokenPage, entry_index)
        };
        let last = account_objects_nft_max(account);
        let mut current_key = view
            .succ_key(first.key, Some(last.key.next()))?
            .unwrap_or(last.key);
        let mut current_page =
            view.read_entry(Keylet::new(LedgerEntryType::NFTokenPage, current_key))?;

        while let Some(page) = current_page {
            items.push(nft_page_json(&page));
            mlimit = mlimit.saturating_sub(1);
            if mlimit == 0 {
                if nft_next_page_min(&page).is_some() {
                    return Ok(AccountObjectsTraversal {
                        items,
                        marker: Some(AccountObjectsMarker::NftPage { page: current_key }),
                    });
                }
            }

            if let Some(next_page_min) = nft_next_page_min(&page) {
                current_key = next_page_min;
                current_page =
                    view.read_entry(Keylet::new(LedgerEntryType::NFTokenPage, current_key))?;
            } else {
                current_page = None;
            }
        }

        // Once NFT pages are exhausted the reference code falls back to directory
        // traversal and resets the entry marker.
        if type_filter.is_some_and(|filter| filter == [LedgerEntryType::NFTokenPage]) {
            return Ok(AccountObjectsTraversal {
                items,
                marker: None,
            });
        }
    }

    let root = account_objects_owner_root(account);
    let mut found = false;
    let mut current_dir_index = if dir_index.is_zero() {
        found = true;
        root.key
    } else {
        dir_index
    };

    let mut current_dir: Option<STLedgerEntry> = match view.read_entry(Keylet::new(
        LedgerEntryType::DirectoryNode,
        current_dir_index,
    ))? {
        Some(dir) => Some(dir),
        None => {
            return Ok(AccountObjectsTraversal {
                items,
                marker: None,
            });
        }
    };

    let mut dir_count = 0u32;

    loop {
        let Some(dir) = current_dir.as_ref() else {
            return Ok(AccountObjectsTraversal {
                items,
                marker: None,
            });
        };

        let entries = dir.get_field_v256(sf_indexes());
        let mut remaining: Vec<Uint256> = entries.value().to_vec();

        if !found {
            let Some(pos) = remaining.iter().position(|key| *key == entry_index) else {
                return Err(AccountTraversalError::InvalidMarker);
            };
            found = true;
            remaining = remaining.into_iter().skip(pos).collect();
        }

        if dir_count == mlimit && mlimit < limit {
            if let Some(next_key) = remaining.first().copied() {
                return Ok(AccountObjectsTraversal {
                    items,
                    marker: Some(AccountObjectsMarker::Directory {
                        dir_index: current_dir_index,
                        entry_index: next_key,
                    }),
                });
            }
        }

        for entry in remaining.into_iter() {
            let Some(sle_node) = view.read_entry(child_keylet(entry))? else {
                panic!("xrpl::AccountObjects : missing child entry");
            };

            if type_matches(sle_node.get_type()) {
                items.push(sle_node.json(JsonOptions::NONE));
            }

            dir_count = dir_count.saturating_add(1);
            if dir_count == mlimit {
                if let Some(next_key) = entries
                    .value()
                    .iter()
                    .copied()
                    .skip_while(|candidate| *candidate != entry)
                    .nth(1)
                {
                    return Ok(AccountObjectsTraversal {
                        items,
                        marker: Some(AccountObjectsMarker::Directory {
                            dir_index: current_dir_index,
                            entry_index: next_key,
                        }),
                    });
                }

                break;
            }
        }

        let next_page = dir.get_field_u64(sf_index_next());
        if next_page == 0 {
            return Ok(AccountObjectsTraversal {
                items,
                marker: None,
            });
        }

        current_dir_index = page_keylet(root, next_page).key;
        current_dir = view.read_entry(Keylet::new(
            LedgerEntryType::DirectoryNode,
            current_dir_index,
        ))?;
        if current_dir.is_none() {
            return Ok(AccountObjectsTraversal {
                items,
                marker: None,
            });
        }

        if dir_count == mlimit {
            let Some(next_dir) = current_dir.as_ref() else {
                return Ok(AccountObjectsTraversal {
                    items,
                    marker: None,
                });
            };
            let next_entries = next_dir.get_field_v256(sf_indexes());
            if let Some(first) = next_entries.value().iter().copied().next() {
                return Ok(AccountObjectsTraversal {
                    items,
                    marker: Some(AccountObjectsMarker::Directory {
                        dir_index: current_dir_index,
                        entry_index: first,
                    }),
                });
            }
        }
    }
}
