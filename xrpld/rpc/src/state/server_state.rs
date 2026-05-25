//! Narrow `server_state` RPC handler port.

use protocol::JsonValue;

use crate::JsonContext;
use crate::state::server_info::{ServerInfoSource, build_response};

pub use crate::state::server_info::ServerInfoSource as ServerStateSource;

pub fn do_server_state<S: ServerInfoSource>(context: &JsonContext<'_, S>) -> JsonValue {
    build_response(context, false, "state")
}
