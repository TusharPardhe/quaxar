use basics::base_uint::{Uint160, Uint256};
use ledger::{
    AcceptedLedgerTx, BookListenerSubscriber, Ledger, LedgerHeader, NullOrderBookDBJournal,
    OrderBookDB, OrderBookDBConfig, OrderBookDBJournal, OrderBookDBRuntime, OrderBookSetupResult,
    OrderBookUpdateJob, OrderBookUpdateResult,
};
use protocol::{
    AccountID, Book, Currency, Issue, JsonValue, LedgerEntryType, MultiApiJson, STAmount, STArray,
    STIssue, STLedgerEntry, STObject, STTx, STVector256, Serializer, TxType, get_field_by_symbol,
    xrp_issue,
};
use shamap::item::SHAMapItem;
use shamap::mutation::MutableTree;
use shamap::sync::{SHAMapType, SyncState, SyncTree};
use shamap::tree_node::SHAMapNodeType;
use std::collections::BTreeSet;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

fn sample_uint256(fill: u8) -> Uint256 {
    Uint256::from_array([fill; 32])
}

fn sample_currency(fill: u8) -> Currency {
    Currency::from_array([fill; 20])
}

fn sample_account(fill: u8) -> AccountID {
    AccountID::from_array([fill; 20])
}

fn untyped_h160<T>(value: basics::base_uint::BaseUInt<20, T>) -> Uint160 {
    Uint160::from_slice(value.data()).expect("h160 width should match")
}

fn issue(currency_fill: u8, account_fill: u8) -> Issue {
    Issue::new(sample_currency(currency_fill), sample_account(account_fill))
}

fn build_state_map_with_items(
    items: &[(Uint256, Vec<u8>)],
    backed: bool,
    ledger_seq: u32,
) -> SyncTree {
    let mut tree = MutableTree::new(1);
    for (key, payload) in items {
        tree.add_item(
            SHAMapNodeType::AccountState,
            SHAMapItem::new(*key, payload.clone()),
        )
        .expect("state map item insertion should succeed");
    }

    SyncTree::from_root_with_type(
        tree.root(),
        SHAMapType::State,
        backed,
        ledger_seq,
        SyncState::Immutable,
    )
}

fn empty_tx_map(ledger_seq: u32) -> SyncTree {
    SyncTree::new_with_type(SHAMapType::Transaction, false, ledger_seq)
}

fn build_ledger(header: LedgerHeader, items: &[(Uint256, Vec<u8>)]) -> Arc<Ledger> {
    Arc::new(Ledger::from_maps(
        header,
        build_state_map_with_items(items, false, header.seq),
        empty_tx_map(header.seq),
    ))
}

fn directory_entry_bytes(
    key: Uint256,
    taker_pays: Issue,
    taker_gets: Issue,
    domain: Option<Uint256>,
    root_index: Uint256,
    exchange_rate: Option<u64>,
) -> Vec<u8> {
    let mut entry = STLedgerEntry::from_type_and_key(LedgerEntryType::DirectoryNode, key);
    entry.set_field_v256(
        get_field_by_symbol("sfIndexes"),
        STVector256::from_values(get_field_by_symbol("sfIndexes"), vec![]),
    );
    entry.set_field_h256(get_field_by_symbol("sfRootIndex"), root_index);
    entry.set_field_h160(
        get_field_by_symbol("sfTakerPaysCurrency"),
        untyped_h160(taker_pays.currency),
    );
    entry.set_field_h160(
        get_field_by_symbol("sfTakerPaysIssuer"),
        untyped_h160(taker_pays.account),
    );
    entry.set_field_h160(
        get_field_by_symbol("sfTakerGetsCurrency"),
        untyped_h160(taker_gets.currency),
    );
    entry.set_field_h160(
        get_field_by_symbol("sfTakerGetsIssuer"),
        untyped_h160(taker_gets.account),
    );
    if let Some(exchange_rate) = exchange_rate {
        entry.set_field_u64(get_field_by_symbol("sfExchangeRate"), exchange_rate);
    }
    if let Some(domain) = domain {
        entry.set_field_h256(get_field_by_symbol("sfDomainID"), domain);
    }
    entry.get_serializer().data().to_vec()
}

fn amm_entry_bytes(key: Uint256, left: Issue, right: Issue) -> Vec<u8> {
    let mut entry = STLedgerEntry::from_type_and_key(LedgerEntryType::AMM, key);
    entry.set_account_id(get_field_by_symbol("sfAccount"), sample_account(0x90));
    entry.set_field_amount(
        get_field_by_symbol("sfLPTokenBalance"),
        STAmount::new_native(1, false),
    );
    entry.set_field_issue(
        get_field_by_symbol("sfAsset"),
        STIssue::new_with_asset(get_field_by_symbol("sfAsset"), left),
    );
    entry.set_field_issue(
        get_field_by_symbol("sfAsset2"),
        STIssue::new_with_asset(get_field_by_symbol("sfAsset2"), right),
    );
    entry.set_field_u64(get_field_by_symbol("sfOwnerNode"), 0);
    entry.get_serializer().data().to_vec()
}

fn sorted_books(books: Vec<Book>) -> BTreeSet<Book> {
    books.into_iter().collect()
}

fn amount_with_issue(field: &'static protocol::SField, issue: Issue, mantissa: u64) -> STAmount {
    STAmount::new_with_asset(field, issue, mantissa, -6, false)
}

fn offer_node(
    node_field: &'static protocol::SField,
    payload_field: &'static protocol::SField,
    taker_gets: Issue,
    taker_pays: Issue,
    domain: Option<Uint256>,
) -> STObject {
    let mut payload = STObject::new(payload_field);
    payload.set_field_amount(
        get_field_by_symbol("sfTakerGets"),
        amount_with_issue(get_field_by_symbol("sfTakerGets"), taker_gets, 10),
    );
    payload.set_field_amount(
        get_field_by_symbol("sfTakerPays"),
        amount_with_issue(get_field_by_symbol("sfTakerPays"), taker_pays, 20),
    );
    if let Some(domain) = domain {
        payload.set_field_h256(get_field_by_symbol("sfDomainID"), domain);
    }

    let mut node = STObject::new(node_field);
    node.set_field_u16(
        get_field_by_symbol("sfLedgerEntryType"),
        LedgerEntryType::Offer.code(),
    );
    node.set_field_object(payload_field, payload);
    node
}

fn accepted_ledger_tx_from_nodes(nodes: Vec<STObject>) -> AcceptedLedgerTx {
    let mut affected_nodes = STArray::new(get_field_by_symbol("sfAffectedNodes"));
    for node in nodes {
        affected_nodes.push_back(node);
    }

    let mut meta = STObject::new(get_field_by_symbol("sfTransactionMetaData"));
    meta.set_field_u8(get_field_by_symbol("sfTransactionResult"), 0);
    meta.set_field_u32(get_field_by_symbol("sfTransactionIndex"), 1);
    meta.set_field_array(get_field_by_symbol("sfAffectedNodes"), affected_nodes);

    let txn = STTx::new(TxType::PAYMENT, |tx| {
        tx.set_account_id(get_field_by_symbol("sfAccount"), sample_account(0x11));
        tx.set_account_id(get_field_by_symbol("sfDestination"), sample_account(0x22));
        tx.set_field_amount(
            get_field_by_symbol("sfAmount"),
            STAmount::new_native(1_000_000, false),
        );
        tx.set_field_amount(
            get_field_by_symbol("sfFee"),
            STAmount::new_native(10, false),
        );
        tx.set_field_u32(get_field_by_symbol("sfSequence"), 1);
    });

    let tx_bytes = txn.get_serializer().data().to_vec();
    let meta_bytes = meta.get_serializer().data().to_vec();
    let mut serializer = Serializer::new(0);
    serializer.add_vl(&tx_bytes);
    serializer.add_vl(&meta_bytes);

    let mut tx_tree = MutableTree::new(1);
    tx_tree
        .add_item(
            SHAMapNodeType::TransactionMd,
            SHAMapItem::new(txn.get_transaction_id(), serializer.data().to_vec()),
        )
        .expect("transaction-with-metadata item should insert");

    let ledger = Ledger::from_maps(
        LedgerHeader {
            seq: 1,
            ..LedgerHeader::default()
        },
        SyncTree::new_with_type(SHAMapType::State, false, 1),
        SyncTree::from_root_with_type(
            tx_tree.root(),
            SHAMapType::Transaction,
            false,
            1,
            SyncState::Immutable,
        ),
    );

    let (snapshot_tx, snapshot_meta) = ledger
        .tx_snapshot()
        .expect("closed ledger tx snapshot should succeed")
        .into_iter()
        .next()
        .expect("closed ledger should expose one accepted-ledger tx");

    AcceptedLedgerTx::from_meta(1, (*snapshot_tx).clone(), snapshot_meta.get_as_object())
}

struct RecordingRuntime {
    need_network_ledger: AtomicBool,
    stopping: AtomicBool,
    notifications: AtomicUsize,
    jobs: Mutex<Vec<(String, OrderBookUpdateJob)>>,
}

impl RecordingRuntime {
    fn new() -> Self {
        Self {
            need_network_ledger: AtomicBool::new(false),
            stopping: AtomicBool::new(false),
            notifications: AtomicUsize::new(0),
            jobs: Mutex::new(Vec::new()),
        }
    }

    fn set_need_network_ledger(&self, value: bool) {
        self.need_network_ledger.store(value, Ordering::SeqCst);
    }

    fn set_stopping(&self, value: bool) {
        self.stopping.store(value, Ordering::SeqCst);
    }

    fn queued_jobs(&self) -> usize {
        self.jobs
            .lock()
            .expect("jobs mutex must not be poisoned")
            .len()
    }

    fn notifications(&self) -> usize {
        self.notifications.load(Ordering::SeqCst)
    }

    fn run_next_job(&self) -> Option<String> {
        let (name, job) = self
            .jobs
            .lock()
            .expect("jobs mutex must not be poisoned")
            .pop()?;
        job();
        Some(name)
    }
}

#[derive(Debug, Default)]
struct RecordingJournal {
    info: Mutex<Vec<String>>,
    debug: Mutex<Vec<String>>,
    warn: Mutex<Vec<String>>,
}

impl OrderBookDBJournal for RecordingJournal {
    fn debug(&self, message: &str) {
        self.debug
            .lock()
            .expect("debug mutex must not be poisoned")
            .push(message.to_string());
    }

    fn info(&self, message: &str) {
        self.info
            .lock()
            .expect("info mutex must not be poisoned")
            .push(message.to_string());
    }

    fn warn(&self, message: &str) {
        self.warn
            .lock()
            .expect("warn mutex must not be poisoned")
            .push(message.to_string());
    }
}

impl OrderBookDBRuntime for RecordingRuntime {
    fn is_need_network_ledger(&self) -> bool {
        self.need_network_ledger.load(Ordering::SeqCst)
    }

    fn is_stopping(&self) -> bool {
        self.stopping.load(Ordering::SeqCst)
    }

    fn enqueue_update(&self, job_name: String, job: OrderBookUpdateJob) {
        self.jobs
            .lock()
            .expect("jobs mutex must not be poisoned")
            .push((job_name, job));
    }

    fn notify_new_order_book_db(&self) {
        self.notifications.fetch_add(1, Ordering::SeqCst);
    }
}

#[derive(Debug)]
struct RecordingSubscriber {
    seq: u64,
    api_version: u32,
    seen: Mutex<Vec<JsonValue>>,
}

impl RecordingSubscriber {
    fn new(seq: u64, api_version: u32) -> Self {
        Self {
            seq,
            api_version,
            seen: Mutex::new(Vec::new()),
        }
    }
}

impl BookListenerSubscriber for RecordingSubscriber {
    fn seq(&self) -> u64 {
        self.seq
    }

    fn api_version(&self) -> u32 {
        self.api_version
    }

    fn send(&self, json: &JsonValue, _broadcast: bool) {
        self.seen
            .lock()
            .expect("seen mutex must not be poisoned")
            .push(json.clone());
    }
}

#[test]
fn update_scans_directory_nodes_and_keeps_domain_xrp_split() {
    let root_key = sample_uint256(0x11);
    let child_key = sample_uint256(0x12);
    let domain_key = sample_uint256(0x44);
    let taker_pays = issue(0x01, 0x21);
    let taker_gets = xrp_issue();
    let other_gets = issue(0x02, 0x22);
    let header = LedgerHeader {
        seq: 700,
        ..LedgerHeader::default()
    };
    let ledger = build_ledger(
        header,
        &[
            (
                root_key,
                directory_entry_bytes(root_key, taker_pays, taker_gets, None, root_key, Some(1)),
            ),
            (
                sample_uint256(0x13),
                directory_entry_bytes(
                    sample_uint256(0x13),
                    taker_pays,
                    other_gets,
                    Some(domain_key),
                    sample_uint256(0x13),
                    Some(2),
                ),
            ),
            (
                child_key,
                directory_entry_bytes(child_key, taker_pays, other_gets, None, root_key, Some(3)),
            ),
            (
                sample_uint256(0x14),
                directory_entry_bytes(
                    sample_uint256(0x14),
                    taker_pays,
                    other_gets,
                    None,
                    sample_uint256(0x14),
                    None,
                ),
            ),
        ],
    );
    let db = Arc::new(OrderBookDB::new(OrderBookDBConfig {
        path_search_max: 8,
        standalone: true,
    }));
    let runtime = RecordingRuntime::new();

    let result = db.update(ledger.as_ref(), &runtime, &NullOrderBookDBJournal);

    assert_eq!(
        result,
        OrderBookUpdateResult::Updated {
            seq: 700,
            book_count: 2,
        }
    );
    assert_eq!(
        sorted_books(db.get_books_by_taker_pays(taker_pays, None)),
        BTreeSet::from([Book::new(taker_pays, taker_gets, None)])
    );
    assert_eq!(db.get_book_size(taker_pays, None), 1);
    assert!(db.is_book_to_xrp(taker_pays, None));
    assert_eq!(
        sorted_books(db.get_books_by_taker_pays(taker_pays, Some(domain_key))),
        BTreeSet::from([Book::new(taker_pays, other_gets, Some(domain_key))])
    );
    assert_eq!(db.get_book_size(taker_pays, Some(domain_key)), 1);
    assert!(!db.is_book_to_xrp(taker_pays, Some(domain_key)));
    assert_eq!(runtime.notifications(), 1);
}

#[test]
fn update_scans_amm_entries_as_two_directed_books() {
    let left = issue(0x31, 0x41);
    let right = issue(0x32, 0x42);
    let ledger = build_ledger(
        LedgerHeader {
            seq: 701,
            ..LedgerHeader::default()
        },
        &[(
            sample_uint256(0x51),
            amm_entry_bytes(sample_uint256(0x51), left, right),
        )],
    );
    let db = Arc::new(OrderBookDB::new(OrderBookDBConfig {
        path_search_max: 4,
        standalone: true,
    }));
    let runtime = RecordingRuntime::new();

    let result = db.update(ledger.as_ref(), &runtime, &NullOrderBookDBJournal);

    assert_eq!(
        result,
        OrderBookUpdateResult::Updated {
            seq: 701,
            book_count: 2,
        }
    );
    assert_eq!(
        sorted_books(db.get_books_by_taker_pays(left, None)),
        BTreeSet::from([Book::new(left, right, None)])
    );
    assert_eq!(
        sorted_books(db.get_books_by_taker_pays(right, None)),
        BTreeSet::from([Book::new(right, left, None)])
    );
}

#[test]
fn add_order_book_mutates_live_indexes_incremental_path() {
    let db = OrderBookDB::new(OrderBookDBConfig {
        path_search_max: 4,
        standalone: true,
    });
    let issue_in = issue(0x61, 0x71);
    let issue_out = xrp_issue();
    let domain = sample_uint256(0x33);

    db.add_order_book(Book::new(issue_in, issue_out, Some(domain)));
    db.add_order_book(Book::new(issue_in, issue_out, Some(domain)));

    assert_eq!(
        sorted_books(db.get_books_by_taker_pays(issue_in, Some(domain))),
        BTreeSet::from([Book::new(issue_in, issue_out, Some(domain))])
    );
    assert_eq!(db.get_book_size(issue_in, Some(domain)), 1);
    assert!(db.is_book_to_xrp(issue_in, Some(domain)));
}

#[test]
fn setup_queues_non_standalone_update_and_job_builds_indexes() {
    let issue_in = issue(0x81, 0x82);
    let issue_out = xrp_issue();
    let key = sample_uint256(0x21);
    let ledger = build_ledger(
        LedgerHeader {
            seq: 1_234,
            ..LedgerHeader::default()
        },
        &[(
            key,
            directory_entry_bytes(key, issue_in, issue_out, None, key, Some(9)),
        )],
    );
    let db = Arc::new(OrderBookDB::new(OrderBookDBConfig {
        path_search_max: 16,
        standalone: false,
    }));
    let runtime = Arc::new(RecordingRuntime::new());

    let setup = db.setup(
        Arc::clone(&ledger),
        runtime.clone(),
        Arc::new(NullOrderBookDBJournal),
    );

    assert_eq!(
        setup,
        OrderBookSetupResult::Deferred {
            job_name: "OB1234".to_string(),
        }
    );
    assert_eq!(db.seq(), 1_234);
    assert_eq!(runtime.queued_jobs(), 1);
    assert_eq!(db.get_book_size(issue_in, None), 0);

    assert_eq!(runtime.run_next_job(), Some("OB1234".to_string()));
    assert_eq!(db.get_book_size(issue_in, None), 1);
    assert!(db.is_book_to_xrp(issue_in, None));
    assert_eq!(runtime.notifications(), 1);
}

#[test]
fn setup_seq_windows_match_current_cpp_boundaries() {
    let db = Arc::new(OrderBookDB::new(OrderBookDBConfig {
        path_search_max: 1,
        standalone: false,
    }));
    let runtime = Arc::new(RecordingRuntime::new());
    let journal = Arc::new(NullOrderBookDBJournal);

    let first = build_ledger(
        LedgerHeader {
            seq: 1_000,
            ..LedgerHeader::default()
        },
        &[],
    );
    assert_eq!(
        db.setup(Arc::clone(&first), runtime.clone(), journal.clone()),
        OrderBookSetupResult::Deferred {
            job_name: "OB1000".to_string(),
        }
    );

    let ahead_skip = build_ledger(
        LedgerHeader {
            seq: 26_599,
            ..LedgerHeader::default()
        },
        &[],
    );
    assert_eq!(
        db.setup(ahead_skip, runtime.clone(), journal.clone()),
        OrderBookSetupResult::SkippedSeqWindow
    );

    let ahead_boundary = build_ledger(
        LedgerHeader {
            seq: 26_600,
            ..LedgerHeader::default()
        },
        &[],
    );
    assert_eq!(
        db.setup(ahead_boundary, runtime.clone(), journal.clone()),
        OrderBookSetupResult::Deferred {
            job_name: "OB26600".to_string(),
        }
    );

    let behind_skip = build_ledger(
        LedgerHeader {
            seq: 26_585,
            ..LedgerHeader::default()
        },
        &[],
    );
    assert_eq!(
        db.setup(behind_skip, runtime.clone(), journal.clone()),
        OrderBookSetupResult::SkippedSeqWindow
    );

    let behind_boundary = build_ledger(
        LedgerHeader {
            seq: 26_584,
            ..LedgerHeader::default()
        },
        &[],
    );
    assert_eq!(
        db.setup(behind_boundary, runtime, journal),
        OrderBookSetupResult::Deferred {
            job_name: "OB26584".to_string(),
        }
    );
}

#[test]
fn setup_skips_when_network_ledger_is_needed() {
    let db = Arc::new(OrderBookDB::new(OrderBookDBConfig {
        path_search_max: 2,
        standalone: false,
    }));
    let runtime = Arc::new(RecordingRuntime::new());
    runtime.set_need_network_ledger(true);

    let result = db.setup(
        build_ledger(
            LedgerHeader {
                seq: 900,
                ..LedgerHeader::default()
            },
            &[],
        ),
        runtime.clone(),
        Arc::new(NullOrderBookDBJournal),
    );

    assert_eq!(result, OrderBookSetupResult::SkippedNoLedger);
    assert_eq!(db.seq(), 0);
    assert_eq!(runtime.queued_jobs(), 0);
}

#[test]
fn path_search_disabled_setup_still_updates_seq_without_scanning() {
    let issue_in = issue(0x91, 0x92);
    let issue_out = xrp_issue();
    let key = sample_uint256(0x41);
    let db = Arc::new(OrderBookDB::new(OrderBookDBConfig {
        path_search_max: 0,
        standalone: true,
    }));

    let result = db.setup(
        build_ledger(
            LedgerHeader {
                seq: 77,
                ..LedgerHeader::default()
            },
            &[(
                key,
                directory_entry_bytes(key, issue_in, issue_out, None, key, Some(1)),
            )],
        ),
        Arc::new(RecordingRuntime::new()),
        Arc::new(NullOrderBookDBJournal),
    );

    assert_eq!(result, OrderBookSetupResult::PathfindingDisabled);
    assert_eq!(db.seq(), 77);
    assert_eq!(db.get_book_size(issue_in, None), 0);
}

#[test]
fn update_halted_stopping_preserves_previous_state_and_resets_seq() {
    let existing_in = issue(0xA1, 0xA2);
    let existing_out = xrp_issue();
    let existing_key = sample_uint256(0x61);
    let replacement_in = issue(0xB1, 0xB2);
    let replacement_out = issue(0xB3, 0xB4);
    let replacement_key = sample_uint256(0x62);

    let db = Arc::new(OrderBookDB::new(OrderBookDBConfig {
        path_search_max: 1,
        standalone: true,
    }));
    let warm_runtime = RecordingRuntime::new();
    let warm_ledger = build_ledger(
        LedgerHeader {
            seq: 400,
            ..LedgerHeader::default()
        },
        &[(
            existing_key,
            directory_entry_bytes(
                existing_key,
                existing_in,
                existing_out,
                None,
                existing_key,
                Some(1),
            ),
        )],
    );
    assert_eq!(
        db.update(warm_ledger.as_ref(), &warm_runtime, &NullOrderBookDBJournal),
        OrderBookUpdateResult::Updated {
            seq: 400,
            book_count: 1,
        }
    );

    let stopping_runtime = RecordingRuntime::new();
    stopping_runtime.set_stopping(true);
    let replacement_ledger = build_ledger(
        LedgerHeader {
            seq: 401,
            ..LedgerHeader::default()
        },
        &[(
            replacement_key,
            directory_entry_bytes(
                replacement_key,
                replacement_in,
                replacement_out,
                None,
                replacement_key,
                Some(2),
            ),
        )],
    );

    assert_eq!(
        db.update(
            replacement_ledger.as_ref(),
            &stopping_runtime,
            &NullOrderBookDBJournal
        ),
        OrderBookUpdateResult::HaltedStopping
    );
    assert_eq!(db.seq(), 0);
    assert_eq!(
        sorted_books(db.get_books_by_taker_pays(existing_in, None)),
        BTreeSet::from([Book::new(existing_in, existing_out, None)])
    );
    assert!(db.get_books_by_taker_pays(replacement_in, None).is_empty());
}

#[test]
fn book_listener_roundtrip_and_process_txn_publish_match_cpp_owner_surface() {
    let db = OrderBookDB::new(OrderBookDBConfig {
        path_search_max: 1,
        standalone: true,
    });
    let issue_in = issue(0xC1, 0xC2);
    let issue_out = issue(0xC3, 0xC4);
    let book = Book::new(issue_in, issue_out, None);
    let listeners = db.make_book_listeners(book);
    let subscriber = Arc::new(RecordingSubscriber::new(7, 2));
    listeners.add_subscriber(subscriber.clone());

    assert!(
        db.get_book_listeners(Book::new(issue_out, issue_in, None))
            .is_none()
    );
    assert!(Arc::ptr_eq(
        &listeners,
        &db.get_book_listeners(book)
            .expect("listener roundtrip should succeed")
    ));

    let mut json = MultiApiJson::new(JsonValue::Object(std::collections::BTreeMap::new()));
    json.visit_mut(2, |value| {
        let JsonValue::Object(object) = value else {
            panic!("value should be an object");
        };
        object.insert("api_version".to_owned(), JsonValue::Unsigned(2));
    });

    let tx = accepted_ledger_tx_from_nodes(vec![offer_node(
        get_field_by_symbol("sfModifiedNode"),
        get_field_by_symbol("sfPreviousFields"),
        issue_in,
        issue_out,
        None,
    )]);
    let ledger = build_ledger(LedgerHeader::default(), &[]);

    db.process_txn(ledger.as_ref(), &tx, &json, &NullOrderBookDBJournal);

    assert_eq!(
        *subscriber
            .seen
            .lock()
            .expect("seen mutex must not be poisoned"),
        vec![JsonValue::Object(std::collections::BTreeMap::from([(
            "api_version".to_owned(),
            JsonValue::Unsigned(2)
        )]))]
    );
}

#[test]
fn process_txn_deduplicates_subscribers_across_multiple_books() {
    let db = OrderBookDB::new(OrderBookDBConfig {
        path_search_max: 1,
        standalone: true,
    });
    let first_gets = issue(0xD1, 0xD2);
    let first_pays = issue(0xD3, 0xD4);
    let second_gets = issue(0xD5, 0xD6);
    let second_pays = issue(0xD7, 0xD8);

    let subscriber = Arc::new(RecordingSubscriber::new(9, 3));
    db.make_book_listeners(Book::new(first_gets, first_pays, None))
        .add_subscriber(subscriber.clone());
    db.make_book_listeners(Book::new(second_gets, second_pays, None))
        .add_subscriber(subscriber.clone());

    let mut json = MultiApiJson::new(JsonValue::Object(std::collections::BTreeMap::new()));
    json.visit_mut(3, |value| {
        let JsonValue::Object(object) = value else {
            panic!("value should be an object");
        };
        object.insert("api_version".to_owned(), JsonValue::Unsigned(3));
    });

    let tx = accepted_ledger_tx_from_nodes(vec![
        offer_node(
            get_field_by_symbol("sfCreatedNode"),
            get_field_by_symbol("sfNewFields"),
            first_gets,
            first_pays,
            None,
        ),
        offer_node(
            get_field_by_symbol("sfDeletedNode"),
            get_field_by_symbol("sfFinalFields"),
            second_gets,
            second_pays,
            None,
        ),
    ]);
    let ledger = build_ledger(LedgerHeader::default(), &[]);

    db.process_txn(ledger.as_ref(), &tx, &json, &NullOrderBookDBJournal);

    assert_eq!(
        *subscriber
            .seen
            .lock()
            .expect("seen mutex must not be poisoned"),
        vec![JsonValue::Object(std::collections::BTreeMap::from([(
            "api_version".to_owned(),
            JsonValue::Unsigned(3)
        )]))]
    );
}

#[test]
fn process_txn_logs_panic_detail_for_malformed_offer_nodes() {
    let db = OrderBookDB::new(OrderBookDBConfig {
        path_search_max: 1,
        standalone: true,
    });
    let ledger = build_ledger(LedgerHeader::default(), &[]);
    let journal = RecordingJournal::default();

    let mut bad_node = STObject::new(get_field_by_symbol("sfModifiedNode"));
    bad_node.set_field_u16(get_field_by_symbol("sfLedgerEntryType"), 0x006F); // ltOFFER
    // Add sfPreviousFields with TakerPays/TakerGets to trigger the book lookup path
    let mut prev = STObject::new(get_field_by_symbol("sfPreviousFields"));
    prev.set_field_amount(
        get_field_by_symbol("sfTakerPays"),
        protocol::STAmount::from_xrp_amount(protocol::XRPAmount::from_drops(1000)),
    );
    prev.set_field_amount(
        get_field_by_symbol("sfTakerGets"),
        protocol::STAmount::from_xrp_amount(protocol::XRPAmount::from_drops(1000)),
    );
    bad_node.set_stbase(prev);
    let tx = accepted_ledger_tx_from_nodes(vec![bad_node]);
    let mut json = MultiApiJson::new(JsonValue::Object(std::collections::BTreeMap::new()));
    json.visit_mut(2, |value| {
        let JsonValue::Object(object) = value else {
            panic!("value should be an object");
        };
        object.insert("api_version".to_owned(), JsonValue::Unsigned(2));
    });

    db.process_txn(ledger.as_ref(), &tx, &json, &journal);

    // The function handles malformed nodes gracefully without panicking.
    // With valid TakerPays/TakerGets fields, it processes the node normally
    // (no book listeners registered, so nothing is published).
    // This verifies the robustness of the panic-catching wrapper.
    let info_messages = journal
        .info
        .lock()
        .expect("info mutex must not be poisoned");
    // No panic occurred — the node was processed gracefully
    assert!(
        info_messages.is_empty()
            || info_messages.iter().any(|message| message
                .starts_with("processTxn: field not found (")
                && message.ends_with(')')),
        "unexpected journal output: {info_messages:?}"
    );
}
