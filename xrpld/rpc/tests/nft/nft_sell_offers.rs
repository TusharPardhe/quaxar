//! Tests for nft sell offers.

use std::{collections::BTreeMap, time::Duration};

use basics::base_uint::Uint256;
use protocol::{
    AccountID, JsonValue, LedgerEntryType, STAmount, STLedgerEntry, STVector256, StBase,
    get_field_by_symbol, nft_offer_keylet, nft_sell_offers_keylet, page_keylet,
};
use rpc::{
    LedgerLookupLedger, LedgerLookupSource, NFTOffersRequest, NFTOffersSource, RpcRole,
    do_nft_sell_offers,
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
        hash: Uint256::from_array([0xCD; 32]),
        seq: 808,
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

fn sample_account(fill: u8) -> AccountID {
    AccountID::from_array([fill; 20])
}

fn offer_key(sequence: u32) -> Uint256 {
    Uint256::from_hex(&format!("{sequence:064X}")).expect("offer key")
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

fn make_offer(key: Uint256, nft_id: Uint256, amount: u64) -> STLedgerEntry {
    let mut offer = STLedgerEntry::from_type_and_key(LedgerEntryType::NFTokenOffer, key);
    offer.set_field_h256(get_field_by_symbol("sfNFTokenID"), nft_id);
    offer.set_field_u32(get_field_by_symbol("sfFlags"), 1);
    offer.set_account_id(get_field_by_symbol("sfOwner"), sample_account(0x22));
    offer.set_field_amount(
        get_field_by_symbol("sfAmount"),
        STAmount::new_native(amount, false),
    );
    offer.set_field_u64(get_field_by_symbol("sfNFTokenOfferNode"), 0);
    offer
}

fn build_source_with_count(nft_id: Uint256, count: u32) -> FakeSource {
    let root = nft_sell_offers_keylet(nft_id);
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
            make_offer(key, nft_id, u64::from(sequence)),
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
fn nft_sell_offers_returns_cpp_shape_for_single_page() {
    let nft_id = Uint256::from_array([0x55; 32]);
    let offer = Uint256::from_array([0x09; 32]);
    let root = nft_sell_offers_keylet(nft_id);

    let mut source = FakeSource {
        ledger: Some(closed_ledger()),
        ..Default::default()
    };
    source
        .pages
        .insert(root.key, make_page(root, 0, &[offer], 0));
    source
        .offers
        .insert(nft_offer_keylet(offer).key, make_offer(offer, nft_id, 77));

    let result = do_nft_sell_offers(
        &request(object([("nft_id", JsonValue::String(nft_id.to_string()))])),
        &source,
    );
    let object = json_object(&result);
    let JsonValue::Array(offers) = object.get("offers").expect("offers") else {
        panic!("offers should be array");
    };
    assert_eq!(offers.len(), 1);
    let JsonValue::Object(offer_json) = &offers[0] else {
        panic!("offer should be object");
    };
    assert_eq!(
        offer_json.get("nft_offer_index"),
        Some(&JsonValue::String(offer.to_string()))
    );
    assert_eq!(
        offer_json.get("amount"),
        Some(&STAmount::new_native(77, false).json(protocol::JsonOptions::NONE))
    );
    assert_eq!(object.get("marker"), None);
}

#[test]
fn nft_sell_offers_rejects_marker_for_other_token() {
    let nft_id = Uint256::from_array([0x66; 32]);
    let wrong_nft_id = Uint256::from_array([0x67; 32]);
    let offer = Uint256::from_array([0x0A; 32]);
    let root = nft_sell_offers_keylet(nft_id);

    let mut source = FakeSource {
        ledger: Some(closed_ledger()),
        ..Default::default()
    };
    source
        .pages
        .insert(root.key, make_page(root, 0, &[offer], 0));
    source.offers.insert(
        nft_offer_keylet(offer).key,
        make_offer(offer, wrong_nft_id, 88),
    );

    let result = do_nft_sell_offers(
        &request(object([
            ("nft_id", JsonValue::String(nft_id.to_string())),
            ("marker", JsonValue::String(offer.to_string())),
        ])),
        &source,
    );
    assert_eq!(
        json_object(&result).get("error"),
        Some(&JsonValue::String("invalidParams".to_owned()))
    );
}

#[test]
fn nft_sell_offers_match_cpp_count_matrix() {
    for (count, expected_markers) in [(0u32, 0u32), (1, 0), (250, 0), (251, 1), (500, 1), (501, 2)]
    {
        let nft_id = Uint256::from_hex(&format!("{:064X}", 0xBBu32 + count)).expect("nft id");
        let source = build_source_with_count(nft_id, count);

        if count == 0 {
            let response = do_nft_sell_offers(
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

            let response = do_nft_sell_offers(&request(object(params)), &source);
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
