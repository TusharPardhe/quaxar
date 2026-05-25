use std::collections::BTreeMap;

use protocol::JsonValue;

use crate::state::app_server_info_json::insert_serde_json_field;
use crate::state::app_server_info_source::AppServerInfoView;

pub(crate) fn append_state_accounting_fields<V: AppServerInfoView>(
    info: &mut BTreeMap<String, JsonValue>,
    view: &V,
) {
    let Some(source) = view.status_metrics() else {
        return;
    };

    insert_serde_json_field(info, "state_accounting", source.state_accounting_json());

    if let Some(duration) = source.server_state_duration_us() {
        info.insert(
            "server_state_duration_us".to_owned(),
            JsonValue::String(duration),
        );
    }

    if let Some(duration) = source.initial_sync_duration_us() {
        info.insert(
            "initial_sync_duration_us".to_owned(),
            JsonValue::String(duration),
        );
    }
}
