//! Tests for the account channels RPC handler.

use std::{collections::BTreeMap, time::Duration};

use basics::base_uint::{Uint160, Uint256};
use protocol::{
    AccountID, JsonValue, LedgerEntryType, STAmount, STLedgerEntry, STVector256, TokenType,
    account_keylet, encode_base58_token, get_field_by_symbol, owner_dir_keylet, page_keylet,
    to_base58,
};
use rpc::Role;
use rpc::{AccountChannelsRequest, AccountChannelsSource, do_account_channels};
use rpc::{LedgerLookupLedger, LedgerLookupSource};

#[derive(Debug, Default)]
struct FakeSource {
    ledger: Option<LedgerLookupLedger>,
    account_roots: BTreeMap<AccountID, STLedgerEntry>,
    owner_pages: BTreeMap<(AccountID, u64), STLedgerEntry>,
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

impl AccountChannelsSource for FakeSource {
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

    fn read_ledger_entry(
        &self,
        _ledger: &LedgerLookupLedger,
        entry_index: Uint256,
    ) -> Option<STLedgerEntry> {
        self.entries.get(&entry_index).cloned()
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

fn closed_ledger() -> LedgerLookupLedger {
    LedgerLookupLedger {
        hash: sample_hash(0xBC),
        seq: 202,
        open: false,
    }
}

fn make_account_root(account: AccountID) -> STLedgerEntry {
    let account_key = Uint160::from_slice(account.data()).expect("account width");
    let mut sle = STLedgerEntry::from_type_and_key(
        LedgerEntryType::AccountRoot,
        account_keylet(account_key).key,
    );
    sle.set_account_id(get_field_by_symbol("sfAccount"), account);
    sle.set_field_u32(get_field_by_symbol("sfSequence"), 7);
    sle
}

fn make_owner_page(
    account: AccountID,
    page_index: u64,
    entries: &[Uint256],
    next: u64,
) -> STLedgerEntry {
    let root = owner_dir_keylet(Uint160::from_slice(account.data()).expect("account width"));
    let keylet = if page_index == 0 {
        root
    } else {
        page_keylet(root, page_index)
    };
    let mut page = STLedgerEntry::new(keylet);
    let mut indexes = STVector256::with_field(get_field_by_symbol("sfIndexes"));
    for entry in entries {
        indexes.push_back(*entry);
    }
    page.set_field_v256(get_field_by_symbol("sfIndexes"), indexes);
    if next != 0 {
        page.set_field_u64(get_field_by_symbol("sfIndexNext"), next);
    }
    page
}

fn encode_account_public_base58(public_key: &[u8]) -> String {
    encode_base58_token(TokenType::AccountPublic, public_key)
}

fn make_channel(
    key: Uint256,
    account: AccountID,
    destination: AccountID,
    owner_node: u64,
    public_key: [u8; 33],
) -> STLedgerEntry {
    let mut sle = STLedgerEntry::from_type_and_key(LedgerEntryType::PayChannel, key);
    sle.set_account_id(get_field_by_symbol("sfAccount"), account);
    sle.set_account_id(get_field_by_symbol("sfDestination"), destination);
    sle.set_field_amount(
        get_field_by_symbol("sfAmount"),
        STAmount::new_native(1_000, false),
    );
    sle.set_field_amount(
        get_field_by_symbol("sfBalance"),
        STAmount::new_native(250, false),
    );
    sle.set_field_vl(get_field_by_symbol("sfPublicKey"), &public_key);
    sle.set_field_u64(get_field_by_symbol("sfOwnerNode"), owner_node);
    sle.set_field_u32(get_field_by_symbol("sfSettleDelay"), 60);
    sle.set_field_u32(get_field_by_symbol("sfExpiration"), 11);
    sle.set_field_u32(get_field_by_symbol("sfCancelAfter"), 22);
    sle.set_field_u32(get_field_by_symbol("sfSourceTag"), 33);
    sle.set_field_u32(get_field_by_symbol("sfDestinationTag"), 44);
    sle
}

#[test]
fn account_channels_report_missing_invalid_and_malformed() {
    let source = FakeSource {
        ledger: Some(closed_ledger()),
        ..Default::default()
    };

    let missing = do_account_channels(
        &AccountChannelsRequest {
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

    let invalid = do_account_channels(
        &AccountChannelsRequest {
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

    let malformed = do_account_channels(
        &AccountChannelsRequest {
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
fn account_channels_emit_public_key_base58_hex_and_marker() {
    let account = sample_account(0x11);
    let destination = sample_account(0x22);
    let other_destination = sample_account(0x33);
    let public_key = [
        0x02, 0x7a, 0x11, 0x45, 0x33, 0x21, 0x10, 0x88, 0x89, 0x9a, 0xab, 0xbc, 0xcd, 0xde, 0xef,
        0x01, 0x12, 0x23, 0x34, 0x45, 0x56, 0x67, 0x78, 0x89, 0x9a, 0xab, 0xbc, 0xcd, 0xde, 0xef,
        0xf1, 0x12, 0x24,
    ];
    let channel1 = make_channel(sample_hash(0x01), account, destination, 7, public_key);
    let channel2 = make_channel(sample_hash(0x02), account, other_destination, 8, public_key);
    let channel3 = make_channel(sample_hash(0x03), account, destination, 9, public_key);
    let page0 = make_owner_page(account, 0, &[*channel1.key(), *channel2.key()], 1);
    let page1 = make_owner_page(account, 1, &[*channel3.key()], 0);
    let source = FakeSource {
        ledger: Some(closed_ledger()),
        account_roots: BTreeMap::from([(account, make_account_root(account))]),
        owner_pages: BTreeMap::from([((account, 0), page0), ((account, 1), page1)]),
        entries: BTreeMap::from([
            (*channel1.key(), channel1.clone()),
            (*channel2.key(), channel2.clone()),
            (*channel3.key(), channel3.clone()),
        ]),
    };

    let limited = do_account_channels(
        &AccountChannelsRequest {
            params: &object([
                ("account", JsonValue::String(to_base58(account))),
                ("limit", JsonValue::Unsigned(2)),
            ]),
            api_version: 1,
            role: Role::Admin,
        },
        &source,
    );
    let JsonValue::Object(limited) = limited else {
        panic!("limited response must be an object");
    };
    assert_eq!(limited.get("limit"), Some(&JsonValue::Unsigned(2)));
    assert_eq!(
        limited.get("marker"),
        Some(&JsonValue::String(format!("{},{}", channel2.key(), 8)))
    );
    let JsonValue::Array(channels) = limited.get("channels").expect("channels array") else {
        panic!("channels must be an array");
    };
    assert_eq!(channels.len(), 2);
    let JsonValue::Object(first) = &channels[0] else {
        panic!("channel must be an object");
    };
    assert_eq!(
        first.get("public_key"),
        Some(&JsonValue::String(encode_account_public_base58(
            &public_key
        )))
    );
    assert_eq!(
        first.get("public_key_hex"),
        Some(&JsonValue::String(basics::str_hex::str_hex(public_key)))
    );

    let filtered = do_account_channels(
        &AccountChannelsRequest {
            params: &object([
                ("account", JsonValue::String(to_base58(account))),
                (
                    "destination_account",
                    JsonValue::String(to_base58(destination)),
                ),
            ]),
            api_version: 1,
            role: Role::Admin,
        },
        &source,
    );
    let JsonValue::Object(filtered) = filtered else {
        panic!("filtered response must be an object");
    };
    let JsonValue::Array(filtered_channels) = filtered.get("channels").expect("channels array")
    else {
        panic!("channels must be an array");
    };
    assert_eq!(filtered_channels.len(), 2);
}

#[test]
fn account_channels_reject_marker_for_unrelated_object() {
    let account = sample_account(0x44);
    let destination = sample_account(0x45);
    let outsider = sample_account(0x46);
    let public_key = [0xED; 33];
    let related = make_channel(sample_hash(0x11), account, destination, 4, public_key);
    let unrelated = make_channel(sample_hash(0x22), outsider, destination, 5, public_key);
    let page0 = make_owner_page(account, 0, &[*related.key()], 0);

    let result = do_account_channels(
        &AccountChannelsRequest {
            params: &object([
                ("account", JsonValue::String(to_base58(account))),
                (
                    "marker",
                    JsonValue::String(format!("{},{}", unrelated.key(), 5)),
                ),
            ]),
            api_version: 1,
            role: Role::Admin,
        },
        &FakeSource {
            ledger: Some(closed_ledger()),
            account_roots: BTreeMap::from([(account, make_account_root(account))]),
            owner_pages: BTreeMap::from([((account, 0), page0)]),
            entries: BTreeMap::from([(*related.key(), related), (*unrelated.key(), unrelated)]),
        },
    );
    let JsonValue::Object(result) = result else {
        panic!("result must be an object");
    };
    assert_eq!(
        result.get("error"),
        Some(&JsonValue::String("invalidParams".to_owned()))
    );
}
