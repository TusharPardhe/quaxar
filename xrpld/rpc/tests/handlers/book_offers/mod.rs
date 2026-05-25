//! Tests for the book offers RPC handler.

//! Tests for the book offers RPC handler.

use std::{cell::RefCell, collections::BTreeMap, time::Duration};

use basics::base_uint::Uint256;
use protocol::{
    AccountID, Book, Issue, JsonValue, currency_from_string, to_base58, xrp_account, xrp_currency,
};
use rpc::Role;
use rpc::{BookOffersRequest, BookOffersRuntime, BookOffersSource, do_book_offers};
use rpc::{LedgerLookupLedger, LedgerLookupSource};

#[derive(Debug, Default)]
pub(super) struct FakeSource {
    ledger: Option<LedgerLookupLedger>,
    client_jobs: u32,
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

impl BookOffersSource for FakeSource {
    fn client_job_count_gt(&self, threshold: u32) -> bool {
        self.client_jobs > threshold
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CapturedCall {
    ledger: LedgerLookupLedger,
    book: Book,
    taker: AccountID,
    proof: bool,
    limit: u32,
    marker: JsonValue,
}

#[derive(Debug, Default)]
pub(super) struct FakeRuntime {
    call: RefCell<Option<CapturedCall>>,
}

impl BookOffersRuntime for FakeRuntime {
    fn get_book_page(
        &self,
        ledger: &LedgerLookupLedger,
        book: Book,
        taker: AccountID,
        proof: bool,
        limit: u32,
        marker: JsonValue,
        result: &mut JsonValue,
    ) {
        self.call.replace(Some(CapturedCall {
            ledger: *ledger,
            book,
            taker,
            proof,
            limit,
            marker: marker.clone(),
        }));

        let JsonValue::Object(object) = result else {
            panic!("result should be an object");
        };

        object.insert(
            "offers".to_owned(),
            JsonValue::Array(vec![JsonValue::Object(BTreeMap::from([(
                "shape".to_owned(),
                JsonValue::String("delegated".to_owned()),
            )]))]),
        );
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

pub(super) fn sample_account(fill: u8) -> AccountID {
    AccountID::from_array([fill; 20])
}

pub(super) fn sample_hash(fill: u8) -> Uint256 {
    Uint256::from_array([fill; 32])
}

fn closed_ledger() -> LedgerLookupLedger {
    LedgerLookupLedger {
        hash: sample_hash(0xAA),
        seq: 101,
        open: false,
    }
}

pub(super) fn result_object(result: JsonValue) -> BTreeMap<String, JsonValue> {
    let JsonValue::Object(object) = result else {
        panic!("result must be an object");
    };
    object
}

pub(super) fn run(params: JsonValue, source: &FakeSource, runtime: &FakeRuntime) -> JsonValue {
    let request = BookOffersRequest {
        params: &params,
        api_version: 2,
        role: Role::Admin,
    };
    do_book_offers(&request, source, runtime)
}

mod error_handling;
mod page_shaping;
