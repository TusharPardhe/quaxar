//! Tests for the ledger header RPC handler (rippled `doLedgerHeader`).

use basics::base_uint::Uint256;
use basics::sha_map_hash::SHAMapHash;
use basics::str_hex::str_hex;
use ledger::LedgerFillOptions;
use protocol::{JsonValue, LedgerHeader};
use rpc::{
    LedgerHeaderRequest, LedgerHeaderSource, LedgerLookupLedger, LedgerLookupSource, LedgerSource,
    RpcRole, RpcStatus, do_ledger_header, serialize_ledger_header,
};
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::time::Duration;

fn ledger(seq: u32, fill: u8) -> LedgerLookupLedger {
    LedgerLookupLedger {
        hash: Uint256::from_array([fill; 32]),
        seq,
        open: false,
    }
}

fn header_of(ledger: &LedgerLookupLedger) -> LedgerHeader {
    LedgerHeader {
        seq: ledger.seq,
        drops: 10,
        hash: SHAMapHash::new(ledger.hash),
        parent_hash: SHAMapHash::new(Uint256::from_array([0x22; 32])),
        close_time: 9,
        close_time_resolution: 10,
        ..LedgerHeader::default()
    }
}

struct Source {
    older: LedgerLookupLedger,
    validated: LedgerLookupLedger,
    rendered: RefCell<Vec<(u32, u32, u32)>>,
}

impl LedgerLookupSource for Source {
    fn get_ledger_by_hash(&self, hash: Uint256) -> Option<LedgerLookupLedger> {
        [self.older, self.validated]
            .into_iter()
            .find(|ledger| ledger.hash == hash)
    }
    fn get_ledger_by_seq(&self, seq: u32) -> Option<LedgerLookupLedger> {
        [self.older, self.validated]
            .into_iter()
            .find(|ledger| ledger.seq == seq)
    }
    fn get_current_ledger(&self) -> Option<LedgerLookupLedger> {
        None
    }
    fn get_closed_ledger(&self) -> Option<LedgerLookupLedger> {
        Some(self.validated)
    }
    fn get_validated_ledger(&self) -> Option<LedgerLookupLedger> {
        Some(self.validated)
    }
    fn get_valid_ledger_index(&self) -> u32 {
        self.validated.seq
    }
    fn get_validated_ledger_age(&self) -> Duration {
        Duration::from_secs(1)
    }
    fn is_validated(&self, _ledger: &LedgerLookupLedger) -> bool {
        true
    }
}

impl LedgerSource for Source {
    fn render_selected_ledger(
        &self,
        ledger: LedgerLookupLedger,
        options: LedgerFillOptions,
        api_version: u32,
    ) -> Result<JsonValue, RpcStatus> {
        self.rendered
            .borrow_mut()
            .push((ledger.seq, options.bits(), api_version));
        Ok(JsonValue::Object(BTreeMap::from([
            (
                "ledger_index".to_owned(),
                JsonValue::Unsigned(u64::from(ledger.seq)),
            ),
            (
                "close_time_human".to_owned(),
                JsonValue::String("rendered".to_owned()),
            ),
        ])))
    }
    fn render_closed_ledger(&self) -> Result<JsonValue, RpcStatus> {
        unreachable!("ledger_header renders the selected ledger")
    }
    fn render_open_ledger(&self) -> Result<JsonValue, RpcStatus> {
        unreachable!("ledger_header renders the selected ledger")
    }
}

impl LedgerHeaderSource for Source {
    fn ledger_header(&self, ledger: &LedgerLookupLedger) -> Option<LedgerHeader> {
        Some(header_of(ledger))
    }
}

fn source() -> Source {
    Source {
        older: ledger(3, 0x11),
        validated: ledger(5, 0x55),
        rendered: RefCell::new(Vec::new()),
    }
}

fn call(source: &Source, params: JsonValue, api_version: u32) -> BTreeMap<String, JsonValue> {
    let JsonValue::Object(object) = do_ledger_header(
        &LedgerHeaderRequest {
            params: &params,
            api_version,
            role: RpcRole::Admin,
        },
        source,
    ) else {
        panic!("result must be an object");
    };
    object
}

/// rippled doLedgerHeader honors the ledger selector. quaxar answered for its
/// newest ledger whatever was asked (mainnet: asked 108548865, answered
/// 108548889), and hand-built a partial `ledger` without the `close_*`
/// fields.
#[test]
fn ledger_header_answers_for_the_requested_ledger() {
    let source = source();
    let params = JsonValue::Object(BTreeMap::from([(
        "ledger_index".to_owned(),
        JsonValue::Unsigned(3),
    )]));
    let object = call(&source, params, 1);

    assert_eq!(
        object.get("ledger_hash"),
        Some(&JsonValue::String(source.older.hash.to_string()))
    );
    assert_eq!(object.get("ledger_index"), Some(&JsonValue::Unsigned(3)));
    assert_eq!(object.get("validated"), Some(&JsonValue::Bool(true)));
    assert_eq!(
        object.get("ledger_data"),
        Some(&JsonValue::String(str_hex(serialize_ledger_header(
            &header_of(&source.older)
        ))))
    );
    let Some(JsonValue::Object(ledger)) = object.get("ledger") else {
        panic!("ledger object");
    };
    assert_eq!(
        ledger.get("close_time_human"),
        Some(&JsonValue::String("rendered".to_owned())),
        "the ledger comes from the LedgerToJson writer"
    );
    assert_eq!(
        ledger.get("ledger_index"),
        Some(&JsonValue::String("3".to_owned())),
        "API v1 writes the header ledger_index as a string"
    );
    assert_eq!(
        source.rendered.borrow().as_slice(),
        &[(3, 0, 1)],
        "rendered once, for the requested ledger, with no fill options"
    );
}

#[test]
fn ledger_header_v2_writes_ledger_index_as_a_number() {
    let source = source();
    let object = call(
        &source,
        JsonValue::Object(BTreeMap::from([(
            "ledger_index".to_owned(),
            JsonValue::String("validated".to_owned()),
        )])),
        2,
    );
    let Some(JsonValue::Object(ledger)) = object.get("ledger") else {
        panic!("ledger object");
    };
    assert_eq!(ledger.get("ledger_index"), Some(&JsonValue::Unsigned(5)));
}

#[test]
fn ledger_header_reports_a_missing_ledger() {
    let source = source();
    let params = JsonValue::Object(BTreeMap::from([(
        "ledger_index".to_owned(),
        JsonValue::Unsigned(4),
    )]));
    let object = call(&source, params, 1);
    assert_eq!(
        object.get("error"),
        Some(&JsonValue::String("lgrNotFound".to_owned()))
    );
}
