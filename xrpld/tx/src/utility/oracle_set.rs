//! the reference implementation compatibility surface.
//!
//! This ports the current deterministic `preflight(...)`, `preclaim(...)`,
//! and `doApply()` control-flow shells.

use std::collections::{BTreeMap, BTreeSet};

use protocol::{NotTec, Ter, is_tes_success};

pub const MAX_ORACLE_URI_LEN: usize = 256;
pub const MAX_ORACLE_PROVIDER_LEN: usize = 256;
pub const MAX_ORACLE_DATA_SERIES: usize = 10;
pub const MAX_ORACLE_SYMBOL_CLASS_LEN: usize = 16;
pub const ORACLE_LAST_UPDATE_TIME_EPOCH_OFFSET_SECS: u64 = 946_684_800;
pub const MAX_ORACLE_LAST_UPDATE_TIME_DELTA_SECS: u64 = 300;
pub const MAX_ORACLE_PRICE_SCALE: u16 = 20;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OracleSetPreflightFacts {
    pub price_data_series_len: usize,
    pub provider_len: Option<usize>,
    pub uri_len: Option<usize>,
    pub asset_class_len: Option<usize>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OracleSetPreclaimFrontFacts {
    pub account_exists: bool,
    pub close_time_secs: u64,
    pub last_update_time_secs: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct OracleTokenPair {
    pub base_asset: String,
    pub quote_asset: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OracleSetSeriesEntry {
    pub pair: OracleTokenPair,
    pub asset_price: Option<u64>,
    pub scale: Option<u16>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OracleSetPreclaimFacts {
    pub front: OracleSetPreclaimFrontFacts,
    pub oracle_exists: bool,
    pub tx_provider_present: bool,
    pub tx_asset_class_present: bool,
    pub tx_provider_matches_existing: bool,
    pub tx_asset_class_matches_existing: bool,
    pub previous_last_update_time_secs: u64,
    pub tx_series: Vec<OracleSetSeriesEntry>,
    pub existing_pairs: Vec<OracleTokenPair>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OracleSetApplyFacts {
    pub provider: String,
    pub asset_class: String,
    pub uri: Option<String>,
    pub last_update_time_secs: u64,
    pub oracle_document_id: u32,
    pub tx_series: Vec<OracleSetSeriesEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OracleSetLoadedOracle {
    pub has_oracle_document_id: bool,
    pub price_data_series: Vec<OracleSetSeriesEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OracleSetUpdateMutation {
    pub updated_series: Vec<OracleSetSeriesEntry>,
    pub uri: Option<String>,
    pub last_update_time_secs: u64,
    pub set_oracle_document_id: bool,
    pub oracle_document_id: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OracleSetCreateMutation {
    pub provider: String,
    pub asset_class: String,
    pub uri: Option<String>,
    pub price_data_series: Vec<OracleSetSeriesEntry>,
    pub last_update_time_secs: u64,
    pub include_oracle_document_id: bool,
    pub oracle_document_id: u32,
    pub owner_node: u64,
}

pub trait OracleSetReserveSink {
    fn is_reserve_sufficient(&mut self, adjust_reserve: i8) -> bool;
}

pub trait OracleSetApplySink {
    fn existing_oracle(&mut self) -> Option<OracleSetLoadedOracle>;
    fn fix_include_keylet_fields_enabled(&mut self) -> bool;
    fn fix_price_oracle_order_enabled(&mut self) -> bool;
    fn adjust_owner_count(&mut self, delta: i8) -> bool;
    fn update_existing_oracle(&mut self, mutation: OracleSetUpdateMutation);
    fn insert_owner_dir(&mut self) -> Option<u64>;
    fn create_oracle(&mut self, mutation: OracleSetCreateMutation);
}

fn invalid_length(length: Option<usize>, max_len: usize) -> bool {
    matches!(length, Some(0)) || length.is_some_and(|len| len > max_len)
}

fn owner_count_for_pair_count(pair_count: usize) -> i8 {
    if pair_count > 5 { 2 } else { 1 }
}

fn collect_requested_pairs(
    facts: &OracleSetPreclaimFacts,
) -> Result<(BTreeSet<OracleTokenPair>, BTreeSet<OracleTokenPair>), Ter> {
    let mut pairs = BTreeSet::new();
    let mut pairs_del = BTreeSet::new();

    for entry in &facts.tx_series {
        if entry.pair.base_asset == entry.pair.quote_asset {
            return Err(Ter::TEM_MALFORMED);
        }

        if pairs.contains(&entry.pair) || pairs_del.contains(&entry.pair) {
            return Err(Ter::TEM_MALFORMED);
        }

        if entry.scale.unwrap_or(0) > MAX_ORACLE_PRICE_SCALE {
            return Err(Ter::TEM_MALFORMED);
        }

        if entry.asset_price.is_some() {
            pairs.insert(entry.pair.clone());
        } else if facts.oracle_exists {
            pairs_del.insert(entry.pair.clone());
        } else {
            return Err(Ter::TEM_MALFORMED);
        }
    }

    Ok((pairs, pairs_del))
}

fn collect_current_pairs_for_update(
    loaded: &OracleSetLoadedOracle,
) -> BTreeMap<OracleTokenPair, OracleSetSeriesEntry> {
    let mut pairs = BTreeMap::new();
    for entry in &loaded.price_data_series {
        pairs.insert(
            entry.pair.clone(),
            OracleSetSeriesEntry {
                pair: entry.pair.clone(),
                asset_price: None,
                scale: None,
            },
        );
    }
    pairs
}

fn populated_entry(entry: &OracleSetSeriesEntry) -> OracleSetSeriesEntry {
    OracleSetSeriesEntry {
        pair: entry.pair.clone(),
        asset_price: entry.asset_price,
        scale: entry.scale,
    }
}

pub fn run_oracle_set_preflight(facts: OracleSetPreflightFacts) -> NotTec {
    if facts.price_data_series_len == 0 {
        return Ter::TEM_ARRAY_EMPTY;
    }

    if facts.price_data_series_len > MAX_ORACLE_DATA_SERIES {
        return Ter::TEM_ARRAY_TOO_LARGE;
    }

    if invalid_length(facts.provider_len, MAX_ORACLE_PROVIDER_LEN)
        || invalid_length(facts.uri_len, MAX_ORACLE_URI_LEN)
        || invalid_length(facts.asset_class_len, MAX_ORACLE_SYMBOL_CLASS_LEN)
    {
        return Ter::TEM_MALFORMED;
    }

    Ter::TES_SUCCESS
}

pub fn run_oracle_set_preclaim_front(facts: OracleSetPreclaimFrontFacts) -> Ter {
    if !facts.account_exists {
        return Ter::TER_NO_ACCOUNT;
    }

    if facts.last_update_time_secs < ORACLE_LAST_UPDATE_TIME_EPOCH_OFFSET_SECS {
        return Ter::TEC_INVALID_UPDATE_TIME;
    }

    let last_update_time_epoch =
        facts.last_update_time_secs - ORACLE_LAST_UPDATE_TIME_EPOCH_OFFSET_SECS;
    if facts.close_time_secs < MAX_ORACLE_LAST_UPDATE_TIME_DELTA_SECS {
        return Ter::TEC_INTERNAL;
    }

    let lower_bound = facts.close_time_secs - MAX_ORACLE_LAST_UPDATE_TIME_DELTA_SECS;
    let upper_bound = facts.close_time_secs + MAX_ORACLE_LAST_UPDATE_TIME_DELTA_SECS;
    if last_update_time_epoch < lower_bound || last_update_time_epoch > upper_bound {
        return Ter::TEC_INVALID_UPDATE_TIME;
    }

    Ter::TES_SUCCESS
}

pub fn run_oracle_set_preclaim<S: OracleSetReserveSink>(
    facts: OracleSetPreclaimFacts,
    sink: &mut S,
) -> Ter {
    let front = run_oracle_set_preclaim_front(facts.front);
    if !is_tes_success(front) {
        return front;
    }

    let (mut pairs, mut pairs_del) = match collect_requested_pairs(&facts) {
        Ok(collected) => collected,
        Err(err) => return err,
    };

    let adjust_reserve = if facts.oracle_exists {
        if facts.front.last_update_time_secs <= facts.previous_last_update_time_secs {
            return Ter::TEC_INVALID_UPDATE_TIME;
        }

        if !facts.tx_provider_matches_existing || !facts.tx_asset_class_matches_existing {
            return Ter::TEM_MALFORMED;
        }

        for pair in &facts.existing_pairs {
            if !pairs.contains(pair) {
                if pairs_del.contains(pair) {
                    pairs_del.remove(pair);
                } else {
                    pairs.insert(pair.clone());
                }
            }
        }

        if !pairs_del.is_empty() {
            return Ter::TEC_TOKEN_PAIR_NOT_FOUND;
        }

        owner_count_for_pair_count(pairs.len())
            - owner_count_for_pair_count(facts.existing_pairs.len())
    } else {
        if !facts.tx_provider_present || !facts.tx_asset_class_present {
            return Ter::TEM_MALFORMED;
        }

        owner_count_for_pair_count(pairs.len())
    };

    if pairs.is_empty() {
        return Ter::TEC_ARRAY_EMPTY;
    }

    if pairs.len() > MAX_ORACLE_DATA_SERIES {
        return Ter::TEC_ARRAY_TOO_LARGE;
    }

    if !sink.is_reserve_sufficient(adjust_reserve) {
        return Ter::TEC_INSUFFICIENT_RESERVE;
    }

    Ter::TES_SUCCESS
}

pub fn run_oracle_set_do_apply<S: OracleSetApplySink>(
    facts: OracleSetApplyFacts,
    sink: &mut S,
) -> Ter {
    if let Some(loaded) = sink.existing_oracle() {
        let mut pairs = collect_current_pairs_for_update(&loaded);
        let old_count = owner_count_for_pair_count(pairs.len());

        for entry in &facts.tx_series {
            if entry.asset_price.is_none() {
                pairs.remove(&entry.pair);
            } else if let Some(current) = pairs.get_mut(&entry.pair) {
                current.asset_price = entry.asset_price;
                if entry.scale.is_some() {
                    current.scale = entry.scale;
                }
            } else {
                pairs.insert(entry.pair.clone(), populated_entry(entry));
            }
        }

        let updated_series: Vec<_> = pairs.into_values().collect();
        let adjust = owner_count_for_pair_count(updated_series.len()) - old_count;
        if adjust != 0 && !sink.adjust_owner_count(adjust) {
            return Ter::TEF_INTERNAL;
        }

        let set_oracle_document_id =
            !loaded.has_oracle_document_id && sink.fix_include_keylet_fields_enabled();
        sink.update_existing_oracle(OracleSetUpdateMutation {
            updated_series,
            uri: facts.uri,
            last_update_time_secs: facts.last_update_time_secs,
            set_oracle_document_id,
            oracle_document_id: facts.oracle_document_id,
        });
        return Ter::TES_SUCCESS;
    }

    let price_data_series: Vec<_> = if !sink.fix_price_oracle_order_enabled() {
        facts.tx_series.iter().map(populated_entry).collect()
    } else {
        let mut pairs = BTreeMap::new();
        for entry in &facts.tx_series {
            pairs.insert(entry.pair.clone(), populated_entry(entry));
        }
        pairs.into_values().collect()
    };

    let owner_node = match sink.insert_owner_dir() {
        Some(owner_node) => owner_node,
        None => return Ter::TEC_DIR_FULL,
    };

    let adjust = owner_count_for_pair_count(price_data_series.len());
    if !sink.adjust_owner_count(adjust) {
        return Ter::TEF_INTERNAL;
    }

    let include_oracle_document_id = sink.fix_include_keylet_fields_enabled();
    sink.create_oracle(OracleSetCreateMutation {
        provider: facts.provider,
        asset_class: facts.asset_class,
        uri: facts.uri,
        price_data_series,
        last_update_time_secs: facts.last_update_time_secs,
        include_oracle_document_id,
        oracle_document_id: facts.oracle_document_id,
        owner_node,
    });
    Ter::TES_SUCCESS
}

#[cfg(test)]
mod tests {
    use protocol::{Ter, trans_token};

    use super::{
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
        events: Vec<String>,
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
                events: Vec::new(),
            }
        }
    }

    impl OracleSetApplySink for TestApplySink {
        fn existing_oracle(&mut self) -> Option<OracleSetLoadedOracle> {
            self.events.push("existing_oracle".to_string());
            self.existing_oracle.clone()
        }

        fn fix_include_keylet_fields_enabled(&mut self) -> bool {
            self.events.push("fix_include".to_string());
            self.fix_include_keylet_fields_enabled
        }

        fn fix_price_oracle_order_enabled(&mut self) -> bool {
            self.events.push("fix_order".to_string());
            self.fix_price_oracle_order_enabled
        }

        fn adjust_owner_count(&mut self, delta: i8) -> bool {
            self.events.push(format!("adjust:{delta}"));
            self.owner_count_adjustments.push(delta);
            self.adjust_owner_count_ok
        }

        fn update_existing_oracle(&mut self, mutation: OracleSetUpdateMutation) {
            self.events.push("update".to_string());
            self.update_mutation = Some(mutation);
        }

        fn insert_owner_dir(&mut self) -> Option<u64> {
            self.events.push("dir_insert".to_string());
            self.insert_owner_dir_result
        }

        fn create_oracle(&mut self, mutation: OracleSetCreateMutation) {
            self.events.push("create".to_string());
            self.create_mutation = Some(mutation);
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
    fn oracle_set_preflight_rejects_empty_series_before_other_checks() {
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
    fn oracle_set_preflight_rejects_oversized_series_before_field_lengths() {
        let result = run_oracle_set_preflight(OracleSetPreflightFacts {
            price_data_series_len: MAX_ORACLE_DATA_SERIES + 1,
            provider_len: Some(0),
            uri_len: None,
            asset_class_len: None,
        });

        assert_eq!(result, Ter::TEM_ARRAY_TOO_LARGE);
        assert_eq!(trans_token(result), "temARRAY_TOO_LARGE");
    }

    #[test]
    fn oracle_set_preflight_rejects_invalid_provider_length() {
        let result = run_oracle_set_preflight(OracleSetPreflightFacts {
            provider_len: Some(MAX_ORACLE_PROVIDER_LEN + 1),
            ..preflight_facts()
        });

        assert_eq!(result, Ter::TEM_MALFORMED);
        assert_eq!(trans_token(result), "temMALFORMED");
    }

    #[test]
    fn oracle_set_preflight_rejects_zero_length_uri() {
        let result = run_oracle_set_preflight(OracleSetPreflightFacts {
            uri_len: Some(0),
            ..preflight_facts()
        });

        assert_eq!(result, Ter::TEM_MALFORMED);
    }

    #[test]
    fn oracle_set_preflight_rejects_invalid_asset_class_length() {
        let result = run_oracle_set_preflight(OracleSetPreflightFacts {
            asset_class_len: Some(MAX_ORACLE_SYMBOL_CLASS_LEN + 1),
            ..preflight_facts()
        });

        assert_eq!(result, Ter::TEM_MALFORMED);
    }

    #[test]
    fn oracle_set_preflight_accepts_present_fields_at_max_length() {
        let result = run_oracle_set_preflight(OracleSetPreflightFacts {
            price_data_series_len: MAX_ORACLE_DATA_SERIES,
            provider_len: Some(MAX_ORACLE_PROVIDER_LEN),
            uri_len: Some(MAX_ORACLE_URI_LEN),
            asset_class_len: Some(MAX_ORACLE_SYMBOL_CLASS_LEN),
        });

        assert_eq!(result, Ter::TES_SUCCESS);
    }

    #[test]
    fn oracle_set_preclaim_front_rejects_missing_account_before_time_checks() {
        let result = run_oracle_set_preclaim_front(OracleSetPreclaimFrontFacts {
            account_exists: false,
            close_time_secs: 0,
            last_update_time_secs: 0,
        });

        assert_eq!(result, Ter::TER_NO_ACCOUNT);
        assert_eq!(trans_token(result), "terNO_ACCOUNT");
    }

    #[test]
    fn oracle_set_preclaim_front_rejects_last_update_before_epoch() {
        let result = run_oracle_set_preclaim_front(OracleSetPreclaimFrontFacts {
            last_update_time_secs: ORACLE_LAST_UPDATE_TIME_EPOCH_OFFSET_SECS - 1,
            ..preclaim_front_facts()
        });

        assert_eq!(result, Ter::TEC_INVALID_UPDATE_TIME);
        assert_eq!(trans_token(result), "tecINVALID_UPDATE_TIME");
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
    fn oracle_set_preclaim_front_rejects_times_below_lower_bound() {
        let result = run_oracle_set_preclaim_front(OracleSetPreclaimFrontFacts {
            last_update_time_secs: ORACLE_LAST_UPDATE_TIME_EPOCH_OFFSET_SECS + 9_699,
            ..preclaim_front_facts()
        });

        assert_eq!(result, Ter::TEC_INVALID_UPDATE_TIME);
    }

    #[test]
    fn oracle_set_preclaim_front_accepts_exact_window_bounds() {
        let lower = run_oracle_set_preclaim_front(OracleSetPreclaimFrontFacts {
            last_update_time_secs: ORACLE_LAST_UPDATE_TIME_EPOCH_OFFSET_SECS + 9_700,
            ..preclaim_front_facts()
        });
        let upper = run_oracle_set_preclaim_front(OracleSetPreclaimFrontFacts {
            last_update_time_secs: ORACLE_LAST_UPDATE_TIME_EPOCH_OFFSET_SECS + 10_300,
            ..preclaim_front_facts()
        });

        assert_eq!(lower, Ter::TES_SUCCESS);
        assert_eq!(upper, Ter::TES_SUCCESS);
    }

    #[test]
    fn oracle_set_preclaim_rejects_same_asset_pair() {
        let mut sink = TestReserveSink::new();
        let result = run_oracle_set_preclaim(
            OracleSetPreclaimFacts {
                tx_series: vec![entry("USD", "USD", Some(740), Some(1))],
                ..preclaim_facts()
            },
            &mut sink,
        );

        assert_eq!(result, Ter::TEM_MALFORMED);
        assert!(sink.seen_adjust_reserve.is_empty());
    }

    #[test]
    fn oracle_set_preclaim_rejects_duplicate_pair() {
        let mut sink = TestReserveSink::new();
        let result = run_oracle_set_preclaim(
            OracleSetPreclaimFacts {
                oracle_exists: true,
                tx_series: vec![
                    entry("XRP", "USD", Some(740), Some(1)),
                    entry("XRP", "USD", None, None),
                ],
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
    fn oracle_set_preclaim_rejects_missing_create_fields() {
        let mut sink = TestReserveSink::new();
        let missing_provider = run_oracle_set_preclaim(
            OracleSetPreclaimFacts {
                tx_provider_present: false,
                ..preclaim_facts()
            },
            &mut sink,
        );
        let missing_asset_class = run_oracle_set_preclaim(
            OracleSetPreclaimFacts {
                tx_asset_class_present: false,
                ..preclaim_facts()
            },
            &mut sink,
        );

        assert_eq!(missing_provider, Ter::TEM_MALFORMED);
        assert_eq!(missing_asset_class, Ter::TEM_MALFORMED);
    }

    #[test]
    fn oracle_set_preclaim_rejects_non_monotonic_update_time() {
        let mut sink = TestReserveSink::new();
        let result = run_oracle_set_preclaim(
            OracleSetPreclaimFacts {
                oracle_exists: true,
                previous_last_update_time_secs: ORACLE_LAST_UPDATE_TIME_EPOCH_OFFSET_SECS + 10_000,
                existing_pairs: vec![pair("XRP", "USD")],
                ..preclaim_facts()
            },
            &mut sink,
        );

        assert_eq!(result, Ter::TEC_INVALID_UPDATE_TIME);
    }

    #[test]
    fn oracle_set_preclaim_rejects_missing_delete_target() {
        let mut sink = TestReserveSink::new();
        let result = run_oracle_set_preclaim(
            OracleSetPreclaimFacts {
                oracle_exists: true,
                tx_series: vec![entry("XRP", "EUR", None, None)],
                existing_pairs: vec![pair("XRP", "USD")],
                ..preclaim_facts()
            },
            &mut sink,
        );

        assert_eq!(result, Ter::TEC_TOKEN_PAIR_NOT_FOUND);
    }

    #[test]
    fn oracle_set_preclaim_rejects_empty_resulting_pairs() {
        let mut sink = TestReserveSink::new();
        let result = run_oracle_set_preclaim(
            OracleSetPreclaimFacts {
                oracle_exists: true,
                tx_series: vec![entry("XRP", "USD", None, None)],
                existing_pairs: vec![pair("XRP", "USD")],
                ..preclaim_facts()
            },
            &mut sink,
        );

        assert_eq!(result, Ter::TEC_ARRAY_EMPTY);
    }

    #[test]
    fn oracle_set_preclaim_rejects_too_many_resulting_pairs() {
        let mut sink = TestReserveSink::new();
        let result = run_oracle_set_preclaim(
            OracleSetPreclaimFacts {
                oracle_exists: true,
                tx_series: vec![
                    entry("XRP", "US1", Some(740), Some(1)),
                    entry("XRP", "US2", Some(750), Some(1)),
                    entry("XRP", "US3", Some(740), Some(1)),
                    entry("XRP", "US4", Some(750), Some(1)),
                    entry("XRP", "US5", Some(740), Some(1)),
                    entry("XRP", "US6", Some(750), Some(1)),
                    entry("XRP", "US7", Some(740), Some(1)),
                    entry("XRP", "US8", Some(750), Some(1)),
                    entry("XRP", "US9", Some(740), Some(1)),
                    entry("XRP", "U10", Some(750), Some(1)),
                ],
                existing_pairs: vec![pair("XRP", "USD")],
                ..preclaim_facts()
            },
            &mut sink,
        );

        assert_eq!(result, Ter::TEC_ARRAY_TOO_LARGE);
    }

    #[test]
    fn oracle_set_preclaim_maps_insufficient_reserve() {
        let mut sink = TestReserveSink::new();
        sink.reserve_ok = false;

        let result = run_oracle_set_preclaim(preclaim_facts(), &mut sink);

        assert_eq!(result, Ter::TEC_INSUFFICIENT_RESERVE);
        assert_eq!(sink.seen_adjust_reserve, vec![1]);
    }

    #[test]
    fn oracle_set_preclaim_preserves_update_merge_and_owner_count_delta() {
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
        assert_eq!(
            sink.events,
            [
                "existing_oracle",
                "fix_order",
                "dir_insert",
                "adjust:1",
                "fix_include",
                "create"
            ]
        );
        assert_eq!(sink.owner_count_adjustments, vec![1]);
        let create = sink.create_mutation.expect("create mutation");
        assert_eq!(create.price_data_series[0].pair, pair("XRP", "USD"));
        assert_eq!(create.price_data_series[1].pair, pair("XRP", "EUR"));
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
        assert_eq!(create.oracle_document_id, 7);
    }

    #[test]
    fn oracle_set_do_apply_create_maps_dir_insert_failure() {
        let mut sink = TestApplySink::new();
        sink.insert_owner_dir_result = None;

        let result = run_oracle_set_do_apply(apply_facts(), &mut sink);

        assert_eq!(result, Ter::TEC_DIR_FULL);
        assert_eq!(sink.events, ["existing_oracle", "fix_order", "dir_insert"]);
        assert!(sink.create_mutation.is_none());
    }

    #[test]
    fn oracle_set_do_apply_create_maps_owner_count_failure() {
        let mut sink = TestApplySink::new();
        sink.adjust_owner_count_ok = false;

        let result = run_oracle_set_do_apply(apply_facts(), &mut sink);

        assert_eq!(result, Ter::TEF_INTERNAL);
        assert_eq!(
            sink.events,
            ["existing_oracle", "fix_order", "dir_insert", "adjust:1"]
        );
        assert!(sink.create_mutation.is_none());
    }

    #[test]
    fn oracle_set_do_apply_update_merges_pairs_and_sets_fields() {
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
        assert_eq!(
            sink.events,
            ["existing_oracle", "adjust:-1", "fix_include", "update"]
        );
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
        assert_eq!(sink.events, ["existing_oracle", "adjust:-1"]);
        assert!(sink.update_mutation.is_none());
    }

    #[test]
    fn oracle_set_do_apply_update_skips_adjust_when_pair_band_unchanged() {
        let mut sink = TestApplySink::new();
        sink.existing_oracle = Some(OracleSetLoadedOracle {
            has_oracle_document_id: true,
            price_data_series: vec![entry("XRP", "USD", Some(741), Some(2))],
        });

        let result = run_oracle_set_do_apply(
            OracleSetApplyFacts {
                tx_series: vec![entry("XRP", "USD", Some(742), Some(3))],
                ..apply_facts()
            },
            &mut sink,
        );

        assert_eq!(result, Ter::TES_SUCCESS);
        assert_eq!(sink.events, ["existing_oracle", "update"]);
        assert!(sink.owner_count_adjustments.is_empty());
    }
}
