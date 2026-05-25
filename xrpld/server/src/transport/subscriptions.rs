use std::sync::Arc;

use protocol::JsonValue;
use tokio::sync::broadcast;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StreamKind {
    Ledger,
    Transactions,
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
            "transactions" | "rt_transactions" | "transactions_proposed" => {
                Some(Self::Transactions)
            }
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
            Self::Transactions => "transactions",
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
    pub payload: JsonValue,
}

#[derive(Debug, Clone)]
pub struct SubscriptionManager {
    ledger: Arc<broadcast::Sender<SubscriptionEvent>>,
    transactions: Arc<broadcast::Sender<SubscriptionEvent>>,
    book_changes: Arc<broadcast::Sender<SubscriptionEvent>>,
    server: Arc<broadcast::Sender<SubscriptionEvent>>,
    manifests: Arc<broadcast::Sender<SubscriptionEvent>>,
    validations: Arc<broadcast::Sender<SubscriptionEvent>>,
    peer_status: Arc<broadcast::Sender<SubscriptionEvent>>,
    consensus: Arc<broadcast::Sender<SubscriptionEvent>>,
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
            transactions: channel(capacity),
            book_changes: channel(capacity),
            server: channel(capacity),
            manifests: channel(capacity),
            validations: channel(capacity),
            peer_status: channel(capacity),
            consensus: channel(capacity),
        }
    }

    pub fn subscribe(&self, stream: StreamKind) -> broadcast::Receiver<SubscriptionEvent> {
        tracing::debug!(target: "server", stream = stream.as_name(), "Client subscribed");
        match stream {
            StreamKind::Ledger => self.ledger.subscribe(),
            StreamKind::Transactions => self.transactions.subscribe(),
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
            StreamKind::Transactions => self.transactions.send(event),
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
        self.publish(SubscriptionEvent { stream, payload })
    }

    pub fn unsubscribe(&self, stream: StreamKind) {
        tracing::debug!(target: "server", stream = stream.as_name(), "Client unsubscribed");
    }
}
