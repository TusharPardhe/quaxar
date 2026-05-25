use basics::base_uint::{Uint160, Uint256};
use ledger::{
    FreezeHandling, Ledger, LedgerHeader, amm_account_holds, amm_holds, amm_lp_holds,
    get_trading_fee, is_only_liquidity_provider,
};
use protocol::{
    AccountID, IOUAmount, Issue, Keylet, LedgerEntryType, STAmount, STArray, STIssue,
    STLedgerEntry, STObject, STVector256, account_keylet, currency_from_string,
    get_field_by_symbol, line, owner_dir_keylet, page_keylet, sf_generic,
};
use shamap::item::SHAMapItem;
use shamap::mutation::MutableTree;
use shamap::sync::{SHAMapType, SyncState, SyncTree};
use shamap::tree_node::SHAMapNodeType;

fn sample_key(fill: u8) -> Uint256 {
    Uint256::from_array([fill; 32])
}

fn sample_account(fill: u8) -> AccountID {
    AccountID::from_array([fill; 20])
}

fn owner_root(owner: AccountID) -> Keylet {
    owner_dir_keylet(Uint160::from_slice(owner.data()).expect("account width"))
}

fn issue(currency: &str, issuer: AccountID) -> Issue {
    Issue::new(currency_from_string(currency), issuer)
}

fn build_ledger(header: LedgerHeader, items: &[(Uint256, Vec<u8>)]) -> Ledger {
    let cowid = header.seq.max(1);
    let mut tree = MutableTree::new(cowid);
    for (key, payload) in items {
        tree.add_item(
            SHAMapNodeType::AccountState,
            SHAMapItem::new(*key, payload.clone()),
        )
        .expect("state item should insert");
    }

    Ledger::from_maps(
        header,
        SyncTree::from_root_with_type(
            tree.root(),
            SHAMapType::State,
            false,
            cowid,
            SyncState::Immutable,
        ),
        SyncTree::new_with_type(SHAMapType::Transaction, false, cowid),
    )
}

fn account_root_entry(account: AccountID, balance: u64, flags: u32) -> Vec<u8> {
    let mut entry = STLedgerEntry::from_type_and_key(
        LedgerEntryType::AccountRoot,
        account_keylet(Uint160::from_slice(account.data()).expect("account width")).key,
    );
    entry.set_account_id(get_field_by_symbol("sfAccount"), account);
    entry.set_field_amount(
        get_field_by_symbol("sfBalance"),
        STAmount::new_native(balance, false),
    );
    entry.set_field_u32(get_field_by_symbol("sfSequence"), 1);
    entry.set_field_u32(get_field_by_symbol("sfOwnerCount"), 0);
    entry.set_field_h256(get_field_by_symbol("sfPreviousTxnID"), sample_key(0xA1));
    entry.set_field_u32(get_field_by_symbol("sfPreviousTxnLgrSeq"), 1);
    if flags != 0 {
        entry.set_field_u32(get_field_by_symbol("sfFlags"), flags);
    }
    entry.get_serializer().data().to_vec()
}

fn trustline_entry(
    low: AccountID,
    high: AccountID,
    currency: &str,
    balance: i64,
    flags: u32,
) -> Vec<u8> {
    let currency = currency_from_string(currency);
    let mut entry = STLedgerEntry::from_type_and_key(
        LedgerEntryType::RippleState,
        line(low, high, currency).key,
    );
    entry.set_field_amount(
        get_field_by_symbol("sfBalance"),
        STAmount::from_iou_amount(
            sf_generic(),
            IOUAmount::from_parts(balance, 0).expect("trustline balance"),
            Issue::new(currency, low),
        ),
    );
    entry.set_field_amount(
        get_field_by_symbol("sfLowLimit"),
        STAmount::from_iou_amount(
            sf_generic(),
            IOUAmount::from_parts(1_000_000, 0).expect("low limit"),
            Issue::new(currency, low),
        ),
    );
    entry.set_field_amount(
        get_field_by_symbol("sfHighLimit"),
        STAmount::from_iou_amount(
            sf_generic(),
            IOUAmount::from_parts(1_000_000, 0).expect("high limit"),
            Issue::new(currency, high),
        ),
    );
    entry.set_field_h256(get_field_by_symbol("sfPreviousTxnID"), sample_key(0xA2));
    entry.set_field_u32(get_field_by_symbol("sfPreviousTxnLgrSeq"), 1);
    if flags != 0 {
        entry.set_field_u32(get_field_by_symbol("sfFlags"), flags);
    }
    entry.get_serializer().data().to_vec()
}

fn make_vote_entry(account: AccountID) -> STObject {
    let mut vote = STObject::make_inner_object(get_field_by_symbol("sfVoteEntry"));
    vote.set_account_id(get_field_by_symbol("sfAccount"), account);
    vote.set_field_u16(get_field_by_symbol("sfTradingFee"), 25);
    vote.set_field_u32(get_field_by_symbol("sfVoteWeight"), 12_500);
    vote
}

fn make_auction_slot(
    owner: AccountID,
    discounted_fee: u16,
    expiration: u32,
    auth_accounts: &[AccountID],
) -> STObject {
    let mut slot = STObject::make_inner_object(get_field_by_symbol("sfAuctionSlot"));
    slot.set_account_id(get_field_by_symbol("sfAccount"), owner);
    slot.set_field_u16(get_field_by_symbol("sfDiscountedFee"), discounted_fee);
    slot.set_field_u32(get_field_by_symbol("sfExpiration"), expiration);
    slot.set_field_amount(
        get_field_by_symbol("sfPrice"),
        STAmount::new_with_asset(
            get_field_by_symbol("sfPrice"),
            issue("LPT", owner),
            100,
            0,
            false,
        ),
    );
    if !auth_accounts.is_empty() {
        let mut auth = STArray::new(get_field_by_symbol("sfAuthAccounts"));
        for account in auth_accounts {
            let mut entry = STObject::make_inner_object(get_field_by_symbol("sfAuthAccount"));
            entry.set_account_id(get_field_by_symbol("sfAccount"), *account);
            auth.push_back(entry);
        }
        slot.set_field_array(get_field_by_symbol("sfAuthAccounts"), auth);
    }
    slot
}

fn amm_entry(amm_account: AccountID, amm_key: Uint256, issue1: Issue, issue2: Issue) -> Vec<u8> {
    let mut entry = STLedgerEntry::from_type_and_key(LedgerEntryType::AMM, amm_key);
    entry.set_account_id(get_field_by_symbol("sfAccount"), amm_account);
    entry.set_field_u16(get_field_by_symbol("sfTradingFee"), 17);
    entry.set_field_amount(
        get_field_by_symbol("sfLPTokenBalance"),
        STAmount::from_iou_amount(
            sf_generic(),
            IOUAmount::from_parts(5_600, 0).expect("lp token balance"),
            issue("LPT", amm_account),
        ),
    );
    entry.set_field_issue(
        get_field_by_symbol("sfAsset"),
        STIssue::new_with_asset(get_field_by_symbol("sfAsset"), issue1),
    );
    entry.set_field_issue(
        get_field_by_symbol("sfAsset2"),
        STIssue::new_with_asset(get_field_by_symbol("sfAsset2"), issue2),
    );
    let mut votes = STArray::new(get_field_by_symbol("sfVoteSlots"));
    votes.push_back(make_vote_entry(sample_account(0x70)));
    entry.set_field_array(get_field_by_symbol("sfVoteSlots"), votes);
    entry.set_field_object(
        get_field_by_symbol("sfAuctionSlot"),
        make_auction_slot(
            sample_account(0x71),
            5,
            200,
            &[sample_account(0x72), sample_account(0x73)],
        ),
    );
    entry.get_serializer().data().to_vec()
}

fn directory_page_payload(root: Keylet, page: u64, indexes: &[Uint256], next: u64) -> Vec<u8> {
    let mut entry = STLedgerEntry::new(page_keylet(root, page));
    entry.set_field_v256(
        get_field_by_symbol("sfIndexes"),
        STVector256::from_values(get_field_by_symbol("sfIndexes"), indexes.to_vec()),
    );
    entry.set_field_u64(get_field_by_symbol("sfIndexNext"), next);
    entry.set_field_u64(
        get_field_by_symbol("sfIndexPrevious"),
        if page == 0 { 0 } else { page - 1 },
    );
    entry.get_serializer().data().to_vec()
}

#[test]
fn amm_holds_reorders_optional_issue_and_rejects_invalid_pairs() {
    let amm_account = sample_account(0x11);
    let usd = issue("USD", sample_account(0x21));
    let eur = issue("EUR", sample_account(0x22));
    let key = sample_key(0x31);
    let entry = STLedgerEntry::from_serial_iter(
        &mut protocol::SerialIter::new(&amm_entry(amm_account, key, usd, eur)),
        key,
    );
    let ledger = build_ledger(
        LedgerHeader::default(),
        &[
            (
                line(amm_account, usd.account, usd.currency).key,
                trustline_entry(amm_account, usd.account, "USD", 77, 0),
            ),
            (
                line(amm_account, eur.account, eur.currency).key,
                trustline_entry(amm_account, eur.account, "EUR", 55, 0),
            ),
        ],
    );

    let reordered = amm_holds(
        &ledger,
        &entry,
        Some(eur),
        None,
        FreezeHandling::IgnoreFreeze,
    );
    assert!(reordered.has_value());
    let (first, second, lp) = reordered.value();
    assert_eq!(first.iou(), IOUAmount::from_parts(55, 0).expect("eur"));
    assert_eq!(second.iou(), IOUAmount::from_parts(77, 0).expect("usd"));
    assert_eq!(lp.iou(), IOUAmount::from_parts(5_600, 0).expect("lp"));

    let invalid = amm_holds(
        &ledger,
        &entry,
        Some(issue("JPY", sample_account(0x23))),
        Some(eur),
        FreezeHandling::IgnoreFreeze,
    );
    assert!(!invalid.has_value());
    assert_eq!(*invalid.error(), protocol::Ter::TEC_AMM_INVALID_TOKENS);
}

#[test]
fn get_trading_fee_matches_owner_auth_and_expiry_paths() {
    let amm_account = sample_account(0x41);
    let usd = issue("USD", sample_account(0x42));
    let eur = issue("EUR", sample_account(0x43));
    let key = sample_key(0x44);
    let entry = STLedgerEntry::from_serial_iter(
        &mut protocol::SerialIter::new(&amm_entry(amm_account, key, usd, eur)),
        key,
    );
    let open = build_ledger(
        LedgerHeader {
            parent_close_time: 100,
            ..LedgerHeader::default()
        },
        &[],
    );
    let expired = build_ledger(
        LedgerHeader {
            parent_close_time: 200,
            ..LedgerHeader::default()
        },
        &[],
    );

    assert_eq!(get_trading_fee(&open, &entry, sample_account(0x71)), 5);
    assert_eq!(get_trading_fee(&open, &entry, sample_account(0x72)), 5);
    assert_eq!(get_trading_fee(&open, &entry, sample_account(0x99)), 17);
    assert_eq!(get_trading_fee(&expired, &entry, sample_account(0x71)), 17);
}

#[test]
fn amm_lp_holds_and_account_holds_handle_missing_and_frozen_paths() {
    let amm_account = sample_account(0x51);
    let lp_account = sample_account(0x61);
    let gateway = sample_account(0x71);
    let usd = issue("USD", gateway);
    let header = LedgerHeader::default();
    let ledger = build_ledger(
        header,
        &[
            (
                account_keylet(Uint160::from_slice(amm_account.data()).expect("account width")).key,
                account_root_entry(amm_account, 1_000, 0),
            ),
            (
                line(
                    lp_account,
                    amm_account,
                    protocol::amm_lpt_currency(usd.currency, currency_from_string("EUR")),
                )
                .key,
                trustline_entry(lp_account, amm_account, "LPT", 90, protocol::lsfLowFreeze),
            ),
            (
                line(amm_account, gateway, usd.currency).key,
                trustline_entry(amm_account, gateway, "USD", 33, 0),
            ),
        ],
    );

    let lp_holds = amm_lp_holds(
        &ledger,
        usd.currency,
        currency_from_string("EUR"),
        amm_account,
        lp_account,
    )
    .expect("lp holds");
    assert_eq!(lp_holds.signum(), 0);

    let xrp_balance = amm_account_holds(&ledger, amm_account, protocol::xrp_issue()).expect("xrp");
    assert_eq!(xrp_balance.xrp().drops(), 1_000);

    let usd_balance = amm_account_holds(&ledger, amm_account, usd).expect("usd");
    assert_eq!(
        usd_balance.iou(),
        IOUAmount::from_parts(33, 0).expect("usd")
    );
}

#[test]
fn is_only_liquidity_provider_matches_owner_dir_shape() {
    let amm_account = sample_account(0x81);
    let lp_account = sample_account(0x82);
    let other_lp = sample_account(0x83);
    let gateway = sample_account(0x84);
    let amm_issue = issue("LPT", amm_account);
    let root = owner_root(amm_account);
    let amm_key = sample_key(0x91);
    let lp_line = line(lp_account, amm_account, amm_issue.currency).key;
    let asset_line = line(amm_account, gateway, currency_from_string("USD")).key;
    let other_lp_line = line(other_lp, amm_account, amm_issue.currency).key;

    let true_ledger = build_ledger(
        LedgerHeader::default(),
        &[
            (
                root.key,
                directory_page_payload(root, 0, &[amm_key, lp_line, asset_line], 0),
            ),
            (
                amm_key,
                amm_entry(
                    amm_account,
                    amm_key,
                    issue("USD", gateway),
                    issue("EUR", gateway),
                ),
            ),
            (
                lp_line,
                trustline_entry(lp_account, amm_account, "LPT", 10, 0),
            ),
            (
                asset_line,
                trustline_entry(amm_account, gateway, "USD", 10, 0),
            ),
        ],
    );
    let result = is_only_liquidity_provider(&true_ledger, amm_issue, lp_account);
    assert!(result.has_value());
    assert!(*result.value());

    let false_ledger = build_ledger(
        LedgerHeader::default(),
        &[
            (
                root.key,
                directory_page_payload(root, 0, &[amm_key, lp_line, asset_line, other_lp_line], 0),
            ),
            (
                amm_key,
                amm_entry(
                    amm_account,
                    amm_key,
                    issue("USD", gateway),
                    issue("EUR", gateway),
                ),
            ),
            (
                lp_line,
                trustline_entry(lp_account, amm_account, "LPT", 10, 0),
            ),
            (
                asset_line,
                trustline_entry(amm_account, gateway, "USD", 10, 0),
            ),
            (
                other_lp_line,
                trustline_entry(other_lp, amm_account, "LPT", 10, 0),
            ),
        ],
    );
    let result = is_only_liquidity_provider(&false_ledger, amm_issue, lp_account);
    assert!(result.has_value());
    assert!(!*result.value());
}
