//! Account-local queue context for `TxQ::apply(...)` immediately after the
//! queue lock is held.
//!
//! This preserves the the reference implementation order for:
//! 1. inspecting the relevant account queue tail from `lower_bound(acctSeqProx)`,
//! 2. rejecting lone/co-resident blocker combinations,
//! 3. rejecting a pre-existing queued blocker that is not being replaced,
//! 4. rejecting insufficient replacement fee,
//! 5. carrying the remaining account-local queue context forward.
//!

use protocol::SeqProxy;

use crate::{
    AccountQueueWindow, ApplyResult, BlockerQueueAdmission, FeeLevel64, FeeQueueKey, MaybeTx,
    QueueViews, QueuedBlockerAdmission, ReplacementFeeDecision, evaluate_blocker_queue_admission,
    evaluate_queued_blocker_admission, evaluate_replacement_fee, inspect_account_queue_window,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueueApplyAccountContext<Account> {
    pub window: AccountQueueWindow,
    pub first_relevant_retries_remaining: Option<i32>,
    pub replacement_decision: Option<ReplacementFeeDecision>,
    pub replaced: Option<FeeQueueKey<Account>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QueueApplyAccountStage<Account> {
    RejectBlockerAdmission(BlockerQueueAdmission),
    RejectQueuedBlocker(QueuedBlockerAdmission),
    RejectReplacementFee(ReplacementFeeDecision),
    Ready(QueueApplyAccountContext<Account>),
}

impl<Account> QueueApplyAccountStage<Account> {
    pub fn apply_result(&self) -> ApplyResult {
        match self {
            Self::RejectBlockerAdmission(admission) => {
                ApplyResult::new(admission.ter().expect("blocker rejection"), false, false)
            }
            Self::RejectQueuedBlocker(admission) => ApplyResult::new(
                admission.ter().expect("queued blocker rejection"),
                false,
                false,
            ),
            Self::RejectReplacementFee(decision) => {
                ApplyResult::new(decision.ter().expect("replacement rejection"), false, false)
            }
            Self::Ready(_) => ApplyResult::new(protocol::Ter::TES_SUCCESS, false, false),
        }
    }
}

pub fn run_queue_apply_account_stage<Account, Tx, Journal, ParentBatchId>(
    views: &QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
    account: &Account,
    account_seq_proxy: SeqProxy,
    tx_seq_proxy: SeqProxy,
    is_blocker: bool,
    fee_level_paid: FeeLevel64,
    retry_sequence_percent: u32,
) -> QueueApplyAccountStage<Account>
where
    Account: Clone + Ord + PartialEq,
{
    let tx_q_account = views.accounts.get(account);
    let window = inspect_account_queue_window(tx_q_account, account_seq_proxy, tx_seq_proxy);

    let blocker_admission = evaluate_blocker_queue_admission(
        is_blocker,
        window.relevant_tx_count,
        tx_seq_proxy,
        window.first_relevant_seq_proxy,
    );
    if blocker_admission != BlockerQueueAdmission::Allowed {
        return QueueApplyAccountStage::RejectBlockerAdmission(blocker_admission);
    }

    let queued_blocker_admission = evaluate_queued_blocker_admission(window, tx_seq_proxy);
    if queued_blocker_admission != QueuedBlockerAdmission::Allowed {
        return QueueApplyAccountStage::RejectQueuedBlocker(queued_blocker_admission);
    }

    let replacement_decision = tx_q_account
        .and_then(|queued_account| queued_account.transactions.get(&tx_seq_proxy))
        .map(|existing| {
            evaluate_replacement_fee(
                fee_level_paid,
                existing.payload.fee_level,
                retry_sequence_percent,
            )
        });
    if let Some(decision) = replacement_decision {
        if decision.ter().is_some() {
            return QueueApplyAccountStage::RejectReplacementFee(decision);
        }
    }

    let first_relevant_retries_remaining =
        window.first_relevant_seq_proxy.and_then(|first_relevant| {
            tx_q_account.and_then(|queued_account| {
                queued_account
                    .transactions
                    .get(&first_relevant)
                    .map(|queued| queued.payload.retries_remaining)
            })
        });
    let replaced = window
        .replaces_existing
        .then(|| FeeQueueKey::new(account.clone(), tx_seq_proxy));

    QueueApplyAccountStage::Ready(QueueApplyAccountContext {
        window,
        first_relevant_retries_remaining,
        replacement_decision,
        replaced,
    })
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use basics::base_uint::Uint256;
    use protocol::{Rules, SeqProxy, Ter};

    use super::{QueueApplyAccountContext, QueueApplyAccountStage, run_queue_apply_account_stage};
    use crate::{
        AccountQueueWindow, ApplyFlags, ApplyResult, BlockerQueueAdmission, MaybeTx, MaybeTxCore,
        PreflightResult, ReplacementFeeDecision, TxConsequences, TxConsequencesCategory,
        TxQAccount,
    };

    fn queued(
        account: &'static str,
        seq_proxy: SeqProxy,
        tx_id: u64,
        fee_level: u64,
        consequences: TxConsequences,
    ) -> MaybeTx<&'static str, &'static str, &'static str, &'static str> {
        MaybeTx::new(
            Uint256::from_u64(tx_id),
            fee_level,
            account,
            Some(200),
            seq_proxy,
            ApplyFlags::NONE,
            PreflightResult::new(
                "tx",
                None::<&str>,
                Rules::new(std::iter::empty()),
                consequences,
                ApplyFlags::NONE,
                "journal",
                Ter::TES_SUCCESS,
            ),
        )
    }

    #[test]
    fn account_stage_returns_ready_for_missing_queue_account() {
        let views = crate::QueueViews::<
            &'static str,
            MaybeTx<&'static str, &'static str, &'static str, &'static str>,
        >::new(BTreeMap::new(), vec![]);

        let stage = run_queue_apply_account_stage(
            &views,
            &"acct",
            SeqProxy::sequence(8),
            SeqProxy::sequence(8),
            false,
            100,
            25,
        );

        assert_eq!(
            stage,
            QueueApplyAccountStage::Ready(QueueApplyAccountContext {
                window: AccountQueueWindow::default(),
                first_relevant_retries_remaining: None,
                replacement_decision: None,
                replaced: None,
            })
        );
    }

    #[test]
    fn account_stage_rejects_blocker_before_replacement_or_retry_context() {
        let mut queued_account = TxQAccount::new("acct");
        queued_account.add(
            SeqProxy::sequence(5),
            MaybeTxCore::new(
                queued(
                    "acct",
                    SeqProxy::sequence(5),
                    5,
                    90,
                    TxConsequences::with_category(
                        1,
                        SeqProxy::sequence(5),
                        TxConsequencesCategory::Blocker,
                    ),
                ),
                TxConsequences::with_category(
                    1,
                    SeqProxy::sequence(5),
                    TxConsequencesCategory::Blocker,
                ),
            ),
        );
        let views = crate::QueueViews::new(BTreeMap::from([("acct", queued_account)]), vec![]);

        let stage = run_queue_apply_account_stage(
            &views,
            &"acct",
            SeqProxy::sequence(5),
            SeqProxy::sequence(6),
            true,
            100,
            25,
        );

        assert_eq!(
            stage,
            QueueApplyAccountStage::RejectBlockerAdmission(
                BlockerQueueAdmission::RejectsNonReplacementOfLoneEntry
            )
        );
        assert_eq!(
            stage.apply_result(),
            ApplyResult::new(Ter::TEL_CAN_NOT_QUEUE_BLOCKS, false, false)
        );
    }

    #[test]
    fn account_stage_rejects_insufficient_replacement_fee() {
        let mut queued_account = TxQAccount::new("acct");
        queued_account.add(
            SeqProxy::sequence(5),
            MaybeTxCore::new(
                queued(
                    "acct",
                    SeqProxy::sequence(5),
                    5,
                    100,
                    TxConsequences::new(1, SeqProxy::sequence(5)),
                ),
                TxConsequences::new(1, SeqProxy::sequence(5)),
            ),
        );
        let views = crate::QueueViews::new(BTreeMap::from([("acct", queued_account)]), vec![]);

        let stage = run_queue_apply_account_stage(
            &views,
            &"acct",
            SeqProxy::sequence(5),
            SeqProxy::sequence(5),
            false,
            125,
            25,
        );

        assert_eq!(
            stage,
            QueueApplyAccountStage::RejectReplacementFee(ReplacementFeeDecision::InsufficientFee {
                required_fee_level: 125,
            })
        );
        assert_eq!(
            stage.apply_result(),
            ApplyResult::new(Ter::TEL_CAN_NOT_QUEUE_FEE, false, false)
        );
    }

    #[test]
    fn account_stage_carries_ready_window_retry_and_replacement_context() {
        let mut queued_account = TxQAccount::new("acct");
        let mut first = queued(
            "acct",
            SeqProxy::sequence(7),
            7,
            100,
            TxConsequences::new(1, SeqProxy::sequence(7)),
        );
        first.retries_remaining = 8;
        queued_account.add(
            SeqProxy::sequence(7),
            MaybeTxCore::new(first, TxConsequences::new(1, SeqProxy::sequence(7))),
        );
        queued_account.add(
            SeqProxy::ticket(9),
            MaybeTxCore::new(
                queued(
                    "acct",
                    SeqProxy::ticket(9),
                    9,
                    80,
                    TxConsequences::new(1, SeqProxy::ticket(9)),
                ),
                TxConsequences::new(1, SeqProxy::ticket(9)),
            ),
        );
        let views = crate::QueueViews::new(BTreeMap::from([("acct", queued_account)]), vec![]);

        let stage = run_queue_apply_account_stage(
            &views,
            &"acct",
            SeqProxy::sequence(7),
            SeqProxy::sequence(7),
            false,
            126,
            25,
        );

        assert_eq!(
            stage,
            QueueApplyAccountStage::Ready(QueueApplyAccountContext {
                window: AccountQueueWindow {
                    account_is_in_queue: true,
                    first_relevant_seq_proxy: Some(SeqProxy::sequence(7)),
                    relevant_tx_count: 2,
                    replaces_existing: true,
                    front_is_blocker: false,
                },
                first_relevant_retries_remaining: Some(8),
                replacement_decision: Some(ReplacementFeeDecision::ReplaceAllowed {
                    required_fee_level: 125,
                }),
                replaced: Some(crate::FeeQueueKey::new("acct", SeqProxy::sequence(7))),
            })
        );
    }
}
