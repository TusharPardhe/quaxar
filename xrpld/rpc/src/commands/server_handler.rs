//! RPC handler registry and request-shaping helpers aligned with
//!
//! # Performance note
//! [`fill_handler`] is on the hot path (called for every inbound RPC request).
//! Handler lookup uses a compile-time [`phf`] perfect-hash map so dispatch is
//! O(1) with zero heap allocation, rather than O(n) linear scan.
//! `xrpld/rpc/detail/the reference source`.

use protocol::JsonValue;

use crate::state::context::RpcRuntime;
use crate::state::role::Role;
use crate::status::{RpcErrorCode, Status};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HandlerCondition {
    None,
    NeedsCurrentLedger,
    NeedsClosedLedger,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RpcHandlerSpec {
    pub name: &'static str,
    pub required_role: Role,
    pub condition: HandlerCondition,
    pub min_api_version: u32,
    pub max_api_version: u32,
}

impl RpcHandlerSpec {
    const fn new(
        name: &'static str,
        required_role: Role,
        condition: HandlerCondition,
        min_api_version: u32,
        max_api_version: u32,
    ) -> Self {
        Self {
            name,
            required_role,
            condition,
            min_api_version,
            max_api_version,
        }
    }

    fn supports_api(self, api_version: u32) -> bool {
        api_version >= self.min_api_version && api_version <= self.max_api_version
    }
}

// ---------------------------------------------------------------------------
// PHF perfect-hash map: command name → &'static RpcHandlerSpec
// Built at compile time; lookup is O(1) with no heap allocation.
//
// IMPORTANT: every entry in HANDLERS below *must* also appear here.
// The macro will fail at compile time if duplicate keys are present.
// ---------------------------------------------------------------------------
static HANDLER_MAP: phf::Map<&'static str, RpcHandlerSpec> = phf::phf_map! {
    "account_info"          => RpcHandlerSpec::new("account_info",          Role::User,  HandlerCondition::None,                1, u32::MAX),
    "account_lines"         => RpcHandlerSpec::new("account_lines",         Role::User,  HandlerCondition::None,                1, u32::MAX),
    "account_tx"            => RpcHandlerSpec::new("account_tx",            Role::User,  HandlerCondition::None,                1, u32::MAX),
    "can_delete"            => RpcHandlerSpec::new("can_delete",            Role::Admin, HandlerCondition::None,                1, u32::MAX),
    "export_snapshot"       => RpcHandlerSpec::new("export_snapshot",       Role::Admin, HandlerCondition::None,                1, u32::MAX),
    "channel_authorize"     => RpcHandlerSpec::new("channel_authorize",     Role::User,  HandlerCondition::None,                1, u32::MAX),
    "connect"               => RpcHandlerSpec::new("connect",               Role::Admin, HandlerCondition::None,                1, u32::MAX),
    "fee"                   => RpcHandlerSpec::new("fee",                   Role::User,  HandlerCondition::NeedsCurrentLedger,  1, u32::MAX),
    "ledger"                => RpcHandlerSpec::new("ledger",                Role::User,  HandlerCondition::None,                1, u32::MAX),
    "ledger_accept"         => RpcHandlerSpec::new("ledger_accept",         Role::Admin, HandlerCondition::None,                1, u32::MAX),
    "ledger_cleaner"        => RpcHandlerSpec::new("ledger_cleaner",        Role::Admin, HandlerCondition::None,                1, u32::MAX),
    "ledger_closed"         => RpcHandlerSpec::new("ledger_closed",         Role::User,  HandlerCondition::NeedsClosedLedger,   1, u32::MAX),
    "ledger_current"        => RpcHandlerSpec::new("ledger_current",        Role::User,  HandlerCondition::NeedsCurrentLedger,  1, u32::MAX),
    "ledger_entry"          => RpcHandlerSpec::new("ledger_entry",          Role::User,  HandlerCondition::None,                1, u32::MAX),
    "ledger_request"        => RpcHandlerSpec::new("ledger_request",        Role::Admin, HandlerCondition::None,                1, u32::MAX),
    "log_level"             => RpcHandlerSpec::new("log_level",             Role::Admin, HandlerCondition::None,                1, u32::MAX),
    "log_rotate"            => RpcHandlerSpec::new("log_rotate",            Role::Admin, HandlerCondition::None,                1, u32::MAX),
    "manifest"              => RpcHandlerSpec::new("manifest",              Role::User,  HandlerCondition::None,                1, u32::MAX),
    "path_find"             => RpcHandlerSpec::new("path_find",             Role::User,  HandlerCondition::NeedsCurrentLedger,  1, u32::MAX),
    "peers"                 => RpcHandlerSpec::new("peers",                 Role::Admin, HandlerCondition::None,                1, u32::MAX),
    "peer_reservations_add" => RpcHandlerSpec::new("peer_reservations_add", Role::Admin, HandlerCondition::None,                1, u32::MAX),
    "peer_reservations_del" => RpcHandlerSpec::new("peer_reservations_del", Role::Admin, HandlerCondition::None,                1, u32::MAX),
    "peer_reservations_list"=> RpcHandlerSpec::new("peer_reservations_list",Role::Admin, HandlerCondition::None,                1, u32::MAX),
    "ripple_path_find"      => RpcHandlerSpec::new("ripple_path_find",      Role::User,  HandlerCondition::None,                1, u32::MAX),
    "ping"                  => RpcHandlerSpec::new("ping",                  Role::User,  HandlerCondition::None,                1, u32::MAX),
    "server_definitions"    => RpcHandlerSpec::new("server_definitions",    Role::User,  HandlerCondition::None,                1, u32::MAX),
    "server_info"           => RpcHandlerSpec::new("server_info",           Role::User,  HandlerCondition::None,                1, u32::MAX),
    "server_state"          => RpcHandlerSpec::new("server_state",          Role::User,  HandlerCondition::None,                1, u32::MAX),
    "sign_for"              => RpcHandlerSpec::new("sign_for",              Role::User,  HandlerCondition::None,                1, u32::MAX),
    "simulate"              => RpcHandlerSpec::new("simulate",              Role::User,  HandlerCondition::None,                1, u32::MAX),
    "stop"                  => RpcHandlerSpec::new("stop",                  Role::Admin, HandlerCondition::None,                1, u32::MAX),
    "submit"                => RpcHandlerSpec::new("submit",                Role::User,  HandlerCondition::None,                1, u32::MAX),
    "subscribe"             => RpcHandlerSpec::new("subscribe",             Role::User,  HandlerCondition::None,                1, u32::MAX),
    "transaction_entry"     => RpcHandlerSpec::new("transaction_entry",     Role::User,  HandlerCondition::None,                1, u32::MAX),
    "tx"                    => RpcHandlerSpec::new("tx",                    Role::User,  HandlerCondition::None,                1, u32::MAX),
    "submit_multisigned"    => RpcHandlerSpec::new("submit_multisigned",    Role::User,  HandlerCondition::None,                1, u32::MAX),
    "unsubscribe"           => RpcHandlerSpec::new("unsubscribe",           Role::User,  HandlerCondition::None,                1, u32::MAX),
    "validation_create"     => RpcHandlerSpec::new("validation_create",     Role::Admin, HandlerCondition::None,                1, u32::MAX),
    "wallet_propose"        => RpcHandlerSpec::new("wallet_propose",        Role::User,  HandlerCondition::None,                1, u32::MAX),
    "account_channels"      => RpcHandlerSpec::new("account_channels",      Role::User,  HandlerCondition::None,                1, u32::MAX),
    "account_currencies"    => RpcHandlerSpec::new("account_currencies",    Role::User,  HandlerCondition::None,                1, u32::MAX),
    "account_nfts"          => RpcHandlerSpec::new("account_nfts",          Role::User,  HandlerCondition::None,                1, u32::MAX),
    "account_objects"       => RpcHandlerSpec::new("account_objects",       Role::User,  HandlerCondition::None,                1, u32::MAX),
    "account_offers"        => RpcHandlerSpec::new("account_offers",        Role::User,  HandlerCondition::None,                1, u32::MAX),
    "book_changes"          => RpcHandlerSpec::new("book_changes",          Role::User,  HandlerCondition::None,                1, u32::MAX),
    "book_offers"           => RpcHandlerSpec::new("book_offers",           Role::User,  HandlerCondition::None,                1, u32::MAX),
    "consensus_info"        => RpcHandlerSpec::new("consensus_info",        Role::Admin, HandlerCondition::None,                1, u32::MAX),
    "deposit_authorized"    => RpcHandlerSpec::new("deposit_authorized",    Role::User,  HandlerCondition::None,                1, u32::MAX),
    "gateway_balances"      => RpcHandlerSpec::new("gateway_balances",      Role::User,  HandlerCondition::None,                1, u32::MAX),
    "get_counts"            => RpcHandlerSpec::new("get_counts",            Role::Admin, HandlerCondition::None,                1, u32::MAX),
    "ledger_data"           => RpcHandlerSpec::new("ledger_data",           Role::User,  HandlerCondition::None,                1, u32::MAX),
    "ledger_header"         => RpcHandlerSpec::new("ledger_header",         Role::User,  HandlerCondition::None,                1, u32::MAX),
    "no_ripple_check"       => RpcHandlerSpec::new("no_ripple_check",       Role::User,  HandlerCondition::None,                1, u32::MAX),
    "nft_buy_offers"        => RpcHandlerSpec::new("nft_buy_offers",        Role::User,  HandlerCondition::None,                1, u32::MAX),
    "nft_sell_offers"       => RpcHandlerSpec::new("nft_sell_offers",       Role::User,  HandlerCondition::None,                1, u32::MAX),
    "owner_info"            => RpcHandlerSpec::new("owner_info",            Role::User,  HandlerCondition::None,                1, u32::MAX),
    "print"                 => RpcHandlerSpec::new("print",                 Role::Admin, HandlerCondition::None,                1, u32::MAX),
    "random"                => RpcHandlerSpec::new("random",                Role::User,  HandlerCondition::None,                1, u32::MAX),
    "sign"                  => RpcHandlerSpec::new("sign",                  Role::User,  HandlerCondition::None,                1, u32::MAX),
    "tx_history"            => RpcHandlerSpec::new("tx_history",            Role::User,  HandlerCondition::None,                1, u32::MAX),
    "unl_list"              => RpcHandlerSpec::new("unl_list",              Role::Admin, HandlerCondition::None,                1, u32::MAX),
    "validator_info"        => RpcHandlerSpec::new("validator_info",        Role::Admin, HandlerCondition::None,                1, u32::MAX),
    "validator_list_sites"  => RpcHandlerSpec::new("validator_list_sites",  Role::Admin, HandlerCondition::None,                1, u32::MAX),
    "validators"            => RpcHandlerSpec::new("validators",            Role::Admin, HandlerCondition::None,                1, u32::MAX),
    "feature"               => RpcHandlerSpec::new("feature",               Role::Admin, HandlerCondition::None,                1, u32::MAX),
    "fetch_info"            => RpcHandlerSpec::new("fetch_info",            Role::Admin, HandlerCondition::None,                1, u32::MAX),
    "amm_info"              => RpcHandlerSpec::new("amm_info",              Role::User,  HandlerCondition::None,                1, u32::MAX),
    "blacklist"             => RpcHandlerSpec::new("blacklist",             Role::Admin, HandlerCondition::None,                1, u32::MAX),
    "channel_verify"        => RpcHandlerSpec::new("channel_verify",        Role::User,  HandlerCondition::None,                1, u32::MAX),
    "get_aggregate_price"   => RpcHandlerSpec::new("get_aggregate_price",   Role::User,  HandlerCondition::None,                1, u32::MAX),
    "logrotate"             => RpcHandlerSpec::new("logrotate",             Role::Admin, HandlerCondition::None,                1, u32::MAX),
    "noripple_check"        => RpcHandlerSpec::new("noripple_check",        Role::User,  HandlerCondition::None,                1, u32::MAX),
    "tx_reduce_relay"       => RpcHandlerSpec::new("tx_reduce_relay",       Role::User,  HandlerCondition::None,                1, u32::MAX),
    "vault_info"            => RpcHandlerSpec::new("vault_info",            Role::User,  HandlerCondition::None,                1, u32::MAX),
    "version"               => RpcHandlerSpec::new("version",               Role::User,  HandlerCondition::None,                1, u32::MAX),
};

const HANDLERS: &[RpcHandlerSpec] = &[
    RpcHandlerSpec::new(
        "account_info",
        Role::User,
        HandlerCondition::None,
        1,
        u32::MAX,
    ),
    RpcHandlerSpec::new(
        "account_lines",
        Role::User,
        HandlerCondition::None,
        1,
        u32::MAX,
    ),
    RpcHandlerSpec::new(
        "account_tx",
        Role::User,
        HandlerCondition::None,
        1,
        u32::MAX,
    ),
    RpcHandlerSpec::new(
        "can_delete",
        Role::Admin,
        HandlerCondition::None,
        1,
        u32::MAX,
    ),
    RpcHandlerSpec::new(
        "export_snapshot",
        Role::Admin,
        HandlerCondition::None,
        1,
        u32::MAX,
    ),
    RpcHandlerSpec::new(
        "channel_authorize",
        Role::User,
        HandlerCondition::None,
        1,
        u32::MAX,
    ),
    RpcHandlerSpec::new("connect", Role::Admin, HandlerCondition::None, 1, u32::MAX),
    RpcHandlerSpec::new(
        "fee",
        Role::User,
        HandlerCondition::NeedsCurrentLedger,
        1,
        u32::MAX,
    ),
    RpcHandlerSpec::new("ledger", Role::User, HandlerCondition::None, 1, u32::MAX),
    RpcHandlerSpec::new(
        "ledger_accept",
        Role::Admin,
        HandlerCondition::None,
        1,
        u32::MAX,
    ),
    RpcHandlerSpec::new(
        "ledger_cleaner",
        Role::Admin,
        HandlerCondition::None,
        1,
        u32::MAX,
    ),
    RpcHandlerSpec::new(
        "ledger_closed",
        Role::User,
        HandlerCondition::NeedsClosedLedger,
        1,
        u32::MAX,
    ),
    RpcHandlerSpec::new(
        "ledger_current",
        Role::User,
        HandlerCondition::NeedsCurrentLedger,
        1,
        u32::MAX,
    ),
    RpcHandlerSpec::new(
        "ledger_entry",
        Role::User,
        HandlerCondition::None,
        1,
        u32::MAX,
    ),
    RpcHandlerSpec::new(
        "ledger_request",
        Role::Admin,
        HandlerCondition::None,
        1,
        u32::MAX,
    ),
    RpcHandlerSpec::new(
        "log_level",
        Role::Admin,
        HandlerCondition::None,
        1,
        u32::MAX,
    ),
    RpcHandlerSpec::new(
        "log_rotate",
        Role::Admin,
        HandlerCondition::None,
        1,
        u32::MAX,
    ),
    RpcHandlerSpec::new("manifest", Role::User, HandlerCondition::None, 1, u32::MAX),
    RpcHandlerSpec::new(
        "path_find",
        Role::User,
        HandlerCondition::NeedsCurrentLedger,
        1,
        u32::MAX,
    ),
    RpcHandlerSpec::new("peers", Role::Admin, HandlerCondition::None, 1, u32::MAX),
    RpcHandlerSpec::new(
        "peer_reservations_add",
        Role::Admin,
        HandlerCondition::None,
        1,
        u32::MAX,
    ),
    RpcHandlerSpec::new(
        "peer_reservations_del",
        Role::Admin,
        HandlerCondition::None,
        1,
        u32::MAX,
    ),
    RpcHandlerSpec::new(
        "peer_reservations_list",
        Role::Admin,
        HandlerCondition::None,
        1,
        u32::MAX,
    ),
    RpcHandlerSpec::new(
        "ripple_path_find",
        Role::User,
        HandlerCondition::None,
        1,
        u32::MAX,
    ),
    RpcHandlerSpec::new("ping", Role::User, HandlerCondition::None, 1, u32::MAX),
    RpcHandlerSpec::new(
        "server_definitions",
        Role::User,
        HandlerCondition::None,
        1,
        u32::MAX,
    ),
    RpcHandlerSpec::new(
        "server_info",
        Role::User,
        HandlerCondition::None,
        1,
        u32::MAX,
    ),
    RpcHandlerSpec::new(
        "server_state",
        Role::User,
        HandlerCondition::None,
        1,
        u32::MAX,
    ),
    RpcHandlerSpec::new("sign_for", Role::User, HandlerCondition::None, 1, u32::MAX),
    RpcHandlerSpec::new("simulate", Role::User, HandlerCondition::None, 1, u32::MAX),
    RpcHandlerSpec::new("stop", Role::Admin, HandlerCondition::None, 1, u32::MAX),
    RpcHandlerSpec::new("submit", Role::User, HandlerCondition::None, 1, u32::MAX),
    RpcHandlerSpec::new("subscribe", Role::User, HandlerCondition::None, 1, u32::MAX),
    RpcHandlerSpec::new(
        "transaction_entry",
        Role::User,
        HandlerCondition::None,
        1,
        u32::MAX,
    ),
    RpcHandlerSpec::new("tx", Role::User, HandlerCondition::None, 1, u32::MAX),
    RpcHandlerSpec::new(
        "submit_multisigned",
        Role::User,
        HandlerCondition::None,
        1,
        u32::MAX,
    ),
    RpcHandlerSpec::new(
        "unsubscribe",
        Role::User,
        HandlerCondition::None,
        1,
        u32::MAX,
    ),
    RpcHandlerSpec::new(
        "validation_create",
        Role::Admin,
        HandlerCondition::None,
        1,
        u32::MAX,
    ),
    RpcHandlerSpec::new(
        "wallet_propose",
        Role::User,
        HandlerCondition::None,
        1,
        u32::MAX,
    ),
    // --- Additional handlers matching reference the reference source ---
    RpcHandlerSpec::new(
        "account_channels",
        Role::User,
        HandlerCondition::None,
        1,
        u32::MAX,
    ),
    RpcHandlerSpec::new(
        "account_currencies",
        Role::User,
        HandlerCondition::None,
        1,
        u32::MAX,
    ),
    RpcHandlerSpec::new(
        "account_nfts",
        Role::User,
        HandlerCondition::None,
        1,
        u32::MAX,
    ),
    RpcHandlerSpec::new(
        "account_objects",
        Role::User,
        HandlerCondition::None,
        1,
        u32::MAX,
    ),
    RpcHandlerSpec::new(
        "account_offers",
        Role::User,
        HandlerCondition::None,
        1,
        u32::MAX,
    ),
    RpcHandlerSpec::new(
        "book_changes",
        Role::User,
        HandlerCondition::None,
        1,
        u32::MAX,
    ),
    RpcHandlerSpec::new(
        "book_offers",
        Role::User,
        HandlerCondition::None,
        1,
        u32::MAX,
    ),
    RpcHandlerSpec::new(
        "consensus_info",
        Role::Admin,
        HandlerCondition::None,
        1,
        u32::MAX,
    ),
    RpcHandlerSpec::new(
        "deposit_authorized",
        Role::User,
        HandlerCondition::None,
        1,
        u32::MAX,
    ),
    RpcHandlerSpec::new(
        "gateway_balances",
        Role::User,
        HandlerCondition::None,
        1,
        u32::MAX,
    ),
    RpcHandlerSpec::new(
        "get_counts",
        Role::Admin,
        HandlerCondition::None,
        1,
        u32::MAX,
    ),
    RpcHandlerSpec::new(
        "ledger_data",
        Role::User,
        HandlerCondition::None,
        1,
        u32::MAX,
    ),
    RpcHandlerSpec::new(
        "ledger_header",
        Role::User,
        HandlerCondition::None,
        1,
        u32::MAX,
    ),
    RpcHandlerSpec::new(
        "no_ripple_check",
        Role::User,
        HandlerCondition::None,
        1,
        u32::MAX,
    ),
    RpcHandlerSpec::new(
        "nft_buy_offers",
        Role::User,
        HandlerCondition::None,
        1,
        u32::MAX,
    ),
    RpcHandlerSpec::new(
        "nft_sell_offers",
        Role::User,
        HandlerCondition::None,
        1,
        u32::MAX,
    ),
    RpcHandlerSpec::new(
        "owner_info",
        Role::User,
        HandlerCondition::None,
        1,
        u32::MAX,
    ),
    RpcHandlerSpec::new("print", Role::Admin, HandlerCondition::None, 1, u32::MAX),
    RpcHandlerSpec::new("random", Role::User, HandlerCondition::None, 1, u32::MAX),
    RpcHandlerSpec::new("sign", Role::User, HandlerCondition::None, 1, u32::MAX),
    RpcHandlerSpec::new(
        "tx_history",
        Role::User,
        HandlerCondition::None,
        1,
        u32::MAX,
    ),
    RpcHandlerSpec::new("unl_list", Role::Admin, HandlerCondition::None, 1, u32::MAX),
    RpcHandlerSpec::new(
        "validator_info",
        Role::Admin,
        HandlerCondition::None,
        1,
        u32::MAX,
    ),
    RpcHandlerSpec::new(
        "validator_list_sites",
        Role::Admin,
        HandlerCondition::None,
        1,
        u32::MAX,
    ),
    RpcHandlerSpec::new(
        "validators",
        Role::Admin,
        HandlerCondition::None,
        1,
        u32::MAX,
    ),
    RpcHandlerSpec::new("feature", Role::Admin, HandlerCondition::None, 1, u32::MAX),
    RpcHandlerSpec::new(
        "fetch_info",
        Role::Admin,
        HandlerCondition::None,
        1,
        u32::MAX,
    ),
    // Additional reference commands
    RpcHandlerSpec::new("amm_info", Role::User, HandlerCondition::None, 1, u32::MAX),
    RpcHandlerSpec::new(
        "blacklist",
        Role::Admin,
        HandlerCondition::None,
        1,
        u32::MAX,
    ),
    RpcHandlerSpec::new(
        "channel_verify",
        Role::User,
        HandlerCondition::None,
        1,
        u32::MAX,
    ),
    RpcHandlerSpec::new(
        "get_aggregate_price",
        Role::User,
        HandlerCondition::None,
        1,
        u32::MAX,
    ),
    RpcHandlerSpec::new(
        "logrotate",
        Role::Admin,
        HandlerCondition::None,
        1,
        u32::MAX,
    ),
    RpcHandlerSpec::new(
        "noripple_check",
        Role::User,
        HandlerCondition::None,
        1,
        u32::MAX,
    ),
    RpcHandlerSpec::new(
        "tx_reduce_relay",
        Role::User,
        HandlerCondition::None,
        1,
        u32::MAX,
    ),
    RpcHandlerSpec::new(
        "vault_info",
        Role::User,
        HandlerCondition::None,
        1,
        u32::MAX,
    ),
    RpcHandlerSpec::new("version", Role::User, HandlerCondition::None, 1, u32::MAX),
];

pub fn handler_specs() -> &'static [RpcHandlerSpec] {
    HANDLERS
}

fn command_field(params: &JsonValue) -> Result<&str, Status> {
    let JsonValue::Object(object) = params else {
        return Err(Status::new(RpcErrorCode::UnknownCommand));
    };

    let command = object.get("command");
    let method = object.get("method");
    match (command, method) {
        (None, None) => Err(Status::new(RpcErrorCode::UnknownCommand)),
        (Some(JsonValue::String(command)), None) => Ok(command),
        (None, Some(JsonValue::String(method))) => Ok(method),
        (Some(JsonValue::String(command)), Some(JsonValue::String(method))) => {
            if command == method {
                Ok(command)
            } else {
                Err(Status::new(RpcErrorCode::UnknownCommand))
            }
        }
        (Some(_), None) => Err(Status::expected_field_error("command", "string")),
        (None, Some(_)) => Err(Status::expected_field_error("method", "string")),
        (Some(_), Some(_)) => Err(Status::new(RpcErrorCode::UnknownCommand)),
    }
}

fn meets_condition<Runtime: RpcRuntime>(condition: HandlerCondition, runtime: &Runtime) -> Status {
    // conditioned commands, returning rpcNO_NETWORK / rpcNOT_SYNCED.
    if condition != HandlerCondition::None && !runtime.network_synced() {
        return Status::new(RpcErrorCode::NoNetwork);
    }
    match condition {
        HandlerCondition::None => Status::OK,
        HandlerCondition::NeedsCurrentLedger => {
            if runtime.has_current_ledger() {
                Status::OK
            } else {
                Status::new(RpcErrorCode::NotSynced)
            }
        }
        HandlerCondition::NeedsClosedLedger => {
            if runtime.has_closed_ledger() {
                Status::OK
            } else {
                Status::new(RpcErrorCode::NotSynced)
            }
        }
    }
}

pub fn fill_handler<Runtime: RpcRuntime>(
    params: &JsonValue,
    role: Role,
    api_version: u32,
    runtime: &Runtime,
) -> Result<&'static RpcHandlerSpec, Status> {
    if !crate::state::role::is_unlimited(role)
        && runtime.client_job_count() > runtime.max_job_queue_clients()
    {
        return Err(Status::new(RpcErrorCode::TooBusy));
    }

    let command = command_field(params)?;

    // O(1) PHF lookup — replaces the former O(n) linear scan over HANDLERS.
    let Some(handler) = HANDLER_MAP.get(command) else {
        return Err(Status::new(RpcErrorCode::UnknownCommand));
    };

    if !handler.supports_api(api_version) {
        return Err(Status::new(RpcErrorCode::UnknownCommand));
    }

    if handler.required_role == Role::Admin && role != Role::Admin {
        tracing::warn!(target: "rpc", method = handler.name, "RPC permission denied - admin required");
        return Err(Status::new(RpcErrorCode::NoPermission));
    }

    let condition = meets_condition(handler.condition, runtime);
    if !condition.is_ok() {
        return Err(condition);
    }

    Ok(handler)
}

pub fn role_required(api_version: u32, _beta_enabled: bool, method: &str) -> Role {
    // O(1) PHF lookup — replaces the former O(n) linear scan over HANDLERS.
    HANDLER_MAP
        .get(method)
        .filter(|handler| handler.supports_api(api_version))
        .map(|handler| handler.required_role)
        .unwrap_or(Role::Forbid)
}

pub fn method_from_params(params: &JsonValue) -> Result<&str, Status> {
    command_field(params)
}

#[cfg(test)]
mod tests {
    use super::{HandlerCondition, fill_handler, role_required};
    use crate::state::role::Role;
    use crate::{RpcErrorCode, RpcRuntime};
    use protocol::JsonValue;
    use std::cell::Cell;
    use std::collections::BTreeMap;

    fn object(entries: impl IntoIterator<Item = (&'static str, JsonValue)>) -> JsonValue {
        JsonValue::Object(
            entries
                .into_iter()
                .map(|(key, value)| (key.to_owned(), value))
                .collect::<BTreeMap<_, _>>(),
        )
    }

    #[derive(Default)]
    struct FakeRuntime {
        jobs: Cell<u32>,
        max_jobs: Cell<u32>,
        has_current_ledger: Cell<bool>,
        has_closed_ledger: Cell<bool>,
    }

    impl RpcRuntime for FakeRuntime {
        fn client_job_count(&self) -> u32 {
            self.jobs.get()
        }

        fn max_job_queue_clients(&self) -> u32 {
            self.max_jobs.get()
        }

        fn has_current_ledger(&self) -> bool {
            self.has_current_ledger.get()
        }

        fn has_closed_ledger(&self) -> bool {
            self.has_closed_ledger.get()
        }
    }

    #[test]
    fn server_definitions_is_registered() {
        let runtime = FakeRuntime {
            max_jobs: Cell::new(50),
            has_current_ledger: Cell::new(true),
            has_closed_ledger: Cell::new(true),
            ..FakeRuntime::default()
        };

        let handler = fill_handler(
            &object([(
                "command",
                JsonValue::String("server_definitions".to_owned()),
            )]),
            Role::User,
            2,
            &runtime,
        )
        .expect("server_definitions should resolve");

        assert_eq!(handler.name, "server_definitions");
        assert_eq!(handler.required_role, Role::User);
        assert_eq!(handler.condition, HandlerCondition::None);
        assert_eq!(role_required(2, false, "server_definitions"), Role::User);
    }

    #[test]
    fn server_definitions_uses_unknown_command_on_bad_version_or_payload() {
        let runtime = FakeRuntime {
            max_jobs: Cell::new(50),
            has_current_ledger: Cell::new(true),
            has_closed_ledger: Cell::new(true),
            ..FakeRuntime::default()
        };

        let error = fill_handler(
            &object([(
                "command",
                JsonValue::String("server_definitions".to_owned()),
            )]),
            Role::User,
            0,
            &runtime,
        )
        .expect_err("unsupported api version should fail");
        assert_eq!(error.error_code(), Some(RpcErrorCode::UnknownCommand));

        let error = fill_handler(
            &JsonValue::Object(BTreeMap::from([(
                "command".to_owned(),
                JsonValue::Unsigned(1),
            )])),
            Role::User,
            2,
            &runtime,
        )
        .expect_err("non-string command should fail");
        assert_eq!(error.error_code(), Some(RpcErrorCode::InvalidParams));
    }

    #[test]
    fn ripple_path_find_stays_user_visible_without_current_ledger_gate() {
        let runtime = FakeRuntime {
            max_jobs: Cell::new(50),
            has_current_ledger: Cell::new(false),
            has_closed_ledger: Cell::new(false),
            ..FakeRuntime::default()
        };

        let handler = fill_handler(
            &object([("command", JsonValue::String("ripple_path_find".to_owned()))]),
            Role::User,
            2,
            &runtime,
        )
        .expect("ripple_path_find should resolve without current-ledger gate");
        assert_eq!(handler.name, "ripple_path_find");
        assert_eq!(handler.required_role, Role::User);
        assert_eq!(handler.condition, HandlerCondition::None);
        assert_eq!(role_required(2, false, "ripple_path_find"), Role::User);
    }
}
