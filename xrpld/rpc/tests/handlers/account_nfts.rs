//! Tests for the account nfts RPC handler.

use std::{
    collections::{BTreeMap, HashMap},
    time::Duration,
};

use basics::base_uint::{Uint160, Uint256};
use protocol::{
    AccountID, JsonValue, LedgerEntryType, STArray, STLedgerEntry, STObject, account_keylet,
    get_field_by_symbol, nft_page_keylet, nft_page_min_keylet, to_base58,
};
use rpc::RpcRole;
use rpc::{AccountNFTsRequest, AccountNFTsSource, do_account_nfts};
use rpc::{LedgerLookupLedger, LedgerLookupSource};

#[derive(Debug, Default)]
struct FakeSource {
    ledger: Option<LedgerLookupLedger>,
    account_roots: HashMap<AccountID, STLedgerEntry>,
    nft_pages: BTreeMap<Uint256, STLedgerEntry>,
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

impl AccountNFTsSource for FakeSource {
    fn read_account_root(
        &self,
        _ledger: &LedgerLookupLedger,
        account_id: AccountID,
    ) -> Option<STLedgerEntry> {
        self.account_roots.get(&account_id).cloned()
    }

    fn read_nft_page(
        &self,
        _ledger: &LedgerLookupLedger,
        page_key: Uint256,
    ) -> Option<STLedgerEntry> {
        self.nft_pages.get(&page_key).cloned()
    }

    fn succ_nft_page(
        &self,
        _ledger: &LedgerLookupLedger,
        start: Uint256,
        last_exclusive: Uint256,
    ) -> Option<Uint256> {
        self.nft_pages
            .range((std::ops::Bound::Excluded(start), std::ops::Bound::Unbounded))
            .map(|(key, _)| *key)
            .find(|key| *key < last_exclusive)
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
        seq: 91,
        open: false,
    }
}

fn make_nft_id(flags: u16, fee: u16, issuer: AccountID, taxon: u32, serial: u32) -> Uint256 {
    let cipher = taxon ^ ((384_160_001u32.wrapping_mul(serial)).wrapping_add(2_459));
    let mut bytes = [0u8; 32];
    bytes[..2].copy_from_slice(&flags.to_be_bytes());
    bytes[2..4].copy_from_slice(&fee.to_be_bytes());
    bytes[4..24].copy_from_slice(issuer.data());
    bytes[24..28].copy_from_slice(&cipher.to_be_bytes());
    bytes[28..32].copy_from_slice(&serial.to_be_bytes());
    Uint256::from_array(bytes)
}

fn make_nft_entry(nft_id: Uint256) -> STObject {
    let mut nft = STObject::make_inner_object(get_field_by_symbol("sfNFToken"));
    nft.set_field_h256(get_field_by_symbol("sfNFTokenID"), nft_id);
    nft
}

fn make_nft_page(account: AccountID, token_ids: &[Uint256]) -> STLedgerEntry {
    let owner = Uint160::from_slice(account.data()).expect("account width");
    let mut page = STLedgerEntry::from_type_and_key(
        LedgerEntryType::NFTokenPage,
        nft_page_keylet(nft_page_min_keylet(owner), token_ids[0]).key,
    );
    let mut tokens = STArray::new(get_field_by_symbol("sfNFTokens"));
    for token_id in token_ids {
        tokens.push_back(make_nft_entry(*token_id));
    }
    page.set_field_array(get_field_by_symbol("sfNFTokens"), tokens);
    page
}

#[test]
fn account_nfts_reports_account_errors() {
    let mut source = FakeSource {
        ledger: Some(closed_ledger()),
        ..Default::default()
    };
    let limit_account = sample_account(0x66);
    source.account_roots.insert(
        limit_account,
        STLedgerEntry::from_type_and_key(
            LedgerEntryType::AccountRoot,
            account_keylet(Uint160::from_slice(limit_account.data()).expect("account width")).key,
        ),
    );

    let missing = do_account_nfts(
        &AccountNFTsRequest {
            params: &JsonValue::Object(Default::default()),
            api_version: 1,
            role: RpcRole::Admin,
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

    let invalid = do_account_nfts(
        &AccountNFTsRequest {
            params: &object([("account", JsonValue::Unsigned(1))]),
            api_version: 1,
            role: RpcRole::Admin,
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

    let malformed = do_account_nfts(
        &AccountNFTsRequest {
            params: &object([("account", JsonValue::String("foo".to_owned()))]),
            api_version: 1,
            role: RpcRole::Admin,
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
    assert_eq!(malformed.get("error_code"), Some(&JsonValue::Signed(35)));

    let bad_limit = do_account_nfts(
        &AccountNFTsRequest {
            params: &object([
                ("account", JsonValue::String(to_base58(limit_account))),
                ("limit", JsonValue::String("ten".to_owned())),
            ]),
            api_version: 1,
            role: RpcRole::Admin,
        },
        &source,
    );
    let JsonValue::Object(bad_limit) = bad_limit else {
        panic!("bad limit response must be an object");
    };
    assert_eq!(
        bad_limit.get("error_message"),
        Some(&JsonValue::String(
            "Invalid field 'limit', not unsigned integer.".to_owned()
        ))
    );
}

#[test]
fn account_nfts_shapes_pages_limits_and_markers() {
    let account = sample_account(0x11);
    let issuer = sample_account(0x22);
    let first = make_nft_id(0x0010, 0x0000, issuer, 0x01020304, 1);
    let second = make_nft_id(0x0011, 0x0032, issuer, 0x11121314, 2);
    let third = make_nft_id(0x0012, 0x0000, issuer, 0x21222324, 3);

    let mut source = FakeSource {
        ledger: Some(closed_ledger()),
        ..Default::default()
    };
    source.account_roots.insert(
        account,
        STLedgerEntry::from_type_and_key(
            LedgerEntryType::AccountRoot,
            account_keylet(Uint160::from_slice(account.data()).expect("account width")).key,
        ),
    );
    source.nft_pages.insert(
        nft_page_keylet(
            nft_page_min_keylet(Uint160::from_slice(account.data()).expect("account width")),
            first,
        )
        .key,
        make_nft_page(account, &[first, second]),
    );
    source.nft_pages.insert(
        nft_page_keylet(
            nft_page_min_keylet(Uint160::from_slice(account.data()).expect("account width")),
            third,
        )
        .key,
        make_nft_page(account, &[third]),
    );

    let params = object([
        ("account", JsonValue::String(to_base58(account))),
        ("limit", JsonValue::Unsigned(2)),
    ]);
    let result = do_account_nfts(
        &AccountNFTsRequest {
            params: &params,
            api_version: 1,
            role: RpcRole::Admin,
        },
        &source,
    );
    let JsonValue::Object(result) = result else {
        panic!("result must be an object");
    };
    let JsonValue::Array(nfts) = result.get("account_nfts").expect("account_nfts") else {
        panic!("account_nfts must be an array");
    };
    assert_eq!(nfts.len(), 2);
    assert_eq!(
        result.get("account"),
        Some(&JsonValue::String(to_base58(account)))
    );
    assert_eq!(result.get("limit"), Some(&JsonValue::Unsigned(2)));
    assert_eq!(
        result.get("marker"),
        Some(&JsonValue::String(second.to_string()))
    );

    let JsonValue::Object(first_nft) = &nfts[0] else {
        panic!("first nft must be an object");
    };
    assert_eq!(
        first_nft.get("NFTokenID"),
        Some(&JsonValue::String(first.to_string()))
    );
    assert_eq!(first_nft.get("Flags"), Some(&JsonValue::Unsigned(0x0010)));
    assert_eq!(
        first_nft.get("Issuer"),
        Some(&JsonValue::String(to_base58(issuer)))
    );
    assert_eq!(
        first_nft.get("NFTokenTaxon"),
        Some(&JsonValue::Unsigned(0x01020304))
    );
    assert_eq!(first_nft.get("nft_serial"), Some(&JsonValue::Unsigned(1)));
    assert!(!first_nft.contains_key("TransferFee"));

    let JsonValue::Object(second_nft) = &nfts[1] else {
        panic!("second nft must be an object");
    };
    assert_eq!(
        second_nft.get("NFTokenID"),
        Some(&JsonValue::String(second.to_string()))
    );
    assert_eq!(second_nft.get("Flags"), Some(&JsonValue::Unsigned(0x0011)));
    assert_eq!(
        second_nft.get("TransferFee"),
        Some(&JsonValue::Unsigned(0x0032))
    );
}

#[test]
fn account_nfts_reports_invalid_markers_and_missing_accounts() {
    let account = sample_account(0x33);
    let issuer = sample_account(0x44);
    let nft_id = make_nft_id(0x0001, 0x0000, issuer, 0x55555555, 4);

    let mut source = FakeSource {
        ledger: Some(closed_ledger()),
        ..Default::default()
    };
    source.account_roots.insert(
        account,
        STLedgerEntry::from_type_and_key(
            LedgerEntryType::AccountRoot,
            account_keylet(Uint160::from_slice(account.data()).expect("account width")).key,
        ),
    );
    source.nft_pages.insert(
        nft_page_keylet(
            nft_page_min_keylet(Uint160::from_slice(account.data()).expect("account width")),
            nft_id,
        )
        .key,
        make_nft_page(account, &[nft_id]),
    );

    let invalid_marker_type = do_account_nfts(
        &AccountNFTsRequest {
            params: &object([
                ("account", JsonValue::String(to_base58(account))),
                ("marker", JsonValue::Unsigned(1)),
            ]),
            api_version: 1,
            role: RpcRole::User,
        },
        &source,
    );
    let JsonValue::Object(invalid_marker_type) = invalid_marker_type else {
        panic!("invalid marker response must be an object");
    };
    assert_eq!(
        invalid_marker_type.get("error_message"),
        Some(&JsonValue::String(
            "Invalid field 'marker', not string.".to_owned()
        ))
    );

    let invalid_marker_value = do_account_nfts(
        &AccountNFTsRequest {
            params: &object([
                ("account", JsonValue::String(to_base58(account))),
                (
                    "marker",
                    JsonValue::String(
                        "00000000000000000000000000000000000000000000000000000000000000ZZ"
                            .to_owned(),
                    ),
                ),
            ]),
            api_version: 1,
            role: RpcRole::User,
        },
        &source,
    );
    let JsonValue::Object(invalid_marker_value) = invalid_marker_value else {
        panic!("invalid marker value response must be an object");
    };
    assert_eq!(
        invalid_marker_value.get("error_message"),
        Some(&JsonValue::String("Invalid field 'marker'.".to_owned()))
    );

    let missing_account = do_account_nfts(
        &AccountNFTsRequest {
            params: &object([(
                "account",
                JsonValue::String(to_base58(sample_account(0x55))),
            )]),
            api_version: 1,
            role: RpcRole::User,
        },
        &source,
    );
    let JsonValue::Object(missing_account) = missing_account else {
        panic!("missing account response must be an object");
    };
    assert_eq!(
        missing_account.get("error"),
        Some(&JsonValue::String("actNotFound".to_owned()))
    );
}

#[test]
fn account_nfts_response_structure_fields() {
    let account = sample_account(0x40);
    let issuer = sample_account(0x41);
    let nft_id = make_nft_id(0x0008, 0x0064, issuer, 0xDEADBEEF, 42);
    let page_key = nft_page_keylet(
        nft_page_min_keylet(Uint160::from_slice(account.data()).expect("account width")),
        Uint256::zero(),
    )
    .key
    .next();

    let account_key = Uint160::from_slice(account.data()).expect("account width");
    let mut account_root = STLedgerEntry::from_type_and_key(
        LedgerEntryType::AccountRoot,
        account_keylet(account_key).key,
    );
    account_root.set_account_id(get_field_by_symbol("sfAccount"), account);

    let mut source = FakeSource {
        ledger: Some(closed_ledger()),
        ..Default::default()
    };
    source.account_roots.insert(account, account_root);
    source
        .nft_pages
        .insert(page_key, make_nft_page(account, &[nft_id]));

    let result = do_account_nfts(
        &AccountNFTsRequest {
            params: &object([("account", JsonValue::String(to_base58(account)))]),
            api_version: 2,
            role: RpcRole::Admin,
        },
        &source,
    );
    let JsonValue::Object(result) = result else {
        panic!("result must be an object");
    };
    assert_eq!(result.get("error"), None);
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

    let JsonValue::Array(nfts) = result.get("account_nfts").expect("account_nfts") else {
        panic!("account_nfts must be an array");
    };
    assert_eq!(nfts.len(), 1);

    let JsonValue::Object(token) = &nfts[0] else {
        panic!("token must be an object");
    };
    assert_eq!(
        token.get("NFTokenID"),
        Some(&JsonValue::String(nft_id.to_string()))
    );
    assert_eq!(
        token.get("NFTokenTaxon"),
        Some(&JsonValue::Unsigned(0xDEADBEEF))
    );
    assert_eq!(token.get("nft_serial"), Some(&JsonValue::Unsigned(42)));
    assert_eq!(token.get("Flags"), Some(&JsonValue::Unsigned(0x0008)));
    assert_eq!(
        token.get("Issuer"),
        Some(&JsonValue::String(to_base58(issuer)))
    );
    assert_eq!(token.get("TransferFee"), Some(&JsonValue::Unsigned(0x0064)));
}

#[test]
fn account_nfts_empty_for_account_with_no_pages() {
    let account = sample_account(0x50);
    let account_key = Uint160::from_slice(account.data()).expect("account width");
    let mut account_root = STLedgerEntry::from_type_and_key(
        LedgerEntryType::AccountRoot,
        account_keylet(account_key).key,
    );
    account_root.set_account_id(get_field_by_symbol("sfAccount"), account);

    let mut source = FakeSource {
        ledger: Some(closed_ledger()),
        ..Default::default()
    };
    source.account_roots.insert(account, account_root);

    let result = do_account_nfts(
        &AccountNFTsRequest {
            params: &object([("account", JsonValue::String(to_base58(account)))]),
            api_version: 2,
            role: RpcRole::Admin,
        },
        &source,
    );
    let JsonValue::Object(result) = result else {
        panic!("result must be an object");
    };
    assert_eq!(result.get("error"), None);
    let JsonValue::Array(nfts) = result.get("account_nfts").expect("account_nfts") else {
        panic!("account_nfts must be an array");
    };
    assert_eq!(nfts.len(), 0);
    assert!(!result.contains_key("marker"));
}

#[test]
fn account_nfts_account_not_found() {
    let account = sample_account(0x70);
    let source = FakeSource {
        ledger: Some(closed_ledger()),
        ..Default::default()
    };

    let result = do_account_nfts(
        &AccountNFTsRequest {
            params: &object([("account", JsonValue::String(to_base58(account)))]),
            api_version: 2,
            role: RpcRole::Admin,
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
    assert_eq!(result.get("error_code"), Some(&JsonValue::Signed(19)));
}
