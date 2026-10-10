use std::collections::BTreeSet;
use std::sync::Arc;

use protocol::{JsonValue, MPTID};
use tokio::sync::broadcast;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StreamKind {
    Ledger,
    LedgerDelta,
    /// Validated/accepted transaction events only.
    Transactions,
    /// Real-time transaction events: proposed application events and the
    /// terminal validated publication for the same transaction.
    TransactionsProposed,
    BookChanges,
    Server,
    Manifests,
    Validations,
    PeerStatus,
    Consensus,
}

impl StreamKind {
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "ledger" => Some(Self::Ledger),
            "ledger_delta" => Some(Self::LedgerDelta),
            "transactions" => Some(Self::Transactions),
            "rt_transactions" | "transactions_proposed" => Some(Self::TransactionsProposed),
            "book_changes" => Some(Self::BookChanges),
            "server" => Some(Self::Server),
            "manifests" => Some(Self::Manifests),
            "validations" => Some(Self::Validations),
            "peer_status" => Some(Self::PeerStatus),
            "consensus" => Some(Self::Consensus),
            _ => None,
        }
    }

    pub fn as_name(self) -> &'static str {
        match self {
            Self::Ledger => "ledger",
            Self::LedgerDelta => "ledger_delta",
            Self::Transactions => "transactions",
            Self::TransactionsProposed => "transactions_proposed",
            Self::BookChanges => "book_changes",
            Self::Server => "server",
            Self::Manifests => "manifests",
            Self::Validations => "validations",
            Self::PeerStatus => "peer_status",
            Self::Consensus => "consensus",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct SubscriptionEvent {
    pub stream: StreamKind,
    pub payload: bytes::Bytes,
}

/// rippled `kMaxSubscriptionsPerConnection`: the per-connection bound on the
/// subscription sets counted by `InfoSub::totalSubscriptionCount`.
pub const MAX_SUBSCRIPTIONS_PER_CONNECTION: usize = 100_000;

/// rippled `exceedsSubscriptionCap`.
pub fn exceeds_subscription_cap(current: usize, additional: usize, cap: usize) -> bool {
    additional > cap || current > cap - additional
}

/// A validated transaction published to `mpt_issuances` subscribers, carrying
/// the issuance ids from `TxMeta::getAffectedMPTs` so each connection can
/// deliver it once when any of its subscribed issuances is affected.
#[derive(Debug, Clone, PartialEq)]
pub struct MptTransactionEvent {
    pub mpt_ids: Arc<BTreeSet<MPTID>>,
    pub payload: bytes::Bytes,
}

#[derive(Debug, Clone)]
pub struct SubscriptionManager {
    ledger: Arc<broadcast::Sender<SubscriptionEvent>>,
    ledger_delta: Arc<broadcast::Sender<SubscriptionEvent>>,
    transactions: Arc<broadcast::Sender<SubscriptionEvent>>,
    transactions_proposed: Arc<broadcast::Sender<SubscriptionEvent>>,
    book_changes: Arc<broadcast::Sender<SubscriptionEvent>>,
    server: Arc<broadcast::Sender<SubscriptionEvent>>,
    manifests: Arc<broadcast::Sender<SubscriptionEvent>>,
    validations: Arc<broadcast::Sender<SubscriptionEvent>>,
    peer_status: Arc<broadcast::Sender<SubscriptionEvent>>,
    consensus: Arc<broadcast::Sender<SubscriptionEvent>>,
    mpt_transactions: Arc<broadcast::Sender<MptTransactionEvent>>,
}

impl Default for SubscriptionManager {
    fn default() -> Self {
        Self::new(32)
    }
}

impl SubscriptionManager {
    pub fn new(capacity: usize) -> Self {
        fn channel(capacity: usize) -> Arc<broadcast::Sender<SubscriptionEvent>> {
            Arc::new(broadcast::channel(capacity).0)
        }

        Self {
            ledger: channel(capacity),
            ledger_delta: channel(capacity),
            transactions: channel(capacity),
            transactions_proposed: channel(capacity),
            book_changes: channel(capacity),
            server: channel(capacity),
            manifests: channel(capacity),
            validations: channel(capacity),
            peer_status: channel(capacity),
            consensus: channel(capacity),
            mpt_transactions: Arc::new(broadcast::channel(capacity).0),
        }
    }

    pub fn subscribe_mpt_transactions(&self) -> broadcast::Receiver<MptTransactionEvent> {
        self.mpt_transactions.subscribe()
    }

    /// rippled `NetworkOPsImp::pubMPTTransaction`: send the validated
    /// transaction JSON unchanged (type `transaction`, rippled #8539) to
    /// subscribers of any affected issuance. Returns early, before
    /// serializing, when nobody holds an MPT subscription.
    pub fn publish_mpt_transaction(&self, mpt_ids: BTreeSet<MPTID>, payload: &JsonValue) -> usize {
        if mpt_ids.is_empty() || self.mpt_transactions.receiver_count() == 0 {
            return 0;
        }
        let json = crate::json::from_protocol_json(payload);
        let text = sonic_rs::to_string(&json).unwrap_or_default();
        self.mpt_transactions
            .send(MptTransactionEvent {
                mpt_ids: Arc::new(mpt_ids),
                payload: bytes::Bytes::from(text),
            })
            .unwrap_or(0)
    }

    pub fn subscribe(&self, stream: StreamKind) -> broadcast::Receiver<SubscriptionEvent> {
        tracing::debug!(target: "server", stream = stream.as_name(), "Client subscribed");
        match stream {
            StreamKind::Ledger => self.ledger.subscribe(),
            StreamKind::LedgerDelta => self.ledger_delta.subscribe(),
            StreamKind::Transactions => self.transactions.subscribe(),
            StreamKind::TransactionsProposed => self.transactions_proposed.subscribe(),
            StreamKind::BookChanges => self.book_changes.subscribe(),
            StreamKind::Server => self.server.subscribe(),
            StreamKind::Manifests => self.manifests.subscribe(),
            StreamKind::Validations => self.validations.subscribe(),
            StreamKind::PeerStatus => self.peer_status.subscribe(),
            StreamKind::Consensus => self.consensus.subscribe(),
        }
    }

    pub fn publish(&self, event: SubscriptionEvent) -> usize {
        let stream = event.stream;
        let subscriber_count = match stream {
            StreamKind::Ledger => self.ledger.send(event),
            StreamKind::LedgerDelta => self.ledger_delta.send(event),
            StreamKind::Transactions => self.transactions.send(event),
            StreamKind::TransactionsProposed => self.transactions_proposed.send(event),
            StreamKind::BookChanges => self.book_changes.send(event),
            StreamKind::Server => self.server.send(event),
            StreamKind::Manifests => self.manifests.send(event),
            StreamKind::Validations => self.validations.send(event),
            StreamKind::PeerStatus => self.peer_status.send(event),
            StreamKind::Consensus => self.consensus.send(event),
        }
        .unwrap_or(0);
        tracing::debug!(target: "server", stream = stream.as_name(), subscriber_count, "Subscription event published");
        subscriber_count
    }

    pub fn publish_json(&self, stream: StreamKind, payload: JsonValue) -> usize {
        let json = crate::json::from_protocol_json(&payload);
        let text = sonic_rs::to_string(&json).unwrap_or_default();
        self.publish(SubscriptionEvent {
            stream,
            payload: bytes::Bytes::from(text),
        })
    }

    pub fn unsubscribe(&self, stream: StreamKind) {
        tracing::debug!(target: "server", stream = stream.as_name(), "Client unsubscribed");
    }
}

#[cfg(test)]
mod tests {
    use super::StreamKind;

    #[test]
    fn accepted_and_real_time_transaction_streams_are_distinct() {
        assert_eq!(
            StreamKind::from_name("transactions"),
            Some(StreamKind::Transactions)
        );
        assert_eq!(
            StreamKind::from_name("transactions_proposed"),
            Some(StreamKind::TransactionsProposed)
        );
        assert_eq!(
            StreamKind::from_name("rt_transactions"),
            Some(StreamKind::TransactionsProposed)
        );
        assert_ne!(
            StreamKind::Transactions,
            StreamKind::TransactionsProposed,
            "proposed events must not be multiplexed into accepted-only subscriptions"
        );
    }
}
