//! Narrow `tx_reduce_relay` RPC handler port.
//!
//! The reference handler is just a direct overlay metrics adapter, so Rust keeps the
//! same shape with a single explicit source seam.

use protocol::JsonValue;

pub trait TxReduceRelaySource {
    fn tx_metrics_json(&self) -> JsonValue;
}

pub fn do_tx_reduce_relay<S: TxReduceRelaySource>(source: &S) -> JsonValue {
    source.tx_metrics_json()
}
