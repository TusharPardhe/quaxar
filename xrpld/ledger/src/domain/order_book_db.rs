//! `OrderBookDBImpl` owner core from `xrpld/app/ledger/OrderBookDBImpl.*`.
//!
//! This module maintains the real book-indexing logic, sequence gating, hardened
//! container choices, and explicit runtime hooks.

use crate::{AcceptedLedgerTx, BookListeners, Ledger};
use basics::sha_map_hash::SHAMapHash;
use basics::unordered_containers::{HardenedHashMap, HardenedHashSet, HashMap, HashSet};
use protocol::{
    AccountID, Asset, Book, Currency, Domain, Issue, LedgerEntryType, MultiApiJson, STIssue,
    StBase, get_field_by_symbol,
};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

const RECENT_FORWARD_WINDOW: u32 = 25_600;
const RECENT_BACKWARD_WINDOW: u32 = 16;
const JOB_NAME_MODULUS: u32 = 1_000_000_000;

pub type OrderBookUpdateJob = Box<dyn FnOnce() + Send + 'static>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct OrderBookDBConfig {
    pub path_search_max: i32,
    pub standalone: bool,
}

pub trait OrderBookDBJournal: Send + Sync + 'static {
    fn debug(&self, message: &str);
    fn info(&self, message: &str);
    fn warn(&self, message: &str);
}

#[derive(Debug, Default)]
pub struct NullOrderBookDBJournal;

impl OrderBookDBJournal for NullOrderBookDBJournal {
    fn debug(&self, _message: &str) {}
    fn info(&self, _message: &str) {}
    fn warn(&self, _message: &str) {}
}

pub trait OrderBookDBRuntime: Send + Sync + 'static {
    fn is_need_network_ledger(&self) -> bool;
    fn is_stopping(&self) -> bool;
    fn enqueue_update(&self, job_name: String, job: OrderBookUpdateJob);
    fn notify_new_order_book_db(&self);
}

#[derive(Debug, Default)]
pub struct NullOrderBookDBRuntime;

impl OrderBookDBRuntime for NullOrderBookDBRuntime {
    fn is_need_network_ledger(&self) -> bool {
        false
    }

    fn is_stopping(&self) -> bool {
        false
    }

    fn enqueue_update(&self, _job_name: String, _job: OrderBookUpdateJob) {}

    fn notify_new_order_book_db(&self) {}
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OrderBookSetupResult {
    SkippedNoLedger,
    SkippedSeqWindow,
    SkippedRace,
    PathfindingDisabled,
    Updated(OrderBookUpdateResult),
    Deferred { job_name: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OrderBookUpdateResult {
    PathfindingDisabled,
    ElidedPendingLater { pending_seq: u32 },
    HaltedStopping,
    MissingNode { hash: SHAMapHash },
    Updated { seq: u32, book_count: usize },
}

type DomainKey = (Issue, Domain);
type IssueSet = HardenedHashSet<Issue>;
type BookListenersMap = HashMap<Book, Arc<BookListeners>>;

#[derive(Default)]
struct OrderBookIndexState {
    all_books: HardenedHashMap<Issue, IssueSet>,
    domain_books: HardenedHashMap<DomainKey, IssueSet>,
    xrp_books: HardenedHashSet<Issue>,
    xrp_domain_books: HardenedHashSet<DomainKey>,
}

pub struct OrderBookDB {
    path_search_max: i32,
    standalone: bool,
    state: Mutex<OrderBookIndexState>,
    listeners: Mutex<BookListenersMap>,
    seq: AtomicU32,
}

impl OrderBookDB {
    pub fn new(config: OrderBookDBConfig) -> Self {
        Self {
            path_search_max: config.path_search_max,
            standalone: config.standalone,
            state: Mutex::new(OrderBookIndexState::default()),
            listeners: Mutex::new(BookListenersMap::default()),
            seq: AtomicU32::new(0),
        }
    }

    pub fn setup(
        self: &Arc<Self>,
        ledger: Arc<Ledger>,
        runtime: Arc<dyn OrderBookDBRuntime>,
        journal: Arc<dyn OrderBookDBJournal>,
    ) -> OrderBookSetupResult {
        if !self.standalone && runtime.is_need_network_ledger() {
            journal.warn("Eliding full order book update: no ledger");
            return OrderBookSetupResult::SkippedNoLedger;
        }

        let ledger_seq = ledger.header().seq;
        let seq = self.seq.load(Ordering::SeqCst);

        if seq != 0 {
            if ledger_seq > seq && (ledger_seq - seq) < RECENT_FORWARD_WINDOW {
                return OrderBookSetupResult::SkippedSeqWindow;
            }

            if ledger_seq <= seq && (seq - ledger_seq) < RECENT_BACKWARD_WINDOW {
                return OrderBookSetupResult::SkippedSeqWindow;
            }
        }

        if self.seq.swap(ledger_seq, Ordering::SeqCst) != seq {
            return OrderBookSetupResult::SkippedRace;
        }

        journal.debug(&format!(
            "Full order book update: {} to {}",
            seq, ledger_seq
        ));

        if self.path_search_max == 0 {
            return OrderBookSetupResult::PathfindingDisabled;
        }

        if self.standalone {
            return OrderBookSetupResult::Updated(self.update(
                ledger.as_ref(),
                runtime.as_ref(),
                journal.as_ref(),
            ));
        }

        let job_name = format!("OB{}", ledger_seq % JOB_NAME_MODULUS);
        let db = Arc::clone(self);
        let job_runtime = Arc::clone(&runtime);
        let job_journal = Arc::clone(&journal);
        runtime.enqueue_update(
            job_name.clone(),
            Box::new(move || {
                db.update(ledger.as_ref(), job_runtime.as_ref(), job_journal.as_ref());
            }),
        );
        OrderBookSetupResult::Deferred { job_name }
    }

    pub fn update(
        &self,
        ledger: &Ledger,
        runtime: &dyn OrderBookDBRuntime,
        journal: &dyn OrderBookDBJournal,
    ) -> OrderBookUpdateResult {
        if self.path_search_max == 0 {
            return OrderBookUpdateResult::PathfindingDisabled;
        }

        let ledger_seq = ledger.header().seq;
        let seq = self.seq.load(Ordering::SeqCst);
        if seq > ledger_seq {
            let pending_seq = seq;
            journal.debug(&format!(
                "Eliding update for {} because of pending update to later {}",
                ledger_seq, pending_seq
            ));
            return OrderBookUpdateResult::ElidedPendingLater { pending_seq };
        }

        let mut next_state = OrderBookIndexState::default();
        {
            let state = self
                .state
                .lock()
                .expect("order book index mutex must not be poisoned");
            next_state.all_books.reserve(state.all_books.len());
            next_state.xrp_books.reserve(state.xrp_books.len());
        }

        journal.debug(&format!("Beginning update ({})", ledger_seq));

        let mut book_count = 0usize;
        let mut halted = false;
        let visit_result = ledger.visit_state_sles_while(&mut |sle| {
            if runtime.is_stopping() {
                journal.info("Update halted because the process is stopping");
                self.seq.store(0, Ordering::SeqCst);
                halted = true;
                return false;
            }

            book_count += add_order_books_from_sle(sle, &mut next_state);
            true
        });

        if halted {
            return OrderBookUpdateResult::HaltedStopping;
        }

        if let Err(err) = visit_result {
            match err {
                shamap::traversal::TraversalError::MissingNode(hash) => {
                    journal.info(&format!(
                        "Missing node in {} during update: Missing Node: State Tree: hash {}",
                        ledger_seq, hash
                    ));
                    self.seq.store(0, Ordering::SeqCst);
                    return OrderBookUpdateResult::MissingNode { hash };
                }
                _ => {
                    journal.info(&format!(
                        "Traversal error in {} during update: {:?}",
                        ledger_seq, err
                    ));
                    self.seq.store(0, Ordering::SeqCst);
                    return OrderBookUpdateResult::HaltedStopping;
                }
            }
        }

        journal.debug(&format!(
            "Update completed ({}): {} books found",
            ledger_seq, book_count
        ));

        {
            let mut state = self
                .state
                .lock()
                .expect("order book index mutex must not be poisoned");
            *state = next_state;
        }

        runtime.notify_new_order_book_db();
        OrderBookUpdateResult::Updated {
            seq: ledger_seq,
            book_count,
        }
    }

    pub fn add_order_book(&self, book: Book) {
        let mut state = self
            .state
            .lock()
            .expect("order book index mutex must not be poisoned");
        add_book(&mut state, book);
    }

    pub fn get_books_by_taker_pays(&self, issue: Issue, domain: Option<Domain>) -> Vec<Book> {
        let mut ret = Vec::new();
        let state = self
            .state
            .lock()
            .expect("order book index mutex must not be poisoned");

        let mut get_books = |books: &IssueSet| {
            ret.reserve(books.len());
            for gets in books {
                ret.push(Book::new(issue, *gets, domain));
            }
        };

        match domain {
            Some(domain) => {
                if let Some(books) = state.domain_books.get(&(issue, domain)) {
                    get_books(books);
                }
            }
            None => {
                if let Some(books) = state.all_books.get(&issue) {
                    get_books(books);
                }
            }
        }

        ret
    }

    pub fn get_book_size(&self, issue: Issue, domain: Option<Domain>) -> i32 {
        let state = self
            .state
            .lock()
            .expect("order book index mutex must not be poisoned");

        match domain {
            Some(domain) => state
                .domain_books
                .get(&(issue, domain))
                .map(|books| books.len() as i32)
                .unwrap_or(0),
            None => state
                .all_books
                .get(&issue)
                .map(|books| books.len() as i32)
                .unwrap_or(0),
        }
    }

    pub fn is_book_to_xrp(&self, issue: Issue, domain: Option<Domain>) -> bool {
        let state = self
            .state
            .lock()
            .expect("order book index mutex must not be poisoned");

        match domain {
            Some(domain) => state.xrp_domain_books.contains(&(issue, domain)),
            None => state.xrp_books.contains(&issue),
        }
    }

    pub fn seq(&self) -> u32 {
        self.seq.load(Ordering::SeqCst)
    }

    pub fn get_book_listeners(&self, book: Book) -> Option<Arc<BookListeners>> {
        self.listeners
            .lock()
            .expect("order book listeners mutex must not be poisoned")
            .get(&book)
            .cloned()
    }

    pub fn make_book_listeners(&self, book: Book) -> Arc<BookListeners> {
        let mut listeners = self
            .listeners
            .lock()
            .expect("order book listeners mutex must not be poisoned");
        listeners
            .entry(book)
            .or_insert_with(|| Arc::new(BookListeners::default()))
            .clone()
    }

    pub fn process_txn(
        &self,
        _ledger: &Ledger,
        al_tx: &AcceptedLedgerTx,
        jv_obj: &MultiApiJson,
        journal: &dyn OrderBookDBJournal,
    ) {
        let mut have_published = HashSet::default();

        for node in al_tx.get_meta().get_nodes().iter() {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                self.process_txn_node(node, jv_obj, &mut have_published);
            }));

            if let Err(payload) = result {
                let detail = unwind_message(payload)
                    .unwrap_or_else(|| "panic payload was not a string".to_string());
                journal.info(&format!("processTxn: field not found ({detail})"));
            }
        }
    }

    fn process_txn_node(
        &self,
        node: &protocol::STObject,
        jv_obj: &MultiApiJson,
        have_published: &mut HashSet<u64>,
    ) {
        if node.get_field_u16(get_field_by_symbol("sfLedgerEntryType"))
            != LedgerEntryType::Offer.code()
        {
            return;
        }

        let fields = match node.fname() {
            field if field == get_field_by_symbol("sfModifiedNode") => {
                Some(node.get_field_object(get_field_by_symbol("sfPreviousFields")))
            }
            field if field == get_field_by_symbol("sfCreatedNode") => {
                Some(node.get_field_object(get_field_by_symbol("sfNewFields")))
            }
            field if field == get_field_by_symbol("sfDeletedNode") => {
                Some(node.get_field_object(get_field_by_symbol("sfFinalFields")))
            }
            _ => None,
        };

        let Some(data) = fields else {
            return;
        };
        if !data.is_field_present(get_field_by_symbol("sfTakerPays"))
            || !data.is_field_present(get_field_by_symbol("sfTakerGets"))
        {
            return;
        }

        let book = Book::new(
            data.get_field_amount(get_field_by_symbol("sfTakerGets"))
                .issue(),
            data.get_field_amount(get_field_by_symbol("sfTakerPays"))
                .issue(),
            data.is_field_present(get_field_by_symbol("sfDomainID"))
                .then(|| data.get_field_h256(get_field_by_symbol("sfDomainID"))),
        );

        if let Some(listeners) = self.get_book_listeners(book) {
            listeners.publish(jv_obj, have_published);
        }
    }
}

fn unwind_message(payload: Box<dyn std::any::Any + Send>) -> Option<String> {
    match payload.downcast::<String>() {
        Ok(message) => Some(*message),
        Err(payload) => match payload.downcast::<&'static str>() {
            Ok(message) => Some((*message).to_string()),
            Err(_) => None,
        },
    }
}

fn add_order_books_from_sle(
    sle: &protocol::STLedgerEntry,
    state: &mut OrderBookIndexState,
) -> usize {
    match sle.get_type() {
        LedgerEntryType::DirectoryNode
            if sle.is_field_present(get_field_by_symbol("sfExchangeRate"))
                && sle.get_field_h256(get_field_by_symbol("sfRootIndex")) == *sle.key() =>
        {
            let book = Book::new(
                Issue::new(
                    currency_from_h160(
                        sle.get_field_h160(get_field_by_symbol("sfTakerPaysCurrency")),
                    ),
                    account_from_h160(sle.get_field_h160(get_field_by_symbol("sfTakerPaysIssuer"))),
                ),
                Issue::new(
                    currency_from_h160(
                        sle.get_field_h160(get_field_by_symbol("sfTakerGetsCurrency")),
                    ),
                    account_from_h160(sle.get_field_h160(get_field_by_symbol("sfTakerGetsIssuer"))),
                ),
                sle.is_field_present(get_field_by_symbol("sfDomainID"))
                    .then(|| sle.get_field_h256(get_field_by_symbol("sfDomainID"))),
            );
            add_book(state, book);
            1
        }
        LedgerEntryType::AMM => {
            let Some(issue1) =
                issue_from_stissue(sle.get_field_issue(get_field_by_symbol("sfAsset")))
            else {
                return 0;
            };
            let Some(issue2) =
                issue_from_stissue(sle.get_field_issue(get_field_by_symbol("sfAsset2")))
            else {
                return 0;
            };

            add_book(state, Book::new(issue1, issue2, None));
            add_book(state, Book::new(issue2, issue1, None));
            2
        }
        _ => 0,
    }
}

fn issue_from_stissue(issue: STIssue) -> Option<Issue> {
    match issue.asset() {
        Asset::Issue(issue) => Some(issue),
        Asset::MPTIssue(_) => None,
    }
}

fn currency_from_h160(value: basics::base_uint::Uint160) -> Currency {
    Currency::from_slice(value.data()).expect("currency width should match")
}

fn account_from_h160(value: basics::base_uint::Uint160) -> AccountID {
    AccountID::from_slice(value.data()).expect("account width should match")
}

fn add_book(state: &mut OrderBookIndexState, book: Book) {
    let to_xrp = book.out.native();

    match book.domain {
        Some(domain) => {
            state
                .domain_books
                .entry((book.r#in, domain))
                .or_default()
                .insert(book.out);
            if to_xrp {
                state.xrp_domain_books.insert((book.r#in, domain));
            }
        }
        None => {
            state
                .all_books
                .entry(book.r#in)
                .or_default()
                .insert(book.out);
            if to_xrp {
                state.xrp_books.insert(book.r#in);
            }
        }
    }
}
