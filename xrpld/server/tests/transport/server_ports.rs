use app::{ApplicationRoot, PublishedGrpcPort, PublishedServerPort, PublishedServerPortsSource};
use protocol::JsonValue;
use server::{BuiltinDispatcher, RequestMetadata, RpcDispatcher, RpcRequest, SubscriptionManager};
use std::collections::BTreeMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;

#[derive(Debug, Clone)]
struct FixedPublishedServerPortsSource {
    ports: Vec<PublishedServerPort>,
    grpc: Option<PublishedGrpcPort>,
}

impl PublishedServerPortsSource for FixedPublishedServerPortsSource {
    fn published_server_ports(&self) -> Vec<PublishedServerPort> {
        self.ports.clone()
    }

    fn published_grpc_port(&self) -> Option<PublishedGrpcPort> {
        self.grpc.clone()
    }
}

fn metadata(role: rpc::RpcRole) -> RequestMetadata {
    let request = http::Request::builder()
        .method("POST")
        .uri("/")
        .body(axum::body::Body::from(Vec::<u8>::new()))
        .expect("request should build");
    let mut metadata = RequestMetadata::new(
        SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 51234),
        &request,
    );
    metadata.role = role;
    metadata.unlimited = matches!(role, rpc::RpcRole::Admin);
    metadata
}

#[test]
fn dispatcher_hides_admin_ports_from_non_admin_requests() {
    let mut app = ApplicationRoot::new(0).expect("root shell should build");
    app.attach_published_server_ports(Arc::new(FixedPublishedServerPortsSource {
        ports: vec![
            PublishedServerPort {
                port: "5005".to_owned(),
                protocols: vec!["http".to_owned()],
                admin_nets_v4_configured: false,
                admin_nets_v6_configured: false,
                admin_user: None,
                admin_password: None,
            },
            PublishedServerPort {
                port: "6006".to_owned(),
                protocols: vec!["peer".to_owned()],
                admin_nets_v4_configured: true,
                admin_nets_v6_configured: false,
                admin_user: None,
                admin_password: None,
            },
        ],
        grpc: Some(PublishedGrpcPort {
            ip: "127.0.0.1".to_owned(),
            port: "50051".to_owned(),
        }),
    }));

    let dispatcher = BuiltinDispatcher::new(
        rpc::ApplicationServerInfo::new(&app),
        SubscriptionManager::default(),
    );
    let admin = metadata(rpc::RpcRole::Admin);
    let user = metadata(rpc::RpcRole::User);

    let admin_reply = dispatcher.dispatch(RpcRequest {
        method: "server_info",
        params: &JsonValue::Object(BTreeMap::new()),
        metadata: &admin,
        session: None,
    });
    let user_reply = dispatcher.dispatch(RpcRequest {
        method: "server_info",
        params: &JsonValue::Object(BTreeMap::new()),
        metadata: &user,
        session: None,
    });

    let server::RpcReply::Result(admin_reply) = admin_reply else {
        panic!("admin reply must be a result");
    };
    let server::RpcReply::Result(user_reply) = user_reply else {
        panic!("user reply must be a result");
    };

    assert_eq!(
        server::from_protocol_json(&admin_reply)["info"]["ports"]
            .as_array()
            .unwrap()
            .len(),
        3
    );
    assert_eq!(
        server::from_protocol_json(&user_reply)["info"]["ports"]
            .as_array()
            .unwrap()
            .len(),
        2
    );

    let admin_json = server::from_protocol_json(&admin_reply);
    let admin_ports = admin_json["info"]["ports"].as_array().unwrap();
    assert_eq!(admin_ports[0]["port"], "5005");
    assert_eq!(admin_ports[1]["port"], "6006");
    assert_eq!(admin_ports[2]["port"], "50051");
    let user_json = server::from_protocol_json(&user_reply);
    let user_ports = user_json["info"]["ports"].as_array().unwrap();
    assert_eq!(user_ports[0]["port"], "5005");
    assert_eq!(user_ports[1]["port"], "50051");
    assert!(!user_ports.iter().any(|port| port["port"] == "6006"));
}
