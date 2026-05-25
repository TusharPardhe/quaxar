//! Overlay traffic categorization and counters.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};

use protocol::JsonValue;

use crate::message::{ProtocolMessage, ProtocolMessageType, ProtocolPayload};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum TrafficCategory {
    Base,
    Cluster,
    Overlay,
    Manifests,
    Transaction,
    TransactionDuplicate,
    Proposal,
    ProposalUntrusted,
    ProposalDuplicate,
    Validation,
    ValidationUntrusted,
    ValidationDuplicate,
    ValidatorList,
    Squelch,
    SquelchSuppressed,
    SquelchIgnored,
    GetSet,
    ShareSet,
    LdTscGet,
    LdTscShare,
    LdTxnGet,
    LdTxnShare,
    LdAsnGet,
    LdAsnShare,
    LdGet,
    LdShare,
    GlTscShare,
    GlTscGet,
    GlTxnShare,
    GlTxnGet,
    GlAsnShare,
    GlAsnGet,
    GlShare,
    GlGet,
    ShareHashLedger,
    GetHashLedger,
    ShareHashTx,
    GetHashTx,
    ShareHashTxNode,
    GetHashTxNode,
    ShareHashAsNode,
    GetHashAsNode,
    ShareCasObject,
    GetCasObject,
    ShareFetchPack,
    GetFetchPack,
    GetTransactions,
    ShareHash,
    GetHash,
    ProofPathRequest,
    ProofPathResponse,
    ReplayDeltaRequest,
    ReplayDeltaResponse,
    HaveTransactions,
    RequestedTransactions,
    Total,
    Unknown,
}

impl TrafficCategory {
    pub fn categorize(message: &ProtocolMessage, inbound: bool) -> Self {
        match message.message_type {
            ProtocolMessageType::MtPing | ProtocolMessageType::MtStatusChange => Self::Base,
            ProtocolMessageType::MtManifests => Self::Manifests,
            ProtocolMessageType::MtEndpoints => Self::Overlay,
            ProtocolMessageType::MtTransaction => Self::Transaction,
            ProtocolMessageType::MtValidatorList
            | ProtocolMessageType::MtValidatorListCollection => Self::ValidatorList,
            ProtocolMessageType::MtValidation => Self::Validation,
            ProtocolMessageType::MtProposeLedger => Self::Proposal,
            ProtocolMessageType::MtProofPathReq => Self::ProofPathRequest,
            ProtocolMessageType::MtProofPathResponse => Self::ProofPathResponse,
            ProtocolMessageType::MtReplayDeltaReq => Self::ReplayDeltaRequest,
            ProtocolMessageType::MtReplayDeltaResponse => Self::ReplayDeltaResponse,
            ProtocolMessageType::MtHaveTransactions => Self::HaveTransactions,
            ProtocolMessageType::MtTransactions => Self::RequestedTransactions,
            ProtocolMessageType::MtSquelch => Self::Squelch,
            ProtocolMessageType::MtHaveSet => {
                if inbound {
                    Self::GetSet
                } else {
                    Self::ShareSet
                }
            }
            ProtocolMessageType::MtLedgerData => match &message.payload {
                ProtocolPayload::LedgerData(ledger_data) => match ledger_data.r#type {
                    3 => {
                        if inbound && ledger_data.request_cookie.is_none() {
                            Self::LdTscGet
                        } else {
                            Self::LdTscShare
                        }
                    }
                    1 => {
                        if inbound && ledger_data.request_cookie.is_none() {
                            Self::LdTxnGet
                        } else {
                            Self::LdTxnShare
                        }
                    }
                    2 => {
                        if inbound && ledger_data.request_cookie.is_none() {
                            Self::LdAsnGet
                        } else {
                            Self::LdAsnShare
                        }
                    }
                    _ => {
                        if inbound && ledger_data.request_cookie.is_none() {
                            Self::LdGet
                        } else {
                            Self::LdShare
                        }
                    }
                },
                _ => Self::Unknown,
            },
            ProtocolMessageType::MtGetLedger => match &message.payload {
                ProtocolPayload::GetLedger(get_ledger) => match get_ledger.itype {
                    3 => {
                        if inbound || get_ledger.request_cookie.is_some() {
                            Self::GlTscShare
                        } else {
                            Self::GlTscGet
                        }
                    }
                    1 => {
                        if inbound || get_ledger.request_cookie.is_some() {
                            Self::GlTxnShare
                        } else {
                            Self::GlTxnGet
                        }
                    }
                    2 => {
                        if inbound || get_ledger.request_cookie.is_some() {
                            Self::GlAsnShare
                        } else {
                            Self::GlAsnGet
                        }
                    }
                    _ => {
                        if inbound || get_ledger.request_cookie.is_some() {
                            Self::GlShare
                        } else {
                            Self::GlGet
                        }
                    }
                },
                _ => Self::Unknown,
            },
            ProtocolMessageType::MtGetObjects => match &message.payload {
                ProtocolPayload::GetObjects(get_objects) => match get_objects.r#type {
                    1 => {
                        if get_objects.query == inbound {
                            Self::ShareHashLedger
                        } else {
                            Self::GetHashLedger
                        }
                    }
                    2 => {
                        if get_objects.query == inbound {
                            Self::ShareHashTx
                        } else {
                            Self::GetHashTx
                        }
                    }
                    3 => {
                        if get_objects.query == inbound {
                            Self::ShareHashTxNode
                        } else {
                            Self::GetHashTxNode
                        }
                    }
                    4 => {
                        if get_objects.query == inbound {
                            Self::ShareHashAsNode
                        } else {
                            Self::GetHashAsNode
                        }
                    }
                    5 => {
                        if get_objects.query == inbound {
                            Self::ShareCasObject
                        } else {
                            Self::GetCasObject
                        }
                    }
                    6 => {
                        if get_objects.query == inbound {
                            Self::ShareFetchPack
                        } else {
                            Self::GetFetchPack
                        }
                    }
                    7 => Self::GetTransactions,
                    _ => {
                        if get_objects.query == inbound {
                            Self::ShareHash
                        } else {
                            Self::GetHash
                        }
                    }
                },
                _ => Self::Unknown,
            },
            ProtocolMessageType::MtCluster => Self::Cluster,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Base => "overhead",
            Self::Cluster => "overhead_cluster",
            Self::Overlay => "overhead_overlay",
            Self::Manifests => "overhead_manifest",
            Self::Transaction => "transactions",
            Self::TransactionDuplicate => "transactions_duplicate",
            Self::Proposal => "proposals",
            Self::ProposalUntrusted => "proposals_untrusted",
            Self::ProposalDuplicate => "proposals_duplicate",
            Self::Validation => "validations",
            Self::ValidationUntrusted => "validations_untrusted",
            Self::ValidationDuplicate => "validations_duplicate",
            Self::ValidatorList => "validator_lists",
            Self::Squelch => "squelch",
            Self::SquelchSuppressed => "squelch_suppressed",
            Self::SquelchIgnored => "squelch_ignored",
            Self::GetSet => "set_get",
            Self::ShareSet => "set_share",
            Self::LdTscGet => "ledger_data_Transaction_Set_candidate_get",
            Self::LdTscShare => "ledger_data_Transaction_Set_candidate_share",
            Self::LdTxnGet => "ledger_data_Transaction_Node_get",
            Self::LdTxnShare => "ledger_data_Transaction_Node_share",
            Self::LdAsnGet => "ledger_data_Account_State_Node_get",
            Self::LdAsnShare => "ledger_data_Account_State_Node_share",
            Self::LdGet => "ledger_data_get",
            Self::LdShare => "ledger_data_share",
            Self::GlTscShare => "ledger_Transaction_Set_candidate_share",
            Self::GlTscGet => "ledger_Transaction_Set_candidate_get",
            Self::GlTxnShare => "ledger_Transaction_node_share",
            Self::GlTxnGet => "ledger_Transaction_node_get",
            Self::GlAsnShare => "ledger_Account_State_node_share",
            Self::GlAsnGet => "ledger_Account_State_node_get",
            Self::GlShare => "ledger_share",
            Self::GlGet => "ledger_get",
            Self::ShareHashLedger => "share_hash_ledger",
            Self::GetHashLedger => "get_hash_ledger",
            Self::ShareHashTx => "share_hash_tx",
            Self::GetHashTx => "get_hash_tx",
            Self::ShareHashTxNode => "share_hash_txnode",
            Self::GetHashTxNode => "get_hash_txnode",
            Self::ShareHashAsNode => "share_hash_asnode",
            Self::GetHashAsNode => "get_hash_asnode",
            Self::ShareCasObject => "share_cas_object",
            Self::GetCasObject => "get_cas_object",
            Self::ShareFetchPack => "share_fetch_pack",
            Self::GetFetchPack => "get_fetch_pack",
            Self::GetTransactions => "get_transactions",
            Self::ShareHash => "share_hash",
            Self::GetHash => "get_hash",
            Self::ProofPathRequest => "proof_path_request",
            Self::ProofPathResponse => "proof_path_response",
            Self::ReplayDeltaRequest => "replay_delta_request",
            Self::ReplayDeltaResponse => "replay_delta_response",
            Self::HaveTransactions => "have_transactions",
            Self::RequestedTransactions => "requested_transactions",
            Self::Total => "total",
            Self::Unknown => "unknown",
        }
    }
}

#[derive(Debug)]
pub struct TrafficStats {
    pub name: &'static str,
    pub bytes_in: AtomicU64,
    pub bytes_out: AtomicU64,
    pub messages_in: AtomicU64,
    pub messages_out: AtomicU64,
}

impl TrafficStats {
    fn new(category: TrafficCategory) -> Self {
        Self {
            name: category.as_str(),
            bytes_in: AtomicU64::new(0),
            bytes_out: AtomicU64::new(0),
            messages_in: AtomicU64::new(0),
            messages_out: AtomicU64::new(0),
        }
    }
}

#[derive(Debug)]
pub struct TrafficCount {
    counts: BTreeMap<TrafficCategory, TrafficStats>,
}

impl Default for TrafficCount {
    fn default() -> Self {
        let counts = ALL_CATEGORIES
            .into_iter()
            .map(|category| (category, TrafficStats::new(category)))
            .collect();
        Self { counts }
    }
}

impl TrafficCount {
    pub fn add_count(&self, category: TrafficCategory, inbound: bool, bytes: u64) {
        let Some(stats) = self.counts.get(&category) else {
            return;
        };
        if inbound {
            stats.bytes_in.fetch_add(bytes, Ordering::Relaxed);
            stats.messages_in.fetch_add(1, Ordering::Relaxed);
        } else {
            stats.bytes_out.fetch_add(bytes, Ordering::Relaxed);
            stats.messages_out.fetch_add(1, Ordering::Relaxed);
        }
    }

    pub fn counts(&self) -> &BTreeMap<TrafficCategory, TrafficStats> {
        &self.counts
    }

    pub fn json(&self) -> JsonValue {
        JsonValue::Object(
            self.counts
                .iter()
                .map(|(category, stats)| {
                    (
                        category.as_str().to_owned(),
                        JsonValue::Object(BTreeMap::from([
                            (
                                "bytes_in".to_owned(),
                                JsonValue::Unsigned(stats.bytes_in.load(Ordering::Relaxed)),
                            ),
                            (
                                "bytes_out".to_owned(),
                                JsonValue::Unsigned(stats.bytes_out.load(Ordering::Relaxed)),
                            ),
                            (
                                "messages_in".to_owned(),
                                JsonValue::Unsigned(stats.messages_in.load(Ordering::Relaxed)),
                            ),
                            (
                                "messages_out".to_owned(),
                                JsonValue::Unsigned(stats.messages_out.load(Ordering::Relaxed)),
                            ),
                        ])),
                    )
                })
                .collect(),
        )
    }
}

const ALL_CATEGORIES: [TrafficCategory; 57] = [
    TrafficCategory::Base,
    TrafficCategory::Cluster,
    TrafficCategory::Overlay,
    TrafficCategory::Manifests,
    TrafficCategory::Transaction,
    TrafficCategory::TransactionDuplicate,
    TrafficCategory::Proposal,
    TrafficCategory::ProposalUntrusted,
    TrafficCategory::ProposalDuplicate,
    TrafficCategory::Validation,
    TrafficCategory::ValidationUntrusted,
    TrafficCategory::ValidationDuplicate,
    TrafficCategory::ValidatorList,
    TrafficCategory::Squelch,
    TrafficCategory::SquelchSuppressed,
    TrafficCategory::SquelchIgnored,
    TrafficCategory::GetSet,
    TrafficCategory::ShareSet,
    TrafficCategory::LdTscGet,
    TrafficCategory::LdTscShare,
    TrafficCategory::LdTxnGet,
    TrafficCategory::LdTxnShare,
    TrafficCategory::LdAsnGet,
    TrafficCategory::LdAsnShare,
    TrafficCategory::LdGet,
    TrafficCategory::LdShare,
    TrafficCategory::GlTscShare,
    TrafficCategory::GlTscGet,
    TrafficCategory::GlTxnShare,
    TrafficCategory::GlTxnGet,
    TrafficCategory::GlAsnShare,
    TrafficCategory::GlAsnGet,
    TrafficCategory::GlShare,
    TrafficCategory::GlGet,
    TrafficCategory::ShareHashLedger,
    TrafficCategory::GetHashLedger,
    TrafficCategory::ShareHashTx,
    TrafficCategory::GetHashTx,
    TrafficCategory::ShareHashTxNode,
    TrafficCategory::GetHashTxNode,
    TrafficCategory::ShareHashAsNode,
    TrafficCategory::GetHashAsNode,
    TrafficCategory::ShareCasObject,
    TrafficCategory::GetCasObject,
    TrafficCategory::ShareFetchPack,
    TrafficCategory::GetFetchPack,
    TrafficCategory::GetTransactions,
    TrafficCategory::ShareHash,
    TrafficCategory::GetHash,
    TrafficCategory::ProofPathRequest,
    TrafficCategory::ProofPathResponse,
    TrafficCategory::ReplayDeltaRequest,
    TrafficCategory::ReplayDeltaResponse,
    TrafficCategory::HaveTransactions,
    TrafficCategory::RequestedTransactions,
    TrafficCategory::Total,
    TrafficCategory::Unknown,
];

#[cfg(test)]
mod tests {
    use std::sync::atomic::Ordering;

    use super::{TrafficCategory, TrafficCount};
    use crate::message::{ProtocolMessage, ProtocolPayload, TmGetObjectByHash, TmPing};

    #[test]
    fn categorize_known_and_unknown_paths() {
        let ping = ProtocolMessage::new(ProtocolPayload::Ping(TmPing {
            r#type: 0,
            seq: None,
            ping_time: None,
            net_time: None,
        }));
        assert_eq!(
            TrafficCategory::categorize(&ping, false),
            TrafficCategory::Base
        );

        let get_objects = ProtocolMessage::new(ProtocolPayload::GetObjects(TmGetObjectByHash {
            r#type: 6,
            query: true,
            ledger_hash: None,
            fat: None,
            objects: Vec::new(),
        }));
        assert_eq!(
            TrafficCategory::categorize(&get_objects, true),
            TrafficCategory::ShareFetchPack
        );
        assert_eq!(
            TrafficCategory::categorize(&get_objects, false),
            TrafficCategory::GetFetchPack
        );
    }

    #[test]
    fn add_count_updates_directional_counters() {
        let traffic = TrafficCount::default();
        traffic.add_count(TrafficCategory::Total, true, 10);
        traffic.add_count(TrafficCategory::Total, false, 20);
        let stats = traffic
            .counts()
            .get(&TrafficCategory::Total)
            .expect("stats");
        assert_eq!(stats.bytes_in.load(Ordering::Relaxed), 10);
        assert_eq!(stats.bytes_out.load(Ordering::Relaxed), 20);
        assert_eq!(stats.messages_in.load(Ordering::Relaxed), 1);
        assert_eq!(stats.messages_out.load(Ordering::Relaxed), 1);
    }
}
