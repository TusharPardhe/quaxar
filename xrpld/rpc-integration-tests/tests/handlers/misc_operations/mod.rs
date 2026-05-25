//! Miscellaneous handler operation tests.

//! Integration tests for misc operations.

pub(super) use protocol::{get_field_by_symbol, to_base58, JsonValue, STAmount, STTx, TxType};
pub(super) use rpc_integration_tests::env::*;
pub(super) use rpc_integration_tests::helpers::*;

mod extended_operations;
mod handler_responses;
