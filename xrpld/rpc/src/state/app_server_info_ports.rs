use std::collections::BTreeMap;

use app::PublishedServerPort;
use protocol::JsonValue;

use crate::state::app_server_info_source::AppServerInfoView;

const ALLOWED_PROTOCOLS: [&str; 7] = ["http", "https", "peer", "ws", "ws2", "wss", "wss2"];

fn allowed_protocols(port: &PublishedServerPort) -> Vec<JsonValue> {
    let mut protocols = port.protocols.clone();
    protocols.sort();

    let mut allowed_index = 0;
    let mut filtered = Vec::new();

    for protocol in protocols {
        while allowed_index < ALLOWED_PROTOCOLS.len()
            && ALLOWED_PROTOCOLS[allowed_index] < protocol.as_str()
        {
            allowed_index += 1;
        }

        if allowed_index < ALLOWED_PROTOCOLS.len()
            && ALLOWED_PROTOCOLS[allowed_index] == protocol.as_str()
        {
            filtered.push(JsonValue::String(protocol));
        }
    }

    filtered
}

pub(crate) fn append_ports_field<V: AppServerInfoView>(
    info: &mut BTreeMap<String, JsonValue>,
    view: &V,
    admin: bool,
) {
    let mut ports = Vec::new();

    if let Some(source) = view.published_server_ports() {
        for port in source.published_server_ports() {
            if !admin && port.has_admin_restrictions() {
                continue;
            }

            let protocols = allowed_protocols(&port);
            if protocols.is_empty() {
                continue;
            }

            ports.push(JsonValue::Object(BTreeMap::from([
                ("port".to_owned(), JsonValue::String(port.port)),
                ("protocol".to_owned(), JsonValue::Array(protocols)),
            ])));
        }

        if let Some(grpc) = source.published_grpc_port()
            && !grpc.ip.is_empty()
            && !grpc.port.is_empty()
        {
            ports.push(JsonValue::Object(BTreeMap::from([
                ("port".to_owned(), JsonValue::String(grpc.port)),
                (
                    "protocol".to_owned(),
                    JsonValue::Array(vec![JsonValue::String("grpc".to_owned())]),
                ),
            ])));
        }
    }

    info.insert("ports".to_owned(), JsonValue::Array(ports));
}

#[cfg(test)]
mod tests {
    use super::{ALLOWED_PROTOCOLS, allowed_protocols};
    use app::PublishedServerPort;
    use protocol::JsonValue;

    #[test]
    fn allowed_protocols_follow_cpp_filter_and_sort_order() {
        let port = PublishedServerPort {
            port: "5005".to_owned(),
            protocols: vec![
                "ws".to_owned(),
                "http".to_owned(),
                "grpc".to_owned(),
                "ws".to_owned(),
                "peer".to_owned(),
            ],
            admin_nets_v4_configured: false,
            admin_nets_v6_configured: false,
            admin_user: None,
            admin_password: None,
        };

        assert_eq!(
            allowed_protocols(&port),
            vec![
                JsonValue::String("http".to_owned()),
                JsonValue::String("peer".to_owned()),
                JsonValue::String("ws".to_owned()),
                JsonValue::String("ws".to_owned()),
            ]
        );
        assert_eq!(ALLOWED_PROTOCOLS.len(), 7);
    }

    #[test]
    fn allowed_protocols_preserve_duplicate_configured_entries() {
        let port = PublishedServerPort {
            port: "5005".to_owned(),
            protocols: vec![
                "ws".to_owned(),
                "http".to_owned(),
                "ws".to_owned(),
                "peer".to_owned(),
            ],
            admin_nets_v4_configured: false,
            admin_nets_v6_configured: false,
            admin_user: None,
            admin_password: None,
        };

        assert_eq!(
            allowed_protocols(&port),
            vec![
                JsonValue::String("http".to_owned()),
                JsonValue::String("peer".to_owned()),
                JsonValue::String("ws".to_owned()),
                JsonValue::String("ws".to_owned()),
            ]
        );
    }
}
