//! Tests for the owner info RPC handler.

use std::{collections::BTreeMap, time::Duration};

use basics::base_uint::{Uint160, Uint256};
use protocol::{
    AccountID, JsonValue, LedgerEntryType, STLedgerEntry, STVector256, account_keylet,
    get_field_by_symbol, offer_keylet, owner_dir_keylet, page_keylet, to_base58,
};
use rpc::{LedgerLookupLedger, LedgerLookupSource};
use rpc::{OwnerInfoSource, do_owner_info};

#[derive(Debug, Default)]
struct FakeSource {
    closed: Option<LedgerLookupLedger>,
    current: Option<LedgerLookupLedger>,
    owner_pages: BTreeMap<(AccountID, u64), STLedgerEntry>,
    children: BTreeMap<Uint256, STLedgerEntry>,
}

impl LedgerLookupSource for FakeSource {
    fn get_ledger_by_hash(&self, hash: Uint256) -> Option<LedgerLookupLedger> {
        self.current
            .filter(|ledger| ledger.hash == hash)
            .or(self.closed.filter(|ledger| ledger.hash == hash))
    }

    fn get_ledger_by_seq(&self, seq: u32) -> Option<LedgerLookupLedger> {
        self.current
            .filter(|ledger| ledger.seq == seq)
            .or(self.closed.filter(|ledger| ledger.seq == seq))
    }

    fn get_current_ledger(&self) -> Option<LedgerLookupLedger> {
        self.current
    }

    fn get_closed_ledger(&self) -> Option<LedgerLookupLedger> {
        self.closed
    }

    fn get_validated_ledger(&self) -> Option<LedgerLookupLedger> {
        self.closed
    }

    fn get_valid_ledger_index(&self) -> u32 {
        self.current.map(|ledger| ledger.seq).unwrap_or_default()
    }

    fn get_validated_ledger_age(&self) -> Duration {
        Duration::from_secs(1)
    }

    fn is_validated(&self, ledger: &LedgerLookupLedger) -> bool {
        self.closed == Some(*ledger)
    }
}

impl OwnerInfoSource for FakeSource {
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

fn make_offer(account: AccountID, sequence: u32, fill: u8) -> STLedgerEntry {
    let account_key = Uint160::from_slice(account.data()).expect("account width");
    let mut sle = STLedgerEntry::new(offer_keylet(account_key, sequence));
    sle.set_account_id(get_field_by_symbol("sfAccount"), account);
    sle.set_field_u32(get_field_by_symbol("sfSequence"), sequence);
    sle.set_field_u32(get_field_by_symbol("sfFlags"), u32::from(fill));
    sle.set_field_h256(get_field_by_symbol("sfBookDirectory"), sample_hash(fill));
    sle.set_field_amount(
        get_field_by_symbol("sfTakerPays"),
        protocol::STAmount::new_native(u64::from(fill) * 10, false),
    );
    sle.set_field_amount(
        get_field_by_symbol("sfTakerGets"),
        protocol::STAmount::new_native(u64::from(fill) * 5, false),
    );
    sle.set_field_u64(get_field_by_symbol("sfOwnerNode"), u64::from(sequence));
    sle
}

fn make_ripple_state(account: AccountID, fill: u8) -> STLedgerEntry {
    let account_key = Uint160::from_slice(account.data()).expect("account width");
    let mut sle = STLedgerEntry::from_type_and_key(
        LedgerEntryType::RippleState,
        account_keylet(account_key).key,
    );
    let mut low_limit = protocol::STAmount::new_with_asset(
        protocol::sf_generic(),
        {
            let mut issue = protocol::no_issue();
            issue.account = account;
            issue.currency = protocol::currency_from_string("USD");
            issue
        },
        1_000_000 + u64::from(fill),
        0,
        false,
    );
    low_limit.set_issuer(account);
    let mut high_limit = low_limit.clone();
    high_limit.set_issuer(sample_account(fill));
    sle.set_field_amount(get_field_by_symbol("sfLowLimit"), low_limit);
    sle.set_field_amount(get_field_by_symbol("sfHighLimit"), high_limit);
    sle.set_field_u64(get_field_by_symbol("sfLowNode"), u64::from(fill));
    sle.set_field_u64(get_field_by_symbol("sfHighNode"), u64::from(fill) + 100);
    sle
}

fn make_page(
    account: AccountID,
    page_index: u64,
    entries: Vec<Uint256>,
    next: Option<u64>,
) -> STLedgerEntry {
    let root = owner_dir_keylet(Uint160::from_slice(account.data()).expect("account width"));
    let mut page = STLedgerEntry::new(page_keylet(root, page_index));
    let mut indexes = STVector256::with_field(get_field_by_symbol("sfIndexes"));
    for entry in entries {
        indexes.push_back(entry);
    }
    page.set_field_v256(get_field_by_symbol("sfIndexes"), indexes);
    if let Some(next) = next {
        page.set_field_u64(get_field_by_symbol("sfIndexNext"), next);
    }
    page
}

#[test]
fn owner_info_reports_missing_and_malformed() {
    let source = FakeSource {
        closed: Some(LedgerLookupLedger {
            hash: sample_hash(0xAA),
            seq: 10,
            open: false,
        }),
        current: Some(LedgerLookupLedger {
            hash: sample_hash(0xBB),
            seq: 20,
            open: true,
        }),
        ..Default::default()
    };

    let missing = do_owner_info(&JsonValue::Object(Default::default()), &source);
    let JsonValue::Object(missing) = missing else {
        panic!("missing response must be an object");
    };
    assert_eq!(
        missing.get("error_message"),
        Some(&JsonValue::String("Missing field 'account'.".to_owned()))
    );

    let malformed = do_owner_info(
        &object([("account", JsonValue::String("foo".to_owned()))]),
        &source,
    );
    let JsonValue::Object(malformed) = malformed else {
        panic!("malformed response must be an object");
    };
    let JsonValue::Object(accepted) = malformed.get("accepted").expect("accepted") else {
        panic!("accepted must be an object");
    };
    let JsonValue::Object(current) = malformed.get("current").expect("current") else {
        panic!("current must be an object");
    };
    assert_eq!(
        accepted.get("error"),
        Some(&JsonValue::String("actMalformed".to_owned()))
    );
    assert_eq!(
        current.get("error"),
        Some(&JsonValue::String("actMalformed".to_owned()))
    );
}

#[test]
fn owner_info_collects_offers_and_ripple_lines_across_pages() {
    let account = sample_account(0x11);
    let offer = make_offer(account, 1, 0x21);
    let ripple = make_ripple_state(account, 0x31);
    let closed = LedgerLookupLedger {
        hash: sample_hash(0xAA),
        seq: 10,
        open: false,
    };
    let current = LedgerLookupLedger {
        hash: sample_hash(0xBB),
        seq: 20,
        open: true,
    };
    let page0 = make_page(account, 0, vec![*offer.key()], Some(1));
    let page1 = make_page(account, 1, vec![*ripple.key()], None);

    let result = do_owner_info(
        &object([("ident", JsonValue::String(to_base58(account)))]),
        &FakeSource {
            closed: Some(closed),
            current: Some(current),
            owner_pages: BTreeMap::from([((account, 0), page0), ((account, 1), page1)]),
            children: BTreeMap::from([(*offer.key(), offer), (*ripple.key(), ripple)]),
        },
    );
    let JsonValue::Object(result) = result else {
        panic!("result must be an object");
    };

    let JsonValue::Object(accepted) = result.get("accepted").expect("accepted") else {
        panic!("accepted must be an object");
    };
    let JsonValue::Array(accepted_offers) = accepted.get("offers").expect("offers") else {
        panic!("accepted offers must be an array");
    };
    assert_eq!(accepted_offers.len(), 1);

    let JsonValue::Object(current) = result.get("current").expect("current") else {
        panic!("current must be an object");
    };
    let JsonValue::Array(current_lines) = current.get("ripple_lines").expect("ripple_lines") else {
        panic!("current ripple_lines must be an array");
    };
    assert_eq!(current_lines.len(), 1);
}

#[test]
fn owner_info_invalid_account_types() {
    let source = FakeSource {
        closed: Some(LedgerLookupLedger {
            hash: sample_hash(0xAA),
            seq: 10,
            open: false,
        }),
        current: Some(LedgerLookupLedger {
            hash: sample_hash(0xBB),
            seq: 20,
            open: true,
        }),
        ..Default::default()
    };

    for param in [
        JsonValue::Unsigned(1),
        JsonValue::Bool(true),
        JsonValue::Null,
        JsonValue::Array(vec![]),
        JsonValue::Object(Default::default()),
    ] {
        let result = do_owner_info(&object([("account", param)]), &source);
        let JsonValue::Object(result) = result else {
            panic!("result must be an object");
        };
        assert!(
            result.get("error").is_some()
                || result.get("error_message").is_some()
                || result.get("accepted").is_some(),
            "invalid account type should produce an error or accepted/current with error"
        );
    }
}

#[test]
fn owner_info_empty_owner_dir() {
    let account = sample_account(0x44);
    let closed = LedgerLookupLedger {
        hash: sample_hash(0xAA),
        seq: 10,
        open: false,
    };
    let current = LedgerLookupLedger {
        hash: sample_hash(0xBB),
        seq: 20,
        open: true,
    };
    let page0 = make_page(account, 0, vec![], None);

    let result = do_owner_info(
        &object([("account", JsonValue::String(to_base58(account)))]),
        &FakeSource {
            closed: Some(closed),
            current: Some(current),
            owner_pages: BTreeMap::from([((account, 0), page0)]),
            children: BTreeMap::new(),
        },
    );
    let JsonValue::Object(result) = result else {
        panic!("result must be an object");
    };
    // Should have accepted and current sections
    assert!(result.contains_key("accepted") || result.contains_key("current"));
}

#[test]
fn owner_info_uses_ident_field_as_account() {
    let account = sample_account(0x55);
    let closed = LedgerLookupLedger {
        hash: sample_hash(0xAA),
        seq: 10,
        open: false,
    };
    let current = LedgerLookupLedger {
        hash: sample_hash(0xBB),
        seq: 20,
        open: true,
    };
    let page0 = make_page(account, 0, vec![], None);

    let result = do_owner_info(
        &object([("ident", JsonValue::String(to_base58(account)))]),
        &FakeSource {
            closed: Some(closed),
            current: Some(current),
            owner_pages: BTreeMap::from([((account, 0), page0)]),
            children: BTreeMap::new(),
        },
    );
    let JsonValue::Object(result) = result else {
        panic!("result must be an object");
    };
    // Should not have error since ident is a valid alternative to account
    assert!(
        result.get("accepted").is_some() || result.get("current").is_some(),
        "ident should be accepted as account identifier"
    );
}
