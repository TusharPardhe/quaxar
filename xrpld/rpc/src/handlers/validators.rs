//! Narrow `validators` RPC wrapper.

use protocol::JsonValue;

use crate::JsonContext;

pub trait ValidatorsSource {
    fn get_validators(&self) -> JsonValue;
}

pub fn do_validators<S: ValidatorsSource>(context: &JsonContext<'_, S>) -> JsonValue {
    context.env.get_validators()
}
