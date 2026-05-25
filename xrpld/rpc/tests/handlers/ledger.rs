//! Tests for the ledger RPC handler.

use std::{
    cell::{Cell, RefCell},
    collections::BTreeMap,
    time::Duration,
};

use basics::base_uint::Uint256;
use ledger::LedgerFillOptions;
use protocol::JsonValue;
use rpc::{
    LedgerLookupLedger, LedgerLookupSource, LedgerSource, RpcErrorCode, RpcRole, RpcStatus,
    WARN_RPC_FIELDS_DEPRECATED, do_ledger,
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
    fee_track_loaded_local: bool,
    selected_result: JsonValue,
    closed_result: JsonValue,
    open_result: JsonValue,
    selected_calls: RefCell<Vec<(LedgerLookupLedger, u32)>>,
    closed_calls: Cell<u32>,
    open_calls: Cell<u32>,
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

impl LedgerSource for TestSource {
    fn fee_track_loaded_local(&self) -> bool {
        self.fee_track_loaded_local
    }

    fn render_selected_ledger(
        &self,
        ledger: LedgerLookupLedger,
        options: LedgerFillOptions,
    ) -> Result<JsonValue, RpcStatus> {
        self.selected_calls
            .borrow_mut()
            .push((ledger, options.bits()));
        Ok(self.selected_result.clone())
    }

    fn render_closed_ledger(&self) -> Result<JsonValue, RpcStatus> {
        self.closed_calls.set(self.closed_calls.get() + 1);
        Ok(self.closed_result.clone())
    }

    fn render_open_ledger(&self) -> Result<JsonValue, RpcStatus> {
        self.open_calls.set(self.open_calls.get() + 1);
        Ok(self.open_result.clone())
    }
}

fn source() -> TestSource {
    let current = LedgerLookupLedger {
        hash: Uint256::from_array([0x11; 32]),
        seq: 101,
        open: true,
    };
    let closed = LedgerLookupLedger {
        hash: Uint256::from_array([0x22; 32]),
        seq: 100,
        open: false,
    };
    let validated = LedgerLookupLedger {
        hash: Uint256::from_array([0x33; 32]),
        seq: 99,
        open: false,
    };

    TestSource {
        current: Some(current),
        closed: Some(closed),
        validated: Some(validated),
        by_seq: BTreeMap::from([(100, closed), (99, validated)]),
        by_hash: BTreeMap::from([(closed.hash, closed), (validated.hash, validated)]),
        valid_ledger_index: 101,
        validated_age: Duration::from_secs(30),
        standalone: false,
        fee_track_loaded_local: false,
        selected_result: object([
            (
                "ledger",
                JsonValue::Object(BTreeMap::from([(
                    "closed".to_owned(),
                    JsonValue::Bool(false),
                )])),
            ),
            (
                "queue_data",
                JsonValue::Array(vec![JsonValue::Object(BTreeMap::from([(
                    "tx".to_owned(),
                    JsonValue::String("queued".to_owned()),
                )]))]),
            ),
        ]),
        closed_result: JsonValue::Object(BTreeMap::from([(
            "closed".to_owned(),
            JsonValue::Bool(true),
        )])),
        open_result: JsonValue::Object(BTreeMap::from([(
            "closed".to_owned(),
            JsonValue::Bool(false),
        )])),
        selected_calls: RefCell::new(Vec::new()),
        closed_calls: Cell::new(0),
        open_calls: Cell::new(0),
    }
}

#[test]
fn ledger_without_explicit_selector_returns_closed_and_open() {
    let source = source();
    let result = do_ledger(&object([]), RpcRole::Guest, 2, &source);

    let JsonValue::Object(result) = result else {
        panic!("result must be an object");
    };
    assert_eq!(
        result.get("closed"),
        Some(&JsonValue::Object(BTreeMap::from([(
            "closed".to_owned(),
            JsonValue::Bool(true),
        )])))
    );
    assert_eq!(
        result.get("open"),
        Some(&JsonValue::Object(BTreeMap::from([(
            "closed".to_owned(),
            JsonValue::Bool(false),
        )])))
    );
    assert_eq!(source.closed_calls.get(), 1);
    assert_eq!(source.open_calls.get(), 1);
    assert!(source.selected_calls.borrow().is_empty());
}

#[test]
fn ledger_selected_open_merges_lookup_rendering_and_warning() {
    let source = source();
    let params = object([
        ("ledger_index", JsonValue::String("current".to_owned())),
        ("queue", JsonValue::Bool(true)),
        ("transactions", JsonValue::Bool(true)),
        ("expand", JsonValue::Bool(true)),
        ("type", JsonValue::String("hashes".to_owned())),
    ]);

    let result = do_ledger(&params, RpcRole::Admin, 2, &source);
    let JsonValue::Object(result) = result else {
        panic!("result must be an object");
    };

    assert_eq!(
        result.get("ledger_current_index"),
        Some(&JsonValue::Unsigned(101))
    );
    assert_eq!(result.get("validated"), Some(&JsonValue::Bool(false)));
    assert!(result.contains_key("ledger"));
    assert!(result.contains_key("queue_data"));

    let warnings = result.get("warnings").expect("warnings should be present");
    let JsonValue::Array(warnings) = warnings else {
        panic!("warnings must be an array");
    };
    let JsonValue::Object(warning) = &warnings[0] else {
        panic!("warning entry must be an object");
    };
    assert_eq!(
        warning.get("id"),
        Some(&JsonValue::Signed(WARN_RPC_FIELDS_DEPRECATED))
    );

    let calls = source.selected_calls.borrow();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].0.seq, 101);
    assert_eq!(
        calls[0].1,
        (LedgerFillOptions::DUMP_TXRP | LedgerFillOptions::EXPAND | LedgerFillOptions::DUMP_QUEUE)
            .bits()
    );
}

#[test]
fn ledger_queue_requires_open_ledger() {
    let source = source();
    let params = object([
        ("ledger_index", JsonValue::String("validated".to_owned())),
        ("queue", JsonValue::Bool(true)),
    ]);

    let result = do_ledger(&params, RpcRole::Admin, 2, &source);
    let JsonValue::Object(result) = result else {
        panic!("result must be an object");
    };
    assert_eq!(
        result.get("error"),
        Some(&JsonValue::String(
            RpcErrorCode::InvalidParams.token().to_owned()
        ))
    );
}

#[test]
fn ledger_accounts_and_full_require_unlimited_role() {
    let source = source();
    let params = object([
        ("ledger_index", JsonValue::String("validated".to_owned())),
        ("accounts", JsonValue::Bool(true)),
        ("full", JsonValue::Bool(true)),
    ]);

    let result = do_ledger(&params, RpcRole::Guest, 2, &source);
    let JsonValue::Object(result) = result else {
        panic!("result must be an object");
    };
    assert_eq!(
        result.get("error"),
        Some(&JsonValue::String(
            RpcErrorCode::NoPermission.token().to_owned()
        ))
    );
    assert!(source.selected_calls.borrow().is_empty());
}

#[test]
fn ledger_selected_validated_returns_validated_flag() {
    let source = source();
    let params = object([("ledger_index", JsonValue::String("validated".to_owned()))]);

    let result = do_ledger(&params, RpcRole::Admin, 2, &source);
    let JsonValue::Object(result) = result else {
        panic!("result must be an object");
    };
    assert_eq!(result.get("validated"), Some(&JsonValue::Bool(true)));
    assert_eq!(
        result.get("ledger_hash"),
        Some(&JsonValue::String(
            Uint256::from_array([0x33; 32]).to_string()
        ))
    );
    assert_eq!(result.get("ledger_index"), Some(&JsonValue::Unsigned(99)));
    assert!(result.contains_key("ledger"));
}

#[test]
fn ledger_selected_by_seq() {
    let source = source();
    let params = object([("ledger_index", JsonValue::Unsigned(100))]);

    let result = do_ledger(&params, RpcRole::Admin, 2, &source);
    let JsonValue::Object(result) = result else {
        panic!("result must be an object");
    };
    assert_eq!(result.get("error"), None);
    assert_eq!(
        result.get("ledger_hash"),
        Some(&JsonValue::String(
            Uint256::from_array([0x22; 32]).to_string()
        ))
    );
    assert_eq!(result.get("ledger_index"), Some(&JsonValue::Unsigned(100)));
}

#[test]
fn ledger_selected_by_hash() {
    let source = source();
    let hash = Uint256::from_array([0x22; 32]);
    let params = object([("ledger_hash", JsonValue::String(hash.to_string()))]);

    let result = do_ledger(&params, RpcRole::Admin, 2, &source);
    let JsonValue::Object(result) = result else {
        panic!("result must be an object");
    };
    assert_eq!(result.get("error"), None);
    assert_eq!(
        result.get("ledger_hash"),
        Some(&JsonValue::String(hash.to_string()))
    );
    assert_eq!(result.get("ledger_index"), Some(&JsonValue::Unsigned(100)));
}

#[test]
fn ledger_not_found_by_seq() {
    let source = source();
    let params = object([("ledger_index", JsonValue::Unsigned(999))]);

    let result = do_ledger(&params, RpcRole::Admin, 2, &source);
    let JsonValue::Object(result) = result else {
        panic!("result must be an object");
    };
    assert_eq!(
        result.get("error"),
        Some(&JsonValue::String(
            RpcErrorCode::LedgerNotFound.token().to_owned()
        ))
    );
}

#[test]
fn ledger_not_found_by_hash() {
    let source = source();
    let params = object([(
        "ledger_hash",
        JsonValue::String(Uint256::from_array([0xFF; 32]).to_string()),
    )]);

    let result = do_ledger(&params, RpcRole::Admin, 2, &source);
    let JsonValue::Object(result) = result else {
        panic!("result must be an object");
    };
    assert_eq!(
        result.get("error"),
        Some(&JsonValue::String(
            RpcErrorCode::LedgerNotFound.token().to_owned()
        ))
    );
}

#[test]
fn ledger_expand_and_transactions_flags_passed_to_renderer() {
    let source = source();
    let params = object([
        ("ledger_index", JsonValue::Unsigned(100)),
        ("transactions", JsonValue::Bool(true)),
        ("expand", JsonValue::Bool(true)),
    ]);

    let result = do_ledger(&params, RpcRole::Admin, 2, &source);
    let JsonValue::Object(result) = result else {
        panic!("result must be an object");
    };
    assert_eq!(result.get("error"), None);

    let calls = source.selected_calls.borrow();
    assert_eq!(calls.len(), 1);
    let options = LedgerFillOptions::new(calls[0].1);
    assert!(options.contains(LedgerFillOptions::DUMP_TXRP));
    assert!(options.contains(LedgerFillOptions::EXPAND));
}

#[test]
fn ledger_closed_selector() {
    let source = source();
    let params = object([("ledger_index", JsonValue::String("closed".to_owned()))]);

    let result = do_ledger(&params, RpcRole::Admin, 2, &source);
    let JsonValue::Object(result) = result else {
        panic!("result must be an object");
    };
    assert_eq!(result.get("error"), None);
    assert_eq!(result.get("ledger_index"), Some(&JsonValue::Unsigned(100)));
}

#[test]
fn ledger_invalid_ledger_index_type_returns_error() {
    let source = source();
    // Double/float-like large number should be rejected
    let params = object([("ledger_index", JsonValue::String("invalid".to_owned()))]);

    let result = do_ledger(&params, RpcRole::Admin, 2, &source);
    let JsonValue::Object(result) = result else {
        panic!("result must be an object");
    };
    assert!(result.contains_key("error"));
}

#[test]
fn ledger_full_flag_requires_admin_or_unlimited() {
    let source = source();
    let params = object([
        ("ledger_index", JsonValue::String("validated".to_owned())),
        ("full", JsonValue::Bool(true)),
    ]);

    // Guest should be denied
    let result = do_ledger(&params, RpcRole::Guest, 2, &source);
    let JsonValue::Object(result) = result else {
        panic!("result must be an object");
    };
    assert_eq!(
        result.get("error"),
        Some(&JsonValue::String(
            RpcErrorCode::NoPermission.token().to_owned()
        ))
    );

    // Admin should succeed
    let result = do_ledger(&params, RpcRole::Admin, 2, &source);
    let JsonValue::Object(result) = result else {
        panic!("result must be an object");
    };
    assert_eq!(result.get("error"), None);
    assert!(result.contains_key("ledger"));
}

#[test]
fn ledger_accounts_flag_requires_admin_or_unlimited() {
    let source = source();
    let params = object([
        ("ledger_index", JsonValue::String("validated".to_owned())),
        ("accounts", JsonValue::Bool(true)),
    ]);

    let result = do_ledger(&params, RpcRole::Guest, 2, &source);
    let JsonValue::Object(result) = result else {
        panic!("result must be an object");
    };
    assert_eq!(
        result.get("error"),
        Some(&JsonValue::String(
            RpcErrorCode::NoPermission.token().to_owned()
        ))
    );
}

#[test]
fn ledger_queue_flag_only_valid_for_open_ledger() {
    let source = source();

    // Queue on validated should fail
    let params = object([
        ("ledger_index", JsonValue::String("validated".to_owned())),
        ("queue", JsonValue::Bool(true)),
    ]);
    let result = do_ledger(&params, RpcRole::Admin, 2, &source);
    let JsonValue::Object(result) = result else {
        panic!("result must be an object");
    };
    assert_eq!(
        result.get("error"),
        Some(&JsonValue::String(
            RpcErrorCode::InvalidParams.token().to_owned()
        ))
    );

    // Queue on current should succeed
    let params = object([
        ("ledger_index", JsonValue::String("current".to_owned())),
        ("queue", JsonValue::Bool(true)),
    ]);
    let result = do_ledger(&params, RpcRole::Admin, 2, &source);
    let JsonValue::Object(result) = result else {
        panic!("result must be an object");
    };
    assert_eq!(result.get("error"), None);
    assert!(result.contains_key("queue_data"));
}

#[test]
fn ledger_validated_includes_ledger_hash_and_index() {
    let source = source();
    let params = object([("ledger_index", JsonValue::String("validated".to_owned()))]);

    let result = do_ledger(&params, RpcRole::Admin, 2, &source);
    let JsonValue::Object(result) = result else {
        panic!("result must be an object");
    };
    assert!(result.contains_key("ledger_hash"));
    assert!(result.contains_key("ledger_index"));
    assert!(result.contains_key("ledger"));
    assert_eq!(result.get("validated"), Some(&JsonValue::Bool(true)));
}

#[test]
fn ledger_current_includes_ledger_current_index() {
    let source = source();
    let params = object([("ledger_index", JsonValue::String("current".to_owned()))]);

    let result = do_ledger(&params, RpcRole::Admin, 2, &source);
    let JsonValue::Object(result) = result else {
        panic!("result must be an object");
    };
    assert_eq!(
        result.get("ledger_current_index"),
        Some(&JsonValue::Unsigned(101))
    );
    assert_eq!(result.get("validated"), Some(&JsonValue::Bool(false)));
}
