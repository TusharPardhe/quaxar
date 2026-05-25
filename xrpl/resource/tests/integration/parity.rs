use resource::{
    Charge, Consumer, Disposition, JournalLevel, NullCollector, NullJournal, ResourceJournal,
    make_manager,
};
use std::sync::{Arc, Mutex};

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

const CPP_NODE_PUBLIC_KEY: [u8; 33] = [
    0x03, 0xD4, 0x9C, 0x56, 0xE1, 0xB1, 0x85, 0xF1, 0xBE, 0x89, 0x9A, 0xE6, 0x6A, 0x02, 0xEF, 0xC1,
    0x7F, 0x78, 0xEA, 0x6F, 0xC5, 0x3A, 0xF8, 0x5E, 0x0F, 0xE5, 0x4C, 0x6E, 0x8B, 0x7F, 0x8C, 0x71,
    0xA8,
];

const GENESIS_NODE_PUBLIC_BASE58: &str = "n94a1u4jAz288pZLtw6yFWVbi89YamiC6JBXPVUj5zmExe5fTVg9";

#[test]
fn charge_display_and_ordering() {
    let small = Charge::new(12, "RPC");
    let large = Charge::new(48, "RPC");

    assert_eq!(small.to_string(), "RPC ($12)");
    assert!(small < large);
}

#[test]
fn make_manager_reports_inbound_consumers() {
    let manager = make_manager(Arc::new(NullCollector), Arc::new(NullJournal));
    let consumer =
        manager.new_inbound_endpoint("127.0.0.1:51234".parse().expect("endpoint should parse"));

    assert_eq!(consumer.to_string(), "IP Address: 127.0.0.1:0");

    let json = manager.on_write();
    assert_eq!(json["inbound"].as_array().expect("inbound array").len(), 1);
    assert_eq!(json["inbound"][0]["name"], "IP Address: 127.0.0.1:0");
    assert_eq!(json["inbound"][0]["count"].as_u64().expect("count"), 1);
    assert_eq!(json["inbound"][0]["balance"].as_i64().expect("balance"), 0);
    assert_eq!(format!("{consumer}"), "IP Address: 127.0.0.1:0");
}

#[test]
fn set_public_key_uses_cpp_node_public_base58_in_fingerprint() {
    let manager = make_manager(Arc::new(NullCollector), Arc::new(NullJournal));
    let consumer =
        manager.new_inbound_endpoint("127.0.0.1:51234".parse().expect("endpoint should parse"));

    consumer.set_public_key(CPP_NODE_PUBLIC_KEY);

    assert_eq!(
        consumer.to_string(),
        format!("IP Address: 127.0.0.1:0, Public Key: {GENESIS_NODE_PUBLIC_BASE58}")
    );

    let json = manager.on_write();
    assert_eq!(
        json["inbound"][0]["name"],
        format!("IP Address: 127.0.0.1:0, Public Key: {GENESIS_NODE_PUBLIC_BASE58}")
    );
}

#[test]
fn default_consumer_empty_surface() {
    let consumer = Consumer::default();

    assert_eq!(consumer.to_string(), "(none)");
    assert!(!consumer.is_unlimited());
    assert_eq!(consumer.disposition(), Disposition::Ok);
    assert_eq!(consumer.charge(Charge::new(100, "RPC")), Disposition::Ok);
    consumer.elevate("named");
    assert_eq!(consumer.to_string(), "(none)");
}

#[test]
fn disconnect_logs_to_the_caller_supplied_journal() {
    let manager_journal = Arc::new(RecordingJournal::default());
    let caller_journal = Arc::new(RecordingJournal::default());
    let manager = make_manager(Arc::new(NullCollector), manager_journal.clone());
    let consumer =
        manager.new_inbound_endpoint("127.0.0.1:51234".parse().expect("endpoint should parse"));

    assert_eq!(
        consumer.charge(Charge::new(800_001, "abuse")),
        Disposition::Drop
    );
    assert!(consumer.disconnect(caller_journal.as_ref()));
    assert!(caller_journal.contains(JournalLevel::Debug, "disconnecting IP Address: 127.0.0.1:0"));
    assert!(
        !manager_journal.contains(JournalLevel::Debug, "disconnecting IP Address: 127.0.0.1:0")
    );
}
