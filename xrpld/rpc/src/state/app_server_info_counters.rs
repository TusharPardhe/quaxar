use std::collections::BTreeMap;

use protocol::JsonValue;

use crate::state::app_server_info_json::{
    insert_serde_json_field, to_protocol_json, with_object_field,
};
use crate::state::app_server_info_source::AppServerInfoView;

pub(crate) fn append_counters_fields<V: AppServerInfoView>(
    info: &mut BTreeMap<String, JsonValue>,
    view: &V,
    counters: bool,
) {
    if !counters {
        return;
    }

    let Some(source) = view.status_metrics() else {
        return;
    };

    info.insert(
        "counters".to_owned(),
        with_object_field(
            to_protocol_json(source.counters_json()),
            "nodestore",
            to_protocol_json(source.nodestore_counts_json()),
        ),
    );
    insert_serde_json_field(info, "current_activities", source.current_activities_json());
}
