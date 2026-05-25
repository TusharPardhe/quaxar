//! Narrow the reference implementation compatibility surface.
//!
//! This lands the honest Rust slice that current protocol and tx seams can
//! support:
//! - `XChainCreateBridge::{preflight,preclaim,doApply}`,
//! - `BridgeModify::{getFlagsMask,preflight,preclaim,doApply}`,
//! - and the bridge-spec helper shape those methods depend on.
//!
//! The wider claim/commit/attestation flows remain separate because they still
//! depend on missing ledger/runtime/attestation substrate.

use protocol::{
    AccountID, FlagValue, Issue, NotTec, STAmount, Ter, XCHAIN_MODIFY_BRIDGE_FLAGS_MASK,
    genesis_account_id,
};

pub const XBRIDGE_MAX_ACCOUNT_CREATE_CLAIMS: usize = 128;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum XChainBridgeChainType {
    Locking,
    Issuing,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct XChainBridgeSpec {
    pub locking_chain_door: AccountID,
    pub locking_chain_issue: Issue,
    pub issuing_chain_door: AccountID,
    pub issuing_chain_issue: Issue,
}

impl XChainBridgeSpec {
    pub fn door(self, chain_type: XChainBridgeChainType) -> AccountID {
        match chain_type {
            XChainBridgeChainType::Locking => self.locking_chain_door,
            XChainBridgeChainType::Issuing => self.issuing_chain_door,
        }
    }

    pub fn issue(self, chain_type: XChainBridgeChainType) -> Issue {
        match chain_type {
            XChainBridgeChainType::Locking => self.locking_chain_issue,
            XChainBridgeChainType::Issuing => self.issuing_chain_issue,
        }
    }

    pub fn other_chain(chain_type: XChainBridgeChainType) -> XChainBridgeChainType {
        match chain_type {
            XChainBridgeChainType::Locking => XChainBridgeChainType::Issuing,
            XChainBridgeChainType::Issuing => XChainBridgeChainType::Locking,
        }
    }

    pub fn src_chain(was_locking_chain_send: bool) -> XChainBridgeChainType {
        if was_locking_chain_send {
            XChainBridgeChainType::Locking
        } else {
            XChainBridgeChainType::Issuing
        }
    }

    pub fn dst_chain(was_locking_chain_send: bool) -> XChainBridgeChainType {
        Self::other_chain(Self::src_chain(was_locking_chain_send))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct XChainCreateBridgePreflightFacts {
    pub account: AccountID,
    pub reward: STAmount,
    pub min_account_create: Option<STAmount>,
    pub bridge: XChainBridgeSpec,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct XChainCreateBridgePreclaimFacts {
    pub account: AccountID,
    pub bridge: XChainBridgeSpec,
    pub bridge_exists_on_locking: bool,
    pub bridge_exists_on_issuing: bool,
    pub source_issue_issuer_exists: bool,
    pub source_issue_allows_clawback: bool,
    pub account_exists: bool,
    pub reserve_sufficient: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct XChainCreateBridgeApplyFacts {
    pub account: AccountID,
    pub reward: STAmount,
    pub min_account_create: Option<STAmount>,
    pub bridge: XChainBridgeSpec,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct XChainCreateBridgeMutation {
    pub account: AccountID,
    pub reward: STAmount,
    pub min_account_create: Option<STAmount>,
    pub bridge: XChainBridgeSpec,
    pub chain_type: XChainBridgeChainType,
    pub xchain_claim_id: u64,
    pub xchain_account_create_count: u64,
    pub xchain_account_claim_count: u64,
    pub owner_node: u64,
}

pub trait XChainCreateBridgeApplySink {
    fn account_exists(&mut self) -> bool;
    fn insert_owner_dir(&mut self) -> Option<u64>;
    fn adjust_owner_count(&mut self, delta: i32);
    fn create_bridge(&mut self, mutation: XChainCreateBridgeMutation);
    fn update_account(&mut self);
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct XChainModifyBridgePreflightFacts {
    pub account: AccountID,
    pub reward: Option<STAmount>,
    pub min_account_create: Option<STAmount>,
    pub clear_account_create: bool,
    pub bridge: XChainBridgeSpec,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct XChainModifyBridgePreclaimFacts {
    pub bridge_exists: bool,
}

pub trait XChainModifyBridgeApplySink {
    fn account_exists(&mut self) -> bool;
    fn bridge_exists(&mut self) -> bool;
    fn set_reward(&mut self, reward: STAmount);
    fn set_min_account_create(&mut self, amount: STAmount);
    fn clear_min_account_create_if_present(&mut self);
    fn finish_bridge_update(&mut self);
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct XChainModifyBridgeApplyFacts {
    pub reward: Option<STAmount>,
    pub min_account_create: Option<STAmount>,
    pub clear_account_create: bool,
}

fn is_xrp_issue(issue: Issue) -> bool {
    issue.native()
}

fn xrp_root_account() -> AccountID {
    AccountID::from_slice(genesis_account_id().data())
        .expect("genesis account id width must match AccountID")
}

pub fn run_xchain_create_bridge_preflight(facts: XChainCreateBridgePreflightFacts) -> NotTec {
    let bridge = facts.bridge;

    if bridge.locking_chain_door == bridge.issuing_chain_door {
        return Ter::TEM_XCHAIN_EQUAL_DOOR_ACCOUNTS;
    }

    if bridge.locking_chain_door != facts.account && bridge.issuing_chain_door != facts.account {
        return Ter::TEM_XCHAIN_BRIDGE_NONDOOR_OWNER;
    }

    if is_xrp_issue(bridge.locking_chain_issue) != is_xrp_issue(bridge.issuing_chain_issue) {
        return Ter::TEM_XCHAIN_BRIDGE_BAD_ISSUES;
    }

    if !facts.reward.native() || facts.reward.signum() < 0 {
        return Ter::TEM_XCHAIN_BRIDGE_BAD_REWARD_AMOUNT;
    }

    if let Some(min_account_create) = facts.min_account_create
        && ((!min_account_create.native() || min_account_create.signum() <= 0)
            || !is_xrp_issue(bridge.locking_chain_issue)
            || !is_xrp_issue(bridge.issuing_chain_issue))
    {
        return Ter::TEM_XCHAIN_BRIDGE_BAD_MIN_ACCOUNT_CREATE_AMOUNT;
    }

    if is_xrp_issue(bridge.issuing_chain_issue) {
        if bridge.issuing_chain_door != xrp_root_account() {
            return Ter::TEM_XCHAIN_BRIDGE_BAD_ISSUES;
        }
    } else if bridge.issuing_chain_door != bridge.issuing_chain_issue.account {
        return Ter::TEM_XCHAIN_BRIDGE_BAD_ISSUES;
    }

    if bridge.locking_chain_door == bridge.locking_chain_issue.account {
        return Ter::TEM_XCHAIN_BRIDGE_BAD_ISSUES;
    }

    Ter::TES_SUCCESS
}

pub fn run_xchain_create_bridge_preclaim(facts: XChainCreateBridgePreclaimFacts) -> Ter {
    if facts.bridge_exists_on_issuing || facts.bridge_exists_on_locking {
        return Ter::TEC_DUPLICATE;
    }

    let chain_type = XChainBridgeSpec::src_chain(facts.account == facts.bridge.locking_chain_door);
    if !is_xrp_issue(facts.bridge.issue(chain_type)) {
        if !facts.source_issue_issuer_exists {
            return Ter::TEC_NO_ISSUER;
        }

        if facts.source_issue_allows_clawback {
            return Ter::TEC_NO_PERMISSION;
        }
    }

    if !facts.account_exists {
        return Ter::TER_NO_ACCOUNT;
    }

    if !facts.reserve_sufficient {
        return Ter::TEC_INSUFFICIENT_RESERVE;
    }

    Ter::TES_SUCCESS
}

pub fn run_xchain_create_bridge_do_apply<S: XChainCreateBridgeApplySink>(
    facts: XChainCreateBridgeApplyFacts,
    sink: &mut S,
) -> Ter {
    if !sink.account_exists() {
        return Ter::TEC_INTERNAL;
    }

    let chain_type = XChainBridgeSpec::src_chain(facts.account == facts.bridge.locking_chain_door);
    let Some(owner_node) = sink.insert_owner_dir() else {
        return Ter::TEC_DIR_FULL;
    };

    sink.adjust_owner_count(1);
    sink.create_bridge(XChainCreateBridgeMutation {
        account: facts.account,
        reward: facts.reward,
        min_account_create: facts.min_account_create,
        bridge: facts.bridge,
        chain_type,
        xchain_claim_id: 0,
        xchain_account_create_count: 0,
        xchain_account_claim_count: 0,
        owner_node,
    });
    sink.update_account();

    Ter::TES_SUCCESS
}

pub const fn run_xchain_modify_bridge_get_flags_mask() -> FlagValue {
    XCHAIN_MODIFY_BRIDGE_FLAGS_MASK
}

pub fn run_xchain_modify_bridge_preflight(facts: XChainModifyBridgePreflightFacts) -> NotTec {
    let bridge = facts.bridge;

    if facts.reward.is_none() && facts.min_account_create.is_none() && !facts.clear_account_create {
        return Ter::TEM_MALFORMED;
    }

    if facts.min_account_create.is_some() && facts.clear_account_create {
        return Ter::TEM_MALFORMED;
    }

    if bridge.locking_chain_door != facts.account && bridge.issuing_chain_door != facts.account {
        return Ter::TEM_XCHAIN_BRIDGE_NONDOOR_OWNER;
    }

    if facts
        .reward
        .is_some_and(|reward| !reward.native() || reward.signum() < 0)
    {
        return Ter::TEM_XCHAIN_BRIDGE_BAD_REWARD_AMOUNT;
    }

    if let Some(min_account_create) = facts.min_account_create
        && ((!min_account_create.native() || min_account_create.signum() <= 0)
            || !is_xrp_issue(bridge.locking_chain_issue)
            || !is_xrp_issue(bridge.issuing_chain_issue))
    {
        return Ter::TEM_XCHAIN_BRIDGE_BAD_MIN_ACCOUNT_CREATE_AMOUNT;
    }

    Ter::TES_SUCCESS
}

pub fn run_xchain_modify_bridge_preclaim(facts: XChainModifyBridgePreclaimFacts) -> Ter {
    if !facts.bridge_exists {
        return Ter::TEC_NO_ENTRY;
    }

    Ter::TES_SUCCESS
}

pub fn run_xchain_modify_bridge_do_apply<S: XChainModifyBridgeApplySink>(
    facts: XChainModifyBridgeApplyFacts,
    sink: &mut S,
) -> Ter {
    if !sink.account_exists() {
        return Ter::TEC_INTERNAL;
    }

    if !sink.bridge_exists() {
        return Ter::TEC_INTERNAL;
    }

    if let Some(reward) = facts.reward {
        sink.set_reward(reward);
    }
    if let Some(min_account_create) = facts.min_account_create {
        sink.set_min_account_create(min_account_create);
    }
    if facts.clear_account_create {
        sink.clear_min_account_create_if_present();
    }
    sink.finish_bridge_update();

    Ter::TES_SUCCESS
}
