//! Full-queue eviction branch for `TxQ::apply(...)`.
//!
//! This ports the deterministic "reject or evict the cheapest other-account
//! tail entry" rule after the later `canBeHeld(...)` fallback, plus the
//! synchronized erase mutation on the queue owner.

use std::collections::BTreeMap;

use protocol::Ter;

use crate::{FeeLevel64, FeeQueueEntry, FeeQueueKey, MaybeTxCore, QueueViews, TxQAccount};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QueueApplyFullQueueDecision<Account> {
    Bypass,
    RejectFullSameAccount,
    RejectFullLowerFee {
        end_effective_fee_level: FeeLevel64,
    },
    EvictCheapest {
        dropped: FeeQueueKey<Account>,
        end_effective_fee_level: FeeLevel64,
    },
}

impl<Account> QueueApplyFullQueueDecision<Account> {
    pub const fn ter(&self) -> Option<Ter> {
        match self {
            Self::RejectFullSameAccount | Self::RejectFullLowerFee { .. } => {
                Some(Ter::TEL_CAN_NOT_QUEUE_FULL)
            }
            Self::Bypass | Self::EvictCheapest { .. } => None,
        }
    }

    pub const fn rejects_full(&self) -> bool {
        matches!(
            self,
            Self::RejectFullSameAccount | Self::RejectFullLowerFee { .. }
        )
    }
}

pub fn apply_queue_apply_full_queue_decision<Account, T>(
    views: &mut QueueViews<Account, T>,
    decision: QueueApplyFullQueueDecision<Account>,
) -> QueueApplyFullQueueDecision<Account>
where
    Account: Clone + Ord + PartialEq,
{
    if let QueueApplyFullQueueDecision::EvictCheapest { dropped, .. } = &decision {
        let removed = views.remove_fee_candidate_by_key(dropped);
        assert!(
            removed == dropped.clone(),
            "xrpl::TxQ::apply : cheapest transaction found"
        );
    }

    decision
}

fn compute_end_effective_fee_level<Account, T, FeeLevelOf>(
    end_tail_fee_level: FeeLevel64,
    candidate_fee_level: FeeLevel64,
    end_account: &TxQAccount<Account, T>,
    mut fee_level_of: FeeLevelOf,
) -> FeeLevel64
where
    FeeLevelOf: FnMut(&MaybeTxCore<T>) -> FeeLevel64,
{
    if end_tail_fee_level > candidate_fee_level || end_account.transactions.len() == 1 {
        return end_tail_fee_level;
    }

    let count = end_account.transactions.len() as FeeLevel64;
    let mut total_div = 0_u64;
    let mut total_mod = 0_u64;

    for queued in end_account.transactions.values() {
        let fee_level = fee_level_of(queued);
        let next_div = fee_level / count;
        let next_mod = fee_level % count;

        if total_div >= FeeLevel64::MAX - next_div || total_mod >= FeeLevel64::MAX - next_mod {
            return FeeLevel64::MAX;
        }

        total_div += next_div;
        total_mod += next_mod;
    }

    total_div + total_mod / count
}

pub fn evaluate_queue_apply_full_queue<Account, T, FeeLevelOf>(
    replaces_existing: bool,
    queue_is_full: bool,
    candidate_account: &Account,
    candidate_fee_level: FeeLevel64,
    fee_order: &[FeeQueueEntry<Account>],
    accounts: &BTreeMap<Account, TxQAccount<Account, T>>,
    fee_level_of: FeeLevelOf,
) -> QueueApplyFullQueueDecision<Account>
where
    Account: Clone + Ord + PartialEq,
    FeeLevelOf: FnMut(&MaybeTxCore<T>) -> FeeLevel64 + Copy,
{
    if replaces_existing || !queue_is_full {
        return QueueApplyFullQueueDecision::Bypass;
    }

    let Some(last_other_account) = fee_order
        .iter()
        .rev()
        .find(|entry| &entry.key.account != candidate_account)
    else {
        return QueueApplyFullQueueDecision::RejectFullSameAccount;
    };

    let end_account = accounts
        .get(&last_other_account.key.account)
        .expect("xrpl::TxQ::apply : end account exists");
    let end_effective_fee_level = compute_end_effective_fee_level(
        last_other_account.candidate.fee_level,
        candidate_fee_level,
        end_account,
        fee_level_of,
    );

    if candidate_fee_level <= end_effective_fee_level {
        return QueueApplyFullQueueDecision::RejectFullLowerFee {
            end_effective_fee_level,
        };
    }

    let dropped = end_account
        .transactions
        .last_key_value()
        .map(|(seq_proxy, _)| FeeQueueKey::new(last_other_account.key.account.clone(), *seq_proxy))
        .expect("xrpl::TxQ::apply : cheapest transaction found");

    QueueApplyFullQueueDecision::EvictCheapest {
        dropped,
        end_effective_fee_level,
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use basics::base_uint::Uint256;
    use protocol::SeqProxy;

    use super::{
        QueueApplyFullQueueDecision, apply_queue_apply_full_queue_decision,
        evaluate_queue_apply_full_queue,
    };
    use crate::{
        FeeQueueEntry, FeeQueueKey, MaybeTxCore, QueueAdvanceCandidate, QueueViews, TxConsequences,
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
    fn full_queue_decision_bypasses_when_not_applicable() {
        let account = TxQAccount::<&str, u64>::new("acct");

        assert_eq!(
            evaluate_queue_apply_full_queue(
                true,
                true,
                &"acct",
                100,
                &[],
                &BTreeMap::from([("acct", account.clone())]),
                |queued| queued.payload,
            ),
            QueueApplyFullQueueDecision::Bypass
        );
        assert_eq!(
            evaluate_queue_apply_full_queue(
                false,
                false,
                &"acct",
                100,
                &[],
                &BTreeMap::from([("acct", account)]),
                |queued| queued.payload,
            ),
            QueueApplyFullQueueDecision::Bypass
        );
    }

    #[test]
    fn full_queue_decision_rejects_when_only_same_account_entries_exist() {
        let mut alice = TxQAccount::new("alice");
        alice.add(
            SeqProxy::sequence(5),
            MaybeTxCore::new(10_u64, TxConsequences::new(1, SeqProxy::sequence(5))),
        );

        let fee_order = vec![FeeQueueEntry::new(
            FeeQueueKey::new("alice", SeqProxy::sequence(5)),
            candidate(SeqProxy::sequence(5), 5, 10),
        )];

        let decision = evaluate_queue_apply_full_queue(
            false,
            true,
            &"alice",
            20,
            &fee_order,
            &BTreeMap::from([("alice", alice)]),
            |queued| queued.payload,
        );

        assert_eq!(decision, QueueApplyFullQueueDecision::RejectFullSameAccount);
    }

    #[test]
    fn full_queue_decision_uses_single_tail_fee_or_account_average() {
        let mut alice = TxQAccount::new("alice");
        alice.add(
            SeqProxy::sequence(5),
            MaybeTxCore::new(10_u64, TxConsequences::new(1, SeqProxy::sequence(5))),
        );

        let mut bob = TxQAccount::new("bob");
        bob.add(
            SeqProxy::sequence(8),
            MaybeTxCore::new(40_u64, TxConsequences::new(1, SeqProxy::sequence(8))),
        );
        bob.add(
            SeqProxy::sequence(9),
            MaybeTxCore::new(80_u64, TxConsequences::new(1, SeqProxy::sequence(9))),
        );

        let accounts = BTreeMap::from([("alice", alice), ("bob", bob)]);
        let fee_order = vec![
            FeeQueueEntry::new(
                FeeQueueKey::new("alice", SeqProxy::sequence(5)),
                candidate(SeqProxy::sequence(5), 5, 100),
            ),
            FeeQueueEntry::new(
                FeeQueueKey::new("bob", SeqProxy::sequence(8)),
                candidate(SeqProxy::sequence(8), 8, 80),
            ),
            FeeQueueEntry::new(
                FeeQueueKey::new("bob", SeqProxy::sequence(9)),
                candidate(SeqProxy::sequence(9), 9, 40),
            ),
        ];

        let evict = evaluate_queue_apply_full_queue(
            false,
            true,
            &"alice",
            70,
            &fee_order,
            &accounts,
            |queued| queued.payload,
        );
        assert_eq!(
            evict,
            QueueApplyFullQueueDecision::EvictCheapest {
                dropped: FeeQueueKey::new("bob", SeqProxy::sequence(9)),
                end_effective_fee_level: 60,
            }
        );

        let reject = evaluate_queue_apply_full_queue(
            false,
            true,
            &"alice",
            60,
            &fee_order,
            &accounts,
            |queued| queued.payload,
        );
        assert_eq!(
            reject,
            QueueApplyFullQueueDecision::RejectFullLowerFee {
                end_effective_fee_level: 60,
            }
        );
    }

    #[test]
    fn full_queue_decision_saturates_average_on_overflow() {
        let mut alice = TxQAccount::new("alice");
        alice.add(
            SeqProxy::sequence(5),
            MaybeTxCore::new(10_u64, TxConsequences::new(1, SeqProxy::sequence(5))),
        );

        let mut bob = TxQAccount::new("bob");
        bob.add(
            SeqProxy::sequence(8),
            MaybeTxCore::new(u64::MAX, TxConsequences::new(1, SeqProxy::sequence(8))),
        );
        bob.add(
            SeqProxy::sequence(9),
            MaybeTxCore::new(u64::MAX, TxConsequences::new(1, SeqProxy::sequence(9))),
        );

        let decision = evaluate_queue_apply_full_queue(
            false,
            true,
            &"alice",
            u64::MAX - 1,
            &[FeeQueueEntry::new(
                FeeQueueKey::new("bob", SeqProxy::sequence(9)),
                candidate(SeqProxy::sequence(9), 9, u64::MAX),
            )],
            &BTreeMap::from([("alice", alice), ("bob", bob)]),
            |queued| queued.payload,
        );

        assert_eq!(
            decision,
            QueueApplyFullQueueDecision::RejectFullLowerFee {
                end_effective_fee_level: u64::MAX,
            }
        );
    }

    #[test]
    fn full_queue_execution_evicts_the_selected_tail_from_both_views() {
        let mut alice = TxQAccount::new("alice");
        alice.add(
            SeqProxy::sequence(5),
            MaybeTxCore::new(10_u64, TxConsequences::new(1, SeqProxy::sequence(5))),
        );

        let mut bob = TxQAccount::new("bob");
        bob.add(
            SeqProxy::sequence(8),
            MaybeTxCore::new(80_u64, TxConsequences::new(1, SeqProxy::sequence(8))),
        );
        bob.add(
            SeqProxy::sequence(9),
            MaybeTxCore::new(40_u64, TxConsequences::new(1, SeqProxy::sequence(9))),
        );

        let mut views = QueueViews::new(
            BTreeMap::from([("alice", alice), ("bob", bob)]),
            vec![
                FeeQueueEntry::new(
                    FeeQueueKey::new("alice", SeqProxy::sequence(5)),
                    candidate(SeqProxy::sequence(5), 5, 100),
                ),
                FeeQueueEntry::new(
                    FeeQueueKey::new("bob", SeqProxy::sequence(8)),
                    candidate(SeqProxy::sequence(8), 8, 80),
                ),
                FeeQueueEntry::new(
                    FeeQueueKey::new("bob", SeqProxy::sequence(9)),
                    candidate(SeqProxy::sequence(9), 9, 40),
                ),
            ],
        );

        let result = apply_queue_apply_full_queue_decision(
            &mut views,
            QueueApplyFullQueueDecision::EvictCheapest {
                dropped: FeeQueueKey::new("bob", SeqProxy::sequence(9)),
                end_effective_fee_level: 60,
            },
        );

        assert_eq!(
            result,
            QueueApplyFullQueueDecision::EvictCheapest {
                dropped: FeeQueueKey::new("bob", SeqProxy::sequence(9)),
                end_effective_fee_level: 60,
            }
        );
        assert_eq!(
            views
                .fee_order
                .iter()
                .map(|entry| entry.key.clone())
                .collect::<Vec<_>>(),
            vec![
                FeeQueueKey::new("alice", SeqProxy::sequence(5)),
                FeeQueueKey::new("bob", SeqProxy::sequence(8)),
            ]
        );
        assert!(
            !views.accounts["bob"]
                .transactions
                .contains_key(&SeqProxy::sequence(9))
        );
        assert!(
            views.accounts["bob"]
                .transactions
                .contains_key(&SeqProxy::sequence(8))
        );
    }

    #[test]
    fn full_queue_execution_leaves_views_unchanged_for_bypass_and_reject() {
        let mut alice = TxQAccount::new("alice");
        alice.add(
            SeqProxy::sequence(5),
            MaybeTxCore::new(10_u64, TxConsequences::new(1, SeqProxy::sequence(5))),
        );

        let expected = vec![FeeQueueEntry::new(
            FeeQueueKey::new("alice", SeqProxy::sequence(5)),
            candidate(SeqProxy::sequence(5), 5, 10),
        )];

        let mut bypass_views =
            QueueViews::new(BTreeMap::from([("alice", alice.clone())]), expected.clone());
        let bypass = apply_queue_apply_full_queue_decision(
            &mut bypass_views,
            QueueApplyFullQueueDecision::Bypass,
        );
        assert_eq!(bypass, QueueApplyFullQueueDecision::Bypass);
        assert_eq!(bypass_views.fee_order, expected);
        assert_eq!(bypass_views.accounts["alice"], alice);

        let mut reject_views =
            QueueViews::new(BTreeMap::from([("alice", alice.clone())]), expected.clone());
        let reject = apply_queue_apply_full_queue_decision(
            &mut reject_views,
            QueueApplyFullQueueDecision::RejectFullSameAccount,
        );
        assert_eq!(reject, QueueApplyFullQueueDecision::RejectFullSameAccount);
        assert_eq!(reject_views.fee_order, expected);
        assert_eq!(reject_views.accounts["alice"], alice);
    }
}
