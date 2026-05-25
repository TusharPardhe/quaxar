//! Named stream subscription manager ported from the reference `InfoSub`/`SubscriptionManager`
//! surfaces.

use std::collections::BTreeMap;
use std::fmt;
use std::str::FromStr;
use std::sync::{Arc, RwLock};

use protocol::JsonValue;
use tokio::sync::broadcast;

use crate::commands::session::InfoSub;
use crate::status::{RpcErrorCode, Status};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SubscriptionStream {
    Server,
    Ledger,
    BookChanges,
    Manifests,
    Transactions,
    TransactionsProposed,
    Validations,
    PeerStatus,
    Consensus,
}

impl SubscriptionStream {
    pub const ALL: [Self; 9] = [
        Self::Server,
        Self::Ledger,
        Self::BookChanges,
        Self::Manifests,
        Self::Transactions,
        Self::TransactionsProposed,
        Self::Validations,
        Self::PeerStatus,
        Self::Consensus,
    ];

    pub const fn name(self) -> &'static str {
        match self {
            Self::Server => "server",
            Self::Ledger => "ledger",
            Self::BookChanges => "book_changes",
            Self::Manifests => "manifests",
            Self::Transactions => "transactions",
            Self::TransactionsProposed => "transactions_proposed",
            Self::Validations => "validations",
            Self::PeerStatus => "peer_status",
            Self::Consensus => "consensus",
        }
    }

    pub fn aliases(self) -> &'static [&'static str] {
        match self {
            Self::TransactionsProposed => &["transactions_proposed", "rt_transactions"],
            _ => &[],
        }
    }
}

impl fmt::Display for SubscriptionStream {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

impl FromStr for SubscriptionStream {
    type Err = Status;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        normalize_stream_name(s)
            .ok_or_else(|| Status::with_message(RpcErrorCode::StreamMalformed, "Stream malformed."))
    }
}

pub fn normalize_stream_name(name: &str) -> Option<SubscriptionStream> {
    match name {
        "server" => Some(SubscriptionStream::Server),
        "ledger" => Some(SubscriptionStream::Ledger),
        "book_changes" => Some(SubscriptionStream::BookChanges),
        "manifests" => Some(SubscriptionStream::Manifests),
        "transactions" => Some(SubscriptionStream::Transactions),
        "transactions_proposed" | "rt_transactions" => {
            Some(SubscriptionStream::TransactionsProposed)
        }
        "validations" => Some(SubscriptionStream::Validations),
        "peer_status" => Some(SubscriptionStream::PeerStatus),
        "consensus" => Some(SubscriptionStream::Consensus),
        _ => None,
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct SubscriptionMessage {
    pub stream: SubscriptionStream,
    pub payload: JsonValue,
}

#[derive(Debug, Clone)]
pub struct SubscriptionManager {
    streams: Arc<RwLock<BTreeMap<SubscriptionStream, broadcast::Sender<SubscriptionMessage>>>>,
    capacity: usize,
}

impl Default for SubscriptionManager {
    fn default() -> Self {
        Self::new()
    }
}

impl SubscriptionManager {
    pub fn new() -> Self {
        let manager = Self {
            streams: Arc::new(RwLock::new(BTreeMap::new())),
            capacity: 32,
        };
        for stream in SubscriptionStream::ALL {
            manager.register_stream(stream);
        }
        manager
    }

    pub fn with_capacity(capacity: usize) -> Self {
        let manager = Self {
            streams: Arc::new(RwLock::new(BTreeMap::new())),
            capacity,
        };
        for stream in SubscriptionStream::ALL {
            manager.register_stream(stream);
        }
        manager
    }

    pub fn register_stream(
        &self,
        stream: SubscriptionStream,
    ) -> broadcast::Sender<SubscriptionMessage> {
        let mut streams = self.streams.write().expect("subscription manager poisoned");
        streams
            .entry(stream)
            .or_insert_with(|| broadcast::channel(self.capacity).0)
            .clone()
    }

    pub fn known_streams(&self) -> Vec<SubscriptionStream> {
        self.streams
            .read()
            .expect("subscription manager poisoned")
            .keys()
            .copied()
            .collect()
    }

    pub fn subscribe(
        &self,
        session: &mut InfoSub,
        stream: SubscriptionStream,
    ) -> Result<broadcast::Receiver<SubscriptionMessage>, Status> {
        let sender = self
            .streams
            .read()
            .expect("subscription manager poisoned")
            .get(&stream)
            .cloned()
            .ok_or_else(|| {
                Status::with_message(RpcErrorCode::StreamMalformed, "Stream malformed.")
            })?;
        session.subscribe_stream(stream);
        Ok(sender.subscribe())
    }

    pub fn unsubscribe(&self, session: &mut InfoSub, stream: SubscriptionStream) -> bool {
        session.unsubscribe_stream(stream)
    }

    pub fn publish(&self, stream: SubscriptionStream, payload: JsonValue) -> Result<usize, Status> {
        let sender = self
            .streams
            .read()
            .expect("subscription manager poisoned")
            .get(&stream)
            .cloned()
            .ok_or_else(|| {
                Status::with_message(RpcErrorCode::StreamMalformed, "Stream malformed.")
            })?;
        sender
            .send(SubscriptionMessage { stream, payload })
            .map_err(|_| Status::new(RpcErrorCode::Internal))
    }

    pub fn publish_named(&self, stream: &str, payload: JsonValue) -> Result<usize, Status> {
        let stream = normalize_stream_name(stream).ok_or_else(|| {
            Status::with_message(RpcErrorCode::StreamMalformed, "Stream malformed.")
        })?;
        self.publish(stream, payload)
    }
}

pub fn parse_streams(value: &JsonValue) -> Result<Vec<SubscriptionStream>, Status> {
    let JsonValue::Array(items) = value else {
        return Err(Status::expected_field_error("streams", "array"));
    };

    items
        .iter()
        .map(|item| match item {
            JsonValue::String(text) => normalize_stream_name(text).ok_or_else(|| {
                Status::with_message(RpcErrorCode::StreamMalformed, "Stream malformed.")
            }),
            _ => Err(Status::with_message(
                RpcErrorCode::StreamMalformed,
                "Stream malformed.",
            )),
        })
        .collect()
}

pub fn streams_json(session: &InfoSub) -> JsonValue {
    JsonValue::Array(
        session
            .subscribed_streams()
            .map(|stream| JsonValue::String(stream.name().to_owned()))
            .collect(),
    )
}

pub fn is_stream_field_map(value: &JsonValue) -> Option<&BTreeMap<String, JsonValue>> {
    match value {
        JsonValue::Object(object) => Some(object),
        _ => None,
    }
}
