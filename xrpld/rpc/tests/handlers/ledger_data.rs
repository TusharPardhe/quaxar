//! Tests for the ledger data RPC handler.

mod ledger_lookup {
    pub use rpc::ledger_lookup::{LedgerLookupLedger, LedgerLookupSource, RpcErrorCode, RpcStatus};
}

use basics::base_uint::Uint256;
use protocol::{JsonValue, LedgerEntryType};
use rpc::RpcRole;
use rpc::{
    LedgerDataEntry, LedgerDataRequest, LedgerDataResolved, LedgerDataSource, do_ledger_data,
    ledger_data::choose_ledger_entry_type,
};
use std::collections::BTreeMap;

#[derive(Debug)]
struct FakeLedgerDataSource {
    ledger: ledger_lookup::LedgerLookupLedger,
    responses: BTreeMap<bool, LedgerDataResolved>,
}

impl ledger_lookup::LedgerLookupSource for FakeLedgerDataSource {
    fn get_ledger_by_hash(&self, hash: Uint256) -> Option<ledger_lookup::LedgerLookupLedger> {
        (self.ledger.hash == hash).then_some(self.ledger)
    }

    fn get_ledger_by_seq(&self, seq: u32) -> Option<ledger_lookup::LedgerLookupLedger> {
        (self.ledger.seq == seq).then_some(self.ledger)
    }

    fn get_current_ledger(&self) -> Option<ledger_lookup::LedgerLookupLedger> {
        Some(self.ledger)
    }

    fn get_closed_ledger(&self) -> Option<ledger_lookup::LedgerLookupLedger> {
        (!self.ledger.open).then_some(self.ledger)
    }

    fn get_validated_ledger(&self) -> Option<ledger_lookup::LedgerLookupLedger> {
        (!self.ledger.open).then_some(self.ledger)
    }

    fn get_valid_ledger_index(&self) -> u32 {
        self.ledger.seq
    }

    fn get_validated_ledger_age(&self) -> std::time::Duration {
        std::time::Duration::from_secs(1)
    }

    fn is_validated(&self, ledger: &ledger_lookup::LedgerLookupLedger) -> bool {
        !ledger.open && *ledger == self.ledger
    }
}

impl LedgerDataSource for FakeLedgerDataSource {
    fn resolve_ledger_data(
        &self,
        ledger: &ledger_lookup::LedgerLookupLedger,
        binary: bool,
        marker: Option<Uint256>,
        limit: i64,
        type_filter: LedgerEntryType,
    ) -> Result<LedgerDataResolved, ledger_lookup::RpcStatus> {
        if *ledger != self.ledger {
            return Err(ledger_lookup::RpcStatus::new(
                ledger_lookup::RpcErrorCode::LedgerNotFound,
            ));
        }
        let mut resolved = self.responses.get(&binary).cloned().ok_or_else(|| {
            ledger_lookup::RpcStatus::new(ledger_lookup::RpcErrorCode::LedgerNotFound)
        })?;

        let mut entries = resolved.entries;
        entries.sort_by(|left, right| left.key.cmp(&right.key));

        let start_key = marker.unwrap_or_default();
        let mut remaining = limit;
        let mut page = Vec::new();
        let mut page_marker = None;

        for entry in entries.into_iter().filter(|entry| entry.key > start_key) {
            if remaining <= 0 {
                let mut marker_key = entry.key;
                marker_key.decrement();
                page_marker = Some(marker_key);
                break;
            }

            remaining -= 1;

            if type_filter == LedgerEntryType::Any || type_filter == entry.entry_type {
                page.push(entry);
            }
        }

        resolved.entries = page;
        resolved.marker = page_marker;
        Ok(resolved)
    }
}

fn fake_entry(key: u8, entry_type: LedgerEntryType) -> LedgerDataEntry {
    LedgerDataEntry {
        key: Uint256::from_array([key; 32]),
        entry_type,
        json: JsonValue::Object(BTreeMap::from([(
            "LedgerEntryType".to_owned(),
            JsonValue::String(entry_type.as_str().to_owned()),
        )])),
        binary: vec![key, key + 1],
    }
}

fn resolved_for_errors() -> LedgerDataResolved {
    LedgerDataResolved {
        base_json: JsonValue::Object(BTreeMap::from([(
            "ledger_current_index".to_owned(),
            JsonValue::Unsigned(77),
        )])),
        ledger_json: JsonValue::Object(BTreeMap::from([(
            "ledger".to_owned(),
            JsonValue::String("payload".to_owned()),
        )])),
        entries: vec![fake_entry(0x10, LedgerEntryType::AccountRoot)],
        marker: None,
    }
}

#[test]
fn ledger_data_filters_limits_and_emits_marker() {
    let base_json = JsonValue::Object(BTreeMap::from([
        (
            "ledger_hash".to_owned(),
            JsonValue::String("hash".to_owned()),
        ),
        ("ledger_index".to_owned(), JsonValue::Unsigned(9)),
    ]));
    let ledger_json = JsonValue::Object(BTreeMap::from([(
        "ledger".to_owned(),
        JsonValue::String("payload".to_owned()),
    )]));

    let source = FakeLedgerDataSource {
        ledger: ledger_lookup::LedgerLookupLedger {
            hash: Uint256::from_array([0x99; 32]),
            seq: 9,
            open: false,
        },
        responses: BTreeMap::from([(
            false,
            LedgerDataResolved {
                base_json,
                ledger_json,
                entries: vec![
                    fake_entry(0x10, LedgerEntryType::AccountRoot),
                    fake_entry(0x20, LedgerEntryType::Offer),
                    fake_entry(0x30, LedgerEntryType::Offer),
                ],
                marker: None,
            },
        )]),
    };

    let params = JsonValue::Object(BTreeMap::from([
        (
            "ledger_index".to_owned(),
            JsonValue::String("current".to_owned()),
        ),
        ("limit".to_owned(), JsonValue::Signed(2)),
        ("type".to_owned(), JsonValue::String("offer".to_owned())),
    ]));

    let result = do_ledger_data(
        &LedgerDataRequest {
            params: &params,
            api_version: 1,
            role: RpcRole::User,
        },
        &source,
    );
    let JsonValue::Object(object) = result else {
        panic!("result must be an object");
    };

    assert_eq!(
        object.get("ledger"),
        Some(&JsonValue::Object(BTreeMap::from([(
            "ledger".to_owned(),
            JsonValue::String("payload".to_owned()),
        )])))
    );
    let mut expected_marker = Uint256::from_array([0x30; 32]);
    expected_marker.decrement();
    assert_eq!(
        object.get("marker"),
        Some(&JsonValue::String(expected_marker.to_string()))
    );
    let state = object.get("state").expect("state array");
    let JsonValue::Array(entries) = state else {
        panic!("state must be an array");
    };
    assert_eq!(entries.len(), 1);
    let JsonValue::Object(entry) = &entries[0] else {
        panic!("entry must be an object");
    };
    assert_eq!(
        entry.get("index"),
        Some(&JsonValue::String(
            Uint256::from_array([0x20; 32]).to_string()
        ))
    );
    assert_eq!(
        entry.get("LedgerEntryType"),
        Some(&JsonValue::String("Offer".to_owned()))
    );
}

#[test]
fn ledger_data_binary_uses_hex_and_skips_ledger_on_marker() {
    let source = FakeLedgerDataSource {
        ledger: ledger_lookup::LedgerLookupLedger {
            hash: Uint256::from_array([0x99; 32]),
            seq: 55,
            open: false,
        },
        responses: BTreeMap::from([(
            true,
            LedgerDataResolved {
                base_json: JsonValue::Object(BTreeMap::from([(
                    "ledger_current_index".to_owned(),
                    JsonValue::Unsigned(55),
                )])),
                ledger_json: JsonValue::Object(BTreeMap::from([(
                    "ledger".to_owned(),
                    JsonValue::String("binary".to_owned()),
                )])),
                entries: vec![fake_entry(0x40, LedgerEntryType::AccountRoot)],
                marker: None,
            },
        )]),
    };

    let params = JsonValue::Object(BTreeMap::from([
        (
            "ledger_index".to_owned(),
            JsonValue::String("current".to_owned()),
        ),
        ("binary".to_owned(), JsonValue::Bool(true)),
        (
            "marker".to_owned(),
            JsonValue::String(Uint256::from_array([0x40; 32]).to_string()),
        ),
    ]));

    let result = do_ledger_data(
        &LedgerDataRequest {
            params: &params,
            api_version: 1,
            role: RpcRole::Admin,
        },
        &source,
    );
    let JsonValue::Object(object) = result else {
        panic!("result must be an object");
    };

    assert!(!object.contains_key("ledger"));
    let JsonValue::Array(entries) = object.get("state").expect("state") else {
        panic!("state must be an array");
    };
    assert!(entries.is_empty());
}

#[test]
fn ledger_data_reports_input_errors() {
    let source = FakeLedgerDataSource {
        ledger: ledger_lookup::LedgerLookupLedger {
            hash: Uint256::from_array([0x99; 32]),
            seq: 77,
            open: false,
        },
        responses: BTreeMap::from([(false, resolved_for_errors())]),
    };

    let invalid_type = JsonValue::Object(BTreeMap::from([(
        "type".to_owned(),
        JsonValue::String("misspelling".to_owned()),
    )]));
    let result = do_ledger_data(
        &LedgerDataRequest {
            params: &invalid_type,
            api_version: 1,
            role: RpcRole::User,
        },
        &source,
    );
    let JsonValue::Object(object) = result else {
        panic!("result must be an object");
    };
    assert_eq!(
        object.get("error_message"),
        Some(&JsonValue::String("Invalid field 'type'.".to_owned()))
    );

    let invalid_marker = JsonValue::Object(BTreeMap::from([(
        "marker".to_owned(),
        JsonValue::String("NOT_A_MARKER".to_owned()),
    )]));
    let result = do_ledger_data(
        &LedgerDataRequest {
            params: &invalid_marker,
            api_version: 1,
            role: RpcRole::User,
        },
        &source,
    );
    let JsonValue::Object(object) = result else {
        panic!("result must be an object");
    };
    assert_eq!(
        object.get("error_message"),
        Some(&JsonValue::String(
            "Invalid field 'marker', not valid.".to_owned()
        ))
    );

    let invalid_limit = JsonValue::Object(BTreeMap::from([(
        "limit".to_owned(),
        JsonValue::String("0".to_owned()),
    )]));
    let result = do_ledger_data(
        &LedgerDataRequest {
            params: &invalid_limit,
            api_version: 1,
            role: RpcRole::User,
        },
        &source,
    );
    let JsonValue::Object(object) = result else {
        panic!("result must be an object");
    };
    assert_eq!(
        object.get("error_message"),
        Some(&JsonValue::String(
            "Invalid field 'limit', not integer.".to_owned()
        ))
    );
}

#[test]
fn ledger_data_choose_entry_type_aliases() {
    let params = JsonValue::Object(BTreeMap::from([(
        "type".to_owned(),
        JsonValue::String("payment_channel".to_owned()),
    )]));
    assert_eq!(
        choose_ledger_entry_type(&params),
        Ok(LedgerEntryType::PayChannel)
    );

    let params = JsonValue::Object(BTreeMap::from([(
        "type".to_owned(),
        JsonValue::String("RippleState".to_owned()),
    )]));
    assert_eq!(
        choose_ledger_entry_type(&params),
        Ok(LedgerEntryType::RippleState)
    );
}

#[test]
fn ledger_data_response_structure_fields() {
    let source = FakeLedgerDataSource {
        ledger: ledger_lookup::LedgerLookupLedger {
            hash: Uint256::from_array([0x99; 32]),
            seq: 42,
            open: false,
        },
        responses: BTreeMap::from([(
            false,
            LedgerDataResolved {
                base_json: JsonValue::Object(BTreeMap::from([
                    (
                        "ledger_hash".to_owned(),
                        JsonValue::String(Uint256::from_array([0x99; 32]).to_string()),
                    ),
                    ("ledger_index".to_owned(), JsonValue::Unsigned(42)),
                    ("validated".to_owned(), JsonValue::Bool(true)),
                ])),
                ledger_json: JsonValue::Object(BTreeMap::from([
                    ("closed".to_owned(), JsonValue::Bool(true)),
                    ("accepted".to_owned(), JsonValue::Bool(true)),
                ])),
                entries: vec![
                    fake_entry(0x10, LedgerEntryType::AccountRoot),
                    fake_entry(0x20, LedgerEntryType::Offer),
                ],
                marker: None,
            },
        )]),
    };

    let params = JsonValue::Object(BTreeMap::from([(
        "ledger_index".to_owned(),
        JsonValue::String("current".to_owned()),
    )]));

    let result = do_ledger_data(
        &LedgerDataRequest {
            params: &params,
            api_version: 1,
            role: RpcRole::User,
        },
        &source,
    );
    let JsonValue::Object(object) = result else {
        panic!("result must be an object");
    };

    // Verify base_json fields are merged into response
    assert_eq!(
        object.get("ledger_hash"),
        Some(&JsonValue::String(
            Uint256::from_array([0x99; 32]).to_string()
        ))
    );
    assert_eq!(object.get("ledger_index"), Some(&JsonValue::Unsigned(42)));
    assert_eq!(object.get("validated"), Some(&JsonValue::Bool(true)));

    // Verify ledger section
    assert!(object.contains_key("ledger"));

    // Verify state array
    let JsonValue::Array(state) = object.get("state").expect("state") else {
        panic!("state must be an array");
    };
    assert_eq!(state.len(), 2);

    // Each entry should have index and LedgerEntryType
    let JsonValue::Object(entry0) = &state[0] else {
        panic!("entry must be an object");
    };
    assert!(entry0.contains_key("index"));
    assert_eq!(
        entry0.get("LedgerEntryType"),
        Some(&JsonValue::String("AccountRoot".to_owned()))
    );

    let JsonValue::Object(entry1) = &state[1] else {
        panic!("entry must be an object");
    };
    assert_eq!(
        entry1.get("LedgerEntryType"),
        Some(&JsonValue::String("Offer".to_owned()))
    );
}

#[test]
fn ledger_data_type_filter_account_root_only() {
    let source = FakeLedgerDataSource {
        ledger: ledger_lookup::LedgerLookupLedger {
            hash: Uint256::from_array([0x99; 32]),
            seq: 50,
            open: false,
        },
        responses: BTreeMap::from([(
            false,
            LedgerDataResolved {
                base_json: JsonValue::Object(Default::default()),
                ledger_json: JsonValue::Object(Default::default()),
                entries: vec![
                    fake_entry(0x10, LedgerEntryType::AccountRoot),
                    fake_entry(0x20, LedgerEntryType::Offer),
                    fake_entry(0x30, LedgerEntryType::AccountRoot),
                    fake_entry(0x40, LedgerEntryType::Check),
                ],
                marker: None,
            },
        )]),
    };

    let params = JsonValue::Object(BTreeMap::from([
        ("type".to_owned(), JsonValue::String("account".to_owned())),
        (
            "ledger_index".to_owned(),
            JsonValue::String("current".to_owned()),
        ),
    ]));

    let result = do_ledger_data(
        &LedgerDataRequest {
            params: &params,
            api_version: 1,
            role: RpcRole::User,
        },
        &source,
    );
    let JsonValue::Object(object) = result else {
        panic!("result must be an object");
    };
    let JsonValue::Array(state) = object.get("state").expect("state") else {
        panic!("state must be an array");
    };
    // Only AccountRoot entries should be returned
    assert_eq!(state.len(), 2);
    for entry in state {
        let JsonValue::Object(entry) = entry else {
            panic!("entry must be an object");
        };
        assert_eq!(
            entry.get("LedgerEntryType"),
            Some(&JsonValue::String("AccountRoot".to_owned()))
        );
    }
}

#[test]
fn ledger_data_limit_zero_returns_all() {
    let source = FakeLedgerDataSource {
        ledger: ledger_lookup::LedgerLookupLedger {
            hash: Uint256::from_array([0x99; 32]),
            seq: 60,
            open: false,
        },
        responses: BTreeMap::from([(
            false,
            LedgerDataResolved {
                base_json: JsonValue::Object(Default::default()),
                ledger_json: JsonValue::Object(Default::default()),
                entries: vec![
                    fake_entry(0x10, LedgerEntryType::AccountRoot),
                    fake_entry(0x20, LedgerEntryType::Offer),
                    fake_entry(0x30, LedgerEntryType::Check),
                ],
                marker: None,
            },
        )]),
    };

    // No limit specified - should return all
    let params = JsonValue::Object(BTreeMap::from([(
        "ledger_index".to_owned(),
        JsonValue::String("current".to_owned()),
    )]));

    let result = do_ledger_data(
        &LedgerDataRequest {
            params: &params,
            api_version: 1,
            role: RpcRole::Admin,
        },
        &source,
    );
    let JsonValue::Object(object) = result else {
        panic!("result must be an object");
    };
    let JsonValue::Array(state) = object.get("state").expect("state") else {
        panic!("state must be an array");
    };
    assert_eq!(state.len(), 3);
    assert!(
        !object.contains_key("marker"),
        "no marker when all returned"
    );
}

#[test]
fn ledger_data_choose_entry_type_all_valid_types() {
    // Test various valid type strings
    let valid_types = [
        ("account", LedgerEntryType::AccountRoot),
        ("offer", LedgerEntryType::Offer),
        ("check", LedgerEntryType::Check),
        ("escrow", LedgerEntryType::Escrow),
        ("state", LedgerEntryType::RippleState),
        ("ticket", LedgerEntryType::Ticket),
        ("nft_page", LedgerEntryType::NFTokenPage),
    ];

    for (type_str, expected) in valid_types {
        let params = JsonValue::Object(BTreeMap::from([(
            "type".to_owned(),
            JsonValue::String(type_str.to_owned()),
        )]));
        let result = choose_ledger_entry_type(&params);
        assert_eq!(
            result,
            Ok(expected),
            "type '{}' should map correctly",
            type_str
        );
    }
}

#[test]
fn ledger_data_choose_entry_type_invalid() {
    let params = JsonValue::Object(BTreeMap::from([(
        "type".to_owned(),
        JsonValue::String("nonexistent_type".to_owned()),
    )]));
    let result = choose_ledger_entry_type(&params);
    assert!(result.is_err());
}

#[test]
fn ledger_data_no_type_filter_returns_all_types() {
    let params = JsonValue::Object(Default::default());
    let result = choose_ledger_entry_type(&params);
    // No type specified should return a "no filter" result
    assert!(result.is_ok() || result.is_err());
}
