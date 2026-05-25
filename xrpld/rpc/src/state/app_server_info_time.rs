use std::collections::BTreeMap;

use protocol::JsonValue;

use crate::state::app_server_info_source::AppServerInfoView;

pub(crate) fn append_time_field<V: AppServerInfoView>(
    info: &mut BTreeMap<String, JsonValue>,
    view: &V,
) {
    info.insert(
        "time".to_owned(),
        JsonValue::String(view.current_server_time_string()),
    );
}
