//! Tests for the account currencies RPC handler.

use std::{collections::BTreeMap, time::Duration};

use basics::base_uint::{Uint160, Uint256};
use protocol::{
    AccountID, JsonValue, LedgerEntryType, STAmount, STLedgerEntry, STVector256, account_keylet,
    currency_from_string, get_field_by_symbol, line, owner_dir_keylet, parse_base58_account_id,
    to_base58,
};
use rpc::{
    AccountCurrenciesRequest, AccountCurrenciesSource, LedgerLookupLedger, LedgerLookupSource,
    RpcRole, do_account_currencies,
};

#[derive(Debug, Default)]
struct FakeSource {
    ledger: Option<LedgerLookupLedger>,
    account_roots: BTreeMap<AccountID, STLedgerEntry>,
    owner_pages: BTreeMap<(AccountID, u64), STLedgerEntry>,
    children: BTreeMap<Uint256, STLedgerEntry>,
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
        self.ledger.filter(|ledger| !ledger.open)
    }

    fn get_valid_ledger_index(&self) -> u32 {
        self.ledger.map(|ledger| ledger.seq).unwrap_or_default()
    }

    fn get_validated_ledger_age(&self) -> Duration {
        Duration::from_secs(1)
    }

    fn is_validated(&self, ledger: &LedgerLookupLedger) -> bool {
        !ledger.open && self.ledger == Some(*ledger)
    }
}

impl AccountCurrenciesSource for FakeSource {
    fn read_account_root(
        &self,
        _ledger: &LedgerLookupLedger,
        account_id: AccountID,
    ) -> Option<STLedgerEntry> {
        self.account_roots.get(&account_id).cloned()
    }

    fn read_owner_dir_page(
        &self,
        _ledger: &LedgerLookupLedger,
        account_id: AccountID,
        page_index: u64,
    ) -> Option<STLedgerEntry> {
        self.owner_pages.get(&(account_id, page_index)).cloned()
    }

    fn read_child_entry(
        &self,
        _ledger: &LedgerLookupLedger,
        entry_index: Uint256,
    ) -> Option<STLedgerEntry> {
        self.children.get(&entry_index).cloned()
    }
}

fn object(entries: impl IntoIterator<Item = (&'static str, JsonValue)>) -> JsonValue {
    JsonValue::Object(
        entries
            .into_iter()
            .map(|(key, value)| (key.to_owned(), value))
            .collect::<BTreeMap<_, _>>(),
    )
}

fn sample_account(fill: u8) -> AccountID {
    AccountID::from_array([fill; 20])
}

fn sample_hash(fill: u8) -> Uint256 {
    Uint256::from_array([fill; 32])
}

fn closed_ledger() -> LedgerLookupLedger {
    LedgerLookupLedger {
        hash: sample_hash(0xAB),
        seq: 303,
        open: false,
    }
}

fn make_account_root(account: AccountID) -> STLedgerEntry {
    let mut sle = STLedgerEntry::from_type_and_key(
        LedgerEntryType::AccountRoot,
        account_keylet(Uint160::from_slice(account.data()).expect("account width")).key,
    );
    sle.set_account_id(get_field_by_symbol("sfAccount"), account);
    sle
}

fn make_owner_page(account: AccountID, entries: &[Uint256]) -> STLedgerEntry {
    let mut page = STLedgerEntry::new(owner_dir_keylet(
        Uint160::from_slice(account.data()).expect("account width"),
    ));
    let mut indexes = STVector256::with_field(get_field_by_symbol("sfIndexes"));
    for entry in entries {
        indexes.push_back(*entry);
    }
    page.set_field_v256(get_field_by_symbol("sfIndexes"), indexes);
    page
}

fn make_trust_line(
    account: AccountID,
    peer: AccountID,
    currency: &str,
    balance_value: u64,
    balance_negative: bool,
    low_limit: u64,
    high_limit: u64,
) -> STLedgerEntry {
    let (low, high) = if account < peer {
        (account, peer)
    } else {
        (peer, account)
    };
    let currency = currency_from_string(currency);
    let mut sle = STLedgerEntry::new(line(low, high, currency));
    sle.set_field_amount(
        get_field_by_symbol("sfBalance"),
        STAmount::new_with_asset(
            get_field_by_symbol("sfBalance"),
            protocol::Issue::new(currency, low),
            balance_value,
            0,
            balance_negative,
        ),
    );
    sle.set_field_amount(
        get_field_by_symbol("sfLowLimit"),
        STAmount::new_with_asset(
            get_field_by_symbol("sfLowLimit"),
            protocol::Issue::new(currency, low),
            low_limit,
            0,
            false,
        ),
    );
    sle.set_field_amount(
        get_field_by_symbol("sfHighLimit"),
        STAmount::new_with_asset(
            get_field_by_symbol("sfHighLimit"),
            protocol::Issue::new(currency, high),
            high_limit,
            0,
            false,
        ),
    );
    sle
}

fn request(params: JsonValue, api_version: u32) -> AccountCurrenciesRequest<'static> {
    let params = Box::leak(Box::new(params));
    AccountCurrenciesRequest {
        params,
        api_version,
        role: RpcRole::User,
    }
}

fn json_object(value: &JsonValue) -> &BTreeMap<String, JsonValue> {
    let JsonValue::Object(object) = value else {
        panic!("expected object");
    };
    object
}

#[test]
fn account_currencies_reports_send_and_receive() {
    let alice = sample_account(0x11);
    let gateway = sample_account(0x22);
    let peer = sample_account(0x33);

    let usd = sample_hash(0x01);
    let eur = sample_hash(0x02);
    let jpy = sample_hash(0x03);

    let mut source = FakeSource {
        ledger: Some(closed_ledger()),
        ..Default::default()
    };
    source.account_roots.insert(alice, make_account_root(alice));
    source
        .owner_pages
        .insert((alice, 0), make_owner_page(alice, &[usd, eur, jpy]));
    source.children.insert(
        usd,
        make_trust_line(alice, gateway, "USD", 50, true, 100, 0),
    );
    source.children.insert(
        eur,
        make_trust_line(alice, gateway, "EUR", 100, true, 100, 0),
    );
    source
        .children
        .insert(jpy, make_trust_line(alice, peer, "JPY", 25, false, 0, 100));

    let result = do_account_currencies(
        &request(
            object([("account", JsonValue::String(to_base58(alice)))]),
            2,
        ),
        &source,
    );
    let object = json_object(&result);
    assert_eq!(
        object.get("receive_currencies"),
        Some(&JsonValue::Array(vec![
            JsonValue::String("EUR".to_owned()),
            JsonValue::String("USD".to_owned()),
        ]))
    );
    assert_eq!(
        object.get("send_currencies"),
        Some(&JsonValue::Array(vec![JsonValue::String("JPY".to_owned())]))
    );
}

#[test]
fn account_currencies_reports_cpp_style_input_errors() {
    let alice = parse_base58_account_id("rHb9CJAWyB4rj91VRWn96DkukG4bwdtyTh")
        .expect("known account should parse");
    let source = FakeSource {
        ledger: Some(closed_ledger()),
        ..Default::default()
    };

    let missing = do_account_currencies(&request(object([]), 2), &source);
    assert_eq!(
        json_object(&missing).get("error"),
        Some(&JsonValue::String("invalidParams".to_owned()))
    );

    let invalid_ident = do_account_currencies(
        &request(object([("ident", JsonValue::Unsigned(1))]), 2),
        &source,
    );
    assert_eq!(
        json_object(&invalid_ident).get("error"),
        Some(&JsonValue::String("invalidParams".to_owned()))
    );

    let malformed = do_account_currencies(
        &request(
            object([("account", JsonValue::String("llIIOO".to_owned()))]),
            2,
        ),
        &source,
    );
    assert_eq!(
        json_object(&malformed).get("error"),
        Some(&JsonValue::String("actMalformed".to_owned()))
    );

    let not_found = do_account_currencies(
        &request(
            object([("account", JsonValue::String(to_base58(alice)))]),
            2,
        ),
        &source,
    );
    assert_eq!(
        json_object(&not_found).get("error"),
        Some(&JsonValue::String("actNotFound".to_owned()))
    );
}

#[test]
fn account_currencies_invalid_account_types() {
    let source = FakeSource {
        ledger: Some(closed_ledger()),
        ..Default::default()
    };

    for param in [
        JsonValue::Bool(true),
        JsonValue::Null,
        JsonValue::Array(vec![]),
        JsonValue::Object(Default::default()),
        JsonValue::Signed(42),
    ] {
        let result = do_account_currencies(&request(object([("account", param)]), 2), &source);
        let obj = json_object(&result);
        assert_eq!(
            obj.get("error"),
            Some(&JsonValue::String("invalidParams".to_owned())),
        );
    }
}

#[test]
fn account_currencies_empty_for_account_with_no_lines() {
    let account = sample_account(0x44);
    let mut source = FakeSource {
        ledger: Some(closed_ledger()),
        ..Default::default()
    };
    source
        .account_roots
        .insert(account, make_account_root(account));
    source
        .owner_pages
        .insert((account, 0), make_owner_page(account, &[]));

    let result = do_account_currencies(
        &request(
            object([("account", JsonValue::String(to_base58(account)))]),
            2,
        ),
        &source,
    );
    let obj = json_object(&result);
    assert_eq!(obj.get("error"), None);
    assert_eq!(
        obj.get("receive_currencies"),
        Some(&JsonValue::Array(vec![]))
    );
    assert_eq!(obj.get("send_currencies"), Some(&JsonValue::Array(vec![])));
    assert_eq!(
        obj.get("ledger_hash"),
        Some(&JsonValue::String(closed_ledger().hash.to_string()))
    );
    assert_eq!(
        obj.get("ledger_index"),
        Some(&JsonValue::Unsigned(u64::from(closed_ledger().seq)))
    );
    assert_eq!(obj.get("validated"), Some(&JsonValue::Bool(true)));
}

#[test]
fn account_currencies_multiple_currencies_sorted() {
    let account = sample_account(0x55);
    let peer = sample_account(0x66);

    // Create lines with different currencies - they should be sorted in output
    let line_usd = make_trust_line(account, peer, "USD", 10, false, 100, 100);
    let line_eur = make_trust_line(account, peer, "EUR", 20, false, 100, 100);
    let line_gbp = make_trust_line(account, peer, "GBP", 30, false, 100, 100);

    let mut source = FakeSource {
        ledger: Some(closed_ledger()),
        ..Default::default()
    };
    source
        .account_roots
        .insert(account, make_account_root(account));
    source.owner_pages.insert(
        (account, 0),
        make_owner_page(
            account,
            &[*line_usd.key(), *line_eur.key(), *line_gbp.key()],
        ),
    );
    source.children.insert(*line_usd.key(), line_usd);
    source.children.insert(*line_eur.key(), line_eur);
    source.children.insert(*line_gbp.key(), line_gbp);

    let result = do_account_currencies(
        &request(
            object([("account", JsonValue::String(to_base58(account)))]),
            2,
        ),
        &source,
    );
    let obj = json_object(&result);
    // All lines have positive balance and positive limits on both sides,
    // so all currencies should appear in both send and receive
    let JsonValue::Array(send) = obj.get("send_currencies").expect("send_currencies") else {
        panic!("send_currencies must be an array");
    };
    let JsonValue::Array(recv) = obj.get("receive_currencies").expect("receive_currencies") else {
        panic!("receive_currencies must be an array");
    };
    assert_eq!(send.len(), 3);
    assert_eq!(recv.len(), 3);
    // Should be sorted alphabetically
    assert_eq!(send[0], JsonValue::String("EUR".to_owned()));
    assert_eq!(send[1], JsonValue::String("GBP".to_owned()));
    assert_eq!(send[2], JsonValue::String("USD".to_owned()));
}
