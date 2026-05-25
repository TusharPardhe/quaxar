//! Current overlay protobuf message, framing, and decode surface.

use std::fmt;
use std::sync::{Arc, OnceLock};

use prost::Message as ProstMessage;
use protocol::PublicKey;

use crate::compression::{
    Compressed, CompressionAlgorithm, HEADER_BYTES, HEADER_BYTES_COMPRESSED, compress, decompress,
};
use crate::traffic_count::TrafficCategory;

pub const MAXIMUM_MESSAGE_SIZE: usize = 64 * 1024 * 1024;

pub mod wire {
    include!(concat!(env!("OUT_DIR"), "/protocol.rs"));
}

pub use wire::{
    MessageType as ProtocolMessageType, TmCluster, TmEndpoints, TmGetLedger, TmGetObjectByHash,
    TmHaveTransactionSet, TmHaveTransactions, TmLedgerData, TmManifests, TmPing,
    TmProofPathRequest, TmProofPathResponse, TmProposeSet, TmReplayDeltaRequest,
    TmReplayDeltaResponse, TmSquelch, TmStatusChange, TmTransaction, TmTransactions, TmValidation,
    TmValidatorList, TmValidatorListCollection,
};

#[derive(Debug, Clone, PartialEq)]
pub enum ProtocolPayload {
    Manifests(TmManifests),
    Ping(TmPing),
    Cluster(TmCluster),
    Endpoints(TmEndpoints),
    Transaction(TmTransaction),
    GetLedger(TmGetLedger),
    LedgerData(TmLedgerData),
    ProposeLedger(TmProposeSet),
    StatusChange(TmStatusChange),
    HaveSet(TmHaveTransactionSet),
    Validation(TmValidation),
    ValidatorList(TmValidatorList),
    ValidatorListCollection(TmValidatorListCollection),
    GetObjects(TmGetObjectByHash),
    HaveTransactions(TmHaveTransactions),
    Transactions(TmTransactions),
    Squelch(TmSquelch),
    ProofPathRequest(TmProofPathRequest),
    ProofPathResponse(TmProofPathResponse),
    ReplayDeltaRequest(TmReplayDeltaRequest),
    ReplayDeltaResponse(TmReplayDeltaResponse),
}

impl ProtocolPayload {
    pub fn message_type(&self) -> ProtocolMessageType {
        match self {
            Self::Manifests(_) => ProtocolMessageType::MtManifests,
            Self::Ping(_) => ProtocolMessageType::MtPing,
            Self::Cluster(_) => ProtocolMessageType::MtCluster,
            Self::Endpoints(_) => ProtocolMessageType::MtEndpoints,
            Self::Transaction(_) => ProtocolMessageType::MtTransaction,
            Self::GetLedger(_) => ProtocolMessageType::MtGetLedger,
            Self::LedgerData(_) => ProtocolMessageType::MtLedgerData,
            Self::ProposeLedger(_) => ProtocolMessageType::MtProposeLedger,
            Self::StatusChange(_) => ProtocolMessageType::MtStatusChange,
            Self::HaveSet(_) => ProtocolMessageType::MtHaveSet,
            Self::Validation(_) => ProtocolMessageType::MtValidation,
            Self::ValidatorList(_) => ProtocolMessageType::MtValidatorList,
            Self::ValidatorListCollection(_) => ProtocolMessageType::MtValidatorListCollection,
            Self::GetObjects(_) => ProtocolMessageType::MtGetObjects,
            Self::HaveTransactions(_) => ProtocolMessageType::MtHaveTransactions,
            Self::Transactions(_) => ProtocolMessageType::MtTransactions,
            Self::Squelch(_) => ProtocolMessageType::MtSquelch,
            Self::ProofPathRequest(_) => ProtocolMessageType::MtProofPathReq,
            Self::ProofPathResponse(_) => ProtocolMessageType::MtProofPathResponse,
            Self::ReplayDeltaRequest(_) => ProtocolMessageType::MtReplayDeltaReq,
            Self::ReplayDeltaResponse(_) => ProtocolMessageType::MtReplayDeltaResponse,
        }
    }

    pub fn encode_to_vec(&self) -> Vec<u8> {
        match self {
            Self::Manifests(message) => message.encode_to_vec(),
            Self::Ping(message) => message.encode_to_vec(),
            Self::Cluster(message) => message.encode_to_vec(),
            Self::Endpoints(message) => message.encode_to_vec(),
            Self::Transaction(message) => message.encode_to_vec(),
            Self::GetLedger(message) => message.encode_to_vec(),
            Self::LedgerData(message) => message.encode_to_vec(),
            Self::ProposeLedger(message) => message.encode_to_vec(),
            Self::StatusChange(message) => message.encode_to_vec(),
            Self::HaveSet(message) => message.encode_to_vec(),
            Self::Validation(message) => message.encode_to_vec(),
            Self::ValidatorList(message) => message.encode_to_vec(),
            Self::ValidatorListCollection(message) => message.encode_to_vec(),
            Self::GetObjects(message) => message.encode_to_vec(),
            Self::HaveTransactions(message) => message.encode_to_vec(),
            Self::Transactions(message) => message.encode_to_vec(),
            Self::Squelch(message) => message.encode_to_vec(),
            Self::ProofPathRequest(message) => message.encode_to_vec(),
            Self::ProofPathResponse(message) => message.encode_to_vec(),
            Self::ReplayDeltaRequest(message) => message.encode_to_vec(),
            Self::ReplayDeltaResponse(message) => message.encode_to_vec(),
        }
    }

    pub fn encoded_len(&self) -> usize {
        match self {
            Self::Manifests(message) => message.encoded_len(),
            Self::Ping(message) => message.encoded_len(),
            Self::Cluster(message) => message.encoded_len(),
            Self::Endpoints(message) => message.encoded_len(),
            Self::Transaction(message) => message.encoded_len(),
            Self::GetLedger(message) => message.encoded_len(),
            Self::LedgerData(message) => message.encoded_len(),
            Self::ProposeLedger(message) => message.encoded_len(),
            Self::StatusChange(message) => message.encoded_len(),
            Self::HaveSet(message) => message.encoded_len(),
            Self::Validation(message) => message.encoded_len(),
            Self::ValidatorList(message) => message.encoded_len(),
            Self::ValidatorListCollection(message) => message.encoded_len(),
            Self::GetObjects(message) => message.encoded_len(),
            Self::HaveTransactions(message) => message.encoded_len(),
            Self::Transactions(message) => message.encoded_len(),
            Self::Squelch(message) => message.encoded_len(),
            Self::ProofPathRequest(message) => message.encoded_len(),
            Self::ProofPathResponse(message) => message.encoded_len(),
            Self::ReplayDeltaRequest(message) => message.encoded_len(),
            Self::ReplayDeltaResponse(message) => message.encoded_len(),
        }
    }

    pub fn decode(
        message_type: ProtocolMessageType,
        payload: &[u8],
    ) -> Result<Self, ProtocolMessageError> {
        match message_type {
            ProtocolMessageType::MtManifests => Ok(Self::Manifests(
                TmManifests::decode(payload).map_err(ProtocolMessageError::Decode)?,
            )),
            ProtocolMessageType::MtPing => Ok(Self::Ping(
                TmPing::decode(payload).map_err(ProtocolMessageError::Decode)?,
            )),
            ProtocolMessageType::MtCluster => Ok(Self::Cluster(
                TmCluster::decode(payload).map_err(ProtocolMessageError::Decode)?,
            )),
            ProtocolMessageType::MtEndpoints => Ok(Self::Endpoints(
                TmEndpoints::decode(payload).map_err(ProtocolMessageError::Decode)?,
            )),
            ProtocolMessageType::MtTransaction => Ok(Self::Transaction(
                TmTransaction::decode(payload).map_err(ProtocolMessageError::Decode)?,
            )),
            ProtocolMessageType::MtGetLedger => Ok(Self::GetLedger(
                TmGetLedger::decode(payload).map_err(ProtocolMessageError::Decode)?,
            )),
            ProtocolMessageType::MtLedgerData => Ok(Self::LedgerData(
                TmLedgerData::decode(payload).map_err(ProtocolMessageError::Decode)?,
            )),
            ProtocolMessageType::MtProposeLedger => Ok(Self::ProposeLedger(
                TmProposeSet::decode(payload).map_err(ProtocolMessageError::Decode)?,
            )),
            ProtocolMessageType::MtStatusChange => Ok(Self::StatusChange(
                TmStatusChange::decode(payload).map_err(ProtocolMessageError::Decode)?,
            )),
            ProtocolMessageType::MtHaveSet => Ok(Self::HaveSet(
                TmHaveTransactionSet::decode(payload).map_err(ProtocolMessageError::Decode)?,
            )),
            ProtocolMessageType::MtValidation => Ok(Self::Validation(
                TmValidation::decode(payload).map_err(ProtocolMessageError::Decode)?,
            )),
            ProtocolMessageType::MtValidatorList => Ok(Self::ValidatorList(
                TmValidatorList::decode(payload).map_err(ProtocolMessageError::Decode)?,
            )),
            ProtocolMessageType::MtValidatorListCollection => Ok(Self::ValidatorListCollection(
                TmValidatorListCollection::decode(payload).map_err(ProtocolMessageError::Decode)?,
            )),
            ProtocolMessageType::MtGetObjects => Ok(Self::GetObjects(
                TmGetObjectByHash::decode(payload).map_err(ProtocolMessageError::Decode)?,
            )),
            ProtocolMessageType::MtHaveTransactions => Ok(Self::HaveTransactions(
                TmHaveTransactions::decode(payload).map_err(ProtocolMessageError::Decode)?,
            )),
            ProtocolMessageType::MtTransactions => Ok(Self::Transactions(
                TmTransactions::decode(payload).map_err(ProtocolMessageError::Decode)?,
            )),
            ProtocolMessageType::MtSquelch => Ok(Self::Squelch(
                TmSquelch::decode(payload).map_err(ProtocolMessageError::Decode)?,
            )),
            ProtocolMessageType::MtProofPathReq => Ok(Self::ProofPathRequest(
                TmProofPathRequest::decode(payload).map_err(ProtocolMessageError::Decode)?,
            )),
            ProtocolMessageType::MtProofPathResponse => Ok(Self::ProofPathResponse(
                TmProofPathResponse::decode(payload).map_err(ProtocolMessageError::Decode)?,
            )),
            ProtocolMessageType::MtReplayDeltaReq => Ok(Self::ReplayDeltaRequest(
                TmReplayDeltaRequest::decode(payload).map_err(ProtocolMessageError::Decode)?,
            )),
            ProtocolMessageType::MtReplayDeltaResponse => Ok(Self::ReplayDeltaResponse(
                TmReplayDeltaResponse::decode(payload).map_err(ProtocolMessageError::Decode)?,
            )),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ProtocolMessage {
    pub message_type: ProtocolMessageType,
    pub payload: ProtocolPayload,
}

impl ProtocolMessage {
    pub fn new(payload: ProtocolPayload) -> Self {
        Self {
            message_type: payload.message_type(),
            payload,
        }
    }
}

struct MessageInner {
    protocol: ProtocolMessage,
    buffer: Vec<u8>,
    buffer_compressed: OnceLock<Option<Vec<u8>>>,
    category: TrafficCategory,
    validator_key: Option<PublicKey>,
}

#[derive(Clone)]
pub struct Message {
    inner: Arc<MessageInner>,
}

impl Message {
    pub fn new(protocol: ProtocolMessage, validator_key: Option<PublicKey>) -> Self {
        let message_bytes = protocol.payload.encoded_len();
        assert!(
            message_bytes != 0,
            "overlay message payload must be non-empty"
        );

        let mut buffer = vec![0u8; HEADER_BYTES + message_bytes];
        set_header(
            &mut buffer,
            message_bytes as u32,
            protocol.message_type as i32,
            CompressionAlgorithm::None,
            0,
        );
        let payload = protocol.payload.encode_to_vec();
        buffer[HEADER_BYTES..].copy_from_slice(&payload);

        tracing::trace!(
            target: "overlay",
            msg_type = ?protocol.message_type,
            compressed_size = buffer.len(),
            uncompressed_size = buffer.len(),
            "Message encoded"
        );

        Self {
            inner: Arc::new(MessageInner {
                category: TrafficCategory::categorize(&protocol, false),
                protocol,
                buffer,
                buffer_compressed: OnceLock::new(),
                validator_key,
            }),
        }
    }

    pub fn protocol(&self) -> &ProtocolMessage {
        &self.inner.protocol
    }

    pub fn get_buffer_size(&self) -> usize {
        self.inner.buffer.len()
    }

    pub fn get_buffer(&self, try_compressed: Compressed) -> &[u8] {
        if try_compressed == Compressed::Off {
            return &self.inner.buffer;
        }
        let compressed = self
            .inner
            .buffer_compressed
            .get_or_init(|| compress_message(&self.inner));
        if let Some(buffer) = compressed {
            buffer
        } else {
            &self.inner.buffer
        }
    }

    pub fn category(&self) -> TrafficCategory {
        self.inner.category
    }

    pub fn validator_key(&self) -> Option<PublicKey> {
        self.inner.validator_key
    }
}

impl fmt::Debug for Message {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Message")
            .field("message_type", &self.inner.protocol.message_type)
            .field("category", &self.inner.category)
            .field("validator_key", &self.inner.validator_key)
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MessageHeader {
    pub total_wire_size: u32,
    pub header_size: u32,
    pub payload_wire_size: u32,
    pub uncompressed_size: u32,
    pub message_type: u16,
    pub algorithm: CompressionAlgorithm,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DecodedProtocolMessage {
    pub header: MessageHeader,
    pub message: Option<ProtocolMessage>,
    pub consumed: usize,
    pub hint: usize,
}

#[derive(Debug)]
pub enum ProtocolMessageError {
    InvalidHeader,
    UnsupportedCompression,
    CompressionDisabled,
    MessageTooLarge,
    DecompressionFailed,
    BadMessage,
    Decode(prost::DecodeError),
}

impl fmt::Display for ProtocolMessageError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidHeader => formatter.write_str("invalid overlay message header"),
            Self::UnsupportedCompression => {
                formatter.write_str("unsupported overlay compression algorithm")
            }
            Self::CompressionDisabled => {
                formatter.write_str("received compressed overlay message without negotiation")
            }
            Self::MessageTooLarge => formatter.write_str("overlay message exceeds maximum size"),
            Self::DecompressionFailed => {
                formatter.write_str("overlay message decompression failed")
            }
            Self::BadMessage => formatter.write_str("overlay message payload failed to decode"),
            Self::Decode(error) => write!(formatter, "{error}"),
        }
    }
}

impl std::error::Error for ProtocolMessageError {}

pub trait ProtocolMessageHandler {
    fn compression_enabled(&self) -> bool {
        true
    }

    fn on_message_begin(&mut self, _header: &MessageHeader, _compressed: bool) {}

    fn on_message(&mut self, message: &ProtocolMessage);

    fn on_message_end(&mut self, _header: &MessageHeader, _message: &ProtocolMessage) {}

    fn on_message_unknown(&mut self, _message_type: u16) {}
}

pub fn protocol_message_name(message_type: i32) -> &'static str {
    match ProtocolMessageType::try_from(message_type) {
        Ok(message_type) => match message_type {
            ProtocolMessageType::MtManifests => "manifests",
            ProtocolMessageType::MtPing => "ping",
            ProtocolMessageType::MtCluster => "cluster",
            ProtocolMessageType::MtEndpoints => "endpoints",
            ProtocolMessageType::MtTransaction => "tx",
            ProtocolMessageType::MtGetLedger => "get_ledger",
            ProtocolMessageType::MtLedgerData => "ledger_data",
            ProtocolMessageType::MtProposeLedger => "propose",
            ProtocolMessageType::MtStatusChange => "status",
            ProtocolMessageType::MtHaveSet => "have_set",
            ProtocolMessageType::MtValidation => "validation",
            ProtocolMessageType::MtGetObjects => "get_objects",
            ProtocolMessageType::MtValidatorList => "validator_list",
            ProtocolMessageType::MtSquelch => "squelch",
            ProtocolMessageType::MtValidatorListCollection => "validator_list_collection",
            ProtocolMessageType::MtProofPathReq => "proof_path_request",
            ProtocolMessageType::MtProofPathResponse => "proof_path_response",
            ProtocolMessageType::MtReplayDeltaReq => "replay_delta_request",
            ProtocolMessageType::MtReplayDeltaResponse => "replay_delta_response",
            ProtocolMessageType::MtHaveTransactions => "have_transactions",
            ProtocolMessageType::MtTransactions => "transactions",
        },
        Err(_) => "unknown",
    }
}

pub fn parse_message_header(bytes: &[u8]) -> Result<Option<MessageHeader>, ProtocolMessageError> {
    if bytes.is_empty() {
        return Ok(None);
    }

    let first = bytes[0];
    if first & 0x80 != 0 {
        if bytes.len() < HEADER_BYTES_COMPRESSED {
            return Ok(None);
        }
        if first & 0x0C != 0 {
            return Err(ProtocolMessageError::InvalidHeader);
        }

        let algorithm = CompressionAlgorithm::from_header_bits(first & 0xF0)
            .ok_or(ProtocolMessageError::UnsupportedCompression)?;
        if algorithm != CompressionAlgorithm::Lz4 {
            return Err(ProtocolMessageError::UnsupportedCompression);
        }

        let payload_wire_size =
            u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) & 0x0FFF_FFFF;
        let message_type = u16::from_be_bytes([bytes[4], bytes[5]]);
        let uncompressed_size = u32::from_be_bytes([bytes[6], bytes[7], bytes[8], bytes[9]]);
        return Ok(Some(MessageHeader {
            total_wire_size: HEADER_BYTES_COMPRESSED as u32 + payload_wire_size,
            header_size: HEADER_BYTES_COMPRESSED as u32,
            payload_wire_size,
            uncompressed_size,
            message_type,
            algorithm,
        }));
    }

    if first & 0xFC == 0 {
        if bytes.len() < HEADER_BYTES {
            return Ok(None);
        }

        let payload_wire_size = u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
        let message_type = u16::from_be_bytes([bytes[4], bytes[5]]);
        return Ok(Some(MessageHeader {
            total_wire_size: HEADER_BYTES as u32 + payload_wire_size,
            header_size: HEADER_BYTES as u32,
            payload_wire_size,
            uncompressed_size: payload_wire_size,
            message_type,
            algorithm: CompressionAlgorithm::None,
        }));
    }

    Err(ProtocolMessageError::InvalidHeader)
}

pub fn decode_protocol_message(
    bytes: &[u8],
    compression_enabled: bool,
) -> Result<DecodedProtocolMessage, ProtocolMessageError> {
    let Some(header) = parse_message_header(bytes)? else {
        return Ok(DecodedProtocolMessage {
            header: MessageHeader {
                total_wire_size: 0,
                header_size: 0,
                payload_wire_size: 0,
                uncompressed_size: 0,
                message_type: 0,
                algorithm: CompressionAlgorithm::None,
            },
            message: None,
            consumed: 0,
            hint: 0,
        });
    };

    if header.payload_wire_size as usize > MAXIMUM_MESSAGE_SIZE
        || header.uncompressed_size as usize > MAXIMUM_MESSAGE_SIZE
    {
        return Err(ProtocolMessageError::MessageTooLarge);
    }

    if !compression_enabled && header.algorithm != CompressionAlgorithm::None {
        return Err(ProtocolMessageError::CompressionDisabled);
    }

    if header.total_wire_size as usize > bytes.len() {
        return Ok(DecodedProtocolMessage {
            header,
            message: None,
            consumed: 0,
            hint: header.total_wire_size as usize - bytes.len(),
        });
    }

    let payload = &bytes[header.header_size as usize..header.total_wire_size as usize];
    let payload = if header.algorithm == CompressionAlgorithm::None {
        payload.to_vec()
    } else {
        decompress(payload, header.uncompressed_size as usize, header.algorithm)
            .ok_or(ProtocolMessageError::DecompressionFailed)?
    };

    let message = match ProtocolMessageType::try_from(header.message_type as i32) {
        Ok(message_type) => Some(ProtocolMessage {
            message_type,
            payload: ProtocolPayload::decode(message_type, &payload)?,
        }),
        Err(_) => {
            tracing::warn!(target: "overlay", "Failed to decode message from peer");
            None
        }
    };

    Ok(DecodedProtocolMessage {
        header,
        message,
        consumed: header.total_wire_size as usize,
        hint: 0,
    })
}

pub fn invoke_protocol_message<H: ProtocolMessageHandler>(
    bytes: &[u8],
    handler: &mut H,
    hint: &mut usize,
) -> Result<usize, ProtocolMessageError> {
    let decoded = decode_protocol_message(bytes, handler.compression_enabled())?;
    *hint = decoded.hint;
    if decoded.consumed == 0 {
        return Ok(0);
    }

    if let Some(message) = decoded.message.as_ref() {
        handler.on_message_begin(
            &decoded.header,
            decoded.header.algorithm != CompressionAlgorithm::None,
        );
        handler.on_message(message);
        handler.on_message_end(&decoded.header, message);
        Ok(decoded.consumed)
    } else {
        handler.on_message_unknown(decoded.header.message_type);
        Ok(decoded.consumed)
    }
}

fn compress_message(inner: &MessageInner) -> Option<Vec<u8>> {
    let message_bytes = inner.buffer.len() - HEADER_BYTES;
    if !is_compressible(inner.protocol.message_type, message_bytes) {
        return None;
    }

    let compressed = compress(&inner.buffer[HEADER_BYTES..], CompressionAlgorithm::Lz4)?;
    if compressed.len() >= message_bytes - (HEADER_BYTES_COMPRESSED - HEADER_BYTES) {
        return None;
    }

    let mut buffer = vec![0u8; HEADER_BYTES_COMPRESSED + compressed.len()];
    set_header(
        &mut buffer,
        compressed.len() as u32,
        inner.protocol.message_type as i32,
        CompressionAlgorithm::Lz4,
        message_bytes as u32,
    );
    buffer[HEADER_BYTES_COMPRESSED..].copy_from_slice(&compressed);
    tracing::trace!(
        target: "overlay",
        msg_type = ?inner.protocol.message_type,
        compressed_size = buffer.len(),
        uncompressed_size = HEADER_BYTES + message_bytes,
        "Message encoded"
    );
    Some(buffer)
}

fn is_compressible(message_type: ProtocolMessageType, message_bytes: usize) -> bool {
    if message_bytes <= 70 {
        return false;
    }

    matches!(
        message_type,
        ProtocolMessageType::MtManifests
            | ProtocolMessageType::MtEndpoints
            | ProtocolMessageType::MtTransaction
            | ProtocolMessageType::MtGetLedger
            | ProtocolMessageType::MtLedgerData
            | ProtocolMessageType::MtGetObjects
            | ProtocolMessageType::MtValidatorList
            | ProtocolMessageType::MtValidatorListCollection
            | ProtocolMessageType::MtReplayDeltaResponse
            | ProtocolMessageType::MtTransactions
    )
}

fn set_header(
    buffer: &mut [u8],
    payload_bytes: u32,
    message_type: i32,
    compression: CompressionAlgorithm,
    uncompressed_bytes: u32,
) {
    let mut payload_header = payload_bytes.to_be_bytes();
    if compression != CompressionAlgorithm::None {
        payload_header[0] |= compression as u8;
    }
    buffer[..4].copy_from_slice(&payload_header);
    buffer[4..6].copy_from_slice(&(message_type as u16).to_be_bytes());
    if compression != CompressionAlgorithm::None {
        buffer[6..10].copy_from_slice(&uncompressed_bytes.to_be_bytes());
    }
}

#[cfg(test)]
mod tests {
    use super::{
        Compressed, Message, ProtocolMessage, ProtocolMessageError, ProtocolMessageHandler,
        ProtocolMessageType, ProtocolPayload, TmManifests, TmPing, TmTransaction,
        decode_protocol_message, invoke_protocol_message, parse_message_header,
        protocol_message_name, wire,
    };

    #[test]
    fn parse_uncompressed_header_layout() {
        let message = Message::new(
            ProtocolMessage::new(ProtocolPayload::Ping(TmPing {
                r#type: 0,
                seq: Some(7),
                ping_time: None,
                net_time: None,
            })),
            None,
        );
        let header = parse_message_header(message.get_buffer(Compressed::Off))
            .expect("header parse")
            .expect("header present");
        assert_eq!(header.header_size, 6);
        assert_eq!(header.message_type, ProtocolMessageType::MtPing as u16);
        assert_eq!(header.uncompressed_size, header.payload_wire_size);
    }

    #[test]
    fn compressed_message_round_trips() {
        let manifests = TmManifests {
            list: (0..40)
                .map(|index| wire::TmManifest {
                    stobject: vec![index as u8; 100],
                })
                .collect(),
            ..Default::default()
        };
        let message = Message::new(
            ProtocolMessage::new(ProtocolPayload::Manifests(manifests)),
            None,
        );
        let wire = message.get_buffer(Compressed::On);
        let decoded = decode_protocol_message(wire, true).expect("decode");
        assert_eq!(decoded.header.header_size, 10);
        assert!(decoded.header.payload_wire_size < decoded.header.uncompressed_size);
        assert!(matches!(
            decoded.message,
            Some(ProtocolMessage {
                payload: ProtocolPayload::Manifests(_),
                ..
            })
        ));
    }

    #[test]
    fn decode_rejects_compressed_message_when_not_negotiated() {
        let manifests = TmManifests {
            list: (0..40)
                .map(|index| wire::TmManifest {
                    stobject: vec![index as u8; 100],
                })
                .collect(),
            ..Default::default()
        };
        let message = Message::new(
            ProtocolMessage::new(ProtocolPayload::Manifests(manifests)),
            None,
        );
        let error =
            decode_protocol_message(message.get_buffer(Compressed::On), false).expect_err("error");
        assert!(matches!(error, ProtocolMessageError::CompressionDisabled));
    }

    #[test]
    fn invoke_routes_known_and_unknown_messages() {
        #[derive(Default)]
        struct Handler {
            seen_message: bool,
            unknown_type: Option<u16>,
        }

        impl ProtocolMessageHandler for Handler {
            fn on_message(&mut self, _message: &ProtocolMessage) {
                self.seen_message = true;
            }

            fn on_message_unknown(&mut self, message_type: u16) {
                self.unknown_type = Some(message_type);
            }
        }

        let message = Message::new(
            ProtocolMessage::new(ProtocolPayload::Transaction(TmTransaction {
                raw_transaction: vec![1, 2, 3, 4],
                status: 1,
                receive_timestamp: None,
                deferred: None,
            })),
            None,
        );

        let mut handler = Handler::default();
        let mut hint = 0;
        let consumed =
            invoke_protocol_message(message.get_buffer(Compressed::Off), &mut handler, &mut hint)
                .expect("invoke");
        assert_eq!(consumed, message.get_buffer_size());
        assert!(handler.seen_message);
        assert_eq!(hint, 0);

        let mut wire = message.get_buffer(Compressed::Off).to_vec();
        wire[4..6].copy_from_slice(&999u16.to_be_bytes());
        let mut handler = Handler::default();
        let consumed = invoke_protocol_message(&wire, &mut handler, &mut hint).expect("invoke");
        assert_eq!(consumed, message.get_buffer_size());
        assert_eq!(handler.unknown_type, Some(999));
        assert_eq!(protocol_message_name(999), "unknown");
    }
}
