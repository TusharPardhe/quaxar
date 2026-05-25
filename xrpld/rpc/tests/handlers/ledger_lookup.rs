//! Tests for the ledger lookup RPC handler.

use std::{collections::BTreeMap, time::Duration};

use basics::base_uint::Uint256;
use protocol::JsonValue;
use rpc::{
    LedgerLookupContext, LedgerLookupLedger, LedgerLookupSource, RpcErrorCode, RpcRole, RpcStatus,
    lookup_ledger, lookup_ledger_json,
};

fn object(entries: impl IntoIterator<Item = (&'static str, JsonValue)>) -> JsonValue {
    JsonValue::Object(
        entries
            .into_iter()
            .map(|(key, value)| (key.to_owned(), value))
            .collect::<BTreeMap<_, _>>(),
    )
}

#[derive(Clone)]
struct TestSource {
    current: Option<LedgerLookupLedger>,
    closed: Option<LedgerLookupLedger>,
    validated: Option<LedgerLookupLedger>,
    by_seq: BTreeMap<u32, LedgerLookupLedger>,
    by_hash: BTreeMap<Uint256, LedgerLookupLedger>,
    valid_ledger_index: u32,
    validated_age: Duration,
    standalone: bool,
}

impl LedgerLookupSource for TestSource {
    fn get_ledger_by_hash(&self, hash: Uint256) -> Option<LedgerLookupLedger> {
        self.by_hash.get(&hash).copied()
    }

    fn get_ledger_by_seq(&self, seq: u32) -> Option<LedgerLookupLedger> {
        self.by_seq.get(&seq).copied()
    }

    fn get_current_ledger(&self) -> Option<LedgerLookupLedger> {
        self.current
    }

    fn get_closed_ledger(&self) -> Option<LedgerLookupLedger> {
        self.closed
    }

    fn get_validated_ledger(&self) -> Option<LedgerLookupLedger> {
        self.validated
    }

    fn get_valid_ledger_index(&self) -> u32 {
        self.valid_ledger_index
    }

    fn get_validated_ledger_age(&self) -> Duration {
        self.validated_age
    }

    fn is_validated(&self, ledger: &LedgerLookupLedger) -> bool {
        self.validated == Some(*ledger)
    }

    fn standalone(&self) -> bool {
        self.standalone
    }
}

fn source() -> TestSource {
    let validated = LedgerLookupLedger {
        hash: Uint256::from_array([0x11; 32]),
        seq: 90,
        open: false,
    };
    let current = LedgerLookupLedger {
        hash: Uint256::from_array([0x22; 32]),
        seq: 100,
        open: true,
    };
    let closed = LedgerLookupLedger {
        hash: Uint256::from_array([0x33; 32]),
        seq: 99,
        open: false,
    };
    let mut by_seq = BTreeMap::new();
    by_seq.insert(validated.seq, validated);
    by_seq.insert(closed.seq, closed);
    let mut by_hash = BTreeMap::new();
    by_hash.insert(validated.hash, validated);
    by_hash.insert(closed.hash, closed);

    TestSource {
        current: Some(current),
        closed: Some(closed),
        validated: Some(validated),
        by_seq,
        by_hash,
        valid_ledger_index: 100,
        validated_age: Duration::from_secs(30),
        standalone: false,
    }
}

#[test]
fn lookup_ledger_defaults_to_current_ledger_shape() {
    let source = source();
    let params = object([]);
    let result = lookup_ledger(&LedgerLookupContext {
        params: &params,
        source: &source,
        api_version: 2,
        role: RpcRole::Guest,
    })
    .expect("lookup should succeed");

    let JsonValue::Object(result) = result else {
        panic!("result should be an object");
    };
    assert_eq!(
        result.get("ledger_current_index"),
        Some(&JsonValue::Unsigned(100))
    );
    assert_eq!(result.get("validated"), Some(&JsonValue::Bool(false)));
}

#[test]
fn lookup_ledger_rejects_conflicting_fields() {
    let source = source();
    let params = object([
        ("ledger", JsonValue::String("closed".into())),
        (
            "ledger_hash",
            JsonValue::String(
                "1111111111111111111111111111111111111111111111111111111111111111".into(),
            ),
        ),
    ]);
    let status = lookup_ledger(&LedgerLookupContext {
        params: &params,
        source: &source,
        api_version: 2,
        role: RpcRole::Guest,
    })
    .expect_err("lookup should fail");

    assert_eq!(
        status,
        RpcStatus::with_message(
            RpcErrorCode::InvalidParams,
            "Exactly one of 'ledger', 'ledger_hash', or 'ledger_index' can be specified."
        )
    );
}

#[test]
fn lookup_ledger_json_injects_not_synced_error_for_newer_api() {
    let mut source = source();
    source.validated_age = Duration::from_secs(121);
    let params = object([("ledger_index", JsonValue::String("validated".into()))]);
    let result = lookup_ledger_json(&LedgerLookupContext {
        params: &params,
        source: &source,
        api_version: 2,
        role: RpcRole::Guest,
    });

    let JsonValue::Object(result) = result else {
        panic!("result should be an object");
    };
    assert_eq!(
        result.get("error"),
        Some(&JsonValue::String("notSynced".into()))
    );
    assert_eq!(result.get("error_code"), Some(&JsonValue::Signed(18)));
}
