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

fn format_human_load_factor(factor: u32, base: u32) -> String {
    format_human_ratio(u64::from(factor), u64::from(base))
}

pub(crate) fn format_human_ratio(numerator: u64, denominator: u64) -> String {
    if denominator == 0 {
        return "0".to_owned();
    }

    let whole = numerator / denominator;
    let remainder = numerator % denominator;
    if remainder == 0 {
        return whole.to_string();
    }

    let fractional = ((remainder as u128) * 1_000_000) / (denominator as u128);
    let mut formatted = format!("{whole}.{fractional:06}");
    while formatted.ends_with('0') {
        formatted.pop();
    }
    formatted
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
            JsonValue::String(format_human_ratio(load_factor, load_base_u64)),
        );

        if load_factor_server != load_factor {
            info.insert(
                "load_factor_server".to_owned(),
                JsonValue::String(format_human_ratio(load_factor_server, load_base_u64)),
            );
        }

        if admin {
            let local = load_fee_track.local_fee();
            if local != load_base {
                info.insert(
                    "load_factor_local".to_owned(),
                    JsonValue::String(format_human_load_factor(local, load_base)),
                );
            }

            let remote = load_fee_track.remote_fee();
            if remote != load_base {
                info.insert(
                    "load_factor_net".to_owned(),
                    JsonValue::String(format_human_load_factor(remote, load_base)),
                );
            }

            let cluster = load_fee_track.cluster_fee();
            if cluster != load_base {
                info.insert(
                    "load_factor_cluster".to_owned(),
                    JsonValue::String(format_human_load_factor(cluster, load_base)),
                );
            }
        }

        if let Some(queue_load) = queue_load {
            if queue_load.open_ledger_level != queue_load.reference_level
                && (admin || queue_load.escalated_load_factor != load_factor)
            {
                info.insert(
                    "load_factor_fee_escalation".to_owned(),
                    JsonValue::String(format_human_ratio(
                        queue_load.open_ledger_level,
                        queue_load.reference_level,
                    )),
                );
            }

            if queue_load.minimum_level != queue_load.reference_level {
                info.insert(
                    "load_factor_fee_queue".to_owned(),
                    JsonValue::String(format_human_ratio(
                        queue_load.minimum_level,
                        queue_load.reference_level,
                    )),
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
    use super::format_human_ratio;

    #[test]
    fn format_human_ratio_trims_fractional_zeros() {
        assert_eq!(format_human_ratio(256, 256), "1");
        assert_eq!(format_human_ratio(768, 256), "3");
        assert_eq!(format_human_ratio(1250, 1000), "1.25");
    }
}
