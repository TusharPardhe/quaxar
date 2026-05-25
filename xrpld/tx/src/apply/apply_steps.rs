//! Deterministic `tx/applySteps.h` carrier types and early-exit flow rules.
//!
//! This keeps the the reference implementation names for the preflight/preclaim result objects,
//! but narrows them to owned generic payloads so the deterministic gating
//! behavior stays explicit.

use protocol::{NotTec, Rules, Ter};

use crate::{ApplyFlags, ApplyResult, likely_to_claim_fee};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreflightResult<Tx, Consequences, Journal, ParentBatchId> {
    pub tx: Tx,
    pub parent_batch_id: Option<ParentBatchId>,
    pub rules: Rules,
    pub consequences: Consequences,
    pub flags: ApplyFlags,
    pub journal: Journal,
    pub ter: NotTec,
}

impl<Tx, Consequences, Journal, ParentBatchId>
    PreflightResult<Tx, Consequences, Journal, ParentBatchId>
{
    pub fn new(
        tx: Tx,
        parent_batch_id: Option<ParentBatchId>,
        rules: Rules,
        consequences: Consequences,
        flags: ApplyFlags,
        journal: Journal,
        ter: NotTec,
    ) -> Self {
        Self {
            tx,
            parent_batch_id,
            rules,
            consequences,
            flags,
            journal,
            ter,
        }
    }

    pub fn needs_reflight(&self, current_rules: &Rules) -> bool {
        self.rules != *current_rules
    }
}

impl<Tx: Clone, Consequences, Journal: Clone, ParentBatchId: Clone>
    PreflightResult<Tx, Consequences, Journal, ParentBatchId>
{
    pub fn to_preclaim(
        &self,
        ledger_seq: u32,
        ter: Ter,
    ) -> PreclaimResult<Tx, Journal, ParentBatchId> {
        PreclaimResult::new(
            ledger_seq,
            self.tx.clone(),
            self.parent_batch_id.clone(),
            self.flags,
            self.journal.clone(),
            ter,
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreclaimResult<Tx, Journal, ParentBatchId> {
    pub ledger_seq: u32,
    pub tx: Tx,
    pub parent_batch_id: Option<ParentBatchId>,
    pub flags: ApplyFlags,
    pub journal: Journal,
    pub ter: Ter,
    pub likely_to_claim_fee: bool,
}

impl<Tx, Journal, ParentBatchId> PreclaimResult<Tx, Journal, ParentBatchId> {
    pub fn new(
        ledger_seq: u32,
        tx: Tx,
        parent_batch_id: Option<ParentBatchId>,
        flags: ApplyFlags,
        journal: Journal,
        ter: Ter,
    ) -> Self {
        Self {
            ledger_seq,
            tx,
            parent_batch_id,
            flags,
            journal,
            ter,
            likely_to_claim_fee: likely_to_claim_fee(ter, flags),
        }
    }

    pub fn early_apply_result(&self, current_ledger_seq: u32) -> Option<ApplyResult> {
        if self.ledger_seq != current_ledger_seq {
            return Some(ApplyResult::new(Ter::TEF_EXCEPTION, false, false));
        }

        if !self.likely_to_claim_fee {
            return Some(ApplyResult::new(self.ter, false, false));
        }

        None
    }
}

#[cfg(test)]
mod tests {
    use super::{PreclaimResult, PreflightResult};
    use crate::ApplyFlags;
    use protocol::{Rules, Ter};

    #[test]
    fn preflight_result_detects_when_rules_changed() {
        let preset = protocol::feature_single_asset_vault();
        let old_rules = Rules::from_ledger(
            [preset],
            basics::base_uint::Uint256::from_array([0x11; 32]),
            std::iter::empty(),
        );
        let new_rules = Rules::from_ledger(
            [preset],
            basics::base_uint::Uint256::from_array([0x12; 32]),
            std::iter::empty(),
        );
        let result = PreflightResult::new(
            "tx",
            Some("batch"),
            old_rules,
            "normal",
            ApplyFlags::RETRY,
            "journal",
            Ter::TES_SUCCESS,
        );

        assert!(result.needs_reflight(&new_rules));
    }

    #[test]
    fn preflight_result_to_preclaim_copies_flow_fields() {
        let rules = Rules::new(std::iter::empty());
        let result = PreflightResult::new(
            "tx",
            Some("batch"),
            rules,
            "normal",
            ApplyFlags::FAIL_HARD,
            "journal",
            Ter::TES_SUCCESS,
        );

        let preclaim = result.to_preclaim(7, Ter::TEC_CLAIM);

        assert_eq!(preclaim.ledger_seq, 7);
        assert_eq!(preclaim.tx, "tx");
        assert_eq!(preclaim.parent_batch_id, Some("batch"));
        assert_eq!(preclaim.flags, ApplyFlags::FAIL_HARD);
        assert_eq!(preclaim.journal, "journal");
        assert_eq!(preclaim.ter, Ter::TEC_CLAIM);
        assert!(preclaim.likely_to_claim_fee);
    }

    #[test]
    fn preclaim_early_apply_result_matches_current_cpp_gate() {
        let success = PreclaimResult::new(
            9,
            "tx",
            None::<&str>,
            ApplyFlags::NONE,
            (),
            Ter::TES_SUCCESS,
        );
        assert_eq!(success.early_apply_result(9), None);

        let retry =
            PreclaimResult::new(9, "tx", None::<&str>, ApplyFlags::NONE, (), Ter::TER_RETRY);
        assert_eq!(
            retry.early_apply_result(9),
            Some(crate::ApplyResult::new(Ter::TER_RETRY, false, false))
        );

        let stale = PreclaimResult::new(
            9,
            "tx",
            None::<&str>,
            ApplyFlags::NONE,
            (),
            Ter::TES_SUCCESS,
        );
        assert_eq!(
            stale.early_apply_result(10),
            Some(crate::ApplyResult::new(Ter::TEF_EXCEPTION, false, false))
        );
    }
}
