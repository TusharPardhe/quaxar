use basics::base_uint::{Uint160, Uint256};
use basics::number::NumberParts as RuntimeNumber;
use ledger::{
    Ledger, LedgerHeader,
    vault_helpers::{
        TruncateShares, WaiveUnrealizedLoss, assets_to_shares_withdraw, is_sole_shareholder,
        shares_to_assets_withdraw,
    },
};
use protocol::{
    AccountID, Asset, IOUAmount, Issue, MPTAmount, MPTIssue, Rules, STAmount, STIssue,
    STLedgerEntry, STNumber, currency_from_string, get_field_by_symbol, make_mpt_id,
    mpt_issuance_keylet_from_mptid, mptoken_keylet_from_mptid, sf_generic, vault_keylet,
};
use shamap::item::SHAMapItem;
use shamap::mutation::MutableTree;
use shamap::sync::{SHAMapType, SyncState, SyncTree};
use shamap::tree_node::SHAMapNodeType;

fn sf(name: &str) -> &'static protocol::SField {
    get_field_by_symbol(name)
}

fn account(byte: u8) -> AccountID {
    AccountID::from_array([byte; 20])
}

fn account_raw(account: AccountID) -> Uint160 {
    Uint160::from_slice(account.data()).expect("account width")
}

fn asset_number(asset: Asset, value: i64) -> STNumber {
    let mut number = STNumber::from(RuntimeNumber::from_i64(value));
    number.associate_asset(asset);
    number
}

fn amount_number(amount: &STAmount) -> RuntimeNumber {
    if amount.native() {
        RuntimeNumber::from(amount.xrp())
    } else if amount.holds_mpt_issue() {
        RuntimeNumber::from(amount.mpt())
    } else {
        RuntimeNumber::from(amount.iou())
    }
}

fn asset_amount(asset: Asset, value: i64) -> STAmount {
    match asset {
        Asset::Issue(issue) if issue.native() => {
            STAmount::from_xrp_amount(protocol::XRPAmount::from_drops(value))
        }
        Asset::Issue(issue) => STAmount::from_iou_amount(
            sf_generic(),
            IOUAmount::from_number(RuntimeNumber::from_i64(value)).expect("iou amount"),
            issue,
        ),
        Asset::MPTIssue(issue) => {
            STAmount::from_mpt_amount(sf_generic(), MPTAmount::from_value(value), issue)
        }
    }
}

fn share_amount(share_id: protocol::MPTID, value: i64) -> STAmount {
    STAmount::from_mpt_amount(
        sf_generic(),
        MPTAmount::from_value(value),
        MPTIssue::new(share_id),
    )
}

fn vault_entry(
    owner: AccountID,
    pseudo: AccountID,
    sequence: u32,
    asset: Asset,
    share_id: protocol::MPTID,
    assets_total: i64,
    loss_unrealized: i64,
) -> STLedgerEntry {
    let mut entry = STLedgerEntry::new(vault_keylet(account_raw(owner), sequence));
    entry.set_field_u32(sf("sfFlags"), 0);
    entry.set_field_h256(sf("sfPreviousTxnID"), Uint256::from_array([0xB1; 32]));
    entry.set_field_u32(sf("sfPreviousTxnLgrSeq"), 1);
    entry.set_field_u32(sf("sfSequence"), sequence);
    entry.set_field_u64(sf("sfOwnerNode"), 0);
    entry.set_account_id(sf("sfOwner"), owner);
    entry.set_account_id(sf("sfAccount"), pseudo);
    entry.set_field_issue(sf("sfAsset"), STIssue::new_with_asset(sf("sfAsset"), asset));
    entry.set_field_h192(sf("sfShareMPTID"), share_id);
    entry.set_field_number(sf("sfAssetsTotal"), asset_number(asset, assets_total));
    entry.set_field_number(sf("sfAssetsAvailable"), asset_number(asset, assets_total));
    entry.set_field_number(sf("sfLossUnrealized"), asset_number(asset, loss_unrealized));
    entry
}

fn issuance_entry(issuer: AccountID, sequence: u32, outstanding: u64) -> STLedgerEntry {
    let share_id = make_mpt_id(sequence, issuer);
    let mut entry = STLedgerEntry::new(mpt_issuance_keylet_from_mptid(share_id));
    entry.set_account_id(sf("sfIssuer"), issuer);
    entry.set_field_u32(sf("sfSequence"), sequence);
    entry.set_field_u64(sf("sfOutstandingAmount"), outstanding);
    entry.set_field_u32(sf("sfFlags"), protocol::lsfMPTCanTransfer);
    entry.set_field_u64(sf("sfOwnerNode"), 0);
    entry
}

fn mptoken_entry(holder: AccountID, share_id: protocol::MPTID, amount: u64) -> STLedgerEntry {
    let mut entry = STLedgerEntry::new(mptoken_keylet_from_mptid(share_id, account_raw(holder)));
    entry.set_account_id(sf("sfAccount"), holder);
    entry.set_field_h192(sf("sfMPTokenIssuanceID"), share_id);
    entry.set_field_u64(sf("sfMPTAmount"), amount);
    entry.set_field_u32(sf("sfFlags"), 0);
    entry.set_field_u64(sf("sfOwnerNode"), 0);
    entry
}

fn ledger_with(entries: impl IntoIterator<Item = STLedgerEntry>) -> Ledger {
    let mut tree = MutableTree::new(1);
    for entry in entries {
        tree.add_item(
            SHAMapNodeType::AccountState,
            SHAMapItem::new(*entry.key(), entry.get_serializer().data().to_vec()),
        )
        .expect("state insertion should succeed");
    }

    let mut ledger = Ledger::from_maps(
        LedgerHeader {
            seq: 1,
            parent_close_time: 500,
            ..LedgerHeader::default()
        },
        SyncTree::from_root_with_type(
            tree.root(),
            SHAMapType::State,
            false,
            1,
            SyncState::Immutable,
        ),
        SyncTree::new_with_type(SHAMapType::Transaction, false, 1),
    );
    ledger.set_rules(Rules::default());
    ledger
}

#[test]
fn withdraw_helpers_can_waive_unrealized_loss_for_sole_shareholder_math() {
    let owner = account(0x11);
    let pseudo = account(0x12);
    let share_issuer = account(0x13);
    let share_id = make_mpt_id(7, share_issuer);
    let asset_issuer = account(0x14);
    let asset = Asset::Issue(Issue::new(currency_from_string("USD"), asset_issuer));
    let vault = vault_entry(owner, pseudo, 1, asset, share_id, 1_000, 500);
    let issuance = issuance_entry(share_issuer, 7, 100);

    let shares = share_amount(share_id, 10);
    let normal_assets =
        shares_to_assets_withdraw(&vault, &issuance, &shares, WaiveUnrealizedLoss::No)
            .expect("normal withdraw amount");
    let waived_assets =
        shares_to_assets_withdraw(&vault, &issuance, &shares, WaiveUnrealizedLoss::Yes)
            .expect("waived withdraw amount");

    assert_eq!(amount_number(&normal_assets), RuntimeNumber::from_i64(50));
    assert_eq!(amount_number(&waived_assets), RuntimeNumber::from_i64(100));

    let assets = asset_amount(asset, 100);
    let normal_shares = assets_to_shares_withdraw(
        &vault,
        &issuance,
        &assets,
        TruncateShares::No,
        WaiveUnrealizedLoss::No,
    )
    .expect("normal shares");
    let waived_shares = assets_to_shares_withdraw(
        &vault,
        &issuance,
        &assets,
        TruncateShares::No,
        WaiveUnrealizedLoss::Yes,
    )
    .expect("waived shares");

    assert_eq!(normal_shares.mpt().value(), 20);
    assert_eq!(waived_shares.mpt().value(), 10);
}

#[test]
fn is_sole_shareholder_matches_outstanding_amount() {
    let holder = account(0x21);
    let issuer = account(0x22);
    let share_id = make_mpt_id(9, issuer);
    let issuance = issuance_entry(issuer, 9, 50);

    let ledger = ledger_with([issuance.clone(), mptoken_entry(holder, share_id, 50)]);
    assert!(is_sole_shareholder(&ledger, &holder, &issuance).expect("read sole holder state"));

    let ledger = ledger_with([issuance.clone(), mptoken_entry(holder, share_id, 49)]);
    assert!(!is_sole_shareholder(&ledger, &holder, &issuance).expect("read partial holder state"));

    let ledger = ledger_with([issuance.clone()]);
    assert!(!is_sole_shareholder(&ledger, &holder, &issuance).expect("read missing holder state"));
}
