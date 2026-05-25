use http::{HeaderMap, HeaderValue};
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
