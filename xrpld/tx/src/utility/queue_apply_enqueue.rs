//! Final enqueue mutation block for `TxQ::apply(...)`.
//!
//! This helper ports the deterministic "remove replacement, create account
//! when needed, strip `tapRETRY`, add to account order, and insert into fee
//! order" behavior.

use std::collections::btree_map::Entry;

use basics::base_uint::Uint256;
use protocol::{SeqProxy, Ter};

use crate::{
    ApplyFlags, FeeLevel64, FeeQueueEntry, FeeQueueKey, MaybeTx, MaybeTxCore, OrderCandidates,
    PreflightResult, QueueAdvanceCandidate, QueueViews, TxConsequences, TxQAccount,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueueApplyEnqueueResult<Account> {
    pub queued: FeeQueueKey<Account>,
    pub removed_replacement: Option<FeeQueueKey<Account>>,
    pub account_created: bool,
    pub stored_flags: ApplyFlags,
}

impl<Account> QueueApplyEnqueueResult<Account> {
    pub const fn ter(&self) -> Ter {
        Ter::TER_QUEUED
    }
}

pub fn enqueue_queue_apply_candidate<Account, Tx, Journal, ParentBatchId>(
    views: &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
    replaced: Option<FeeQueueKey<Account>>,
    account: Account,
    tx_id: Uint256,
    last_valid: Option<u32>,
    seq_proxy: SeqProxy,
    fee_level: FeeLevel64,
    flags: ApplyFlags,
    pf_result: PreflightResult<Tx, TxConsequences, Journal, ParentBatchId>,
    order: &OrderCandidates,
) -> QueueApplyEnqueueResult<Account>
where
    Account: Clone + Ord + PartialEq,
{
    let removed_replacement = views
        .remove_replaced_candidate(replaced, &account, seq_proxy)
        .removed;

    let account_created = match views.accounts.entry(account.clone()) {
        Entry::Occupied(_) => false,
        Entry::Vacant(slot) => {
            slot.insert(TxQAccount::new(account.clone()));
            true
        }
    };

    let stored_flags = flags & !ApplyFlags::RETRY;
    let queued = MaybeTx::new(
        tx_id,
        fee_level,
        account.clone(),
        last_valid,
        seq_proxy,
        stored_flags,
        pf_result,
    );
    let consequences = *queued.consequences();

    views
        .accounts
        .get_mut(&account)
        .expect("xrpl::TxQ::apply : account created")
        .add(seq_proxy, MaybeTxCore::new(queued, consequences));

    let queued_key = FeeQueueKey::new(account, seq_proxy);
    views.insert_fee_entry(
        FeeQueueEntry::new(
            queued_key.clone(),
            QueueAdvanceCandidate {
                fee_level,
                tx_id,
                seq_proxy,
            },
        ),
        order,
    );

    let queue_size = views.fee_order.len();
    tracing::debug!(target: "tx", tx_type = "queued", hash = %tx_id, "Transaction queued");
    tracing::info!(target: "tx", queue_size, "Transaction queue size");

    QueueApplyEnqueueResult {
        queued: queued_key,
        removed_replacement,
        account_created,
        stored_flags,
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use basics::base_uint::Uint256;
    use protocol::{Rules, SeqProxy, Ter};

    use super::{QueueApplyEnqueueResult, enqueue_queue_apply_candidate};
    use crate::{
        ApplyFlags, FeeQueueEntry, FeeQueueKey, MaybeTx, MaybeTxCore, OrderCandidates,
        PreflightResult, QueueAdvanceCandidate, QueueViews, TxConsequences, TxQAccount,
    };

    fn queued(
        account: &'static str,
        seq_proxy: SeqProxy,
        tx_id: u64,
        fee_level: u64,
        flags: ApplyFlags,
    ) -> MaybeTx<&'static str, &'static str, &'static str, &'static str> {
        MaybeTx::new(
            Uint256::from_u64(tx_id),
            fee_level,
            account,
            Some(200),
            seq_proxy,
            flags,
            PreflightResult::new(
                "tx",
                None,
                Rules::new(std::iter::empty()),
                TxConsequences::new(1, seq_proxy),
                flags,
                "journal",
                Ter::TES_SUCCESS,
            ),
        )
    }

    #[test]
    fn enqueue_replaces_existing_entry_strips_retry_and_returns_ter_queued() {
        let existing = queued(
            "a",
            SeqProxy::sequence(5),
            5,
            90,
            ApplyFlags::FAIL_HARD | ApplyFlags::RETRY,
        );
        let mut account_a = TxQAccount::new("a");
        account_a.add(
            SeqProxy::sequence(5),
            MaybeTxCore::new(existing, TxConsequences::new(1, SeqProxy::sequence(5))),
        );

        let other = queued("b", SeqProxy::sequence(7), 7, 100, ApplyFlags::NONE);
        let mut account_b = TxQAccount::new("b");
        account_b.add(
            SeqProxy::sequence(7),
            MaybeTxCore::new(other, TxConsequences::new(1, SeqProxy::sequence(7))),
        );

        let mut views = QueueViews::new(
            BTreeMap::from([("a", account_a), ("b", account_b)]),
            vec![
                FeeQueueEntry::new(
                    FeeQueueKey::new("b", SeqProxy::sequence(7)),
                    QueueAdvanceCandidate {
                        fee_level: 100,
                        tx_id: Uint256::from_u64(7),
                        seq_proxy: SeqProxy::sequence(7),
                    },
                ),
                FeeQueueEntry::new(
                    FeeQueueKey::new("a", SeqProxy::sequence(5)),
                    QueueAdvanceCandidate {
                        fee_level: 90,
                        tx_id: Uint256::from_u64(5),
                        seq_proxy: SeqProxy::sequence(5),
                    },
                ),
            ],
        );

        let result = enqueue_queue_apply_candidate(
            &mut views,
            Some(FeeQueueKey::new("a", SeqProxy::sequence(5))),
            "a",
            Uint256::from_u64(9),
            Some(250),
            SeqProxy::sequence(5),
            110,
            ApplyFlags::FAIL_HARD | ApplyFlags::RETRY,
            PreflightResult::new(
                "replacement",
                None,
                Rules::new(std::iter::empty()),
                TxConsequences::new(2, SeqProxy::sequence(5)),
                ApplyFlags::FAIL_HARD | ApplyFlags::RETRY,
                "journal",
                Ter::TES_SUCCESS,
            ),
            &OrderCandidates::new(Uint256::from_u64(0)),
        );

        assert_eq!(
            result,
            QueueApplyEnqueueResult {
                queued: FeeQueueKey::new("a", SeqProxy::sequence(5)),
                removed_replacement: Some(FeeQueueKey::new("a", SeqProxy::sequence(5))),
                account_created: false,
                stored_flags: ApplyFlags::FAIL_HARD,
            }
        );
        assert_eq!(result.ter(), Ter::TER_QUEUED);
        assert_eq!(
            views
                .fee_order
                .iter()
                .map(|entry| entry.key.clone())
                .collect::<Vec<_>>(),
            vec![
                FeeQueueKey::new("a", SeqProxy::sequence(5)),
                FeeQueueKey::new("b", SeqProxy::sequence(7)),
            ]
        );

        let queued = &views.accounts["a"].transactions[&SeqProxy::sequence(5)].payload;
        assert_eq!(queued.flags, ApplyFlags::FAIL_HARD);
        assert_eq!(queued.tx_id, Uint256::from_u64(9));
        assert_eq!(queued.last_valid, Some(250));
    }

    #[test]
    fn enqueue_creates_new_account_and_respects_fee_order_tie_breaker() {
        let existing = queued("b", SeqProxy::sequence(7), 2, 100, ApplyFlags::NONE);
        let mut account_b = TxQAccount::new("b");
        account_b.add(
            SeqProxy::sequence(7),
            MaybeTxCore::new(existing, TxConsequences::new(1, SeqProxy::sequence(7))),
        );

        let mut views = QueueViews::new(
            BTreeMap::from([("b", account_b)]),
            vec![FeeQueueEntry::new(
                FeeQueueKey::new("b", SeqProxy::sequence(7)),
                QueueAdvanceCandidate {
                    fee_level: 100,
                    tx_id: Uint256::from_u64(2),
                    seq_proxy: SeqProxy::sequence(7),
                },
            )],
        );

        let result = enqueue_queue_apply_candidate(
            &mut views,
            None,
            "a",
            Uint256::from_u64(1),
            None,
            SeqProxy::sequence(5),
            100,
            ApplyFlags::NONE,
            PreflightResult::new(
                "fresh",
                None,
                Rules::new(std::iter::empty()),
                TxConsequences::new(1, SeqProxy::sequence(5)),
                ApplyFlags::NONE,
                "journal",
                Ter::TES_SUCCESS,
            ),
            &OrderCandidates::new(Uint256::from_u64(0)),
        );

        assert_eq!(
            result,
            QueueApplyEnqueueResult {
                queued: FeeQueueKey::new("a", SeqProxy::sequence(5)),
                removed_replacement: None,
                account_created: true,
                stored_flags: ApplyFlags::NONE,
            }
        );
        assert_eq!(views.accounts["a"].get_txn_count(), 1);
        assert_eq!(
            views
                .fee_order
                .iter()
                .map(|entry| entry.key.clone())
                .collect::<Vec<_>>(),
            vec![
                FeeQueueKey::new("a", SeqProxy::sequence(5)),
                FeeQueueKey::new("b", SeqProxy::sequence(7)),
            ]
        );
    }
}
