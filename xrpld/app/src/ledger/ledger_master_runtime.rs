//! App-owned `LedgerMaster` bridge for local and held transaction runtime.
//!
//! This crate owns the full reference `LedgerMaster` / `NetworkOPs` graph and
//! the owner-level transaction lifecycle around `LocalTxs` and held transactions:
//! - `updateLocalTx(ReadView const&)` through the real `LocalTxs::sweep(...)`,
//! - `getLocalTxCount()` through the real ledger-owned container,
//! - `addHeldTransaction(...)` through the real ledger-owned canonical set,
//! - `popAcctTransaction(...)` when an account transaction succeeds,
//! - and `applyHeldTransactions()` through a caller-owned transaction-set sink.
//!
//! This keeps those behaviors attached to one app-facing owner instead of
//! leaving them as isolated crate-local helpers.

use crate::tx_queue::transaction::Transaction;
use basics::base_uint::Uint256;
use basics::hardened_hash::HardenedHashBuilder;
use basics::sha_map_hash::SHAMapHash;
use basics::tagged_cache::MonotonicClock;
use ledger::{CanonicalTXSet, Ledger, LedgerMaster, LedgerMasterConfig, NullLedgerJournal};
use protocol::STTx;
use shamap::traversal::TraversalError;
use std::sync::{Arc, Mutex};

pub type AppLedgerMaster = LedgerMaster<MonotonicClock, HardenedHashBuilder>;

#[derive(Debug, Clone)]
pub struct AppLedgerMasterRuntime {
    ledger_master: Arc<AppLedgerMaster>,
    /// Hash of the consensus ledger being requested from peers.
    /// Set by `request_consensus_ledger`; consumed by the bootstrap event loop.
    pub(crate) pending_consensus_ledger: Arc<Mutex<Option<Uint256>>>,
    /// Receiver for completed InboundLedger results.
    /// Polled by the bootstrap thread (50ms) for immediate storeLedger.
    pub(crate) completed_ledgers_rx: Arc<Mutex<Option<std::sync::mpsc::Receiver<Arc<ledger::Ledger>>>>>,
}

const APP_LEDGER_MASTER_MAX_PUBLISH_GAP: u32 = 100;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppLedgerMasterPublishAdvance {
    FirstPublished,
    GapTooLarge,
    Sequential,
    NothingToPublish,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AppLedgerMasterMissingLedger {
    pub hash: Uint256,
    pub seq: u32,
}

#[derive(Debug, Clone)]
pub struct AppLedgerMasterAdvanceReport {
    pub decision: AppLedgerMasterPublishAdvance,
    pub published: Vec<Arc<Ledger>>,
    pub missing: Option<AppLedgerMasterMissingLedger>,
}

impl Default for AppLedgerMasterRuntime {
    fn default() -> Self {
        Self::new(LedgerMasterConfig::default())
    }
}

impl AppLedgerMasterRuntime {
    pub fn new(config: LedgerMasterConfig) -> Self {
        Self::with_ledger_master(Arc::new(AppLedgerMaster::new(
            MonotonicClock::default(),
            config,
        )))
    }

    pub fn with_ledger_master(ledger_master: Arc<AppLedgerMaster>) -> Self {
        Self {
            ledger_master,
            pending_consensus_ledger: Arc::new(Mutex::new(None)),
            completed_ledgers_rx: Arc::new(Mutex::new(None)),
        }
    }

    pub fn ledger_master(&self) -> Arc<AppLedgerMaster> {
        Arc::clone(&self.ledger_master)
    }

    /// Returns the hash of the consensus ledger currently being requested,
    /// if any. The bootstrap event loop polls this to drive acquisition.
    pub fn take_pending_consensus_ledger(&self) -> Option<Uint256> {
        self.pending_consensus_ledger
            .lock()
            .expect("pending_consensus_ledger mutex must not be poisoned")
            .take()
    }

    /// Returns the hash without consuming it (for polling without clearing).
    pub fn pending_consensus_ledger(&self) -> Option<Uint256> {
        *self
            .pending_consensus_ledger
            .lock()
            .expect("pending_consensus_ledger mutex must not be poisoned")
    }

    pub fn update_local_tx(&self, view: &Ledger) -> Result<(), TraversalError> {
        self.ledger_master.local_txs().sweep(view)
    }

    pub fn get_local_tx_count(&self) -> usize {
        self.ledger_master.local_txs().size()
    }

    pub fn local_tx_set(&self) -> CanonicalTXSet {
        self.ledger_master.local_txs().get_tx_set()
    }

    pub fn push_local_tx(&self, index: u32, transaction: Arc<STTx>) {
        self.ledger_master.local_txs().push_back(index, transaction);
    }

    pub fn add_held_transaction(&self, transaction: &Transaction) {
        self.ledger_master
            .add_held_transaction(Arc::clone(transaction.get_s_transaction()));
    }

    pub fn add_held_sttx(&self, transaction: Arc<STTx>) {
        self.ledger_master.add_held_transaction(transaction);
    }

    pub fn held_transaction_count(&self) -> usize {
        self.ledger_master.held_transaction_count()
    }

    pub fn pop_acct_transaction(&self, tx: &Arc<STTx>) -> Option<Arc<STTx>> {
        self.ledger_master.pop_acct_transaction(tx)
    }

    pub fn pop_acct_transaction_for(&self, transaction: &Transaction) -> Option<Arc<STTx>> {
        self.pop_acct_transaction(transaction.get_s_transaction())
    }

    pub fn take_held_transactions(
        &self,
        next_open_ledger_parent_hash: SHAMapHash,
    ) -> CanonicalTXSet {
        self.ledger_master
            .take_held_transactions(*next_open_ledger_parent_hash.as_uint256())
    }

    pub fn apply_held_transactions<F>(
        &self,
        next_open_ledger_parent_hash: SHAMapHash,
        process_transaction_set: F,
    ) -> usize
    where
        F: FnMut(CanonicalTXSet),
    {
        self.ledger_master.apply_held_transactions(
            *next_open_ledger_parent_hash.as_uint256(),
            process_transaction_set,
        )
    }

    pub fn check_accept(&self, hash: Uint256, seq: u32) -> bool {
        if seq <= self.ledger_master.valid_ledger_seq() {
            return false;
        }
        self.ledger_master
            .get_ledger_by_hash(SHAMapHash::new(hash))
            .is_some()
    }

    pub fn plan_advance_publication(&self) -> AppLedgerMasterAdvanceReport {
        let Some(validated) = self.ledger_master.validated_ledger() else {
            return AppLedgerMasterAdvanceReport {
                decision: AppLedgerMasterPublishAdvance::NothingToPublish,
                published: Vec::new(),
                missing: None,
            };
        };

        let valid_seq = validated.header().seq;
        let published = self.ledger_master.published_ledger();
        let decision = match published.as_ref().map(|ledger| ledger.header().seq) {
            None => AppLedgerMasterPublishAdvance::FirstPublished,
            Some(published_seq)
                if valid_seq > published_seq.saturating_add(APP_LEDGER_MASTER_MAX_PUBLISH_GAP) =>
            {
                AppLedgerMasterPublishAdvance::GapTooLarge
            }
            Some(published_seq) if valid_seq <= published_seq => {
                AppLedgerMasterPublishAdvance::NothingToPublish
            }
            Some(_) => AppLedgerMasterPublishAdvance::Sequential,
        };

        match decision {
            AppLedgerMasterPublishAdvance::NothingToPublish => AppLedgerMasterAdvanceReport {
                decision,
                published: Vec::new(),
                missing: None,
            },
            AppLedgerMasterPublishAdvance::FirstPublished
            | AppLedgerMasterPublishAdvance::GapTooLarge => AppLedgerMasterAdvanceReport {
                decision,
                published: vec![validated],
                missing: None,
            },
            AppLedgerMasterPublishAdvance::Sequential => {
                let mut to_publish = Vec::new();
                let mut missing = None;
                let mut next_seq = published
                    .as_ref()
                    .map(|ledger| ledger.header().seq)
                    .unwrap_or(0)
                    .saturating_add(1);

                while next_seq <= valid_seq {
                    let next_ledger = if next_seq == valid_seq {
                        Some(Arc::clone(&validated))
                    } else {
                        let Some(hash) = validated.hash_of_seq(next_seq, &NullLedgerJournal) else {
                            break;
                        };
                        if hash.is_zero() {
                            break;
                        }
                        self.ledger_master.get_ledger_by_hash(hash).or_else(|| {
                            missing = Some(AppLedgerMasterMissingLedger {
                                hash: *hash.as_uint256(),
                                seq: next_seq,
                            });
                            None
                        })
                    };

                    let Some(next_ledger) = next_ledger else {
                        break;
                    };

                    if next_ledger.header().seq != next_seq {
                        break;
                    }

                    if let Some(previous) = to_publish
                        .last()
                        .cloned()
                        .or_else(|| self.ledger_master.published_ledger())
                        && previous.header().hash != next_ledger.header().parent_hash
                    {
                        break;
                    }

                    to_publish.push(next_ledger);
                    next_seq = next_seq.saturating_add(1);
                }

                AppLedgerMasterAdvanceReport {
                    decision,
                    published: to_publish,
                    missing,
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{AppLedgerMasterPublishAdvance, AppLedgerMasterRuntime};
    use crate::tx_queue::transaction::Transaction;
    use basics::base_uint::{Uint160, Uint256};
    use basics::sha_map_hash::SHAMapHash;
    use ledger::{LEDGER_DEFAULT_TIME_RESOLUTION, Ledger, LedgerHeader};
    use protocol::{
        AccountID, LedgerEntryType, STAmount, STLedgerEntry, STTx, TxType, account_keylet,
        get_field_by_symbol,
    };
    use shamap::item::SHAMapItem;
    use shamap::mutation::MutableTree;
    use shamap::sync::{SHAMapType, SyncState, SyncTree};
    use shamap::tree_node::SHAMapNodeType;
    use std::sync::Arc;

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

    fn ledger_view(
        seq: u32,
        account: AccountID,
        account_sequence: u32,
        tx_ids: &[Uint256],
    ) -> Ledger {
        let mut state_tree = MutableTree::new(1);
        let mut account_root = STLedgerEntry::from_type_and_key(
            LedgerEntryType::AccountRoot,
            account_keylet(raw_account_id(account)).key,
        );
        account_root.set_account_id(get_field_by_symbol("sfAccount"), account);
        account_root.set_field_u32(get_field_by_symbol("sfSequence"), account_sequence);
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
                close_time: 500 + seq,
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

    fn immutable_ledger(seq: u32, hash_seed: u8) -> Arc<Ledger> {
        let mut header = LedgerHeader {
            seq,
            close_time: 500 + seq,
            close_time_resolution: LEDGER_DEFAULT_TIME_RESOLUTION,
            ..LedgerHeader::default()
        };
        header.hash = SHAMapHash::new(Uint256::from_array([hash_seed; 32]));
        let mut ledger = Ledger::new(header, false);
        ledger.set_immutable(true);
        Arc::new(ledger)
    }

    fn linked_ledger(previous: &Arc<Ledger>, close_time: u32) -> Arc<Ledger> {
        let mut ledger = Ledger::from_previous(previous.as_ref(), close_time);
        ledger
            .update_skip_list()
            .expect("skip list should update for linked ledger");
        ledger.set_immutable(true);
        Arc::new(ledger)
    }

    #[test]
    fn runtime_updates_local_tx_count_from_real_localtxs_owner() {
        let runtime = AppLedgerMasterRuntime::default();
        let source = account("1111111111111111111111111111111111111111");
        let tx = payment_tx(
            source,
            account("AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"),
            1,
            None,
            10,
        );
        let tx_id = tx.get_transaction_id();

        runtime.push_local_tx(10, Arc::clone(&tx));
        assert_eq!(runtime.get_local_tx_count(), 1);

        runtime
            .update_local_tx(&ledger_view(11, source, 1, &[tx_id]))
            .expect("local tx sweep should succeed");

        assert_eq!(runtime.get_local_tx_count(), 0);
    }

    #[test]
    fn runtime_adds_pops_and_applies_held_transactions() {
        let runtime = AppLedgerMasterRuntime::default();
        let source = account("2222222222222222222222222222222222222222");
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
        let ticket = payment_tx(
            source,
            account("CCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCC"),
            0,
            Some(2),
            12,
        );

        let next_tx = Transaction::new(Arc::clone(&next));
        runtime.add_held_transaction(&next_tx);
        runtime.add_held_sttx(Arc::clone(&ticket));

        assert_eq!(runtime.held_transaction_count(), 2);
        assert_eq!(
            runtime
                .pop_acct_transaction(&current)
                .expect("next sequence should pop")
                .get_transaction_id(),
            next.get_transaction_id()
        );
        assert_eq!(runtime.held_transaction_count(), 1);

        let mut drained = Vec::new();
        let count = runtime
            .apply_held_transactions(SHAMapHash::new(Uint256::from_u64(55)), |set| {
                drained = set.iter().map(|tx| tx.get_transaction_id()).collect()
            });

        assert_eq!(count, 1);
        assert_eq!(drained, vec![ticket.get_transaction_id()]);
        assert_eq!(runtime.held_transaction_count(), 0);

        runtime.add_held_sttx(Arc::clone(&next));
        let drained = runtime.take_held_transactions(SHAMapHash::new(Uint256::from_u64(77)));
        assert_eq!(drained.key(), Uint256::from_u64(55));
        assert_eq!(
            drained
                .iter()
                .map(|tx| tx.get_transaction_id())
                .collect::<Vec<_>>(),
            vec![next.get_transaction_id()]
        );
    }

    #[test]
    fn runtime_plans_first_publish_from_validated_ledger() {
        let runtime = AppLedgerMasterRuntime::default();
        let validated = immutable_ledger(25, 0x44);

        runtime
            .ledger_master()
            .set_valid_ledger(Arc::clone(&validated), None, None)
            .expect("validated ledger should update");

        let report = runtime.plan_advance_publication();
        assert_eq!(
            report.decision,
            AppLedgerMasterPublishAdvance::FirstPublished
        );
        assert_eq!(report.published.len(), 1);
        assert_eq!(report.published[0].header().hash, validated.header().hash);
        assert!(report.missing.is_none());
    }

    #[test]
    fn runtime_plans_exact_missing_publish_gap() {
        let runtime = AppLedgerMasterRuntime::default();
        let first = immutable_ledger(1, 0x10);
        let second = linked_ledger(&first, 600);
        let third = linked_ledger(&second, 700);

        runtime.ledger_master().set_pub_ledger(Arc::clone(&first));
        runtime
            .ledger_master()
            .set_valid_ledger(Arc::clone(&third), None, None)
            .expect("validated ledger should update");

        let report = runtime.plan_advance_publication();
        assert_eq!(report.decision, AppLedgerMasterPublishAdvance::Sequential);
        assert!(report.published.is_empty());
        let missing = report
            .missing
            .expect("seq=2 should be the first publish gap");
        assert_eq!(missing.seq, 2);
        assert_eq!(missing.hash, *second.header().hash.as_uint256());
    }
}
