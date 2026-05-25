//! Synchronized queue-view owner above the landed `TxQAccount`, fee-order,
//! single-candidate erase, and `eraseAndAdvance(...)` decision seams.
//!
//! This keeps a deterministic account-local view and fee-ordered view in sync.

use std::{
    collections::BTreeMap,
    ops::Bound::{Excluded, Unbounded},
};

use protocol::SeqProxy;

use crate::{
    AdvanceTarget, FeeQueueKey, OrderCandidates, QueueAdvanceCandidate, TxQAccount,
    choose_next_after_erase,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FeeQueueEntry<Account> {
    pub key: FeeQueueKey<Account>,
    pub candidate: QueueAdvanceCandidate,
}

impl<Account> FeeQueueEntry<Account> {
    pub const fn new(key: FeeQueueKey<Account>, candidate: QueueAdvanceCandidate) -> Self {
        Self { key, candidate }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QueueViewNext<Account> {
    End,
    FeeNext(FeeQueueKey<Account>),
    AccountNext(FeeQueueKey<Account>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueueEraseAdvanceResult<Account> {
    pub removed: FeeQueueKey<Account>,
    pub next: QueueViewNext<Account>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoveReplacedResult<Account> {
    pub removed: Option<FeeQueueKey<Account>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirectApplyCleanupResult<Account> {
    pub removed: Option<FeeQueueKey<Account>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct QueueViews<Account, T> {
    pub fee_order: Vec<FeeQueueEntry<Account>>,
    pub accounts: BTreeMap<Account, TxQAccount<Account, T>>,
}

impl<Account, T> QueueViews<Account, T> {
    pub fn new(
        accounts: BTreeMap<Account, TxQAccount<Account, T>>,
        fee_order: Vec<FeeQueueEntry<Account>>,
    ) -> Self {
        Self {
            fee_order,
            accounts,
        }
    }
}

impl<Account, T> QueueViews<Account, T>
where
    Account: Clone + Ord + PartialEq,
{
    pub fn find_fee_candidate_index(&self, target: &FeeQueueKey<Account>) -> Option<usize> {
        self.fee_order.iter().position(|entry| entry.key == *target)
    }

    pub fn next_fee_candidate_key(
        &self,
        target: &FeeQueueKey<Account>,
    ) -> Option<FeeQueueKey<Account>> {
        self.find_fee_candidate_index(target)
            .and_then(|index| self.fee_order.get(index + 1))
            .map(|entry| entry.key.clone())
    }

    pub fn remove_fee_candidate_by_key(
        &mut self,
        target: &FeeQueueKey<Account>,
    ) -> FeeQueueKey<Account> {
        let delete_index = self
            .find_fee_candidate_index(target)
            .expect("xrpl::TxQ::erase : candidate found in byFee");

        let found = self
            .accounts
            .get_mut(&target.account)
            .expect("xrpl::TxQ::erase : account found")
            .remove(target.seq_proxy);
        assert!(found, "xrpl::TxQ::erase : account removed");

        self.fee_order.remove(delete_index).key
    }

    pub fn insert_fee_entry(
        &mut self,
        entry: FeeQueueEntry<Account>,
        order: &OrderCandidates,
    ) -> usize {
        let insert_index = self
            .fee_order
            .iter()
            .position(|existing| {
                order.compares_by_fee_and_tx_id(
                    entry.candidate.fee_level,
                    entry.candidate.tx_id,
                    existing.candidate.fee_level,
                    existing.candidate.tx_id,
                )
            })
            .unwrap_or(self.fee_order.len());

        self.fee_order.insert(insert_index, entry);
        insert_index
    }

    pub fn erase_and_advance(
        &mut self,
        candidate_index: usize,
        order: &OrderCandidates,
    ) -> QueueEraseAdvanceResult<Account> {
        let current_entry = self
            .fee_order
            .get(candidate_index)
            .cloned()
            .expect("xrpl::TxQ::eraseAndAdvance : found in byFee");

        let txq_account = self
            .accounts
            .get(&current_entry.key.account)
            .expect("xrpl::TxQ::eraseAndAdvance : account found");
        let current_is_first_for_account = txq_account
            .transactions
            .first_key_value()
            .is_some_and(|(seq_proxy, _)| *seq_proxy == current_entry.key.seq_proxy);

        let account_next_key = txq_account
            .transactions
            .range((Excluded(current_entry.key.seq_proxy), Unbounded))
            .next()
            .map(|(seq_proxy, _)| FeeQueueKey::new(current_entry.key.account.clone(), *seq_proxy));
        let account_next_entry = account_next_key.as_ref().map(|key| {
            self.fee_order
                .iter()
                .find(|entry| entry.key == *key)
                .cloned()
                .expect("xrpl::TxQ::eraseAndAdvance : account next found in byFee")
        });
        let fee_next_entry = self.fee_order.get(candidate_index + 1).cloned();

        let next = match choose_next_after_erase(
            current_entry.candidate,
            current_is_first_for_account,
            account_next_entry.as_ref().map(|entry| entry.candidate),
            fee_next_entry.as_ref().map(|entry| entry.candidate),
            order,
        ) {
            AdvanceTarget::End => QueueViewNext::End,
            AdvanceTarget::FeeNext(_) => {
                QueueViewNext::FeeNext(fee_next_entry.expect("fee-next target must exist").key)
            }
            AdvanceTarget::AccountNext(_) => QueueViewNext::AccountNext(
                account_next_entry
                    .expect("account-next target must exist")
                    .key,
            ),
        };

        let removed = self.fee_order.remove(candidate_index).key;
        let found = self
            .accounts
            .get_mut(&removed.account)
            .expect("xrpl::TxQ::eraseAndAdvance : account found")
            .remove(removed.seq_proxy);
        assert!(found, "xrpl::TxQ::eraseAndAdvance : account found");

        QueueEraseAdvanceResult { removed, next }
    }

    pub fn remove_replaced_candidate(
        &mut self,
        replaced: Option<FeeQueueKey<Account>>,
        expected_account: &Account,
        expected_seq_proxy: SeqProxy,
    ) -> RemoveReplacedResult<Account> {
        let Some(replaced) = replaced else {
            return RemoveReplacedResult { removed: None };
        };

        let delete_index = self
            .fee_order
            .iter()
            .position(|entry| entry.key == replaced)
            .expect("xrpl::TxQ::removeFromByFee : found in byFee");

        assert_eq!(
            replaced.seq_proxy, expected_seq_proxy,
            "xrpl::TxQ::removeFromByFee : matching sequence"
        );
        assert!(
            &replaced.account == expected_account,
            "xrpl::TxQ::removeFromByFee : matching account"
        );

        let found = self
            .accounts
            .get_mut(&replaced.account)
            .expect("xrpl::TxQ::removeFromByFee : matching transaction")
            .remove(replaced.seq_proxy);
        assert!(found, "xrpl::TxQ::removeFromByFee : matching transaction");

        let removed = self.fee_order.remove(delete_index).key;
        RemoveReplacedResult {
            removed: Some(removed),
        }
    }

    pub fn cleanup_direct_apply_success(
        &mut self,
        applied_account: &Account,
        applied_seq_proxy: SeqProxy,
    ) -> DirectApplyCleanupResult<Account> {
        let replaced = self
            .accounts
            .get(applied_account)
            .and_then(|account_queue| {
                account_queue
                    .transactions
                    .contains_key(&applied_seq_proxy)
                    .then(|| FeeQueueKey::new(applied_account.clone(), applied_seq_proxy))
            });

        let result = self.remove_replaced_candidate(replaced, applied_account, applied_seq_proxy);
        DirectApplyCleanupResult {
            removed: result.removed,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use basics::base_uint::Uint256;
    use protocol::SeqProxy;

    use super::{
        DirectApplyCleanupResult, FeeQueueEntry, QueueEraseAdvanceResult, QueueViewNext,
        QueueViews, RemoveReplacedResult,
    };
    use crate::{
        FeeQueueKey, MaybeTxCore, OrderCandidates, QueueAdvanceCandidate, TxConsequences,
        TxQAccount,
    };

    fn candidate(seq_proxy: SeqProxy, tx_id: u64, fee_level: u64) -> QueueAdvanceCandidate {
        QueueAdvanceCandidate {
            fee_level,
            tx_id: Uint256::from_u64(tx_id),
            seq_proxy,
        }
    }

    #[test]
    fn erase_and_advance_prefers_account_next_even_when_it_is_elsewhere_in_fee_order() {
        let mut account_a = TxQAccount::new("a");
        account_a.add(
            SeqProxy::sequence(5),
            MaybeTxCore::new("a5", TxConsequences::new(1, SeqProxy::sequence(5))),
        );
        account_a.add(
            SeqProxy::sequence(6),
            MaybeTxCore::new("a6", TxConsequences::new(1, SeqProxy::sequence(6))),
        );

        let mut account_b = TxQAccount::new("b");
        account_b.add(
            SeqProxy::sequence(7),
            MaybeTxCore::new("b7", TxConsequences::new(1, SeqProxy::sequence(7))),
        );

        let mut views = QueueViews::new(
            BTreeMap::from([("a", account_a), ("b", account_b)]),
            vec![
                FeeQueueEntry::new(
                    FeeQueueKey::new("a", SeqProxy::sequence(6)),
                    candidate(SeqProxy::sequence(6), 6, 110),
                ),
                FeeQueueEntry::new(
                    FeeQueueKey::new("b", SeqProxy::sequence(7)),
                    candidate(SeqProxy::sequence(7), 7, 105),
                ),
                FeeQueueEntry::new(
                    FeeQueueKey::new("a", SeqProxy::sequence(5)),
                    candidate(SeqProxy::sequence(5), 5, 100),
                ),
            ],
        );

        let result = views.erase_and_advance(2, &OrderCandidates::new(Uint256::from_u64(0)));

        assert_eq!(
            result,
            QueueEraseAdvanceResult {
                removed: FeeQueueKey::new("a", SeqProxy::sequence(5)),
                next: QueueViewNext::AccountNext(FeeQueueKey::new("a", SeqProxy::sequence(6))),
            }
        );
        assert_eq!(views.accounts["a"].get_txn_count(), 1);
        assert!(
            views.accounts["a"]
                .transactions
                .contains_key(&SeqProxy::sequence(6))
        );
        assert_eq!(
            views.find_fee_candidate_index(&FeeQueueKey::new("b", SeqProxy::sequence(7))),
            Some(1)
        );
    }

    #[test]
    fn erase_and_advance_falls_back_to_fee_next_when_it_stays_ahead() {
        let mut account_a = TxQAccount::new("a");
        account_a.add(
            SeqProxy::sequence(5),
            MaybeTxCore::new("a5", TxConsequences::new(1, SeqProxy::sequence(5))),
        );
        account_a.add(
            SeqProxy::sequence(6),
            MaybeTxCore::new("a6", TxConsequences::new(1, SeqProxy::sequence(6))),
        );

        let mut account_b = TxQAccount::new("b");
        account_b.add(
            SeqProxy::sequence(7),
            MaybeTxCore::new("b7", TxConsequences::new(1, SeqProxy::sequence(7))),
        );

        let mut views = QueueViews::new(
            BTreeMap::from([("a", account_a), ("b", account_b)]),
            vec![
                FeeQueueEntry::new(
                    FeeQueueKey::new("a", SeqProxy::sequence(5)),
                    candidate(SeqProxy::sequence(5), 5, 100),
                ),
                FeeQueueEntry::new(
                    FeeQueueKey::new("b", SeqProxy::sequence(7)),
                    candidate(SeqProxy::sequence(7), 7, 95),
                ),
                FeeQueueEntry::new(
                    FeeQueueKey::new("a", SeqProxy::sequence(6)),
                    candidate(SeqProxy::sequence(6), 6, 90),
                ),
            ],
        );

        let result = views.erase_and_advance(0, &OrderCandidates::new(Uint256::from_u64(0)));

        assert_eq!(
            result,
            QueueEraseAdvanceResult {
                removed: FeeQueueKey::new("a", SeqProxy::sequence(5)),
                next: QueueViewNext::FeeNext(FeeQueueKey::new("b", SeqProxy::sequence(7))),
            }
        );
        assert_eq!(views.accounts["a"].get_txn_count(), 1);
        assert!(
            views.accounts["a"]
                .transactions
                .contains_key(&SeqProxy::sequence(6))
        );
        assert_eq!(
            views.next_fee_candidate_key(&FeeQueueKey::new("a", SeqProxy::sequence(6))),
            None
        );
    }

    #[test]
    fn remove_replaced_candidate_removes_matching_entry_from_both_views() {
        let mut account_a = TxQAccount::new("a");
        account_a.add(
            SeqProxy::sequence(5),
            MaybeTxCore::new("a5", TxConsequences::new(1, SeqProxy::sequence(5))),
        );

        let mut views = QueueViews::new(
            BTreeMap::from([("a", account_a)]),
            vec![FeeQueueEntry::new(
                FeeQueueKey::new("a", SeqProxy::sequence(5)),
                candidate(SeqProxy::sequence(5), 5, 100),
            )],
        );

        let result = views.remove_replaced_candidate(
            Some(FeeQueueKey::new("a", SeqProxy::sequence(5))),
            &"a",
            SeqProxy::sequence(5),
        );

        assert_eq!(
            result,
            RemoveReplacedResult {
                removed: Some(FeeQueueKey::new("a", SeqProxy::sequence(5))),
            }
        );
        assert!(views.fee_order.is_empty());
        assert!(views.accounts["a"].empty());
    }

    #[test]
    fn remove_replaced_candidate_is_noop_when_no_replacement_exists() {
        let views = QueueViews::<&str, &str>::default();
        let mut views = views;

        let result = views.remove_replaced_candidate(None, &"a", SeqProxy::sequence(5));

        assert_eq!(result, RemoveReplacedResult { removed: None });
        assert!(views.fee_order.is_empty());
        assert!(views.accounts.is_empty());
    }

    #[test]
    fn cleanup_direct_apply_success_removes_matching_replacement_from_both_views() {
        let mut account_a = TxQAccount::new("a");
        account_a.add(
            SeqProxy::sequence(5),
            MaybeTxCore::new("a5", TxConsequences::new(1, SeqProxy::sequence(5))),
        );

        let mut views = QueueViews::new(
            BTreeMap::from([("a", account_a)]),
            vec![FeeQueueEntry::new(
                FeeQueueKey::new("a", SeqProxy::sequence(5)),
                candidate(SeqProxy::sequence(5), 5, 100),
            )],
        );

        let result = views.cleanup_direct_apply_success(&"a", SeqProxy::sequence(5));

        assert_eq!(
            result,
            DirectApplyCleanupResult {
                removed: Some(FeeQueueKey::new("a", SeqProxy::sequence(5))),
            }
        );
        assert!(views.fee_order.is_empty());
        assert!(views.accounts["a"].empty());
    }

    #[test]
    fn cleanup_direct_apply_success_is_noop_when_account_has_no_matching_replacement() {
        let mut account_a = TxQAccount::new("a");
        account_a.add(
            SeqProxy::sequence(6),
            MaybeTxCore::new("a6", TxConsequences::new(1, SeqProxy::sequence(6))),
        );

        let mut views = QueueViews::new(
            BTreeMap::from([("a", account_a)]),
            vec![FeeQueueEntry::new(
                FeeQueueKey::new("a", SeqProxy::sequence(6)),
                candidate(SeqProxy::sequence(6), 6, 100),
            )],
        );

        let result = views.cleanup_direct_apply_success(&"a", SeqProxy::sequence(5));

        assert_eq!(result, DirectApplyCleanupResult { removed: None });
        assert_eq!(views.fee_order.len(), 1);
        assert_eq!(views.accounts["a"].get_txn_count(), 1);
    }

    #[test]
    fn remove_fee_candidate_by_key_removes_matching_entry_from_both_views() {
        let mut account_a = TxQAccount::new("a");
        account_a.add(
            SeqProxy::sequence(5),
            MaybeTxCore::new("a5", TxConsequences::new(1, SeqProxy::sequence(5))),
        );

        let mut views = QueueViews::new(
            BTreeMap::from([("a", account_a)]),
            vec![FeeQueueEntry::new(
                FeeQueueKey::new("a", SeqProxy::sequence(5)),
                candidate(SeqProxy::sequence(5), 5, 100),
            )],
        );

        let removed =
            views.remove_fee_candidate_by_key(&FeeQueueKey::new("a", SeqProxy::sequence(5)));

        assert_eq!(removed, FeeQueueKey::new("a", SeqProxy::sequence(5)));
        assert!(views.fee_order.is_empty());
        assert!(views.accounts["a"].empty());
    }

    #[test]
    fn insert_fee_entry_orders_by_fee_then_xor_hash_tie_breaker() {
        let mut views = QueueViews::<&str, &str>::default();

        let first_index = views.insert_fee_entry(
            FeeQueueEntry::new(
                FeeQueueKey::new("b", SeqProxy::sequence(6)),
                candidate(SeqProxy::sequence(6), 2, 100),
            ),
            &OrderCandidates::new(Uint256::from_u64(0)),
        );
        let second_index = views.insert_fee_entry(
            FeeQueueEntry::new(
                FeeQueueKey::new("a", SeqProxy::sequence(5)),
                candidate(SeqProxy::sequence(5), 1, 100),
            ),
            &OrderCandidates::new(Uint256::from_u64(0)),
        );

        assert_eq!(first_index, 0);
        assert_eq!(second_index, 0);
        assert_eq!(
            views
                .fee_order
                .iter()
                .map(|entry| entry.key.clone())
                .collect::<Vec<_>>(),
            vec![
                FeeQueueKey::new("a", SeqProxy::sequence(5)),
                FeeQueueKey::new("b", SeqProxy::sequence(6)),
            ]
        );
    }
}
