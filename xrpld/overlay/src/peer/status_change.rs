use std::collections::BTreeMap;

use basics::base_uint::Uint256;
use protocol::JsonValue;

use crate::message::TmStatusChange;

const NS_CONNECTING: i32 = 1;
const NS_CONNECTED: i32 = 2;
const NS_MONITORING: i32 = 3;
const NS_VALIDATING: i32 = 4;
const NS_SHUTTING: i32 = 5;

const NE_CLOSING_LEDGER: i32 = 1;
const NE_ACCEPTED_LEDGER: i32 = 2;
const NE_SWITCHED_LEDGER: i32 = 3;
const NE_LOST_SYNC: i32 = 4;

pub(crate) fn lost_sync_event() -> i32 {
    NE_LOST_SYNC
}

pub(crate) fn build_peer_status_event(
    effective_status: Option<i32>,
    message: &TmStatusChange,
    closed_ledger_hash: Uint256,
) -> JsonValue {
    let mut object = BTreeMap::new();
    object.insert(
        "type".to_owned(),
        JsonValue::String("peerStatusChange".to_owned()),
    );

    if let Some(status) = effective_status.and_then(status_name) {
        object.insert("status".to_owned(), JsonValue::String(status.to_owned()));
    }

    if let Some(action) = message.new_event.and_then(event_name) {
        object.insert("action".to_owned(), JsonValue::String(action.to_owned()));
    }

    if let Some(ledger_seq) = message.ledger_seq {
        object.insert(
            "ledger_index".to_owned(),
            JsonValue::Unsigned(ledger_seq as u64),
        );
    }

    if message.ledger_hash.is_some() {
        object.insert(
            "ledger_hash".to_owned(),
            JsonValue::String(closed_ledger_hash.to_string()),
        );
    }

    if let Some(network_time) = message.network_time {
        object.insert("date".to_owned(), JsonValue::Unsigned(network_time));
    }

    if let (Some(first_seq), Some(last_seq)) = (message.first_seq, message.last_seq) {
        object.insert(
            "ledger_index_min".to_owned(),
            JsonValue::Unsigned(first_seq as u64),
        );
        object.insert(
            "ledger_index_max".to_owned(),
            JsonValue::Unsigned(last_seq as u64),
        );
    }

    JsonValue::Object(object)
}

fn status_name(status: i32) -> Option<&'static str> {
    match status {
        NS_CONNECTING => Some("CONNECTING"),
        NS_CONNECTED => Some("CONNECTED"),
        NS_MONITORING => Some("MONITORING"),
        NS_VALIDATING => Some("VALIDATING"),
        NS_SHUTTING => Some("SHUTTING"),
        _ => None,
    }
}

fn event_name(event: i32) -> Option<&'static str> {
    match event {
        NE_CLOSING_LEDGER => Some("CLOSING_LEDGER"),
        NE_ACCEPTED_LEDGER => Some("ACCEPTED_LEDGER"),
        NE_SWITCHED_LEDGER => Some("SWITCHED_LEDGER"),
        NE_LOST_SYNC => Some("LOST_SYNC"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use basics::base_uint::Uint256;
    use protocol::JsonValue;

    use super::build_peer_status_event;
    use crate::message::TmStatusChange;

    #[test]
    fn peer_status_event_uses_effective_status_and_type_wrapper() {
        let payload = build_peer_status_event(
            Some(2),
            &TmStatusChange {
                new_status: None,
                new_event: Some(3),
                ledger_seq: Some(300),
                ledger_hash: Some(vec![1; 32]),
                ledger_hash_previous: None,
                network_time: Some(55),
                first_seq: Some(250),
                last_seq: Some(300),
            },
            Uint256::from_u64(0xABCD),
        );

        let JsonValue::Object(object) = payload else {
            panic!("peer status payload should be an object");
        };
        assert_eq!(
            object.get("type"),
            Some(&JsonValue::String("peerStatusChange".to_owned()))
        );
        assert_eq!(
            object.get("status"),
            Some(&JsonValue::String("CONNECTED".to_owned()))
        );
        assert_eq!(
            object.get("action"),
            Some(&JsonValue::String("SWITCHED_LEDGER".to_owned()))
        );
        assert_eq!(
            object.get("ledger_hash"),
            Some(&JsonValue::String(Uint256::from_u64(0xABCD).to_string()))
        );
    }
}
