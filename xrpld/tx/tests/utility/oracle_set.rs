//! Integration tests that pin the narrowed Rust `OracleSet.cpp` shells to the
//! current C++ behavior.

use protocol::{Ter, trans_token};
use tx::{
    MAX_ORACLE_DATA_SERIES, MAX_ORACLE_LAST_UPDATE_TIME_DELTA_SECS, MAX_ORACLE_PRICE_SCALE,
    MAX_ORACLE_PROVIDER_LEN, MAX_ORACLE_SYMBOL_CLASS_LEN, MAX_ORACLE_URI_LEN,
    ORACLE_LAST_UPDATE_TIME_EPOCH_OFFSET_SECS, OracleSetApplyFacts, OracleSetApplySink,
    OracleSetCreateMutation, OracleSetLoadedOracle, OracleSetPreclaimFacts,
    OracleSetPreclaimFrontFacts, OracleSetPreflightFacts, OracleSetReserveSink,
    OracleSetSeriesEntry, OracleSetUpdateMutation, OracleTokenPair, run_oracle_set_do_apply,
    run_oracle_set_preclaim, run_oracle_set_preclaim_front, run_oracle_set_preflight,
};

#[derive(Debug, Clone, PartialEq, Eq)]
struct TestReserveSink {
    reserve_ok: bool,
    seen_adjust_reserve: Vec<i8>,
}

impl TestReserveSink {
    fn new() -> Self {
        Self {
            reserve_ok: true,
            seen_adjust_reserve: Vec::new(),
        }
    }
}

impl OracleSetReserveSink for TestReserveSink {
    fn is_reserve_sufficient(&mut self, adjust_reserve: i8) -> bool {
        self.seen_adjust_reserve.push(adjust_reserve);
        self.reserve_ok
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct TestApplySink {
    existing_oracle: Option<OracleSetLoadedOracle>,
    fix_include_keylet_fields_enabled: bool,
    fix_price_oracle_order_enabled: bool,
    adjust_owner_count_ok: bool,
    insert_owner_dir_result: Option<u64>,
    owner_count_adjustments: Vec<i8>,
    update_mutation: Option<OracleSetUpdateMutation>,
    create_mutation: Option<OracleSetCreateMutation>,
}

impl TestApplySink {
    fn new() -> Self {
        Self {
            existing_oracle: None,
            fix_include_keylet_fields_enabled: false,
            fix_price_oracle_order_enabled: false,
            adjust_owner_count_ok: true,
            insert_owner_dir_result: Some(42),
            owner_count_adjustments: Vec::new(),
            update_mutation: None,
            create_mutation: None,
        }
    }
}

impl OracleSetApplySink for TestApplySink {
    fn existing_oracle(&mut self) -> Option<OracleSetLoadedOracle> {
        self.existing_oracle.clone()
    }

    fn fix_include_keylet_fields_enabled(&mut self) -> bool {
        self.fix_include_keylet_fields_enabled
    }

    fn fix_price_oracle_order_enabled(&mut self) -> bool {
        self.fix_price_oracle_order_enabled
    }

    fn adjust_owner_count(&mut self, delta: i8) -> bool {
        self.owner_count_adjustments.push(delta);
        self.adjust_owner_count_ok
    }

    fn update_existing_oracle(&mut self, mutation: OracleSetUpdateMutation) {
        self.update_mutation = Some(mutation);
    }

    fn insert_owner_dir(&mut self) -> Option<u64> {
        self.insert_owner_dir_result
    }

    fn create_oracle(&mut self, mutation: OracleSetCreateMutation) {
        self.create_mutation = Some(mutation);
    }
}

fn preflight_facts() -> OracleSetPreflightFacts {
    OracleSetPreflightFacts {
        price_data_series_len: 1,
        provider_len: None,
        uri_len: None,
        asset_class_len: None,
    }
}

fn preclaim_front_facts() -> OracleSetPreclaimFrontFacts {
    OracleSetPreclaimFrontFacts {
        account_exists: true,
        close_time_secs: 10_000,
        last_update_time_secs: ORACLE_LAST_UPDATE_TIME_EPOCH_OFFSET_SECS + 10_000,
    }
}

fn pair(base: &str, quote: &str) -> OracleTokenPair {
    OracleTokenPair {
        base_asset: base.to_string(),
        quote_asset: quote.to_string(),
    }
}

fn entry(
    base: &str,
    quote: &str,
    asset_price: Option<u64>,
    scale: Option<u16>,
) -> OracleSetSeriesEntry {
    OracleSetSeriesEntry {
        pair: pair(base, quote),
        asset_price,
        scale,
    }
}

fn preclaim_facts() -> OracleSetPreclaimFacts {
    OracleSetPreclaimFacts {
        front: preclaim_front_facts(),
        oracle_exists: false,
        tx_provider_present: true,
        tx_asset_class_present: true,
        tx_provider_matches_existing: true,
        tx_asset_class_matches_existing: true,
        previous_last_update_time_secs: ORACLE_LAST_UPDATE_TIME_EPOCH_OFFSET_SECS + 9_000,
        tx_series: vec![entry("XRP", "USD", Some(740), Some(1))],
        existing_pairs: Vec::new(),
    }
}

fn apply_facts() -> OracleSetApplyFacts {
    OracleSetApplyFacts {
        provider: "provider".to_string(),
        asset_class: "currency".to_string(),
        uri: Some("URI".to_string()),
        last_update_time_secs: ORACLE_LAST_UPDATE_TIME_EPOCH_OFFSET_SECS + 10_000,
        oracle_document_id: 7,
        tx_series: vec![entry("XRP", "USD", Some(740), Some(1))],
    }
}

#[test]
fn oracle_set_preflight_rejects_empty_series_before_field_length_checks() {
    let result = run_oracle_set_preflight(OracleSetPreflightFacts {
        price_data_series_len: 0,
        provider_len: Some(0),
        uri_len: Some(MAX_ORACLE_URI_LEN + 1),
        asset_class_len: Some(MAX_ORACLE_SYMBOL_CLASS_LEN + 1),
    });

    assert_eq!(result, Ter::TEM_ARRAY_EMPTY);
    assert_eq!(trans_token(result), "temARRAY_EMPTY");
}

#[test]
fn oracle_set_preflight_rejects_oversized_series() {
    let result = run_oracle_set_preflight(OracleSetPreflightFacts {
        price_data_series_len: MAX_ORACLE_DATA_SERIES + 1,
        provider_len: Some(MAX_ORACLE_PROVIDER_LEN + 1),
        uri_len: None,
        asset_class_len: None,
    });

    assert_eq!(result, Ter::TEM_ARRAY_TOO_LARGE);
    assert_eq!(trans_token(result), "temARRAY_TOO_LARGE");
}

#[test]
fn oracle_set_preflight_rejects_zero_length_provider() {
    let result = run_oracle_set_preflight(OracleSetPreflightFacts {
        provider_len: Some(0),
        ..preflight_facts()
    });

    assert_eq!(result, Ter::TEM_MALFORMED);
    assert_eq!(trans_token(result), "temMALFORMED");
}

#[test]
fn oracle_set_preflight_accepts_max_lengths() {
    let result = run_oracle_set_preflight(OracleSetPreflightFacts {
        price_data_series_len: MAX_ORACLE_DATA_SERIES,
        provider_len: Some(MAX_ORACLE_PROVIDER_LEN),
        uri_len: Some(MAX_ORACLE_URI_LEN),
        asset_class_len: Some(MAX_ORACLE_SYMBOL_CLASS_LEN),
    });

    assert_eq!(result, Ter::TES_SUCCESS);
}

#[test]
fn oracle_set_preclaim_front_rejects_missing_account() {
    let result = run_oracle_set_preclaim_front(OracleSetPreclaimFrontFacts {
        account_exists: false,
        close_time_secs: 0,
        last_update_time_secs: 0,
    });

    assert_eq!(result, Ter::TER_NO_ACCOUNT);
    assert_eq!(trans_token(result), "terNO_ACCOUNT");
}

#[test]
fn oracle_set_preclaim_front_maps_small_close_time_to_internal() {
    let result = run_oracle_set_preclaim_front(OracleSetPreclaimFrontFacts {
        close_time_secs: MAX_ORACLE_LAST_UPDATE_TIME_DELTA_SECS - 1,
        last_update_time_secs: ORACLE_LAST_UPDATE_TIME_EPOCH_OFFSET_SECS,
        ..preclaim_front_facts()
    });

    assert_eq!(result, Ter::TEC_INTERNAL);
    assert_eq!(trans_token(result), "tecINTERNAL");
}

#[test]
fn oracle_set_preclaim_rejects_duplicate_pair() {
    let mut sink = TestReserveSink::new();
    let result = run_oracle_set_preclaim(
        OracleSetPreclaimFacts {
            tx_series: vec![
                entry("XRP", "USD", Some(740), Some(1)),
                entry("XRP", "USD", None, None),
            ],
            oracle_exists: true,
            existing_pairs: vec![pair("XRP", "USD")],
            ..preclaim_facts()
        },
        &mut sink,
    );

    assert_eq!(result, Ter::TEM_MALFORMED);
}

#[test]
fn oracle_set_preclaim_rejects_scale_above_max() {
    let mut sink = TestReserveSink::new();
    let result = run_oracle_set_preclaim(
        OracleSetPreclaimFacts {
            tx_series: vec![entry(
                "USD",
                "BTC",
                Some(740),
                Some(MAX_ORACLE_PRICE_SCALE + 1),
            )],
            ..preclaim_facts()
        },
        &mut sink,
    );

    assert_eq!(result, Ter::TEM_MALFORMED);
}

#[test]
fn oracle_set_preclaim_preserves_update_owner_count_band_change() {
    let mut sink = TestReserveSink::new();
    let result = run_oracle_set_preclaim(
        OracleSetPreclaimFacts {
            oracle_exists: true,
            tx_series: vec![
                entry("XRP", "USD", Some(742), Some(2)),
                entry("XRP", "EUR", Some(711), Some(2)),
                entry("ETH", "EUR", None, None),
                entry("YAN", "EUR", None, None),
                entry("CAN", "EUR", None, None),
            ],
            existing_pairs: vec![
                pair("XRP", "USD"),
                pair("XRP", "EUR"),
                pair("BTC", "USD"),
                pair("ETH", "EUR"),
                pair("YAN", "EUR"),
                pair("CAN", "EUR"),
            ],
            ..preclaim_facts()
        },
        &mut sink,
    );

    assert_eq!(result, Ter::TES_SUCCESS);
    assert_eq!(sink.seen_adjust_reserve, vec![-1]);
}

#[test]
fn oracle_set_do_apply_create_preserves_input_order_without_fix() {
    let mut sink = TestApplySink::new();
    let result = run_oracle_set_do_apply(
        OracleSetApplyFacts {
            tx_series: vec![
                entry("XRP", "USD", Some(742), Some(2)),
                entry("XRP", "EUR", Some(711), Some(2)),
            ],
            ..apply_facts()
        },
        &mut sink,
    );

    assert_eq!(result, Ter::TES_SUCCESS);
    assert_eq!(sink.owner_count_adjustments, vec![1]);
    let create = sink.create_mutation.expect("create mutation");
    assert_eq!(create.price_data_series[0].pair, pair("XRP", "USD"));
    assert_eq!(create.price_data_series[1].pair, pair("XRP", "EUR"));
    assert_eq!(create.oracle_document_id, 7);
    assert!(!create.include_oracle_document_id);
}

#[test]
fn oracle_set_do_apply_create_sorts_pairs_with_fix() {
    let mut sink = TestApplySink::new();
    sink.fix_include_keylet_fields_enabled = true;
    sink.fix_price_oracle_order_enabled = true;

    let result = run_oracle_set_do_apply(
        OracleSetApplyFacts {
            tx_series: vec![
                entry("XRP", "USD", Some(742), Some(2)),
                entry("XRP", "EUR", Some(711), Some(2)),
            ],
            ..apply_facts()
        },
        &mut sink,
    );

    assert_eq!(result, Ter::TES_SUCCESS);
    let create = sink.create_mutation.expect("create mutation");
    assert_eq!(create.price_data_series[0].pair, pair("XRP", "EUR"));
    assert_eq!(create.price_data_series[1].pair, pair("XRP", "USD"));
    assert!(create.include_oracle_document_id);
}

#[test]
fn oracle_set_do_apply_create_raises_owner_count_for_more_than_five_pairs() {
    let mut sink = TestApplySink::new();
    let result = run_oracle_set_do_apply(
        OracleSetApplyFacts {
            tx_series: vec![
                entry("XRP", "USD", Some(740), Some(1)),
                entry("BTC", "USD", Some(740), Some(1)),
                entry("ETH", "USD", Some(740), Some(1)),
                entry("CAN", "USD", Some(740), Some(1)),
                entry("YAN", "USD", Some(740), Some(1)),
                entry("GBP", "USD", Some(740), Some(1)),
            ],
            ..apply_facts()
        },
        &mut sink,
    );

    assert_eq!(result, Ter::TES_SUCCESS);
    assert_eq!(sink.owner_count_adjustments, vec![2]);
}

#[test]
fn oracle_set_do_apply_create_maps_dir_insert_failure() {
    let mut sink = TestApplySink::new();
    sink.insert_owner_dir_result = None;

    let result = run_oracle_set_do_apply(apply_facts(), &mut sink);

    assert_eq!(result, Ter::TEC_DIR_FULL);
    assert!(sink.create_mutation.is_none());
}

#[test]
fn oracle_set_do_apply_update_merges_pairs_and_sets_document_id() {
    let mut sink = TestApplySink::new();
    sink.existing_oracle = Some(OracleSetLoadedOracle {
        has_oracle_document_id: false,
        price_data_series: vec![
            entry("XRP", "USD", Some(741), Some(2)),
            entry("XRP", "EUR", Some(710), Some(2)),
            entry("BTC", "USD", Some(741), Some(2)),
            entry("ETH", "EUR", Some(710), Some(2)),
            entry("YAN", "EUR", Some(710), Some(2)),
            entry("CAN", "EUR", Some(710), Some(2)),
        ],
    });
    sink.fix_include_keylet_fields_enabled = true;

    let result = run_oracle_set_do_apply(
        OracleSetApplyFacts {
            tx_series: vec![
                entry("XRP", "USD", Some(742), Some(2)),
                entry("XRP", "EUR", Some(711), Some(2)),
                entry("ETH", "EUR", None, None),
                entry("YAN", "EUR", None, None),
                entry("CAN", "EUR", None, None),
            ],
            ..apply_facts()
        },
        &mut sink,
    );

    assert_eq!(result, Ter::TES_SUCCESS);
    assert_eq!(sink.owner_count_adjustments, vec![-1]);
    let update = sink.update_mutation.expect("update mutation");
    assert!(update.set_oracle_document_id);
    assert_eq!(update.oracle_document_id, 7);
    assert_eq!(
        update.updated_series,
        vec![
            entry("BTC", "USD", None, None),
            entry("XRP", "EUR", Some(711), Some(2)),
            entry("XRP", "USD", Some(742), Some(2)),
        ]
    );
}

#[test]
fn oracle_set_do_apply_update_preserves_creation_order_difference_without_fix() {
    let mut sink = TestApplySink::new();
    sink.existing_oracle = Some(OracleSetLoadedOracle {
        has_oracle_document_id: true,
        price_data_series: vec![
            entry("XRP", "USD", Some(742), Some(2)),
            entry("XRP", "EUR", Some(711), Some(2)),
        ],
    });

    let result = run_oracle_set_do_apply(
        OracleSetApplyFacts {
            tx_series: vec![
                entry("XRP", "USD", Some(742), Some(2)),
                entry("XRP", "EUR", Some(711), Some(2)),
            ],
            ..apply_facts()
        },
        &mut sink,
    );

    assert_eq!(result, Ter::TES_SUCCESS);
    let update = sink.update_mutation.expect("update mutation");
    assert_eq!(update.updated_series[0].pair, pair("XRP", "EUR"));
    assert_eq!(update.updated_series[1].pair, pair("XRP", "USD"));
}

#[test]
fn oracle_set_do_apply_update_maps_owner_count_failure() {
    let mut sink = TestApplySink::new();
    sink.adjust_owner_count_ok = false;
    sink.existing_oracle = Some(OracleSetLoadedOracle {
        has_oracle_document_id: true,
        price_data_series: vec![
            entry("XRP", "USD", Some(741), Some(2)),
            entry("XRP", "EUR", Some(710), Some(2)),
            entry("BTC", "USD", Some(741), Some(2)),
            entry("ETH", "EUR", Some(710), Some(2)),
            entry("YAN", "EUR", Some(710), Some(2)),
            entry("CAN", "EUR", Some(710), Some(2)),
        ],
    });

    let result = run_oracle_set_do_apply(
        OracleSetApplyFacts {
            tx_series: vec![
                entry("XRP", "USD", Some(742), Some(2)),
                entry("XRP", "EUR", Some(711), Some(2)),
                entry("ETH", "EUR", None, None),
                entry("YAN", "EUR", None, None),
                entry("CAN", "EUR", None, None),
            ],
            ..apply_facts()
        },
        &mut sink,
    );

    assert_eq!(result, Ter::TEF_INTERNAL);
    assert!(sink.update_mutation.is_none());
}
