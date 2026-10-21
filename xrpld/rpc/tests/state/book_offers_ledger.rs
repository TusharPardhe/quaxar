//! `book_offers` must walk the ledger selected by `ledger_index`/`ledger_hash`
//! (rippled `RPC::getBookPage` on the looked-up ledger), not the validated
//! ledger. A historical request answered from the validated ledger produced
//! wrong sparse replay captures for mainnet ledger 107805952 (2026-10-21).

use std::sync::Arc;
use std::time::Duration;

use app::{
    AmendmentStatus, ApplicationRoot, ApplicationRootOptions, JobQueue, OverlayStatusSource,
    PublishedServerPortsSource, SharedLoadFeeTrack, SharedTransaction, StatusMetricsSource,
    StatusRpcSnapshot, UnsupportedMajorityWarningDetails, ValidatorList,
    ValidatorListStatusSnapshot,
};
use basics::base_uint::{Uint160, Uint256};
use ledger::{Ledger, RawView};
use protocol::{
    AccountID, Asset, Book, Issue, JsonValue, LedgerEntryType, PublicKey, STAmount, STLedgerEntry,
    STVector256, SecretKey, currency_from_string, get_book_base, get_field_by_symbol, offer_keylet,
    sf_generic, xrp_issue,
};
use rpc::handlers::book_offers::BookOffersRuntime;
use rpc::{AppServerInfoView, ApplicationServerInfo, LedgerLookupLedger, OwnedApplicationServerInfo};
use tx::QueueTxQRpcReport;

struct TwoLedgerView {
    inner: OwnedApplicationServerInfo,
    validated: Arc<Ledger>,
    closed: Arc<Ledger>,
}

impl AppServerInfoView for TwoLedgerView {
    fn app(&self) -> Option<&ApplicationRoot> {
        // No LedgerMaster: only the two ledgers below are resolvable.
        None
    }

    fn status_snapshot(&self) -> StatusRpcSnapshot {
        self.inner.status_snapshot()
    }

    fn status_metrics(&self) -> Option<Arc<dyn StatusMetricsSource>> {
        self.inner.status_metrics()
    }

    fn overlay_status(&self) -> Option<Arc<dyn OverlayStatusSource>> {
        self.inner.overlay_status()
    }

    fn published_server_ports(&self) -> Option<Arc<dyn PublishedServerPortsSource>> {
        self.inner.published_server_ports()
    }

    fn validators(&self) -> Arc<ValidatorList> {
        self.inner.validators()
    }

    fn validator_list_status_snapshot(&self) -> ValidatorListStatusSnapshot {
        self.inner.validator_list_status_snapshot()
    }

    fn admin_pubkey_validator(&self) -> String {
        self.inner.admin_pubkey_validator()
    }

    fn current_server_time_string(&self) -> String {
        self.inner.current_server_time_string()
    }

    fn current_close_time_seconds(&self) -> u32 {
        self.inner.current_close_time_seconds()
    }

    fn close_time_offset_seconds(&self) -> i64 {
        self.inner.close_time_offset_seconds()
    }

    fn validated_ledger_age(&self) -> Duration {
        self.inner.validated_ledger_age()
    }

    fn validated_ledger(&self) -> Option<Arc<Ledger>> {
        Some(Arc::clone(&self.validated))
    }

    fn closed_ledger(&self) -> Option<Arc<Ledger>> {
        Some(Arc::clone(&self.closed))
    }

    fn published_ledger(&self) -> Option<Arc<Ledger>> {
        None
    }

    fn load_fee_track(&self) -> Arc<SharedLoadFeeTrack> {
        self.inner.load_fee_track()
    }

    fn job_queue(&self) -> JobQueue {
        self.inner.job_queue()
    }

    fn network_ops_runtime(&self) -> Option<Arc<app::AppNetworkOpsRuntime>> {
        self.inner.network_ops_runtime()
    }

    fn network_ops_operating_mode(&self) -> app::NetworkOpsOperatingMode {
        self.inner.network_ops_operating_mode()
    }

    fn network_ops_operating_mode_string(&self, admin: bool) -> &'static str {
        self.inner.network_ops_operating_mode_string(admin)
    }

    fn need_network_ledger(&self) -> bool {
        self.inner.need_network_ledger()
    }

    fn amendment_blocked(&self) -> bool {
        self.inner.amendment_blocked()
    }

    fn unl_blocked(&self) -> bool {
        self.inner.unl_blocked()
    }

    fn unsupported_majority_warned(&self) -> bool {
        self.inner.unsupported_majority_warned()
    }

    fn unsupported_majority_warning_details(&self) -> Option<UnsupportedMajorityWarningDetails> {
        self.inner.unsupported_majority_warning_details()
    }

    fn amendment_status(&self) -> Arc<AmendmentStatus> {
        self.inner.amendment_status()
    }

    fn node_identity(&self) -> Option<(PublicKey, SecretKey)> {
        self.inner.node_identity()
    }

    fn validation_public_key(&self) -> Option<PublicKey> {
        self.inner.validation_public_key()
    }

    fn status_rpc_current_ledger_index(&self) -> Option<u32> {
        self.inner.status_rpc_current_ledger_index()
    }

    fn status_rpc_queue_report(&self) -> Option<QueueTxQRpcReport> {
        self.inner.status_rpc_queue_report()
    }

    fn path_search_old(&self) -> u32 {
        self.inner.path_search_old()
    }

    fn path_search(&self) -> u32 {
        self.inner.path_search()
    }

    fn path_search_fast(&self) -> u32 {
        self.inner.path_search_fast()
    }

    fn path_search_max(&self) -> u32 {
        self.inner.path_search_max()
    }

    fn standalone(&self) -> bool {
        self.inner.standalone()
    }

    fn network_id(&self) -> u32 {
        self.inner.network_id()
    }

    fn fetch_cached_transaction(&self, txn_id: &Uint256) -> Option<SharedTransaction> {
        self.inner.fetch_cached_transaction(txn_id)
    }

    fn accept_standalone_ledger(&self) -> Result<u32, String> {
        self.inner.accept_standalone_ledger()
    }
}

#[test]
fn book_offers_reads_the_requested_ledger_not_the_validated_ledger() {
    let app = ApplicationRoot::with_options(ApplicationRootOptions::default()).expect("app");
    let issuer = AccountID::from_array([0x42; 20]);
    let usd = Issue::new(currency_from_string("USD"), issuer);
    let book = Book {
        r#in: Asset::Issue(xrp_issue()),
        out: Asset::Issue(usd),
        domain: None,
    };
    // rippled getQualityIndex: the low 64 bits of the book base hold the rate.
    let mut quality_bytes = *get_book_base(book).data();
    quality_bytes[24..].copy_from_slice(&0x5A1A_3809_F838_A480_u64.to_be_bytes());
    let quality_dir = Uint256::from_slice(&quality_bytes).expect("quality key");
    let offer = offer_keylet(Uint160::from_slice(issuer.data()).expect("account"), 9);

    let validated = Ledger::from_ledger_seq_and_close_time(10, 100, false);
    let mut closed = Ledger::from_ledger_seq_and_close_time(11, 110, false);
    let mut dir = STLedgerEntry::from_type_and_key(LedgerEntryType::DirectoryNode, quality_dir);
    dir.set_field_v256(
        get_field_by_symbol("sfIndexes"),
        STVector256::from_values(get_field_by_symbol("sfIndexes"), vec![offer.key]),
    );
    dir.set_field_h256(get_field_by_symbol("sfRootIndex"), quality_dir);
    closed.raw_insert(Arc::new(dir)).expect("book directory");
    let mut sle = STLedgerEntry::new(offer);
    sle.set_account_id(get_field_by_symbol("sfAccount"), issuer);
    sle.set_field_u32(get_field_by_symbol("sfSequence"), 9);
    sle.set_field_h256(get_field_by_symbol("sfBookDirectory"), quality_dir);
    sle.set_field_amount(get_field_by_symbol("sfTakerPays"), STAmount::new_native(656_501_727, false));
    sle.set_field_amount(
        get_field_by_symbol("sfTakerGets"),
        STAmount::new_with_asset(sf_generic(), usd, 8_895_729_709_111_601, -13, false),
    );
    sle.set_field_u64(get_field_by_symbol("sfOwnerNode"), 0);
    sle.set_field_u64(get_field_by_symbol("sfBookNode"), 0);
    closed.raw_insert(Arc::new(sle)).expect("offer");

    let requested = LedgerLookupLedger {
        hash: *closed.header().hash.as_uint256(),
        seq: closed.header().seq,
        open: false,
    };
    let source = ApplicationServerInfo::new(TwoLedgerView {
        inner: OwnedApplicationServerInfo::from_application_root(&app),
        validated: Arc::new(validated),
        closed: Arc::new(closed),
    });
    let mut result = JsonValue::Object(Default::default());
    source.get_book_page(&requested, book, issuer, false, 10, JsonValue::Null, &mut result);
    let offers = match &result {
        JsonValue::Object(map) => match map.get("offers") {
            Some(JsonValue::Array(offers)) => offers.len(),
            other => panic!("offers must be an array: {other:?}"),
        },
        other => panic!("result must be an object: {other:?}"),
    };
    assert_eq!(offers, 1, "the requested ledger's book tip must be returned");
}
