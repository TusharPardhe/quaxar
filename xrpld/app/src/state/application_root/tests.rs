use super::{
    AppOpenLedgerTxQApplyRuntime, ApplicationRoot, NodeFamilyRuntime, queue_apply_preclaim_ter,
};
use crate::ledger::ledger_master_runtime::AppLedgerMasterRuntime;
use crate::network::network_ops_runtime::AppNetworkOpsApplyHeldOutcome;
use crate::runtime::main_runtime::{GrpcRuntime, ManagedComponent};
use crate::shamap::shamap_store_service::SHAMapStoreService;
use crate::tx_queue::transaction::Transaction;
use crate::{
    AppOpenLedgerView, AppQueueApplyTxSource, AppTxQ, NetworkOpsOperatingMode,
    NetworkOpsProcessSetOwnerSync, NetworkOpsTransactionSetOutcome, SHAMapStore,
    SHAMapStoreCloseTimeProvider, SHAMapStoreComponent, SHAMapStoreComponentRuntime,
    SHAMapStoreHealthRuntime, SHAMapStoreOperatingMode, SHAMapStoreRuntime, SharedAppTxQ,
    SharedSHAMapStoreHealthState,
};
use basics::base_uint::{Uint160, Uint256};
use basics::sha_map_hash::SHAMapHash;
use ledger::{LEDGER_DEFAULT_TIME_RESOLUTION, Ledger, LedgerHeader, ReadView, Sandbox};
use protocol::{
    AccountID, KeyType, LedgerEntryType, Rules, STAmount, STLedgerEntry, STTx, SecretKey, SeqProxy,
    Ter, TxType, account_keylet, calc_account_id, derive_public_key, get_field_by_symbol,
    ticket_keylet_from_seq_proxy,
};
use shamap::item::SHAMapItem;
use shamap::mutation::MutableTree;
use shamap::sync::{SHAMapType, SyncState, SyncTree};
use shamap::traversal::TraversalError;
use shamap::tree_node::SHAMapNodeType;
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use tx::ApplyResult;
use tx::{
    ApplyFlags, FeeQueueEntry, FeeQueueKey, MaybeTx, MaybeTxCore, PreflightResult,
    QueueAcceptOwnerState, QueueAdvanceCandidate, QueueApplyExecutionRuntime,
    QueueApplyHoldPreflightTxSource, QueueApplyObservedAccountLookup,
    QueueApplyObservedTicketLookup, QueueApplyObservedTxSource, QueueApplyObservedViewSource,
    QueueFeeMetricsSnapshot, QueueViews, TxConsequences, TxQAccount, TxQSetup,
};

#[derive(Default)]
struct RecordingNodeFamily {
    resets: AtomicUsize,
    sweeps: AtomicUsize,
    seq_calls: Mutex<Vec<(u32, [u8; 32])>>,
    hash_calls: Mutex<Vec<([u8; 32], u32)>>,
}

impl NodeFamilyRuntime for RecordingNodeFamily {
    fn sweep(&self) {
        self.sweeps.fetch_add(1, Ordering::Relaxed);
    }

    fn reset(&self) {
        self.resets.fetch_add(1, Ordering::Relaxed);
    }

    fn fetch_cached_node(
        &self,
        _hash: basics::sha_map_hash::SHAMapHash,
        _ledger_seq: u32,
    ) -> Option<basics::intrusive_pointer::SharedIntrusive<shamap::tree_node::SHAMapTreeNode>> {
        None
    }

    fn missing_node_acquire_by_seq(&self, seq: u32, hash: basics::base_uint::Uint256) {
        self.seq_calls
            .lock()
            .expect("seq calls mutex must not be poisoned")
            .push((seq, *hash.data()));
    }

    fn missing_node_acquire_by_hash(&self, hash: basics::base_uint::Uint256, seq: u32) {
        self.hash_calls
            .lock()
            .expect("hash calls mutex must not be poisoned")
            .push((*hash.data(), seq));
    }

    fn visit_state_map_hashes(
        &self,
        _ledger: &Ledger,
        _visit: &mut dyn FnMut(Uint256) -> bool,
    ) -> Result<(), TraversalError> {
        Ok(())
    }
}

#[derive(Default)]
struct RecordingComponent {
    stops: AtomicUsize,
}

impl ManagedComponent for RecordingComponent {
    fn start(&self) -> Result<(), String> {
        Ok(())
    }

    fn stop(&self) {
        self.stops.fetch_add(1, Ordering::Relaxed);
    }
}

#[derive(Default)]
struct FixedCloseTimeProvider;

impl SHAMapStoreCloseTimeProvider for FixedCloseTimeProvider {
    fn current_close_time(&self) -> u32 {
        120
    }
}

#[derive(Default)]
struct ServiceRuntime;

impl SHAMapStoreRuntime for ServiceRuntime {
    fn start_background_work(&mut self) {}

    fn stop_background_work(&mut self) {}

    fn minimum_sql_seq(&self) -> Option<u32> {
        None
    }
}

impl SHAMapStoreHealthRuntime for ServiceRuntime {
    fn is_stopping(&self) -> bool {
        false
    }

    fn operating_mode(&self) -> SHAMapStoreOperatingMode {
        SHAMapStoreOperatingMode::Full
    }

    fn validated_ledger_age(&self) -> std::time::Duration {
        std::time::Duration::from_secs(1)
    }
}

impl SHAMapStoreComponentRuntime for ServiceRuntime {}

fn account(hex: &str) -> AccountID {
    AccountID::from_hex(hex).expect("account hex should parse")
}

fn raw_account_id(account: AccountID) -> Uint160 {
    Uint160::from_slice(account.data()).expect("account width should match Uint160")
}

fn payment_tx(
    source: AccountID,
    destination: AccountID,
    sequence: u32,
    ticket_sequence: Option<u32>,
    fee_drops: u64,
) -> Arc<STTx> {
    Arc::new(STTx::new(TxType::PAYMENT, |tx| {
        tx.set_account_id(get_field_by_symbol("sfAccount"), source);
        tx.set_account_id(get_field_by_symbol("sfDestination"), destination);
        tx.set_field_amount(
            get_field_by_symbol("sfAmount"),
            STAmount::new_native(1_000_000, false),
        );
        tx.set_field_amount(
            get_field_by_symbol("sfFee"),
            STAmount::new_native(fee_drops, false),
        );
        tx.set_field_u32(get_field_by_symbol("sfSequence"), sequence);
        if let Some(ticket_sequence) = ticket_sequence {
            tx.set_field_u32(get_field_by_symbol("sfTicketSequence"), ticket_sequence);
        }
    }))
}

fn signed_payment_tx(
    seed: u8,
    destination: AccountID,
    sequence: u32,
    fee_drops: u64,
) -> (AccountID, Arc<STTx>) {
    let secret = SecretKey::from_bytes([seed; 32]);
    let public = derive_public_key(KeyType::Secp256k1, &secret).expect("public key");
    let source = calc_account_id(public.as_bytes());
    let mut tx = STTx::new(TxType::PAYMENT, |tx| {
        tx.set_account_id(get_field_by_symbol("sfAccount"), source);
        tx.set_account_id(get_field_by_symbol("sfDestination"), destination);
        tx.set_field_amount(
            get_field_by_symbol("sfAmount"),
            STAmount::new_native(1_000_000, false),
        );
        tx.set_field_amount(
            get_field_by_symbol("sfFee"),
            STAmount::new_native(fee_drops, false),
        );
        tx.set_field_u32(get_field_by_symbol("sfSequence"), sequence);
        tx.set_field_vl(get_field_by_symbol("sfSigningPubKey"), public.as_bytes());
    });
    tx.sign(&public, &secret, None)
        .expect("signature should succeed");
    (source, Arc::new(tx))
}

fn signed_payment_tx_with_account_txn_id(
    seed: u8,
    destination: AccountID,
    sequence: u32,
    account_txn_id: Uint256,
    fee_drops: u64,
) -> (AccountID, Arc<STTx>) {
    let secret = SecretKey::from_bytes([seed; 32]);
    let public = derive_public_key(KeyType::Secp256k1, &secret).expect("public key");
    let source = calc_account_id(public.as_bytes());
    let mut tx = STTx::new(TxType::PAYMENT, |tx| {
        tx.set_account_id(get_field_by_symbol("sfAccount"), source);
        tx.set_account_id(get_field_by_symbol("sfDestination"), destination);
        tx.set_field_amount(
            get_field_by_symbol("sfAmount"),
            STAmount::new_native(1_000_000, false),
        );
        tx.set_field_amount(
            get_field_by_symbol("sfFee"),
            STAmount::new_native(fee_drops, false),
        );
        tx.set_field_u32(get_field_by_symbol("sfSequence"), sequence);
        tx.set_field_h256(get_field_by_symbol("sfAccountTxnID"), account_txn_id.into());
        tx.set_field_vl(get_field_by_symbol("sfSigningPubKey"), public.as_bytes());
    });
    tx.sign(&public, &secret, None)
        .expect("signature should succeed");
    (source, Arc::new(tx))
}

fn signed_payment_tx_with_ticket(
    seed: u8,
    destination: AccountID,
    ticket_sequence: u32,
    fee_drops: u64,
) -> (AccountID, Arc<STTx>) {
    let secret = SecretKey::from_bytes([seed; 32]);
    let public = derive_public_key(KeyType::Secp256k1, &secret).expect("public key");
    let source = calc_account_id(public.as_bytes());
    let mut tx = STTx::new(TxType::PAYMENT, |tx| {
        tx.set_account_id(get_field_by_symbol("sfAccount"), source);
        tx.set_account_id(get_field_by_symbol("sfDestination"), destination);
        tx.set_field_amount(
            get_field_by_symbol("sfAmount"),
            STAmount::new_native(1_000_000, false),
        );
        tx.set_field_amount(
            get_field_by_symbol("sfFee"),
            STAmount::new_native(fee_drops, false),
        );
        tx.set_field_u32(get_field_by_symbol("sfSequence"), 0);
        tx.set_field_u32(get_field_by_symbol("sfTicketSequence"), ticket_sequence);
        tx.set_field_vl(get_field_by_symbol("sfSigningPubKey"), public.as_bytes());
    });
    tx.sign(&public, &secret, None)
        .expect("signature should succeed");
    (source, Arc::new(tx))
}

fn signed_ticket_create_tx(
    seed: u8,
    sequence: u32,
    ticket_count: u32,
    fee_drops: u64,
) -> (AccountID, Arc<STTx>) {
    let secret = SecretKey::from_bytes([seed; 32]);
    let public = derive_public_key(KeyType::Secp256k1, &secret).expect("public key");
    let source = calc_account_id(public.as_bytes());
    let mut tx = STTx::new(TxType::TICKET_CREATE, |tx| {
        tx.set_account_id(get_field_by_symbol("sfAccount"), source);
        tx.set_field_u32(get_field_by_symbol("sfTicketCount"), ticket_count);
        tx.set_field_amount(
            get_field_by_symbol("sfFee"),
            STAmount::new_native(fee_drops, false),
        );
        tx.set_field_u32(get_field_by_symbol("sfSequence"), sequence);
        tx.set_field_vl(get_field_by_symbol("sfSigningPubKey"), public.as_bytes());
    });
    tx.sign(&public, &secret, None)
        .expect("signature should succeed");
    (source, Arc::new(tx))
}

fn ledger_view(seq: u32, account: AccountID, account_sequence: u32, tx_ids: &[Uint256]) -> Ledger {
    let mut state_tree = MutableTree::new(1);
    let mut account_root = STLedgerEntry::from_type_and_key(
        LedgerEntryType::AccountRoot,
        account_keylet(raw_account_id(account)).key,
    );
    account_root.set_account_id(get_field_by_symbol("sfAccount"), account);
    account_root.set_field_u32(get_field_by_symbol("sfSequence"), account_sequence);
    account_root.set_field_amount(
        get_field_by_symbol("sfBalance"),
        STAmount::new_native(1_000_000_000, false),
    );
    state_tree
        .add_item(
            SHAMapNodeType::AccountState,
            SHAMapItem::new(
                account_keylet(raw_account_id(account)).key,
                account_root.get_serializer().data().to_vec(),
            ),
        )
        .expect("account root should insert");

    let mut tx_tree = MutableTree::new(1);
    for (index, tx_id) in tx_ids.iter().enumerate() {
        tx_tree
            .add_item(
                SHAMapNodeType::TransactionNm,
                SHAMapItem::new(*tx_id, vec![index as u8 + 1; 12]),
            )
            .expect("tx should insert");
    }

    Ledger::from_maps(
        LedgerHeader {
            seq,
            close_time: 800 + seq,
            close_time_resolution: LEDGER_DEFAULT_TIME_RESOLUTION,
            ..LedgerHeader::default()
        },
        SyncTree::from_root_with_type(
            state_tree.root(),
            SHAMapType::State,
            false,
            seq,
            SyncState::Modifying,
        ),
        SyncTree::from_root_with_type(
            tx_tree.root(),
            SHAMapType::Transaction,
            false,
            seq,
            SyncState::Modifying,
        ),
    )
}

fn ledger_view_with_balance_and_owner_count(
    seq: u32,
    account: AccountID,
    account_sequence: u32,
    balance_drops: u64,
    owner_count: u32,
    tx_ids: &[Uint256],
) -> Ledger {
    let mut state_tree = MutableTree::new(1);
    let mut account_root = STLedgerEntry::from_type_and_key(
        LedgerEntryType::AccountRoot,
        account_keylet(raw_account_id(account)).key,
    );
    account_root.set_account_id(get_field_by_symbol("sfAccount"), account);
    account_root.set_field_u32(get_field_by_symbol("sfSequence"), account_sequence);
    account_root.set_field_amount(
        get_field_by_symbol("sfBalance"),
        STAmount::new_native(balance_drops, false),
    );
    account_root.set_field_u32(get_field_by_symbol("sfOwnerCount"), owner_count);
    state_tree
        .add_item(
            SHAMapNodeType::AccountState,
            SHAMapItem::new(
                account_keylet(raw_account_id(account)).key,
                account_root.get_serializer().data().to_vec(),
            ),
        )
        .expect("account root should insert");

    let mut tx_tree = MutableTree::new(1);
    for (index, tx_id) in tx_ids.iter().enumerate() {
        tx_tree
            .add_item(
                SHAMapNodeType::TransactionNm,
                SHAMapItem::new(*tx_id, vec![index as u8 + 1; 12]),
            )
            .expect("tx should insert");
    }

    Ledger::from_maps(
        LedgerHeader {
            seq,
            close_time: 800 + seq,
            close_time_resolution: LEDGER_DEFAULT_TIME_RESOLUTION,
            ..LedgerHeader::default()
        },
        SyncTree::from_root_with_type(
            state_tree.root(),
            SHAMapType::State,
            false,
            seq,
            SyncState::Modifying,
        ),
        SyncTree::from_root_with_type(
            tx_tree.root(),
            SHAMapType::Transaction,
            false,
            seq,
            SyncState::Modifying,
        ),
    )
}

fn ledger_view_with_account_txn_id(
    seq: u32,
    account: AccountID,
    account_sequence: u32,
    account_txn_id: Uint256,
    tx_ids: &[Uint256],
) -> Ledger {
    let mut state_tree = MutableTree::new(1);
    let mut account_root = STLedgerEntry::from_type_and_key(
        LedgerEntryType::AccountRoot,
        account_keylet(raw_account_id(account)).key,
    );
    account_root.set_account_id(get_field_by_symbol("sfAccount"), account);
    account_root.set_field_u32(get_field_by_symbol("sfSequence"), account_sequence);
    account_root.set_field_h256(get_field_by_symbol("sfAccountTxnID"), account_txn_id.into());
    account_root.set_field_amount(
        get_field_by_symbol("sfBalance"),
        STAmount::new_native(1_000_000_000, false),
    );
    state_tree
        .add_item(
            SHAMapNodeType::AccountState,
            SHAMapItem::new(
                account_keylet(raw_account_id(account)).key,
                account_root.get_serializer().data().to_vec(),
            ),
        )
        .expect("account root should insert");

    let mut tx_tree = MutableTree::new(1);
    for (index, tx_id) in tx_ids.iter().enumerate() {
        tx_tree
            .add_item(
                SHAMapNodeType::TransactionNm,
                SHAMapItem::new(*tx_id, vec![index as u8 + 1; 12]),
            )
            .expect("tx should insert");
    }

    Ledger::from_maps(
        LedgerHeader {
            seq,
            close_time: 800 + seq,
            close_time_resolution: LEDGER_DEFAULT_TIME_RESOLUTION,
            ..LedgerHeader::default()
        },
        SyncTree::from_root_with_type(
            state_tree.root(),
            SHAMapType::State,
            false,
            seq,
            SyncState::Modifying,
        ),
        SyncTree::from_root_with_type(
            tx_tree.root(),
            SHAMapType::Transaction,
            false,
            seq,
            SyncState::Modifying,
        ),
    )
}

fn ledger_view_with_ticket(
    seq: u32,
    account: AccountID,
    account_sequence: u32,
    balance_drops: u64,
    ticket_seq: SeqProxy,
) -> Ledger {
    let mut state_tree = MutableTree::new(1);
    let mut account_root = STLedgerEntry::from_type_and_key(
        LedgerEntryType::AccountRoot,
        account_keylet(raw_account_id(account)).key,
    );
    account_root.set_account_id(get_field_by_symbol("sfAccount"), account);
    account_root.set_field_u32(get_field_by_symbol("sfSequence"), account_sequence);
    account_root.set_field_amount(
        get_field_by_symbol("sfBalance"),
        STAmount::new_native(balance_drops, false),
    );
    state_tree
        .add_item(
            SHAMapNodeType::AccountState,
            SHAMapItem::new(
                account_keylet(raw_account_id(account)).key,
                account_root.get_serializer().data().to_vec(),
            ),
        )
        .expect("account root should insert");

    let ticket = STLedgerEntry::from_type_and_key(
        LedgerEntryType::Ticket,
        ticket_keylet_from_seq_proxy(raw_account_id(account), ticket_seq).key,
    );
    state_tree
        .add_item(
            SHAMapNodeType::AccountState,
            SHAMapItem::new(
                ticket_keylet_from_seq_proxy(raw_account_id(account), ticket_seq).key,
                ticket.get_serializer().data().to_vec(),
            ),
        )
        .expect("ticket should insert");

    Ledger::from_maps(
        LedgerHeader {
            seq,
            close_time: 800 + seq,
            close_time_resolution: LEDGER_DEFAULT_TIME_RESOLUTION,
            ..LedgerHeader::default()
        },
        SyncTree::from_root_with_type(
            state_tree.root(),
            SHAMapType::State,
            false,
            seq,
            SyncState::Modifying,
        ),
        SyncTree::new_with_type(SHAMapType::Transaction, false, seq),
    )
}

fn apply_submit_tx_for_test(
    open_ledger: &mut AppOpenLedgerView,
    submit_view: &mut Sandbox<Ledger>,
    tx: Arc<STTx>,
    current_ledger_index: u32,
) -> ApplyResult {
    let rules = submit_view.rules().clone();
    let preclaim_ter = queue_apply_preclaim_ter(submit_view, tx.as_ref(), current_ledger_index);
    let mut runtime = AppOpenLedgerTxQApplyRuntime::new(
        open_ledger,
        submit_view,
        tx,
        rules,
        ApplyFlags::NONE,
        current_ledger_index,
        preclaim_ter,
    );
    runtime.direct_apply()
}

#[test]
fn app_queue_apply_tx_source_reports_sttx_facts_submit_path() {
    let account = AccountID::from_array([0x61; 20]);
    let destination = AccountID::from_array([0x62; 20]);
    let tx = Arc::new(STTx::new(TxType::PAYMENT, |tx| {
        tx.set_account_id(get_field_by_symbol("sfAccount"), account);
        tx.set_account_id(get_field_by_symbol("sfDestination"), destination);
        tx.set_field_amount(
            get_field_by_symbol("sfAmount"),
            STAmount::new_native(1_000_000, false),
        );
        tx.set_field_amount(
            get_field_by_symbol("sfFee"),
            STAmount::new_native(25, false),
        );
        tx.set_field_u32(get_field_by_symbol("sfSequence"), 0);
        tx.set_field_u32(get_field_by_symbol("sfTicketSequence"), 8);
        tx.set_field_h256(
            get_field_by_symbol("sfPreviousTxnID"),
            Uint256::from_u64(9).into(),
        );
        tx.set_field_h256(
            get_field_by_symbol("sfAccountTxnID"),
            Uint256::from_u64(10).into(),
        );
        tx.set_field_u32(get_field_by_symbol("sfLastLedgerSequence"), 123);
    }));

    let source = AppQueueApplyTxSource::new(tx.as_ref());
    assert_eq!(*source.account(), account);
    assert_eq!(source.transaction_id(), tx.get_transaction_id());
    assert_eq!(source.tx_id(), tx.get_transaction_id());
    assert_eq!(source.tx_seq_proxy(), SeqProxy::ticket(8));
    assert!(source.has_previous_txn_id());
    assert!(source.has_account_txn_id());
    assert_eq!(source.last_valid_ledger(), Some(123));
}

#[test]
fn app_open_ledger_queue_apply_view_reads_live_account_and_ticket_facts() {
    let account = AccountID::from_array([0x41; 20]);
    let destination = AccountID::from_array([0x42; 20]);
    let ticket_seq = SeqProxy::ticket(8);
    let ledger = ledger_view_with_ticket(10, account, 7, 5_000, ticket_seq);
    let tx = Arc::new(STTx::new(TxType::PAYMENT, |tx| {
        tx.set_account_id(get_field_by_symbol("sfAccount"), account);
        tx.set_account_id(get_field_by_symbol("sfDestination"), destination);
        tx.set_field_amount(
            get_field_by_symbol("sfAmount"),
            STAmount::new_native(1_000_000, false),
        );
        tx.set_field_amount(
            get_field_by_symbol("sfFee"),
            STAmount::new_native(25, false),
        );
        tx.set_field_u32(get_field_by_symbol("sfSequence"), 0);
        tx.set_field_u32(get_field_by_symbol("sfTicketSequence"), 8);
    }));

    let mut open_ledger = AppOpenLedgerView::with_parent_hash(11, 10, Uint256::from_u64(99));
    open_ledger.push_transaction(Arc::clone(&tx));
    let metrics_snapshot = QueueFeeMetricsSnapshot {
        txns_expected: 32,
        escalation_multiplier: tx::TXQ_BASE_LEVEL * 500,
    };

    let view = open_ledger.queue_apply_view(&ledger, tx.as_ref(), metrics_snapshot);

    assert_eq!(
        view.account_lookup(&account),
        QueueApplyObservedAccountLookup::Present {
            sequence: 7,
            balance_drops: 5_000,
        }
    );
    assert_eq!(
        view.ticket_lookup(&account, ticket_seq),
        QueueApplyObservedTicketLookup::Present
    );
    assert_eq!(
        view.ticket_lookup(&account, SeqProxy::sequence(7)),
        QueueApplyObservedTicketLookup::NotRequired
    );
    assert_eq!(view.fee_paid_drops(), 25);
    assert_eq!(view.open_ledger_tx_count(), 1);
    assert_eq!(view.open_ledger_seq(), 11);
    assert_eq!(view.base_fee_drops(), ledger.fees().base);
    assert_eq!(view.reserve_drops(), ledger.fees().account_reserve(0));
    assert_eq!(view.metrics_snapshot(), metrics_snapshot);
    assert_eq!(view.rules(), &ledger.rules().clone());
}

#[test]
fn submit_direct_apply_ticket_create_updates_ticket_tracking() {
    let (source, ticket_create) = signed_ticket_create_tx(0x51, 1, 2, 10);
    let base = Arc::new(ledger_view_with_balance_and_owner_count(
        1,
        source,
        1,
        2_000_000,
        0,
        &[],
    ));
    let mut open_ledger =
        AppOpenLedgerView::with_parent_hash(2, 10, *base.header().hash.as_uint256());
    let mut submit_view = Sandbox::new(Arc::clone(&base), ApplyFlags::NONE);

    let result = apply_submit_tx_for_test(&mut open_ledger, &mut submit_view, ticket_create, 2);

    assert_eq!(result.ter, Ter::TES_SUCCESS);
    assert!(result.applied);

    let account_root = submit_view
        .read(account_keylet(raw_account_id(source)))
        .expect("account read should succeed")
        .expect("account should exist");
    assert_eq!(
        account_root.get_field_u32(get_field_by_symbol("sfSequence")),
        4
    );
    assert_eq!(
        account_root.get_field_u32(get_field_by_symbol("sfTicketCount")),
        2
    );
    assert_eq!(
        account_root.get_field_u32(get_field_by_symbol("sfOwnerCount")),
        2
    );
    assert_eq!(
        account_root
            .get_field_amount(get_field_by_symbol("sfBalance"))
            .xrp()
            .drops(),
        1_999_990
    );
    assert!(
        submit_view
            .exists(protocol::ticket_keylet(raw_account_id(source), 2))
            .expect("ticket 2 lookup should succeed")
    );
    assert!(
        submit_view
            .exists(protocol::ticket_keylet(raw_account_id(source), 3))
            .expect("ticket 3 lookup should succeed")
    );
}

#[test]
fn submit_direct_apply_ticket_use_clears_ticket_tracking() {
    let destination = account("F0F0F0F0F0F0F0F0F0F0F0F0F0F0F0F0F0F0F0F0");
    let (source, ticket_create) = signed_ticket_create_tx(0x61, 1, 1, 10);
    let (_, ticket_payment) = signed_payment_tx_with_ticket(0x61, destination, 2, 11);
    let base = Arc::new(ledger_view_with_balance_and_owner_count(
        1,
        source,
        1,
        2_000_000,
        0,
        &[],
    ));
    let mut open_ledger =
        AppOpenLedgerView::with_parent_hash(2, 10, *base.header().hash.as_uint256());
    let mut submit_view = Sandbox::new(Arc::clone(&base), ApplyFlags::NONE);

    let create_result =
        apply_submit_tx_for_test(&mut open_ledger, &mut submit_view, ticket_create, 2);
    assert_eq!(create_result.ter, Ter::TES_SUCCESS);
    assert!(create_result.applied);

    let payment_result =
        apply_submit_tx_for_test(&mut open_ledger, &mut submit_view, ticket_payment, 2);
    assert_eq!(payment_result.ter, Ter::TES_SUCCESS);
    assert!(payment_result.applied);

    let account_root = submit_view
        .read(account_keylet(raw_account_id(source)))
        .expect("account read should succeed")
        .expect("account should exist");
    assert_eq!(
        account_root.get_field_u32(get_field_by_symbol("sfSequence")),
        3
    );
    assert_eq!(
        account_root.get_field_u32(get_field_by_symbol("sfOwnerCount")),
        0
    );
    assert!(!account_root.is_field_present(get_field_by_symbol("sfTicketCount")));
    assert_eq!(
        account_root
            .get_field_amount(get_field_by_symbol("sfBalance"))
            .xrp()
            .drops(),
        999_979
    );
    assert!(
        !submit_view
            .exists(protocol::ticket_keylet(raw_account_id(source), 2))
            .expect("ticket lookup should succeed")
    );
}

#[test]
fn submit_direct_apply_ticket_create_uses_pre_fee_balance_for_reserve() {
    let (source, ticket_create) = signed_ticket_create_tx(0x62, 1, 1, 10);
    let base = Arc::new(ledger_view_with_balance_and_owner_count(
        1,
        source,
        1,
        259,
        0,
        &[],
    ));
    let mut open_ledger =
        AppOpenLedgerView::with_parent_hash(2, 10, *base.header().hash.as_uint256());
    let mut submit_view = Sandbox::new(Arc::clone(&base), ApplyFlags::NONE);

    let result = apply_submit_tx_for_test(&mut open_ledger, &mut submit_view, ticket_create, 2);

    assert_eq!(result.ter, Ter::TES_SUCCESS);
    assert!(result.applied);

    let account_root = submit_view
        .read(account_keylet(raw_account_id(source)))
        .expect("account read should succeed")
        .expect("account should exist");
    assert_eq!(
        account_root
            .get_field_amount(get_field_by_symbol("sfBalance"))
            .xrp()
            .drops(),
        249
    );
    assert_eq!(
        account_root.get_field_u32(get_field_by_symbol("sfTicketCount")),
        1
    );
    assert!(
        submit_view
            .exists(protocol::ticket_keylet(raw_account_id(source), 2))
            .expect("ticket lookup should succeed")
    );
}

#[test]
fn application_root_reads_live_account_queue_txs_from_app_owned_txq() {
    let mut app = ApplicationRoot::new(0).expect("root shell should build");
    let account = AccountID::from_array([0x55; 20]);
    let destination = AccountID::from_array([0x77; 20]);
    let seq_proxy = SeqProxy::sequence(7);
    let consequences = TxConsequences::with_potential_spend(12, seq_proxy, 100);
    let tx = payment_tx(account, destination, 7, None, 10);

    let mut queued_account = TxQAccount::new(account);
    queued_account.add(
        seq_proxy,
        MaybeTxCore::new(
            MaybeTx::new(
                Uint256::from_u64(77),
                256,
                account,
                Some(120),
                seq_proxy,
                ApplyFlags::NONE,
                PreflightResult::new(
                    Arc::clone(&tx),
                    None::<String>,
                    Rules::new(std::iter::empty()),
                    consequences,
                    ApplyFlags::NONE,
                    "journal".to_owned(),
                    Ter::TES_SUCCESS,
                ),
            ),
            consequences,
        ),
    );

    app.registry.tx_q = SharedAppTxQ::new(AppTxQ::new_from_setup(
        TxQSetup::default(),
        None,
        QueueAcceptOwnerState::new(Uint256::from_u64(0)),
        QueueViews::new(BTreeMap::from([(account, queued_account)]), Vec::new()),
    ));

    let queue_txs = app.tx_q_account_txs(account);
    assert_eq!(queue_txs.len(), 1);
    assert_eq!(queue_txs[0].seq_proxy, seq_proxy);
    assert_eq!(queue_txs[0].fee_level, 256);
    assert_eq!(queue_txs[0].last_valid, Some(120));
    assert_eq!(queue_txs[0].account, account);
    assert_eq!(queue_txs[0].consequences.fee(), 12);
    assert_eq!(queue_txs[0].consequences.potential_spend(), 100);
    assert_eq!(
        queue_txs[0].tx.get_transaction_id(),
        tx.get_transaction_id()
    );
}

#[test]
fn application_root_accept_ledger_runs_closed_ledger_txq_maintenance() {
    let mut app = ApplicationRoot::new(0).expect("root shell should build");
    let account = AccountID::from_array([0x33; 20]);
    let destination = AccountID::from_array([0x44; 20]);
    let seq_proxy = SeqProxy::sequence(7);
    let tx_id = Uint256::from_u64(77);
    let consequences = TxConsequences::with_potential_spend(12, seq_proxy, 100);
    let tx = payment_tx(account, destination, 7, None, 10);

    let mut queued_account = TxQAccount::new(account);
    queued_account.add(
        seq_proxy,
        MaybeTxCore::new(
            MaybeTx::new(
                tx_id,
                256,
                account,
                Some(1),
                seq_proxy,
                ApplyFlags::NONE,
                PreflightResult::new(
                    Arc::clone(&tx),
                    None::<String>,
                    Rules::new(std::iter::empty()),
                    consequences,
                    ApplyFlags::NONE,
                    "journal".to_owned(),
                    Ter::TES_SUCCESS,
                ),
            ),
            consequences,
        ),
    );

    app.registry.tx_q = SharedAppTxQ::new(AppTxQ::new_from_setup(
        TxQSetup::default(),
        None,
        QueueAcceptOwnerState::new(Uint256::from_u64(0)),
        QueueViews::new(
            BTreeMap::from([(account, queued_account)]),
            vec![FeeQueueEntry::new(
                FeeQueueKey::new(account, seq_proxy),
                QueueAdvanceCandidate {
                    fee_level: 256,
                    tx_id,
                    seq_proxy,
                },
            )],
        ),
    ));

    let next_open = app
        .accept_ledger(1, 1_234, 10)
        .expect("ledger accept should complete");

    assert_eq!(next_open, 2);
    assert!(app.tx_q_account_txs(account).is_empty());
    assert_eq!(app.tx_q_rpc_report().current_queue_size, "0");
}

#[test]
fn application_root_accept_ledger_rebuilds_next_open_with_current_and_queued_txs() {
    let mut app = ApplicationRoot::new(0).expect("root shell should build");
    let current_account = AccountID::from_array([0x11; 20]);
    let queued_account_id = AccountID::from_array([0x22; 20]);
    let destination = AccountID::from_array([0x99; 20]);
    let current_tx = payment_tx(current_account, destination, 1, None, 10);
    let queued_tx = payment_tx(queued_account_id, destination, 1, None, 12);
    let queued_seq = SeqProxy::sequence(1);
    let queued_id = queued_tx.get_transaction_id();
    let consequences = TxConsequences::with_potential_spend(12, queued_seq, 100);

    let mut parent = ledger_view(1, current_account, 1, &[]);
    parent.set_accepted(1_111, ledger::LEDGER_DEFAULT_TIME_RESOLUTION, true);
    let parent = Arc::new(parent);
    app.on_closed_ledger(Arc::clone(&parent));
    let _ = app.open_ledger().modify(|view| {
        *view = AppOpenLedgerView::with_parent_hash(2, 10, *parent.header().hash.as_uint256());
        view.push_transaction(Arc::clone(&current_tx));
        true
    });

    let mut queued_account = TxQAccount::new(queued_account_id);
    queued_account.add(
        queued_seq,
        MaybeTxCore::new(
            MaybeTx::new(
                queued_id,
                512,
                queued_account_id,
                Some(10),
                queued_seq,
                ApplyFlags::NONE,
                PreflightResult::new(
                    Arc::clone(&queued_tx),
                    None::<String>,
                    Rules::new(std::iter::empty()),
                    consequences,
                    ApplyFlags::NONE,
                    "journal".to_owned(),
                    Ter::TES_SUCCESS,
                ),
            ),
            consequences,
        ),
    );

    app.registry.tx_q = SharedAppTxQ::new(AppTxQ::new_from_setup(
        TxQSetup::default(),
        None,
        QueueAcceptOwnerState::new(Uint256::from_u64(0)),
        QueueViews::new(
            BTreeMap::from([(queued_account_id, queued_account)]),
            vec![FeeQueueEntry::new(
                FeeQueueKey::new(queued_account_id, queued_seq),
                QueueAdvanceCandidate {
                    fee_level: 512,
                    tx_id: queued_id,
                    seq_proxy: queued_seq,
                },
            )],
        ),
    ));

    let next_open = app
        .accept_ledger(2, 1_234, 10)
        .expect("ledger accept should complete");

    let rebuilt = app.open_ledger().current();
    assert_eq!(next_open, 3);
    assert_eq!(rebuilt.ledger_current_index, 3);
    assert_eq!(rebuilt.base_fee_drops, 10);
    assert_eq!(
        rebuilt.parent_hash,
        *app.closed_ledger()
            .expect("closed")
            .header()
            .hash
            .as_uint256()
    );
    assert_eq!(
        rebuilt.tx_ids(),
        vec![
            current_tx.get_transaction_id(),
            queued_tx.get_transaction_id()
        ]
    );
    assert!(app.tx_q_account_txs(queued_account_id).is_empty());
}

#[test]
fn application_root_applies_network_ops_pending_to_open_ledger_through_app_txq_runtime() {
    let mut app = ApplicationRoot::new(0).expect("root shell should build");
    let runtime = app.attach_default_network_ops_runtime();
    let destination = account("CCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCC");
    let (source, direct) = signed_payment_tx(0x13, destination, 1, 10);
    let (_, queued) = signed_payment_tx(0x13, destination, 2, 11);

    let mut parent = ledger_view_with_balance_and_owner_count(1, source, 1, 1_000_000_000, 0, &[]);
    parent.set_accepted(1_111, ledger::LEDGER_DEFAULT_TIME_RESOLUTION, true);
    let parent = Arc::new(parent);
    app.on_closed_ledger(Arc::clone(&parent));
    let _ = app.open_ledger().modify(|view| {
        *view = AppOpenLedgerView::with_parent_hash(2, 10, *parent.header().hash.as_uint256());
        true
    });
    let mut direct_shared = Arc::new(Mutex::new(Transaction::new(Arc::clone(&direct))));
    let mut queued_shared = Arc::new(Mutex::new(Transaction::new(Arc::clone(&queued))));
    app.canonicalize_transaction(&mut direct_shared);
    app.canonicalize_transaction(&mut queued_shared);

    assert!(runtime.stage_transaction(Arc::clone(&direct_shared), false, false, false));
    assert!(runtime.stage_transaction(Arc::clone(&queued_shared), false, false, false));

    let report = app
        .apply_network_ops_pending_to_open_ledger()
        .expect("pending batch should apply to open ledger");

    assert_eq!(report.start.taken_transactions, 2);
    assert_eq!(report.entries.len(), 2);
    assert!(report.entries[0].applied, "{report:?}");
    assert_eq!(report.entries[0].result, Ter::TES_SUCCESS);
    assert!(report.entries[1].applied, "{report:?}");
    assert_eq!(report.entries[1].result, Ter::TES_SUCCESS);
    assert_eq!(
        app.open_ledger().current().tx_ids(),
        vec![direct.get_transaction_id(), queued.get_transaction_id()]
    );

    let queued_txs = app.tx_q_account_txs(source);
    assert!(queued_txs.is_empty());
    assert_eq!(app.network_ops_pending_transaction_count(), Some(0));
}

#[test]
fn application_root_submit_batch_reuses_live_ticket_and_sequence_state() {
    let mut app = ApplicationRoot::new(0).expect("root shell should build");
    let runtime = app.attach_default_network_ops_runtime();
    let destination = account("DDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDD");
    let (source, ticket_create) = signed_ticket_create_tx(0x23, 1, 1, 10);
    let (_, ticket_payment) = signed_payment_tx_with_ticket(0x23, destination, 2, 11);
    let (_, sequence_payment) = signed_payment_tx(0x23, destination, 3, 12);

    let mut parent = ledger_view_with_balance_and_owner_count(1, source, 1, 2_000_100, 0, &[]);
    parent.set_accepted(1_111, ledger::LEDGER_DEFAULT_TIME_RESOLUTION, true);
    let parent = Arc::new(parent);
    app.on_closed_ledger(Arc::clone(&parent));
    let _ = app.open_ledger().modify(|view| {
        *view = AppOpenLedgerView::with_parent_hash(2, 10, *parent.header().hash.as_uint256());
        true
    });

    let mut ticket_create_shared =
        Arc::new(Mutex::new(Transaction::new(Arc::clone(&ticket_create))));
    let mut ticket_payment_shared =
        Arc::new(Mutex::new(Transaction::new(Arc::clone(&ticket_payment))));
    let mut sequence_payment_shared =
        Arc::new(Mutex::new(Transaction::new(Arc::clone(&sequence_payment))));
    app.canonicalize_transaction(&mut ticket_create_shared);
    app.canonicalize_transaction(&mut ticket_payment_shared);
    app.canonicalize_transaction(&mut sequence_payment_shared);

    assert!(runtime.stage_transaction(Arc::clone(&ticket_create_shared), false, false, false));
    assert!(runtime.stage_transaction(Arc::clone(&ticket_payment_shared), false, false, false));
    assert!(runtime.stage_transaction(Arc::clone(&sequence_payment_shared), false, false, false));

    let report = app
        .apply_network_ops_pending_to_open_ledger()
        .expect("pending batch should apply to open ledger");

    assert_eq!(report.start.taken_transactions, 3);
    assert_eq!(report.entries.len(), 3);
    for entry in &report.entries {
        assert!(entry.applied, "{report:?}");
        assert_eq!(entry.result, Ter::TES_SUCCESS);
    }
    assert_eq!(
        app.open_ledger().current().tx_ids(),
        vec![
            ticket_create.get_transaction_id(),
            ticket_payment.get_transaction_id(),
            sequence_payment.get_transaction_id()
        ]
    );
    assert_eq!(app.network_ops_pending_transaction_count(), Some(0));
}

#[test]
fn application_root_submit_batch_reuses_live_account_txn_id_state() {
    let mut app = ApplicationRoot::new(0).expect("root shell should build");
    let runtime = app.attach_default_network_ops_runtime();
    let destination = account("EEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEE");
    let (source, first) = signed_payment_tx(0x31, destination, 1, 10);
    let (_, second) =
        signed_payment_tx_with_account_txn_id(0x31, destination, 2, first.get_transaction_id(), 11);

    let mut parent = ledger_view_with_account_txn_id(1, source, 1, Uint256::from_u64(777), &[]);
    parent.set_accepted(1_111, ledger::LEDGER_DEFAULT_TIME_RESOLUTION, true);
    let parent = Arc::new(parent);
    app.on_closed_ledger(Arc::clone(&parent));
    let _ = app.open_ledger().modify(|view| {
        *view = AppOpenLedgerView::with_parent_hash(2, 10, *parent.header().hash.as_uint256());
        true
    });

    let mut first_shared = Arc::new(Mutex::new(Transaction::new(Arc::clone(&first))));
    let mut second_shared = Arc::new(Mutex::new(Transaction::new(Arc::clone(&second))));
    app.canonicalize_transaction(&mut first_shared);
    app.canonicalize_transaction(&mut second_shared);

    assert!(runtime.stage_transaction(Arc::clone(&first_shared), false, false, false));
    assert!(runtime.stage_transaction(Arc::clone(&second_shared), false, false, false));

    let report = app
        .apply_network_ops_pending_to_open_ledger()
        .expect("pending batch should apply to open ledger");

    assert_eq!(report.start.taken_transactions, 2);
    assert_eq!(report.entries.len(), 2);
    assert!(report.entries[0].applied, "{report:?}");
    assert_eq!(report.entries[0].result, Ter::TES_SUCCESS);
    assert!(report.entries[1].applied, "{report:?}");
    assert_eq!(report.entries[1].result, Ter::TES_SUCCESS);
    assert_eq!(
        app.open_ledger().current().tx_ids(),
        vec![first.get_transaction_id(), second.get_transaction_id()]
    );
}

#[test]
fn application_root_tracks_stop_reason_family_cleanup_and_runtime_bindings() {
    let mut app = ApplicationRoot::new(0).expect("root shell should build");
    let family = Arc::new(RecordingNodeFamily::default());
    let family_runtime: Arc<dyn NodeFamilyRuntime> = family.clone();

    assert!(app.attach_node_family(family_runtime).is_none());
    let callback = app.wire_node_family_reset().expect("family reset callback");

    let server = Arc::new(RecordingComponent::default());
    assert!(app.bind_server(server.clone()).is_none());
    let shamap_store = Arc::new(RecordingComponent::default());
    assert!(app.bind_shamap_store(shamap_store.clone()).is_none());
    app.disable_grpc("disabled for parity");

    let events = Arc::new(Mutex::new(Vec::new()));
    let events_clone = Arc::clone(&events);
    app.register_stop_callback("marker", move || {
        events_clone
            .lock()
            .expect("events mutex must not be poisoned")
            .push("stopped");
    });

    assert!(app.signal_stop("testing"));
    assert!(!app.signal_stop("ignored"));
    assert!(app.is_stopping());
    assert_eq!(app.stop_reason(), Some("testing".to_owned()));
    assert_eq!(
        app.job_queue()
            .get_job_count(crate::job::job_types::JobType::Accept),
        0
    );
    assert_eq!(app.time_keeper().close_offset(), time::Duration::seconds(0));
    assert_eq!(family.resets.load(Ordering::Relaxed), 1);
    assert_eq!(
        events
            .lock()
            .expect("events mutex must not be poisoned")
            .as_slice(),
        &["stopped"]
    );
    assert_eq!(callback.name(), "node-family-reset");
    assert!(matches!(
        &app.runtime_bindings().grpc,
        GrpcRuntime::DisabledExplicit { .. }
    ));
    assert!(app.runtime_bindings().server.is_some());
    assert!(app.runtime_bindings().shamap_store.is_some());
}

#[test]
fn application_root_routes_validated_ledger_and_mode_into_attached_shamap_store_service() {
    let mut app = ApplicationRoot::new(0).expect("root shell should build");
    let health = Arc::new(SharedSHAMapStoreHealthState::new(Arc::new(
        FixedCloseTimeProvider,
    )));
    let component = Arc::new(SHAMapStoreComponent::new(
        SHAMapStore::new(256, false, 9),
        Box::new(ServiceRuntime),
        None,
    ));
    let service = Arc::new(SHAMapStoreService::new(component.clone(), health.clone()));

    assert!(app.attach_shamap_store_service(service).is_none());
    assert!(app.set_shamap_store_operating_mode(SHAMapStoreOperatingMode::Full));
    assert!(
        app.on_validated_ledger(Arc::new(Ledger::from_ledger_seq_and_close_time(
            1_156, 100, false,
        )))
    );

    assert!(app.runtime_bindings().shamap_store.is_some());
    assert!(app.shamap_store_service().is_some());
    assert_eq!(health.operating_mode(), SHAMapStoreOperatingMode::Full);
    assert_eq!(
        app.shamap_store_operating_mode(),
        Some(SHAMapStoreOperatingMode::Full)
    );
    assert_eq!(
        health.validated_ledger_age(),
        std::time::Duration::from_secs(20)
    );
    assert_eq!(app.validated_ledger_seq(), Some(1_156));
    assert_eq!(component.snapshot().queued_ledger_seq(), Some(1_156));
}

#[test]
fn application_root_can_note_validated_ledger_without_store_hooks_for_sync() {
    let mut app = ApplicationRoot::new(0).expect("root shell should build");
    let health = Arc::new(SharedSHAMapStoreHealthState::new(Arc::new(
        FixedCloseTimeProvider,
    )));
    let component = Arc::new(SHAMapStoreComponent::new(
        SHAMapStore::new(256, false, 9),
        Box::new(ServiceRuntime),
        None,
    ));
    let service = Arc::new(SHAMapStoreService::new(component.clone(), health));

    assert!(app.attach_shamap_store_service(service).is_none());
    app.note_validated_ledger_for_sync(Arc::new(Ledger::from_ledger_seq_and_close_time(
        1_157, 100, false,
    )));

    assert_eq!(app.validated_ledger_seq(), Some(1_157));
    assert_eq!(
        component.snapshot().queued_ledger_seq(),
        None,
        "sync hot path must not run heavier validated-ledger store hooks before publish advancement"
    );
}

#[test]
fn application_root_tracks_network_ops_operating_mode_strings() {
    let app = ApplicationRoot::new(0).expect("root shell should build");

    assert_eq!(
        app.network_ops_operating_mode(),
        NetworkOpsOperatingMode::Disconnected
    );
    assert_eq!(app.network_ops_operating_mode_string(), "disconnected");

    let previous = app.set_network_ops_operating_mode(NetworkOpsOperatingMode::Tracking);
    assert_eq!(previous, NetworkOpsOperatingMode::Disconnected);
    assert_eq!(
        app.network_ops_operating_mode(),
        NetworkOpsOperatingMode::Tracking
    );
    assert_eq!(app.network_ops_operating_mode_string(), "tracking");
}

#[test]
fn application_root_normalizes_connected_to_syncing_with_fresh_validated_ledger() {
    let app = ApplicationRoot::new(0).expect("root shell should build");
    let now_close_time = app.time_keeper().close_time().as_seconds();
    app.note_validated_ledger_for_sync(Arc::new(Ledger::from_ledger_seq_and_close_time(
        1_200,
        now_close_time,
        false,
    )));

    let previous = app.set_network_ops_operating_mode(NetworkOpsOperatingMode::Connected);

    assert_eq!(previous, NetworkOpsOperatingMode::Disconnected);
    assert_eq!(
        app.network_ops_operating_mode(),
        NetworkOpsOperatingMode::Syncing
    );
    assert_eq!(app.network_ops_operating_mode_string(), "syncing");
}

#[test]
fn application_root_can_start_network_ops_in_full_mode_when_start_valid_is_set() {
    let app = ApplicationRoot::with_options(super::ApplicationRootOptions {
        start_valid: true,
        ..super::ApplicationRootOptions::default()
    })
    .expect("root shell should build");

    assert_eq!(
        app.network_ops_operating_mode(),
        NetworkOpsOperatingMode::Full
    );
    assert_eq!(app.network_ops_operating_mode_string(), "full");
}

#[test]
fn application_root_tracks_validated_and_published_ledgers_without_service() {
    let app = ApplicationRoot::new(0).expect("root shell should build");
    app.on_closed_ledger(Arc::new(Ledger::from_ledger_seq_and_close_time(
        1_154, 95, false,
    )));
    assert!(
        app.on_validated_ledger(Arc::new(Ledger::from_ledger_seq_and_close_time(
            1_156, 100, false,
        )))
    );
    app.on_published_ledger(Arc::new(Ledger::from_ledger_seq_and_close_time(
        1_155, 99, false,
    )));
    let now_close_time = app.time_keeper().close_time().as_seconds();
    app.ledger_master_state()
        .set_validated_close_time(now_close_time.saturating_sub(20));
    app.ledger_master_state()
        .set_published_close_time(now_close_time.saturating_sub(21));

    assert_eq!(app.closed_ledger_seq(), Some(1_154));
    assert_eq!(app.validated_ledger_seq(), Some(1_156));
    assert_eq!(app.published_ledger_seq(), Some(1_155));
    assert_eq!(
        app.validated_ledger_age(),
        std::time::Duration::from_secs(20)
    );
    assert_eq!(
        app.validated_ledger()
            .expect("validated ledger should exist")
            .header()
            .seq,
        1_156
    );
}

#[test]
fn application_root_can_own_ledger_master_runtime_local_and_held_tx_paths() {
    let mut app = ApplicationRoot::new(0).expect("root shell should build");
    let runtime = Arc::new(AppLedgerMasterRuntime::default());
    assert!(
        app.attach_ledger_master_runtime(Arc::clone(&runtime))
            .is_none()
    );
    assert!(app.ledger_master_runtime().is_some());

    let source = account("4444444444444444444444444444444444444444");
    let current = payment_tx(
        source,
        account("AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"),
        5,
        None,
        10,
    );
    let next = payment_tx(
        source,
        account("BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB"),
        6,
        None,
        11,
    );

    runtime.push_local_tx(10, Arc::clone(&current));
    assert_eq!(app.local_tx_count(), Some(1));
    assert!(
        app.update_local_tx(&ledger_view(11, source, 5, &[current.get_transaction_id()]))
            .expect("local tx update should succeed")
    );
    assert_eq!(app.local_tx_count(), Some(0));

    let next_tx = Transaction::new(Arc::clone(&next));
    assert!(app.add_held_transaction(&next_tx));
    assert_eq!(app.held_transaction_count(), Some(1));
    assert_eq!(
        app.pop_acct_transaction(&Transaction::new(Arc::clone(&current)))
            .expect("next sequence should pop")
            .get_transaction_id(),
        next.get_transaction_id()
    );
    assert_eq!(app.held_transaction_count(), Some(0));
}

#[test]
fn application_root_can_own_network_ops_runtime_and_bridge_held_tx_queue() {
    let mut app = ApplicationRoot::new(0).expect("root shell should build");
    let runtime = app.attach_default_network_ops_runtime();
    assert!(app.network_ops_runtime().is_some());
    assert!(app.ledger_master_runtime().is_some());

    let source = account("5555555555555555555555555555555555555555");
    let first = payment_tx(
        source,
        account("AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"),
        1,
        None,
        10,
    );
    let second = payment_tx(
        source,
        account("BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB"),
        2,
        None,
        11,
    );

    runtime
        .ledger_master_runtime()
        .add_held_sttx(Arc::clone(&second));
    runtime
        .ledger_master_runtime()
        .add_held_sttx(Arc::clone(&first));

    let syncs = Mutex::new(Vec::new());
    let outcome = app
        .apply_held_transactions_to_network_ops(SHAMapHash::new(Uint256::from_u64(101)), |sync| {
            syncs
                .lock()
                .expect("sync mutex must not be poisoned")
                .push(sync);
        })
        .expect("network ops runtime should be attached");

    assert_eq!(
        outcome,
        AppNetworkOpsApplyHeldOutcome {
            drained_count: 2,
            process_outcome: Some(NetworkOpsTransactionSetOutcome::SyncBatch { added_count: 2 }),
        }
    );
    assert_eq!(app.network_ops_pending_transaction_count(), Some(2));
    assert_eq!(app.network_ops_submit_held_count(), Some(0));
    assert_eq!(
        syncs.into_inner().expect("sync mutex must not be poisoned"),
        vec![NetworkOpsProcessSetOwnerSync {
            added_count: 2,
            had_pending_before: false,
            has_applying_after_merge: true,
        }]
    );
}

#[test]
fn application_root_accepts_a_standalone_ledger_and_promotes_live_state() {
    let mut app = ApplicationRoot::with_options(super::ApplicationRootOptions {
        standalone: true,
        ..super::ApplicationRootOptions::default()
    })
    .expect("standalone root shell should build");
    let _runtime = app.attach_default_network_ops_runtime();

    let (source, tx) = signed_payment_tx(
        0x66,
        account("AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"),
        1,
        10,
    );
    let mut parent = ledger_view(1, source, 1, &[]);
    parent.set_accepted(1_111, ledger::LEDGER_DEFAULT_TIME_RESOLUTION, true);
    let parent = Arc::new(parent);
    app.on_closed_ledger(Arc::clone(&parent));
    let _ = app.open_ledger().modify(|view| {
        *view = AppOpenLedgerView::new(2, 10);
        true
    });

    let tx_id = tx.get_transaction_id();
    let mut cached = Arc::new(Mutex::new(Transaction::new(Arc::clone(&tx))));
    app.canonicalize_transaction(&mut cached);
    let transaction = Transaction::new(Arc::clone(&tx));
    assert!(app.add_held_transaction(&transaction));
    assert_eq!(app.held_transaction_count(), Some(1));

    let next_open_index = app
        .accept_standalone_ledger()
        .expect("standalone accept should succeed");

    assert_eq!(next_open_index, 3);
    assert_eq!(app.closed_ledger_seq(), Some(2));
    assert_eq!(app.published_ledger_seq(), Some(2));
    assert_eq!(app.validated_ledger_seq(), Some(2));
    assert_eq!(app.live_current_ledger_index(), Some(3));
    assert_eq!(app.status_rpc_current_ledger_index(), Some(3));
    assert_eq!(app.network_ops_pending_transaction_count(), Some(0));
    assert_eq!(app.held_transaction_count(), Some(0));

    let cached = app
        .fetch_cached_transaction(&tx_id)
        .expect("accepted tx should remain in cache");
    let cached = cached
        .lock()
        .expect("transaction mutex must not be poisoned");
    assert_eq!(cached.get_ledger(), 2);
    assert!(cached.is_validated());
}

#[test]
fn application_root_accept_ledger_builds_from_closed_parent_view() {
    let app = ApplicationRoot::new(0).expect("root shell should build");
    let parent_account = account("7777777777777777777777777777777777777777");
    let mut parent = ledger_view(1, parent_account, 1, &[]);
    parent.set_accepted(1_111, ledger::LEDGER_DEFAULT_TIME_RESOLUTION, true);
    let parent = Arc::new(parent);

    app.on_closed_ledger(Arc::clone(&parent));

    let next_open = app
        .accept_ledger(2, 1_234, 10)
        .expect("ledger accept should complete");
    let closed = app
        .closed_ledger()
        .expect("closed ledger should be recorded");

    assert_eq!(next_open, 3);
    assert_eq!(closed.header().seq, 2);
    assert_eq!(closed.header().parent_hash, parent.header().hash);
    assert!(
        closed
            .read(account_keylet(raw_account_id(parent_account)))
            .expect("closed ledger read should succeed")
            .is_some()
    );
}

#[test]
fn application_root_server_okay_matches_current_gate_order() {
    let app = ApplicationRoot::with_options(super::ApplicationRootOptions {
        elb_support: true,
        ..super::ApplicationRootOptions::default()
    })
    .expect("root shell should build");

    assert_eq!(app.server_okay(), Err(crate::SERVER_OKAY_NOT_SYNCED_REASON));

    app.set_need_network_ledger(true);
    assert_eq!(
        app.server_okay(),
        Err(crate::SERVER_OKAY_NEED_NETWORK_LEDGER_REASON)
    );

    app.set_need_network_ledger(false);
    app.set_amendment_blocked(true);
    assert_eq!(
        app.server_okay(),
        Err(crate::SERVER_OKAY_AMENDMENT_BLOCKED_REASON)
    );

    app.set_amendment_blocked(false);
    app.set_unl_blocked(true);
    assert_eq!(
        app.server_okay(),
        Err(crate::SERVER_OKAY_UNL_BLOCKED_REASON)
    );

    app.set_unl_blocked(false);
    app.set_network_ops_operating_mode(NetworkOpsOperatingMode::Full);
    app.on_published_ledger(Arc::new(Ledger::from_ledger_seq_and_close_time(
        1_155, 99, false,
    )));
    let now_close_time = app.time_keeper().close_time().as_seconds();
    app.ledger_master_state()
        .set_published_close_time(now_close_time.saturating_sub(21));
    assert_eq!(app.server_okay(), Err("No published ledger"));

    app.on_validated_ledger(Arc::new(Ledger::from_ledger_seq_and_close_time(
        1_156, 100, false,
    )));
    app.ledger_master_state()
        .set_published_close_time(now_close_time.saturating_sub(21));
    app.ledger_master_state()
        .set_validated_close_time(now_close_time.saturating_sub(20));
    assert_eq!(app.server_okay(), Ok(()));

    use crate::load::load_manager::LoadFeeControl;
    assert!(!app.load_fee_track().raise_local_fee());
    assert!(app.load_fee_track().raise_local_fee());
    assert_eq!(
        app.server_okay(),
        Err(crate::SERVER_OKAY_TOO_MUCH_LOAD_REASON)
    );
}

#[test]
fn application_root_attach_shamap_store_component_builds_service_from_root_time_keeper() {
    let mut app = ApplicationRoot::new(0).expect("root shell should build");
    let component = Arc::new(SHAMapStoreComponent::new(
        SHAMapStore::new(256, false, 9),
        Box::new(ServiceRuntime),
        None,
    ));
    let service = app.attach_shamap_store_component(component.clone());

    assert!(app.shamap_store_service().is_some());
    assert!(app.runtime_bindings().shamap_store.is_some());
    assert_eq!(service.component().fd_required(), component.fd_required());
    assert_eq!(service.validated_ledger_seq(), None);
}

#[test]
fn attached_shamap_store_service_reads_root_network_ops_mode() {
    let mut app = ApplicationRoot::new(0).expect("root shell should build");
    let component = Arc::new(SHAMapStoreComponent::new(
        SHAMapStore::new(256, false, 9),
        Box::new(ServiceRuntime),
        None,
    ));
    let service = app.attach_shamap_store_component(component);

    assert_eq!(service.operating_mode(), SHAMapStoreOperatingMode::Other);

    app.set_network_ops_operating_mode(NetworkOpsOperatingMode::Tracking);
    assert_eq!(service.operating_mode(), SHAMapStoreOperatingMode::Other);

    app.set_network_ops_operating_mode(NetworkOpsOperatingMode::Full);
    assert_eq!(service.operating_mode(), SHAMapStoreOperatingMode::Full);
}

#[test]
fn attached_shamap_store_service_reads_root_validated_age_from_ledger_master_state() {
    let mut app = ApplicationRoot::new(0).expect("root shell should build");
    let component = Arc::new(SHAMapStoreComponent::new(
        SHAMapStore::new(256, false, 9),
        Box::new(ServiceRuntime),
        None,
    ));
    let service = app.attach_shamap_store_component(component);

    app.on_validated_ledger(Arc::new(Ledger::from_ledger_seq_and_close_time(
        1_156, 100, false,
    )));
    let now_close_time = app.time_keeper().close_time().as_seconds();
    app.ledger_master_state()
        .set_validated_close_time(now_close_time.saturating_sub(20));

    assert_eq!(service.validated_ledger_seq(), Some(1_156));
    assert_eq!(
        service.health().validated_ledger_age(),
        app.validated_ledger_age()
    );
}

#[test]
fn consensus_built_switches_lcl_without_promoting_validated_or_published() {
    let mut app = ApplicationRoot::new(0).expect("root shell should build");
    let ledger_master_runtime = Arc::new(AppLedgerMasterRuntime::default());
    let _ = app.attach_ledger_master_runtime(Arc::clone(&ledger_master_runtime));

    let parent = Arc::new(Ledger::from_ledger_seq_and_close_time(10, 1_000, false));
    app.on_closed_ledger(Arc::clone(&parent));
    app.on_published_ledger(Arc::clone(&parent));
    assert!(app.on_validated_ledger(Arc::clone(&parent)));

    let source = account("0000000000000000000000000000000000000044");
    let destination = account("0000000000000000000000000000000000000055");
    let current_tx = payment_tx(source, destination, 1, None, 10);
    let local_tx = payment_tx(
        account("0000000000000000000000000000000000000066"),
        account("0000000000000000000000000000000000000077"),
        1,
        None,
        10,
    );
    let _ = app.open_ledger().modify(|view| {
        *view = AppOpenLedgerView::with_parent_hash(
            11,
            parent.fees().base,
            *parent.header().hash.as_uint256(),
        );
        view.push_transaction(Arc::clone(&current_tx));
        true
    });
    ledger_master_runtime.push_local_tx(10, Arc::clone(&local_tx));

    let mut built = Ledger::from_ledger_seq_and_close_time(11, 1_010, false);
    built.set_immutable(false);
    let built = Arc::new(built);
    app.on_consensus_built_ledger(Arc::clone(&built));

    assert_eq!(app.closed_ledger_seq(), Some(11));
    assert_eq!(app.published_ledger_seq(), Some(10));
    assert_eq!(app.validated_ledger_seq(), Some(10));
    assert_eq!(
        ledger_master_runtime
            .ledger_master()
            .get_ledger_by_hash(built.header().hash)
            .expect("built ledger should be visible through closed-ledger lookup")
            .header()
            .seq,
        11
    );

    let current = app.open_ledger().current();
    assert_eq!(current.ledger_current_index, 12);
    assert_eq!(current.parent_hash, *built.header().hash.as_uint256());
    let tx_ids = current.tx_ids();
    assert_eq!(tx_ids.len(), 2);
    assert!(tx_ids.contains(&current_tx.get_transaction_id()));
    assert!(tx_ids.contains(&local_tx.get_transaction_id()));
    assert_eq!(app.status_rpc_current_ledger_index(), Some(12));
}
