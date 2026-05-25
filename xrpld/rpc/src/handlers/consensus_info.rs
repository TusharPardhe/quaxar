//! Narrow `consensus_info` RPC wrapper.

use std::collections::BTreeMap;

use protocol::JsonValue;

use crate::JsonContext;

pub trait ConsensusInfoSource {
    fn get_consensus_info(&self) -> JsonValue;
}

pub fn do_consensus_info<S: ConsensusInfoSource>(context: &JsonContext<'_, S>) -> JsonValue {
    JsonValue::Object(BTreeMap::from([(
        "info".to_owned(),
        context.env.get_consensus_info(),
    )]))
}
