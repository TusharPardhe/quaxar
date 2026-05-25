//! Tests for the account offers RPC handler.

use std::{collections::BTreeMap, time::Duration};

use basics::base_uint::{Uint160, Uint256};
use protocol::{
    AccountID, JsonValue, LedgerEntryType, STLedgerEntry, STVector256, account_keylet,
    get_field_by_symbol, offer_keylet, owner_dir_keylet, page_keylet, to_base58,
};
use rpc::Role;
use rpc::{AccountOffersRequest, AccountOffersSource, do_account_offers};
use rpc::{LedgerLookupLedger, LedgerLookupSource};

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

impl AccountOffersSource for FakeSource {
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

fn sample_account(fill: u8) -> AccountID {
    AccountID::from_array([fill; 20])
}

fn sample_hash(fill: u8) -> Uint256 {
    Uint256::from_array([fill; 32])
}

fn object(params: impl IntoIterator<Item = (&'static str, JsonValue)>) -> JsonValue {
    JsonValue::Object(
        params
            .into_iter()
            .map(|(key, value)| (key.to_owned(), value))
            .collect(),
    )
}

fn make_account_root(account: AccountID) -> STLedgerEntry {
    let account_key = Uint160::from_slice(account.data()).expect("account width");
    let mut sle = STLedgerEntry::from_type_and_key(
        LedgerEntryType::AccountRoot,
        account_keylet(account_key).key,
    );
    sle.set_account_id(get_field_by_symbol("sfAccount"), account);
    sle.set_field_u32(get_field_by_symbol("sfSequence"), 7);
    sle.set_field_u32(get_field_by_symbol("sfOwnerCount"), 2);
    sle
}

fn make_offer(
    account: AccountID,
    sequence: u32,
    book_quality: Uint256,
    taker_pays: protocol::STAmount,
    taker_gets: protocol::STAmount,
    expiration: Option<u32>,
) -> STLedgerEntry {
    let account_key = Uint160::from_slice(account.data()).expect("account width");
    let mut sle = STLedgerEntry::new(offer_keylet(account_key, sequence));
    sle.set_account_id(get_field_by_symbol("sfAccount"), account);
    sle.set_field_u32(get_field_by_symbol("sfSequence"), sequence);
    sle.set_field_u32(get_field_by_symbol("sfFlags"), 0xA5A5_0001);
    sle.set_field_h256(get_field_by_symbol("sfBookDirectory"), book_quality);
    sle.set_field_amount(get_field_by_symbol("sfTakerPays"), taker_pays);
    sle.set_field_amount(get_field_by_symbol("sfTakerGets"), taker_gets);
    sle.set_field_u64(get_field_by_symbol("sfOwnerNode"), u64::from(sequence));
    if let Some(expiration) = expiration {
        sle.set_field_u32(get_field_by_symbol("sfExpiration"), expiration);
    }
    sle
}

fn closed_ledger() -> LedgerLookupLedger {
    LedgerLookupLedger {
        hash: sample_hash(0xAA),
        seq: 101,
        open: false,
    }
}

#[test]
fn account_offers_reports_missing_invalid_and_malformed() {
    let source = FakeSource {
        ledger: Some(closed_ledger()),
        ..Default::default()
    };

    let missing = do_account_offers(
        &AccountOffersRequest {
            params: &JsonValue::Object(Default::default()),
            api_version: 1,
            role: Role::Admin,
        },
        &source,
    );
    let JsonValue::Object(missing) = missing else {
        panic!("missing response must be an object");
    };
    assert_eq!(
        missing.get("error_message"),
        Some(&JsonValue::String("Missing field 'account'.".to_owned()))
    );

    let invalid = do_account_offers(
        &AccountOffersRequest {
            params: &object([("account", JsonValue::Unsigned(1))]),
            api_version: 1,
            role: Role::Admin,
        },
        &source,
    );
    let JsonValue::Object(invalid) = invalid else {
        panic!("invalid response must be an object");
    };
    assert_eq!(
        invalid.get("error"),
        Some(&JsonValue::String("invalidParams".to_owned()))
    );

    let malformed = do_account_offers(
        &AccountOffersRequest {
            params: &object([("account", JsonValue::String("foo".to_owned()))]),
            api_version: 1,
            role: Role::Admin,
        },
        &source,
    );
    let JsonValue::Object(malformed) = malformed else {
        panic!("malformed response must be an object");
    };
    assert_eq!(
        malformed.get("error"),
        Some(&JsonValue::String("actMalformed".to_owned()))
    );
}

#[test]
fn account_offers_respects_marker_limit_and_offer_json() {
    let account = sample_account(0x11);
    let offer1 = make_offer(
        account,
        1,
        sample_hash(0x01),
        protocol::STAmount::new_native(1_000, false),
        protocol::STAmount::new_native(500, false),
        Some(7),
    );
    let offer2 = make_offer(
        account,
        2,
        sample_hash(0x02),
        protocol::STAmount::new_native(2_000, false),
        protocol::STAmount::new_native(1_000, false),
        None,
    );
    let offer3 = make_offer(
        account,
        3,
        sample_hash(0x03),
        protocol::STAmount::new_native(3_000, false),
        protocol::STAmount::new_native(1_500, false),
        None,
    );

    let root = owner_dir_keylet(Uint160::from_slice(account.data()).expect("account width"));
    let mut page0 = STLedgerEntry::new(root);
    let mut indexes = STVector256::with_field(get_field_by_symbol("sfIndexes"));
    indexes.push_back(offer1.key().to_owned());
    indexes.push_back(offer2.key().to_owned());
    page0.set_field_v256(get_field_by_symbol("sfIndexes"), indexes);
    page0.set_field_u64(get_field_by_symbol("sfIndexNext"), 1);

    let mut page1 = STLedgerEntry::new(page_keylet(root, 1));
    let mut indexes1 = STVector256::with_field(get_field_by_symbol("sfIndexes"));
    indexes1.push_back(offer3.key().to_owned());
    page1.set_field_v256(get_field_by_symbol("sfIndexes"), indexes1);

    let result = do_account_offers(
        &AccountOffersRequest {
            params: &object([
                ("account", JsonValue::String(to_base58(account))),
                ("limit", JsonValue::Unsigned(2)),
            ]),
            api_version: 1,
            role: Role::Admin,
        },
        &FakeSource {
            ledger: Some(closed_ledger()),
            account_roots: BTreeMap::from([(account, make_account_root(account))]),
            owner_pages: BTreeMap::from([
                ((account, 0), page0.clone()),
                ((account, 1), page1.clone()),
            ]),
            children: BTreeMap::from([
                (*offer1.key(), offer1.clone()),
                (*offer2.key(), offer2.clone()),
                (*offer3.key(), offer3.clone()),
            ]),
        },
    );
    let JsonValue::Object(result) = result else {
        panic!("result must be an object");
    };

    assert_eq!(
        result.get("account"),
        Some(&JsonValue::String(to_base58(account)))
    );
    assert_eq!(result.get("limit"), Some(&JsonValue::Unsigned(2)));
    assert_eq!(
        result.get("marker"),
        Some(&JsonValue::String(format!(
            "{},{}",
            offer2.key(),
            offer2.get_field_u64(get_field_by_symbol("sfOwnerNode"))
        )))
    );

    let JsonValue::Array(offers) = result.get("offers").expect("offers array") else {
        panic!("offers must be an array");
    };
    assert_eq!(offers.len(), 2);
    let JsonValue::Object(first) = &offers[0] else {
        panic!("first offer must be an object");
    };
    assert_eq!(
        first.get("seq"),
        Some(&JsonValue::Unsigned(u64::from(
            offer1.get_field_u32(get_field_by_symbol("sfSequence"))
        )))
    );
    assert_eq!(first.get("expiration"), Some(&JsonValue::Unsigned(7)));
    assert_eq!(
        first.get("quality"),
        Some(&JsonValue::String("0".to_owned()))
    );

    let marker_result = do_account_offers(
        &AccountOffersRequest {
            params: &object([
                ("account", JsonValue::String(to_base58(account))),
                (
                    "marker",
                    JsonValue::String(format!(
                        "{},{}",
                        offer2.key(),
                        offer2.get_field_u64(get_field_by_symbol("sfOwnerNode"))
                    )),
                ),
            ]),
            api_version: 1,
            role: Role::Admin,
        },
        &FakeSource {
            ledger: Some(closed_ledger()),
            account_roots: BTreeMap::from([(account, make_account_root(account))]),
            owner_pages: BTreeMap::from([((account, 0), page0), ((account, 1), page1)]),
            children: BTreeMap::from([
                (*offer1.key(), offer1),
                (*offer2.key(), offer2),
                (*offer3.key(), offer3),
            ]),
        },
    );
    let JsonValue::Object(marker_result) = marker_result else {
        panic!("marker result must be an object");
    };
    let JsonValue::Array(marker_offers) = marker_result.get("offers").expect("offers array") else {
        panic!("offers must be an array");
    };
    assert_eq!(marker_offers.len(), 1);
}

#[test]
fn account_offers_account_not_found() {
    let account = sample_account(0xAA);
    let source = FakeSource {
        ledger: Some(closed_ledger()),
        ..Default::default()
    };

    let result = do_account_offers(
        &AccountOffersRequest {
            params: &object([("account", JsonValue::String(to_base58(account)))]),
            api_version: 1,
            role: Role::Admin,
        },
        &source,
    );
    let JsonValue::Object(result) = result else {
        panic!("result must be an object");
    };
    assert_eq!(
        result.get("error"),
        Some(&JsonValue::String("actNotFound".to_owned()))
    );
    assert_eq!(
        result.get("error_message"),
        Some(&JsonValue::String("Account not found.".to_owned()))
    );
    assert_eq!(result.get("error_code"), Some(&JsonValue::Signed(19)));
}

#[test]
fn account_offers_empty_for_funded_account_with_no_offers() {
    let account = sample_account(0xBB);
    let root = owner_dir_keylet(Uint160::from_slice(account.data()).expect("account width"));
    let mut page0 = STLedgerEntry::new(root);
    let indexes = STVector256::with_field(get_field_by_symbol("sfIndexes"));
    page0.set_field_v256(get_field_by_symbol("sfIndexes"), indexes);

    let source = FakeSource {
        ledger: Some(closed_ledger()),
        account_roots: BTreeMap::from([(account, make_account_root(account))]),
        owner_pages: BTreeMap::from([((account, 0), page0)]),
        children: BTreeMap::new(),
    };

    let result = do_account_offers(
        &AccountOffersRequest {
            params: &object([("account", JsonValue::String(to_base58(account)))]),
            api_version: 1,
            role: Role::Admin,
        },
        &source,
    );
    let JsonValue::Object(result) = result else {
        panic!("result must be an object");
    };
    assert_eq!(result.get("error"), None);
    let JsonValue::Array(offers) = result.get("offers").expect("offers") else {
        panic!("offers must be an array");
    };
    assert_eq!(offers.len(), 0);
    assert!(!result.contains_key("marker"));
    assert_eq!(
        result.get("account"),
        Some(&JsonValue::String(to_base58(account)))
    );
    assert_eq!(
        result.get("ledger_hash"),
        Some(&JsonValue::String(closed_ledger().hash.to_string()))
    );
    assert_eq!(
        result.get("ledger_index"),
        Some(&JsonValue::Unsigned(u64::from(closed_ledger().seq)))
    );
    assert_eq!(result.get("validated"), Some(&JsonValue::Bool(true)));
}

#[test]
fn account_offers_invalid_account_types() {
    let source = FakeSource {
        ledger: Some(closed_ledger()),
        ..Default::default()
    };

    for param in [
        JsonValue::Bool(true),
        JsonValue::Null,
        JsonValue::Array(vec![]),
        JsonValue::Object(Default::default()),
    ] {
        let result = do_account_offers(
            &AccountOffersRequest {
                params: &object([("account", param)]),
                api_version: 1,
                role: Role::Admin,
            },
            &source,
        );
        let JsonValue::Object(result) = result else {
            panic!("result must be an object");
        };
        assert_eq!(
            result.get("error"),
            Some(&JsonValue::String("invalidParams".to_owned()))
        );
        assert_eq!(
            result.get("error_message"),
            Some(&JsonValue::String("Invalid field 'account'.".to_owned()))
        );
    }
}

#[test]
fn account_offers_negative_limit_rejected() {
    let account = sample_account(0xCC);
    let source = FakeSource {
        ledger: Some(closed_ledger()),
        account_roots: BTreeMap::from([(account, make_account_root(account))]),
        ..Default::default()
    };

    let result = do_account_offers(
        &AccountOffersRequest {
            params: &object([
                ("account", JsonValue::String(to_base58(account))),
                ("limit", JsonValue::Signed(-1)),
            ]),
            api_version: 1,
            role: Role::Admin,
        },
        &source,
    );
    let JsonValue::Object(result) = result else {
        panic!("result must be an object");
    };
    assert!(
        result.get("error").is_some() || result.get("error_message").is_some(),
        "negative limit should produce an error"
    );
}

#[test]
fn account_offers_non_string_marker_rejected() {
    let account = sample_account(0xDD);
    let source = FakeSource {
        ledger: Some(closed_ledger()),
        account_roots: BTreeMap::from([(account, make_account_root(account))]),
        ..Default::default()
    };

    let result = do_account_offers(
        &AccountOffersRequest {
            params: &object([
                ("account", JsonValue::String(to_base58(account))),
                ("marker", JsonValue::Bool(true)),
            ]),
            api_version: 1,
            role: Role::Admin,
        },
        &source,
    );
    let JsonValue::Object(result) = result else {
        panic!("result must be an object");
    };
    assert!(
        result.get("error").is_some() || result.get("error_message").is_some(),
        "non-string marker should produce an error"
    );
}
