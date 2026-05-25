//! the reference implementation parity helpers for balance lookup plus the narrow
//! holding lifecycle used by vault and lending transactors.

use std::sync::Arc;

use crate::{ApplyView, Ledger, ReadView, is_deep_frozen, is_frozen};
use basics::base_uint::Uint160;
use protocol::{
    AccountID, Asset, IOUAmount, Issue, LedgerEntryType, MPTIssue, STAmount, STLedgerEntry,
    STObject, StBase, XRPAmount, account_keylet, get_field_by_symbol, line, lsfDefaultRipple,
    owner_dir_keylet, sf_generic,
};
use shamap::traversal::TraversalError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FreezeHandling {
    IgnoreFreeze,
    ZeroIfFrozen,
}

fn confine_owner_count(current: u32, adjustment: i32) -> u32 {
    (i64::from(current) + i64::from(adjustment)).clamp(0, i64::from(u32::MAX)) as u32
}

fn zero_iou(issue: Issue) -> STAmount {
    STAmount::from_iou_amount(sf_generic(), IOUAmount::new(), issue)
}

fn to_uint160(account: AccountID) -> Uint160 {
    Uint160::from_slice(account.data()).expect("account width should match")
}

fn sf(name: &str) -> &'static protocol::SField {
    get_field_by_symbol(name)
}

pub fn xrp_liquid(
    ledger: &Ledger,
    account: AccountID,
    owner_count_adj: i32,
) -> Result<protocol::XRPAmount, TraversalError> {
    let Some(account_root) = ledger.read(account_keylet(to_uint160(account)))? else {
        return Ok(protocol::XRPAmount::new());
    };

    let owner_count = confine_owner_count(
        account_root.get_field_u32(protocol::get_field_by_symbol("sfOwnerCount")),
        owner_count_adj,
    );
    let reserve = ledger.fees().account_reserve(owner_count as usize);
    let balance = account_root
        .get_field_amount(protocol::get_field_by_symbol("sfBalance"))
        .xrp();

    Ok(if balance.drops() < reserve as i64 {
        protocol::XRPAmount::new()
    } else {
        balance
            - protocol::XRPAmount::from_drops(
                i64::try_from(reserve).expect("reserve should fit within XRPAmount range"),
            )
    })
}

pub fn account_funds(
    ledger: &Ledger,
    account: AccountID,
    default_amount: &STAmount,
    freeze_handling: FreezeHandling,
) -> Result<STAmount, TraversalError> {
    if default_amount.native() {
        return Ok(STAmount::from_xrp_amount(xrp_liquid(ledger, account, 0)?));
    }

    let issue = default_amount.issue();
    if issue.issuer() == account {
        return Ok(default_amount.clone());
    }

    if freeze_handling == FreezeHandling::ZeroIfFrozen
        && (is_frozen(
            ledger,
            to_uint160(account),
            issue.currency,
            to_uint160(issue.issuer()),
        )? || is_deep_frozen(
            ledger,
            to_uint160(account),
            issue.currency,
            to_uint160(issue.issuer()),
        )?)
    {
        return Ok(zero_iou(issue));
    }

    let mut amount = zero_iou(issue);
    if let Some(trustline) = ledger.read(line(account, issue.issuer(), issue.currency))? {
        amount = trustline.get_field_amount(get_field_by_symbol("sfBalance"));
        if account > issue.issuer() {
            amount.negate();
        }
        amount.set_issuer(issue.issuer());
    }
    Ok(amount)
}

pub fn account_funds_text(
    ledger: &Ledger,
    account: AccountID,
    default_amount: &STAmount,
    freeze_handling: FreezeHandling,
) -> Result<String, TraversalError> {
    Ok(account_funds(ledger, account, default_amount, freeze_handling)?.text())
}

pub fn can_add_holding<V: ReadView>(view: &V, asset: &Asset) -> protocol::Ter {
    asset.visit(
        |issue| {
            if issue.native() {
                return protocol::Ter::TES_SUCCESS;
            }
            let Ok(Some(issuer)) = view.read(account_keylet(to_uint160(issue.issuer()))) else {
                return protocol::Ter::TER_NO_ACCOUNT;
            };
            if issuer.get_field_u32(sf("sfFlags")) & lsfDefaultRipple == 0 {
                return protocol::Ter::TER_NO_RIPPLE;
            }
            protocol::Ter::TES_SUCCESS
        },
        |mpt_issue| {
            let Ok(Some(issuance)) =
                view.read(protocol::mpt_issuance_keylet_from_mptid(mpt_issue.mpt_id()))
            else {
                return protocol::Ter::TEC_OBJECT_NOT_FOUND;
            };
            if issuance.get_field_u32(sf("sfFlags")) & protocol::lsfMPTCanTransfer == 0 {
                return protocol::Ter::TEC_NO_AUTH;
            }
            protocol::Ter::TES_SUCCESS
        },
    )
}

pub fn add_empty_holding<V: ApplyView>(
    view: &mut V,
    account: &AccountID,
    prior_balance: XRPAmount,
    asset: &Asset,
) -> protocol::Ter {
    match asset {
        Asset::Issue(issue) => add_empty_iou_holding(view, account, prior_balance, issue),
        Asset::MPTIssue(mpt_issue) => {
            add_empty_mpt_holding(view, account, prior_balance, mpt_issue)
        }
    }
}

pub fn remove_empty_holding<V: ApplyView>(
    view: &mut V,
    account: &AccountID,
    asset: &Asset,
) -> protocol::Ter {
    match asset {
        Asset::Issue(issue) => remove_empty_iou_holding(view, account, issue),
        Asset::MPTIssue(mpt_issue) => remove_empty_mpt_holding(view, account, mpt_issue),
    }
}

fn erase_empty_owner_dir_root<V: ApplyView>(view: &mut V, account: &AccountID) {
    let keylet = owner_dir_keylet(to_uint160(*account));
    let Ok(Some(root)) = view.peek(keylet) else {
        return;
    };
    if root.get_field_v256(sf("sfIndexes")).value().is_empty() {
        let _ = view.erase(root);
    }
}

fn add_empty_iou_holding<V: ApplyView>(
    view: &mut V,
    account: &AccountID,
    prior_balance: XRPAmount,
    issue: &Issue,
) -> protocol::Ter {
    if issue.native() || *account == issue.issuer() {
        return protocol::Ter::TES_SUCCESS;
    }
    let src = issue.issuer();
    let dst = *account;
    let high = src > dst;
    let line_keylet = line(src, dst, issue.currency);
    let Ok(Some(src_sle)) = view.peek(account_keylet(to_uint160(src))) else {
        return protocol::Ter::TEF_BAD_LEDGER;
    };
    let Ok(Some(dst_sle)) = view.peek(account_keylet(to_uint160(dst))) else {
        return protocol::Ter::TEF_BAD_LEDGER;
    };
    if src_sle.get_field_u32(sf("sfFlags")) & protocol::lsfGlobalFreeze != 0 {
        return protocol::Ter::TEC_FROZEN;
    }
    if src_sle.get_field_u32(sf("sfFlags")) & lsfDefaultRipple == 0 {
        return protocol::Ter::TEC_INTERNAL;
    }
    if view.read(line_keylet).ok().flatten().is_some() {
        return protocol::Ter::TEC_DUPLICATE;
    }

    let owner_count = dst_sle.get_field_u32(sf("sfOwnerCount"));
    // lsfDisableMaster = 0x00100000, lsfDepositAuth = 0x01000000
    let pseudo_flags: u32 = 0x00100000 | 0x01000000;
    let is_pseudo = (dst_sle.get_field_u32(sf("sfFlags")) & pseudo_flags) == pseudo_flags;
    if !is_pseudo
        && prior_balance
            < XRPAmount::from_drops(
                i64::try_from(view.fees().account_reserve(owner_count as usize + 1))
                    .expect("reserve should fit within XRPAmount"),
            )
    {
        return protocol::Ter::TEC_NO_LINE_INSUF_RESERVE;
    }

    let mut obj = STObject::new(sf_generic());
    obj.set_field_u16(sf("sfLedgerEntryType"), LedgerEntryType::RippleState as u16);
    obj.set_field_amount(
        sf("sfBalance"),
        STAmount::from_iou_amount(
            sf("sfBalance"),
            IOUAmount::new(),
            Issue::new(issue.currency, protocol::no_account()),
        ),
    );
    let low_limit = STAmount::new_with_asset(
        sf("sfLowLimit"),
        Asset::Issue(Issue::new(issue.currency, if high { dst } else { src })),
        0,
        0,
        false,
    );
    let high_limit = STAmount::new_with_asset(
        sf("sfHighLimit"),
        Asset::Issue(Issue::new(issue.currency, if high { src } else { dst })),
        0,
        0,
        false,
    );
    obj.set_field_amount(sf("sfLowLimit"), low_limit);
    obj.set_field_amount(sf("sfHighLimit"), high_limit);
    let mut flags = if high {
        0x0002_0000_u32
    } else {
        0x0001_0000_u32
    };
    flags |= if high { 0x0010_0000 } else { 0x0020_0000 };
    obj.set_field_u32(sf("sfFlags"), flags);

    let low_dir = owner_dir_keylet(to_uint160(if high { dst } else { src }));
    let low_node = match crate::dir_insert(view, &low_dir, line_keylet.key, &|_| {}) {
        Ok(Some(page)) => page,
        _ => return protocol::Ter::TEF_BAD_LEDGER,
    };
    let high_dir = owner_dir_keylet(to_uint160(if high { src } else { dst }));
    let high_node = match crate::dir_insert(view, &high_dir, line_keylet.key, &|_| {}) {
        Ok(Some(page)) => page,
        _ => return protocol::Ter::TEF_BAD_LEDGER,
    };
    obj.set_field_u64(sf("sfLowNode"), low_node);
    obj.set_field_u64(sf("sfHighNode"), high_node);

    if view
        .insert(Arc::new(STLedgerEntry::from_stobject(obj, line_keylet.key)))
        .is_err()
    {
        return protocol::Ter::TEF_BAD_LEDGER;
    }
    let _ = crate::adjust_owner_count(view, &dst_sle, 1);
    protocol::Ter::TES_SUCCESS
}

fn remove_empty_iou_holding<V: ApplyView>(
    view: &mut V,
    account: &AccountID,
    issue: &Issue,
) -> protocol::Ter {
    if issue.native() {
        let Ok(Some(sle)) = view.peek(account_keylet(to_uint160(*account))) else {
            return protocol::Ter::TEC_INTERNAL;
        };
        if sle.get_field_amount(sf("sfBalance")).xrp().drops() != 0 {
            return protocol::Ter::TEC_HAS_OBLIGATIONS;
        }
        return protocol::Ter::TES_SUCCESS;
    }

    let account_is_issuer = *account == issue.issuer();
    let line_keylet = line(*account, issue.issuer(), issue.currency);
    let Ok(Some(line_sle)) = view.peek(line_keylet) else {
        return if account_is_issuer {
            protocol::Ter::TES_SUCCESS
        } else {
            protocol::Ter::TEC_OBJECT_NOT_FOUND
        };
    };
    if !account_is_issuer && line_sle.get_field_amount(sf("sfBalance")).signum() != 0 {
        return protocol::Ter::TEC_HAS_OBLIGATIONS;
    }

    let low_limit = line_sle.get_field_amount(sf("sfLowLimit")).issue().issuer();
    let high_limit = line_sle
        .get_field_amount(sf("sfHighLimit"))
        .issue()
        .issuer();
    if !account_is_issuer
        && let Ok(Some(acct_sle)) = view.peek(account_keylet(to_uint160(*account)))
    {
        let _ = crate::adjust_owner_count(view, &acct_sle, -1);
    }

    let _ = crate::dir_remove(
        view,
        &owner_dir_keylet(to_uint160(low_limit)),
        line_sle.get_field_u64(sf("sfLowNode")),
        *line_sle.key(),
        false,
    );
    let _ = crate::dir_remove(
        view,
        &owner_dir_keylet(to_uint160(high_limit)),
        line_sle.get_field_u64(sf("sfHighNode")),
        *line_sle.key(),
        false,
    );
    if view.erase(line_sle).is_err() {
        return protocol::Ter::TEF_BAD_LEDGER;
    }
    erase_empty_owner_dir_root(view, &low_limit);
    erase_empty_owner_dir_root(view, &high_limit);
    protocol::Ter::TES_SUCCESS
}

fn add_empty_mpt_holding<V: ApplyView>(
    view: &mut V,
    account: &AccountID,
    prior_balance: XRPAmount,
    issue: &MPTIssue,
) -> protocol::Ter {
    let mpt_id = issue.mpt_id();
    let Ok(Some(issuance)) = view.peek(protocol::mpt_issuance_keylet_from_mptid(mpt_id)) else {
        return protocol::Ter::TEF_BAD_LEDGER;
    };
    if issuance.get_field_u32(sf("sfFlags")) & protocol::lsfMPTLocked != 0 {
        return protocol::Ter::TEF_BAD_LEDGER;
    }
    if view
        .peek(protocol::mptoken_keylet_from_mptid(
            mpt_id,
            to_uint160(*account),
        ))
        .ok()
        .flatten()
        .is_some()
    {
        return protocol::Ter::TEC_DUPLICATE;
    }
    if *account == issue.issuer() {
        return protocol::Ter::TES_SUCCESS;
    }

    let Ok(Some(acct_sle)) = view.peek(account_keylet(to_uint160(*account))) else {
        return protocol::Ter::TEC_INTERNAL;
    };
    let owner_count = acct_sle.get_field_u32(sf("sfOwnerCount"));
    let reserve = if owner_count < 2 {
        XRPAmount::from_drops(0)
    } else {
        XRPAmount::from_drops(
            i64::try_from(view.fees().account_reserve(owner_count as usize + 1))
                .expect("reserve should fit within XRPAmount"),
        )
    };
    if prior_balance < reserve {
        return protocol::Ter::TEC_INSUFFICIENT_RESERVE;
    }

    let token_keylet = protocol::mptoken_keylet_from_mptid(mpt_id, to_uint160(*account));
    let owner_dir = owner_dir_keylet(to_uint160(*account));
    let owner_node = match crate::dir_append(view, &owner_dir, token_keylet.key, &|_| {}) {
        Ok(Some(page)) => page,
        _ => return protocol::Ter::TEF_BAD_LEDGER,
    };

    let mut token = STLedgerEntry::new(token_keylet);
    token.set_account_id(sf("sfAccount"), *account);
    token.set_field_h192(sf("sfMPTokenIssuanceID"), mpt_id);
    token.set_field_u64(sf("sfMPTAmount"), 0);
    token.set_field_u32(sf("sfFlags"), 0);
    token.set_field_u64(sf("sfOwnerNode"), owner_node);
    if view.insert(Arc::new(token)).is_err() {
        return protocol::Ter::TEF_BAD_LEDGER;
    }
    let _ = crate::adjust_owner_count(view, &acct_sle, 1);
    protocol::Ter::TES_SUCCESS
}

fn remove_empty_mpt_holding<V: ApplyView>(
    view: &mut V,
    account: &AccountID,
    issue: &MPTIssue,
) -> protocol::Ter {
    let account_is_issuer = *account == issue.issuer();
    let token_keylet = protocol::mptoken_keylet_from_mptid(issue.mpt_id(), to_uint160(*account));
    let Ok(Some(token_sle)) = view.peek(token_keylet) else {
        return if account_is_issuer {
            protocol::Ter::TES_SUCCESS
        } else {
            protocol::Ter::TEC_OBJECT_NOT_FOUND
        };
    };
    if token_sle.get_field_u64(sf("sfMPTAmount")) != 0 {
        return protocol::Ter::TEC_HAS_OBLIGATIONS;
    }
    if token_sle.is_field_present(sf("sfLockedAmount"))
        && token_sle.get_field_u64(sf("sfLockedAmount")) != 0
    {
        return protocol::Ter::TEC_HAS_OBLIGATIONS;
    }
    let _ = crate::dir_remove(
        view,
        &owner_dir_keylet(to_uint160(*account)),
        token_sle.get_field_u64(sf("sfOwnerNode")),
        *token_sle.key(),
        false,
    );
    let Ok(Some(acct_sle)) = view.peek(account_keylet(to_uint160(*account))) else {
        return protocol::Ter::TEF_BAD_LEDGER;
    };
    let _ = crate::adjust_owner_count(view, &acct_sle, -1);
    if view.erase(token_sle).is_err() {
        return protocol::Ter::TEF_BAD_LEDGER;
    }
    erase_empty_owner_dir_root(view, account);
    protocol::Ter::TES_SUCCESS
}

#[cfg(test)]
mod tests {
    use super::{FreezeHandling, account_funds, xrp_liquid};
    use crate::{Fees, Ledger, LedgerHeader};
    use basics::base_uint::{Uint160, Uint256};
    use protocol::{
        AccountID, Currency, IOUAmount, Issue, LedgerEntryType, STAmount, STLedgerEntry,
        account_keylet, currency_from_string, get_field_by_symbol, line, sf_generic,
    };
    use shamap::item::SHAMapItem;
    use shamap::mutation::MutableTree;
    use shamap::sync::{SHAMapType, SyncState, SyncTree};
    use shamap::tree_node::SHAMapNodeType;

    fn sample_uint256(fill: u8) -> Uint256 {
        Uint256::from_array([fill; 32])
    }

    fn sample_account(fill: u8) -> Uint160 {
        Uint160::from_array([fill; 20])
    }

    fn to_account_id(account: Uint160) -> AccountID {
        AccountID::from_slice(account.data()).expect("account width should match")
    }

    fn build_ledger(entries: &[(Uint256, Vec<u8>)], fees: Fees) -> Ledger {
        let seq = 88;
        let mut tree = MutableTree::new(seq);
        for (key, payload) in entries {
            tree.add_item(
                SHAMapNodeType::AccountState,
                SHAMapItem::new(*key, payload.clone()),
            )
            .expect("state item should insert");
        }

        let mut ledger = Ledger::from_maps(
            LedgerHeader {
                seq,
                ..LedgerHeader::default()
            },
            SyncTree::from_root_with_type(
                tree.root(),
                SHAMapType::State,
                false,
                seq,
                SyncState::Immutable,
            ),
            SyncTree::new_with_type(SHAMapType::Transaction, false, seq),
        );
        ledger.set_fees(fees);
        ledger
    }

    fn account_root_entry(account: Uint160, balance: u64, owner_count: u32) -> Vec<u8> {
        let mut entry = STLedgerEntry::from_type_and_key(
            LedgerEntryType::AccountRoot,
            account_keylet(account).key,
        );
        entry.set_account_id(get_field_by_symbol("sfAccount"), to_account_id(account));
        entry.set_field_u32(get_field_by_symbol("sfSequence"), 1);
        entry.set_field_amount(
            get_field_by_symbol("sfBalance"),
            STAmount::new_native(balance, false),
        );
        entry.set_field_u32(get_field_by_symbol("sfOwnerCount"), owner_count);
        entry.set_field_h256(get_field_by_symbol("sfPreviousTxnID"), sample_uint256(0x51));
        entry.set_field_u32(get_field_by_symbol("sfPreviousTxnLgrSeq"), 1);
        entry.get_serializer().data().to_vec()
    }

    fn trustline_entry(low: Uint160, high: Uint160, currency: Currency, balance: i64) -> Vec<u8> {
        let mut entry = STLedgerEntry::from_type_and_key(
            LedgerEntryType::RippleState,
            line(to_account_id(low), to_account_id(high), currency).key,
        );
        entry.set_field_h256(get_field_by_symbol("sfPreviousTxnID"), sample_uint256(0x61));
        entry.set_field_u32(get_field_by_symbol("sfPreviousTxnLgrSeq"), 1);
        entry.set_field_amount(
            get_field_by_symbol("sfBalance"),
            STAmount::from_iou_amount(
                sf_generic(),
                IOUAmount::from_parts(balance, 0).expect("trustline balance should normalize"),
                Issue::new(currency, to_account_id(low)),
            ),
        );
        entry.set_field_amount(
            get_field_by_symbol("sfLowLimit"),
            STAmount::from_iou_amount(
                sf_generic(),
                IOUAmount::from_parts(100, 0).expect("low limit should normalize"),
                Issue::new(currency, to_account_id(low)),
            ),
        );
        entry.set_field_amount(
            get_field_by_symbol("sfHighLimit"),
            STAmount::from_iou_amount(
                sf_generic(),
                IOUAmount::from_parts(100, 0).expect("high limit should normalize"),
                Issue::new(currency, to_account_id(high)),
            ),
        );
        entry.get_serializer().data().to_vec()
    }

    #[test]
    fn xrp_liquid_subtracts_reserve() {
        let account = sample_account(0x11);
        let ledger = build_ledger(
            &[(
                account_keylet(account).key,
                account_root_entry(account, 1_000, 2),
            )],
            Fees {
                base: 10,
                reserve: 200,
                increment: 50,
            },
        );

        let liquid = xrp_liquid(&ledger, to_account_id(account), 0)
            .expect("xrp liquid lookup should succeed");

        assert_eq!(liquid.drops(), 700);
    }

    #[test]
    fn account_funds_returns_trustline_balance_for_non_issuer() {
        let low = sample_account(0x21);
        let high = sample_account(0x31);
        let currency = currency_from_string("USD");
        let issue = Issue::new(currency, to_account_id(high));
        let ledger = build_ledger(
            &[(
                line(to_account_id(low), to_account_id(high), currency).key,
                trustline_entry(low, high, currency, 77),
            )],
            Fees::default(),
        );

        let funds = account_funds(
            &ledger,
            to_account_id(low),
            &STAmount::from_iou_amount(
                get_field_by_symbol("sfTakerGets"),
                IOUAmount::from_parts(1, 0).expect("offer amount should normalize"),
                issue,
            ),
            FreezeHandling::IgnoreFreeze,
        )
        .expect("account funds lookup should succeed");

        assert_eq!(funds.issue(), issue);
        assert_eq!(
            funds.iou(),
            IOUAmount::from_parts(77, 0).expect("expected canonical amount")
        );
    }
}
