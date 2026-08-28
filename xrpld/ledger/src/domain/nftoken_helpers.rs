//! the reference implementation parity — NFT page management, token insertion/removal,
//! offer lifecycle, and directory repair.

use crate::views::apply_view::ApplyView;
use crate::views::read_view::{ReadView, ViewError};
use crate::{adjust_owner_count, dir_remove};
use basics::base_uint::{Uint160, Uint256};
use protocol::{
    AccountID, Asset, Keylet, LedgerEntryType, Rules, STAmount, STArray, STLedgerEntry, STObject,
    Ter, account_keylet, get_field_by_symbol, lsfGlobalFreeze, lsfHighFreeze, lsfLowFreeze,
    lsfSellNFToken, nft, nft_buy_offers_keylet, nft_offer_keylet, nft_offer_keylet_for_owner,
    nft_page_keylet, nft_page_max_keylet, nft_page_min_keylet, nft_sell_offers_keylet,
    owner_dir_keylet, tfSellNFToken,
};
use std::sync::Arc;

fn sf(name: &str) -> &'static protocol::SField {
    get_field_by_symbol(name)
}

fn to_uint160(account: AccountID) -> Uint160 {
    Uint160::from_slice(account.data()).expect("account width should match")
}

/// Maximum tokens per NFT page.
pub const DIR_MAX_TOKENS_PER_PAGE: usize = 32;

/// Compare two NFToken IDs for page-sorted ordering.
pub fn compare_tokens(a: &Uint256, b: &Uint256) -> std::cmp::Ordering {
    let mask = protocol::nft_page_mask();
    let a_low = *a & mask;
    let b_low = *b & mask;
    match a_low.cmp(&b_low) {
        std::cmp::Ordering::Equal => a.cmp(b),
        other => other,
    }
}

/// Locate the NFT page containing the given token ID (read-only).
pub fn locate_page(
    view: &dyn ReadView,
    owner: &AccountID,
    id: &Uint256,
) -> Result<Option<Arc<STLedgerEntry>>, ViewError> {
    let first = nft_page_keylet(nft_page_min_keylet(to_uint160(*owner)), *id);
    let last = nft_page_max_keylet(to_uint160(*owner));

    let succ_key = view.succ(first.key, Some(last.key.next()))?;
    let page_key = succ_key.unwrap_or(last.key);
    view.read(Keylet::new(LedgerEntryType::NFTokenPage, page_key))
}

/// Locate the NFT page containing the given token ID (mutable).
pub fn locate_page_mut(
    view: &mut dyn ApplyView,
    owner: &AccountID,
    id: &Uint256,
) -> Result<Option<Arc<STLedgerEntry>>, ViewError> {
    let first = nft_page_keylet(nft_page_min_keylet(to_uint160(*owner)), *id);
    let last = nft_page_max_keylet(to_uint160(*owner));

    let succ_key = view.succ(first.key, Some(last.key.next()))?;
    let page_key = succ_key.unwrap_or(last.key);
    view.peek(Keylet::new(LedgerEntryType::NFTokenPage, page_key))
}

/// Find a specific NFToken in the owner's pages (read-only).
///
pub fn find_token(
    view: &dyn ReadView,
    owner: &AccountID,
    nftoken_id: &Uint256,
) -> Result<Option<STObject>, ViewError> {
    let Some(page) = locate_page(view, owner, nftoken_id)? else {
        return Ok(None);
    };

    let arr = page.get_field_array(sf("sfNFTokens"));
    for t in arr.iter() {
        if t.get_field_h256(sf("sfNFTokenID")) == *nftoken_id {
            return Ok(Some(t.clone()));
        }
    }
    Ok(None)
}

/// Insert a token into the owner's NFT page directory.
///
pub fn insert_token(
    view: &mut dyn ApplyView,
    owner: AccountID,
    nft: STObject,
) -> Result<Ter, ViewError> {
    let nftoken_id = nft.get_field_h256(sf("sfNFTokenID"));

    let last = nft_page_max_keylet(to_uint160(owner));
    let first_kl = nft_page_keylet(nft_page_min_keylet(to_uint160(owner)), nftoken_id);

    let succ_key = view.succ(first_kl.key, Some(last.key.next()))?;
    let page_key = succ_key.unwrap_or(last.key);
    let page_kl = Keylet::new(LedgerEntryType::NFTokenPage, page_key);

    let page = if let Some(p) = view.peek(page_kl)? {
        p
    } else {
        // Create new max page
        let mut new_page = STLedgerEntry::new(last);
        new_page.set_field_array(sf("sfNFTokens"), STArray::new(sf("sfNFTokens")));
        let arc = Arc::new(new_page);
        view.insert(arc.clone())?;
        if let Some(acct) = view.peek(account_keylet(to_uint160(owner)))? {
            adjust_owner_count(view, &acct, 1)?;
        }
        arc
    };

    // Check page capacity
    let arr = page.get_field_array(sf("sfNFTokens"));
    if arr.iter().count() >= DIR_MAX_TOKENS_PER_PAGE {
        return Ok(Ter::TEC_NO_SUITABLE_NFTOKEN_PAGE);
    }

    // Insert and sort
    let mut tokens: Vec<STObject> = arr.iter().cloned().collect();
    tokens.push(nft);
    tokens.sort_by(|a, b| {
        compare_tokens(
            &a.get_field_h256(sf("sfNFTokenID")),
            &b.get_field_h256(sf("sfNFTokenID")),
        )
    });

    let mut new_arr = STArray::new(sf("sfNFTokens"));
    for t in tokens {
        new_arr.push_back(t);
    }

    let mut updated = (*page).clone();
    updated.set_field_array(sf("sfNFTokens"), new_arr);
    view.update(Arc::new(updated))?;

    Ok(Ter::TES_SUCCESS)
}

/// Remove a token from the owner's NFT page directory.
///
pub fn remove_token(
    view: &mut dyn ApplyView,
    owner: &AccountID,
    nftoken_id: &Uint256,
) -> Result<Ter, ViewError> {
    let Some(page) = locate_page_mut(view, owner, nftoken_id)? else {
        return Ok(Ter::TEC_NO_ENTRY);
    };

    let arr = page.get_field_array(sf("sfNFTokens"));
    let tokens: Vec<STObject> = arr.iter().cloned().collect();
    let pos = tokens
        .iter()
        .position(|obj| obj.get_field_h256(sf("sfNFTokenID")) == *nftoken_id);

    let Some(idx) = pos else {
        return Ok(Ter::TEC_NO_ENTRY);
    };

    let mut new_tokens: Vec<STObject> = tokens;
    new_tokens.remove(idx);

    if !new_tokens.is_empty() {
        let mut new_arr = STArray::new(sf("sfNFTokens"));
        for t in new_tokens {
            new_arr.push_back(t);
        }
        let mut updated = (*page).clone();
        updated.set_field_array(sf("sfNFTokens"), new_arr);
        view.update(Arc::new(updated))?;
    } else {
        // Empty page — erase it
        view.erase(page)?;
        if let Some(acct) = view.peek(account_keylet(to_uint160(*owner)))? {
            adjust_owner_count(view, &acct, -1)?;
        }
    }

    Ok(Ter::TES_SUCCESS)
}

/// Delete an NFToken offer from the ledger.
///
pub fn delete_token_offer(
    view: &mut dyn ApplyView,
    offer: Arc<STLedgerEntry>,
) -> Result<bool, ViewError> {
    if offer.get_type() != LedgerEntryType::NFTokenOffer {
        return Ok(false);
    }

    let owner = offer.get_account_id(sf("sfOwner"));
    let owner_node = offer.get_field_u64(sf("sfOwnerNode"));

    if !dir_remove(
        view,
        &owner_dir_keylet(to_uint160(owner)),
        owner_node,
        *offer.key(),
        false,
    )? {
        return Ok(false);
    }

    let nftoken_id = offer.get_field_h256(sf("sfNFTokenID"));
    let offer_node = offer.get_field_u64(sf("sfNFTokenOfferNode"));
    let dir_kl = if offer.is_flag(lsfSellNFToken) {
        nft_sell_offers_keylet(nftoken_id)
    } else {
        nft_buy_offers_keylet(nftoken_id)
    };

    if !dir_remove(view, &dir_kl, offer_node, *offer.key(), false)? {
        return Ok(false);
    }

    if let Some(acct) = view.peek(account_keylet(to_uint160(owner)))? {
        adjust_owner_count(view, &acct, -1)?;
    }

    view.erase(offer)?;
    Ok(true)
}

/// Preflight checks for NFToken offer creation.
///
pub fn token_offer_create_preflight(
    acct_id: &AccountID,
    amount: &STAmount,
    dest: Option<&AccountID>,
    expiration: Option<u32>,
    nft_flags: u16,
    _rules: &Rules,
    owner: Option<&AccountID>,
    tx_flags: u32,
) -> Ter {
    if amount.negative() {
        return Ter::TEM_BAD_AMOUNT;
    }

    if !amount.native() {
        if (nft_flags & nft::FLAG_ONLY_XRP) != 0 {
            return Ter::TEM_BAD_AMOUNT;
        }
        if amount.mantissa() == 0 {
            return Ter::TEM_BAD_AMOUNT;
        }
    }

    let is_sell = (tx_flags & tfSellNFToken) != 0;
    if !is_sell && amount.mantissa() == 0 {
        return Ter::TEM_BAD_AMOUNT;
    }

    if let Some(exp) = expiration
        && exp == 0
    {
        return Ter::TEM_BAD_EXPIRATION;
    }

    if owner.is_some() == is_sell {
        return Ter::TEM_MALFORMED;
    }

    if let Some(o) = owner
        && o == acct_id
    {
        return Ter::TEM_MALFORMED;
    }

    if let Some(d) = dest
        && d == acct_id
    {
        return Ter::TEM_MALFORMED;
    }

    Ter::TES_SUCCESS
}

/// Find a token and its containing page (mutable).
///
pub fn find_token_and_page(
    view: &mut dyn ApplyView,
    owner: &AccountID,
    nftoken_id: &Uint256,
) -> Result<Option<(STObject, Arc<STLedgerEntry>)>, ViewError> {
    let Some(page) = locate_page_mut(view, owner, nftoken_id)? else {
        return Ok(None);
    };

    let arr = page.get_field_array(sf("sfNFTokens"));
    for t in arr.iter() {
        if t.get_field_h256(sf("sfNFTokenID")) == *nftoken_id {
            return Ok(Some((t.clone(), page)));
        }
    }
    Ok(None)
}

/// Change the URI of an existing NFToken.
///
pub fn change_token_uri(
    view: &mut dyn ApplyView,
    owner: &AccountID,
    nftoken_id: &Uint256,
    uri: Option<&[u8]>,
) -> Result<Ter, ViewError> {
    let Some(page) = locate_page_mut(view, owner, nftoken_id)? else {
        return Ok(Ter::TEC_INTERNAL);
    };

    let arr = page.get_field_array(sf("sfNFTokens"));
    let tokens: Vec<STObject> = arr.iter().cloned().collect();
    let Some(idx) = tokens
        .iter()
        .position(|obj| obj.get_field_h256(sf("sfNFTokenID")) == *nftoken_id)
    else {
        return Ok(Ter::TEC_INTERNAL);
    };

    let mut new_tokens = tokens;
    if let Some(uri_data) = uri {
        new_tokens[idx].set_field_vl(sf("sfURI"), uri_data);
    } else if new_tokens[idx].is_field_present(sf("sfURI")) {
        new_tokens[idx].make_field_absent(sf("sfURI"));
    }

    let mut new_arr = STArray::new(sf("sfNFTokens"));
    for t in new_tokens {
        new_arr.push_back(t);
    }
    let mut updated = (*page).clone();
    updated.set_field_array(sf("sfNFTokens"), new_arr);
    view.update(Arc::new(updated))?;
    Ok(Ter::TES_SUCCESS)
}

/// Remove token offers from a directory with a limit.
///
pub fn remove_token_offers_with_limit(
    view: &mut dyn ApplyView,
    directory: &Keylet,
    max_deletable: usize,
) -> Result<usize, ViewError> {
    if max_deletable == 0 {
        return Ok(0);
    }

    let mut deleted = 0usize;
    let mut page_index: Option<u64> = Some(0);

    while let Some(idx) = page_index {
        let page_kl = protocol::page_keylet(*directory, idx);
        let Some(page) = view.peek(page_kl)? else {
            break;
        };

        page_index = if page.is_field_present(sf("sfIndexNext")) {
            Some(page.get_field_u64(sf("sfIndexNext")))
        } else {
            None
        };

        let offer_indexes = page.get_field_v256(sf("sfIndexes"));
        let values: Vec<Uint256> = offer_indexes.value().to_vec();

        for offer_key in values.iter().rev() {
            if let Some(offer) = view.peek(nft_offer_keylet(*offer_key))? {
                if !delete_token_offer(view, offer)? {
                    return Err(ViewError::Conversion(format!(
                        "NFToken offer {offer_key} cannot be deleted"
                    )));
                }
                deleted += 1;
            }
            if deleted >= max_deletable {
                break;
            }
        }

        if deleted >= max_deletable {
            break;
        }
    }

    Ok(deleted)
}

/// Check trustline authorization for NFToken operations.
///
pub fn check_trustline_authorized(
    view: &dyn ReadView,
    id: &AccountID,
    issue: &protocol::Issue,
) -> Result<Ter, ViewError> {
    if issue.native() {
        return Ok(Ter::TES_SUCCESS);
    }

    if !view
        .rules()
        .enabled(&protocol::fix_enforce_nftoken_trustline_v2())
    {
        return Ok(Ter::TES_SUCCESS);
    }

    let issuer_account = view.read(account_keylet(to_uint160(issue.issuer())))?;
    let Some(issuer_sle) = issuer_account else {
        return Ok(Ter::TEC_NO_ISSUER);
    };

    if issue.issuer() == *id {
        return Ok(Ter::TES_SUCCESS);
    }

    if issuer_sle.is_flag(protocol::lsfRequireAuth) {
        let trust_line = view.read(protocol::line(*id, issue.issuer(), issue.currency))?;

        let Some(tl) = trust_line else {
            return Ok(Ter::TEC_NO_LINE);
        };

        let auth_flag = if *id > issue.issuer() {
            protocol::lsfLowAuth
        } else {
            protocol::lsfHighAuth
        };

        if !tl.is_flag(auth_flag) {
            return Ok(Ter::TEC_NO_AUTH);
        }
    }

    Ok(Ter::TES_SUCCESS)
}

/// Check trustline deep freeze for NFToken operations.
///
pub fn check_trustline_deep_frozen(
    view: &dyn ReadView,
    id: &AccountID,
    issue: &protocol::Issue,
) -> Result<Ter, ViewError> {
    if issue.native() {
        return Ok(Ter::TES_SUCCESS);
    }

    if !view.rules().enabled(&protocol::feature_deep_freeze()) {
        return Ok(Ter::TES_SUCCESS);
    }

    if view
        .read(account_keylet(to_uint160(issue.issuer())))?
        .is_none()
    {
        return Ok(Ter::TEC_NO_ISSUER);
    }

    if issue.issuer() == *id {
        return Ok(Ter::TES_SUCCESS);
    }

    let trust_line = view.read(protocol::line(*id, issue.issuer(), issue.currency))?;

    let Some(tl) = trust_line else {
        return Ok(Ter::TES_SUCCESS);
    };

    let deep_frozen =
        (tl.get_flags() & (protocol::lsfLowDeepFreeze | protocol::lsfHighDeepFreeze)) != 0;

    if deep_frozen {
        return Ok(Ter::TEC_FROZEN);
    }

    Ok(Ter::TES_SUCCESS)
}

fn nft_iou_frozen(
    view: &dyn ReadView,
    account: AccountID,
    issue: protocol::Issue,
) -> Result<bool, ViewError> {
    if issue.native() || account == issue.account {
        return Ok(false);
    }
    if view
        .read(account_keylet(to_uint160(issue.account)))?
        .is_some_and(|issuer| issuer.is_flag(lsfGlobalFreeze))
    {
        return Ok(true);
    }
    Ok(view
        .read(protocol::line(account, issue.account, issue.currency))?
        .is_some_and(|line| {
            line.is_flag(if account > issue.account {
                lsfLowFreeze
            } else {
                lsfHighFreeze
            })
        }))
}

fn nft_account_funds_positive(
    view: &dyn ReadView,
    account: AccountID,
    amount: &STAmount,
) -> Result<bool, ViewError> {
    match amount.asset() {
        Asset::Issue(issue) if issue.native() => {
            let Some(root) = view.read(account_keylet(to_uint160(account)))? else {
                return Ok(false);
            };
            let balance = view
                .balance_hook_iou(
                    account,
                    protocol::xrp_account(),
                    root.get_field_amount(sf("sfBalance")),
                )
                .xrp()
                .drops();
            let reserve = if crate::is_pseudo_account(&root) {
                0
            } else {
                let owner_count = view
                    .owner_count_hook(account, crate::OwnerCounts::from_sle(&root))
                    .count();
                crate::effective_account_reserve_with_owner_count(
                    view.fees(),
                    &root,
                    owner_count,
                    0,
                    0,
                ) as i64
            };
            Ok(balance.saturating_sub(reserve).max(0) > 0)
        }
        Asset::Issue(issue) if issue.account == account => Ok(true),
        Asset::Issue(issue) => {
            if nft_iou_frozen(view, account, issue)? {
                return Ok(false);
            }
            let Some(line) = view.read(protocol::line(account, issue.account, issue.currency))?
            else {
                return Ok(false);
            };
            let mut balance = line.get_field_amount(sf("sfBalance"));
            if account > issue.account {
                balance.negate();
            }
            Ok(balance.signum() > 0)
        }
        // NFToken offer Amount fields do not support MPT issues in the pinned
        // transaction formats.  Keep malformed bypasses fail-closed.
        Asset::MPTIssue(_) => Ok(false),
    }
}

/// Preclaim checks for NFToken offer creation.
///
pub fn token_offer_create_preclaim(
    view: &dyn ReadView,
    acct_id: &AccountID,
    nft_issuer: &AccountID,
    amount: &STAmount,
    dest: Option<&AccountID>,
    nft_flags: u16,
    xfer_fee: u16,
    owner: Option<&AccountID>,
    tx_flags: u32,
) -> Result<Ter, ViewError> {
    // Exact tokenOfferCreatePreclaim ordering from rippled.  Before
    // NFTokenMintOffer the royalty issuer always needed a trust line; after
    // it, an IOU issuer accepting its own currency is exempt.
    if ((nft_flags & nft::FLAG_CREATE_TRUST_LINES) == 0) && !amount.native() && (xfer_fee != 0) {
        if !view.exists(account_keylet(to_uint160(*nft_issuer)))? {
            return Ok(Ter::TEC_NO_ISSUER);
        }
        let Asset::Issue(issue) = amount.asset() else {
            return Ok(Ter::TEC_INTERNAL);
        };
        let royalty_line_required = !view
            .rules()
            .enabled(&protocol::feature_nftoken_mint_offer())
            || *nft_issuer != issue.account;
        if royalty_line_required
            && !view.exists(protocol::line(*nft_issuer, issue.account, issue.currency))?
        {
            return Ok(Ter::TEC_NO_LINE);
        }
        if nft_iou_frozen(view, *nft_issuer, issue)? {
            return Ok(Ter::TEC_FROZEN);
        }
    }

    // Non-transferable NFT check
    if nft_issuer != acct_id && ((nft_flags & nft::FLAG_TRANSFERABLE) == 0) {
        let Some(root) = view.read(account_keylet(to_uint160(*nft_issuer)))? else {
            return Ok(Ter::TEC_INTERNAL);
        };
        if root.is_field_present(sf("sfNFTokenMinter")) {
            let minter = root.get_account_id(sf("sfNFTokenMinter"));
            if minter != *acct_id {
                return Ok(Ter::TEF_NFTOKEN_IS_NOT_TRANSFERABLE);
            }
        } else {
            return Ok(Ter::TEF_NFTOKEN_IS_NOT_TRANSFERABLE);
        }
    }

    if let Asset::Issue(issue) = amount.asset()
        && !issue.native()
        && nft_iou_frozen(view, *acct_id, issue)?
    {
        return Ok(Ter::TEC_FROZEN);
    }

    // A buy offer only needs some currently spendable funds.  It does not
    // reserve the full offer amount; the offer may later become unfunded.
    if (tx_flags & tfSellNFToken) == 0 && !nft_account_funds_positive(view, *acct_id, amount)? {
        return Ok(Ter::TEC_UNFUNDED_OFFER);
    }

    // Check destination exists and allows incoming offers
    if let Some(d) = dest {
        let Some(sle_dst) = view.read(account_keylet(to_uint160(*d)))? else {
            return Ok(Ter::TEC_NO_DST);
        };
        if sle_dst.is_flag(protocol::lsfDisallowIncomingNFTokenOffer) {
            return Ok(Ter::TEC_NO_PERMISSION);
        }
    }

    // Check owner allows incoming offers
    if let Some(o) = owner {
        let Some(sle_owner) = view.read(account_keylet(to_uint160(*o)))? else {
            return Ok(Ter::TEC_NO_TARGET);
        };
        if sle_owner.is_flag(protocol::lsfDisallowIncomingNFTokenOffer) {
            return Ok(Ter::TEC_NO_PERMISSION);
        }
    }

    if view
        .rules()
        .enabled(&protocol::fix_enforce_nftoken_trustline_v2())
        && !amount.native()
    {
        let Asset::Issue(issue) = amount.asset() else {
            return Ok(Ter::TEC_INTERNAL);
        };
        let auth = check_trustline_authorized(view, acct_id, &issue)?;
        if auth != Ter::TES_SUCCESS {
            return Ok(auth);
        }
    }

    Ok(Ter::TES_SUCCESS)
}

/// Apply logic for NFToken offer creation.
///
pub fn token_offer_create_apply(
    view: &mut dyn ApplyView,
    acct_id: &AccountID,
    amount: &STAmount,
    dest: Option<&AccountID>,
    expiration: Option<u32>,
    seq: u32,
    nftoken_id: &Uint256,
    pre_fee_balance_drops: Option<i64>,
    tx_flags: u32,
) -> Result<Ter, ViewError> {
    // Match rippled's tokenOfferCreateApply: assess the next owner-reserve
    // requirement against the account balance before the transaction fee is
    // charged and before either directory can be mutated.
    let account = view.read(account_keylet(to_uint160(*acct_id)))?;
    let Some(account) = account else {
        return Ok(Ter::TEF_BAD_LEDGER);
    };
    let prior_balance = pre_fee_balance_drops
        .unwrap_or_else(|| account.get_field_amount(sf("sfBalance")).xrp().drops());
    let reserve = crate::effective_account_reserve(view.fees(), &account, 1, 0);
    if prior_balance < reserve as i64 {
        return Ok(Ter::TEC_INSUFFICIENT_RESERVE);
    }

    let offer_id = nft_offer_keylet_for_owner(to_uint160(*acct_id), seq);

    // Add to owner directory
    let owner_node = crate::dir_insert(
        view,
        &owner_dir_keylet(to_uint160(*acct_id)),
        offer_id.key,
        &crate::describe_owner_dir(*acct_id),
    )?;
    let Some(owner_node_val) = owner_node else {
        return Ok(Ter::TEC_DIR_FULL);
    };

    let is_sell = (tx_flags & tfSellNFToken) != 0;

    // Add to token's buy or sell offer directory
    let offer_dir = if is_sell {
        nft_sell_offers_keylet(*nftoken_id)
    } else {
        nft_buy_offers_keylet(*nftoken_id)
    };
    let offer_node = crate::dir_insert(view, &offer_dir, offer_id.key, &|sle| {
        sle.set_field_u32(
            sf("sfFlags"),
            if is_sell {
                protocol::lsfNFTokenSellOffers
            } else {
                protocol::lsfNFTokenBuyOffers
            },
        );
        sle.set_field_h256(sf("sfNFTokenID"), *nftoken_id);
    })?;
    let Some(offer_node_val) = offer_node else {
        return Ok(Ter::TEC_DIR_FULL);
    };

    // Create the offer SLE
    let mut offer = STLedgerEntry::new(offer_id);
    offer.set_account_id(sf("sfOwner"), *acct_id);
    offer.set_field_h256(sf("sfNFTokenID"), *nftoken_id);
    offer.set_field_amount(sf("sfAmount"), amount.clone());
    let mut flags = 0u32;
    if is_sell {
        flags |= lsfSellNFToken;
    }
    offer.set_field_u32(sf("sfFlags"), flags);
    offer.set_field_u64(sf("sfOwnerNode"), owner_node_val);
    offer.set_field_u64(sf("sfNFTokenOfferNode"), offer_node_val);

    if let Some(exp) = expiration {
        offer.set_field_u32(sf("sfExpiration"), exp);
    }
    if let Some(d) = dest {
        offer.set_account_id(sf("sfDestination"), *d);
    }

    view.insert(Arc::new(offer))?;

    // Update owner count
    if let Some(acct) = view.peek(account_keylet(to_uint160(*acct_id)))? {
        adjust_owner_count(view, &acct, 1)?;
    }

    Ok(Ter::TES_SUCCESS)
}

/// Repair broken NFToken directory page links.
///
pub fn repair_nftoken_directory_links(
    view: &mut dyn ApplyView,
    owner: &AccountID,
) -> Result<bool, ViewError> {
    let last = nft_page_max_keylet(to_uint160(*owner));
    let first_kl = nft_page_min_keylet(to_uint160(*owner));

    let succ_key = view.succ(first_kl.key, Some(last.key.next()))?;
    let page_key = succ_key.unwrap_or(last.key);

    let Some(mut page) = view.peek(Keylet::new(LedgerEntryType::NFTokenPage, page_key))? else {
        return Ok(false);
    };

    let mut did_repair = false;

    if page_key == last.key {
        // Single page — should have no links
        let next_present = page.is_field_present(sf("sfNextPageMin"));
        let prev_present = page.is_field_present(sf("sfPreviousPageMin"));
        if next_present || prev_present {
            did_repair = true;
            let mut updated = (*page).clone();
            if prev_present {
                updated.make_field_absent(sf("sfPreviousPageMin"));
            }
            if next_present {
                updated.make_field_absent(sf("sfNextPageMin"));
            }
            view.update(Arc::new(updated))?;
        }
        return Ok(did_repair);
    }

    // First page should not have a previous link
    if page.is_field_present(sf("sfPreviousPageMin")) {
        did_repair = true;
        let mut updated = (*page).clone();
        updated.make_field_absent(sf("sfPreviousPageMin"));
        page = Arc::new(updated);
        view.update(Arc::clone(&page))?;
    }

    // Walk physical ledger order, repairing both links between every pair.
    // This deliberately does not trust the links being repaired.
    let mut next_page = None;
    loop {
        let next_key = view
            .succ(page.key().next(), Some(last.key.next()))?
            .unwrap_or(last.key);
        let Some(mut next) = view.peek(Keylet::new(LedgerEntryType::NFTokenPage, next_key))? else {
            break;
        };

        if !page.is_field_present(sf("sfNextPageMin"))
            || page.get_field_h256(sf("sfNextPageMin")) != *next.key()
        {
            did_repair = true;
            let mut updated = (*page).clone();
            updated.set_field_h256(sf("sfNextPageMin"), *next.key());
            page = Arc::new(updated);
            view.update(Arc::clone(&page))?;
        }

        if !next.is_field_present(sf("sfPreviousPageMin"))
            || next.get_field_h256(sf("sfPreviousPageMin")) != *page.key()
        {
            did_repair = true;
            let mut updated = (*next).clone();
            updated.set_field_h256(sf("sfPreviousPageMin"), *page.key());
            next = Arc::new(updated);
            view.update(Arc::clone(&next))?;
        }

        if *next.key() == last.key {
            next_page = Some(next);
            break;
        }
        page = next;
    }

    if next_page.is_none() {
        // The physical last page has a non-canonical key. Move its contents
        // to the canonical max key and repair the predecessor's forward link.
        did_repair = true;
        let mut canonical_last = STLedgerEntry::new(last);
        canonical_last.set_field_array(sf("sfNFTokens"), page.get_field_array(sf("sfNFTokens")));

        if page.is_field_present(sf("sfPreviousPageMin")) {
            let previous_key = page.get_field_h256(sf("sfPreviousPageMin"));
            canonical_last.set_field_h256(sf("sfPreviousPageMin"), previous_key);
            let Some(previous) =
                view.peek(Keylet::new(LedgerEntryType::NFTokenPage, previous_key))?
            else {
                return Err(ViewError::Conversion(format!(
                    "NFTokenPage directory for {owner} cannot be repaired: missing previous page"
                )));
            };
            let mut updated = (*previous).clone();
            updated.set_field_h256(sf("sfNextPageMin"), last.key);
            view.update(Arc::new(updated))?;
        }
        view.erase(page)?;
        view.insert(Arc::new(canonical_last))?;
        return Ok(did_repair);
    }

    let last_page = next_page.expect("checked above");
    if last_page.is_field_present(sf("sfNextPageMin")) {
        did_repair = true;
        let mut updated = (*last_page).clone();
        updated.make_field_absent(sf("sfNextPageMin"));
        view.update(Arc::new(updated))?;
    }

    Ok(did_repair)
}
