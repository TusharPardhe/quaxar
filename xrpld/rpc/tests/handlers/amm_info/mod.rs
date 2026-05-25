//! Tests for the amm_info RPC handler.

//! Tests for the amm info RPC handler.

use std::{collections::BTreeMap, time::Duration};

use basics::base_uint::{Uint160, Uint256};
use protocol::{
    AccountID, Asset, Currency, Issue, JsonValue, LedgerEntryType, STAmount, STArray, STIssue,
    STLedgerEntry, STObject, account_keylet, amm, currency_from_string, get_field_by_symbol, line,
    lsfGlobalFreeze, lsfLowFreeze, to_base58,
};
use rpc::{
    AmmInfoRequest, AmmInfoSource, LedgerLookupLedger, LedgerLookupSource, RpcRole, do_amm_info,
};

#[derive(Debug, Default)]
struct FakeSource {
    ledger: Option<LedgerLookupLedger>,
    validated: Option<LedgerLookupLedger>,
    account_roots: BTreeMap<AccountID, STLedgerEntry>,
    entries: BTreeMap<Uint256, STLedgerEntry>,
}

impl LedgerLookupSource for FakeSource {
    fn get_ledger_by_hash(&self, hash: Uint256) -> Option<LedgerLookupLedger> {
        self.ledger.filter(|ledger| ledger.hash == hash)
    }

    fn get_ledger_by_seq(&self, seq: u32) -> Option<LedgerLookupLedger> {
        self.ledger.filter(|ledger| ledger.seq == seq)
    }

    fn get_current_ledger(&self) -> Option<LedgerLookupLedger> {
        self.ledger
    }

    fn get_closed_ledger(&self) -> Option<LedgerLookupLedger> {
        self.ledger.filter(|ledger| !ledger.open)
    }

    fn get_validated_ledger(&self) -> Option<LedgerLookupLedger> {
        self.validated
    }

    fn get_valid_ledger_index(&self) -> u32 {
        self.ledger.map(|ledger| ledger.seq).unwrap_or_default()
    }

    fn get_validated_ledger_age(&self) -> Duration {
        Duration::from_secs(1)
    }

    fn is_validated(&self, ledger: &LedgerLookupLedger) -> bool {
        self.validated == Some(*ledger)
    }
}

impl AmmInfoSource for FakeSource {
    fn read_account_root(
        &self,
        _ledger: &LedgerLookupLedger,
        account_id: AccountID,
    ) -> Option<STLedgerEntry> {
        self.account_roots.get(&account_id).cloned()
    }

    fn read_ledger_entry(
        &self,
        _ledger: &LedgerLookupLedger,
        entry_index: Uint256,
    ) -> Option<STLedgerEntry> {
        self.entries.get(&entry_index).cloned()
    }
}

pub(super) fn object(entries: impl IntoIterator<Item = (&'static str, JsonValue)>) -> JsonValue {
    JsonValue::Object(
        entries
            .into_iter()
            .map(|(key, value)| (key.to_owned(), value))
            .collect::<BTreeMap<_, _>>(),
    )
}

pub(super) fn json_object(value: &JsonValue) -> &BTreeMap<String, JsonValue> {
    let JsonValue::Object(object) = value else {
        panic!("expected object");
    };
    object
}

pub(super) fn json_string(value: &JsonValue) -> &str {
    let JsonValue::String(text) = value else {
        panic!("expected string");
    };
    text
}

pub(super) fn sample_account(fill: u8) -> AccountID {
    AccountID::from_array([fill; 20])
}

pub(super) fn sample_hash(fill: u8) -> Uint256 {
    Uint256::from_array([fill; 32])
}

pub(super) fn closed_ledger() -> LedgerLookupLedger {
    LedgerLookupLedger {
        hash: sample_hash(0xAB),
        seq: 404,
        open: false,
    }
}

fn request(params: JsonValue, api_version: u32) -> AmmInfoRequest<'static> {
    let params = Box::leak(Box::new(params));
    AmmInfoRequest {
        params,
        api_version,
        role: RpcRole::User,
    }
}

fn issue_json(issue: Issue) -> JsonValue {
    let mut map = BTreeMap::new();
    map.insert(
        "currency".to_owned(),
        JsonValue::String(protocol::currency_to_string(issue.currency)),
    );
    if !protocol::is_xrp_currency(issue.currency) {
        map.insert(
            "issuer".to_owned(),
            JsonValue::String(to_base58(issue.account)),
        );
    }
    JsonValue::Object(map)
}

pub(super) fn make_account_root(
    account: AccountID,
    amm_id: Option<Uint256>,
    frozen: bool,
) -> STLedgerEntry {
    let mut root = STLedgerEntry::from_type_and_key(
        LedgerEntryType::AccountRoot,
        account_keylet(Uint160::from_slice(account.data()).expect("account width")).key,
    );
    root.set_account_id(get_field_by_symbol("sfAccount"), account);
    if let Some(amm_id) = amm_id {
        root.set_field_h256(get_field_by_symbol("sfAMMID"), amm_id);
    }
    if frozen {
        root.set_flag(lsfGlobalFreeze);
    }
    root
}

pub(super) fn make_trust_line(
    account: AccountID,
    peer: AccountID,
    currency: Currency,
    freeze_flag: u32,
) -> STLedgerEntry {
    let (low, high) = if account < peer {
        (account, peer)
    } else {
        (peer, account)
    };
    let mut sle = STLedgerEntry::new(line(low, high, currency));
    sle.set_field_amount(
        get_field_by_symbol("sfBalance"),
        STAmount::new_with_asset(
            get_field_by_symbol("sfBalance"),
            Issue::new(currency, low),
            500,
            0,
            false,
        ),
    );
    sle.set_field_amount(
        get_field_by_symbol("sfLowLimit"),
        STAmount::new_with_asset(
            get_field_by_symbol("sfLowLimit"),
            Issue::new(currency, low),
            1000,
            0,
            false,
        ),
    );
    sle.set_field_amount(
        get_field_by_symbol("sfHighLimit"),
        STAmount::new_with_asset(
            get_field_by_symbol("sfHighLimit"),
            Issue::new(currency, high),
            1000,
            0,
            false,
        ),
    );
    sle.set_flag(freeze_flag);
    sle
}

pub(super) fn make_vote_entry(account: AccountID, trading_fee: u16, vote_weight: u16) -> STObject {
    let mut vote = STObject::new(get_field_by_symbol("sfVoteEntry"));
    vote.set_account_id(get_field_by_symbol("sfAccount"), account);
    vote.set_field_u16(get_field_by_symbol("sfTradingFee"), trading_fee);
    vote.set_field_u32(get_field_by_symbol("sfVoteWeight"), u32::from(vote_weight));
    vote
}

pub(super) fn make_auth_account(account: AccountID) -> STObject {
    let mut auth = STObject::new(get_field_by_symbol("sfAccount"));
    auth.set_account_id(get_field_by_symbol("sfAccount"), account);
    auth
}

pub(super) fn make_auction_slot(
    account: AccountID,
    price_issue: Issue,
    price_value: u64,
    discounted_fee: u32,
    expiration: u32,
    auth_accounts: &[AccountID],
) -> STObject {
    let mut slot = STObject::new(get_field_by_symbol("sfAuctionSlot"));
    slot.set_account_id(get_field_by_symbol("sfAccount"), account);
    slot.set_field_u16(
        get_field_by_symbol("sfDiscountedFee"),
        discounted_fee as u16,
    );
    slot.set_field_u32(get_field_by_symbol("sfExpiration"), expiration);
    slot.set_field_amount(
        get_field_by_symbol("sfPrice"),
        STAmount::new_with_asset(
            get_field_by_symbol("sfPrice"),
            price_issue,
            price_value,
            0,
            false,
        ),
    );

    if !auth_accounts.is_empty() {
        let mut auth = STArray::new(get_field_by_symbol("sfAuthAccounts"));
        for account in auth_accounts {
            auth.push_back(make_auth_account(*account));
        }
        slot.set_field_array(get_field_by_symbol("sfAuthAccounts"), auth);
    }

    slot
}

pub(super) fn make_amm_entry(
    amm_account: AccountID,
    amm_key: Uint256,
    issue1: Issue,
    issue2: Issue,
) -> STLedgerEntry {
    let lpt_issue = Issue::new(currency_from_string("LPT"), amm_account);
    let mut entry = STLedgerEntry::from_type_and_key(LedgerEntryType::AMM, amm_key);
    entry.set_account_id(get_field_by_symbol("sfAccount"), amm_account);
    entry.set_field_u16(get_field_by_symbol("sfTradingFee"), 17);
    entry.set_field_amount(
        get_field_by_symbol("sfLPTokenBalance"),
        STAmount::new_with_asset(
            get_field_by_symbol("sfLPTokenBalance"),
            lpt_issue,
            5_600,
            0,
            false,
        ),
    );
    entry.set_field_issue(
        get_field_by_symbol("sfAsset"),
        STIssue::new_with_asset(get_field_by_symbol("sfAsset"), Asset::from(issue1)),
    );
    entry.set_field_issue(
        get_field_by_symbol("sfAsset2"),
        STIssue::new_with_asset(get_field_by_symbol("sfAsset2"), Asset::from(issue2)),
    );

    let mut votes = STArray::new(get_field_by_symbol("sfVoteSlots"));
    votes.push_back(make_vote_entry(sample_account(0x70), 25, 12_500));
    entry.set_field_array(get_field_by_symbol("sfVoteSlots"), votes);

    let slot = make_auction_slot(
        sample_account(0x71),
        lpt_issue,
        5_600,
        17,
        123_456,
        &[sample_account(0x72), sample_account(0x73)],
    );
    entry.set_field_object(get_field_by_symbol("sfAuctionSlot"), slot);
    entry
}

fn error_fields(value: &JsonValue) -> (&str, i64, &str) {
    let JsonValue::Object(object) = value else {
        panic!("expected error object");
    };
    let JsonValue::String(error) = object.get("error").expect("error") else {
        panic!("expected error string");
    };
    let JsonValue::Signed(code) = object.get("error_code").expect("error_code") else {
        panic!("expected error code");
    };
    let JsonValue::String(message) = object.get("error_message").expect("error_message") else {
        panic!("expected error message");
    };
    (error, *code, message)
}

mod error_cases;
mod response_fields;
