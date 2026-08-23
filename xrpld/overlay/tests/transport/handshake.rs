use http::{HeaderMap, HeaderValue, Method, Request, Response, StatusCode, Version};
use overlay::handshake::{negotiate_inbound_peer_upgrade, validate_outbound_peer_upgrade};
use overlay::{
    feature_enabled, get_feature_value, is_feature_value, make_features_request_header,
    make_request,
};

#[test]
fn handshake_feature_header_cases() {
    let mut headers = HeaderMap::new();
    headers.insert(
        "X-Protocol-Ctl",
        HeaderValue::from_static(
            "feature1=v1,v2,v3; feature2=v4; feature3=10; feature4=1; feature5=v6",
        ),
    );

    assert!(!feature_enabled(&headers, "feature1"));
    assert!(!is_feature_value(&headers, "feature1", "2"));
    assert!(is_feature_value(&headers, "feature1", "v1"));
    assert!(is_feature_value(&headers, "feature1", "v2"));
    assert!(is_feature_value(&headers, "feature1", "v3"));
    assert!(is_feature_value(&headers, "feature2", "v4"));
    assert!(!is_feature_value(&headers, "feature3", "1"));
    assert!(is_feature_value(&headers, "feature3", "10"));
    assert!(!is_feature_value(&headers, "feature4", "10"));
    assert!(is_feature_value(&headers, "feature4", "1"));
    assert_eq!(
        get_feature_value(&headers, "feature5"),
        Some("v6".to_owned())
    );
}

#[test]
fn handshake_upgrade_requires_reference_http_method_and_version() {
    let request = make_request(true, false, false, false, false);
    assert!(negotiate_inbound_peer_upgrade(&request).is_some());

    let http_ten_request = Request::builder()
        .method(Method::GET)
        .version(Version::HTTP_10)
        .header("Connection", "Upgrade")
        .header("Connect-As", "Peer")
        .header("Upgrade", "XRPL/2.2")
        .body(())
        .expect("http/1.0 request");
    assert!(negotiate_inbound_peer_upgrade(&http_ten_request).is_none());

    let post_request = Request::builder()
        .method(Method::POST)
        .version(Version::HTTP_11)
        .header("Connection", "Upgrade")
        .header("Connect-As", "Peer")
        .header("Upgrade", "XRPL/2.2")
        .body(())
        .expect("post request");
    assert!(negotiate_inbound_peer_upgrade(&post_request).is_none());

    let http_ten_response = Response::builder()
        .status(StatusCode::SWITCHING_PROTOCOLS)
        .version(Version::HTTP_10)
        .header("Connection", "Upgrade")
        .header("Upgrade", "XRPL/2.2")
        .body(())
        .expect("http/1.0 response");
    assert!(validate_outbound_peer_upgrade(&http_ten_response).is_err());
}

#[test]
fn handshake_upgrade_request_generates_cpp_header_shape() {
    let request = make_request(true, true, true, true, true);
    assert_eq!(request.method(), http::Method::GET);
    assert_eq!(request.uri(), "/");
    assert_eq!(request.version(), http::Version::HTTP_11);
    assert_eq!(request.headers()["Connection"], "Upgrade");
    assert_eq!(request.headers()["Connect-As"], "Peer");
    assert_eq!(request.headers()["Crawl"], "public");
    assert_eq!(
        request.headers()["X-Protocol-Ctl"],
        make_features_request_header(true, true, true, true)
    );
}
