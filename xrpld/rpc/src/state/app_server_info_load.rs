use std::collections::BTreeMap;

use protocol::JsonValue;

use crate::state::app_server_info_json::insert_serde_json_field;
use crate::state::app_server_info_source::AppServerInfoView;

pub(crate) fn append_load_fields<V: AppServerInfoView>(
    info: &mut BTreeMap<String, JsonValue>,
    view: &V,
    admin: bool,
) {
    if admin {
        insert_serde_json_field(info, "load", view.job_queue().get_json(0));
    }
}
