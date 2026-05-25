use std::collections::BTreeMap;

use app::{StatusRpcLastClose, StatusRpcSnapshot};
use protocol::JsonValue;

use crate::state::app_server_info_fee::format_human_ratio;
use crate::state::app_server_info_source::AppServerInfoView;

fn duration_millis_u64(duration: std::time::Duration) -> u64 {
    duration.as_millis().min(u128::from(u64::MAX)) as u64
}

fn append_last_close(
    info: &mut BTreeMap<String, JsonValue>,
    last_close: StatusRpcLastClose,
    human: bool,
) {
    let mut value = BTreeMap::from([(
        "proposers".to_owned(),
        JsonValue::Unsigned(u64::from(last_close.proposers)),
    )]);
    let converge_millis = duration_millis_u64(last_close.converge_time);
    if human {
        value.insert(
            "converge_time_s".to_owned(),
            JsonValue::String(format_human_ratio(converge_millis, 1_000)),
        );
    } else {
        value.insert(
            "converge_time".to_owned(),
            JsonValue::Unsigned(converge_millis),
        );
    }
    info.insert("last_close".to_owned(), JsonValue::Object(value));
}

pub(crate) fn append_status_snapshot_fields<V: AppServerInfoView>(
    info: &mut BTreeMap<String, JsonValue>,
    view: &V,
    snapshot: &StatusRpcSnapshot,
    human: bool,
) {
    let overlay_snapshot = view
        .overlay_status()
        .map(|overlay| overlay.status_snapshot());

    if let Some(peer_count) = overlay_snapshot
        .as_ref()
        .map(|overlay| overlay.peers)
        .or_else(|| snapshot.peer_count.map(u64::from))
    {
        info.insert("peers".to_owned(), JsonValue::Unsigned(peer_count));
    }

    if let Some(network_id) = overlay_snapshot
        .as_ref()
        .and_then(|overlay| overlay.network_id)
        .or(snapshot.network_id)
    {
        info.insert(
            "network_id".to_owned(),
            JsonValue::Unsigned(u64::from(network_id)),
        );
    }

    if let Some(overlay) = overlay_snapshot {
        info.insert(
            "jq_trans_overflow".to_owned(),
            JsonValue::String(overlay.jq_trans_overflow.to_string()),
        );
        info.insert(
            "peer_disconnects".to_owned(),
            JsonValue::String(overlay.peer_disconnects.to_string()),
        );
        info.insert(
            "peer_disconnects_resources".to_owned(),
            JsonValue::String(overlay.peer_disconnect_charges.to_string()),
        );
    }

    if let Some(last_close) = snapshot.last_close.clone() {
        append_last_close(info, last_close, human);
    }
}

#[cfg(test)]
mod tests {
    use super::duration_millis_u64;
    use std::time::Duration;

    #[test]
    fn duration_millis_is_bounded_to_u64() {
        assert_eq!(duration_millis_u64(Duration::from_millis(850)), 850);
    }
}
