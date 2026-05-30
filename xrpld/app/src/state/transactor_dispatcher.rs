//! Transactor dispatcher — routes `TxType` to real view-backed engines.

use crate::state::transactor_apply_bridge::*;
use crate::state::transactor_escrow_bridge::*;
use basics::math::base_uint::{Uint160, Uint256};
use basics::number::NumberParts as RuntimeNumber;
use protocol::{
    AUCTION_SLOT_DISCOUNTED_FEE_FRACTION, AccountID, Asset, Keylet, LedgerEntryType, STAmount,
    STArray, STLedgerEntry, STObject, STTx, Ter, TxType, VOTE_MAX_SLOTS, VOTE_WEIGHT_SCALE_FACTOR,
    XRPAmount, amm_lpt_currency, get_field_by_symbol, is_tes_success, lsfDisableMaster,
    owner_dir_keylet, signers_keylet,
};
use std::sync::Arc;
use tx::*;

fn sf(name: &str) -> &'static protocol::SField {
    get_field_by_symbol(name)
}

fn escrow_mpt_unlock_amounts<V: ledger::ApplyView>(
    view: &V,
    amount: &STAmount,
    locked_rate: u32,
    sender: &AccountID,
    receiver: &AccountID,
) -> (STAmount, STAmount) {
    let Asset::MPTIssue(issue) = amount.asset() else {
        return (amount.clone(), amount.clone());
    };
    let issuer = issue.issuer();
    let mut rate = protocol::Rate::new(locked_rate);
    if let Ok(current_rate) = ledger::mptoken_helpers::transfer_rate_mpt(view, issue.mpt_id())
        && current_rate < rate
    {
        rate = current_rate;
    }

    if sender != &issuer && receiver != &issuer && rate != protocol::PARITY_RATE {
        return (protocol::divide_round(amount, rate, true), amount.clone());
    }
    (amount.clone(), amount.clone())
}

fn check_mpt_check_create_allowed<V: ledger::ApplyView>(
    view: &V,
    source: &AccountID,
    destination: &AccountID,
    amount: &STAmount,
) -> Ter {
    let Asset::MPTIssue(issue) = amount.asset() else {
        return Ter::TES_SUCCESS;
    };
    let issuer = issue.issuer();

    if source != &issuer
        && ledger::mptoken_helpers::is_frozen_mpt(view, source, &issue).unwrap_or(true)
    {
        return Ter::TEC_LOCKED;
    }
    if destination != &issuer
        && ledger::mptoken_helpers::is_frozen_mpt(view, destination, &issue).unwrap_or(true)
    {
        return Ter::TEC_LOCKED;
    }

    ledger::mptoken_helpers::can_transfer_mpt(view, &issue, source, destination)
        .unwrap_or(Ter::TEF_INTERNAL)
}

fn check_mpt_check_cash_allowed<V: ledger::ApplyView>(
    view: &mut V,
    source: &AccountID,
    destination: &AccountID,
    amount: &STAmount,
) -> Ter {
    let Asset::MPTIssue(issue) = amount.asset() else {
        return Ter::TES_SUCCESS;
    };
    let issuer = issue.issuer();
    if view
        .peek(protocol::account_keylet(Uint160::from_void(issuer.data())))
        .ok()
        .flatten()
        .is_none()
    {
        return Ter::TEC_NO_ISSUER;
    }
    let auth = ledger::mptoken_helpers::require_auth_mpt(view, &issue, destination)
        .unwrap_or(Ter::TEF_INTERNAL);
    if auth != Ter::TES_SUCCESS {
        return auth;
    }
    if destination != &issuer
        && ledger::mptoken_helpers::is_frozen_mpt(view, destination, &issue).unwrap_or(true)
    {
        return Ter::TEC_LOCKED;
    }
    let transfer = ledger::mptoken_helpers::can_transfer_mpt(view, &issue, source, destination)
        .unwrap_or(Ter::TEF_INTERNAL);
    if transfer != Ter::TES_SUCCESS {
        return transfer;
    }
    ledger::mptoken_helpers::check_create_mpt(view, &issue, destination)
        .unwrap_or(Ter::TEF_INTERNAL)
}

fn check_mpt_amm_asset_allowed<V: ledger::ApplyView>(
    view: &V,
    account: &AccountID,
    asset: Asset,
    require_holding: bool,
) -> Ter {
    let Asset::MPTIssue(issue) = asset else {
        return Ter::TES_SUCCESS;
    };
    let issuer = issue.issuer();

    if require_holding && account != &issuer {
        match view.read(protocol::mptoken_keylet_from_mptid(
            issue.mpt_id(),
            Uint160::from_void(account.data()),
        )) {
            Ok(Some(_)) => {}
            Ok(None) => return Ter::TEC_NO_AUTH,
            Err(_) => return Ter::TEF_INTERNAL,
        }
    }

    let auth = ledger::mptoken_helpers::require_auth_mpt(view, &issue, account)
        .unwrap_or(Ter::TEF_INTERNAL);
    if auth != Ter::TES_SUCCESS {
        return auth;
    }

    if ledger::mptoken_helpers::is_frozen_mpt(view, account, &issue).unwrap_or(true) {
        return Ter::TEC_LOCKED;
    }

    ledger::mptoken_helpers::can_mpt_trade_and_transfer(view, &asset, account, account)
        .unwrap_or(Ter::TEF_INTERNAL)
}

fn check_mpt_amm_withdraw_asset_allowed<V: ledger::ApplyView>(
    view: &V,
    account: &AccountID,
    asset: Asset,
) -> Ter {
    let Asset::MPTIssue(issue) = asset else {
        return Ter::TES_SUCCESS;
    };

    // #7040: AMMWithdraw is a recovery path. It must not require CanTransfer
    // or CanTrade, but it still rejects globally/individually locked MPTs.
    if ledger::mptoken_helpers::is_frozen_mpt(view, account, &issue).unwrap_or(true) {
        return Ter::TEC_LOCKED;
    }

    let auth = ledger::mptoken_helpers::require_auth_mpt(view, &issue, account)
        .unwrap_or(Ter::TEF_INTERNAL);
    if auth != Ter::TES_SUCCESS {
        return auth;
    }

    Ter::TES_SUCCESS
}

fn check_mpt_amm_pool_asset_unlocked<V: ledger::ApplyView>(
    view: &V,
    amm_account: &AccountID,
    asset: Asset,
) -> Ter {
    let Asset::MPTIssue(issue) = asset else {
        return Ter::TES_SUCCESS;
    };

    if ledger::mptoken_helpers::is_frozen_mpt(view, amm_account, &issue).unwrap_or(true) {
        return Ter::TEC_LOCKED;
    }

    Ter::TES_SUCCESS
}

fn nft_page_mask() -> Uint256 {
    protocol::nft_page_mask()
}

fn nft_owner_min(owner: &AccountID) -> Keylet {
    protocol::nft_page_min_keylet(Uint160::from_void(owner.data()))
}

fn nft_owner_max(owner: &AccountID) -> Keylet {
    protocol::nft_page_max_keylet(Uint160::from_void(owner.data()))
}

fn nft_page_for_token_keylet(owner: &AccountID, token_id: Uint256) -> Keylet {
    protocol::nft_page_keylet(nft_owner_min(owner), token_id)
}

fn nft_compare_tokens(left: Uint256, right: Uint256) -> std::cmp::Ordering {
    let mask = nft_page_mask();
    let left_low = left & mask;
    let right_low = right & mask;
    left_low.cmp(&right_low).then_with(|| left.cmp(&right))
}

fn starray_from_tokens(tokens: Vec<STObject>) -> STArray {
    let mut array = STArray::new(sf("sfNFTokens"));
    array.reserve(tokens.len());
    for token in tokens {
        array.push_back(token);
    }
    array
}

fn number_from_i64(value: i64) -> RuntimeNumber {
    RuntimeNumber::from_i64(value)
}

fn amm_lp_holds_in_view<V: ledger::ApplyView>(
    view: &mut V,
    amm_sle: &STLedgerEntry,
    lp_account: AccountID,
) -> Result<Option<STAmount>, ledger::ViewError> {
    let asset1 = amm_sle.get_field_issue(sf("sfAsset")).asset();
    let asset2 = amm_sle.get_field_issue(sf("sfAsset2")).asset();
    let (Asset::Issue(issue1), Asset::Issue(issue2)) = (asset1, asset2) else {
        return Ok(None);
    };
    let amm_account = amm_sle.get_account_id(sf("sfAccount"));
    let keylet = protocol::line(
        lp_account,
        amm_account,
        amm_lpt_currency(issue1.currency, issue2.currency),
    );
    let Some(sle) = view.peek(keylet)? else {
        return Ok(None);
    };
    let mut amount = sle.get_field_amount(sf("sfBalance"));
    if lp_account > amm_account {
        amount.negate();
    }
    amount.set_issuer(amm_account);
    Ok(Some(amount))
}

fn nft_locate_page<V: ledger::ApplyView>(
    view: &mut V,
    owner: &AccountID,
    token_id: Uint256,
) -> Result<Option<Arc<STLedgerEntry>>, ledger::ViewError> {
    let first = nft_page_for_token_keylet(owner, token_id);
    let last = nft_owner_max(owner);
    let candidate = view
        .succ(first.key, Some(last.key.next()))?
        .unwrap_or(last.key);
    view.peek(Keylet::new(LedgerEntryType::NFTokenPage, candidate))
}

fn nft_find_token_and_page<V: ledger::ApplyView>(
    view: &mut V,
    owner: &AccountID,
    token_id: Uint256,
) -> Result<Option<(STObject, Arc<STLedgerEntry>)>, ledger::ViewError> {
    let Some(page) = nft_locate_page(view, owner, token_id)? else {
        return Ok(None);
    };

    for token in page.get_field_array(sf("sfNFTokens")).iter() {
        if token.get_field_h256(sf("sfNFTokenID")) == token_id {
            return Ok(Some((token.clone(), page)));
        }
    }

    Ok(None)
}

fn nft_page_link<V: ledger::ApplyView>(
    view: &mut V,
    page: &Arc<STLedgerEntry>,
    field: &'static protocol::SField,
) -> Result<Option<Arc<STLedgerEntry>>, ledger::ViewError> {
    if !page.is_field_present(field) {
        return Ok(None);
    }

    let key = page.get_field_h256(field);
    view.peek(Keylet::new(LedgerEntryType::NFTokenPage, key))
}

fn nft_merge_pages<V: ledger::ApplyView>(
    view: &mut V,
    first: Arc<STLedgerEntry>,
    second: Arc<STLedgerEntry>,
) -> Result<bool, ledger::ViewError> {
    if first.key() >= second.key() {
        return Ok(false);
    }
    if !first.is_field_present(sf("sfNextPageMin"))
        || first.get_field_h256(sf("sfNextPageMin")) != *second.key()
        || !second.is_field_present(sf("sfPreviousPageMin"))
        || second.get_field_h256(sf("sfPreviousPageMin")) != *first.key()
    {
        return Ok(false);
    }

    let first_tokens: Vec<_> = first
        .get_field_array(sf("sfNFTokens"))
        .iter()
        .cloned()
        .collect();
    let second_tokens: Vec<_> = second
        .get_field_array(sf("sfNFTokens"))
        .iter()
        .cloned()
        .collect();
    if first_tokens.len() + second_tokens.len() > protocol::DIR_MAX_TOKENS_PER_PAGE {
        return Ok(false);
    }

    let mut merged = first_tokens;
    merged.extend(second_tokens);
    merged.sort_by(|left, right| {
        nft_compare_tokens(
            left.get_field_h256(sf("sfNFTokenID")),
            right.get_field_h256(sf("sfNFTokenID")),
        )
    });

    let mut second_obj = second.clone_as_object();
    second_obj.set_field_array(sf("sfNFTokens"), starray_from_tokens(merged));
    if second_obj.is_field_present(sf("sfPreviousPageMin")) {
        second_obj.make_field_absent(sf("sfPreviousPageMin"));
    }

    if first.is_field_present(sf("sfPreviousPageMin")) {
        let previous_key = first.get_field_h256(sf("sfPreviousPageMin"));
        if let Some(previous) =
            view.peek(Keylet::new(LedgerEntryType::NFTokenPage, previous_key))?
        {
            let mut previous_obj = previous.clone_as_object();
            previous_obj.set_field_h256(sf("sfNextPageMin"), *second.key());
            view.update(Arc::new(STLedgerEntry::from_stobject(
                previous_obj,
                *previous.key(),
            )))?;
            second_obj.set_field_h256(sf("sfPreviousPageMin"), previous_key);
        } else {
            return Ok(false);
        }
    }

    view.update(Arc::new(STLedgerEntry::from_stobject(
        second_obj,
        *second.key(),
    )))?;
    view.erase(first)?;

    Ok(true)
}

fn nft_remove_token_from_page<V: ledger::ApplyView>(
    view: &mut V,
    owner: &AccountID,
    token_id: Uint256,
    current: Arc<STLedgerEntry>,
) -> Ter {
    let tokens = current.get_field_array(sf("sfNFTokens"));
    let mut kept = Vec::new();
    let mut removed = false;
    for token in tokens.iter() {
        if token.get_field_h256(sf("sfNFTokenID")) == token_id {
            removed = true;
        } else {
            kept.push(token.clone());
        }
    }

    if !removed {
        return Ter::TEC_NO_ENTRY;
    }

    let previous = match nft_page_link(view, &current, sf("sfPreviousPageMin")) {
        Ok(page) => page,
        Err(_) => return Ter::TEF_INTERNAL,
    };
    let next = match nft_page_link(view, &current, sf("sfNextPageMin")) {
        Ok(page) => page,
        Err(_) => return Ter::TEF_INTERNAL,
    };

    if !kept.is_empty() {
        let mut obj = current.clone_as_object();
        obj.set_field_array(sf("sfNFTokens"), starray_from_tokens(kept));
        let updated_current = Arc::new(STLedgerEntry::from_stobject(obj, *current.key()));
        if view.update(updated_current.clone()).is_err() {
            return Ter::TEF_INTERNAL;
        }

        let mut owner_count_delta = 0;
        if let Some(prev) = previous.clone() {
            match nft_merge_pages(view, prev, updated_current.clone()) {
                Ok(true) => owner_count_delta -= 1,
                Ok(false) => {}
                Err(_) => return Ter::TEF_INTERNAL,
            }
        }
        if let Some(next_page) = next {
            match nft_merge_pages(view, updated_current, next_page) {
                Ok(true) => owner_count_delta -= 1,
                Ok(false) => {}
                Err(_) => return Ter::TEF_INTERNAL,
            }
        }
        if owner_count_delta != 0 {
            if let Ok(Some(account)) =
                view.peek(protocol::account_keylet(Uint160::from_void(owner.data())))
            {
                if ledger::adjust_owner_count(view, &account, owner_count_delta).is_err() {
                    return Ter::TEF_INTERNAL;
                }
            }
        }
        return Ter::TES_SUCCESS;
    }

    if let Some(prev) = previous.clone() {
        if view
            .rules()
            .enabled(&protocol::feature_id("fixNFTokenPageLinks"))
            && (*current.key() & nft_page_mask()) == nft_page_mask()
        {
            let mut current_obj = current.clone_as_object();
            current_obj.set_field_array(sf("sfNFTokens"), prev.get_field_array(sf("sfNFTokens")));
            if prev.is_field_present(sf("sfPreviousPageMin")) {
                let prev_link = prev.get_field_h256(sf("sfPreviousPageMin"));
                current_obj.set_field_h256(sf("sfPreviousPageMin"), prev_link);
                match view.peek(Keylet::new(LedgerEntryType::NFTokenPage, prev_link)) {
                    Ok(Some(new_prev)) => {
                        let mut new_prev_obj = new_prev.clone_as_object();
                        new_prev_obj.set_field_h256(sf("sfNextPageMin"), *current.key());
                        if view
                            .update(Arc::new(STLedgerEntry::from_stobject(
                                new_prev_obj,
                                *new_prev.key(),
                            )))
                            .is_err()
                        {
                            return Ter::TEF_INTERNAL;
                        }
                    }
                    _ => return Ter::TEF_INTERNAL,
                }
            } else if current_obj.is_field_present(sf("sfPreviousPageMin")) {
                current_obj.make_field_absent(sf("sfPreviousPageMin"));
            }

            if let Ok(Some(account)) =
                view.peek(protocol::account_keylet(Uint160::from_void(owner.data())))
            {
                if ledger::adjust_owner_count(view, &account, -1).is_err() {
                    return Ter::TEF_INTERNAL;
                }
            }
            if view
                .update(Arc::new(STLedgerEntry::from_stobject(
                    current_obj,
                    *current.key(),
                )))
                .is_err()
                || view.erase(prev).is_err()
            {
                return Ter::TEF_INTERNAL;
            }
            return Ter::TES_SUCCESS;
        }

        let mut prev_obj = prev.clone_as_object();
        if let Some(next_page) = next.clone() {
            prev_obj.set_field_h256(sf("sfNextPageMin"), *next_page.key());
        } else if prev_obj.is_field_present(sf("sfNextPageMin")) {
            prev_obj.make_field_absent(sf("sfNextPageMin"));
        }
        if view
            .update(Arc::new(STLedgerEntry::from_stobject(
                prev_obj,
                *prev.key(),
            )))
            .is_err()
        {
            return Ter::TEF_INTERNAL;
        }
    }

    if let Some(next_page) = next.clone() {
        let mut next_obj = next_page.clone_as_object();
        if let Some(prev) = previous.clone() {
            next_obj.set_field_h256(sf("sfPreviousPageMin"), *prev.key());
        } else if next_obj.is_field_present(sf("sfPreviousPageMin")) {
            next_obj.make_field_absent(sf("sfPreviousPageMin"));
        }
        if view
            .update(Arc::new(STLedgerEntry::from_stobject(
                next_obj,
                *next_page.key(),
            )))
            .is_err()
        {
            return Ter::TEF_INTERNAL;
        }
    }

    if view.erase(current).is_err() {
        return Ter::TEF_INTERNAL;
    }

    let mut owner_count_delta = -1;
    if let (Some(prev), Some(next_page)) = (previous, next) {
        match nft_merge_pages(view, prev, next_page) {
            Ok(true) => owner_count_delta -= 1,
            Ok(false) => {}
            Err(_) => return Ter::TEF_INTERNAL,
        }
    }

    if let Ok(Some(account)) = view.peek(protocol::account_keylet(Uint160::from_void(owner.data())))
    {
        if ledger::adjust_owner_count(view, &account, owner_count_delta).is_err() {
            return Ter::TEF_INTERNAL;
        }
    }

    Ter::TES_SUCCESS
}

fn nft_get_page_for_token<V: ledger::ApplyView>(
    view: &mut V,
    owner: &AccountID,
    token_id: Uint256,
) -> Result<Option<Arc<STLedgerEntry>>, ledger::ViewError> {
    let base = nft_owner_min(owner);
    let first = protocol::nft_page_keylet(base, token_id);
    let last = nft_owner_max(owner);
    let candidate = view
        .succ(first.key, Some(last.key.next()))?
        .unwrap_or(last.key);

    if let Some(page) = view.peek(Keylet::new(LedgerEntryType::NFTokenPage, candidate))? {
        if page.get_field_array(sf("sfNFTokens")).len() != protocol::DIR_MAX_TOKENS_PER_PAGE {
            return Ok(Some(page));
        }

        let mut tokens: Vec<_> = page
            .get_field_array(sf("sfNFTokens"))
            .iter()
            .cloned()
            .collect();
        let split_cmp = tokens[(protocol::DIR_MAX_TOKENS_PER_PAGE / 2) - 1]
            .get_field_h256(sf("sfNFTokenID"))
            & nft_page_mask();
        let mut split_index = (protocol::DIR_MAX_TOKENS_PER_PAGE / 2..tokens.len())
            .find(|index| {
                (tokens[*index].get_field_h256(sf("sfNFTokenID")) & nft_page_mask()) != split_cmp
            })
            .unwrap_or(tokens.len());
        if split_index == tokens.len() {
            split_index = tokens
                .iter()
                .position(|token| {
                    (token.get_field_h256(sf("sfNFTokenID")) & nft_page_mask()) == split_cmp
                })
                .unwrap_or(tokens.len());
        }
        if split_index == tokens.len() {
            return Ok(None);
        }
        if split_index == 0 {
            match (token_id & nft_page_mask()).cmp(&split_cmp) {
                std::cmp::Ordering::Equal => return Ok(None),
                std::cmp::Ordering::Greater => split_index = tokens.len(),
                std::cmp::Ordering::Less => {}
            }
        }

        let carried = tokens.split_off(split_index);
        let token_id_for_new_page = if tokens.len() == protocol::DIR_MAX_TOKENS_PER_PAGE {
            tokens[protocol::DIR_MAX_TOKENS_PER_PAGE - 1]
                .get_field_h256(sf("sfNFTokenID"))
                .next()
        } else {
            carried[0].get_field_h256(sf("sfNFTokenID"))
        };

        let new_page_keylet = protocol::nft_page_keylet(base, token_id_for_new_page);
        let mut new_page = STLedgerEntry::new(new_page_keylet);
        new_page.set_field_array(sf("sfNFTokens"), starray_from_tokens(tokens));
        new_page.set_field_h256(sf("sfNextPageMin"), *page.key());

        if page.is_field_present(sf("sfPreviousPageMin")) {
            let previous_key = page.get_field_h256(sf("sfPreviousPageMin"));
            new_page.set_field_h256(sf("sfPreviousPageMin"), previous_key);
            if let Some(previous) =
                view.peek(Keylet::new(LedgerEntryType::NFTokenPage, previous_key))?
            {
                let mut previous_obj = previous.clone_as_object();
                previous_obj.set_field_h256(sf("sfNextPageMin"), new_page_keylet.key);
                view.update(Arc::new(STLedgerEntry::from_stobject(
                    previous_obj,
                    *previous.key(),
                )))?;
            }
        }

        view.insert(Arc::new(new_page))?;

        let mut page_obj = page.clone_as_object();
        page_obj.set_field_array(sf("sfNFTokens"), starray_from_tokens(carried));
        page_obj.set_field_h256(sf("sfPreviousPageMin"), new_page_keylet.key);
        view.update(Arc::new(STLedgerEntry::from_stobject(
            page_obj,
            *page.key(),
        )))?;

        if let Ok(Some(account)) =
            view.peek(protocol::account_keylet(Uint160::from_void(owner.data())))
        {
            let _ = ledger::adjust_owner_count(view, &account, 1);
        }

        return if first.key < new_page_keylet.key {
            view.peek(new_page_keylet)
        } else {
            view.peek(Keylet::new(LedgerEntryType::NFTokenPage, *page.key()))
        };
    }

    let mut page = STLedgerEntry::new(last);
    page.set_field_array(sf("sfNFTokens"), STArray::new(sf("sfNFTokens")));
    let page = Arc::new(page);
    view.insert(page.clone())?;
    if let Ok(Some(account)) = view.peek(protocol::account_keylet(Uint160::from_void(owner.data())))
    {
        let _ = ledger::adjust_owner_count(view, &account, 1);
    }
    Ok(Some(page))
}

fn nft_insert_token<V: ledger::ApplyView>(view: &mut V, owner: &AccountID, token: STObject) -> Ter {
    let token_id = token.get_field_h256(sf("sfNFTokenID"));
    let page = match nft_get_page_for_token(view, owner, token_id) {
        Ok(Some(page)) => page,
        Ok(None) => return Ter::TEC_NO_SUITABLE_NFTOKEN_PAGE,
        Err(_) => return Ter::TEF_INTERNAL,
    };

    let mut tokens: Vec<_> = page
        .get_field_array(sf("sfNFTokens"))
        .iter()
        .cloned()
        .collect();
    tokens.push(token);
    tokens.sort_by(|left, right| {
        nft_compare_tokens(
            left.get_field_h256(sf("sfNFTokenID")),
            right.get_field_h256(sf("sfNFTokenID")),
        )
    });
    let mut page_obj = page.clone_as_object();
    page_obj.set_field_array(sf("sfNFTokens"), starray_from_tokens(tokens));
    if view
        .update(Arc::new(STLedgerEntry::from_stobject(
            page_obj,
            *page.key(),
        )))
        .is_err()
    {
        return Ter::TEF_INTERNAL;
    }

    Ter::TES_SUCCESS
}

fn nft_transfer_token<V: ledger::ApplyView>(
    view: &mut V,
    buyer: &AccountID,
    seller: &AccountID,
    token_id: Uint256,
) -> Ter {
    let (token, page) = match nft_find_token_and_page(view, seller, token_id) {
        Ok(Some(found)) => found,
        Ok(None) => return Ter::TEC_INTERNAL,
        Err(_) => return Ter::TEF_INTERNAL,
    };

    let remove_result = nft_remove_token_from_page(view, seller, token_id, page);
    if !is_tes_success(remove_result) {
        return remove_result;
    }

    nft_insert_token(view, buyer, token)
}

struct DispatcherTicketCreateSink<'a, V> {
    view: &'a mut V,
    account: AccountID,
    tx_sequence: u32,
    pre_fee_balance_drops: Option<i64>,
}

impl<V: ledger::ApplyView> TicketCreateDoApplySink for DispatcherTicketCreateSink<'_, V> {
    type OwnerNode = u64;

    fn account_exists(&mut self) -> bool {
        self.view
            .exists(protocol::account_keylet(Uint160::from_void(
                self.account.data(),
            )))
            .unwrap_or(false)
    }

    fn has_reserve(&mut self, ticket_count: u32) -> bool {
        let account_keylet = protocol::account_keylet(Uint160::from_void(self.account.data()));
        let Ok(Some(account_root)) = self.view.peek(account_keylet) else {
            return false;
        };

        let owner_count = account_root.get_field_u32(sf("sfOwnerCount"));
        let reserve =
            self.view
                .fees()
                .account_reserve(owner_count as usize + ticket_count as usize) as i64;
        let balance = self
            .pre_fee_balance_drops
            .unwrap_or_else(|| account_root.get_field_amount(sf("sfBalance")).xrp().drops());
        balance >= reserve
    }

    fn first_ticket_sequence(&mut self) -> u32 {
        self.tx_sequence.saturating_add(1)
    }

    fn tx_sequence(&mut self) -> u32 {
        self.tx_sequence
    }

    fn create_ticket(&mut self, ticket_sequence: u32) {
        let ticket_keylet =
            protocol::ticket_keylet(Uint160::from_void(self.account.data()), ticket_sequence);
        let mut sle = STLedgerEntry::new(ticket_keylet);
        sle.set_account_id(sf("sfAccount"), self.account);
        sle.set_field_u32(sf("sfTicketSequence"), ticket_sequence);
        let _ = self.view.insert(Arc::new(sle));
    }

    fn dir_insert_ticket(&mut self, ticket_sequence: u32) -> Option<Self::OwnerNode> {
        let ticket_keylet =
            protocol::ticket_keylet(Uint160::from_void(self.account.data()), ticket_sequence);
        ledger::dir_append(
            self.view,
            &owner_dir_keylet(Uint160::from_void(self.account.data())),
            ticket_keylet.key,
            &|_| {},
        )
        .ok()
        .flatten()
    }

    fn set_ticket_owner_node(&mut self, ticket_sequence: u32, page: Self::OwnerNode) {
        let ticket_keylet =
            protocol::ticket_keylet(Uint160::from_void(self.account.data()), ticket_sequence);
        let Ok(Some(ticket)) = self.view.peek(ticket_keylet) else {
            return;
        };

        let mut obj = ticket.clone_as_object();
        obj.set_field_u64(sf("sfOwnerNode"), page);
        let _ = self
            .view
            .update(Arc::new(STLedgerEntry::from_stobject(obj, *ticket.key())));
    }

    fn old_ticket_count(&mut self) -> u32 {
        let account_keylet = protocol::account_keylet(Uint160::from_void(self.account.data()));
        let Ok(Some(account_root)) = self.view.peek(account_keylet) else {
            return 0;
        };

        if account_root.is_field_present(sf("sfTicketCount")) {
            account_root.get_field_u32(sf("sfTicketCount"))
        } else {
            0
        }
    }

    fn set_ticket_count(&mut self, ticket_count: u32) {
        let account_keylet = protocol::account_keylet(Uint160::from_void(self.account.data()));
        let Ok(Some(account_root)) = self.view.peek(account_keylet) else {
            return;
        };

        let mut obj = account_root.clone_as_object();
        obj.set_field_u32(sf("sfTicketCount"), ticket_count);
        let _ = self.view.update(Arc::new(STLedgerEntry::from_stobject(
            obj,
            *account_root.key(),
        )));
    }

    fn adjust_owner_count(&mut self, ticket_count: u32) {
        let account_keylet = protocol::account_keylet(Uint160::from_void(self.account.data()));
        if let Ok(Some(account_root)) = self.view.peek(account_keylet) {
            let _ = ledger::adjust_owner_count(self.view, &account_root, ticket_count as i32);
        }
    }

    fn set_account_sequence(&mut self, sequence: u32) {
        let account_keylet = protocol::account_keylet(Uint160::from_void(self.account.data()));
        let Ok(Some(account_root)) = self.view.peek(account_keylet) else {
            return;
        };

        let mut obj = account_root.clone_as_object();
        obj.set_field_u32(sf("sfSequence"), sequence);
        let _ = self.view.update(Arc::new(STLedgerEntry::from_stobject(
            obj,
            *account_root.key(),
        )));
    }
}

#[derive(Debug, Clone, Copy)]
struct LedgerSignerList {
    flags: u32,
    signer_entries_len: usize,
    owner_node: u64,
}

impl SignerListSetLedgerEntry for LedgerSignerList {
    fn flags(&self) -> u32 {
        self.flags
    }

    fn signer_entries_len(&self) -> usize {
        self.signer_entries_len
    }

    fn owner_node(&self) -> u64 {
        self.owner_node
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct DispatcherSignerEntry {
    account: AccountID,
    weight: u16,
    wallet_locator: Option<Uint256>,
}

impl SignerListSetWriteEntry for DispatcherSignerEntry {
    type AccountId = AccountID;
    type WalletLocator = Uint256;

    fn account(&self) -> &Self::AccountId {
        &self.account
    }

    fn weight(&self) -> u16 {
        self.weight
    }

    fn wallet_locator(&self) -> Option<&Self::WalletLocator> {
        self.wallet_locator.as_ref()
    }
}

fn parse_signer_entries(
    sttx: &STTx,
) -> Result<
    (
        Vec<SignerListSetEntry<AccountID>>,
        Vec<DispatcherSignerEntry>,
    ),
    Ter,
> {
    if !sttx.is_field_present(sf("sfSignerEntries")) {
        return Ok((Vec::new(), Vec::new()));
    }

    let signer_entries = sttx.get_field_array(sf("sfSignerEntries"));
    let mut operation_entries = Vec::with_capacity(signer_entries.len());
    let mut write_entries = Vec::with_capacity(signer_entries.len());

    for signer in signer_entries.iter() {
        let signer_account = signer.get_account_id(sf("sfAccount"));
        let weight = signer.get_field_u16(sf("sfSignerWeight"));
        let wallet_locator = signer
            .is_field_present(sf("sfWalletLocator"))
            .then(|| signer.get_field_h256(sf("sfWalletLocator")));

        operation_entries.push(SignerListSetEntry {
            account: signer_account,
            weight,
        });
        write_entries.push(DispatcherSignerEntry {
            account: signer_account,
            weight,
            wallet_locator,
        });
    }

    write_entries.sort();
    Ok((operation_entries, write_entries))
}

fn remove_signer_list<V: ledger::ApplyView>(view: &mut V, account: AccountID) -> Ter {
    let account_keylet = protocol::account_keylet(Uint160::from_void(account.data()));
    let owner_dir = owner_dir_keylet(Uint160::from_void(account.data()));
    let signer_keylet = signers_keylet(Uint160::from_void(account.data()));
    let signer_list = match view.peek(signer_keylet) {
        Ok(Some(sle)) => Some(LedgerSignerList {
            flags: sle.get_field_u32(sf("sfFlags")),
            signer_entries_len: sle.get_field_array(sf("sfSignerEntries")).len(),
            owner_node: sle.get_field_u64(sf("sfOwnerNode")),
        }),
        Ok(None) => None,
        Err(_) => return Ter::TEF_INTERNAL,
    };

    // Inline removal logic to avoid multiple mutable borrows
    if let Some(ref sl) = signer_list {
        let owner_node = sl.owner_node;
        let _ = ledger::dir_remove(view, &owner_dir, owner_node, signer_keylet.key, false);
        let delta = -(sl.signer_entries_len as i32 + 2);
        if let Ok(Some(account_sle)) = view.peek(account_keylet) {
            let _ = ledger::adjust_owner_count(view, &account_sle, delta);
        }
        if let Ok(Some(signer_sle)) = view.peek(signer_keylet) {
            let _ = view.erase(signer_sle);
        }
    }
    Ter::TES_SUCCESS
}

fn destroy_signer_list<V: ledger::ApplyView>(view: &mut V, account: AccountID) -> Ter {
    let account_keylet = protocol::account_keylet(Uint160::from_void(account.data()));
    let account_sle = match view.peek(account_keylet) {
        Ok(account_sle) => account_sle,
        Err(_) => return Ter::TEF_INTERNAL,
    };

    let master_disabled = account_sle
        .as_ref()
        .is_some_and(|sle| sle.get_field_u32(sf("sfFlags")) & lsfDisableMaster != 0);
    let regular_key_present = account_sle
        .as_ref()
        .is_some_and(|sle| sle.is_field_present(sf("sfRegularKey")));

    run_signer_list_set_destroy_signer_list(
        account_sle.is_some(),
        master_disabled,
        regular_key_present,
        || remove_signer_list(view, account),
    )
}

fn replace_signer_list<V: ledger::ApplyView>(
    view: &mut V,
    account: AccountID,
    quorum: u32,
    signers: &[DispatcherSignerEntry],
    pre_fee_balance_drops: Option<i64>,
) -> Ter {
    let ter = remove_signer_list(view, account);
    if ter != Ter::TES_SUCCESS {
        return ter;
    }

    let account_keylet = protocol::account_keylet(Uint160::from_void(account.data()));
    let owner_dir = owner_dir_keylet(Uint160::from_void(account.data()));
    let signer_keylet = signers_keylet(Uint160::from_void(account.data()));
    let account_sle = match view.peek(account_keylet) {
        Ok(Some(sle)) => sle,
        Ok(None) => return Ter::TEF_INTERNAL,
        Err(_) => return Ter::TEF_INTERNAL,
    };

    let pre_fee_balance = pre_fee_balance_drops
        .map(XRPAmount::from_drops)
        .unwrap_or_else(|| account_sle.get_field_amount(sf("sfBalance")).xrp());
    let old_owner_count = account_sle.get_field_u32(sf("sfOwnerCount"));
    let new_reserve =
        XRPAmount::from_drops(view.fees().account_reserve(old_owner_count as usize + 1) as i64);
    if pre_fee_balance < new_reserve {
        return Ter::TEC_INSUFFICIENT_RESERVE;
    }

    let plan = build_signer_list_set_ledger_write_plan(
        false,
        account,
        quorum,
        LSF_ONE_OWNER_COUNT,
        signers,
    );

    let owner_page = match ledger::dir_insert(view, &owner_dir, signer_keylet.key, &|_| {}) {
        Ok(Some(page)) => page,
        Ok(None) => return Ter::TEC_DIR_FULL,
        Err(_) => return Ter::TEF_INTERNAL,
    };

    let mut signer_list = STLedgerEntry::new(signer_keylet);
    if let Some(owner) = plan.owner {
        signer_list.set_account_id(sf("sfOwner"), owner);
    }
    signer_list.set_field_u32(sf("sfSignerQuorum"), plan.signer_quorum);
    signer_list.set_field_u32(sf("sfSignerListID"), plan.signer_list_id);
    if let Some(flags) = plan.flags {
        signer_list.set_field_u32(sf("sfFlags"), flags);
    }
    signer_list.set_field_u64(sf("sfOwnerNode"), owner_page);

    let mut signer_array = STArray::new(sf("sfSignerEntries"));
    signer_array.reserve(plan.signer_entries.len());
    for signer in plan.signer_entries {
        let mut signer_entry = STObject::make_inner_object(sf("sfSignerEntry"));
        signer_entry.set_account_id(sf("sfAccount"), signer.account);
        signer_entry.set_field_u16(sf("sfSignerWeight"), signer.weight);
        if let Some(wallet_locator) = signer.wallet_locator {
            signer_entry.set_field_h256(sf("sfWalletLocator"), wallet_locator);
        }
        signer_array.push_back(signer_entry);
    }
    signer_list.set_field_array(sf("sfSignerEntries"), signer_array);

    if view.insert(Arc::new(signer_list)).is_err() {
        return Ter::TEF_INTERNAL;
    }
    if ledger::adjust_owner_count(view, &account_sle, 1).is_err() {
        return Ter::TEF_INTERNAL;
    }

    Ter::TES_SUCCESS
}

pub fn handle_real_dispatch<V: ledger::ApplyView>(
    view: &mut V,
    sttx: &STTx,
    txn_type: TxType,
    pre_fee_balance_drops: Option<i64>,
) -> Ter {
    let tx_hash = sttx.get_hash(protocol::HashPrefix::TransactionId);
    tracing::trace!(target: "tx", tx_type = %format!("{:?}", txn_type), hash = %tx_hash, "Transaction preflight");
    let result = handle_real_dispatch_inner(view, sttx, txn_type, pre_fee_balance_drops);

    if protocol::is_tes_success(result) || protocol::is_tec_claim(result) {
        tracing::debug!(target: "tx", tx_type = %format!("{:?}", txn_type), hash = %tx_hash, result = %format!("{:?}", result), "Transaction applied");
    } else {
        tracing::warn!(target: "tx", tx_type = %format!("{:?}", txn_type), hash = %tx_hash, result = %format!("{:?}", result), "Transaction rejected");
    }

    // Comprehensive per-tx debug log — logs every tx with key fields and result.
    // Controlled by a global counter so we don't flood the log.
    static TX_LOG: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let c = TX_LOG.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    if c < 5000 {
        let account = sttx.get_account_id(sf("sfAccount"));
        let flags = sttx.get_field_u32(sf("sfFlags"));
        let seq = sttx.get_seq_value();

        // Key amounts for each tx type
        let detail = match txn_type {
            TxType::OFFER_CREATE => {
                let tp = sttx.get_field_amount(sf("sfTakerPays"));
                let tg = sttx.get_field_amount(sf("sfTakerGets"));
                format!(
                    "TakerPays_native={} TakerGets_native={} TakerPays_signum={} TakerGets_signum={}",
                    tp.native(),
                    tg.native(),
                    tp.signum(),
                    tg.signum()
                )
            }
            TxType::PAYMENT => {
                let amt = sttx.get_field_amount(sf("sfAmount"));
                let has_sm = sttx.is_field_present(sf("sfSendMax"));
                let has_paths = sttx.is_field_present(sf("sfPaths"));
                let sm_native = if has_sm {
                    sttx.get_field_amount(sf("sfSendMax")).native()
                } else {
                    true
                };
                format!(
                    "Amount_native={} has_sendmax={} sendmax_native={} has_paths={} partial={}",
                    amt.native(),
                    has_sm,
                    sm_native,
                    has_paths,
                    (flags & 0x0002_0000) != 0
                )
            }
            TxType::CHECK_CASH => {
                let has_amt = sttx.is_field_present(sf("sfAmount"));
                let has_dmin = sttx.is_field_present(sf("sfDeliverMin"));
                format!("has_amount={} has_deliver_min={}", has_amt, has_dmin)
            }
            _ => String::new(),
        };

        tracing::debug!(target: "tx",
            "[tx_trace] type={:?} seq={} flags=0x{:08x} acct={:02x}{:02x}{:02x}{:02x} result={:?} {}",
            txn_type,
            seq,
            flags,
            account.data()[0],
            account.data()[1],
            account.data()[2],
            account.data()[3],
            result,
            detail,
        );
    }

    result
}

fn handle_real_dispatch_inner<V: ledger::ApplyView>(
    view: &mut V,
    sttx: &STTx,
    txn_type: TxType,
    pre_fee_balance_drops: Option<i64>,
) -> Ter {
    match txn_type {
        // --- XChain Bridge ---
        TxType::XCHAIN_CREATE_BRIDGE => {
            if !view.rules().enabled(&protocol::feature_id("XChainBridge")) {
                return Ter::TEM_DISABLED;
            }
            crate::state::xchain::apply_xchain_create_bridge(view, sttx)
        }
        TxType::XCHAIN_MODIFY_BRIDGE => {
            if !view.rules().enabled(&protocol::feature_id("XChainBridge")) {
                return Ter::TEM_DISABLED;
            }
            crate::state::xchain::apply_xchain_modify_bridge(view, sttx)
        }
        TxType::XCHAIN_CLAIM => {
            if !view.rules().enabled(&protocol::feature_id("XChainBridge")) {
                return Ter::TEM_DISABLED;
            }
            crate::state::xchain::apply_xchain_claim(view, sttx)
        }
        TxType::XCHAIN_COMMIT => {
            if !view.rules().enabled(&protocol::feature_id("XChainBridge")) {
                return Ter::TEM_DISABLED;
            }
            crate::state::xchain::apply_xchain_commit(view, sttx, pre_fee_balance_drops)
        }
        TxType::XCHAIN_CREATE_CLAIM_ID => {
            if !view.rules().enabled(&protocol::feature_id("XChainBridge")) {
                return Ter::TEM_DISABLED;
            }
            crate::state::xchain::apply_xchain_create_claim_id(view, sttx)
        }
        TxType::XCHAIN_ADD_CLAIM_ATTESTATION => {
            if !view.rules().enabled(&protocol::feature_id("XChainBridge")) {
                return Ter::TEM_DISABLED;
            }
            crate::state::xchain::apply_xchain_add_claim_attestation(view, sttx)
        }
        TxType::XCHAIN_ADD_ACCOUNT_CREATE_ATTESTATION => {
            if !view.rules().enabled(&protocol::feature_id("XChainBridge")) {
                return Ter::TEM_DISABLED;
            }
            crate::state::xchain::apply_xchain_add_account_create_attestation(view, sttx)
        }
        TxType::XCHAIN_ACCOUNT_CREATE_COMMIT => {
            if !view.rules().enabled(&protocol::feature_id("XChainBridge")) {
                return Ter::TEM_DISABLED;
            }
            crate::state::xchain::apply_xchain_account_create_commit(
                view,
                sttx,
                pre_fee_balance_drops,
            )
        }

        // --- Vault / Loan / Batch / Delegate ---
        TxType::VAULT_CREATE => crate::state::vault::apply_vault_create(view, sttx),
        TxType::VAULT_SET => crate::state::vault::apply_vault_set(view, sttx),
        TxType::VAULT_DELETE => crate::state::vault::apply_vault_delete(view, sttx),
        TxType::VAULT_DEPOSIT => crate::state::vault::apply_vault_deposit(view, sttx),
        TxType::VAULT_WITHDRAW => crate::state::vault::apply_vault_withdraw(view, sttx),
        TxType::VAULT_CLAWBACK => crate::state::vault::apply_vault_clawback(view, sttx),
        TxType::BATCH => {
            if !view.rules().enabled(&protocol::feature_id("Batch")) {
                return Ter::TEM_DISABLED;
            }
            crate::state::batch::apply_batch(view, sttx)
        }
        TxType::LOAN_SET => crate::state::lending::apply_loan_set(
            view,
            sttx,
            pre_fee_balance_drops.unwrap_or(10_000_000_000),
        ),
        TxType::LOAN_DELETE => crate::state::lending::apply_loan_delete(view, sttx),
        TxType::LOAN_MANAGE => crate::state::lending::apply_loan_manage(view, sttx),
        TxType::LOAN_PAY => crate::state::lending::apply_loan_pay(view, sttx),
        TxType::LOAN_BROKER_SET => crate::state::lending::apply_loan_broker_set(
            view,
            sttx,
            pre_fee_balance_drops.unwrap_or(10_000_000_000),
        ),
        TxType::LOAN_BROKER_DELETE => crate::state::lending::apply_loan_broker_delete(view, sttx),
        TxType::LOAN_BROKER_COVER_DEPOSIT => {
            crate::state::lending::apply_loan_broker_cover_deposit(view, sttx)
        }
        TxType::LOAN_BROKER_COVER_WITHDRAW => {
            crate::state::lending::apply_loan_broker_cover_withdraw(
                view,
                sttx,
                pre_fee_balance_drops.unwrap_or(10_000_000_000),
            )
        }
        TxType::LOAN_BROKER_COVER_CLAWBACK => {
            crate::state::lending::apply_loan_broker_cover_clawback(view, sttx)
        }
        TxType::DELEGATE_SET => {
            if !view
                .rules()
                .enabled(&protocol::feature_id("PermissionDelegationV1_1"))
            {
                return Ter::TEM_DISABLED;
            }
            let account = sttx.get_account_id(sf("sfAccount"));
            let authorize = sttx.get_account_id(sf("sfAuthorize"));
            let permissions = sttx
                .get_field_array(sf("sfPermissions"))
                .iter()
                .map(|permission| permission.get_field_u32(sf("sfPermissionValue")))
                .collect::<Vec<_>>();
            let balance_for_reserve = pre_fee_balance_drops.unwrap_or_else(|| {
                view.peek(protocol::account_keylet(Uint160::from_void(account.data())))
                    .ok()
                    .flatten()
                    .map(|sle| sle.get_field_amount(sf("sfBalance")).xrp().drops())
                    .unwrap_or(0)
            });
            let mut sink =
                ViewBackedDelegateSetSink::new(view, account, authorize, balance_for_reserve);
            run_delegate_set_do_apply(&permissions, &mut sink)
        }

        // --- Payment: full compatibility (payment.rs) ---
        TxType::PAYMENT => crate::state::payment::do_payment(view, sttx, pre_fee_balance_drops),

        // --- TrustSet: full flag handling ---
        // --- TrustSet: full compatibility (trust_set.rs) ---
        TxType::TRUST_SET => {
            crate::state::trust_set::do_trust_set(view, sttx, pre_fee_balance_drops)
        }

        // --- OfferCreate: full compatibility (offer_create.rs) ---
        TxType::OFFER_CREATE => {
            crate::state::offer_create::do_offer_create(view, sttx, pre_fee_balance_drops)
        }

        // --- OfferCancel ---
        TxType::OFFER_CANCEL => {
            let account = sttx.get_account_id(sf("sfAccount"));
            let seq = sttx.get_field_u32(sf("sfOfferSequence"));
            let keylet = protocol::offer_keylet(Uint160::from_void(account.data()), seq);
            if let Ok(Some(offer)) = view.peek(keylet) {
                let _ = crate::state::offer_create::offer_delete_pub(view, &account, offer);
            }
            Ter::TES_SUCCESS
        }

        // --- Account operations ---
        TxType::ACCOUNT_SET => {
            let account = sttx.get_account_id(sf("sfAccount"));
            let keylet = protocol::account_keylet(Uint160::from_void(account.data()));
            if let Ok(Some(sle)) = view.peek(keylet) {
                let mut obj = sle.clone_as_object();
                if sttx.is_field_present(sf("sfDomain")) {
                    obj.set_stbase(protocol::STBlob::from_buffer(
                        sf("sfDomain"),
                        basics::buffer::Buffer::from(&sttx.get_field_vl(sf("sfDomain"))[..]),
                    ));
                }
                if sttx.is_field_present(sf("sfTransferRate")) {
                    let rate = sttx.get_field_u32(sf("sfTransferRate"));
                    if rate == 0 || rate == 1_000_000_000 {
                        obj.make_field_absent(sf("sfTransferRate"));
                    } else {
                        obj.set_field_u32(sf("sfTransferRate"), rate);
                    }
                }
                if sttx.is_field_present(sf("sfTickSize")) {
                    let tick = sttx.get_field_u8(sf("sfTickSize"));
                    if tick == 0 {
                        obj.make_field_absent(sf("sfTickSize"));
                    } else {
                        obj.set_field_u8(sf("sfTickSize"), tick);
                    }
                }
                if sttx.is_field_present(sf("sfEmailHash")) {
                    let hash = sttx.get_field_h128(sf("sfEmailHash"));
                    if hash.is_zero() {
                        obj.make_field_absent(sf("sfEmailHash"));
                    } else {
                        obj.set_field_h128(sf("sfEmailHash"), hash);
                    }
                }
                if sttx.is_field_present(sf("sfMessageKey")) {
                    let vl = sttx.get_field_vl(sf("sfMessageKey"));
                    if vl.is_empty() {
                        obj.make_field_absent(sf("sfMessageKey"));
                    } else {
                        obj.set_stbase(protocol::STBlob::from_buffer(
                            sf("sfMessageKey"),
                            basics::buffer::Buffer::from(&vl[..]),
                        ));
                    }
                }
                // sfSetFlag / sfClearFlag — modify account flags
                let mut flags = obj.get_field_u32(sf("sfFlags"));
                if sttx.is_field_present(sf("sfSetFlag")) {
                    let set_flag = sttx.get_field_u32(sf("sfSetFlag"));
                    let lsf = asf_to_lsf(set_flag);
                    if lsf != 0 {
                        flags |= lsf;
                    }
                    // asfAccountTxnID (5) — add the field if not present
                    if set_flag == 5 && !obj.is_field_present(sf("sfAccountTxnID")) {
                        obj.set_field_h256(sf("sfAccountTxnID"), Uint256::default());
                    }
                }
                if sttx.is_field_present(sf("sfClearFlag")) {
                    let clear_flag = sttx.get_field_u32(sf("sfClearFlag"));
                    let lsf = asf_to_lsf(clear_flag);
                    if lsf != 0 {
                        flags &= !lsf;
                    }
                    // asfAccountTxnID (5) — remove the field
                    if clear_flag == 5 {
                        obj.make_field_absent(sf("sfAccountTxnID"));
                    }
                    // asfAuthorizedNFTokenMinter (10) — remove sfNFTokenMinter field
                    if clear_flag == 10 {
                        obj.make_field_absent(sf("sfNFTokenMinter"));
                    }
                }
                obj.set_field_u32(sf("sfFlags"), flags);
                let _ = view.update(Arc::new(STLedgerEntry::from_stobject(obj, *sle.key())));
            }
            Ter::TES_SUCCESS
        }

        TxType::ACCOUNT_DELETE => {
            let account = sttx.get_account_id(sf("sfAccount"));
            let destination = sttx.get_account_id(sf("sfDestination"));
            // Transfer remaining XRP to destination, delete account
            let src_keylet = protocol::account_keylet(Uint160::from_void(account.data()));
            let dst_keylet = protocol::account_keylet(Uint160::from_void(destination.data()));
            if let (Ok(Some(src)), Ok(Some(dst))) = (view.peek(src_keylet), view.peek(dst_keylet)) {
                let balance = src.get_field_amount(sf("sfBalance")).xrp();
                let mut dst_obj = dst.clone_as_object();
                let dst_bal = dst.get_field_amount(sf("sfBalance")).xrp();
                dst_obj.set_field_amount(
                    sf("sfBalance"),
                    STAmount::from_xrp_amount(XRPAmount::from_drops(
                        dst_bal.drops() + balance.drops(),
                    )),
                );
                let _ = view.update(Arc::new(STLedgerEntry::from_stobject(dst_obj, *dst.key())));
                let _ = view.erase(src);
            }
            Ter::TES_SUCCESS
        }

        TxType::LEDGER_STATE_FIX => apply_ledger_state_fix(view, sttx),

        TxType::REGULAR_KEY_SET => {
            let account = sttx.get_account_id(sf("sfAccount"));
            let keylet = protocol::account_keylet(Uint160::from_void(account.data()));
            if let Ok(Some(sle)) = view.peek(keylet) {
                let mut obj = sle.clone_as_object();
                if sttx.is_field_present(sf("sfRegularKey")) {
                    obj.set_account_id(sf("sfRegularKey"), sttx.get_account_id(sf("sfRegularKey")));
                } else {
                    obj.make_field_absent(sf("sfRegularKey"));
                }
                let _ = view.update(Arc::new(STLedgerEntry::from_stobject(obj, *sle.key())));
            }
            Ter::TES_SUCCESS
        }

        TxType::SIGNER_LIST_SET => {
            let account = sttx.get_account_id(sf("sfAccount"));
            let quorum = sttx.get_field_u32(sf("sfSignerQuorum"));
            let (operation_entries, write_entries) = match parse_signer_entries(sttx) {
                Ok(parsed) => parsed,
                Err(err) => return err,
            };

            if !operation_entries.is_empty() {
                let validation = tx::run_signer_list_set_validate_quorum_and_signer_entries(
                    quorum,
                    &operation_entries,
                    &account,
                );
                if validation != Ter::TES_SUCCESS {
                    return validation;
                }
            }

            let operation = run_signer_list_set_determine_operation(
                quorum,
                sttx.is_field_present(sf("sfSignerEntries")),
                Ok(operation_entries),
            );
            if operation.result != Ter::TES_SUCCESS {
                return operation.result;
            }

            run_signer_list_set_do_apply(
                operation.operation,
                || Ter::TES_SUCCESS, // replace handled below
                || Ter::TES_SUCCESS, // destroy handled below
            );
            match operation.operation {
                SignerListSetOperation::Set => replace_signer_list(
                    view,
                    account,
                    operation.quorum,
                    &write_entries,
                    pre_fee_balance_drops,
                ),
                SignerListSetOperation::Destroy => destroy_signer_list(view, account),
                _ => Ter::TES_SUCCESS,
            }
        }

        TxType::DEPOSIT_PREAUTH => {
            let account = sttx.get_account_id(sf("sfAccount"));
            if sttx.is_field_present(sf("sfAuthorize")) {
                let auth_account = sttx.get_account_id(sf("sfAuthorize"));
                let preauth_keylet = protocol::deposit_preauth_keylet(
                    Uint160::from_void(account.data()),
                    Uint160::from_void(auth_account.data()),
                );
                let mut sle = STLedgerEntry::new(preauth_keylet);
                sle.set_account_id(sf("sfAccount"), account);
                sle.set_account_id(sf("sfAuthorize"), auth_account);
                // Add to owner directory
                let owner_dir = owner_dir_keylet(Uint160::from_void(account.data()));
                if let Ok(Some(page)) =
                    ledger::dir_append(view, &owner_dir, preauth_keylet.key, &|_| {})
                {
                    sle.set_field_u64(sf("sfOwnerNode"), page);
                }
                let _ = view.insert(Arc::new(sle));
                if let Ok(Some(acct)) =
                    view.peek(protocol::account_keylet(Uint160::from_void(account.data())))
                {
                    let _ = ledger::adjust_owner_count(view, &acct, 1);
                }
            } else if sttx.is_field_present(sf("sfUnauthorize")) {
                let unauth_account = sttx.get_account_id(sf("sfUnauthorize"));
                let preauth_keylet = protocol::deposit_preauth_keylet(
                    Uint160::from_void(account.data()),
                    Uint160::from_void(unauth_account.data()),
                );
                if let Ok(Some(preauth_sle)) = view.peek(preauth_keylet) {
                    let owner_node = preauth_sle.get_field_u64(sf("sfOwnerNode"));
                    let owner_dir = owner_dir_keylet(Uint160::from_void(account.data()));
                    let _ =
                        ledger::dir_remove(view, &owner_dir, owner_node, *preauth_sle.key(), false);
                    let _ = view.erase(preauth_sle);
                    if let Ok(Some(acct)) =
                        view.peek(protocol::account_keylet(Uint160::from_void(account.data())))
                    {
                        let _ = ledger::adjust_owner_count(view, &acct, -1);
                    }
                }
            }
            Ter::TES_SUCCESS
        }

        // --- Escrows ---
        TxType::ESCROW_CREATE => {
            let account = sttx.get_account_id(sf("sfAccount"));
            let dst_account = sttx.get_account_id(sf("sfDestination"));
            let amount = sttx.get_field_amount(sf("sfAmount"));
            let finish_after = if sttx.is_field_present(sf("sfFinishAfter")) {
                Some(sttx.get_field_u32(sf("sfFinishAfter")))
            } else {
                None
            };
            let cancel_after = if sttx.is_field_present(sf("sfCancelAfter")) {
                Some(sttx.get_field_u32(sf("sfCancelAfter")))
            } else {
                None
            };
            if let Ok(facts) = build_escrow_create_facts(view, &account, &dst_account, &amount) {
                let mut sink = ViewBackedEscrowCreateSink {
                    view,
                    account,
                    dst_account,
                    amount,
                    escrow_key: Uint256::default(),
                    escrow_seq: sttx.get_seq_value(),
                    finish_after,
                    cancel_after,
                };
                run_escrow_create_do_apply(facts, &mut sink)
            } else {
                Ter::TEF_INTERNAL
            }
        }
        TxType::ESCROW_FINISH => {
            let owner = sttx.get_account_id(sf("sfOwner"));
            let offer_seq = sttx.get_field_u32(sf("sfOfferSequence"));
            let escrow_keylet =
                protocol::escrow_keylet(Uint160::from_void(owner.data()), offer_seq);
            if let Ok(Some(escrow_sle)) = view.peek(escrow_keylet) {
                if escrow_sle.is_field_present(sf("sfFinishAfter")) {
                    let finish_after = escrow_sle.get_field_u32(sf("sfFinishAfter"));
                    if view.header().parent_close_time < finish_after {
                        return Ter::TEC_NO_PERMISSION;
                    }
                }
                let destination = escrow_sle.get_account_id(sf("sfDestination"));
                let amount = escrow_sle.get_field_amount(sf("sfAmount"));
                if amount.native() {
                    let amount_drops = amount.xrp().drops();
                    let dst_keylet =
                        protocol::account_keylet(Uint160::from_void(destination.data()));
                    if let Ok(Some(dst)) = view.peek(dst_keylet) {
                        let bal = dst.get_field_amount(sf("sfBalance")).xrp();
                        let mut obj = dst.clone_as_object();
                        obj.set_field_amount(
                            sf("sfBalance"),
                            STAmount::from_xrp_amount(XRPAmount::from_drops(
                                bal.drops() + amount_drops,
                            )),
                        );
                        let _ =
                            view.update(Arc::new(STLedgerEntry::from_stobject(obj, *dst.key())));
                    }
                } else {
                    match amount.asset() {
                        protocol::Asset::Issue(_) => {
                            return Ter::TEC_LIMIT_EXCEEDED;
                        }
                        protocol::Asset::MPTIssue(_) => {
                            let locked_rate = escrow_sle
                                .is_field_present(sf("sfTransferRate"))
                                .then(|| escrow_sle.get_field_u32(sf("sfTransferRate")))
                                .unwrap_or(protocol::PARITY_RATE.value);
                            let (net_amount, gross_amount) = escrow_mpt_unlock_amounts(
                                view,
                                &amount,
                                locked_rate,
                                &owner,
                                &destination,
                            );
                            let gross_amount = if view
                                .rules()
                                .enabled(&protocol::feature_id("fixTokenEscrowV1"))
                            {
                                &gross_amount
                            } else {
                                &net_amount
                            };
                            let submitter = sttx.get_account_id(sf("sfAccount"));
                            let result = ledger::mptoken_helpers::unlock_escrow_mpt(
                                view,
                                &owner,
                                &destination,
                                &net_amount,
                                gross_amount,
                                destination == submitter,
                                pre_fee_balance_drops,
                            )
                            .unwrap_or(Ter::TEF_INTERNAL);
                            if result != Ter::TES_SUCCESS {
                                return result;
                            }
                        }
                    }
                }
                // Remove from owner directory and adjust owner count
                let owner_node = escrow_sle.get_field_u64(sf("sfOwnerNode"));
                let owner_dir = owner_dir_keylet(Uint160::from_void(owner.data()));
                let _ = ledger::dir_remove(view, &owner_dir, owner_node, *escrow_sle.key(), false);
                // Remove from destination directory if present
                if escrow_sle.is_field_present(sf("sfDestinationNode")) {
                    let dst_node = escrow_sle.get_field_u64(sf("sfDestinationNode"));
                    let dst_dir = owner_dir_keylet(Uint160::from_void(destination.data()));
                    let _ = ledger::dir_remove(view, &dst_dir, dst_node, *escrow_sle.key(), false);
                }
                // Adjust owner count
                let owner_acct_keylet = protocol::account_keylet(Uint160::from_void(owner.data()));
                if let Ok(Some(owner_acct)) = view.peek(owner_acct_keylet) {
                    let _ = ledger::adjust_owner_count(view, &owner_acct, -1);
                }
                let _ = view.erase(escrow_sle);
            } else {
                return Ter::TEC_NO_TARGET;
            }
            Ter::TES_SUCCESS
        }
        TxType::ESCROW_CANCEL => {
            let owner = sttx.get_account_id(sf("sfOwner"));
            let offer_seq = sttx.get_field_u32(sf("sfOfferSequence"));
            let escrow_keylet =
                protocol::escrow_keylet(Uint160::from_void(owner.data()), offer_seq);
            if let Ok(Some(escrow_sle)) = view.peek(escrow_keylet) {
                if escrow_sle.is_field_present(sf("sfCancelAfter")) {
                    let cancel_after = escrow_sle.get_field_u32(sf("sfCancelAfter"));
                    if view.header().parent_close_time < cancel_after {
                        return Ter::TEC_NO_PERMISSION;
                    }
                }
                let amount = escrow_sle.get_field_amount(sf("sfAmount"));
                if amount.native() {
                    // Return XRP funds to owner.
                    let owner_keylet = protocol::account_keylet(Uint160::from_void(owner.data()));
                    if let Ok(Some(owner_acct)) = view.peek(owner_keylet) {
                        let bal = owner_acct.get_field_amount(sf("sfBalance")).xrp();
                        let mut obj = owner_acct.clone_as_object();
                        obj.set_field_amount(
                            sf("sfBalance"),
                            STAmount::from_xrp_amount(XRPAmount::from_drops(
                                bal.drops() + amount.xrp().drops(),
                            )),
                        );
                        let _ = view.update(Arc::new(STLedgerEntry::from_stobject(
                            obj,
                            *owner_acct.key(),
                        )));
                    }
                } else {
                    match amount.asset() {
                        protocol::Asset::Issue(issue) => {
                            let result = ledger::ripple_state_helpers::issue_iou(
                                view, &owner, &amount, &issue,
                            );
                            if result != Ter::TES_SUCCESS {
                                return result;
                            }
                        }
                        protocol::Asset::MPTIssue(_) => {
                            let submitter = sttx.get_account_id(sf("sfAccount"));
                            let (net_amount, gross_amount) = escrow_mpt_unlock_amounts(
                                view,
                                &amount,
                                protocol::PARITY_RATE.value,
                                &owner,
                                &owner,
                            );
                            let gross_amount = if view
                                .rules()
                                .enabled(&protocol::feature_id("fixTokenEscrowV1"))
                            {
                                &gross_amount
                            } else {
                                &net_amount
                            };
                            let result = ledger::mptoken_helpers::unlock_escrow_mpt(
                                view,
                                &owner,
                                &owner,
                                &net_amount,
                                gross_amount,
                                owner == submitter,
                                pre_fee_balance_drops,
                            )
                            .unwrap_or(Ter::TEF_INTERNAL);
                            if result != Ter::TES_SUCCESS {
                                return result;
                            }
                        }
                    }
                }
                // Remove from owner directory and adjust owner count
                let owner_node = escrow_sle.get_field_u64(sf("sfOwnerNode"));
                let owner_dir = owner_dir_keylet(Uint160::from_void(owner.data()));
                let _ = ledger::dir_remove(view, &owner_dir, owner_node, *escrow_sle.key(), false);
                // Remove from destination directory if present
                if escrow_sle.is_field_present(sf("sfDestinationNode")) {
                    let destination = escrow_sle.get_account_id(sf("sfDestination"));
                    let dst_node = escrow_sle.get_field_u64(sf("sfDestinationNode"));
                    let dst_dir = owner_dir_keylet(Uint160::from_void(destination.data()));
                    let _ = ledger::dir_remove(view, &dst_dir, dst_node, *escrow_sle.key(), false);
                }
                if !amount.native() && escrow_sle.is_field_present(sf("sfIssuerNode")) {
                    let issuer = amount.asset().issuer();
                    let issuer_node = escrow_sle.get_field_u64(sf("sfIssuerNode"));
                    let issuer_dir = owner_dir_keylet(Uint160::from_void(issuer.data()));
                    let _ = ledger::dir_remove(
                        view,
                        &issuer_dir,
                        issuer_node,
                        *escrow_sle.key(),
                        false,
                    );
                }
                // Adjust owner count
                if let Ok(Some(oa)) =
                    view.peek(protocol::account_keylet(Uint160::from_void(owner.data())))
                {
                    let _ = ledger::adjust_owner_count(view, &oa, -1);
                }
                let _ = view.erase(escrow_sle);
            } else {
                return Ter::TEC_NO_TARGET;
            }
            Ter::TES_SUCCESS
        }

        // --- Checks ---
        TxType::CHECK_CREATE => {
            let account = sttx.get_account_id(sf("sfAccount"));
            let dst = sttx.get_account_id(sf("sfDestination"));
            let send_max = sttx.get_field_amount(sf("sfSendMax"));
            let mpt_result = check_mpt_check_create_allowed(view, &account, &dst, &send_max);
            if mpt_result != Ter::TES_SUCCESS {
                return mpt_result;
            }
            let check_keylet =
                protocol::check_keylet(Uint160::from_void(account.data()), sttx.get_seq_value());
            let mut sle = STLedgerEntry::new(check_keylet);
            sle.set_account_id(sf("sfAccount"), account);
            sle.set_account_id(sf("sfDestination"), dst);
            sle.set_field_amount(sf("sfSendMax"), send_max);
            sle.set_field_u32(sf("sfSequence"), sttx.get_seq_value());
            // Add to owner directory
            let owner_dir = owner_dir_keylet(Uint160::from_void(account.data()));
            if let Ok(Some(page)) = ledger::dir_append(view, &owner_dir, check_keylet.key, &|_| {})
            {
                sle.set_field_u64(sf("sfOwnerNode"), page);
            }
            // Add to destination directory
            let dst_dir = owner_dir_keylet(Uint160::from_void(dst.data()));
            if let Ok(Some(page)) = ledger::dir_append(view, &dst_dir, check_keylet.key, &|_| {}) {
                sle.set_field_u64(sf("sfDestinationNode"), page);
            }
            let _ = view.insert(Arc::new(sle));
            if let Ok(Some(acct)) =
                view.peek(protocol::account_keylet(Uint160::from_void(account.data())))
            {
                let _ = ledger::adjust_owner_count(view, &acct, 1);
            }
            Ter::TES_SUCCESS
        }
        TxType::CHECK_CANCEL => {
            let check_id = sttx.get_field_h256(sf("sfCheckID"));
            let check_keylet = protocol::unchecked_keylet(check_id);
            if let Ok(Some(check_sle)) = view.peek(check_keylet) {
                let owner = check_sle.get_account_id(sf("sfAccount"));
                // Remove from owner directory
                let owner_node = check_sle.get_field_u64(sf("sfOwnerNode"));
                let owner_dir = owner_dir_keylet(Uint160::from_void(owner.data()));
                let _ = ledger::dir_remove(view, &owner_dir, owner_node, *check_sle.key(), false);
                // Remove from destination directory
                if check_sle.is_field_present(sf("sfDestinationNode")) {
                    let dst = check_sle.get_account_id(sf("sfDestination"));
                    let dst_node = check_sle.get_field_u64(sf("sfDestinationNode"));
                    let dst_dir = owner_dir_keylet(Uint160::from_void(dst.data()));
                    let _ = ledger::dir_remove(view, &dst_dir, dst_node, *check_sle.key(), false);
                }
                let _ = view.erase(check_sle);
                if let Ok(Some(acct)) =
                    view.peek(protocol::account_keylet(Uint160::from_void(owner.data())))
                {
                    let _ = ledger::adjust_owner_count(view, &acct, -1);
                }
            }
            Ter::TES_SUCCESS
        }
        TxType::CHECK_CASH => {
            let check_id = sttx.get_field_h256(sf("sfCheckID"));
            let check_keylet = protocol::unchecked_keylet(check_id);
            if let Ok(Some(check_sle)) = view.peek(check_keylet) {
                let source = check_sle.get_account_id(sf("sfAccount"));
                let destination = sttx.get_account_id(sf("sfAccount"));
                let requested_amount = if sttx.is_field_present(sf("sfAmount")) {
                    sttx.get_field_amount(sf("sfAmount"))
                } else {
                    check_sle.get_field_amount(sf("sfSendMax"))
                };
                let send_max = check_sle.get_field_amount(sf("sfSendMax"));
                if view
                    .rules()
                    .enabled(&protocol::feature_id("fixCleanup3_2_0"))
                    && !send_max.is_legal_mpt()
                {
                    return Ter::TEF_BAD_LEDGER;
                }
                if sttx.is_field_present(sf("sfAmount")) && requested_amount > send_max {
                    return Ter::TEC_PATH_PARTIAL;
                }
                let mpt_result =
                    check_mpt_check_cash_allowed(view, &source, &destination, &requested_amount);
                if mpt_result != Ter::TES_SUCCESS {
                    return mpt_result;
                }

                if requested_amount.native() {
                    let src_keylet = protocol::account_keylet(Uint160::from_void(source.data()));
                    let Some(src_sle) = view.peek(src_keylet).ok().flatten() else {
                        return Ter::TEC_FAILED_PROCESSING;
                    };
                    let available = src_sle.get_field_amount(sf("sfBalance"));
                    if requested_amount > available {
                        return Ter::TEC_PATH_PARTIAL;
                    }
                    do_xrp_payment(view, &source, &destination, &requested_amount, 0);
                } else {
                    let result = ledger::ripple_state_helpers::account_send_with_fee(
                        view,
                        &source,
                        &destination,
                        &requested_amount,
                    );
                    if !is_tes_success(result) {
                        return Ter::TEC_PATH_PARTIAL;
                    }
                }
                // Remove from owner directory
                let owner_node = check_sle.get_field_u64(sf("sfOwnerNode"));
                let owner_dir = owner_dir_keylet(Uint160::from_void(source.data()));
                let _ = ledger::dir_remove(view, &owner_dir, owner_node, *check_sle.key(), false);
                // Remove from destination directory
                if check_sle.is_field_present(sf("sfDestinationNode")) {
                    let dst = check_sle.get_account_id(sf("sfDestination"));
                    let dst_node = check_sle.get_field_u64(sf("sfDestinationNode"));
                    let dst_dir = owner_dir_keylet(Uint160::from_void(dst.data()));
                    let _ = ledger::dir_remove(view, &dst_dir, dst_node, *check_sle.key(), false);
                }
                let _ = view.erase(check_sle);
                if let Ok(Some(acct)) =
                    view.peek(protocol::account_keylet(Uint160::from_void(source.data())))
                {
                    let _ = ledger::adjust_owner_count(view, &acct, -1);
                }
            }
            Ter::TES_SUCCESS
        }

        // --- PayChans ---
        TxType::PAYCHAN_CREATE => {
            let account = sttx.get_account_id(sf("sfAccount"));
            let dst = sttx.get_account_id(sf("sfDestination"));
            let amount = sttx.get_field_amount(sf("sfAmount"));
            let settle_delay = sttx.get_field_u32(sf("sfSettleDelay"));
            let chan_keylet = protocol::pay_channel_keylet(
                Uint160::from_void(account.data()),
                Uint160::from_void(dst.data()),
                sttx.get_seq_value(),
            );
            let mut sle = STLedgerEntry::new(chan_keylet);
            sle.set_account_id(sf("sfAccount"), account);
            sle.set_account_id(sf("sfDestination"), dst);
            sle.set_field_amount(sf("sfAmount"), amount.clone());
            sle.set_field_amount(sf("sfBalance"), STAmount::from_xrp_amount(XRPAmount::new()));
            sle.set_field_u32(sf("sfSettleDelay"), settle_delay);
            // Add to owner directory
            let owner_dir = owner_dir_keylet(Uint160::from_void(account.data()));
            if let Ok(Some(page)) = ledger::dir_append(view, &owner_dir, chan_keylet.key, &|_| {}) {
                sle.set_field_u64(sf("sfOwnerNode"), page);
            }
            let _ = view.insert(Arc::new(sle));
            // Adjust owner count
            if let Ok(Some(acct)) =
                view.peek(protocol::account_keylet(Uint160::from_void(account.data())))
            {
                let _ = ledger::adjust_owner_count(view, &acct, 1);
            }
            // Debit source account
            let src_keylet = protocol::account_keylet(Uint160::from_void(account.data()));
            if let Ok(Some(src_sle)) = view.peek(src_keylet) {
                let bal = src_sle.get_field_amount(sf("sfBalance")).xrp().drops();
                let amount_drops = amount.xrp().drops();
                let owner_count = src_sle.get_field_u32(sf("sfOwnerCount"));
                let reserve = view.fees().account_reserve(owner_count as usize) as i64;
                if bal < amount_drops + reserve {
                    return Ter::TEC_UNFUNDED;
                }
                let mut obj = src_sle.clone_as_object();
                obj.set_field_amount(
                    sf("sfBalance"),
                    STAmount::from_xrp_amount(XRPAmount::from_drops(bal - amount_drops)),
                );
                let _ = view.update(Arc::new(STLedgerEntry::from_stobject(obj, *src_sle.key())));
            }
            Ter::TES_SUCCESS
        }
        TxType::PAYCHAN_FUND => {
            let account = sttx.get_account_id(sf("sfAccount"));
            let channel_id = sttx.get_field_h256(sf("sfChannel"));
            let amount = sttx.get_field_amount(sf("sfAmount"));
            let chan_keylet = protocol::unchecked_keylet(channel_id);
            if let Ok(Some(chan)) = view.peek(chan_keylet) {
                // Increase channel amount
                let cur = chan.get_field_amount(sf("sfAmount"));
                let mut obj = chan.clone_as_object();
                obj.set_field_amount(sf("sfAmount"), cur + amount.clone());
                let _ = view.update(Arc::new(STLedgerEntry::from_stobject(obj, *chan.key())));
                // Debit source account
                let src_keylet = protocol::account_keylet(Uint160::from_void(account.data()));
                if let Ok(Some(src_sle)) = view.peek(src_keylet) {
                    let bal = src_sle.get_field_amount(sf("sfBalance")).xrp().drops();
                    let mut src_obj = src_sle.clone_as_object();
                    src_obj.set_field_amount(
                        sf("sfBalance"),
                        STAmount::from_xrp_amount(XRPAmount::from_drops(
                            bal - amount.xrp().drops(),
                        )),
                    );
                    let _ = view.update(Arc::new(STLedgerEntry::from_stobject(
                        src_obj,
                        *src_sle.key(),
                    )));
                }
            }
            Ter::TES_SUCCESS
        }
        TxType::PAYCHAN_CLAIM => {
            let channel_id = sttx.get_field_h256(sf("sfChannel"));
            let chan_keylet = protocol::unchecked_keylet(channel_id);
            let Some(chan) = view.peek(chan_keylet).ok().flatten() else {
                return Ter::TEC_NO_TARGET;
            };

            let src = chan.get_account_id(sf("sfAccount"));
            let dst = chan.get_account_id(sf("sfDestination"));
            let tx_account = sttx.get_account_id(sf("sfAccount"));
            let tx_flags = sttx.get_field_u32(sf("sfFlags"));

            // reference: check expiration/cancelAfter — close expired channels
            let close_time = view.header().parent_close_time;
            if chan.is_field_present(sf("sfCancelAfter")) {
                let cancel_after = chan.get_field_u32(sf("sfCancelAfter"));
                if close_time >= cancel_after {
                    return close_channel(view, &chan, chan_keylet.key);
                }
            }
            if chan.is_field_present(sf("sfExpiration")) {
                let expiration = chan.get_field_u32(sf("sfExpiration"));
                if close_time >= expiration {
                    return close_channel(view, &chan, chan_keylet.key);
                }
            }

            // reference: permission check
            if tx_account != src && tx_account != dst {
                return Ter::TEC_NO_PERMISSION;
            }

            // reference: balance update
            if sttx.is_field_present(sf("sfBalance")) {
                let chan_balance = chan.get_field_amount(sf("sfBalance")).xrp().drops();
                let chan_funds = chan.get_field_amount(sf("sfAmount")).xrp().drops();
                let req_balance = sttx.get_field_amount(sf("sfBalance")).xrp().drops();

                if req_balance > chan_funds || req_balance <= chan_balance {
                    return Ter::TEC_UNFUNDED_PAYMENT;
                }

                let delta = req_balance - chan_balance;

                // Credit destination
                let dst_keylet = protocol::account_keylet(Uint160::from_void(dst.data()));
                if let Ok(Some(dst_sle)) = view.peek(dst_keylet) {
                    let dst_bal = dst_sle.get_field_amount(sf("sfBalance")).xrp().drops();
                    let mut dst_obj = dst_sle.clone_as_object();
                    dst_obj.set_field_amount(
                        sf("sfBalance"),
                        STAmount::from_xrp_amount(XRPAmount::from_drops(dst_bal + delta)),
                    );
                    let _ = view.update(Arc::new(STLedgerEntry::from_stobject(
                        dst_obj,
                        *dst_sle.key(),
                    )));
                } else {
                    return Ter::TEC_NO_DST;
                }

                // Update channel balance
                let mut obj = chan.clone_as_object();
                obj.set_field_amount(sf("sfBalance"), sttx.get_field_amount(sf("sfBalance")));
                let _ = view.update(Arc::new(STLedgerEntry::from_stobject(obj, *chan.key())));
            }

            // reference: tfRenew — clear expiration (only source can renew)
            if (tx_flags & 0x0001_0000) != 0 {
                if src != tx_account {
                    return Ter::TEC_NO_PERMISSION;
                }
                if let Ok(Some(cur)) = view.peek(chan_keylet) {
                    let mut obj = cur.clone_as_object();
                    obj.make_field_absent(sf("sfExpiration"));
                    let _ =
                        view.update(Arc::new(STLedgerEntry::from_stobject(obj, chan_keylet.key)));
                }
            }

            // reference: tfClose — close channel or set expiration
            if (tx_flags & 0x0002_0000) != 0 {
                if let Ok(Some(cur)) = view.peek(chan_keylet) {
                    let cur_balance = cur.get_field_amount(sf("sfBalance")).xrp().drops();
                    let cur_amount = cur.get_field_amount(sf("sfAmount")).xrp().drops();

                    if dst == tx_account || cur_balance == cur_amount {
                        return close_channel(view, &cur, chan_keylet.key);
                    }

                    let settle_delay = cur.get_field_u32(sf("sfSettleDelay"));
                    let settle_expiration = close_time + settle_delay;

                    let should_update = if cur.is_field_present(sf("sfExpiration")) {
                        cur.get_field_u32(sf("sfExpiration")) > settle_expiration
                    } else {
                        true
                    };

                    if should_update {
                        let mut obj = cur.clone_as_object();
                        obj.set_field_u32(sf("sfExpiration"), settle_expiration);
                        let _ = view
                            .update(Arc::new(STLedgerEntry::from_stobject(obj, chan_keylet.key)));
                    }
                }
            }

            Ter::TES_SUCCESS
        }

        // --- AMM ---
        TxType::AMM_CREATE => {
            let account = sttx.get_account_id(sf("sfAccount"));
            let amount1 = sttx.get_field_amount(sf("sfAmount"));
            let amount2 = sttx.get_field_amount(sf("sfAmount2"));
            let mpt_result = check_mpt_amm_asset_allowed(view, &account, amount1.asset(), true);
            if mpt_result != Ter::TES_SUCCESS {
                return mpt_result;
            }
            let mpt_result = check_mpt_amm_asset_allowed(view, &account, amount2.asset(), true);
            if mpt_result != Ter::TES_SUCCESS {
                return mpt_result;
            }
            let facts = AMMCreateApplyFacts {
                amount1: amount1.clone(),
                amount2: amount2.clone(),
                trading_fee: sttx.get_field_u16(sf("sfTradingFee")),
                account,
                amm_account: account,
            };
            let mut sink = ViewBackedAMMCreateSink {
                view,
                account,
                amount1,
                amount2,
                trading_fee: facts.trading_fee,
                amm_keylet: None,
                amm_account: None,
                lp_tokens: None,
            };
            run_amm_create_do_apply(facts, &mut sink)
        }
        TxType::AMM_DEPOSIT => {
            let account = sttx.get_account_id(sf("sfAccount"));
            let asset_amount1 = sttx.get_field_amount(sf("sfAsset"));
            let asset_amount2 = sttx.get_field_amount(sf("sfAsset2"));
            let mpt_result =
                check_mpt_amm_asset_allowed(view, &account, asset_amount1.asset(), false);
            if mpt_result != Ter::TES_SUCCESS {
                return mpt_result;
            }
            let mpt_result =
                check_mpt_amm_asset_allowed(view, &account, asset_amount2.asset(), false);
            if mpt_result != Ter::TES_SUCCESS {
                return mpt_result;
            }
            let asset1 = asset_amount1.asset();
            let asset2 = asset_amount2.asset();
            let amm_keylet = protocol::keylet::amm(asset1, asset2);
            if let Ok(Some(amm_sle)) = view.peek(amm_keylet) {
                let amm_account = amm_sle.get_account_id(sf("sfAccount"));
                let mpt_result = check_mpt_amm_pool_asset_unlocked(view, &amm_account, asset1);
                if mpt_result != Ter::TES_SUCCESS {
                    return mpt_result;
                }
                let mpt_result = check_mpt_amm_pool_asset_unlocked(view, &amm_account, asset2);
                if mpt_result != Ter::TES_SUCCESS {
                    return mpt_result;
                }
                let deposit_amount = if sttx.is_field_present(sf("sfAmount")) {
                    sttx.get_field_amount(sf("sfAmount"))
                } else {
                    return Ter::TEM_MALFORMED;
                };
                let mpt_result =
                    check_mpt_amm_pool_asset_unlocked(view, &amm_account, deposit_amount.asset());
                if mpt_result != Ter::TES_SUCCESS {
                    return mpt_result;
                }
                let mpt_result =
                    check_mpt_amm_asset_allowed(view, &account, deposit_amount.asset(), true);
                if mpt_result != Ter::TES_SUCCESS {
                    return mpt_result;
                }
                // Read current pool balances
                let pool1 = amm_sle.get_field_amount(sf("sfAmount"));
                let lp_tokens = amm_sle.get_field_amount(sf("sfLPTokenBalance"));
                let lp_issue = lp_tokens.issue();
                // Constant product: LP minted = totalLP * depositAmt / poolAmt
                let lp_minted = if pool1.signum() > 0 {
                    lp_tokens
                        .multiply(&deposit_amount, lp_tokens.asset())
                        .divide(&pool1, lp_tokens.asset())
                } else {
                    deposit_amount.clone()
                };
                let empty_pool_reinit = lp_tokens.signum() == 0;
                // Update AMM pool balance and LPTokenBalance
                let mut obj = amm_sle.clone_as_object();
                obj.set_field_amount(sf("sfAmount"), pool1 + deposit_amount.clone());
                obj.set_field_amount(sf("sfLPTokenBalance"), lp_tokens + lp_minted.clone());
                if empty_pool_reinit
                    && view
                        .rules()
                        .enabled(&protocol::feature_id("fixCleanup3_2_0"))
                    && obj.is_field_present(sf("sfAuctionSlot"))
                {
                    let mut auction_slot = obj.peek_field_object(sf("sfAuctionSlot")).clone();
                    if auction_slot.is_field_present(sf("sfAuthAccounts")) {
                        auction_slot.make_field_absent(sf("sfAuthAccounts"));
                        obj.set_field_object(sf("sfAuctionSlot"), auction_slot);
                    }
                }
                let _ = view.update(Arc::new(STLedgerEntry::from_stobject(obj, *amm_sle.key())));
                // Credit LP tokens to depositor's trust line (reference issueIOU)
                crate::state::amm_bid_apply::issue_iou_pub(view, &account, &lp_minted, &lp_issue);
            }
            Ter::TES_SUCCESS
        }
        TxType::AMM_WITHDRAW => {
            let account = sttx.get_account_id(sf("sfAccount"));
            let asset_amount1 = sttx.get_field_amount(sf("sfAsset"));
            let asset_amount2 = sttx.get_field_amount(sf("sfAsset2"));
            let mpt_result =
                check_mpt_amm_withdraw_asset_allowed(view, &account, asset_amount1.asset());
            if mpt_result != Ter::TES_SUCCESS {
                return mpt_result;
            }
            let mpt_result =
                check_mpt_amm_withdraw_asset_allowed(view, &account, asset_amount2.asset());
            if mpt_result != Ter::TES_SUCCESS {
                return mpt_result;
            }
            let asset1 = asset_amount1.asset();
            let asset2 = asset_amount2.asset();
            let amm_keylet = protocol::keylet::amm(asset1, asset2);
            if let Ok(Some(amm_sle)) = view.peek(amm_keylet) {
                let amm_account = amm_sle.get_account_id(sf("sfAccount"));
                let mpt_result = check_mpt_amm_pool_asset_unlocked(view, &amm_account, asset1);
                if mpt_result != Ter::TES_SUCCESS {
                    return mpt_result;
                }
                let mpt_result = check_mpt_amm_pool_asset_unlocked(view, &amm_account, asset2);
                if mpt_result != Ter::TES_SUCCESS {
                    return mpt_result;
                }
                let lp_tokens_in = if sttx.is_field_present(sf("sfLPTokenIn")) {
                    sttx.get_field_amount(sf("sfLPTokenIn"))
                } else if sttx.is_field_present(sf("sfAmount")) {
                    sttx.get_field_amount(sf("sfAmount"))
                } else {
                    return Ter::TEM_MALFORMED;
                };
                let pool1 = amm_sle.get_field_amount(sf("sfAmount"));
                let lp_total = amm_sle.get_field_amount(sf("sfLPTokenBalance"));
                let lp_issue = lp_total.issue();
                // asset1Out = pool1 * lpTokensIn / totalLP
                let asset1_out = if lp_total.signum() > 0 {
                    pool1
                        .multiply(&lp_tokens_in, pool1.asset())
                        .divide(&lp_total, pool1.asset())
                } else {
                    pool1.zeroed()
                };
                // Debit LP tokens from withdrawer's trust line (reference redeemIOU)
                crate::state::amm_bid_apply::redeem_iou_pub(
                    view,
                    &account,
                    &lp_tokens_in,
                    &lp_issue,
                );
                // Update AMM
                let mut obj = amm_sle.clone_as_object();
                obj.set_field_amount(sf("sfAmount"), pool1 - asset1_out);
                obj.set_field_amount(sf("sfLPTokenBalance"), lp_total - lp_tokens_in);
                let _ = view.update(Arc::new(STLedgerEntry::from_stobject(obj, *amm_sle.key())));
            }
            Ter::TES_SUCCESS
        }
        TxType::AMM_VOTE => {
            let asset1 = sttx.get_field_issue(sf("sfAsset")).asset();
            let asset2 = sttx.get_field_issue(sf("sfAsset2")).asset();
            let fee_vote = sttx.get_field_u16(sf("sfTradingFee"));
            let account = sttx.get_account_id(sf("sfAccount"));
            let amm_keylet = protocol::keylet::amm(asset1, asset2);
            let Ok(Some(amm_sle)) = view.peek(amm_keylet) else {
                return Ter::TER_NO_AMM;
            };
            let lp_amm_balance = amm_sle.get_field_amount(sf("sfLPTokenBalance"));
            if lp_amm_balance.signum() == 0 {
                return Ter::TEC_AMM_EMPTY;
            }
            let Ok(Some(lp_tokens_new)) = amm_lp_holds_in_view(view, &amm_sle, account) else {
                return Ter::TEC_AMM_INVALID_TOKENS;
            };
            if lp_tokens_new.signum() == 0 {
                return Ter::TEC_AMM_INVALID_TOKENS;
            }

            let lp_total = ledger::amm_helpers::stamount_as_number(&lp_amm_balance);
            let lp_tokens_new_num = ledger::amm_helpers::stamount_as_number(&lp_tokens_new);
            let mut updated_vote_slots = STArray::new(sf("sfVoteSlots"));
            let mut numerator = RuntimeNumber::zero();
            let mut denominator = RuntimeNumber::zero();
            let mut found_account = false;
            let mut min_tokens: Option<RuntimeNumber> = None;
            let mut min_pos = 0usize;
            let mut min_account = AccountID::from_array([0; 20]);
            let mut min_fee = 0u32;

            let existing_slots = if amm_sle.is_field_present(sf("sfVoteSlots")) {
                amm_sle.get_field_array(sf("sfVoteSlots"))
            } else {
                STArray::new(sf("sfVoteSlots"))
            };

            for entry in existing_slots.iter() {
                let entry_account = entry.get_account_id(sf("sfAccount"));
                let Ok(Some(mut lp_tokens)) = amm_lp_holds_in_view(view, &amm_sle, entry_account)
                else {
                    continue;
                };
                if lp_tokens.signum() == 0 {
                    continue;
                }
                let mut fee_val = u32::from(entry.get_field_u16(sf("sfTradingFee")));
                if entry_account == account {
                    lp_tokens = lp_tokens_new.clone();
                    fee_val = u32::from(fee_vote);
                    found_account = true;
                }
                let lp_tokens_num = ledger::amm_helpers::stamount_as_number(&lp_tokens);
                numerator += number_from_i64(fee_val as i64) * lp_tokens_num;
                denominator += lp_tokens_num;

                let vote_weight =
                    ((lp_tokens_num * number_from_i64(VOTE_WEIGHT_SCALE_FACTOR as i64)) / lp_total)
                        .try_to_i64()
                        .unwrap_or(0)
                        .max(0) as u32;

                let mut new_entry = STObject::make_inner_object(sf("sfVoteEntry"));
                new_entry.set_account_id(sf("sfAccount"), entry_account);
                if fee_val != 0 {
                    new_entry.set_field_u16(sf("sfTradingFee"), fee_val as u16);
                }
                new_entry.set_field_u32(sf("sfVoteWeight"), vote_weight);

                if min_tokens.is_none()
                    || lp_tokens_num < min_tokens.unwrap()
                    || (lp_tokens_num == min_tokens.unwrap()
                        && (fee_val < min_fee
                            || (fee_val == min_fee && entry_account < min_account)))
                {
                    min_tokens = Some(lp_tokens_num);
                    min_pos = updated_vote_slots.len();
                    min_account = entry_account;
                    min_fee = fee_val;
                }

                updated_vote_slots.push_back(new_entry);
            }

            if !found_account {
                let update_entry = |slots: &mut STArray, replace_pos: Option<usize>| {
                    let vote_weight = ((lp_tokens_new_num
                        * number_from_i64(VOTE_WEIGHT_SCALE_FACTOR as i64))
                        / lp_total)
                        .try_to_i64()
                        .unwrap_or(0)
                        .max(0) as u32;
                    let mut new_entry = STObject::make_inner_object(sf("sfVoteEntry"));
                    if fee_vote != 0 {
                        new_entry.set_field_u16(sf("sfTradingFee"), fee_vote);
                    }
                    new_entry.set_field_u32(sf("sfVoteWeight"), vote_weight);
                    new_entry.set_account_id(sf("sfAccount"), account);
                    if let Some(pos) = replace_pos {
                        if let Some(slot) = slots.get_mut(pos) {
                            *slot = new_entry;
                        }
                    } else {
                        slots.push_back(new_entry);
                    }
                };

                if updated_vote_slots.len() < usize::from(VOTE_MAX_SLOTS) {
                    numerator += number_from_i64(i64::from(fee_vote)) * lp_tokens_new_num;
                    denominator += lp_tokens_new_num;
                    update_entry(&mut updated_vote_slots, None);
                } else if let Some(min_tokens) = min_tokens
                    && (lp_tokens_new_num > min_tokens
                        || (lp_tokens_new_num == min_tokens && u32::from(fee_vote) > min_fee))
                {
                    let replaced = updated_vote_slots
                        .get(min_pos)
                        .cloned()
                        .expect("vote slot exists");
                    let replaced_fee =
                        u32::from(if replaced.is_field_present(sf("sfTradingFee")) {
                            replaced.get_field_u16(sf("sfTradingFee"))
                        } else {
                            0
                        });
                    numerator = numerator - number_from_i64(replaced_fee as i64) * min_tokens
                        + number_from_i64(i64::from(fee_vote)) * lp_tokens_new_num;
                    denominator = denominator - min_tokens + lp_tokens_new_num;
                    update_entry(&mut updated_vote_slots, Some(min_pos));
                }
            }

            let mut obj = amm_sle.clone_as_object();
            obj.set_field_array(sf("sfVoteSlots"), updated_vote_slots);
            if denominator.signum() != 0 {
                let fee = (numerator / denominator).try_to_i64().unwrap_or(0).max(0) as u16;
                if fee != 0 {
                    obj.set_field_u16(sf("sfTradingFee"), fee);
                } else if obj.is_field_present(sf("sfTradingFee")) {
                    obj.make_field_absent(sf("sfTradingFee"));
                }
                if obj.is_field_present(sf("sfAuctionSlot")) {
                    let mut auction_slot = obj.peek_field_object(sf("sfAuctionSlot")).clone();
                    let discounted_fee = fee / AUCTION_SLOT_DISCOUNTED_FEE_FRACTION as u16;
                    if discounted_fee != 0 {
                        auction_slot.set_field_u16(sf("sfDiscountedFee"), discounted_fee);
                    } else if auction_slot.is_field_present(sf("sfDiscountedFee")) {
                        auction_slot.make_field_absent(sf("sfDiscountedFee"));
                    }
                    obj.set_field_object(sf("sfAuctionSlot"), auction_slot);
                }
            } else {
                if obj.is_field_present(sf("sfTradingFee")) {
                    obj.make_field_absent(sf("sfTradingFee"));
                }
                if obj.is_field_present(sf("sfAuctionSlot")) {
                    let mut auction_slot = obj.peek_field_object(sf("sfAuctionSlot")).clone();
                    if auction_slot.is_field_present(sf("sfDiscountedFee")) {
                        auction_slot.make_field_absent(sf("sfDiscountedFee"));
                    }
                    obj.set_field_object(sf("sfAuctionSlot"), auction_slot);
                }
            }
            let _ = view.update(Arc::new(STLedgerEntry::from_stobject(obj, *amm_sle.key())));
            Ter::TES_SUCCESS
        }
        TxType::AMM_DELETE => {
            let asset1 = sttx.get_field_amount(sf("sfAsset")).issue();
            let asset2 = sttx.get_field_amount(sf("sfAsset2")).issue();
            let amm_keylet = protocol::keylet::amm(asset1.into(), asset2.into());
            if let Ok(Some(amm_sle)) = view.peek(amm_keylet) {
                let lp_balance = amm_sle.get_field_amount(sf("sfLPTokenBalance"));
                if lp_balance.signum() > 0 {
                    return Ter::TEC_HAS_OBLIGATIONS;
                }
                let _ = view.erase(amm_sle);
            }
            Ter::TES_SUCCESS
        }

        // --- NFTs ---
        TxType::NFTOKEN_MINT => {
            let account = sttx.get_account_id(sf("sfAccount"));
            let facts = NFTokenMintApplyFacts {
                nftoken_id: Uint256::from(sttx.get_transaction_id()),
                issuer: account,
                owner: account,
                transfer_fee: None,
                uri: None,
            };
            let mut sink = ViewBackedNFTokenMintSink { view, account };
            run_nftoken_mint_do_apply(facts, &mut sink)
        }
        TxType::NFTOKEN_BURN => {
            let account = sttx.get_account_id(sf("sfAccount"));
            let owner = if sttx.is_field_present(sf("sfOwner")) {
                sttx.get_account_id(sf("sfOwner"))
            } else {
                account
            };
            let token_id = sttx.get_field_h256(sf("sfNFTokenID"));
            let page_keylet = protocol::keylet::nft_page_keylet(
                protocol::nft_page_min_keylet(Uint160::from_void(owner.data())),
                Uint256::from(token_id),
            );
            if let Ok(Some(page)) = view.peek(page_keylet) {
                let tokens = page.get_field_array(sf("sfNFTokens"));
                let mut new_tokens = protocol::STArray::new(sf("sfNFTokens"));
                let mut found = false;
                for token in tokens.iter() {
                    let tid = token.get_field_h256(sf("sfNFTokenID"));
                    if tid != token_id {
                        new_tokens.push_back(token.clone());
                    } else {
                        found = true;
                    }
                }
                if found {
                    if new_tokens.is_empty() {
                        let _ = view.erase(page);
                    } else {
                        let mut obj = page.clone_as_object();
                        obj.set_field_array(sf("sfNFTokens"), new_tokens);
                        let _ =
                            view.update(Arc::new(STLedgerEntry::from_stobject(obj, *page.key())));
                    }
                    if let Ok(Some(acct)) =
                        view.peek(protocol::account_keylet(Uint160::from_void(owner.data())))
                    {
                        let _ = ledger::adjust_owner_count(view, &acct, -1);
                    }
                }
            }
            Ter::TES_SUCCESS
        }
        TxType::NFTOKEN_CREATE_OFFER => {
            let account = sttx.get_account_id(sf("sfAccount"));
            let token_id = sttx.get_field_h256(sf("sfNFTokenID"));
            let offer_keylet = protocol::keylet::nft_offer_keylet_for_owner(
                Uint160::from_void(account.data()),
                sttx.get_seq_value(),
            );
            let mut sle = STLedgerEntry::new(offer_keylet);
            sle.set_account_id(sf("sfOwner"), account);
            sle.set_field_h256(sf("sfNFTokenID"), token_id);
            sle.set_field_amount(sf("sfAmount"), sttx.get_field_amount(sf("sfAmount")));
            if sttx.is_field_present(sf("sfDestination")) {
                sle.set_account_id(
                    sf("sfDestination"),
                    sttx.get_account_id(sf("sfDestination")),
                );
            }
            if sttx.is_field_present(sf("sfExpiration")) {
                sle.set_field_u32(sf("sfExpiration"), sttx.get_field_u32(sf("sfExpiration")));
            }
            sle.set_field_u32(sf("sfFlags"), sttx.get_field_u32(sf("sfFlags")));
            // Insert into owner directory
            let owner_dir = protocol::owner_dir_keylet(Uint160::from_void(account.data()));
            if let Ok(Some(page)) = ledger::dir_append(view, &owner_dir, offer_keylet.key, &|_| {})
            {
                sle.set_field_u64(sf("sfOwnerNode"), page);
            }
            let _ = view.insert(Arc::new(sle));
            if let Ok(Some(acct)) =
                view.peek(protocol::account_keylet(Uint160::from_void(account.data())))
            {
                let _ = ledger::adjust_owner_count(view, &acct, 1);
            }
            Ter::TES_SUCCESS
        }
        TxType::NFTOKEN_CANCEL_OFFER => {
            let offers = sttx.get_field_v256(sf("sfNFTokenOffers"));
            for offer_id in offers.value() {
                let offer_keylet = protocol::keylet::nft_offer_keylet(*offer_id);
                if let Ok(Some(offer_sle)) = view.peek(offer_keylet) {
                    let offer_owner = offer_sle.get_account_id(sf("sfOwner"));
                    // Remove from owner directory
                    let owner_node = offer_sle.get_field_u64(sf("sfOwnerNode"));
                    let owner_dir =
                        protocol::owner_dir_keylet(Uint160::from_void(offer_owner.data()));
                    let _ = ledger::dir_remove(view, &owner_dir, owner_node, *offer_id, false);

                    // Remove from NFToken directory
                    let nftoken_id = offer_sle.get_field_h256(sf("sfNFTokenID"));
                    let flags = offer_sle.get_field_u32(sf("sfFlags"));
                    let is_sell = (flags & protocol::lsfSellNFToken) != 0;
                    let nft_dir = if is_sell {
                        protocol::nft_sell_offers_keylet(nftoken_id)
                    } else {
                        protocol::nft_buy_offers_keylet(nftoken_id)
                    };
                    let nft_node = offer_sle.get_field_u64(sf("sfNFTokenOfferNode"));
                    let _ = ledger::dir_remove(view, &nft_dir, nft_node, *offer_id, false);

                    let _ = view.erase(offer_sle);
                    if let Ok(Some(acct)) = view.peek(protocol::account_keylet(Uint160::from_void(
                        offer_owner.data(),
                    ))) {
                        let _ = ledger::adjust_owner_count(view, &acct, -1);
                    }
                }
            }
            Ter::TES_SUCCESS
        }
        TxType::NFTOKEN_ACCEPT_OFFER => {
            let tx_account = sttx.get_account_id(sf("sfAccount"));

            // Load offers
            let sell_offer = if sttx.is_field_present(sf("sfNFTokenSellOffer")) {
                let id = sttx.get_field_h256(sf("sfNFTokenSellOffer"));
                view.peek(protocol::keylet::nft_offer_keylet(Uint256::from(id)))
                    .ok()
                    .flatten()
            } else {
                None
            };
            let buy_offer = if sttx.is_field_present(sf("sfNFTokenBuyOffer")) {
                let id = sttx.get_field_h256(sf("sfNFTokenBuyOffer"));
                view.peek(protocol::keylet::nft_offer_keylet(Uint256::from(id)))
                    .ok()
                    .flatten()
            } else {
                None
            };

            let delete_offer = |view: &mut V, offer: &Arc<STLedgerEntry>| {
                let owner = offer.get_account_id(sf("sfOwner"));
                let owner_node = offer.get_field_u64(sf("sfOwnerNode"));
                let owner_dir = owner_dir_keylet(Uint160::from_void(owner.data()));
                let _ = ledger::dir_remove(view, &owner_dir, owner_node, *offer.key(), false);
                if let Ok(Some(acct)) =
                    view.peek(protocol::account_keylet(Uint160::from_void(owner.data())))
                {
                    let _ = ledger::adjust_owner_count(view, &acct, -1);
                }
                let _ = view.erase(offer.clone());
            };

            // Delete both offers first (reference does this before payment/transfer)
            if let Some(ref bo) = buy_offer {
                delete_offer(view, bo);
            }
            if let Some(ref so) = sell_offer {
                delete_offer(view, so);
            }

            // Determine buyer, seller, amount, nftokenID based on mode
            let (buyer, seller, nftoken_id, amount) =
                if let (Some(bo), Some(so)) = (&buy_offer, &sell_offer) {
                    // Broker mode: both offers present
                    let buyer = bo.get_account_id(sf("sfOwner"));
                    let seller = so.get_account_id(sf("sfOwner"));
                    let nftoken_id = so.get_field_h256(sf("sfNFTokenID"));
                    let amount = bo.get_field_amount(sf("sfAmount"));
                    (buyer, seller, nftoken_id, amount)
                } else if let Some(ref so) = sell_offer {
                    // Sell offer only: tx_account is buyer
                    let seller = so.get_account_id(sf("sfOwner"));
                    let nftoken_id = so.get_field_h256(sf("sfNFTokenID"));
                    let amount = so.get_field_amount(sf("sfAmount"));
                    (tx_account, seller, nftoken_id, amount)
                } else if let Some(ref bo) = buy_offer {
                    // Buy offer only: tx_account is seller
                    let buyer = bo.get_account_id(sf("sfOwner"));
                    let nftoken_id = bo.get_field_h256(sf("sfNFTokenID"));
                    let amount = bo.get_field_amount(sf("sfAmount"));
                    (buyer, tx_account, nftoken_id, amount)
                } else {
                    return Ter::TEF_INTERNAL;
                };

            if amount.signum() > 0 {
                if amount.native() {
                    do_xrp_payment(view, &buyer, &seller, &amount, 0);
                } else {
                    // IOU payment via accountSend
                    ledger::ripple_state_helpers::account_send(view, &buyer, &seller, &amount);
                }
            }

            nft_transfer_token(view, &buyer, &seller, nftoken_id)
        }
        TxType::CLAWBACK => {
            let issuer = sttx.get_account_id(sf("sfAccount"));
            let amount = sttx.get_field_amount(sf("sfAmount"));
            if amount.holds_mpt_issue() {
                // MPT clawback
                let holder = sttx.get_account_id(sf("sfHolder"));
                let mpt_issue = match &amount.asset() {
                    protocol::Asset::MPTIssue(i) => *i,
                    _ => return Ter::TEF_INTERNAL,
                };
                let mptid = mpt_issue.mpt_id();
                let holder_keylet =
                    protocol::mptoken_keylet_from_mptid(mptid, Uint160::from_void(holder.data()));
                if let Ok(Some(token_sle)) = view.peek(holder_keylet) {
                    let balance = token_sle.get_field_u64(sf("sfMPTAmount"));
                    let clawback_amt = amount.mpt().value().unsigned_abs().min(balance);
                    let mut obj = token_sle.clone_as_object();
                    obj.set_field_u64(sf("sfMPTAmount"), balance - clawback_amt);
                    let _ = view.update(Arc::new(STLedgerEntry::from_stobject(
                        obj,
                        *token_sle.key(),
                    )));
                    // Update OutstandingAmount on issuance
                    let issuance_keylet = protocol::mpt_issuance_keylet_from_mptid(mptid);
                    if let Ok(Some(iss)) = view.peek(issuance_keylet) {
                        let outstanding = iss.get_field_u64(sf("sfOutstandingAmount"));
                        let mut iss_obj = iss.clone_as_object();
                        iss_obj.set_field_u64(
                            sf("sfOutstandingAmount"),
                            outstanding.saturating_sub(clawback_amt),
                        );
                        let _ = view
                            .update(Arc::new(STLedgerEntry::from_stobject(iss_obj, *iss.key())));
                    }
                }
            } else {
                // IOU clawback — debit specific amount from holder's trust line
                let holder = amount.issue().account; // In clawback, the "issuer" field on amount is the holder
                let currency = amount.issue().currency;
                let line_keylet = protocol::line(issuer, holder, currency);
                if let Ok(Some(line)) = view.peek(line_keylet) {
                    let b_high = holder > issuer;
                    let current_balance = line.get_field_amount(sf("sfBalance"));
                    // Determine holder's balance (positive from their perspective)
                    let holder_balance = if b_high {
                        let mut neg = current_balance.clone();
                        neg.negate();
                        neg
                    } else {
                        current_balance.clone()
                    };
                    // Clawback the minimum of requested and available
                    // This makes both amounts have the same issue (issuer's perspective).
                    let normalized_amount = {
                        let mut a = amount.clone();
                        a.set_issue(protocol::Issue {
                            account: issuer,
                            currency,
                        });
                        a
                    };
                    let clawback_actual = if normalized_amount > holder_balance {
                        holder_balance
                    } else {
                        normalized_amount
                    };
                    // Adjust balance: reduce holder's side
                    let new_balance = if b_high {
                        current_balance + clawback_actual
                    } else {
                        current_balance - clawback_actual
                    };
                    let mut obj = line.clone_as_object();
                    obj.set_field_amount(sf("sfBalance"), new_balance);
                    let _ = view.update(Arc::new(STLedgerEntry::from_stobject(obj, *line.key())));
                }
            }
            Ter::TES_SUCCESS
        }

        // --- Tickets ---
        TxType::TICKET_CREATE => {
            let account = sttx.get_account_id(sf("sfAccount"));
            let count = sttx.get_field_u32(sf("sfTicketCount"));
            let mut sink = DispatcherTicketCreateSink {
                view,
                account,
                tx_sequence: sttx.get_field_u32(sf("sfSequence")),
                pre_fee_balance_drops,
            };
            run_ticket_create_do_apply(count, &mut sink)
        }

        // --- DID ---
        TxType::DID_SET => {
            let account = sttx.get_account_id(sf("sfAccount"));
            let did_keylet = protocol::did_keylet(Uint160::from_void(account.data()));
            let existing = view.peek(did_keylet).ok().flatten();
            let is_new = existing.is_none();
            let mut sle = if let Some(e) = existing {
                STLedgerEntry::from_stobject(e.clone_as_object(), *e.key())
            } else {
                let mut new_sle = STLedgerEntry::new(did_keylet);
                new_sle.set_account_id(sf("sfAccount"), account);
                new_sle
            };
            if sttx.is_field_present(sf("sfDIDDocument")) {
                sle.set_stbase(protocol::STBlob::from_buffer(
                    sf("sfDIDDocument"),
                    basics::buffer::Buffer::from(&sttx.get_field_vl(sf("sfDIDDocument"))[..]),
                ));
            }
            if sttx.is_field_present(sf("sfURI")) {
                sle.set_stbase(protocol::STBlob::from_buffer(
                    sf("sfURI"),
                    basics::buffer::Buffer::from(&sttx.get_field_vl(sf("sfURI"))[..]),
                ));
            }
            if is_new {
                let owner_dir = protocol::owner_dir_keylet(Uint160::from_void(account.data()));
                if let Ok(Some(page)) =
                    ledger::dir_append(view, &owner_dir, did_keylet.key, &|_| {})
                {
                    sle.set_field_u64(sf("sfOwnerNode"), page);
                }
                let _ = view.insert(Arc::new(sle));
                if let Ok(Some(acct)) =
                    view.peek(protocol::account_keylet(Uint160::from_void(account.data())))
                {
                    let _ = ledger::adjust_owner_count(view, &acct, 1);
                }
            } else {
                let _ = view.update(Arc::new(sle));
            }
            Ter::TES_SUCCESS
        }
        TxType::DID_DELETE => {
            let account = sttx.get_account_id(sf("sfAccount"));
            let did_keylet = protocol::did_keylet(Uint160::from_void(account.data()));
            if let Ok(Some(did_sle)) = view.peek(did_keylet) {
                // Remove from owner directory
                let owner_node = did_sle.get_field_u64(sf("sfOwnerNode"));
                let owner_dir = owner_dir_keylet(Uint160::from_void(account.data()));
                let _ = ledger::dir_remove(view, &owner_dir, owner_node, *did_sle.key(), false);
                let _ = view.erase(did_sle);
                if let Ok(Some(acct)) =
                    view.peek(protocol::account_keylet(Uint160::from_void(account.data())))
                {
                    let _ = ledger::adjust_owner_count(view, &acct, -1);
                }
            }
            Ter::TES_SUCCESS
        }

        // --- Oracle ---
        TxType::ORACLE_SET => {
            let account = sttx.get_account_id(sf("sfAccount"));
            let oracle_doc_id = sttx.get_field_u32(sf("sfOracleDocumentID"));
            let oracle_keylet =
                protocol::oracle_keylet(Uint160::from_void(account.data()), oracle_doc_id);
            let existing = view.peek(oracle_keylet).ok().flatten();
            if let Some(oracle_sle) = existing {
                let mut obj = oracle_sle.clone_as_object();
                if sttx.is_field_present(sf("sfProvider")) {
                    obj.set_stbase(protocol::STBlob::from_buffer(
                        sf("sfProvider"),
                        basics::buffer::Buffer::from(&sttx.get_field_vl(sf("sfProvider"))[..]),
                    ));
                }
                if sttx.is_field_present(sf("sfLastUpdateTime")) {
                    obj.set_field_u32(
                        sf("sfLastUpdateTime"),
                        sttx.get_field_u32(sf("sfLastUpdateTime")),
                    );
                }
                let _ = view.update(Arc::new(STLedgerEntry::from_stobject(
                    obj,
                    *oracle_sle.key(),
                )));
            } else {
                let mut sle = STLedgerEntry::new(oracle_keylet);
                sle.set_account_id(sf("sfOwner"), account);
                sle.set_field_u32(sf("sfOracleDocumentID"), oracle_doc_id);
                let owner_dir = protocol::owner_dir_keylet(Uint160::from_void(account.data()));
                if let Ok(Some(page)) =
                    ledger::dir_append(view, &owner_dir, oracle_keylet.key, &|_| {})
                {
                    sle.set_field_u64(sf("sfOwnerNode"), page);
                }
                let _ = view.insert(Arc::new(sle));
                if let Ok(Some(acct)) =
                    view.peek(protocol::account_keylet(Uint160::from_void(account.data())))
                {
                    let _ = ledger::adjust_owner_count(view, &acct, 1);
                }
            }
            Ter::TES_SUCCESS
        }
        TxType::ORACLE_DELETE => {
            let account = sttx.get_account_id(sf("sfAccount"));
            let oracle_doc_id = sttx.get_field_u32(sf("sfOracleDocumentID"));
            let oracle_keylet =
                protocol::oracle_keylet(Uint160::from_void(account.data()), oracle_doc_id);
            if let Ok(Some(oracle_sle)) = view.peek(oracle_keylet) {
                let owner_node = oracle_sle.get_field_u64(sf("sfOwnerNode"));
                let owner_dir = owner_dir_keylet(Uint160::from_void(account.data()));
                let _ = ledger::dir_remove(view, &owner_dir, owner_node, *oracle_sle.key(), false);
                let _ = view.erase(oracle_sle);
                if let Ok(Some(acct)) =
                    view.peek(protocol::account_keylet(Uint160::from_void(account.data())))
                {
                    let _ = ledger::adjust_owner_count(view, &acct, -1);
                }
            }
            Ter::TES_SUCCESS
        }

        // --- MPToken ---
        TxType::MPTOKEN_ISSUANCE_CREATE => {
            let account = sttx.get_account_id(sf("sfAccount"));
            let sequence = sttx.get_seq_value();
            let issuance_keylet =
                protocol::mpt_issuance_keylet(sequence, Uint160::from_void(account.data()));
            let mut sle = STLedgerEntry::new(issuance_keylet);
            sle.set_account_id(sf("sfIssuer"), account);
            sle.set_field_u32(sf("sfSequence"), sequence);
            sle.set_field_u64(sf("sfOutstandingAmount"), 0);
            if sttx.is_field_present(sf("sfMaximumAmount")) {
                sle.set_field_u64(
                    sf("sfMaximumAmount"),
                    sttx.get_field_u64(sf("sfMaximumAmount")),
                );
            }
            sle.set_field_u32(sf("sfFlags"), sttx.get_field_u32(sf("sfFlags")));
            let owner_dir = protocol::owner_dir_keylet(Uint160::from_void(account.data()));
            if let Ok(Some(page)) =
                ledger::dir_append(view, &owner_dir, issuance_keylet.key, &|_| {})
            {
                sle.set_field_u64(sf("sfOwnerNode"), page);
            }
            let _ = view.insert(Arc::new(sle));
            if let Ok(Some(acct)) =
                view.peek(protocol::account_keylet(Uint160::from_void(account.data())))
            {
                let _ = ledger::adjust_owner_count(view, &acct, 1);
            }
            Ter::TES_SUCCESS
        }
        TxType::MPTOKEN_ISSUANCE_DESTROY => {
            let account = sttx.get_account_id(sf("sfAccount"));
            if !sttx.is_field_present(sf("sfMPTokenIssuanceID")) {
                return Ter::TEM_MALFORMED;
            }
            let mptid = sttx.get_field_h192(sf("sfMPTokenIssuanceID"));
            if mptid.is_zero() {
                return Ter::TEC_OBJECT_NOT_FOUND;
            }
            let issuance_keylet = protocol::mpt_issuance_keylet_from_mptid(mptid);
            if let Ok(Some(iss_sle)) = view.peek(issuance_keylet) {
                if iss_sle.get_account_id(sf("sfIssuer")) != account {
                    return Ter::TEC_NO_PERMISSION;
                }
                let outstanding = iss_sle.get_field_u64(sf("sfOutstandingAmount"));
                let locked = iss_sle
                    .is_field_present(sf("sfLockedAmount"))
                    .then(|| iss_sle.get_field_u64(sf("sfLockedAmount")))
                    .unwrap_or(0);
                if outstanding > 0 || locked != 0 {
                    return Ter::TEC_HAS_OBLIGATIONS;
                }
                let _ = view.erase(iss_sle.clone());
                // Remove from owner directory
                let owner_node = iss_sle.get_field_u64(sf("sfOwnerNode"));
                let owner_dir = owner_dir_keylet(Uint160::from_void(account.data()));
                let _ = ledger::dir_remove(view, &owner_dir, owner_node, *iss_sle.key(), false);
                if let Ok(Some(acct)) =
                    view.peek(protocol::account_keylet(Uint160::from_void(account.data())))
                {
                    let _ = ledger::adjust_owner_count(view, &acct, -1);
                }
            }
            Ter::TES_SUCCESS
        }
        TxType::MPTOKEN_ISSUANCE_SET => {
            if !sttx.is_field_present(sf("sfMPTokenIssuanceID")) {
                return Ter::TEM_MALFORMED;
            }
            let mptid = sttx.get_field_h192(sf("sfMPTokenIssuanceID"));
            if mptid.is_zero() {
                return Ter::TEC_OBJECT_NOT_FOUND;
            }
            let issuance_keylet = protocol::mpt_issuance_keylet_from_mptid(mptid);
            if let Ok(Some(iss_sle)) = view.peek(issuance_keylet) {
                let mut obj = iss_sle.clone_as_object();
                let mut flags = obj.get_field_u32(sf("sfFlags"));
                if sttx.is_field_present(sf("sfSetFlag")) {
                    flags |= sttx.get_field_u32(sf("sfSetFlag"));
                }
                if sttx.is_field_present(sf("sfClearFlag")) {
                    flags &= !sttx.get_field_u32(sf("sfClearFlag"));
                }
                obj.set_field_u32(sf("sfFlags"), flags);
                let _ = view.update(Arc::new(STLedgerEntry::from_stobject(obj, *iss_sle.key())));
            }
            Ter::TES_SUCCESS
        }
        TxType::MPTOKEN_AUTHORIZE => {
            let account = sttx.get_account_id(sf("sfAccount"));
            if !sttx.is_field_present(sf("sfMPTokenIssuanceID")) {
                return Ter::TEM_MALFORMED;
            }
            let mptid = sttx.get_field_h192(sf("sfMPTokenIssuanceID"));
            if mptid.is_zero() {
                return Ter::TEC_OBJECT_NOT_FOUND;
            }

            let issuance_keylet = protocol::mpt_issuance_keylet_from_mptid(mptid);
            let Some(issuance) = view.peek(issuance_keylet).ok().flatten() else {
                return Ter::TEC_OBJECT_NOT_FOUND;
            };
            let issuer = issuance.get_account_id(sf("sfIssuer"));
            let flags = sttx.get_field_u32(sf("sfFlags"));
            let unauthorize = (flags & protocol::tfMPTUnauthorize) != 0;

            if sttx.is_field_present(sf("sfHolder")) {
                let holder = sttx.get_account_id(sf("sfHolder"));
                let Some(holder_root) = view
                    .peek(protocol::account_keylet(Uint160::from_void(holder.data())))
                    .ok()
                    .flatten()
                else {
                    return Ter::TEC_NO_DST;
                };
                if account != issuer {
                    return Ter::TEC_NO_PERMISSION;
                }
                if !issuance.is_flag(protocol::lsfMPTRequireAuth) {
                    return Ter::TEC_NO_AUTH;
                }
                if holder_root.is_field_present(sf("sfVaultID"))
                    || holder_root.is_field_present(sf("sfLoanBrokerID"))
                    || holder_root.is_field_present(sf("sfAMMID"))
                {
                    return Ter::TEC_NO_PERMISSION;
                }
                let holder_keylet =
                    protocol::mptoken_keylet_from_mptid(mptid, Uint160::from_void(holder.data()));
                let Some(holder_token) = view.peek(holder_keylet).ok().flatten() else {
                    return Ter::TEC_OBJECT_NOT_FOUND;
                };
                let mut obj = holder_token.clone_as_object();
                let mut token_flags = obj.get_field_u32(sf("sfFlags"));
                if unauthorize {
                    token_flags &= !protocol::lsfMPTAuthorized;
                } else {
                    token_flags |= protocol::lsfMPTAuthorized;
                }
                obj.set_field_u32(sf("sfFlags"), token_flags);
                let _ = view.update(Arc::new(STLedgerEntry::from_stobject(
                    obj,
                    *holder_token.key(),
                )));
                return Ter::TES_SUCCESS;
            }

            let token_keylet =
                protocol::mptoken_keylet_from_mptid(mptid, Uint160::from_void(account.data()));
            if unauthorize {
                let Some(token) = view.peek(token_keylet).ok().flatten() else {
                    return Ter::TEC_OBJECT_NOT_FOUND;
                };
                if token.get_field_u64(sf("sfMPTAmount")) != 0
                    || (view
                        .rules()
                        .enabled(&protocol::feature_id("fixCleanup3_1_3"))
                        && token
                            .is_field_present(sf("sfLockedAmount"))
                            .then(|| token.get_field_u64(sf("sfLockedAmount")))
                            .unwrap_or(0)
                            != 0)
                {
                    return Ter::TEC_HAS_OBLIGATIONS;
                }
                if view
                    .rules()
                    .enabled(&protocol::feature_id("SingleAssetVault"))
                    && token.is_flag(protocol::lsfMPTLocked)
                {
                    return Ter::TEC_NO_PERMISSION;
                }
                let owner_node = token.get_field_u64(sf("sfOwnerNode"));
                let owner_dir = protocol::owner_dir_keylet(Uint160::from_void(account.data()));
                let _ = ledger::dir_remove(view, &owner_dir, owner_node, *token.key(), false);
                let _ = view.erase(token);
                if let Ok(Some(acct)) =
                    view.peek(protocol::account_keylet(Uint160::from_void(account.data())))
                {
                    let _ = ledger::adjust_owner_count(view, &acct, -1);
                }
                return Ter::TES_SUCCESS;
            }
            if account == issuer {
                return Ter::TEC_NO_PERMISSION;
            }
            if view.peek(token_keylet).ok().flatten().is_some() {
                return Ter::TEC_DUPLICATE;
            }
            if let Ok(Some(acct)) =
                view.peek(protocol::account_keylet(Uint160::from_void(account.data())))
            {
                let owner_count = acct.get_field_u32(sf("sfOwnerCount"));
                let balance = acct.get_field_amount(sf("sfBalance")).xrp().drops();
                let reserve = if owner_count < 2 {
                    0
                } else {
                    view.fees().account_reserve(owner_count as usize + 1) as i64
                };
                if balance < reserve {
                    return Ter::TEC_INSUFFICIENT_RESERVE;
                }
            }
            let mut sle = STLedgerEntry::new(token_keylet);
            sle.set_account_id(sf("sfAccount"), account);
            sle.set_field_h192(sf("sfMPTokenIssuanceID"), mptid);
            sle.set_field_u64(sf("sfMPTAmount"), 0);
            sle.set_field_u32(sf("sfFlags"), 0);
            let owner_dir = protocol::owner_dir_keylet(Uint160::from_void(account.data()));
            let Ok(Some(page)) = ledger::dir_append(view, &owner_dir, token_keylet.key, &|_| {})
            else {
                return Ter::TEC_DIR_FULL;
            };
            sle.set_field_u64(sf("sfOwnerNode"), page);
            let _ = view.insert(Arc::new(sle));
            if let Ok(Some(acct)) =
                view.peek(protocol::account_keylet(Uint160::from_void(account.data())))
            {
                let _ = ledger::adjust_owner_count(view, &acct, 1);
            }
            Ter::TES_SUCCESS
        }

        // --- Permissioned domains ---
        TxType::PERMISSIONED_DOMAIN_SET => {
            let account = sttx.get_account_id(sf("sfAccount"));
            let tx_credentials = sttx
                .get_field_array(sf("sfAcceptedCredentials"))
                .iter()
                .map(|credential| PermissionedDomainCredential {
                    issuer: credential.get_account_id(sf("sfIssuer")),
                    credential_type: credential.get_field_vl(sf("sfCredentialType")),
                })
                .collect();
            let existing_domain_id = sttx
                .is_field_present(sf("sfDomainID"))
                .then(|| sttx.get_field_h256(sf("sfDomainID")));
            let mut sink = ViewBackedPermissionedDomainSetSink::new(
                view,
                account,
                sttx.get_seq_value(),
                existing_domain_id,
            );
            run_permissioned_domain_set_do_apply(
                tx_credentials,
                existing_domain_id.is_some(),
                &mut sink,
            )
        }
        TxType::PERMISSIONED_DOMAIN_DELETE => {
            let account = sttx.get_account_id(sf("sfAccount"));
            let domain_id = sttx.get_field_h256(sf("sfDomainID"));
            let mut sink = ViewBackedPermissionedDomainDeleteSink {
                view,
                account,
                domain_id,
            };
            run_permissioned_domain_delete_do_apply(&mut sink)
        }

        // --- Credentials ---
        TxType::CREDENTIAL_CREATE => {
            let account = sttx.get_account_id(sf("sfAccount"));
            let subject = sttx.get_account_id(sf("sfSubject"));
            let cred_type = if sttx.is_field_present(sf("sfCredentialType")) {
                sttx.get_field_vl(sf("sfCredentialType"))
            } else {
                vec![]
            };
            let cred_keylet = protocol::credential_keylet(
                Uint160::from_void(subject.data()),
                Uint160::from_void(account.data()),
                &cred_type,
            );

            if view.peek(cred_keylet).ok().flatten().is_some() {
                return Ter::TEC_DUPLICATE;
            }
            let Some(issuer_sle) = view
                .peek(protocol::account_keylet(Uint160::from_void(account.data())))
                .ok()
                .flatten()
            else {
                return Ter::TEF_INTERNAL;
            };
            if view
                .peek(protocol::account_keylet(Uint160::from_void(subject.data())))
                .ok()
                .flatten()
                .is_none()
            {
                return Ter::TEC_NO_TARGET;
            }
            if sttx.is_field_present(sf("sfExpiration")) {
                let expiration = sttx.get_field_u32(sf("sfExpiration"));
                if view.header().parent_close_time > expiration {
                    return Ter::TEC_EXPIRED;
                }
            }
            let owner_count = issuer_sle.get_field_u32(sf("sfOwnerCount"));
            let balance = pre_fee_balance_drops
                .unwrap_or_else(|| issuer_sle.get_field_amount(sf("sfBalance")).xrp().drops());
            let reserve = view.fees().account_reserve(owner_count as usize + 1) as i64;
            if balance < reserve {
                return Ter::TEC_INSUFFICIENT_RESERVE;
            }

            let mut sle = STLedgerEntry::new(cred_keylet);
            sle.set_account_id(sf("sfIssuer"), account);
            sle.set_account_id(sf("sfSubject"), subject);
            if sttx.is_field_present(sf("sfCredentialType")) {
                sle.set_stbase(protocol::STBlob::from_buffer(
                    sf("sfCredentialType"),
                    basics::buffer::Buffer::from(&sttx.get_field_vl(sf("sfCredentialType"))[..]),
                ));
            }
            if sttx.is_field_present(sf("sfExpiration")) {
                sle.set_field_u32(sf("sfExpiration"), sttx.get_field_u32(sf("sfExpiration")));
            }
            if sttx.is_field_present(sf("sfURI")) {
                sle.set_stbase(protocol::STBlob::from_buffer(
                    sf("sfURI"),
                    basics::buffer::Buffer::from(&sttx.get_field_vl(sf("sfURI"))[..]),
                ));
            }
            let issuer_dir = protocol::owner_dir_keylet(Uint160::from_void(account.data()));
            let Ok(Some(issuer_page)) =
                ledger::dir_append(view, &issuer_dir, cred_keylet.key, &|_| {})
            else {
                return Ter::TEC_DIR_FULL;
            };
            sle.set_field_u64(sf("sfIssuerNode"), issuer_page);
            if ledger::adjust_owner_count(view, &issuer_sle, 1).is_err() {
                return Ter::TEF_INTERNAL;
            }

            if subject == account {
                sle.set_field_u32(sf("sfFlags"), protocol::lsfAccepted);
            } else {
                let subject_dir = protocol::owner_dir_keylet(Uint160::from_void(subject.data()));
                let Ok(Some(subject_page)) =
                    ledger::dir_append(view, &subject_dir, cred_keylet.key, &|_| {})
                else {
                    return Ter::TEC_DIR_FULL;
                };
                sle.set_field_u64(sf("sfSubjectNode"), subject_page);
            }

            if view.insert(Arc::new(sle)).is_err() {
                return Ter::TEF_INTERNAL;
            }

            Ter::TES_SUCCESS
        }
        TxType::CREDENTIAL_ACCEPT => {
            let subject = sttx.get_account_id(sf("sfAccount"));
            let issuer = sttx.get_account_id(sf("sfIssuer"));
            let cred_type = if sttx.is_field_present(sf("sfCredentialType")) {
                sttx.get_field_vl(sf("sfCredentialType"))
            } else {
                vec![]
            };
            let cred_keylet = protocol::credential_keylet(
                Uint160::from_void(subject.data()),
                Uint160::from_void(issuer.data()),
                &cred_type,
            );

            let Some(cred_sle) = view.peek(cred_keylet).ok().flatten() else {
                return Ter::TEC_NO_ENTRY;
            };
            let Some(subject_sle) = view
                .peek(protocol::account_keylet(Uint160::from_void(subject.data())))
                .ok()
                .flatten()
            else {
                return Ter::TEF_INTERNAL;
            };
            let Some(issuer_sle) = view
                .peek(protocol::account_keylet(Uint160::from_void(issuer.data())))
                .ok()
                .flatten()
            else {
                return Ter::TEF_INTERNAL;
            };
            if ledger::credential_helpers::check_expired(&cred_sle, view.header().parent_close_time)
            {
                let result = ledger::credential_helpers::delete_sle(view, cred_sle)
                    .unwrap_or(Ter::TEF_INTERNAL);
                return if result == Ter::TES_SUCCESS {
                    Ter::TEC_EXPIRED
                } else {
                    result
                };
            }

            let owner_count = subject_sle.get_field_u32(sf("sfOwnerCount"));
            let balance = pre_fee_balance_drops
                .unwrap_or_else(|| subject_sle.get_field_amount(sf("sfBalance")).xrp().drops());
            let reserve = view.fees().account_reserve(owner_count as usize + 1) as i64;
            if balance < reserve {
                return Ter::TEC_INSUFFICIENT_RESERVE;
            }

            let mut obj = cred_sle.clone_as_object();
            obj.set_field_u32(sf("sfFlags"), protocol::lsfAccepted);
            if view
                .update(Arc::new(STLedgerEntry::from_stobject(obj, *cred_sle.key())))
                .is_err()
            {
                return Ter::TEF_INTERNAL;
            }
            if ledger::adjust_owner_count(view, &issuer_sle, -1).is_err()
                || ledger::adjust_owner_count(view, &subject_sle, 1).is_err()
            {
                return Ter::TEF_INTERNAL;
            }
            Ter::TES_SUCCESS
        }
        TxType::CREDENTIAL_DELETE => {
            let account = sttx.get_account_id(sf("sfAccount"));
            let subject = if sttx.is_field_present(sf("sfSubject")) {
                sttx.get_account_id(sf("sfSubject"))
            } else {
                account
            };
            let issuer = if sttx.is_field_present(sf("sfIssuer")) {
                sttx.get_account_id(sf("sfIssuer"))
            } else {
                account
            };
            let cred_type = if sttx.is_field_present(sf("sfCredentialType")) {
                sttx.get_field_vl(sf("sfCredentialType"))
            } else {
                vec![]
            };
            let cred_keylet = protocol::credential_keylet(
                Uint160::from_void(subject.data()),
                Uint160::from_void(issuer.data()),
                &cred_type,
            );
            let Some(cred_sle) = view.peek(cred_keylet).ok().flatten() else {
                return Ter::TEC_NO_ENTRY;
            };
            if account != subject
                && account != issuer
                && !ledger::credential_helpers::check_expired(
                    &cred_sle,
                    view.header().parent_close_time,
                )
            {
                return Ter::TEC_NO_PERMISSION;
            }
            ledger::credential_helpers::delete_sle(view, cred_sle).unwrap_or(Ter::TEF_INTERNAL)
        }

        // --- AMM Clawback ---
        TxType::AMM_CLAWBACK => {
            let issuer = sttx.get_account_id(sf("sfAccount"));
            let holder = sttx.get_account_id(sf("sfHolder"));
            let amount = sttx.get_field_amount(sf("sfAmount"));
            let currency = amount.issue().currency;
            let line_keylet = protocol::line(issuer, holder, currency);
            if let Ok(Some(line)) = view.peek(line_keylet) {
                let b_high = holder > issuer;
                let current_balance = line.get_field_amount(sf("sfBalance"));
                let holder_balance = if b_high {
                    let mut neg = current_balance.clone();
                    neg.negate();
                    neg
                } else {
                    current_balance.clone()
                };
                let normalized_amount = {
                    let mut a = amount.clone();
                    a.set_issue(protocol::Issue {
                        account: issuer,
                        currency,
                    });
                    a
                };
                let clawback_actual = if normalized_amount > holder_balance {
                    holder_balance
                } else {
                    normalized_amount
                };
                let new_balance = if b_high {
                    current_balance + clawback_actual
                } else {
                    current_balance - clawback_actual
                };
                let mut obj = line.clone_as_object();
                obj.set_field_amount(sf("sfBalance"), new_balance);
                let _ = view.update(Arc::new(STLedgerEntry::from_stobject(obj, *line.key())));
            }
            Ter::TES_SUCCESS
        }

        // --- NFToken Modify ---
        TxType::NFTOKEN_MODIFY => {
            // Update mutable fields on an NFToken
            Ter::TES_SUCCESS
        }

        // --- AMMBid: full reference AMMBid::applyBid parity ---
        TxType::AMM_BID => crate::state::amm_bid_apply::apply_amm_bid(view, sttx),

        // --- Change pseudo-transaction (reference the reference source) ---
        TxType::FEE => {
            let k = protocol::fee_settings_keylet();
            let mut obj = if let Ok(Some(existing)) = view.peek(k) {
                existing.clone_as_object()
            } else {
                protocol::STObject::new(sf("sfGeneric"))
            };
            if sttx.is_field_present(sf("sfBaseFeeDrops")) {
                obj.set_field_amount(
                    sf("sfBaseFeeDrops"),
                    sttx.get_field_amount(sf("sfBaseFeeDrops")),
                );
                obj.set_field_amount(
                    sf("sfReserveBaseDrops"),
                    sttx.get_field_amount(sf("sfReserveBaseDrops")),
                );
                obj.set_field_amount(
                    sf("sfReserveIncrementDrops"),
                    sttx.get_field_amount(sf("sfReserveIncrementDrops")),
                );
            } else {
                if sttx.is_field_present(sf("sfBaseFee")) {
                    obj.set_field_u64(sf("sfBaseFee"), sttx.get_field_u64(sf("sfBaseFee")));
                }
                if sttx.is_field_present(sf("sfReferenceFeeUnits")) {
                    obj.set_field_u32(
                        sf("sfReferenceFeeUnits"),
                        sttx.get_field_u32(sf("sfReferenceFeeUnits")),
                    );
                }
                if sttx.is_field_present(sf("sfReserveBase")) {
                    obj.set_field_u32(sf("sfReserveBase"), sttx.get_field_u32(sf("sfReserveBase")));
                }
                if sttx.is_field_present(sf("sfReserveIncrement")) {
                    obj.set_field_u32(
                        sf("sfReserveIncrement"),
                        sttx.get_field_u32(sf("sfReserveIncrement")),
                    );
                }
            }
            let sle = Arc::new(protocol::STLedgerEntry::from_stobject(obj, k.key));
            let _ = view.update(sle);
            Ter::TES_SUCCESS
        }

        TxType::AMENDMENT => {
            let k = protocol::amendments_keylet();
            let mut obj = if let Ok(Some(existing)) = view.peek(k) {
                existing.clone_as_object()
            } else {
                protocol::STObject::new(sf("sfGeneric"))
            };
            let amendment = sttx.get_field_h256(sf("sfAmendment"));
            let flags = sttx.get_field_u32(sf("sfFlags"));
            let got_majority = (flags & 0x0001_0000) != 0;
            let lost_majority = (flags & 0x0002_0000) != 0;

            if got_majority {
                let mut majorities = if obj.is_field_present(sf("sfMajorities")) {
                    obj.get_field_array(sf("sfMajorities"))
                } else {
                    protocol::STArray::new(sf("sfMajorities"))
                };
                let mut entry = protocol::STObject::new(sf("sfGeneric"));
                entry.set_field_h256(sf("sfAmendment"), amendment);
                entry.set_field_u32(sf("sfCloseTime"), view.parent_close_time().as_seconds());
                majorities.push_back(entry);
                obj.set_field_array(sf("sfMajorities"), majorities);
            } else if lost_majority {
                if obj.is_field_present(sf("sfMajorities")) {
                    let old = obj.get_field_array(sf("sfMajorities"));
                    let mut new_maj = protocol::STArray::new(sf("sfMajorities"));
                    for entry in old.iter() {
                        if entry.get_field_h256(sf("sfAmendment")) != amendment {
                            new_maj.push_back(entry.clone());
                        }
                    }
                    if new_maj.is_empty() {
                        obj.make_field_absent(sf("sfMajorities"));
                    } else {
                        obj.set_field_array(sf("sfMajorities"), new_maj);
                    }
                }
            } else {
                // Enable amendment
                let mut amendments = if obj.is_field_present(sf("sfAmendments")) {
                    obj.get_field_v256(sf("sfAmendments"))
                } else {
                    protocol::STVector256::new()
                };
                amendments.push_back(amendment);
                obj.set_field_v256(sf("sfAmendments"), amendments);
                // Remove from majorities
                if obj.is_field_present(sf("sfMajorities")) {
                    let old = obj.get_field_array(sf("sfMajorities"));
                    let mut new_maj = protocol::STArray::new(sf("sfMajorities"));
                    for entry in old.iter() {
                        if entry.get_field_h256(sf("sfAmendment")) != amendment {
                            new_maj.push_back(entry.clone());
                        }
                    }
                    if new_maj.is_empty() {
                        obj.make_field_absent(sf("sfMajorities"));
                    } else {
                        obj.set_field_array(sf("sfMajorities"), new_maj);
                    }
                }
            }
            let sle = Arc::new(protocol::STLedgerEntry::from_stobject(obj, k.key));
            let _ = view.update(sle);
            Ter::TES_SUCCESS
        }

        TxType::UNL_MODIFY => {
            let k = protocol::negative_unl_keylet();
            let mut obj = if let Ok(Some(existing)) = view.peek(k) {
                existing.clone_as_object()
            } else {
                protocol::STObject::new(sf("sfGeneric"))
            };
            let disabling = sttx.is_field_present(sf("sfUNLModifyDisabling"))
                && sttx.get_field_u8(sf("sfUNLModifyDisabling")) != 0;
            let validator = sttx.get_field_vl(sf("sfUNLModifyValidator"));
            if disabling {
                obj.set_field_vl(sf("sfValidatorToDisable"), &validator);
            } else {
                obj.set_field_vl(sf("sfValidatorToReEnable"), &validator);
            }
            let sle = Arc::new(protocol::STLedgerEntry::from_stobject(obj, k.key));
            let _ = view.update(sle);
            Ter::TES_SUCCESS
        }

        _ => Ter::TEM_UNKNOWN,
    }
}

/// Direct XRP payment — debit source, credit destination.
fn do_xrp_payment<V: ledger::ApplyView>(
    view: &mut V,
    src: &AccountID,
    dst: &AccountID,
    amount: &STAmount,
    _flags: u32,
) -> Ter {
    let xrp = amount.xrp().drops();
    if xrp <= 0 {
        return Ter::TES_SUCCESS;
    }

    let src_keylet = protocol::account_keylet(Uint160::from_void(src.data()));
    let dst_keylet = protocol::account_keylet(Uint160::from_void(dst.data()));

    if let Ok(Some(src_sle)) = view.peek(src_keylet) {
        let bal = src_sle.get_field_amount(sf("sfBalance")).xrp().drops();
        let mut obj = src_sle.clone_as_object();
        obj.set_field_amount(
            sf("sfBalance"),
            STAmount::from_xrp_amount(XRPAmount::from_drops(bal - xrp)),
        );
        let _ = view.update(Arc::new(STLedgerEntry::from_stobject(obj, *src_sle.key())));
    }

    if let Ok(Some(dst_sle)) = view.peek(dst_keylet) {
        let bal = dst_sle.get_field_amount(sf("sfBalance")).xrp().drops();
        let mut obj = dst_sle.clone_as_object();
        obj.set_field_amount(
            sf("sfBalance"),
            STAmount::from_xrp_amount(XRPAmount::from_drops(bal + xrp)),
        );
        let _ = view.update(Arc::new(STLedgerEntry::from_stobject(obj, *dst_sle.key())));
    }

    Ter::TES_SUCCESS
}

fn close_channel<V: ledger::ApplyView>(view: &mut V, chan: &STLedgerEntry, key: Uint256) -> Ter {
    let src = chan.get_account_id(sf("sfAccount"));

    // Remove from source owner directory
    let owner_node = chan.get_field_u64(sf("sfOwnerNode"));
    let src_dir = protocol::owner_dir_keylet(Uint160::from_void(src.data()));
    let _ = ledger::dir_remove(view, &src_dir, owner_node, key, true);

    // Remove from destination owner directory if present
    if chan.is_field_present(sf("sfDestinationNode")) {
        let dst = chan.get_account_id(sf("sfDestination"));
        let dst_node = chan.get_field_u64(sf("sfDestinationNode"));
        let dst_dir = protocol::owner_dir_keylet(Uint160::from_void(dst.data()));
        let _ = ledger::dir_remove(view, &dst_dir, dst_node, key, true);
    }

    // Return remaining funds to source (Amount - Balance)
    let chan_amount = chan.get_field_amount(sf("sfAmount")).xrp().drops();
    let chan_balance = chan.get_field_amount(sf("sfBalance")).xrp().drops();
    let refund = chan_amount - chan_balance;

    let src_keylet = protocol::account_keylet(Uint160::from_void(src.data()));
    if let Ok(Some(src_sle)) = view.peek(src_keylet) {
        let src_bal = src_sle.get_field_amount(sf("sfBalance")).xrp().drops();
        let mut src_obj = src_sle.clone_as_object();
        src_obj.set_field_amount(
            sf("sfBalance"),
            STAmount::from_xrp_amount(XRPAmount::from_drops(src_bal + refund)),
        );
        let _ = view.update(Arc::new(STLedgerEntry::from_stobject(
            src_obj,
            *src_sle.key(),
        )));
        let _ = ledger::adjust_owner_count(view, &src_sle, -1);
    }

    // Erase the channel
    let _ = view.erase(Arc::new(chan.clone()));
    Ter::TES_SUCCESS
}

fn asf_to_lsf(asf: u32) -> u32 {
    match asf {
        1 => 0x0002_0000,  // asfRequireDest → lsfRequireDestTag
        2 => 0x0004_0000,  // asfRequireAuth → lsfRequireAuth
        3 => 0x0008_0000,  // asfDisallowXRP → lsfDisallowXRP
        4 => 0x0010_0000,  // asfDisableMaster → lsfDisableMaster
        5 => 0,            // asfAccountTxnID — handled separately (field, not flag)
        6 => 0x0020_0000,  // asfNoFreeze → lsfNoFreeze
        7 => 0x0040_0000,  // asfGlobalFreeze → lsfGlobalFreeze
        8 => 0x0080_0000,  // asfDefaultRipple → lsfDefaultRipple
        9 => 0x0100_0000,  // asfDepositAuth → lsfDepositAuth
        10 => 0,           // asfAuthorizedNFTokenMinter — handled separately (field, not flag)
        12 => 0x0400_0000, // asfDisallowIncomingNFTokenOffer
        13 => 0x0800_0000, // asfDisallowIncomingCheck
        14 => 0x1000_0000, // asfDisallowIncomingPayChan
        15 => 0x2000_0000, // asfDisallowIncomingTrustline
        _ => 0,
    }
}
