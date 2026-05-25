//! Narrow `validator_list_sites` RPC wrapper.

use protocol::JsonValue;

use crate::JsonContext;

pub trait ValidatorListSitesSource {
    fn get_validator_list_sites(&self) -> JsonValue;
}

pub fn do_validator_list_sites<S: ValidatorListSitesSource>(
    context: &JsonContext<'_, S>,
) -> JsonValue {
    context.env.get_validator_list_sites()
}
