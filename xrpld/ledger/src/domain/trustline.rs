//! Read-only trustline helpers ported from the reference implementation.

use basics::base_uint::Uint160;
use protocol::{
    AccountID, Currency, Issue, STAmount, account_keylet, get_field_by_symbol, is_xrp_currency,
    line, lsfGlobalFreeze, lsfHighDeepFreeze, lsfHighFreeze, lsfLowDeepFreeze, lsfLowFreeze,
    sf_generic,
};
use shamap::traversal::TraversalError;

use crate::Ledger;

fn to_account_id(account: Uint160) -> AccountID {
    AccountID::from_slice(account.data()).expect("account width should match")
}

fn blank_iou_amount(currency: Currency, account: AccountID) -> STAmount {
    STAmount::new_with_asset(sf_generic(), Issue::new(currency, account), 0, 0, false)
}

pub fn credit_limit(
    ledger: &Ledger,
    account: Uint160,
    issuer: Uint160,
    currency: Currency,
) -> Result<STAmount, TraversalError> {
    let account_id = to_account_id(account);
    let mut result = blank_iou_amount(currency, account_id);

    if let Some(line) = ledger.read(line(
        to_account_id(account),
        to_account_id(issuer),
        currency,
    ))? {
        result = line.get_field_amount(if account < issuer {
            get_field_by_symbol("sfLowLimit")
        } else {
            get_field_by_symbol("sfHighLimit")
        });
        result.set_issuer(account_id);
    }

    Ok(result)
}

pub fn credit_balance(
    ledger: &Ledger,
    account: Uint160,
    issuer: Uint160,
    currency: Currency,
) -> Result<STAmount, TraversalError> {
    let account_id = to_account_id(account);
    let mut result = blank_iou_amount(currency, account_id);

    if let Some(line) = ledger.read(line(
        to_account_id(account),
        to_account_id(issuer),
        currency,
    ))? {
        result = line.get_field_amount(get_field_by_symbol("sfBalance"));
        if account < issuer {
            result.negate();
        }
        result.set_issuer(account_id);
    }

    Ok(result)
}

pub fn is_individual_frozen(
    ledger: &Ledger,
    account: Uint160,
    currency: Currency,
    issuer: Uint160,
) -> Result<bool, TraversalError> {
    if is_xrp_currency(currency) || issuer == account {
        return Ok(false);
    }

    let Some(line) = ledger.read(line(
        to_account_id(account),
        to_account_id(issuer),
        currency,
    ))?
    else {
        return Ok(false);
    };

    Ok(line.is_flag(if issuer > account {
        lsfHighFreeze
    } else {
        lsfLowFreeze
    }))
}

pub fn is_frozen(
    ledger: &Ledger,
    account: Uint160,
    currency: Currency,
    issuer: Uint160,
) -> Result<bool, TraversalError> {
    if is_xrp_currency(currency) {
        return Ok(false);
    }

    if let Some(issuer_root) = ledger.read(account_keylet(issuer))?
        && issuer_root.is_flag(lsfGlobalFreeze)
    {
        return Ok(true);
    }

    if issuer == account {
        return Ok(false);
    }

    let Some(line) = ledger.read(line(
        to_account_id(account),
        to_account_id(issuer),
        currency,
    ))?
    else {
        return Ok(false);
    };

    Ok(line.is_flag(if issuer > account {
        lsfHighFreeze
    } else {
        lsfLowFreeze
    }))
}

pub fn is_deep_frozen(
    ledger: &Ledger,
    account: Uint160,
    currency: Currency,
    issuer: Uint160,
) -> Result<bool, TraversalError> {
    if is_xrp_currency(currency) || issuer == account {
        return Ok(false);
    }

    let Some(line) = ledger.read(line(
        to_account_id(account),
        to_account_id(issuer),
        currency,
    ))?
    else {
        return Ok(false);
    };

    Ok(line.is_flag(lsfHighDeepFreeze) || line.is_flag(lsfLowDeepFreeze))
}
