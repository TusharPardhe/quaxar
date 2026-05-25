//! GRPCHandlers stub ported from `xrpld/rpc/GRPCHandlers.h`.
//!
//! gRPC versions of ledger, ledger_entry, ledger_data, ledger_diff.
//! These are stubs that define the handler signatures. The actual protobuf
//! types come from the `xrpl-protocol` proto definitions.
//!
//! In the reference codebase these handlers take a `GRPCContext<T>` and return
//! `(Response, grpc::Status)`. In Rust we model this with Result types.

#![allow(dead_code)]

use protocol::JsonValue;

/// gRPC status codes (subset matching grpc::StatusCode).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GrpcStatusCode {
    Ok = 0,
    Cancelled = 1,
    Unknown = 2,
    InvalidArgument = 3,
    NotFound = 5,
    AlreadyExists = 6,
    PermissionDenied = 7,
    ResourceExhausted = 8,
    FailedPrecondition = 9,
    Unimplemented = 12,
    Internal = 13,
    Unavailable = 14,
}

/// gRPC status with code and message.
#[derive(Debug, Clone)]
pub struct GrpcStatus {
    pub code: GrpcStatusCode,
    pub message: String,
}

impl GrpcStatus {
    pub fn ok() -> Self {
        Self {
            code: GrpcStatusCode::Ok,
            message: String::new(),
        }
    }

    pub fn error(code: GrpcStatusCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    pub fn is_ok(&self) -> bool {
        self.code == GrpcStatusCode::Ok
    }
}

/// Trait representing the gRPC context for a request.
pub trait GrpcContext<Request> {
    fn request(&self) -> &Request;
    fn is_unlimited(&self) -> bool;
}

/// Placeholder request/response types until protobuf codegen is wired.
/// These will be replaced by the actual generated protobuf types.
#[derive(Debug, Clone, Default)]
pub struct GetLedgerRequest {
    pub ledger_index: u32,
    pub transactions: bool,
    pub expand: bool,
}

#[derive(Debug, Clone, Default)]
pub struct GetLedgerResponse {
    pub ledger_index: u32,
    pub ledger_hash: Vec<u8>,
}

#[derive(Debug, Clone, Default)]
pub struct GetLedgerEntryRequest {
    pub key: Vec<u8>,
    pub ledger_index: u32,
}

#[derive(Debug, Clone, Default)]
pub struct GetLedgerEntryResponse {
    pub entry: Vec<u8>,
    pub ledger_index: u32,
}

#[derive(Debug, Clone, Default)]
pub struct GetLedgerDataRequest {
    pub ledger_index: u32,
    pub marker: Vec<u8>,
    pub limit: u32,
}

#[derive(Debug, Clone, Default)]
pub struct GetLedgerDataResponse {
    pub state_objects: Vec<Vec<u8>>,
    pub marker: Vec<u8>,
    pub ledger_index: u32,
}

#[derive(Debug, Clone, Default)]
pub struct GetLedgerDiffRequest {
    pub base_ledger_index: u32,
    pub desired_ledger_index: u32,
}

#[derive(Debug, Clone, Default)]
pub struct GetLedgerDiffResponse {
    pub diffs: Vec<Vec<u8>>,
}

/// gRPC handler for GetLedger.
pub fn do_ledger_grpc(
    _context: &dyn GrpcContext<GetLedgerRequest>,
) -> (GetLedgerResponse, GrpcStatus) {
    // Stub — will be implemented when full gRPC integration lands
    (
        GetLedgerResponse::default(),
        GrpcStatus::error(GrpcStatusCode::Unimplemented, "Not yet implemented"),
    )
}

/// gRPC handler for GetLedgerEntry.
pub fn do_ledger_entry_grpc(
    _context: &dyn GrpcContext<GetLedgerEntryRequest>,
) -> (GetLedgerEntryResponse, GrpcStatus) {
    (
        GetLedgerEntryResponse::default(),
        GrpcStatus::error(GrpcStatusCode::Unimplemented, "Not yet implemented"),
    )
}

/// gRPC handler for GetLedgerData.
pub fn do_ledger_data_grpc(
    _context: &dyn GrpcContext<GetLedgerDataRequest>,
) -> (GetLedgerDataResponse, GrpcStatus) {
    (
        GetLedgerDataResponse::default(),
        GrpcStatus::error(GrpcStatusCode::Unimplemented, "Not yet implemented"),
    )
}

/// gRPC handler for GetLedgerDiff.
pub fn do_ledger_diff_grpc(
    _context: &dyn GrpcContext<GetLedgerDiffRequest>,
) -> (GetLedgerDiffResponse, GrpcStatus) {
    (
        GetLedgerDiffResponse::default(),
        GrpcStatus::error(GrpcStatusCode::Unimplemented, "Not yet implemented"),
    )
}
