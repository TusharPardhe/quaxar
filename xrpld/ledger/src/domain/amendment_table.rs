//! Rust port of `xrpl::AmendmentTable` from `xrpl/ledger/AmendmentTable.h`.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use basics::base_uint::Uint256;
use basics::chrono::NetClockTimePoint;
use protocol::{PublicKey, Rules, STValidation};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum VoteBehavior {
    Veto,
    UnVeto,
    Enable,
    Supported,
}

pub struct FeatureInfo {
    pub name: String,
    pub feature: Uint256,
    pub vote: VoteBehavior,
}

pub trait AmendmentTable: Send + Sync {
    fn find(&self, name: &str) -> Option<Uint256>;
    fn veto(&self, amendment: Uint256) -> bool;
    fn un_veto(&self, amendment: Uint256) -> bool;
    fn enable(&self, amendment: Uint256) -> bool;
    fn is_enabled(&self, amendment: Uint256) -> bool;
    fn is_supported(&self, amendment: Uint256) -> bool;
    fn has_unsupported_enabled(&self) -> bool;
    fn first_unsupported_expected(&self) -> Option<NetClockTimePoint>;
    fn need_validated_ledger(&self, seq: u32) -> bool;
    fn do_validated_ledger(
        &self,
        ledger_seq: u32,
        enabled: &BTreeSet<Uint256>,
        majority: &BTreeMap<Uint256, u32>,
    );
    fn trust_changed(&self, all_trusted: &BTreeSet<PublicKey>);
    fn do_voting(
        &self,
        rules: &Rules,
        close_time: NetClockTimePoint,
        enabled_amendments: &BTreeSet<Uint256>,
        majority_amendments: &BTreeMap<Uint256, u32>,
        val_set: &[Arc<STValidation>],
    ) -> BTreeMap<Uint256, u32>;
    fn do_validation(&self, enabled: &BTreeSet<Uint256>) -> Vec<Uint256>;
    fn get_desired(&self) -> Vec<Uint256>;
}
