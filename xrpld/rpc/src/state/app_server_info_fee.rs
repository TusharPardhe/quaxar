use std::collections::BTreeMap;

use app::StatusRpcSnapshot;
use protocol::JsonValue;

use crate::state::app_server_info_source::AppServerInfoView;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct QueueLoadStatus {
    reference_level: u64,
    minimum_level: u64,
    open_ledger_level: u64,
    escalated_load_factor: u64,
}

/// rippled `json::valueToString(double)`: `std::to_chars` general format
/// with 16 significant digits (`%.16g`): trailing zeros dropped, scientific
/// notation below 1e-4 or from 1e16, two-digit exponent (`1e-05`).
pub(crate) fn format_json_double(value: f64) -> String {
    if !value.is_finite() || value == 0.0 {
        return "0".to_owned();
    }
    let scientific = format!("{value:.15e}");
    let (mantissa, exponent) = scientific
        .split_once('e')
        .expect("`{:e}` output has an exponent");
    let exponent: i32 = exponent.parse().expect("`{:e}` exponent is an integer");
    let trim = |text: &str| -> String {
        if text.contains('.') {
            text.trim_end_matches('0').trim_end_matches('.').to_owned()
        } else {
            text.to_owned()
        }
    };
    if !(-4..16).contains(&exponent) {
        let sign = if exponent < 0 { '-' } else { '+' };
        return format!("{}e{sign}{:02}", trim(mantissa), exponent.unsigned_abs());
    }
    let precision = usize::try_from(15 - exponent).unwrap_or(0);
    trim(&format!("{value:.precision$}"))
}

/// A JSON number rendered as rippled renders a `double`.
pub(crate) fn json_double(value: f64) -> JsonValue {
    JsonValue::Raw(protocol::RawJson::from_trusted(format_json_double(value)))
}

/// rippled `static_cast<double>(numerator) / denominator` as a JSON number.
pub(crate) fn json_ratio(numerator: u64, denominator: u64) -> JsonValue {
    if denominator == 0 {
        return json_double(0.0);
    }
    json_double(numerator as f64 / denominator as f64)
}

fn parse_u64_string(value: &str) -> Option<u64> {
    value.parse().ok()
}

fn queue_load_status(report: &tx::QueueTxQRpcReport, load_base: u32) -> Option<QueueLoadStatus> {
    let reference_level = parse_u64_string(&report.levels.reference_level)?;
    let minimum_level = parse_u64_string(&report.levels.minimum_level)?;
    let open_ledger_level = parse_u64_string(&report.levels.open_ledger_level)?;
    if reference_level == 0 {
        return None;
    }

    let escalated_load_factor = ((u128::from(open_ledger_level) * u128::from(load_base))
        / u128::from(reference_level))
    .min(u128::from(u64::MAX)) as u64;

    Some(QueueLoadStatus {
        reference_level,
        minimum_level,
        open_ledger_level,
        escalated_load_factor,
    })
}

pub(crate) fn append_load_factor_fields<V: AppServerInfoView>(
    info: &mut BTreeMap<String, JsonValue>,
    snapshot: &StatusRpcSnapshot,
    view: &V,
    human: bool,
    admin: bool,
) {
    let load_fee_track = view.load_fee_track();
    let load_base = load_fee_track.load_base();
    let load_base_u64 = u64::from(load_base);
    let load_factor_server = u64::from(load_fee_track.load_factor());
    let queue_load = snapshot
        .queue_report
        .as_ref()
        .and_then(|report| queue_load_status(report, load_base));
    let load_factor = queue_load.map_or(load_factor_server, |queue| {
        load_factor_server.max(queue.escalated_load_factor)
    });

    if human {
        info.insert(
            "load_factor".to_owned(),
            json_ratio(load_factor, load_base_u64),
        );

        if load_factor_server != load_factor {
            info.insert(
                "load_factor_server".to_owned(),
                json_ratio(load_factor_server, load_base_u64),
            );
        }

        if admin {
            let local = load_fee_track.local_fee();
            if local != load_base {
                info.insert(
                    "load_factor_local".to_owned(),
                    json_ratio(u64::from(local), load_base_u64),
                );
            }

            let remote = load_fee_track.remote_fee();
            if remote != load_base {
                info.insert(
                    "load_factor_net".to_owned(),
                    json_ratio(u64::from(remote), load_base_u64),
                );
            }

            let cluster = load_fee_track.cluster_fee();
            if cluster != load_base {
                info.insert(
                    "load_factor_cluster".to_owned(),
                    json_ratio(u64::from(cluster), load_base_u64),
                );
            }
        }

        if let Some(queue_load) = queue_load {
            if queue_load.open_ledger_level != queue_load.reference_level
                && (admin || queue_load.escalated_load_factor != load_factor)
            {
                info.insert(
                    "load_factor_fee_escalation".to_owned(),
                    json_ratio(queue_load.open_ledger_level, queue_load.reference_level),
                );
            }

            if queue_load.minimum_level != queue_load.reference_level {
                info.insert(
                    "load_factor_fee_queue".to_owned(),
                    json_ratio(queue_load.minimum_level, queue_load.reference_level),
                );
            }
        }
    } else {
        info.insert(
            "load_base".to_owned(),
            JsonValue::Unsigned(load_base.into()),
        );
        info.insert("load_factor".to_owned(), JsonValue::Unsigned(load_factor));
        info.insert(
            "load_factor_server".to_owned(),
            JsonValue::Unsigned(load_factor_server),
        );

        if let Some(queue_load) = queue_load {
            info.insert(
                "load_factor_fee_escalation".to_owned(),
                JsonValue::Unsigned(queue_load.open_ledger_level),
            );
            info.insert(
                "load_factor_fee_queue".to_owned(),
                JsonValue::Unsigned(queue_load.minimum_level),
            );
            info.insert(
                "load_factor_fee_reference".to_owned(),
                JsonValue::Unsigned(queue_load.reference_level),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::format_json_double;

    /// rippled renders server_info's human-mode ratios and XRP amounts as
    /// JSON numbers (`double`, `%.16g`). Quaxar used to send strings such as
    /// `"1"` and `"0.00001"` (mainnet, 2026-11-24).
    #[test]
    fn json_doubles_match_rippled_formatting() {
        assert_eq!(format_json_double(256.0 / 256.0), "1");
        assert_eq!(format_json_double(768.0 / 256.0), "3");
        assert_eq!(format_json_double(1250.0 / 1000.0), "1.25");
        assert_eq!(format_json_double(10.0 / 1_000_000.0), "1e-05");
        assert_eq!(format_json_double(200_000.0 / 1_000_000.0), "0.2");
        assert_eq!(format_json_double(3001.0 / 1000.0), "3.001");
        assert_eq!(format_json_double(1.0 / 3.0), "0.3333333333333333");
        assert_eq!(format_json_double(0.0001), "0.0001");
        assert_eq!(format_json_double(1e16), "1e+16");
        assert_eq!(format_json_double(0.0), "0");
    }
}
