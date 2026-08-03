//! Validator-site fetch and apply owner.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use basics::string_utilities::{ParsedUrl, parse_url};
use protocol::JsonValue;

use crate::validator::validator_list::{
    ListDisposition, PublisherListStats, ValidatorBlobInfo, ValidatorList,
    validator_list_collection_hash,
};
use time::{Duration as TimeDuration, Month, OffsetDateTime};

pub const DEFAULT_REFRESH_INTERVAL: Duration = Duration::from_secs(5 * 60);
pub const ERROR_RETRY_INTERVAL: Duration = Duration::from_secs(30);
pub const MAX_REDIRECTS: u16 = 3;

/// Thread-safe owner for configured and cached validator-list sites. The
/// runtime owns one instance for its whole life; fetch work is synchronous in
/// this port, but state remains synchronized so status reads and refreshes can
/// safely share that instance.
#[derive(Debug, Clone)]
pub struct ValidatorSite {
    request_timeout: Duration,
    sites: Arc<Mutex<Vec<Site>>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Site {
    loaded_resource: SiteResource,
    starting_resource: SiteResource,
    active_resource: Option<SiteResource>,
    redirect_count: u16,
    refresh_interval: Duration,
    next_refresh: SystemTime,
    last_refresh_status: Option<RefreshStatus>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RefreshStatus {
    refreshed: SystemTime,
    disposition: ListDisposition,
    message: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SiteResource {
    pub uri: String,
    pub parsed: ParsedUrl,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SiteResponse {
    pub status: u16,
    pub location: Option<String>,
    pub body: String,
}

pub trait ValidatorSiteTransport {
    fn fetch(&self, resource: &SiteResource, timeout: Duration) -> Result<SiteResponse, String>;
}

pub trait ValidatorSiteSink {
    fn apply_lists(
        &mut self,
        manifest: &str,
        version: u32,
        blobs: &[ValidatorBlobInfo],
        site_uri: String,
        hash: basics::base_uint::Uint256,
    ) -> PublisherListStats;

    fn load_lists(&self) -> Vec<String> {
        Vec::new()
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ReqwestValidatorSiteTransport;

impl ValidatorSiteTransport for ReqwestValidatorSiteTransport {
    fn fetch(&self, resource: &SiteResource, timeout: Duration) -> Result<SiteResponse, String> {
        match resource.parsed.scheme.as_str() {
            "file" => std::fs::read_to_string(&resource.parsed.path)
                .map(|body| SiteResponse {
                    status: 200,
                    location: None,
                    body,
                })
                .map_err(|error| error.to_string()),
            "http" | "https" => {
                let client = reqwest::blocking::Client::builder()
                    .timeout(timeout)
                    .redirect(reqwest::redirect::Policy::none())
                    .build()
                    .map_err(|error| error.to_string())?;
                let response = client
                    .get(&resource.uri)
                    .send()
                    .map_err(|error| error.to_string())?;
                let status = response.status().as_u16();
                let location = response
                    .headers()
                    .get(reqwest::header::LOCATION)
                    .and_then(|value| value.to_str().ok())
                    .map(ToOwned::to_owned);
                let body = response.text().map_err(|error| error.to_string())?;
                Ok(SiteResponse {
                    status,
                    location,
                    body,
                })
            }
            _ => Err("unsupported scheme".to_owned()),
        }
    }
}

impl ValidatorSite {
    pub fn new(request_timeout: Duration) -> Self {
        Self {
            request_timeout,
            sites: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// Configure remote sites. Configuration is all-or-nothing: an invalid
    /// URI leaves no partially configured active site set.
    pub fn load(&self, site_uris: &[String]) -> bool {
        let mut sites = self.sites.lock().expect("validator sites lock");
        let mut configured = Vec::with_capacity(site_uris.len());
        if !Self::append_sites(&mut configured, site_uris) {
            return false;
        }
        *sites = configured;
        true
    }

    /// Add cached validator-list files when configured endpoints are absent or
    /// fail. Cache URIs are deduplicated so retrying an endpoint cannot grow
    /// the site list indefinitely.
    pub fn load_cached_lists<S: ValidatorSiteSink>(&self, sink: &S) -> bool {
        let cached = sink.load_lists();
        let mut sites = self.sites.lock().expect("validator sites lock");
        Self::append_sites(&mut sites, &cached)
    }

    pub fn refresh_due<S: ValidatorSiteSink, T: ValidatorSiteTransport>(
        &self,
        sink: &mut S,
        transport: &T,
        now: SystemTime,
    ) {
        let mut sites = self.sites.lock().expect("validator sites lock");
        let due = sites
            .iter()
            .enumerate()
            .filter_map(|(index, site)| (site.next_refresh <= now).then_some(index))
            .collect::<Vec<_>>();
        for index in due {
            self.refresh_site(&mut sites, index, sink, transport, now);
        }
    }

    pub fn get_json(&self) -> JsonValue {
        let sites = self.sites.lock().expect("validator sites lock");
        let mut entries = Vec::with_capacity(sites.len());
        for site in sites.iter() {
            let mut entry = BTreeMap::new();
            let uri = if site.loaded_resource != site.starting_resource {
                format!(
                    "{} (redirects to {})",
                    site.loaded_resource.uri, site.starting_resource.uri
                )
            } else {
                site.loaded_resource.uri.clone()
            };
            entry.insert("uri".to_owned(), JsonValue::String(uri));
            entry.insert(
                "next_refresh_time".to_owned(),
                JsonValue::String(format_refresh_time(site.next_refresh)),
            );
            entry.insert(
                "refresh_interval_min".to_owned(),
                JsonValue::Unsigned(site.refresh_interval.as_secs() / 60),
            );
            if let Some(status) = &site.last_refresh_status {
                entry.insert(
                    "last_refresh_time".to_owned(),
                    JsonValue::String(format_refresh_time(status.refreshed)),
                );
                entry.insert(
                    "last_refresh_status".to_owned(),
                    JsonValue::String(list_disposition_text(status.disposition).to_owned()),
                );
                if !status.message.is_empty() {
                    entry.insert(
                        "last_refresh_message".to_owned(),
                        JsonValue::String(status.message.clone()),
                    );
                }
            }
            entries.push(JsonValue::Object(entry));
        }
        JsonValue::Object(BTreeMap::from([(
            "validator_sites".to_owned(),
            JsonValue::Array(entries),
        )]))
    }

    fn append_sites(sites: &mut Vec<Site>, site_uris: &[String]) -> bool {
        let mut additions = Vec::new();
        for uri in site_uris {
            if sites.iter().any(|site| site.loaded_resource.uri == *uri)
                || additions
                    .iter()
                    .any(|site: &Site| site.loaded_resource.uri == *uri)
            {
                continue;
            }
            let Ok(resource) = SiteResource::new(uri.clone()) else {
                return false;
            };
            additions.push(Site {
                starting_resource: resource.clone(),
                loaded_resource: resource,
                active_resource: None,
                redirect_count: 0,
                refresh_interval: DEFAULT_REFRESH_INTERVAL,
                next_refresh: SystemTime::now(),
                last_refresh_status: None,
            });
        }
        sites.extend(additions);
        true
    }

    fn refresh_site<S: ValidatorSiteSink, T: ValidatorSiteTransport>(
        &self,
        sites: &mut Vec<Site>,
        site_index: usize,
        sink: &mut S,
        transport: &T,
        now: SystemTime,
    ) {
        let resource = sites[site_index].starting_resource.clone();
        sites[site_index].active_resource = Some(resource.clone());
        sites[site_index].redirect_count = 0;
        sites[site_index].next_refresh = now + sites[site_index].refresh_interval;

        loop {
            let active = sites[site_index]
                .active_resource
                .clone()
                .expect("active resource");
            let response = match transport.fetch(&active, self.request_timeout) {
                Ok(response) => response,
                Err(message) => {
                    tracing::warn!(target: "validator_site", uri = %active.uri, %message, "Fetch failed");
                    self.record_error(sites, site_index, now, &message, true, sink);
                    return;
                }
            };

            match response.status {
                200 => match Self::parse_json_response(sites, site_index, &response.body, sink, now) {
                    Ok(()) => {
                        tracing::info!(target: "validator_site", uri = %active.uri, "Loaded validator list");
                        return;
                    }
                    Err(message) => {
                        tracing::warn!(target: "validator_site", uri = %active.uri, %message, "Parse failed");
                        self.record_error(sites, site_index, now, &message, false, sink);
                        return;
                    }
                },
                301 | 302 | 307 | 308 => match Self::process_redirect(sites, site_index, response.location)
                {
                    Ok(new_resource) => {
                        if matches!(response.status, 301 | 308) {
                            sites[site_index].starting_resource = new_resource.clone();
                        }
                        sites[site_index].active_resource = Some(new_resource);
                    }
                    Err(message) => {
                        self.record_error(sites, site_index, now, &message, false, sink);
                        return;
                    }
                },
                status => {
                    self.record_error(
                        sites,
                        site_index,
                        now,
                        &format!("bad result code {status}"),
                        true,
                        sink,
                    );
                    return;
                }
            }
        }
    }

    fn parse_json_response<S: ValidatorSiteSink>(
        sites: &mut [Site],
        site_index: usize,
        response_text: &str,
        sink: &mut S,
        now: SystemTime,
    ) -> Result<(), String> {
        let body: serde_json::Value =
            serde_json::from_str(response_text).map_err(|_| "bad json".to_owned())?;
        let manifest = body
            .get("manifest")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| "missing fields".to_owned())?;
        let version = body
            .get("version")
            .and_then(serde_json::Value::as_u64)
            .and_then(|value| u32::try_from(value).ok())
            .ok_or_else(|| "missing fields".to_owned())?;
        let blobs = ValidatorList::<crate::validator::validator_list::SystemValidatorListClock>::parse_blobs(
            version, &body,
        );
        if blobs.is_empty() {
            return Err("missing fields".to_owned());
        }
        let uri = sites[site_index]
            .active_resource
            .as_ref()
            .expect("active resource")
            .uri
            .clone();
        let hash = validator_list_collection_hash(manifest, version, &blobs);
        let result = sink.apply_lists(manifest, version, &blobs, uri, hash);

        sites[site_index].last_refresh_status = Some(RefreshStatus {
            refreshed: now,
            disposition: result.best_disposition(),
            message: String::new(),
        });
        if let Some(refresh_minutes) = body
            .get("refresh_interval")
            .and_then(serde_json::Value::as_u64)
        {
            let refresh_minutes = refresh_minutes.clamp(1, 24 * 60);
            sites[site_index].refresh_interval = Duration::from_secs(refresh_minutes * 60);
            sites[site_index].next_refresh = now + sites[site_index].refresh_interval;
        }
        sites[site_index].active_resource = None;
        Ok(())
    }

    fn process_redirect(
        sites: &mut [Site],
        site_index: usize,
        location: Option<String>,
    ) -> Result<SiteResource, String> {
        let Some(location) = location else {
            return Err("missing location".to_owned());
        };
        if sites[site_index].redirect_count == MAX_REDIRECTS {
            return Err("max redirects".to_owned());
        }
        let resource = SiteResource::new(location)?;
        if !matches!(resource.parsed.scheme.as_str(), "http" | "https") {
            return Err(format!(
                "invalid scheme in redirect {}",
                resource.parsed.scheme
            ));
        }
        sites[site_index].redirect_count += 1;
        Ok(resource)
    }

    fn record_error<S: ValidatorSiteSink>(
        &self,
        sites: &mut Vec<Site>,
        site_index: usize,
        now: SystemTime,
        message: &str,
        retry: bool,
        sink: &S,
    ) {
        sites[site_index].last_refresh_status = Some(RefreshStatus {
            refreshed: now,
            disposition: ListDisposition::Invalid,
            message: message.to_owned(),
        });
        if retry {
            sites[site_index].next_refresh = now + ERROR_RETRY_INTERVAL;
        }
        sites[site_index].active_resource = None;

        // Rippled's missingSite() loads valid cache files after every failed
        // endpoint request. They carry a 24-hour refresh interval and their
        // signed expiration is still enforced by ValidatorList.
        let cached = sink.load_lists();
        if !cached.is_empty() && !Self::append_sites(sites, &cached) {
            tracing::warn!(target: "validator_site", "Ignoring invalid cached validator-list URI");
        }
    }
}

impl SiteResource {
    pub fn new(uri: String) -> Result<Self, String> {
        let parsed = parse_url(&uri).ok_or_else(|| format!("URI '{uri}' cannot be parsed"))?;
        match parsed.scheme.as_str() {
            "file" => {
                if !parsed.domain.is_empty() {
                    return Err("file URI cannot contain a hostname".to_owned());
                }
                if parsed.path.is_empty() {
                    return Err("file URI must contain a path".to_owned());
                }
            }
            "http" => {
                if parsed.domain.is_empty() {
                    return Err("http URI must contain a hostname".to_owned());
                }
            }
            "https" => {
                if parsed.domain.is_empty() {
                    return Err("https URI must contain a hostname".to_owned());
                }
            }
            other => return Err(format!("Unsupported scheme: '{other}'")),
        }
        let mut parsed = parsed;
        if parsed.scheme == "http" && parsed.port.is_none() {
            parsed.port = Some(80);
        }
        if parsed.scheme == "https" && parsed.port.is_none() {
            parsed.port = Some(443);
        }
        Ok(Self { uri, parsed })
    }
}

fn list_disposition_text(disposition: ListDisposition) -> &'static str {
    match disposition {
        ListDisposition::Accepted => "accepted",
        ListDisposition::Expired => "expired",
        ListDisposition::Pending => "pending",
        ListDisposition::SameSequence => "same_sequence",
        ListDisposition::KnownSequence => "known_sequence",
        ListDisposition::Stale => "stale",
        ListDisposition::Untrusted => "untrusted",
        ListDisposition::UnsupportedVersion => "unsupported_version",
        ListDisposition::Invalid => "invalid",
    }
}

fn format_refresh_time(time: SystemTime) -> String {
    let date_time = match time.duration_since(UNIX_EPOCH) {
        Ok(duration) => {
            OffsetDateTime::UNIX_EPOCH
                + TimeDuration::new(duration.as_secs() as i64, duration.subsec_nanos() as i32)
        }
        Err(error) => {
            let duration = error.duration();
            OffsetDateTime::UNIX_EPOCH
                - TimeDuration::new(duration.as_secs() as i64, duration.subsec_nanos() as i32)
        }
    };

    format!(
        "{:04}-{}-{:02} {:02}:{:02}:{:02}",
        date_time.year(),
        short_month(date_time.month()),
        date_time.day(),
        date_time.hour(),
        date_time.minute(),
        date_time.second()
    )
}

fn short_month(month: Month) -> &'static str {
    match month {
        Month::January => "Jan",
        Month::February => "Feb",
        Month::March => "Mar",
        Month::April => "Apr",
        Month::May => "May",
        Month::June => "Jun",
        Month::July => "Jul",
        Month::August => "Aug",
        Month::September => "Sep",
        Month::October => "Oct",
        Month::November => "Nov",
        Month::December => "Dec",
    }
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::time::{Duration, SystemTime};

    use crate::validator::validator_list::{ListDisposition, ValidatorBlobInfo};
    use protocol::JsonValue;

    use super::{
        DEFAULT_REFRESH_INTERVAL, PublisherListStats, ReqwestValidatorSiteTransport, SiteResource,
        SiteResponse, ValidatorSite, ValidatorSiteSink, ValidatorSiteTransport,
    };

    #[derive(Default)]
    struct FakeSink {
        calls: Vec<(String, u32, Vec<ValidatorBlobInfo>)>,
        cached: Vec<String>,
    }

    impl ValidatorSiteSink for FakeSink {
        fn apply_lists(
            &mut self,
            manifest: &str,
            version: u32,
            blobs: &[ValidatorBlobInfo],
            _site_uri: String,
            _hash: basics::base_uint::Uint256,
        ) -> PublisherListStats {
            self.calls
                .push((manifest.to_owned(), version, blobs.to_vec()));
            PublisherListStats::new(ListDisposition::Accepted)
        }

        fn load_lists(&self) -> Vec<String> {
            self.cached.clone()
        }
    }

    struct QueueTransport(std::sync::Mutex<VecDeque<Result<SiteResponse, String>>>);

    impl ValidatorSiteTransport for QueueTransport {
        fn fetch(
            &self,
            _resource: &SiteResource,
            _timeout: Duration,
        ) -> Result<SiteResponse, String> {
            self.0
                .lock()
                .expect("transport lock")
                .pop_front()
                .expect("queued response")
        }
    }

    fn valid_body() -> String {
        serde_json::json!({
            "manifest": "ZmFrZQ==",
            "version": 1,
            "blob": "Zm9v",
            "signature": "AABB"
        })
        .to_string()
    }

    #[test]
    fn validator_site_load_rejects_invalid_file_hostnames() {
        let site = ValidatorSite::new(Duration::from_secs(20));
        assert!(!site.load(&["file://localhost/home/user/vl.txt".to_owned()]));
        assert!(site.load(&["https://ripple.com/validators".to_owned()]));
    }

    #[test]
    fn validator_site_applies_valid_json_response() {
        let transport = QueueTransport(std::sync::Mutex::new(VecDeque::from([Ok(SiteResponse {
            status: 200,
            location: None,
            body: valid_body(),
        })])));
        let site = ValidatorSite::new(Duration::from_secs(20));
        assert!(site.load(&["https://ripple.com/validators".to_owned()]));
        let mut sink = FakeSink::default();
        site.refresh_due(&mut sink, &transport, SystemTime::now());
        assert_eq!(sink.calls.len(), 1);
    }

    #[test]
    fn validator_site_tracks_redirects_and_retry_interval() {
        let body = serde_json::json!({
            "manifest": "ZmFrZQ==",
            "version": 1,
            "blob": "Zm9v",
            "signature": "AABB",
            "refresh_interval": 17
        })
        .to_string();
        let transport = QueueTransport(std::sync::Mutex::new(VecDeque::from([
            Ok(SiteResponse {
                status: 301,
                location: Some("https://alt.ripple.com/validators".to_owned()),
                body: String::new(),
            }),
            Ok(SiteResponse {
                status: 200,
                location: None,
                body,
            }),
        ])));
        let site = ValidatorSite::new(Duration::from_secs(20));
        assert!(site.load(&["https://ripple.com/validators".to_owned()]));
        let mut sink = FakeSink::default();
        site.refresh_due(&mut sink, &transport, SystemTime::now());
        let JsonValue::Object(root) = site.get_json() else {
            panic!("validator_sites root must be object");
        };
        let JsonValue::Array(entries) = root.get("validator_sites").expect("validator_sites")
        else {
            panic!("validator_sites must be array");
        };
        let JsonValue::Object(entry) = &entries[0] else {
            panic!("validator site entry must be object");
        };
        assert_eq!(
            entry.get("refresh_interval_min"),
            Some(&JsonValue::Unsigned(17))
        );
        assert_eq!(sink.calls.len(), 1);
        assert!(DEFAULT_REFRESH_INTERVAL.as_secs() > 0);
        let _ = ReqwestValidatorSiteTransport;
    }

    #[test]
    fn validator_site_loads_cached_file_after_endpoint_failure() {
        let site = ValidatorSite::new(Duration::from_secs(20));
        assert!(site.load(&["https://ripple.com/validators".to_owned()]));
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(2_200_000_000);
        let transport = QueueTransport(std::sync::Mutex::new(VecDeque::from([
            Err("offline".to_owned()),
            Err("offline".to_owned()),
            Ok(SiteResponse {
                status: 200,
                location: None,
                body: valid_body(),
            }),
        ])));
        let mut sink = FakeSink {
            cached: vec!["file:///tmp/cached-validator-list.json".to_owned()],
            ..Default::default()
        };

        site.refresh_due(&mut sink, &transport, now);
        site.refresh_due(&mut sink, &transport, now + Duration::from_secs(30));

        assert_eq!(sink.calls.len(), 1, "cached list should be applied after failure");
    }
}
