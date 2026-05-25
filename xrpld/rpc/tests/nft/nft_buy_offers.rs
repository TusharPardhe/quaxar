//! Tests for nft buy offers.

use std::{collections::BTreeMap, time::Duration};

use basics::base_uint::Uint256;
use protocol::{
    AccountID, JsonValue, LedgerEntryType, STAmount, STLedgerEntry, STVector256,
    get_field_by_symbol, nft_buy_offers_keylet, nft_offer_keylet, page_keylet,
};
use rpc::{
    LedgerLookupLedger, LedgerLookupSource, NFTOffersRequest, NFTOffersSource, RpcRole,
    do_nft_buy_offers,
};

#[derive(Debug, Default)]
struct FakeSource {
    ledger: Option<LedgerLookupLedger>,
    pages: BTreeMap<Uint256, STLedgerEntry>,
    offers: BTreeMap<Uint256, STLedgerEntry>,
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

impl NFTOffersSource for FakeSource {
    fn read_directory_page(
        &self,
        _ledger: &LedgerLookupLedger,
        page_key: Uint256,
    ) -> Option<STLedgerEntry> {
        self.pages.get(&page_key).cloned()
    }

    fn read_nft_offer(
        &self,
        _ledger: &LedgerLookupLedger,
        offer_key: Uint256,
    ) -> Option<STLedgerEntry> {
        self.offers.get(&offer_key).cloned()
    }
}

fn closed_ledger() -> LedgerLookupLedger {
    LedgerLookupLedger {
        hash: Uint256::from_array([0xAB; 32]),
        seq: 707,
        open: false,
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

fn request(params: JsonValue) -> NFTOffersRequest<'static> {
    let params = Box::leak(Box::new(params));
    NFTOffersRequest {
        params,
        api_version: 2,
        role: RpcRole::User,
    }
}

fn json_object(value: &JsonValue) -> &BTreeMap<String, JsonValue> {
    let JsonValue::Object(object) = value else {
        panic!("expected object");
    };
    object
}

fn make_page(root: protocol::Keylet, index: u64, entries: &[Uint256], next: u64) -> STLedgerEntry {
    let mut page = STLedgerEntry::new(page_keylet(root, index));
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

fn sample_account(fill: u8) -> AccountID {
    AccountID::from_array([fill; 20])
}

fn offer_key(sequence: u32) -> Uint256 {
    Uint256::from_hex(&format!("{sequence:064X}")).expect("offer key")
}

fn make_offer(
    key: Uint256,
    nft_id: Uint256,
    owner: AccountID,
    amount: u64,
    node: u64,
    with_destination: bool,
) -> STLedgerEntry {
    let mut offer = STLedgerEntry::from_type_and_key(LedgerEntryType::NFTokenOffer, key);
    offer.set_field_h256(get_field_by_symbol("sfNFTokenID"), nft_id);
    offer.set_field_u32(get_field_by_symbol("sfFlags"), 0);
    offer.set_account_id(get_field_by_symbol("sfOwner"), owner);
    offer.set_field_amount(
        get_field_by_symbol("sfAmount"),
        STAmount::new_native(amount, false),
    );
    offer.set_field_u64(get_field_by_symbol("sfNFTokenOfferNode"), node);
    if with_destination {
        offer.set_account_id(get_field_by_symbol("sfDestination"), sample_account(0x55));
        offer.set_field_u32(get_field_by_symbol("sfExpiration"), 1234);
    }
    offer
}

fn build_source_with_count(nft_id: Uint256, count: u32) -> FakeSource {
    let root = nft_buy_offers_keylet(nft_id);
    let buyer = sample_account(0x11);
    let mut source = FakeSource {
        ledger: Some(closed_ledger()),
        ..Default::default()
    };
    let mut pages: BTreeMap<u64, Vec<Uint256>> = BTreeMap::new();

    for sequence in 1..=count {
        let key = offer_key(sequence);
        let page_index = u64::from((sequence - 1) / 250);
        pages.entry(page_index).or_default().push(key);
        source.offers.insert(
            nft_offer_keylet(key).key,
            make_offer(
                key,
                nft_id,
                buyer,
                u64::from(sequence),
                page_index,
                sequence % 2 == 0,
            ),
        );
    }

    for (page_index, entries) in pages {
        let next = if page_index * 250 + 250 < u64::from(count) {
            page_index + 1
        } else {
            0
        };
        let page = make_page(root, page_index, &entries, next);
        source.pages.insert(*page.key(), page);
    }

    source
}

#[test]
fn nft_buy_offers_reports_cpp_style_errors() {
    let source = FakeSource {
        ledger: Some(closed_ledger()),
        ..Default::default()
    };

    let missing = do_nft_buy_offers(&request(object([])), &source);
    assert_eq!(
        json_object(&missing).get("error"),
        Some(&JsonValue::String("invalidParams".to_owned()))
    );

    let malformed = do_nft_buy_offers(
        &request(object([("nft_id", JsonValue::String("nope".to_owned()))])),
        &source,
    );
    assert_eq!(
        json_object(&malformed).get("error"),
        Some(&JsonValue::String("invalidParams".to_owned()))
    );

    let not_found = do_nft_buy_offers(
        &request(object([("nft_id", JsonValue::String("11".repeat(32)))])),
        &source,
    );
    assert_eq!(
        json_object(&not_found).get("error"),
        Some(&JsonValue::String("objectNotFound".to_owned()))
    );
}

#[test]
fn nft_buy_offers_paginates_and_includes_marker_offer() {
    let nft_id = Uint256::from_array([0x44; 32]);
    let root = nft_buy_offers_keylet(nft_id);
    let mut source = FakeSource {
        ledger: Some(closed_ledger()),
        ..Default::default()
    };
    let mut first_page = Vec::new();
    let mut second_page = Vec::new();
    let marker_offer = offer_key(51);

    for sequence in 1..=51u32 {
        let key = offer_key(sequence);
        let page_index = if sequence <= 50 { 0 } else { 1 };
        if page_index == 0 {
            first_page.push(key);
        } else {
            second_page.push(key);
        }
        source.offers.insert(
            nft_offer_keylet(key).key,
            make_offer(
                key,
                nft_id,
                sample_account(0x11 + (sequence as u8 % 3)),
                u64::from(sequence * 10),
                page_index,
                sequence == 1,
            ),
        );
        if sequence == 1
            && let Some(offer) = source.offers.get_mut(&nft_offer_keylet(key).key)
        {
            offer.set_account_id(get_field_by_symbol("sfDestination"), sample_account(0x33));
            offer.set_field_u32(get_field_by_symbol("sfExpiration"), 42);
        }
    }
    source
        .pages
        .insert(root.key, make_page(root, 0, &first_page, 1));
    source.pages.insert(
        page_keylet(root, 1).key,
        make_page(root, 1, &second_page, 0),
    );

    let first_page = do_nft_buy_offers(
        &request(object([
            ("nft_id", JsonValue::String(nft_id.to_string())),
            ("limit", JsonValue::Unsigned(50)),
        ])),
        &source,
    );
    let first_object = json_object(&first_page);
    let JsonValue::Array(first_offers) = first_object.get("offers").expect("offers") else {
        panic!("offers should be array");
    };
    assert_eq!(first_offers.len(), 50);
    assert_eq!(
        first_object.get("marker"),
        Some(&JsonValue::String(marker_offer.to_string()))
    );

    let resumed = do_nft_buy_offers(
        &request(object([
            ("nft_id", JsonValue::String(nft_id.to_string())),
            ("limit", JsonValue::Unsigned(50)),
            ("marker", JsonValue::String(marker_offer.to_string())),
        ])),
        &source,
    );
    let resumed_object = json_object(&resumed);
    let JsonValue::Array(resumed_offers) = resumed_object.get("offers").expect("offers") else {
        panic!("offers should be array");
    };
    assert_eq!(resumed_offers.len(), 1);
    let JsonValue::Object(first_offer) = &resumed_offers[0] else {
        panic!("offer should be object");
    };
    assert_eq!(
        first_offer.get("nft_offer_index"),
        Some(&JsonValue::String(marker_offer.to_string()))
    );
}

#[test]
fn nft_buy_offers_match_cpp_count_matrix() {
    for (count, expected_markers) in [(0u32, 0u32), (1, 0), (250, 0), (251, 1), (500, 1), (501, 2)]
    {
        let nft_id = Uint256::from_hex(&format!("{:064X}", 0xAAu32 + count)).expect("nft id");
        let source = build_source_with_count(nft_id, count);

        if count == 0 {
            let response = do_nft_buy_offers(
                &request(object([("nft_id", JsonValue::String(nft_id.to_string()))])),
                &source,
            );
            assert_eq!(
                json_object(&response).get("error"),
                Some(&JsonValue::String("objectNotFound".to_owned()))
            );
            continue;
        }

        let mut marker: Option<String> = None;
        let mut marker_count = 0u32;
        let mut returned = 0usize;

        loop {
            let mut params = vec![("nft_id", JsonValue::String(nft_id.to_string()))];
            if let Some(current_marker) = marker.clone() {
                params.push(("marker", JsonValue::String(current_marker)));
            }

            let response = do_nft_buy_offers(&request(object(params)), &source);
            let object = json_object(&response);
            let JsonValue::Array(offers) = object.get("offers").expect("offers") else {
                panic!("offers should be array");
            };
            returned += offers.len();

            marker = match object.get("marker") {
                Some(JsonValue::String(value)) => {
                    marker_count += 1;
                    assert_eq!(object.get("limit"), Some(&JsonValue::Unsigned(250)));
                    Some(value.clone())
                }
                _ => None,
            };

            if marker.is_none() {
                break;
            }
        }

        assert_eq!(returned, count as usize);
        assert_eq!(marker_count, expected_markers);
    }
}
