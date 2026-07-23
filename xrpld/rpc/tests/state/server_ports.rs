//! Tests for server ports.

use app::{ApplicationRoot, PublishedGrpcPort, ServerPortSetup, ServerPortsSetup};
use protocol::JsonValue;
use rpc::{
    ApplicationServerInfo, JsonContext, JsonContextHeaders, RpcRole, do_server_info,
    do_server_state,
};
use std::collections::BTreeMap;
use std::sync::Arc;

fn context<'a, Env>(params: &'a JsonValue, env: &'a Env, role: RpcRole) -> JsonContext<'a, Env> {
    JsonContext {
        params,
        env,
        role,
        api_version: 2,
        headers: JsonContextHeaders::default(),
        unlimited: matches!(role, RpcRole::Admin),
    }
}

fn app_with_ports_setup(
    ports: Vec<ServerPortSetup>,
    grpc: Option<PublishedGrpcPort>,
) -> ApplicationRoot {
    let mut app = ApplicationRoot::new(0).expect("root shell should build");
    app.attach_server_ports_setup(Arc::new(ServerPortsSetup {
        ports,
        client: None,
        overlay: None,
        grpc,
    }));
    app
}

#[test]
fn server_info_ports_filter_protocols_and_append_grpc() {
    let app = app_with_ports_setup(
        vec![ServerPortSetup {
            name: "port_rpc".to_owned(),
            ip: "127.0.0.1".to_owned(),
            port: 5005,
            limit: 0,
            protocols: vec![
                "ws".to_owned(),
                "http".to_owned(),
                "grpc".to_owned(),
                "peer".to_owned(),
            ],
            user: "rpc".to_owned(),
            password: "secret".to_owned(),
            admin_user: String::new(),
            admin_password: String::new(),
            ssl_key: String::new(),
            ssl_cert: String::new(),
            ssl_chain: String::new(),
            ssl_ciphers: String::new(),
            admin_nets_v4: Vec::new(),
            admin_nets_v6: Vec::new(),
            secure_gateway_nets_v4: Vec::new(),
            secure_gateway_nets_v6: Vec::new(),
            standalone_mode: false,
        }],
        Some(PublishedGrpcPort {
            ip: "127.0.0.1".to_owned(),
            port: "50051".to_owned(),
        }),
    );

    let params = JsonValue::Object(BTreeMap::new());
    let result = do_server_info(&context(
        &params,
        &ApplicationServerInfo::new(&app),
        RpcRole::Admin,
    ));
    let JsonValue::Object(result) = result else {
        panic!("result must be an object");
    };
    let JsonValue::Object(info) = result.get("info").expect("info must exist") else {
        panic!("info must be an object");
    };
    let JsonValue::Array(ports) = info.get("ports").expect("ports must exist") else {
        panic!("ports must be an array");
    };

    assert_eq!(ports.len(), 2);
    assert_eq!(
        ports[0],
        JsonValue::Object(BTreeMap::from([
            ("port".to_owned(), JsonValue::String("5005".to_owned())),
            (
                "protocol".to_owned(),
                JsonValue::Array(vec![
                    JsonValue::String("http".to_owned()),
                    JsonValue::String("peer".to_owned()),
                    JsonValue::String("ws".to_owned()),
                ]),
            ),
        ]))
    );
    assert_eq!(
        ports[1],
        JsonValue::Object(BTreeMap::from([
            ("port".to_owned(), JsonValue::String("50051".to_owned())),
            (
                "protocol".to_owned(),
                JsonValue::Array(vec![JsonValue::String("grpc".to_owned())]),
            ),
        ]))
    );
}

#[test]
fn server_info_ports_hide_admin_restricted_entries_from_non_admin() {
    let app = app_with_ports_setup(
        vec![
            ServerPortSetup {
                name: "port_rpc".to_owned(),
                ip: "127.0.0.1".to_owned(),
                port: 5005,
                limit: 0,
                protocols: vec!["ws".to_owned(), "http".to_owned(), "ws".to_owned()],
                user: "rpc".to_owned(),
                password: "secret".to_owned(),
                admin_user: String::new(),
                admin_password: String::new(),
                ssl_key: String::new(),
                ssl_cert: String::new(),
                ssl_chain: String::new(),
                ssl_ciphers: String::new(),
                admin_nets_v4: Vec::new(),
                admin_nets_v6: Vec::new(),
                secure_gateway_nets_v4: Vec::new(),
                secure_gateway_nets_v6: Vec::new(),
                standalone_mode: false,
            },
            ServerPortSetup {
                name: "port_admin".to_owned(),
                ip: "127.0.0.1".to_owned(),
                port: 6006,
                limit: 0,
                protocols: vec!["peer".to_owned(), "wss".to_owned()],
                user: "rpc".to_owned(),
                password: "secret".to_owned(),
                admin_user: "rpc".to_owned(),
                admin_password: String::new(),
                ssl_key: String::new(),
                ssl_cert: String::new(),
                ssl_chain: String::new(),
                ssl_ciphers: String::new(),
                admin_nets_v4: vec!["127.0.0.0/8".parse().expect("admin network should parse")],
                admin_nets_v6: Vec::new(),
                secure_gateway_nets_v4: Vec::new(),
                secure_gateway_nets_v6: Vec::new(),
                standalone_mode: false,
            },
        ],
        Some(PublishedGrpcPort {
            ip: "127.0.0.1".to_owned(),
            port: "50051".to_owned(),
        }),
    );

    let params = JsonValue::Object(BTreeMap::new());
    let result = do_server_info(&context(
        &params,
        &ApplicationServerInfo::new(&app),
        RpcRole::User,
    ));
    let JsonValue::Object(result) = result else {
        panic!("result must be an object");
    };
    let JsonValue::Object(info) = result.get("info").expect("info must exist") else {
        panic!("info must be an object");
    };
    let JsonValue::Array(ports) = info.get("ports").expect("ports must exist") else {
        panic!("ports must be an array");
    };

    assert_eq!(ports.len(), 2);
    assert_eq!(
        ports[0],
        JsonValue::Object(BTreeMap::from([
            ("port".to_owned(), JsonValue::String("5005".to_owned())),
            (
                "protocol".to_owned(),
                JsonValue::Array(vec![
                    JsonValue::String("http".to_owned()),
                    JsonValue::String("ws".to_owned()),
                    JsonValue::String("ws".to_owned()),
                ]),
            ),
        ]))
    );
    assert_eq!(
        ports[1],
        JsonValue::Object(BTreeMap::from([
            ("port".to_owned(), JsonValue::String("50051".to_owned())),
            (
                "protocol".to_owned(),
                JsonValue::Array(vec![JsonValue::String("grpc".to_owned())]),
            ),
        ]))
    );
}

#[test]
fn server_info_ports_skip_empty_protocol_entries_and_blank_grpc() {
    let app = app_with_ports_setup(
        vec![ServerPortSetup {
            name: "port_rpc".to_owned(),
            ip: "127.0.0.1".to_owned(),
            port: 5005,
            limit: 0,
            protocols: vec!["grpc".to_owned(), "json".to_owned()],
            user: "rpc".to_owned(),
            password: "secret".to_owned(),
            admin_user: String::new(),
            admin_password: String::new(),
            ssl_key: String::new(),
            ssl_cert: String::new(),
            ssl_chain: String::new(),
            ssl_ciphers: String::new(),
            admin_nets_v4: Vec::new(),
            admin_nets_v6: Vec::new(),
            secure_gateway_nets_v4: Vec::new(),
            secure_gateway_nets_v6: Vec::new(),
            standalone_mode: false,
        }],
        Some(PublishedGrpcPort {
            ip: "".to_owned(),
            port: "50051".to_owned(),
        }),
    );

    let params = JsonValue::Object(BTreeMap::new());
    let result = do_server_info(&context(
        &params,
        &ApplicationServerInfo::new(&app),
        RpcRole::Admin,
    ));
    let JsonValue::Object(result) = result else {
        panic!("result must be an object");
    };
    let JsonValue::Object(info) = result.get("info").expect("info must exist") else {
        panic!("info must be an object");
    };
    let JsonValue::Array(ports) = info.get("ports").expect("ports must exist") else {
        panic!("ports must be an array");
    };

    assert!(ports.is_empty());
}

#[test]
fn server_state_ports_hide_admin_restricted_entries_from_non_admin() {
    let app = app_with_ports_setup(
        vec![
            ServerPortSetup {
                name: "port_rpc".to_owned(),
                ip: "127.0.0.1".to_owned(),
                port: 5005,
                limit: 0,
                protocols: vec![
                    "ws".to_owned(),
                    "http".to_owned(),
                    "grpc".to_owned(),
                    "peer".to_owned(),
                ],
                user: "rpc".to_owned(),
                password: "secret".to_owned(),
                admin_user: String::new(),
                admin_password: String::new(),
                ssl_key: String::new(),
                ssl_cert: String::new(),
                ssl_chain: String::new(),
                ssl_ciphers: String::new(),
                admin_nets_v4: Vec::new(),
                admin_nets_v6: Vec::new(),
                secure_gateway_nets_v4: Vec::new(),
                secure_gateway_nets_v6: Vec::new(),
                standalone_mode: false,
            },
            ServerPortSetup {
                name: "port_admin".to_owned(),
                ip: "127.0.0.1".to_owned(),
                port: 6006,
                limit: 0,
                protocols: vec!["peer".to_owned(), "wss".to_owned()],
                user: "rpc".to_owned(),
                password: "secret".to_owned(),
                admin_user: "rpc".to_owned(),
                admin_password: String::new(),
                ssl_key: String::new(),
                ssl_cert: String::new(),
                ssl_chain: String::new(),
                ssl_ciphers: String::new(),
                admin_nets_v4: vec!["127.0.0.0/8".parse().expect("admin network should parse")],
                admin_nets_v6: Vec::new(),
                secure_gateway_nets_v4: Vec::new(),
                secure_gateway_nets_v6: Vec::new(),
                standalone_mode: false,
            },
        ],
        Some(PublishedGrpcPort {
            ip: "127.0.0.1".to_owned(),
            port: "50051".to_owned(),
        }),
    );

    let params = JsonValue::Object(BTreeMap::new());
    let source = ApplicationServerInfo::new(&app);
    let result = do_server_state(&context(&params, &source, RpcRole::User));
    let JsonValue::Object(result) = result else {
        panic!("result must be an object");
    };
    let JsonValue::Object(state) = result.get("state").expect("state must exist") else {
        panic!("state must be an object");
    };
    let JsonValue::Array(ports) = state.get("ports").expect("ports must exist") else {
        panic!("ports must be an array");
    };

    assert_eq!(ports.len(), 2);
    assert_eq!(
        ports[0],
        JsonValue::Object(BTreeMap::from([
            ("port".to_owned(), JsonValue::String("5005".to_owned())),
            (
                "protocol".to_owned(),
                JsonValue::Array(vec![
                    JsonValue::String("http".to_owned()),
                    JsonValue::String("peer".to_owned()),
                    JsonValue::String("ws".to_owned()),
                ]),
            ),
        ]))
    );
    assert_eq!(
        ports[1],
        JsonValue::Object(BTreeMap::from([
            ("port".to_owned(), JsonValue::String("50051".to_owned())),
            (
                "protocol".to_owned(),
                JsonValue::Array(vec![JsonValue::String("grpc".to_owned())]),
            ),
        ]))
    );
}
