//! Deterministic `TxQ::MaybeTx` metadata/state carrier and ordering helper.
//!
//! This ports the queued-transaction metadata that later queue logic reasons
//! about, plus the `getTxDetails()` and fee/hash ordering behavior.

use std::cmp::Ordering;

use basics::base_uint::Uint256;
use protocol::{Rules, SeqProxy, Ter};

use crate::{ApplyFlags, ApplyResult, PreclaimResult, PreflightResult, TxConsequences};

pub type FeeLevel64 = u64;

pub const MAYBE_TX_RETRIES_ALLOWED: i32 = 10;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TxDetails<Tx, Account> {
    pub fee_level: FeeLevel64,
    pub last_valid: Option<u32>,
    pub consequences: TxConsequences,
    pub account: Account,
    pub seq_proxy: SeqProxy,
    pub tx: Tx,
    pub retries_remaining: i32,
    pub preflight_result: Ter,
    pub last_result: Option<Ter>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MaybeTx<Tx, Account, Journal, ParentBatchId> {
    pub fee_level: FeeLevel64,
    pub tx_id: Uint256,
    pub account: Account,
    pub last_valid: Option<u32>,
    pub seq_proxy: SeqProxy,
    pub retries_remaining: i32,
    pub flags: ApplyFlags,
    pub last_result: Option<Ter>,
    pub pf_result: PreflightResult<Tx, TxConsequences, Journal, ParentBatchId>,
}

impl<Tx, Account, Journal, ParentBatchId> MaybeTx<Tx, Account, Journal, ParentBatchId> {
    pub fn new(
        tx_id: Uint256,
        fee_level: FeeLevel64,
        account: Account,
        last_valid: Option<u32>,
        seq_proxy: SeqProxy,
        flags: ApplyFlags,
        pf_result: PreflightResult<Tx, TxConsequences, Journal, ParentBatchId>,
    ) -> Self {
        Self {
            fee_level,
            tx_id,
            account,
            last_valid,
            seq_proxy,
            retries_remaining: MAYBE_TX_RETRIES_ALLOWED,
            flags,
            last_result: None,
            pf_result,
        }
    }

    pub fn consequences(&self) -> &TxConsequences {
        &self.pf_result.consequences
    }

    pub fn set_last_result(&mut self, result: Ter) {
        self.last_result = Some(result);
    }

    pub fn decrement_retries(&mut self) -> i32 {
        self.retries_remaining -= 1;
        self.retries_remaining
    }

    pub fn record_apply_attempt_result(&mut self, result: &ApplyResult) {
        self.decrement_retries();
        self.set_last_result(result.ter);
    }

    pub fn needs_apply_reflight(&self, current_rules: &Rules) -> bool {
        self.pf_result.needs_reflight(current_rules) || self.pf_result.flags != self.flags
    }

    pub fn format_apply_reflight_debug_message(&self) -> String {
        format!(
            "Queued transaction {} rules or flags have changed. Flags from {} to {}",
            self.tx_id, self.pf_result.flags, self.flags
        )
    }
}

impl<Tx: Clone, Account, Journal: Clone, ParentBatchId>
    MaybeTx<Tx, Account, Journal, ParentBatchId>
{
    pub fn refresh_preflight_if_needed<RunPreflight, DebugFn>(
        &mut self,
        current_rules: &Rules,
        debug: DebugFn,
        run_preflight: RunPreflight,
    ) -> bool
    where
        RunPreflight: FnOnce(
            Tx,
            ApplyFlags,
            Journal,
        )
            -> PreflightResult<Tx, TxConsequences, Journal, ParentBatchId>,
        DebugFn: FnOnce(String),
    {
        if !self.needs_apply_reflight(current_rules) {
            return false;
        }

        debug(self.format_apply_reflight_debug_message());
        self.pf_result = run_preflight(
            self.pf_result.tx.clone(),
            self.flags,
            self.pf_result.journal.clone(),
        );
        true
    }
    pub fn apply_with_current_rules<RunPreflight, RunPreclaim, DoApply, DebugFn>(
        &mut self,
        current_rules: &Rules,
        debug: DebugFn,
        run_preflight: RunPreflight,
        run_preclaim: RunPreclaim,
        do_apply: DoApply,
    ) -> ApplyResult
    where
        RunPreflight: FnOnce(
            Tx,
            ApplyFlags,
            Journal,
        )
            -> PreflightResult<Tx, TxConsequences, Journal, ParentBatchId>,
        RunPreclaim: FnOnce(
            &PreflightResult<Tx, TxConsequences, Journal, ParentBatchId>,
        ) -> PreclaimResult<Tx, Journal, ParentBatchId>,
        DoApply: FnOnce(PreclaimResult<Tx, Journal, ParentBatchId>) -> ApplyResult,
        DebugFn: FnOnce(String),
    {
        self.refresh_preflight_if_needed(current_rules, debug, run_preflight);
        let preclaim_result = run_preclaim(&self.pf_result);
        do_apply(preclaim_result)
    }
}

impl<Tx: Clone, Account: Clone, Journal, ParentBatchId>
    MaybeTx<Tx, Account, Journal, ParentBatchId>
{
    pub fn get_tx_details(&self) -> TxDetails<Tx, Account> {
        TxDetails {
            fee_level: self.fee_level,
            last_valid: self.last_valid,
            consequences: *self.consequences(),
            account: self.account.clone(),
            seq_proxy: self.seq_proxy,
            tx: self.pf_result.tx.clone(),
            retries_remaining: self.retries_remaining,
            preflight_result: self.pf_result.ter,
            last_result: self.last_result,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct OrderCandidates {
    pub parent_hash_comp: Uint256,
}

impl OrderCandidates {
    pub const fn new(parent_hash_comp: Uint256) -> Self {
        Self { parent_hash_comp }
    }

    pub fn compares_by_fee_and_tx_id(
        &self,
        lhs_fee_level: FeeLevel64,
        lhs_tx_id: Uint256,
        rhs_fee_level: FeeLevel64,
        rhs_tx_id: Uint256,
    ) -> bool {
        if lhs_fee_level == rhs_fee_level {
            return (lhs_tx_id ^ self.parent_hash_comp) < (rhs_tx_id ^ self.parent_hash_comp);
        }
        lhs_fee_level > rhs_fee_level
    }

    pub fn compares_before<Tx, Account, Journal, ParentBatchId>(
        &self,
        lhs: &MaybeTx<Tx, Account, Journal, ParentBatchId>,
        rhs: &MaybeTx<Tx, Account, Journal, ParentBatchId>,
    ) -> bool {
        self.compares_by_fee_and_tx_id(lhs.fee_level, lhs.tx_id, rhs.fee_level, rhs.tx_id)
    }

    pub fn cmp<Tx, Account, Journal, ParentBatchId>(
        &self,
        lhs: &MaybeTx<Tx, Account, Journal, ParentBatchId>,
        rhs: &MaybeTx<Tx, Account, Journal, ParentBatchId>,
    ) -> Ordering {
        if self.compares_before(lhs, rhs) {
            Ordering::Less
        } else if self.compares_before(rhs, lhs) {
            Ordering::Greater
        } else {
            Ordering::Equal
        }
    }
}

#[cfg(test)]
mod tests {
    use std::cmp::Ordering;

    use basics::base_uint::Uint256;
    use protocol::{Rules, SeqProxy, Ter};

    use super::{MAYBE_TX_RETRIES_ALLOWED, MaybeTx, OrderCandidates};
    use crate::{ApplyFlags, ApplyResult, PreflightResult, TxConsequences};

    #[test]
    fn maybe_tx_constructor_sets_retry_and_metadata_defaults() {
        let pf_result = PreflightResult::new(
            "tx",
            None::<&str>,
            Rules::new(std::iter::empty()),
            TxConsequences::new(12, SeqProxy::sequence(5)),
            ApplyFlags::RETRY,
            "journal",
            Ter::TES_SUCCESS,
        );
        let queued = MaybeTx::new(
            Uint256::from_u64(9),
            44_u64,
            "acct",
            Some(99),
            SeqProxy::sequence(5),
            ApplyFlags::RETRY,
            pf_result,
        );

        assert_eq!(queued.retries_remaining, MAYBE_TX_RETRIES_ALLOWED);
        assert_eq!(queued.last_result, None);
        assert_eq!(queued.consequences().fee(), 12);
    }

    #[test]
    fn maybe_tx_details_and_failure_state_match_current_cpp_roles() {
        let pf_result = PreflightResult::new(
            "tx",
            Some("batch"),
            Rules::new(std::iter::empty()),
            TxConsequences::with_potential_spend(12, SeqProxy::sequence(5), 77),
            ApplyFlags::NONE,
            "journal",
            Ter::TER_PRE_SEQ,
        );
        let mut queued = MaybeTx::new(
            Uint256::from_u64(7),
            55_u64,
            "acct",
            Some(120),
            SeqProxy::sequence(5),
            ApplyFlags::NONE,
            pf_result,
        );
        queued.set_last_result(Ter::TER_RETRY);
        queued.decrement_retries();

        let details = queued.get_tx_details();
        assert_eq!(details.fee_level, 55);
        assert_eq!(details.last_valid, Some(120));
        assert_eq!(details.account, "acct");
        assert_eq!(details.seq_proxy, SeqProxy::sequence(5));
        assert_eq!(details.tx, "tx");
        assert_eq!(details.retries_remaining, MAYBE_TX_RETRIES_ALLOWED - 1);
        assert_eq!(details.preflight_result, Ter::TER_PRE_SEQ);
        assert_eq!(details.last_result, Some(Ter::TER_RETRY));
        assert_eq!(details.consequences.potential_spend(), 77);
    }

    #[test]
    fn record_apply_attempt_result_updates_retry_and_last_result_together() {
        let pf_result = PreflightResult::new(
            "tx",
            None::<&str>,
            Rules::new(std::iter::empty()),
            TxConsequences::new(1, SeqProxy::sequence(5)),
            ApplyFlags::NONE,
            "journal",
            Ter::TES_SUCCESS,
        );
        let mut queued = MaybeTx::new(
            Uint256::from_u64(7),
            55_u64,
            "acct",
            Some(120),
            SeqProxy::sequence(5),
            ApplyFlags::NONE,
            pf_result,
        );

        queued.record_apply_attempt_result(&ApplyResult::new(Ter::TEF_NO_TICKET, false, false));

        assert_eq!(queued.retries_remaining, MAYBE_TX_RETRIES_ALLOWED - 1);
        assert_eq!(queued.last_result, Some(Ter::TEF_NO_TICKET));
    }

    #[test]
    fn order_candidates_match_fee_then_xor_hash_rule() {
        let mk = |tx_id: u64, fee_level: u64| {
            MaybeTx::new(
                Uint256::from_u64(tx_id),
                fee_level,
                "acct",
                None,
                SeqProxy::sequence(1),
                ApplyFlags::NONE,
                PreflightResult::new(
                    "tx",
                    None::<&str>,
                    Rules::new(std::iter::empty()),
                    TxConsequences::new(1, SeqProxy::sequence(1)),
                    ApplyFlags::NONE,
                    "journal",
                    Ter::TES_SUCCESS,
                ),
            )
        };

        let high_fee = mk(9, 100);
        let low_fee = mk(8, 90);
        let tie_a = mk(3, 100);
        let tie_b = mk(5, 100);
        let order = OrderCandidates::new(Uint256::from_u64(6));

        assert!(order.compares_before(&high_fee, &low_fee));
        assert_eq!(order.cmp(&high_fee, &low_fee), Ordering::Less);
        assert_eq!((tie_a.tx_id ^ order.parent_hash_comp), Uint256::from_u64(5));
        assert_eq!((tie_b.tx_id ^ order.parent_hash_comp), Uint256::from_u64(3));
        assert!(!order.compares_before(&tie_a, &tie_b));
        assert!(order.compares_before(&tie_b, &tie_a));
    }

    #[test]
    fn order_candidates_can_compare_plain_fee_and_txid_inputs() {
        let order = OrderCandidates::new(Uint256::from_u64(6));

        assert!(order.compares_by_fee_and_tx_id(
            100,
            Uint256::from_u64(9),
            90,
            Uint256::from_u64(8)
        ));
        assert!(!order.compares_by_fee_and_tx_id(
            90,
            Uint256::from_u64(8),
            100,
            Uint256::from_u64(9)
        ));
        assert!(order.compares_by_fee_and_tx_id(
            100,
            Uint256::from_u64(5),
            100,
            Uint256::from_u64(3)
        ));
    }
}
