//! `get_aggregate_price` RPC handler — full reference the reference source parity.
//!
//! Aggregates oracle price data across multiple oracle objects for a given
//! base/quote asset pair. Computes mean, median, standard deviation, and
//! optionally trims outliers.

use std::collections::BTreeMap;

use protocol::{JsonValue, STAmount, STObject, get_field_by_symbol};

use crate::commands::rpc_helpers::rpc_error;
use crate::status::RpcErrorCode;

/// Constants matching reference the reference source
const MAX_ORACLES: usize = 200;
const MAX_TRIM: u32 = 25;
const MAX_HISTORY: usize = 20;

/// A collected price entry: (lastUpdateTime, price as f64)
#[derive(Debug, Clone)]
struct PriceEntry {
    last_update_time: u32,
    price: f64,
}

/// Trait for ledger access — allows testing without a real ledger.
pub trait AggregatePriceSource {
    /// Read an Oracle SLE by account + document_id.
    /// Returns the PriceDataSeries array and LastUpdateTime if found.
    fn read_oracle(&self, account: &str, document_id: u32) -> Option<OracleData>;

    /// Read historical oracle prices by walking the transaction chain.
    /// Returns a vector of (last_update_time, price_data_series) for each
    /// historical state of the oracle, up to `max_history` entries.
    ///
    /// `AffectedNodes` → finds Oracle `FinalFields`/`NewFields`.
    fn read_oracle_history(
        &self,
        account: &str,
        document_id: u32,
        max_history: usize,
    ) -> Vec<OracleData> {
        // Default: no history available (single-ledger source)
        let _ = (account, document_id, max_history);
        Vec::new()
    }
}

/// Data extracted from an Oracle SLE.
#[derive(Debug, Clone)]
pub struct OracleData {
    pub last_update_time: u32,
    pub price_data_series: Vec<PriceDataEntry>,
}

/// A single entry in the PriceDataSeries array.
#[derive(Debug, Clone)]
pub struct PriceDataEntry {
    pub base_asset: String,
    pub quote_asset: String,
    pub asset_price: Option<u64>,
    pub scale: u8,
}

/// Compute mean, standard deviation, and size for a slice of prices.
fn compute_stats(prices: &[f64]) -> (f64, f64, u16) {
    let size = prices.len() as u16;
    if size == 0 {
        return (0.0, 0.0, 0);
    }
    let mean: f64 = prices.iter().sum::<f64>() / size as f64;
    let sd = if size > 1 {
        let variance: f64 =
            prices.iter().map(|p| (p - mean) * (p - mean)).sum::<f64>() / (size as f64 - 1.0);
        variance.sqrt()
    } else {
        0.0
    };
    (mean, sd, size)
}

/// Compute median of a sorted slice.
fn compute_median(sorted_prices: &[f64]) -> f64 {
    let n = sorted_prices.len();
    if n == 0 {
        return 0.0;
    }
    if n.is_multiple_of(2) {
        (sorted_prices[n / 2 - 1] + sorted_prices[n / 2]) / 2.0
    } else {
        sorted_prices[n / 2]
    }
}

/// Format a price as a string matching reference STAmount::getText() for oracle prices.
/// Uses scientific-ish notation: integer part with appropriate scaling.
fn format_price(price: f64) -> String {
    if price == 0.0 {
        return "0".to_string();
    }
    // Match reference behavior: output as decimal string
    let s = format!("{}", price);
    // Remove trailing zeros after decimal point
    if s.contains('.') {
        let trimmed = s.trim_end_matches('0').trim_end_matches('.');
        trimmed.to_string()
    } else {
        s
    }
}

/// Execute the get_aggregate_price RPC command.
///
/// Parameters:
/// - `oracles`: array of {account, oracle_document_id}
/// - `base_asset`: currency code to price
/// - `quote_asset`: denomination currency
/// - `trim`: optional percentage of outliers to trim (1-25)
/// - `time_threshold`: optional max age in seconds
pub fn do_get_aggregate_price<S: AggregatePriceSource>(
    params: &JsonValue,
    source: &S,
) -> JsonValue {
    tracing::trace!(target: "rpc", method = "get_aggregate_price", "get_aggregate_price query");
    let JsonValue::Object(obj) = params else {
        return rpc_error(RpcErrorCode::InvalidParams);
    };

    // Validate required fields
    let Some(JsonValue::Array(oracles)) = obj.get("oracles") else {
        return missing_field_error("oracles");
    };
    if oracles.is_empty() || oracles.len() > MAX_ORACLES {
        return oracle_malformed_error();
    }

    let Some(JsonValue::String(base_asset)) = obj.get("base_asset") else {
        return missing_field_error("base_asset");
    };
    if base_asset.is_empty() {
        return rpc_error(RpcErrorCode::InvalidParams);
    }

    let Some(JsonValue::String(quote_asset)) = obj.get("quote_asset") else {
        return missing_field_error("quote_asset");
    };
    if quote_asset.is_empty() {
        return rpc_error(RpcErrorCode::InvalidParams);
    }

    // Parse optional trim
    let trim: u32 = if let Some(trim_val) = obj.get("trim") {
        match trim_val {
            JsonValue::Unsigned(v) => *v as u32,
            JsonValue::String(s) => s.parse::<u32>().unwrap_or(0),
            _ => return rpc_error(RpcErrorCode::InvalidParams),
        }
    } else {
        0
    };
    if obj.contains_key("trim") && (trim == 0 || trim > MAX_TRIM) {
        return rpc_error(RpcErrorCode::InvalidParams);
    }

    // Parse optional time_threshold
    let time_threshold: u32 = if let Some(tt_val) = obj.get("time_threshold") {
        match tt_val {
            JsonValue::Unsigned(v) => *v as u32,
            JsonValue::String(s) => s.parse::<u32>().unwrap_or(0),
            _ => return rpc_error(RpcErrorCode::InvalidParams),
        }
    } else {
        0
    };

    // Collect prices from all oracles
    let mut entries: Vec<PriceEntry> = Vec::new();

    for oracle_param in oracles {
        let JsonValue::Object(oracle_obj) = oracle_param else {
            return oracle_malformed_error();
        };
        let Some(JsonValue::String(account)) = oracle_obj.get("account") else {
            return oracle_malformed_error();
        };
        let document_id = match oracle_obj.get("oracle_document_id") {
            Some(JsonValue::Unsigned(v)) => *v as u32,
            Some(JsonValue::String(s)) => match s.parse::<u32>() {
                Ok(v) => v,
                Err(_) => return rpc_error(RpcErrorCode::InvalidParams),
            },
            _ => return oracle_malformed_error(),
        };

        // Read oracle data from ledger (current state)
        if let Some(oracle_data) = source.read_oracle(account, document_id) {
            // Find matching base/quote pair
            for entry in &oracle_data.price_data_series {
                if entry.base_asset == *base_asset
                    && entry.quote_asset == *quote_asset
                    && entry.asset_price.is_some()
                {
                    let price = entry.asset_price.unwrap();
                    let scale = entry.scale as i32;
                    // Convert to f64: price * 10^(-scale)
                    let price_f64 = price as f64 * 10f64.powi(-scale);
                    entries.push(PriceEntry {
                        last_update_time: oracle_data.last_update_time,
                        price: price_f64,
                    });
                    break; // Only take first matching pair per oracle
                }
            }
        }

        // Read historical oracle data via transaction chain (compatibility:
        // walks PreviousTxnID → AffectedNodes → FinalFields/NewFields)
        let history = source.read_oracle_history(account, document_id, MAX_HISTORY);
        for hist_data in &history {
            for entry in &hist_data.price_data_series {
                if entry.base_asset == *base_asset
                    && entry.quote_asset == *quote_asset
                    && entry.asset_price.is_some()
                {
                    let price = entry.asset_price.unwrap();
                    let scale = entry.scale as i32;
                    let price_f64 = price as f64 * 10f64.powi(-scale);
                    entries.push(PriceEntry {
                        last_update_time: hist_data.last_update_time,
                        price: price_f64,
                    });
                    break;
                }
            }
        }
    }

    if entries.is_empty() {
        return object_not_found_error();
    }

    // Apply time_threshold filter
    let latest_time = entries
        .iter()
        .map(|e| e.last_update_time)
        .max()
        .unwrap_or(0);
    if time_threshold > 0 {
        let cutoff = latest_time.saturating_sub(time_threshold);
        entries.retain(|e| e.last_update_time >= cutoff);
    }

    if entries.is_empty() {
        return rpc_error(RpcErrorCode::Internal);
    }

    // Sort prices ascending for statistics
    let mut sorted_prices: Vec<f64> = entries.iter().map(|e| e.price).collect();
    sorted_prices.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));

    // Compute entire_set statistics
    let (mean, sd, size) = compute_stats(&sorted_prices);
    let median = compute_median(&sorted_prices);

    // Build result
    let mut result = BTreeMap::new();

    // entire_set
    let mut entire_set = BTreeMap::new();
    entire_set.insert("mean".to_string(), JsonValue::String(format_price(mean)));
    entire_set.insert("size".to_string(), JsonValue::Unsigned(size as u64));
    entire_set.insert(
        "standard_deviation".to_string(),
        JsonValue::String(format_price(sd)),
    );
    result.insert("entire_set".to_string(), JsonValue::Object(entire_set));

    // median
    result.insert(
        "median".to_string(),
        JsonValue::String(format_price(median)),
    );

    // time
    result.insert("time".to_string(), JsonValue::Unsigned(latest_time as u64));

    // trimmed_set (if trim specified)
    if trim > 0 {
        let trim_count = sorted_prices.len() * trim as usize / 100;
        let trimmed = &sorted_prices[trim_count..sorted_prices.len() - trim_count];
        let (t_mean, t_sd, t_size) = compute_stats(trimmed);

        let mut trimmed_set = BTreeMap::new();
        trimmed_set.insert("mean".to_string(), JsonValue::String(format_price(t_mean)));
        trimmed_set.insert("size".to_string(), JsonValue::Unsigned(t_size as u64));
        trimmed_set.insert(
            "standard_deviation".to_string(),
            JsonValue::String(format_price(t_sd)),
        );
        result.insert("trimmed_set".to_string(), JsonValue::Object(trimmed_set));
    }

    result.insert(
        "status".to_string(),
        JsonValue::String("success".to_string()),
    );
    JsonValue::Object(result)
}

fn missing_field_error(field: &str) -> JsonValue {
    let mut result = BTreeMap::new();
    result.insert(
        "error".to_string(),
        JsonValue::String("invalidParams".to_string()),
    );
    result.insert(
        "error_message".to_string(),
        JsonValue::String(format!("Missing field '{}'.", field)),
    );
    JsonValue::Object(result)
}

fn oracle_malformed_error() -> JsonValue {
    let mut result = BTreeMap::new();
    result.insert(
        "error".to_string(),
        JsonValue::String("oracleMalformed".to_string()),
    );
    result.insert(
        "error_message".to_string(),
        JsonValue::String("Oracle request is malformed.".to_string()),
    );
    JsonValue::Object(result)
}

fn object_not_found_error() -> JsonValue {
    let mut result = BTreeMap::new();
    result.insert(
        "error".to_string(),
        JsonValue::String("objectNotFound".to_string()),
    );
    result.insert(
        "error_message".to_string(),
        JsonValue::String("The requested object was not found.".to_string()),
    );
    JsonValue::Object(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct MockSource {
        oracles: BTreeMap<(String, u32), OracleData>,
    }

    impl AggregatePriceSource for MockSource {
        fn read_oracle(&self, account: &str, document_id: u32) -> Option<OracleData> {
            self.oracles
                .get(&(account.to_string(), document_id))
                .cloned()
        }
    }

    fn make_source(data: Vec<(&str, u32, u32, &str, &str, u64, u8)>) -> MockSource {
        let mut oracles = BTreeMap::new();
        for (account, doc_id, time, base, quote, price, scale) in data {
            oracles.insert(
                (account.to_string(), doc_id),
                OracleData {
                    last_update_time: time,
                    price_data_series: vec![PriceDataEntry {
                        base_asset: base.to_string(),
                        quote_asset: quote.to_string(),
                        asset_price: Some(price),
                        scale,
                    }],
                },
            );
        }
        MockSource { oracles }
    }

    fn make_params(oracles: Vec<(&str, u32)>, base: &str, quote: &str) -> JsonValue {
        let mut obj = BTreeMap::new();
        let oracle_arr: Vec<JsonValue> = oracles
            .into_iter()
            .map(|(acct, doc_id)| {
                let mut o = BTreeMap::new();
                o.insert("account".to_string(), JsonValue::String(acct.to_string()));
                o.insert(
                    "oracle_document_id".to_string(),
                    JsonValue::Unsigned(doc_id as u64),
                );
                JsonValue::Object(o)
            })
            .collect();
        obj.insert("oracles".to_string(), JsonValue::Array(oracle_arr));
        obj.insert(
            "base_asset".to_string(),
            JsonValue::String(base.to_string()),
        );
        obj.insert(
            "quote_asset".to_string(),
            JsonValue::String(quote.to_string()),
        );
        JsonValue::Object(obj)
    }

    #[test]
    fn test_single_oracle() {
        let source = make_source(vec![("rABC", 1, 1000, "XRP", "USD", 50000, 4)]);
        let params = make_params(vec![("rABC", 1)], "XRP", "USD");
        let result = do_get_aggregate_price(&params, &source);
        let JsonValue::Object(r) = &result else {
            panic!()
        };
        assert_eq!(
            r.get("status"),
            Some(&JsonValue::String("success".to_string()))
        );
        assert_eq!(r.get("time"), Some(&JsonValue::Unsigned(1000)));
        // price = 50000 * 10^-4 = 5.0
        let JsonValue::Object(es) = r.get("entire_set").unwrap() else {
            panic!()
        };
        assert_eq!(es.get("mean"), Some(&JsonValue::String("5".to_string())));
        assert_eq!(es.get("size"), Some(&JsonValue::Unsigned(1)));
    }

    #[test]
    fn test_multiple_oracles() {
        let source = make_source(vec![
            ("rA", 1, 1000, "XRP", "USD", 50000, 4),
            ("rB", 1, 1000, "XRP", "USD", 60000, 4),
            ("rC", 1, 1000, "XRP", "USD", 70000, 4),
        ]);
        let params = make_params(vec![("rA", 1), ("rB", 1), ("rC", 1)], "XRP", "USD");
        let result = do_get_aggregate_price(&params, &source);
        let JsonValue::Object(r) = &result else {
            panic!()
        };
        // prices: 5, 6, 7 → mean=6, median=6
        let JsonValue::Object(es) = r.get("entire_set").unwrap() else {
            panic!()
        };
        assert_eq!(es.get("mean"), Some(&JsonValue::String("6".to_string())));
        assert_eq!(es.get("size"), Some(&JsonValue::Unsigned(3)));
        assert_eq!(r.get("median"), Some(&JsonValue::String("6".to_string())));
    }

    #[test]
    fn test_missing_oracles_field() {
        let params = JsonValue::Object(BTreeMap::new());
        let source = make_source(vec![]);
        let result = do_get_aggregate_price(&params, &source);
        let JsonValue::Object(r) = &result else {
            panic!()
        };
        assert!(r.contains_key("error"));
    }

    #[test]
    fn test_empty_oracles() {
        let mut obj = BTreeMap::new();
        obj.insert("oracles".to_string(), JsonValue::Array(vec![]));
        obj.insert(
            "base_asset".to_string(),
            JsonValue::String("XRP".to_string()),
        );
        obj.insert(
            "quote_asset".to_string(),
            JsonValue::String("USD".to_string()),
        );
        let params = JsonValue::Object(obj);
        let source = make_source(vec![]);
        let result = do_get_aggregate_price(&params, &source);
        let JsonValue::Object(r) = &result else {
            panic!()
        };
        assert_eq!(
            r.get("error"),
            Some(&JsonValue::String("oracleMalformed".to_string()))
        );
    }

    #[test]
    fn test_no_matching_prices() {
        let source = make_source(vec![("rA", 1, 1000, "BTC", "USD", 50000, 2)]);
        let params = make_params(vec![("rA", 1)], "XRP", "USD");
        let result = do_get_aggregate_price(&params, &source);
        let JsonValue::Object(r) = &result else {
            panic!()
        };
        assert_eq!(
            r.get("error"),
            Some(&JsonValue::String("objectNotFound".to_string()))
        );
    }

    #[test]
    fn test_time_threshold() {
        let source = make_source(vec![
            ("rA", 1, 1000, "XRP", "USD", 50000, 4),
            ("rB", 1, 500, "XRP", "USD", 60000, 4),
            ("rC", 1, 100, "XRP", "USD", 70000, 4),
        ]);
        let mut obj = BTreeMap::new();
        let oracle_arr: Vec<JsonValue> = vec![("rA", 1), ("rB", 1), ("rC", 1)]
            .into_iter()
            .map(|(acct, doc_id)| {
                let mut o = BTreeMap::new();
                o.insert("account".to_string(), JsonValue::String(acct.to_string()));
                o.insert(
                    "oracle_document_id".to_string(),
                    JsonValue::Unsigned(doc_id),
                );
                JsonValue::Object(o)
            })
            .collect();
        obj.insert("oracles".to_string(), JsonValue::Array(oracle_arr));
        obj.insert(
            "base_asset".to_string(),
            JsonValue::String("XRP".to_string()),
        );
        obj.insert(
            "quote_asset".to_string(),
            JsonValue::String("USD".to_string()),
        );
        obj.insert("time_threshold".to_string(), JsonValue::Unsigned(600));
        let params = JsonValue::Object(obj);
        let result = do_get_aggregate_price(&params, &source);
        let JsonValue::Object(r) = &result else {
            panic!()
        };
        // Only rA (time=1000) and rB (time=500) should remain (cutoff = 1000-600 = 400)
        let JsonValue::Object(es) = r.get("entire_set").unwrap() else {
            panic!()
        };
        assert_eq!(es.get("size"), Some(&JsonValue::Unsigned(2)));
    }

    #[test]
    fn test_trim() {
        let source = make_source(vec![
            ("r1", 1, 1000, "XRP", "USD", 10000, 4),
            ("r2", 1, 1000, "XRP", "USD", 20000, 4),
            ("r3", 1, 1000, "XRP", "USD", 30000, 4),
            ("r4", 1, 1000, "XRP", "USD", 40000, 4),
            ("r5", 1, 1000, "XRP", "USD", 50000, 4),
            ("r6", 1, 1000, "XRP", "USD", 60000, 4),
            ("r7", 1, 1000, "XRP", "USD", 70000, 4),
            ("r8", 1, 1000, "XRP", "USD", 80000, 4),
            ("r9", 1, 1000, "XRP", "USD", 90000, 4),
            ("rA", 1, 1000, "XRP", "USD", 100000, 4),
        ]);
        let mut obj = BTreeMap::new();
        let oracle_arr: Vec<JsonValue> = (1..=9)
            .map(|i| format!("r{}", i))
            .chain(std::iter::once("rA".to_string()))
            .map(|acct| {
                let mut o = BTreeMap::new();
                o.insert("account".to_string(), JsonValue::String(acct));
                o.insert("oracle_document_id".to_string(), JsonValue::Unsigned(1));
                JsonValue::Object(o)
            })
            .collect();
        obj.insert("oracles".to_string(), JsonValue::Array(oracle_arr));
        obj.insert(
            "base_asset".to_string(),
            JsonValue::String("XRP".to_string()),
        );
        obj.insert(
            "quote_asset".to_string(),
            JsonValue::String("USD".to_string()),
        );
        obj.insert("trim".to_string(), JsonValue::Unsigned(20));
        let params = JsonValue::Object(obj);
        let result = do_get_aggregate_price(&params, &source);
        let JsonValue::Object(r) = &result else {
            panic!()
        };
        // 10 prices, trim 20% = 2 from each end → 6 remaining
        assert!(r.contains_key("trimmed_set"));
        let JsonValue::Object(ts) = r.get("trimmed_set").unwrap() else {
            panic!()
        };
        assert_eq!(ts.get("size"), Some(&JsonValue::Unsigned(6)));
    }

    #[test]
    fn test_trim_invalid_zero() {
        let mut obj = BTreeMap::new();
        obj.insert(
            "oracles".to_string(),
            JsonValue::Array(vec![{
                let mut o = BTreeMap::new();
                o.insert("account".to_string(), JsonValue::String("rA".to_string()));
                o.insert("oracle_document_id".to_string(), JsonValue::Unsigned(1));
                JsonValue::Object(o)
            }]),
        );
        obj.insert(
            "base_asset".to_string(),
            JsonValue::String("XRP".to_string()),
        );
        obj.insert(
            "quote_asset".to_string(),
            JsonValue::String("USD".to_string()),
        );
        obj.insert("trim".to_string(), JsonValue::Unsigned(0));
        let params = JsonValue::Object(obj);
        let source = make_source(vec![]);
        let result = do_get_aggregate_price(&params, &source);
        let JsonValue::Object(r) = &result else {
            panic!()
        };
        assert!(r.contains_key("error"));
    }

    #[test]
    fn test_trim_over_max() {
        let mut obj = BTreeMap::new();
        obj.insert(
            "oracles".to_string(),
            JsonValue::Array(vec![{
                let mut o = BTreeMap::new();
                o.insert("account".to_string(), JsonValue::String("rA".to_string()));
                o.insert("oracle_document_id".to_string(), JsonValue::Unsigned(1));
                JsonValue::Object(o)
            }]),
        );
        obj.insert(
            "base_asset".to_string(),
            JsonValue::String("XRP".to_string()),
        );
        obj.insert(
            "quote_asset".to_string(),
            JsonValue::String("USD".to_string()),
        );
        obj.insert("trim".to_string(), JsonValue::Unsigned(26));
        let params = JsonValue::Object(obj);
        let source = make_source(vec![]);
        let result = do_get_aggregate_price(&params, &source);
        let JsonValue::Object(r) = &result else {
            panic!()
        };
        assert!(r.contains_key("error"));
    }

    #[test]
    fn test_median_even() {
        let source = make_source(vec![
            ("rA", 1, 1000, "XRP", "USD", 40000, 4),
            ("rB", 1, 1000, "XRP", "USD", 60000, 4),
        ]);
        let params = make_params(vec![("rA", 1), ("rB", 1)], "XRP", "USD");
        let result = do_get_aggregate_price(&params, &source);
        let JsonValue::Object(r) = &result else {
            panic!()
        };
        // prices: 4, 6 → median = 5
        assert_eq!(r.get("median"), Some(&JsonValue::String("5".to_string())));
    }

    #[test]
    fn test_median_odd() {
        let source = make_source(vec![
            ("rA", 1, 1000, "XRP", "USD", 30000, 4),
            ("rB", 1, 1000, "XRP", "USD", 50000, 4),
            ("rC", 1, 1000, "XRP", "USD", 70000, 4),
        ]);
        let params = make_params(vec![("rA", 1), ("rB", 1), ("rC", 1)], "XRP", "USD");
        let result = do_get_aggregate_price(&params, &source);
        let JsonValue::Object(r) = &result else {
            panic!()
        };
        // prices: 3, 5, 7 → median = 5
        assert_eq!(r.get("median"), Some(&JsonValue::String("5".to_string())));
    }
}
