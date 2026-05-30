use basics::{base_uint::Uint160, number::NumberParts as RuntimeNumber};
use ledger::{ApplyView, FlowSandbox, ReadView};
use protocol::{
    AccountID, Asset, IOUAmount, MPTAmount, STAmount, STLedgerEntry, XRPAmount, get_field_by_symbol,
};

pub(super) fn sf(name: &str) -> &'static protocol::SField {
    get_field_by_symbol(name)
}

pub(super) fn raw_account_id(account: AccountID) -> Uint160 {
    Uint160::from_slice(account.data()).expect("account width")
}

pub(super) fn optional_u64(sle: &STLedgerEntry, field: &'static protocol::SField) -> u64 {
    if sle.is_field_present(field) {
        sle.get_field_u64(field)
    } else {
        0
    }
}

pub(super) fn amount_to_number(amount: &STAmount) -> RuntimeNumber {
    if amount.native() {
        RuntimeNumber::from(amount.xrp())
    } else if amount.holds_mpt_issue() {
        RuntimeNumber::from(amount.mpt())
    } else {
        RuntimeNumber::from(amount.iou())
    }
}

pub(super) fn account_holds_asset_amount<V: ApplyView>(
    sandbox: &FlowSandbox<V>,
    account: AccountID,
    asset: Asset,
    field: &'static protocol::SField,
) -> Option<STAmount> {
    match asset {
        Asset::Issue(issue) if issue.native() => Some(
            sandbox
                .read(protocol::account_keylet(raw_account_id(account)))
                .ok()
                .flatten()
                .map(|sle| sle.get_field_amount(sf("sfBalance")))
                .unwrap_or_else(|| STAmount::from_xrp_amount(XRPAmount::new())),
        ),
        Asset::Issue(issue) => {
            if issue.issuer() == account {
                return Some(STAmount::from_iou_amount(field, IOUAmount::new(), issue));
            }
            let mut amount = sandbox
                .read(protocol::line(account, issue.issuer(), issue.currency))
                .ok()
                .flatten()
                .map(|sle| sle.get_field_amount(sf("sfBalance")))
                .unwrap_or_else(|| STAmount::from_iou_amount(field, IOUAmount::new(), issue));
            if account > issue.issuer() {
                amount.negate();
            }
            amount.set_issuer(issue.issuer());
            Some(amount)
        }
        Asset::MPTIssue(issue) => {
            let value = sandbox
                .read(protocol::mptoken_keylet_from_mptid(
                    issue.mpt_id(),
                    raw_account_id(account),
                ))
                .ok()
                .flatten()
                .map(|sle| optional_u64(&sle, sf("sfMPTAmount")))
                .unwrap_or(0);
            let value = i64::try_from(value).ok()?;
            Some(STAmount::from_mpt_amount(
                field,
                MPTAmount::from_value(value),
                issue,
            ))
        }
    }
}

pub(super) fn account_holds_asset_number<V: ApplyView>(
    sandbox: &FlowSandbox<V>,
    account: AccountID,
    asset: Asset,
) -> Option<RuntimeNumber> {
    match asset {
        Asset::Issue(issue) if issue.native() => {
            let amount = sandbox
                .read(protocol::account_keylet(raw_account_id(account)))
                .ok()
                .flatten()
                .map(|sle| sle.get_field_amount(sf("sfBalance")))
                .unwrap_or_else(|| STAmount::from_xrp_amount(XRPAmount::new()));
            Some(amount_to_number(&amount))
        }
        Asset::Issue(issue) => {
            if issue.issuer() == account {
                return Some(RuntimeNumber::zero());
            }
            let amount = sandbox
                .read(protocol::line(account, issue.issuer(), issue.currency))
                .ok()
                .flatten()
                .map(|sle| {
                    let mut balance = sle.get_field_amount(sf("sfBalance"));
                    if account > issue.issuer() {
                        balance.negate();
                    }
                    balance
                })
                .unwrap_or_else(|| STAmount::new_with_asset(sf("sfBalance"), issue, 0, 0, false));
            Some(amount_to_number(&amount))
        }
        Asset::MPTIssue(issue) => {
            let amount = sandbox
                .read(protocol::mptoken_keylet_from_mptid(
                    issue.mpt_id(),
                    raw_account_id(account),
                ))
                .ok()
                .flatten()
                .map(|sle| RuntimeNumber::from_i64(sle.get_field_u64(sf("sfMPTAmount")) as i64))
                .unwrap_or_else(RuntimeNumber::zero);
            Some(amount)
        }
    }
}
