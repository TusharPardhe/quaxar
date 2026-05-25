//! Narrow `server_info` / `server_state` warning assembly helper.
//!
//! This keeps the JSON shape for current system warnings in one place without
//! inventing amendment-vote logic or ledger state here.

use std::collections::BTreeMap;

use app::UnsupportedMajorityWarningDetails;
use protocol::JsonValue;

use crate::state::app_server_info_source::AppServerInfoView;

pub const WARN_RPC_UNSUPPORTED_MAJORITY: i64 = 1001;
pub const WARN_RPC_AMENDMENT_BLOCKED: i64 = 1002;
pub const WARN_RPC_EXPIRED_VALIDATOR_LIST: i64 = 1003;

const UNSUPPORTED_MAJORITY_MESSAGE: &str = "One or more unsupported amendments have reached majority. Upgrade to the latest version before they are activated to avoid being amendment blocked.";
const AMENDMENT_BLOCKED_MESSAGE: &str = "This server is amendment blocked, and must be updated to be able to stay in sync with the network.";
const EXPIRED_VALIDATOR_LIST_MESSAGE: &str = "This server has an expired validator list. validators.txt may be incorrectly configured or some [validator_list_sites] may be unreachable.";

fn unsupported_majority_details_json(details: &UnsupportedMajorityWarningDetails) -> JsonValue {
    JsonValue::Object(BTreeMap::from([
        (
            "expected_date".to_owned(),
            JsonValue::Signed(details.expected_date),
        ),
        (
            "expected_date_UTC".to_owned(),
            JsonValue::String(details.expected_date_utc.clone()),
        ),
    ]))
}

fn push_warning(
    warnings: &mut Vec<JsonValue>,
    id: i64,
    message: &str,
    details: Option<&UnsupportedMajorityWarningDetails>,
) {
    let mut warning = BTreeMap::from([
        ("id".to_owned(), JsonValue::Signed(id)),
        ("message".to_owned(), JsonValue::String(message.to_owned())),
    ]);

    if let Some(details) = details {
        warning.insert(
            "details".to_owned(),
            unsupported_majority_details_json(details),
        );
    }

    warnings.push(JsonValue::Object(warning));
}

pub fn build_server_info_warnings<V: AppServerInfoView>(view: &V, admin: bool) -> Vec<JsonValue> {
    let mut warnings = Vec::new();

    if view.amendment_blocked() {
        push_warning(
            &mut warnings,
            WARN_RPC_AMENDMENT_BLOCKED,
            AMENDMENT_BLOCKED_MESSAGE,
            None,
        );
    }

    if view.unl_blocked() {
        push_warning(
            &mut warnings,
            WARN_RPC_EXPIRED_VALIDATOR_LIST,
            EXPIRED_VALIDATOR_LIST_MESSAGE,
            None,
        );
    }

    if admin {
        if view.unsupported_majority_warned() {
            let details = view.unsupported_majority_warning_details();
            push_warning(
                &mut warnings,
                WARN_RPC_UNSUPPORTED_MAJORITY,
                UNSUPPORTED_MAJORITY_MESSAGE,
                details.as_ref(),
            );
        }
    }

    warnings
}
