use basics::chrono::ManualStopwatch;
use resource::{
    Charge, Disposition, FEE_DROP, FEE_EXCEPTION_RPC, FEE_HEAVY_BURDEN_PEER, FEE_HEAVY_BURDEN_RPC,
    FEE_INVALID_DATA, FEE_INVALID_SIGNATURE, FEE_LOG_AS_DEBUG, FEE_LOG_AS_INFO, FEE_LOG_AS_WARN,
    FEE_MALFORMED_REQUEST, FEE_MALFORMED_RPC, FEE_MEDIUM_BURDEN_RPC, FEE_MODERATE_BURDEN_PEER,
    FEE_REFERENCE_RPC, FEE_REQUEST_NO_REPLY, FEE_TRIVIAL_PEER, FEE_USELESS_DATA, FEE_WARNING,
    Gossip, GossipItem, JournalLevel, NullCollector, ResourceJournal, ResourceManager,
    make_manager,
};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

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
fn exported_fee_schedule_fees_cpp() {
    let expected = [
        (&*FEE_MALFORMED_REQUEST, 200, "malformed request"),
        (&*FEE_REQUEST_NO_REPLY, 10, "unsatisfiable request"),
        (&*FEE_INVALID_SIGNATURE, 2_000, "invalid signature"),
        (&*FEE_USELESS_DATA, 150, "useless data"),
        (&*FEE_INVALID_DATA, 400, "invalid data"),
        (&*FEE_MALFORMED_RPC, 100, "malformed RPC"),
        (&*FEE_REFERENCE_RPC, 20, "reference RPC"),
        (&*FEE_EXCEPTION_RPC, 100, "exceptioned RPC"),
        (&*FEE_MEDIUM_BURDEN_RPC, 400, "medium RPC"),
        (&*FEE_HEAVY_BURDEN_RPC, 3_000, "heavy RPC"),
        (&*FEE_TRIVIAL_PEER, 1, "trivial peer request"),
        (&*FEE_MODERATE_BURDEN_PEER, 250, "moderate peer request"),
        (&*FEE_HEAVY_BURDEN_PEER, 2_000, "heavy peer request"),
        (&*FEE_WARNING, 4_000, "received warning"),
        (&*FEE_DROP, 6_000, "dropped"),
    ];

    for (charge, cost, label) in expected {
        assert_eq!(charge.cost(), cost);
        assert_eq!(charge.label(), label);
    }

    assert_eq!(FEE_LOG_AS_WARN, 3_000);
    assert_eq!(FEE_LOG_AS_INFO, 1_000);
    assert_eq!(FEE_LOG_AS_DEBUG, 100);
}

#[test]
fn charge_log_cutoffs_match_cpp_logic() {
    let journal = Arc::new(RecordingJournal::default());
    let manager = new_manager_with_clock(Arc::new(ManualStopwatch::default()), journal.clone());
    let consumer =
        manager.new_inbound_endpoint("127.0.0.1:51234".parse().expect("endpoint should parse"));

    assert_eq!(
        consumer.charge_with_context(Charge::new(99, "trace"), "trace ctx"),
        Disposition::Ok
    );
    assert_eq!(
        consumer.charge_with_context(Charge::new(100, "debug"), "debug ctx"),
        Disposition::Ok
    );
    assert_eq!(
        consumer.charge_with_context(Charge::new(1_000, "info"), "info ctx"),
        Disposition::Ok
    );
    assert_eq!(
        consumer.charge_with_context(Charge::new(3_000, "warn"), "warn ctx"),
        Disposition::Ok
    );

    assert!(journal.contains(
        JournalLevel::Trace,
        "Charging IP Address: 127.0.0.1:0 for trace ($99) (trace ctx)"
    ));
    assert!(journal.contains(
        JournalLevel::Debug,
        "Charging IP Address: 127.0.0.1:0 for debug ($100) (debug ctx)"
    ));
    assert!(journal.contains(
        JournalLevel::Info,
        "Charging IP Address: 127.0.0.1:0 for info ($1000) (info ctx)"
    ));
    assert!(journal.contains(
        JournalLevel::Warn,
        "Charging IP Address: 127.0.0.1:0 for warn ($3000) (warn ctx)"
    ));
}

#[test]
fn warning_consumes_fee_warning_once_per_clock_instant() {
    let start = Instant::now();
    let clock = Arc::new(ManualStopwatch::new(start));
    let journal = Arc::new(RecordingJournal::default());
    let manager = new_manager_with_clock(clock.clone(), journal.clone());
    let consumer =
        manager.new_inbound_endpoint("127.0.0.1:51234".parse().expect("endpoint should parse"));

    assert_eq!(
        consumer.charge(Charge::new(160_000, "warning threshold")),
        Disposition::Warn
    );
    assert_eq!(consumer.disposition(), Disposition::Warn);

    assert!(consumer.warn());
    assert!(!consumer.warn());
    assert_eq!(consumer.balance(), 5_125);
    assert!(journal.contains(JournalLevel::Info, "Load warning: IP Address: 127.0.0.1:0"));
    assert!(journal.contains(
        JournalLevel::Warn,
        "Charging IP Address: 127.0.0.1:0 for received warning ($4000)"
    ));

    clock.advance(Duration::from_secs(1));
    assert_eq!(consumer.disposition(), Disposition::Ok);
    assert!(!consumer.warn());
}

#[test]
fn dropped_consumer_stays_blacklisted_until_expiration_logic_test() {
    let start = Instant::now();
    let clock = Arc::new(ManualStopwatch::new(start));
    let journal = Arc::new(RecordingJournal::default());
    let manager = new_manager_with_clock(clock.clone(), journal.clone());
    let address = "127.0.0.1:51234".parse().expect("endpoint should parse");

    {
        let consumer = manager.new_inbound_endpoint(address);
        assert_eq!(
            consumer.charge(Charge::new(800_001, "drop")),
            Disposition::Drop
        );
        assert!(consumer.disconnect_with_manager_journal());
    }

    manager.periodic_activity();

    {
        let consumer = manager.new_inbound_endpoint(address);
        assert_eq!(consumer.disposition(), Disposition::Drop);
    }

    clock.advance(Duration::from_secs(301));
    manager.periodic_activity();

    let readmitted = manager.new_inbound_endpoint(address);
    assert_ne!(readmitted.disposition(), Disposition::Drop);
    assert!(journal.contains(
        JournalLevel::Warn,
        "Consumer entry IP Address: 127.0.0.1:0 dropped with balance 25000 at or above drop threshold 25000"
    ));
    assert!(journal.contains(
        JournalLevel::Warn,
        "Charging IP Address: 127.0.0.1:0 for dropped ($6000)"
    ));
    assert!(journal.contains(JournalLevel::Debug, "Expired IP Address: 127.0.0.1:0"));
}

#[test]
fn import_replacement_and_expiration_match_cpp_logic() {
    let start = Instant::now();
    let clock = Arc::new(ManualStopwatch::new(start));
    let journal = Arc::new(RecordingJournal::default());
    let manager = new_manager_with_clock(clock.clone(), journal);
    let address = "127.0.0.1:51234".parse().expect("endpoint should parse");

    manager.import_consumers(
        "peer",
        Gossip {
            items: vec![GossipItem::new(1_500, address)],
        },
    );
    assert_eq!(
        manager.get_json_with_threshold(0)["IP Address: 127.0.0.1:0"]["remote"],
        1_500
    );

    manager.import_consumers(
        "peer",
        Gossip {
            items: vec![GossipItem::new(2_000, address)],
        },
    );
    assert_eq!(
        manager.get_json_with_threshold(0)["IP Address: 127.0.0.1:0"]["remote"],
        2_000
    );

    clock.advance(Duration::from_secs(31));
    manager.periodic_activity();

    assert_eq!(
        manager
            .get_json_with_threshold(0)
            .as_object()
            .expect("json object")
            .len(),
        0
    );
}

#[test]
fn inbound_proxy_with_invalid_forwarded_for_falls_back() {
    let manager = make_manager(Arc::new(NullCollector), Arc::new(resource::NullJournal));
    let consumer = manager.new_inbound_endpoint_with_proxy(
        "127.0.0.1:51234".parse().expect("endpoint should parse"),
        true,
        "not-an-ip",
    );

    assert_eq!(consumer.to_string(), "IP Address: 127.0.0.1:0");
}
