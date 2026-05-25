use basics::chrono::ManualStopwatch;
use resource::{
    Charge, Disposition, JournalLevel, NullCollector, NullJournal, ResourceJournal, ResourceManager,
};
use std::sync::{Arc, Mutex};
use std::time::Instant;

#[derive(Default)]
struct RecordingJournal {
    entries: Mutex<Vec<(JournalLevel, String)>>,
}

impl RecordingJournal {
    fn contains(&self, level: JournalLevel, needle: &str) -> bool {
        self.entries
            .lock()
            .expect("journal mutex poisoned")
            .iter()
            .any(|(entry_level, message)| *entry_level == level && message.contains(needle))
    }
}

impl ResourceJournal for RecordingJournal {
    fn log(&self, level: JournalLevel, message: &str) {
        self.entries
            .lock()
            .expect("journal mutex poisoned")
            .push((level, message.to_owned()));
    }
}

fn new_manager_with_clock(
    clock: Arc<ManualStopwatch>,
    journal: Arc<dyn ResourceJournal>,
) -> ResourceManager {
    ResourceManager::new_with_clock(Arc::new(NullCollector), journal, clock)
}

#[test]
fn consumer_clone_and_drop_keep_refcount_and_inactive_lists() {
    let manager =
        new_manager_with_clock(Arc::new(ManualStopwatch::default()), Arc::new(NullJournal));
    let address = "127.0.0.1:51234".parse().expect("endpoint should parse");

    let consumer = manager.new_inbound_endpoint(address);
    let clone = consumer.clone();

    let json = manager.on_write();
    assert_eq!(json["inbound"][0]["count"], 2);

    drop(clone);
    let json = manager.on_write();
    assert_eq!(json["inbound"][0]["count"], 1);

    drop(consumer);
    let json = manager.on_write();
    assert_eq!(json["inactive"][0]["count"].as_u64(), None);
    assert_eq!(json["inactive"][0]["name"], "IP Address: 127.0.0.1:0");
}

#[test]
fn proxied_inbound_endpoint_uses_forwarded_ip_and_logs_invalid_proxy_data() {
    let journal = Arc::new(RecordingJournal::default());
    let manager = new_manager_with_clock(Arc::new(ManualStopwatch::default()), journal.clone());
    let proxy = "10.0.0.9:443".parse().expect("proxy should parse");

    let forwarded = manager.new_inbound_endpoint_with_proxy(proxy, true, "203.0.113.1");
    assert_eq!(forwarded.to_string(), "IP Address: 203.0.113.1:0");

    let invalid = manager.new_inbound_endpoint_with_proxy(proxy, true, "not-an-ip");
    assert_eq!(invalid.to_string(), "IP Address: 10.0.0.9:0");
    assert!(journal.contains(
        JournalLevel::Warn,
        "forwarded for (not-an-ip) from proxy 10.0.0.9:443 doesn't convert to IP endpoint"
    ));
}

#[test]
fn unlimited_consumers_keep_admin_fingerprint_and_bypass_resource_penalties() {
    let start = Instant::now();
    let clock = Arc::new(ManualStopwatch::new(start));
    let manager = new_manager_with_clock(clock, Arc::new(NullJournal));
    let address = "127.0.0.1:7000".parse().expect("endpoint should parse");

    let consumer = manager.new_unlimited_endpoint(address);
    assert!(consumer.is_unlimited());
    assert_eq!(consumer.to_string(), "IP Address: 127.0.0.1:1");
    assert_eq!(
        consumer.charge(Charge::new(900_000, "ignored")),
        Disposition::Ok
    );
    assert_eq!(consumer.disposition(), Disposition::Ok);
    assert!(!consumer.warn());
    assert!(!consumer.disconnect_with_manager_journal());

    let json = manager.on_write();
    assert_eq!(json["admin"][0]["name"], "IP Address: 127.0.0.1:1");
    assert_eq!(json["admin"][0]["balance"], 0);
}
