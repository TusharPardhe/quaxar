use std::collections::{BTreeMap, VecDeque};
use std::sync::Mutex;
use std::time::{Duration, SystemTime};

use app::{
    ListDisposition, PublisherListStats, SiteResource, SiteResponse, ValidatorBlobInfo,
    ValidatorSite, ValidatorSiteSink, ValidatorSiteTransport,
};
use protocol::JsonValue;

#[derive(Default)]
struct RecordingSink {
    calls: Vec<ApplyCall>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ApplyCall {
    manifest: String,
    version: u32,
    blobs: Vec<ValidatorBlobInfo>,
    uri: String,
}

impl ValidatorSiteSink for RecordingSink {
    fn apply_lists(
        &mut self,
        manifest: &str,
        version: u32,
        blobs: &[ValidatorBlobInfo],
        site_uri: String,
        _hash: basics::base_uint::Uint256,
    ) -> PublisherListStats {
        self.calls.push(ApplyCall {
            manifest: manifest.to_owned(),
            version,
            blobs: blobs.to_vec(),
            uri: site_uri,
        });
        PublisherListStats::new(ListDisposition::Accepted)
    }
}

struct QueueTransport(Mutex<VecDeque<Result<SiteResponse, String>>>);

impl ValidatorSiteTransport for QueueTransport {
    fn fetch(&self, _resource: &SiteResource, _timeout: Duration) -> Result<SiteResponse, String> {
        self.0
            .lock()
            .expect("transport queue")
            .pop_front()
            .expect("queued response")
    }
}

fn ok_response(body: &str) -> Result<SiteResponse, String> {
    Ok(SiteResponse {
        status: 200,
        location: None,
        body: body.to_owned(),
    })
}

fn redirect_response(status: u16, location: &str) -> Result<SiteResponse, String> {
    Ok(SiteResponse {
        status,
        location: Some(location.to_owned()),
        body: String::new(),
    })
}

fn entries(site: &ValidatorSite) -> Vec<BTreeMap<String, JsonValue>> {
    let JsonValue::Object(root) = site.get_json() else {
        panic!("validator site json should be an object");
    };
    let JsonValue::Array(entries) = root
        .get("validator_sites")
        .expect("validator_sites should exist")
    else {
        panic!("validator_sites should be an array");
    };
    entries
        .iter()
        .map(|entry| {
            let JsonValue::Object(entry) = entry else {
                panic!("validator site entry should be an object");
            };
            entry.clone()
        })
        .collect()
}

fn json_string<'a>(entry: &'a BTreeMap<String, JsonValue>, key: &str) -> &'a str {
    match entry.get(key) {
        Some(JsonValue::String(text)) => text,
        _ => panic!("{key} should be a string"),
    }
}

#[test]
fn validator_site_load_accepts_cpp_schemes_and_rejects_invalid_uris() {
    let mut site = ValidatorSite::new(Duration::from_secs(20));
    assert!(site.load(&[
        "http://ripple.com/".to_owned(),
        "http://ripple.com/validators".to_owned(),
        "http://ripple.com:8080/validators".to_owned(),
        "https://ripple.com/validators".to_owned(),
        "file:///etc/opt/ripple/validators.txt".to_owned(),
        "file:///".to_owned(),
    ]));

    for invalid in [
        "ftp://ripple.com/validators",
        "wss://ripple.com/validators",
        "ripple.com/validators",
        "file://ripple.com/vl.txt",
        "file://localhost/home/user/vl.txt",
        "file://127.0.0.1/home/user/vl.txt",
        "file://",
    ] {
        let mut invalid_site = ValidatorSite::new(Duration::from_secs(20));
        assert!(!invalid_site.load(&[invalid.to_owned()]), "{invalid}");
    }
}

#[test]
fn validator_site_applies_valid_json_response() {
    let body = serde_json::json!({
        "manifest": "outer-manifest",
        "version": 1,
        "blob": "Zm9v",
        "signature": "AABB"
    })
    .to_string();
    let transport = QueueTransport(Mutex::new(VecDeque::from([ok_response(&body)])));
    let mut site = ValidatorSite::new(Duration::from_secs(20));
    assert!(site.load(&["https://ripple.com/validators".to_owned()]));

    let mut sink = RecordingSink::default();
    let now = SystemTime::UNIX_EPOCH + Duration::from_secs(2_000_000_000);
    site.refresh_due(&mut sink, &transport, now);

    assert_eq!(sink.calls.len(), 1);
    assert_eq!(sink.calls[0].manifest, "outer-manifest");
    assert_eq!(sink.calls[0].version, 1);
    assert_eq!(sink.calls[0].uri, "https://ripple.com/validators");
    assert_eq!(
        sink.calls[0].blobs,
        vec![ValidatorBlobInfo {
            blob: "Zm9v".to_owned(),
            signature: "AABB".to_owned(),
            manifest: None,
        }]
    );

    let entries = entries(&site);
    assert_eq!(
        json_string(&entries[0], "last_refresh_time"),
        "2033-May-18 03:33:20"
    );
    assert_eq!(
        json_string(&entries[0], "next_refresh_time"),
        "2033-May-18 03:38:20"
    );
}

#[test]
fn validator_site_permanent_redirect_updates_uri_and_refresh_interval() {
    let body = serde_json::json!({
        "manifest": "outer-manifest",
        "version": 1,
        "blob": "Zm9v",
        "signature": "AABB",
        "refresh_interval": 17
    })
    .to_string();
    let transport = QueueTransport(Mutex::new(VecDeque::from([
        redirect_response(301, "https://vl.ripple.com/list"),
        ok_response(&body),
    ])));
    let mut site = ValidatorSite::new(Duration::from_secs(20));
    assert!(site.load(&["http://ripple.com/validators".to_owned()]));

    let mut sink = RecordingSink::default();
    site.refresh_due(&mut sink, &transport, SystemTime::now());

    assert_eq!(sink.calls.len(), 1);
    assert_eq!(sink.calls[0].uri, "https://vl.ripple.com/list");

    let entries = entries(&site);
    let uri = entries[0].get("uri");
    assert!(matches!(uri, Some(JsonValue::String(text)) if text.contains("redirects to")));
    assert_eq!(
        entries[0].get("refresh_interval_min"),
        Some(&JsonValue::Unsigned(17))
    );
}

#[test]
fn validator_site_records_retryable_fetch_errors() {
    let transport = QueueTransport(Mutex::new(VecDeque::from([Err("boom".to_owned())])));
    let mut site = ValidatorSite::new(Duration::from_secs(20));
    assert!(site.load(&["https://ripple.com/validators".to_owned()]));

    let mut sink = RecordingSink::default();
    let now = SystemTime::UNIX_EPOCH + Duration::from_secs(2_100_000_000);
    site.refresh_due(&mut sink, &transport, now);
    assert!(sink.calls.is_empty());

    let entries = entries(&site);
    assert_eq!(
        entries[0].get("last_refresh_status"),
        Some(&JsonValue::String("invalid".to_owned()))
    );
    assert_eq!(
        entries[0].get("last_refresh_message"),
        Some(&JsonValue::String("boom".to_owned()))
    );
    assert_eq!(
        json_string(&entries[0], "last_refresh_time"),
        "2036-Jul-18 13:20:00"
    );
    assert_eq!(
        json_string(&entries[0], "next_refresh_time"),
        "2036-Jul-18 13:20:30"
    );
}
