//! Immutable `ReadView` preclaim helpers for every NFToken transaction type.
//!
//! The helper mirrors the corresponding rippled `preclaim(...)` methods using
//! only immutable reads. It does not create a mutable view or use an
//! unowned-type success default.

use std::sync::Arc;

use basics::base_uint::{Uint160, Uint256};
use ledger::ReadView;
use protocol::{
    AccountID, Asset, STAmount, STLedgerEntry, STTx, Ter, TxType, get_field_by_symbol,
    lsfGlobalFreeze, lsfHighFreeze, lsfLowFreeze, lsfSellNFToken,
};

use crate::TF_SELL_NFTOKEN;

fn sf(name: &str) -> &'static protocol::SField {
    get_field_by_symbol(name)
}

fn read_error() -> Ter {
    Ter::TEF_BAD_LEDGER
}

fn account_keylet(account: AccountID) -> protocol::Keylet {
    protocol::account_keylet(Uint160::from_void(account.data()))
}

fn read<V: ReadView>(
    view: &V,
    keylet: protocol::Keylet,
) -> Result<Option<Arc<STLedgerEntry>>, Ter> {
    view.read(keylet).map_err(|_| read_error())
}

fn account<V: ReadView>(view: &V, id: AccountID) -> Result<Option<Arc<STLedgerEntry>>, Ter> {
    read(view, account_keylet(id))
}

fn expired<V: ReadView>(view: &V, entry: &STLedgerEntry) -> bool {
    entry.is_field_present(sf("sfExpiration"))
        && view.parent_close_time().as_seconds() >= entry.get_field_u32(sf("sfExpiration"))
}

fn iou_frozen<V: ReadView>(
    view: &V,
    account_id: AccountID,
    issue: protocol::Issue,
) -> Result<bool, Ter> {
    if issue.native() || issue.account == account_id {
        return Ok(false);
    }
    if account(view, issue.account)?.is_some_and(|issuer| issuer.is_flag(lsfGlobalFreeze)) {
        return Ok(true);
    }
    let line = read(
        view,
        protocol::line(account_id, issue.account, issue.currency),
    )?;
    Ok(line.is_some_and(|sle| {
        sle.is_flag(if account_id > issue.account {
            lsfLowFreeze
        } else {
            lsfHighFreeze
        })
    }))
}

fn funds_at_least<V: ReadView>(
    view: &V,
    account_id: AccountID,
    amount: &STAmount,
) -> Result<bool, Ter> {
    match amount.asset() {
        Asset::Issue(issue) if issue.native() => {
            let Some(root) = account(view, account_id)? else {
                return Ok(false);
            };
            let balance = root.get_field_amount(sf("sfBalance")).xrp().drops();
            let reserve = view
                .fees()
                .account_reserve(root.get_field_u32(sf("sfOwnerCount")) as usize)
                as i64;
            Ok(balance.saturating_sub(reserve) >= amount.xrp().drops())
        }
        Asset::Issue(issue) if issue.account == account_id => Ok(true),
        Asset::Issue(issue) => {
            if iou_frozen(view, account_id, issue)? {
                return Ok(false);
            }
            let Some(line) = read(
                view,
                protocol::line(account_id, issue.account, issue.currency),
            )?
            else {
                return Ok(false);
            };
            let mut balance = line.get_field_amount(sf("sfBalance"));
            if account_id > issue.account {
                balance.negate();
            }
            balance.set_issuer(issue.account);
            Ok(balance >= *amount)
        }
        Asset::MPTIssue(issue) if issue.issuer() == account_id => Ok(true),
        Asset::MPTIssue(issue) => Ok(read(
            view,
            protocol::mptoken_keylet_from_mptid(
                issue.mpt_id(),
                Uint160::from_void(account_id.data()),
            ),
        )?
        .is_some_and(|token| {
            amount.mpt().value() >= 0
                && (amount.mpt().value() as u64) <= token.get_field_u64(sf("sfMPTAmount"))
        })),
    }
}

fn authorize_and_check_deep_freeze<V: ReadView>(
    view: &V,
    account_id: AccountID,
    amount: &STAmount,
    deep_freeze: bool,
) -> Result<Ter, Ter> {
    let Asset::Issue(issue) = amount.asset() else {
        return Ok(Ter::TES_SUCCESS);
    };
    if !view
        .rules()
        .enabled(&protocol::fix_enforce_nftoken_trustline_v2())
    {
        return Ok(Ter::TES_SUCCESS);
    }
    let auth = ledger::nftoken_helpers::check_trustline_authorized(view, &account_id, &issue)
        .map_err(|_| read_error())?;
    if auth != Ter::TES_SUCCESS || !deep_freeze {
        return Ok(auth);
    }
    ledger::nftoken_helpers::check_trustline_deep_frozen(view, &account_id, &issue)
        .map_err(|_| read_error())
}

fn check_offer<V: ReadView>(
    view: &V,
    tx: &STTx,
    field: &'static protocol::SField,
) -> Result<Option<Arc<STLedgerEntry>>, Ter> {
    if !tx.is_field_present(field) {
        return Ok(None);
    }
    let id = tx.get_field_h256(field);
    if id.is_zero() {
        return Err(Ter::TEC_OBJECT_NOT_FOUND);
    }
    let Some(offer) = read(view, protocol::nft_offer_keylet(id))? else {
        return Err(Ter::TEC_OBJECT_NOT_FOUND);
    };
    if expired(view, &offer) && !view.rules().enabled(&protocol::fix_cleanup_3_1_3()) {
        return Err(Ter::TEC_EXPIRED);
    }
    if offer.get_field_amount(sf("sfAmount")).negative() {
        return Err(Ter::TEM_BAD_OFFER);
    }
    Ok(Some(offer))
}

fn token_offer_preclaim<V: ReadView>(
    view: &V,
    account_id: AccountID,
    token_id: Uint256,
    amount: &STAmount,
    destination: Option<AccountID>,
    owner: Option<AccountID>,
    tx_flags: u32,
) -> Result<Ter, Ter> {
    ledger::nftoken_helpers::token_offer_create_preclaim(
        view,
        &account_id,
        &protocol::nft::get_issuer(token_id),
        amount,
        destination.as_ref(),
        protocol::nft::get_flags(token_id),
        protocol::nft::get_transfer_fee(token_id),
        owner.as_ref(),
        tx_flags,
    )
    .map_err(|_| read_error())
}

fn preclaim_create_offer<V: ReadView>(view: &V, tx: &STTx) -> Result<Ter, Ter> {
    if tx.is_field_present(sf("sfExpiration"))
        && view.parent_close_time().as_seconds() >= tx.get_field_u32(sf("sfExpiration"))
    {
        return Ok(Ter::TEC_EXPIRED);
    }
    let account_id = tx.get_account_id(sf("sfAccount"));
    let owner = tx
        .is_field_present(sf("sfOwner"))
        .then(|| tx.get_account_id(sf("sfOwner")));
    let token_id = tx.get_field_h256(sf("sfNFTokenID"));
    let token_owner = if (tx.get_flags() & TF_SELL_NFTOKEN) != 0 {
        account_id
    } else {
        owner.unwrap_or_default()
    };
    if ledger::nftoken_helpers::find_token(view, &token_owner, &token_id)
        .map_err(|_| read_error())?
        .is_none()
    {
        return Ok(Ter::TEC_NO_ENTRY);
    }
    token_offer_preclaim(
        view,
        account_id,
        token_id,
        &tx.get_field_amount(sf("sfAmount")),
        tx.is_field_present(sf("sfDestination"))
            .then(|| tx.get_account_id(sf("sfDestination"))),
        owner,
        tx.get_flags(),
    )
}

fn preclaim_mint<V: ReadView>(view: &V, tx: &STTx) -> Result<Ter, Ter> {
    let account_id = tx.get_account_id(sf("sfAccount"));
    let issuer = tx
        .is_field_present(sf("sfIssuer"))
        .then(|| tx.get_account_id(sf("sfIssuer")));
    if let Some(issuer) = issuer {
        let Some(issuer_sle) = account(view, issuer)? else {
            return Ok(Ter::TEC_NO_ISSUER);
        };
        if !issuer_sle.is_field_present(sf("sfNFTokenMinter"))
            || issuer_sle.get_account_id(sf("sfNFTokenMinter")) != account_id
        {
            return Ok(Ter::TEC_NO_PERMISSION);
        }
    }
    if !tx.is_field_present(sf("sfAmount")) {
        return Ok(Ter::TES_SUCCESS);
    }
    if tx.is_field_present(sf("sfExpiration"))
        && view.parent_close_time().as_seconds() >= tx.get_field_u32(sf("sfExpiration"))
    {
        return Ok(Ter::TEC_EXPIRED);
    }
    // Mint may create only a sell offer. The NFT flags passed to rippled's
    // shared helper are exactly the low 16 transaction flag bits.
    ledger::nftoken_helpers::token_offer_create_preclaim(
        view,
        &account_id,
        &issuer.unwrap_or(account_id),
        &tx.get_field_amount(sf("sfAmount")),
        tx.is_field_present(sf("sfDestination"))
            .then(|| tx.get_account_id(sf("sfDestination")))
            .as_ref(),
        (tx.get_flags() & 0xffff) as u16,
        tx.is_field_present(sf("sfTransferFee"))
            .then(|| tx.get_field_u16(sf("sfTransferFee")))
            .unwrap_or(0),
        None,
        TF_SELL_NFTOKEN,
    )
    .map_err(|_| read_error())
}

fn preclaim_burn<V: ReadView>(view: &V, tx: &STTx) -> Result<Ter, Ter> {
    let account_id = tx.get_account_id(sf("sfAccount"));
    let owner = tx
        .is_field_present(sf("sfOwner"))
        .then(|| tx.get_account_id(sf("sfOwner")))
        .unwrap_or(account_id);
    let token_id = tx.get_field_h256(sf("sfNFTokenID"));
    if ledger::nftoken_helpers::find_token(view, &owner, &token_id)
        .map_err(|_| read_error())?
        .is_none()
    {
        return Ok(Ter::TEC_NO_ENTRY);
    }
    if owner == account_id {
        return Ok(Ter::TES_SUCCESS);
    }
    if protocol::nft::get_flags(token_id) & protocol::nft::FLAG_BURNABLE == 0 {
        return Ok(Ter::TEC_NO_PERMISSION);
    }
    let issuer = protocol::nft::get_issuer(token_id);
    if issuer != account_id
        && let Some(issuer_sle) = account(view, issuer)?
        && (!issuer_sle.is_field_present(sf("sfNFTokenMinter"))
            || issuer_sle.get_account_id(sf("sfNFTokenMinter")) != account_id)
    {
        return Ok(Ter::TEC_NO_PERMISSION);
    }
    Ok(Ter::TES_SUCCESS)
}

fn preclaim_modify<V: ReadView>(view: &V, tx: &STTx) -> Result<Ter, Ter> {
    let account_id = tx.get_account_id(sf("sfAccount"));
    let owner = tx
        .is_field_present(sf("sfOwner"))
        .then(|| tx.get_account_id(sf("sfOwner")))
        .unwrap_or(account_id);
    let token_id = tx.get_field_h256(sf("sfNFTokenID"));
    if ledger::nftoken_helpers::find_token(view, &owner, &token_id)
        .map_err(|_| read_error())?
        .is_none()
    {
        return Ok(Ter::TEC_NO_ENTRY);
    }
    if protocol::nft::get_flags(token_id) & protocol::nft::FLAG_MUTABLE == 0 {
        return Ok(Ter::TEC_NO_PERMISSION);
    }
    let issuer = protocol::nft::get_issuer(token_id);
    if issuer == account_id {
        return Ok(Ter::TES_SUCCESS);
    }
    let Some(issuer_sle) = account(view, issuer)? else {
        return Ok(Ter::TEC_INTERNAL);
    };
    Ok(
        if issuer_sle.is_field_present(sf("sfNFTokenMinter"))
            && issuer_sle.get_account_id(sf("sfNFTokenMinter")) == account_id
        {
            Ter::TES_SUCCESS
        } else {
            Ter::TEC_NO_PERMISSION
        },
    )
}

fn preclaim_cancel_offer<V: ReadView>(view: &V, tx: &STTx) -> Result<Ter, Ter> {
    let account_id = tx.get_account_id(sf("sfAccount"));
    for id in tx.get_field_v256(sf("sfNFTokenOffers")).value() {
        let Some(offer) = read(view, protocol::unchecked_keylet(*id))? else {
            continue;
        };
        if offer.get_type() != protocol::LedgerEntryType::NFTokenOffer {
            return Ok(Ter::TEC_NO_PERMISSION);
        }
        if expired(view, &offer)
            || offer.get_account_id(sf("sfOwner")) == account_id
            || (offer.is_field_present(sf("sfDestination"))
                && offer.get_account_id(sf("sfDestination")) == account_id)
        {
            continue;
        }
        return Ok(Ter::TEC_NO_PERMISSION);
    }
    Ok(Ter::TES_SUCCESS)
}

fn preclaim_accept_offer<V: ReadView>(view: &V, tx: &STTx) -> Result<Ter, Ter> {
    let account_id = tx.get_account_id(sf("sfAccount"));
    // rippled checks the buy offer first, then the sell offer.
    let buy = check_offer(view, tx, sf("sfNFTokenBuyOffer"))?;
    let sell = check_offer(view, tx, sf("sfNFTokenSellOffer"))?;

    if let (Some(buy), Some(sell)) = (&buy, &sell) {
        let buy_amount = buy.get_field_amount(sf("sfAmount"));
        let sell_amount = sell.get_field_amount(sf("sfAmount"));
        if buy.get_field_h256(sf("sfNFTokenID")) != sell.get_field_h256(sf("sfNFTokenID"))
            || buy_amount.asset() != sell_amount.asset()
        {
            return Ok(Ter::TEC_NFTOKEN_BUY_SELL_MISMATCH);
        }
        if buy.get_account_id(sf("sfOwner")) == sell.get_account_id(sf("sfOwner")) {
            return Ok(Ter::TEC_CANT_ACCEPT_OWN_NFTOKEN_OFFER);
        }
        if sell_amount > buy_amount {
            return Ok(Ter::TEC_INSUFFICIENT_PAYMENT);
        }
        for offer in [buy, sell] {
            if offer.is_field_present(sf("sfDestination"))
                && offer.get_account_id(sf("sfDestination")) != account_id
            {
                return Ok(Ter::TEC_NO_PERMISSION);
            }
        }
        if tx.is_field_present(sf("sfNFTokenBrokerFee")) {
            let broker_fee = tx.get_field_amount(sf("sfNFTokenBrokerFee"));
            if broker_fee.asset() != buy_amount.asset() {
                return Ok(Ter::TEC_NFTOKEN_BUY_SELL_MISMATCH);
            }
            if broker_fee >= buy_amount || sell_amount > buy_amount.clone() - broker_fee.clone() {
                return Ok(Ter::TEC_INSUFFICIENT_PAYMENT);
            }
            let result = authorize_and_check_deep_freeze(view, account_id, &broker_fee, true)?;
            if result != Ter::TES_SUCCESS {
                return Ok(result);
            }
        }
    }

    if let Some(buy) = &buy {
        if buy.is_flag(lsfSellNFToken) {
            return Ok(Ter::TEC_NFTOKEN_OFFER_TYPE_MISMATCH);
        }
        if buy.get_account_id(sf("sfOwner")) == account_id {
            return Ok(Ter::TEC_CANT_ACCEPT_OWN_NFTOKEN_OFFER);
        }
        let token_id = buy.get_field_h256(sf("sfNFTokenID"));
        if sell.is_none()
            && ledger::nftoken_helpers::find_token(view, &account_id, &token_id)
                .map_err(|_| read_error())?
                .is_none()
        {
            return Ok(Ter::TEC_NO_PERMISSION);
        }
        if sell.is_none()
            && buy.is_field_present(sf("sfDestination"))
            && buy.get_account_id(sf("sfDestination")) != account_id
        {
            return Ok(Ter::TEC_NO_PERMISSION);
        }
        let amount = buy.get_field_amount(sf("sfAmount"));
        if !funds_at_least(view, buy.get_account_id(sf("sfOwner")), &amount)? {
            return Ok(Ter::TEC_INSUFFICIENT_FUNDS);
        }
        let result = authorize_and_check_deep_freeze(
            view,
            buy.get_account_id(sf("sfOwner")),
            &amount,
            false,
        )?;
        if result != Ter::TES_SUCCESS {
            return Ok(result);
        }
        if sell.is_none() {
            let result = authorize_and_check_deep_freeze(view, account_id, &amount, true)?;
            if result != Ter::TES_SUCCESS {
                return Ok(result);
            }
        }
    }

    if let Some(sell) = &sell {
        if !sell.is_flag(lsfSellNFToken) {
            return Ok(Ter::TEC_NFTOKEN_OFFER_TYPE_MISMATCH);
        }
        if sell.get_account_id(sf("sfOwner")) == account_id {
            return Ok(Ter::TEC_CANT_ACCEPT_OWN_NFTOKEN_OFFER);
        }
        let token_id = sell.get_field_h256(sf("sfNFTokenID"));
        if ledger::nftoken_helpers::find_token(view, &sell.get_account_id(sf("sfOwner")), &token_id)
            .map_err(|_| read_error())?
            .is_none()
        {
            return Ok(Ter::TEC_NO_PERMISSION);
        }
        if buy.is_none()
            && sell.is_field_present(sf("sfDestination"))
            && sell.get_account_id(sf("sfDestination")) != account_id
        {
            return Ok(Ter::TEC_NO_PERMISSION);
        }
        let amount = sell.get_field_amount(sf("sfAmount"));
        if buy.is_none() && !funds_at_least(view, account_id, &amount)? {
            return Ok(Ter::TEC_INSUFFICIENT_FUNDS);
        }
        let result = authorize_and_check_deep_freeze(
            view,
            sell.get_account_id(sf("sfOwner")),
            &amount,
            false,
        )?;
        if result != Ter::TES_SUCCESS {
            return Ok(result);
        }
        if buy.is_none() {
            let result = authorize_and_check_deep_freeze(view, account_id, &amount, false)?;
            if result != Ter::TES_SUCCESS {
                return Ok(result);
            }
        }
    }

    let Some(offer) = buy.as_ref().or(sell.as_ref()) else {
        // Preflight rejects this; keep rippled's defensive internal result
        // explicit rather than inventing a successful default.
        return Ok(Ter::TEC_INTERNAL);
    };
    let token_id = offer.get_field_h256(sf("sfNFTokenID"));
    let amount = offer.get_field_amount(sf("sfAmount"));
    if protocol::nft::get_transfer_fee(token_id) != 0 && !amount.native() {
        let Asset::Issue(issue) = amount.asset() else {
            return Ok(Ter::TEC_INTERNAL);
        };
        let issuer = protocol::nft::get_issuer(token_id);
        if view
            .rules()
            .enabled(&protocol::feature_id("fixEnforceNFTokenTrustline"))
            && protocol::nft::get_flags(token_id) & protocol::nft::FLAG_CREATE_TRUST_LINES == 0
            && issuer != issue.account
            && read(view, protocol::line(issuer, issue.account, issue.currency))?.is_none()
        {
            return Ok(Ter::TEC_NO_LINE);
        }
        let result = authorize_and_check_deep_freeze(view, issuer, &amount, true)?;
        if result != Ter::TES_SUCCESS {
            return Ok(result);
        }
    }
    Ok(Ter::TES_SUCCESS)
}

/// Runs the complete immutable preclaim for the owned NFToken family.
/// `None` means the transaction type is outside this family and is never a
/// permissive result.
pub fn run_nftoken_read_view_preclaim<V: ReadView>(
    view: &V,
    tx: &STTx,
    txn_type: TxType,
) -> Option<Ter> {
    let result = match txn_type {
        TxType::NFTOKEN_MINT => preclaim_mint(view, tx),
        TxType::NFTOKEN_BURN => preclaim_burn(view, tx),
        TxType::NFTOKEN_CREATE_OFFER => preclaim_create_offer(view, tx),
        TxType::NFTOKEN_CANCEL_OFFER => preclaim_cancel_offer(view, tx),
        TxType::NFTOKEN_ACCEPT_OFFER => preclaim_accept_offer(view, tx),
        TxType::NFTOKEN_MODIFY => preclaim_modify(view, tx),
        _ => return None,
    };
    Some(result.unwrap_or_else(|ter| ter))
}

#[cfg(test)]
mod tests {
    use std::{collections::BTreeMap, sync::Arc};

    use basics::base_uint::Uint256;
    use ledger::{Fees, LedgerHeader, ReadView, ReadViewTx, Rules, ViewError};
    use protocol::{AccountID, Keylet, STLedgerEntry, STTx, Ter, TxType};

    use super::{run_nftoken_read_view_preclaim, sf};

    #[derive(Debug, Default)]
    struct View {
        entries: BTreeMap<Uint256, Arc<STLedgerEntry>>,
    }
    impl ReadView for View {
        fn open(&self) -> bool {
            false
        }
        fn header(&self) -> LedgerHeader {
            LedgerHeader::default()
        }
        fn fees(&self) -> Fees {
            Fees::default()
        }
        fn rules(&self) -> Rules {
            Rules::default()
        }
        fn exists(&self, keylet: Keylet) -> Result<bool, ViewError> {
            Ok(self.entries.contains_key(&keylet.key))
        }
        fn succ(&self, _: Uint256, _: Option<Uint256>) -> Result<Option<Uint256>, ViewError> {
            Ok(None)
        }
        fn read(&self, keylet: Keylet) -> Result<Option<Arc<STLedgerEntry>>, ViewError> {
            Ok(self.entries.get(&keylet.key).cloned())
        }
        fn sles(&self) -> Result<Vec<Arc<STLedgerEntry>>, ViewError> {
            Ok(self.entries.values().cloned().collect())
        }
        fn tx_exists(&self, _: Uint256) -> Result<bool, ViewError> {
            Ok(false)
        }
        fn tx_read(&self, _: Uint256) -> Result<Option<ReadViewTx>, ViewError> {
            Ok(None)
        }
        fn txs(&self) -> Result<Vec<ReadViewTx>, ViewError> {
            Ok(Vec::new())
        }
    }

    fn account(fill: u8) -> AccountID {
        AccountID::from_array([fill; 20])
    }

    #[test]
    fn nftoken_helper_has_no_unowned_success_default() {
        let view = View::default();
        let tx = STTx::new(TxType::PAYMENT, |_| {});
        assert_eq!(
            run_nftoken_read_view_preclaim(&view, &tx, TxType::PAYMENT),
            None
        );
    }

    #[test]
    fn burn_and_modify_fail_at_the_exact_missing_token_read() {
        let view = View::default();
        for txn_type in [TxType::NFTOKEN_BURN, TxType::NFTOKEN_MODIFY] {
            let tx = STTx::new(txn_type, |tx| {
                tx.set_account_id(sf("sfAccount"), account(1));
                tx.set_field_h256(sf("sfNFTokenID"), Uint256::from_u64(3));
            });
            assert_eq!(
                run_nftoken_read_view_preclaim(&view, &tx, txn_type),
                Some(Ter::TEC_NO_ENTRY)
            );
        }
        assert!(view.entries.is_empty(), "ReadView preclaim must not mutate");
    }

    #[test]
    fn accept_offer_checks_missing_buy_before_all_other_work() {
        let view = View::default();
        let tx = STTx::new(TxType::NFTOKEN_ACCEPT_OFFER, |tx| {
            tx.set_account_id(sf("sfAccount"), account(1));
            tx.set_field_h256(sf("sfNFTokenBuyOffer"), Uint256::from_u64(9));
        });
        assert_eq!(
            run_nftoken_read_view_preclaim(&view, &tx, TxType::NFTOKEN_ACCEPT_OFFER),
            Some(Ter::TEC_OBJECT_NOT_FOUND)
        );
        assert!(view.entries.is_empty());
    }

    #[test]
    fn mint_validates_optional_issuer_before_offer_fields() {
        let view = View::default();
        let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
            tx.set_account_id(sf("sfAccount"), account(1));
            tx.set_account_id(sf("sfIssuer"), account(2));
        });
        assert_eq!(
            run_nftoken_read_view_preclaim(&view, &tx, TxType::NFTOKEN_MINT),
            Some(Ter::TEC_NO_ISSUER)
        );
    }
}
