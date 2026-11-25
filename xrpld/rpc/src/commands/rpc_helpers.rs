//! RPC helper ports from `xrpld/rpc/detail/RPCHelpers.*`.

#![allow(dead_code)]

use app::ledger_to_json::ledger_to_json_tx::insert_all_synthetic_in_json;
use std::{collections::BTreeMap, sync::Arc};

use basics::{base_uint::Uint256, str_hex::str_hex, string_utilities::to_uint64};
use ledger::ReadView;
use protocol::tokens::decode_base58_token_multibyte;
use protocol::{
    JsonOptions, JsonValue, KeyType, LedgerEntryType, LedgerFormats, PublicKey, STArray, STObject,
    STParsedJSONObject, STTx, SecretKey, Seed, SerialIter, Serializer, SignatureRole, StBase,
    build_multi_signing_data_with_prefix, derive_public_key, generate_secret_key,
    get_field_by_name, get_field_by_symbol, is_tec_claim, is_tes_success, jss,
    parse_base58_account_id, serialize_pay_chan_authorization, sf_generic, sign, signature_field,
    signature_role, signing_prefix,
};

#[cfg(not(test))]
use crate::simulate::SimulateSource;
#[cfg(not(test))]
use crate::state::context::{RpcRequestContext, RpcRuntime};
use crate::state::role::{Role, is_unlimited};
use crate::state::tuning::LimitRange;
use crate::status::{RpcErrorCode, Status};
use crate::{insert_deliver_max, key_type_from_string};
#[cfg(test)]
use rpc::context::{RpcRequestContext, RpcRuntime};
#[cfg(test)]
use rpc::simulate::SimulateSource;

fn log_rpc_error(code: &RpcErrorCode) {
    if matches!(code, RpcErrorCode::ActNotFound) {
        tracing::debug!(target: "rpc", error = ?code, "RPC request returned expected not-found error");
    } else {
        tracing::warn!(target: "rpc", error = ?code, "RPC request failed");
    }
}

pub fn inject_error(code: RpcErrorCode, json: &mut JsonValue) {
    log_rpc_error(&code);
    Status::new(code).inject(json);
}

pub fn inject_error_message(code: RpcErrorCode, message: impl Into<String>, json: &mut JsonValue) {
    log_rpc_error(&code);
    Status::with_message(code, message).inject(json);
}

pub fn make_error(code: RpcErrorCode) -> JsonValue {
    let mut json = JsonValue::Object(BTreeMap::new());
    inject_error(code, &mut json);
    json
}

pub fn make_error_message(code: RpcErrorCode, message: impl Into<String>) -> JsonValue {
    let mut json = JsonValue::Object(BTreeMap::new());
    inject_error_message(code, message, &mut json);
    json
}

pub fn rpc_error(code: RpcErrorCode) -> JsonValue {
    make_error(code)
}

pub fn expected_field_message(name: impl AsRef<str>, ty: impl AsRef<str>) -> String {
    Status::expected_field_message(name, ty)
}

pub fn expected_field_error(name: impl AsRef<str>, ty: impl AsRef<str>) -> JsonValue {
    make_error_message(
        RpcErrorCode::InvalidParams,
        Status::expected_field_message(name, ty),
    )
}

pub fn object_field_error(name: impl AsRef<str>) -> JsonValue {
    expected_field_error(name, "object")
}

pub fn missing_field_message(name: impl AsRef<str>) -> String {
    Status::missing_field_message(name)
}

pub fn missing_field_error(name: impl AsRef<str>) -> JsonValue {
    make_error_message(
        RpcErrorCode::InvalidParams,
        Status::missing_field_message(name),
    )
}

pub fn invalid_field_message(name: impl AsRef<str>) -> String {
    Status::invalid_field_message(name)
}

pub fn invalid_field_error(name: impl AsRef<str>) -> JsonValue {
    make_error_message(
        RpcErrorCode::InvalidParams,
        Status::invalid_field_message(name),
    )
}

/// rippled `Tuning::kDefaultAutoFillFeeMultiplier` and `...Divisor`.
const DEFAULT_AUTO_FILL_FEE_MULTIPLIER: i64 = 10;
const DEFAULT_AUTO_FILL_FEE_DIVISOR: i64 = 1;

/// jsoncpp `Value::isInt`: an integer the JSON reader stores as `Int`,
/// i.e. one within the `i32` range.
fn json_int(value: &JsonValue) -> Option<i64> {
    match value {
        JsonValue::Unsigned(value) if *value <= i32::MAX as u64 => Some(*value as i64),
        JsonValue::Signed(value) if i32::try_from(*value).is_ok() => Some(*value),
        _ => None,
    }
}

/// jsoncpp `operator==`: `Int` and `UInt` compare by value.
fn json_equal(left: &JsonValue, right: &JsonValue) -> bool {
    match (left, right) {
        (JsonValue::Signed(signed), JsonValue::Unsigned(unsigned))
        | (JsonValue::Unsigned(unsigned), JsonValue::Signed(signed)) => {
            i128::from(*signed) == i128::from(*unsigned)
        }
        _ => left == right,
    }
}

/// jsoncpp `Value::asString` for the scalars a tx_json field may hold.
fn json_as_string(value: &JsonValue) -> Option<String> {
    match value {
        JsonValue::Null => Some(String::new()),
        JsonValue::Bool(value) => Some(value.to_string()),
        JsonValue::Unsigned(value) => Some(value.to_string()),
        JsonValue::Signed(value) => Some(value.to_string()),
        JsonValue::String(value) => Some(value.clone()),
        _ => None,
    }
}

/// The account root rippled reads from `OpenLedger::current()`; the closed
/// ledger before the first open view exists.
fn open_ledger_account_root(
    app: &app::ApplicationRoot,
    account: &protocol::AccountID,
) -> Option<protocol::STLedgerEntry> {
    let keylet = protocol::account_keylet(basics::base_uint::Uint160::from_void(account.data()));
    match app.current_open_ledger_entry(keylet) {
        Some(read) => read.ok().flatten(),
        None => app.closed_ledger()?.read(keylet).ok().flatten(),
    }
}

/// rippled `acctMatchesPubKey`: the key must be the account's enabled master
/// key or its regular key. Without an account root, only the master key.
fn acct_matches_pub_key(
    account_root: Option<&protocol::STLedgerEntry>,
    account: protocol::AccountID,
    public_key: &PublicKey,
) -> Result<(), Status> {
    let key_account = protocol::calc_account_id(public_key.as_bytes());
    let is_master_key = key_account == account;
    let Some(sle) = account_root else {
        return if is_master_key {
            Ok(())
        } else {
            Err(Status::new(RpcErrorCode::BadSecret))
        };
    };
    if is_master_key {
        let flags = sle.get_field_u32(get_field_by_symbol("sfFlags"));
        return if flags & protocol::lsfDisableMaster != 0 {
            Err(Status::new(RpcErrorCode::MasterDisabled))
        } else {
            Ok(())
        };
    }
    let regular_key = get_field_by_symbol("sfRegularKey");
    if sle.is_field_present(regular_key) && sle.get_account_id(regular_key) == key_account {
        return Ok(());
    }
    Err(Status::new(RpcErrorCode::BadSecret))
}

/// rippled `checkTxJsonFields`: validates without adding fields and returns
/// the source account.
fn check_tx_json_fields<Runtime: RpcRuntime, Source>(
    ctx: &RpcRequestContext<'_, Source, Runtime>,
    tx_json: &BTreeMap<String, JsonValue>,
    verify: bool,
) -> Result<protocol::AccountID, Status> {
    if !tx_json.contains_key(jss::TransactionType) {
        return Err(Status::missing_field_error("tx_json.TransactionType"));
    }
    let Some(account) = tx_json.get(jss::Account) else {
        return Err(Status::with_message(
            RpcErrorCode::SrcActMissing,
            missing_field_message("tx_json.Account"),
        ));
    };
    let account = json_as_string(account)
        .and_then(|account| parse_base58_account_id(&account))
        .ok_or_else(|| {
            Status::with_message(
                RpcErrorCode::SrcActMalformed,
                invalid_field_message("tx_json.Account"),
            )
        })?;
    if verify
        && !ctx.runtime.standalone()
        && ctx.runtime.validated_ledger_age()
            > crate::state::tuning::Tuning::MAX_VALIDATED_LEDGER_AGE
    {
        return Err(Status::new(if ctx.api_version == 1 {
            RpcErrorCode::NoCurrent
        } else {
            RpcErrorCode::NotSynced
        }));
    }
    if let Some(app) = ctx.runtime.app()
        && app.load_fee_track().is_loaded_cluster()
        && !is_unlimited(ctx.role)
    {
        return Err(Status::new(RpcErrorCode::TooBusy));
    }
    Ok(account)
}

/// rippled `getTxFee`: the base fee of the transaction as written, parsed
/// with placeholder Fee, Sequence and signature fields; the reference fee
/// when it does not parse or fails the local checks.
fn tx_fee_default(app: &app::ApplicationRoot, tx_json: &BTreeMap<String, JsonValue>) -> u64 {
    let reference = app.reference_fee_drops();
    let mut tx = tx_json.clone();
    for (field, placeholder) in [
        (jss::Fee, "0"),
        (jss::Sequence, "0"),
        (jss::SigningPubKey, ""),
        (jss::TxnSignature, ""),
    ] {
        tx.entry(field.to_owned())
            .or_insert_with(|| JsonValue::String(placeholder.to_owned()));
    }
    if let Some(signers) = tx.get_mut(jss::Signers) {
        let JsonValue::Array(signers) = signers else {
            return reference;
        };
        if signers.len() > protocol::STTx::MAX_MULTI_SIGNERS {
            return reference;
        }
        for signer in signers {
            let Some(JsonValue::Object(signer)) = (match signer {
                JsonValue::Object(outer) => outer.get_mut(jss::Signer),
                _ => None,
            }) else {
                return reference;
            };
            for field in [jss::SigningPubKey, jss::TxnSignature] {
                signer
                    .entry(field.to_owned())
                    .or_insert_with(|| JsonValue::String(String::new()));
            }
        }
    }
    let Some(object) = STParsedJSONObject::new("tx_json", &JsonValue::Object(tx)).object else {
        return reference;
    };
    let st_tx = STTx::from_stobject(object);
    if protocol::passes_local_checks(&st_tx).is_err() {
        return reference;
    }
    app.tx_base_fee_drops(&st_tx).unwrap_or(reference)
}

/// rippled `checkFee`: keep a given Fee, otherwise fill
/// `getCurrentNetworkFee`, limited to `fee_mult_max / fee_div_max` times the
/// transaction's base fee.
fn check_fee<Runtime: RpcRuntime, Source>(
    ctx: &RpcRequestContext<'_, Source, Runtime>,
    params: &BTreeMap<String, JsonValue>,
    tx_json: &mut BTreeMap<String, JsonValue>,
    auto_fill: bool,
) -> Result<(), Status> {
    if tx_json.contains_key(jss::Fee) {
        return Ok(());
    }
    if !auto_fill {
        return Err(Status::missing_field_error("tx_json.Fee"));
    }
    let limit_param = |name: &str, default: i64, min: i64| -> Result<i64, Status> {
        let Some(value) = params.get(name) else {
            return Ok(default);
        };
        let message = expected_field_message(name, "a positive integer");
        match json_int(value) {
            Some(value) if value >= min => Ok(value),
            Some(_) => Err(Status::with_message(RpcErrorCode::InvalidParams, message)),
            None => Err(Status::with_message(RpcErrorCode::HighFee, message)),
        }
    };
    let mult = limit_param(jss::fee_mult_max, DEFAULT_AUTO_FILL_FEE_MULTIPLIER, 0)?;
    let div = limit_param(jss::fee_div_max, DEFAULT_AUTO_FILL_FEE_DIVISOR, 1)?;

    let Some(app) = ctx.runtime.app() else {
        return Err(Status::new(RpcErrorCode::NotSynced));
    };
    let fee_default = tx_fee_default(app, tx_json);
    let fee = app.current_network_fee_for(fee_default, is_unlimited(ctx.role));
    let limit = u64::try_from(u128::from(fee_default) * mult as u128 / div as u128)
        .map_err(|_| Status::new(RpcErrorCode::Internal))?;
    if fee > limit {
        return Err(Status::with_message(
            RpcErrorCode::HighFee,
            format!("Fee of {fee} exceeds the requested tx limit of {limit}"),
        ));
    }
    // rippled writes `fee.jsonClipped()`, an integer; quaxar's amount parser
    // takes drops as a string.
    tx_json.insert(jss::Fee.to_owned(), JsonValue::String(fee.to_string()));
    Ok(())
}

/// rippled `checkPayment`, except the `build_path` search itself: the
/// request checks and errors match, but no paths are added (path finding is
/// not wired into signing yet).
fn check_payment<Runtime: RpcRuntime, Source>(
    ctx: &RpcRequestContext<'_, Source, Runtime>,
    params: &BTreeMap<String, JsonValue>,
    tx_json: &mut BTreeMap<String, JsonValue>,
    do_path: bool,
) -> Result<(), Status> {
    if tx_json
        .get(jss::TransactionType)
        .and_then(json_as_string)
        .as_deref()
        != Some("Payment")
    {
        return Ok(());
    }
    if let Some(deliver_max) = tx_json.remove(jss::DeliverMax) {
        match tx_json.get(jss::Amount) {
            Some(amount) if !json_equal(amount, &deliver_max) => {
                return Err(Status::make_param_error(
                    "Cannot specify differing 'Amount' and 'DeliverMax'",
                ));
            }
            Some(_) => {}
            None => {
                tx_json.insert(jss::Amount.to_owned(), deliver_max);
            }
        }
    }
    let parse_amount = |value: &JsonValue| {
        let field = get_field_by_symbol("sfAmount");
        protocol::get_or_throw::<protocol::STAmount>(
            &JsonValue::Object(BTreeMap::from([(field.name().to_owned(), value.clone())])),
            field,
        )
        .ok()
    };
    let amount = tx_json
        .get(jss::Amount)
        .ok_or_else(|| Status::missing_field_error("tx_json.Amount"))?;
    let amount =
        parse_amount(amount).ok_or_else(|| Status::invalid_field_error("tx_json.Amount"))?;
    let destination = tx_json
        .get(jss::Destination)
        .ok_or_else(|| Status::missing_field_error("tx_json.Destination"))?;
    json_as_string(destination)
        .and_then(|destination| parse_base58_account_id(&destination))
        .ok_or_else(|| Status::invalid_field_error("tx_json.Destination"))?;

    let build_path = params.contains_key(jss::build_path);
    let mpt_v2 = ctx.runtime.app().is_some_and(|app| {
        app.current_open_ledger_rules()
            .enabled(&protocol::feature_id("MPTokensV2"))
    });
    if build_path && (!do_path || (!mpt_v2 && amount.holds_mpt_issue())) {
        return Err(Status::make_param_error(
            "Field 'build_path' not allowed in this context.",
        ));
    }
    if build_path && tx_json.contains_key(jss::Paths) {
        return Err(Status::make_param_error(
            "Cannot specify both 'tx_json.Paths' and 'build_path'",
        ));
    }
    if let Some(domain) = tx_json.get("DomainID") {
        let valid = matches!(domain, JsonValue::String(text) if Uint256::from_hex(text).is_ok());
        if !valid {
            return Err(Status::with_message(
                RpcErrorCode::DomainMalformed,
                "Unable to parse 'DomainID'.",
            ));
        }
    }
    if build_path && !tx_json.contains_key(jss::Paths) {
        let send_max_native = match tx_json.get(jss::SendMax) {
            Some(send_max) => parse_amount(send_max)
                .ok_or_else(|| Status::invalid_field_error("tx_json.SendMax"))?
                .native(),
            None => amount.native(),
        };
        if send_max_native && amount.native() {
            return Err(Status::make_param_error("Cannot build XRP to XRP paths."));
        }
    }
    Ok(())
}

/// rippled `transactionPreProcessImpl` for single signing (`sign`, and
/// `submit` with a secret): check and fill tx_json, check that the key may
/// sign for the account, then build and sign the transaction.
fn transaction_pre_process<Runtime: RpcRuntime, Source>(
    ctx: &RpcRequestContext<'_, Source, Runtime>,
    params: &BTreeMap<String, JsonValue>,
) -> Result<STTx, Status> {
    let (public_key, secret_key) = keypair_for_signature(params, ctx.api_version)?;
    let verify = !params.get(jss::offline).is_some_and(json_as_bool);
    let signature_target = parse_signature_target(params)?;
    let signature_role = signature_target.unwrap_or(SignatureRole::Transaction);

    let Some(tx_json) = params.get(jss::tx_json) else {
        return Err(Status::missing_field_error(jss::tx_json));
    };
    let JsonValue::Object(tx_json) = tx_json else {
        return Err(Status::expected_field_error(jss::tx_json, "object"));
    };
    let mut tx_json = tx_json.clone();
    let source = check_tx_json_fields(ctx, &tx_json, verify)?;

    // Offline, the Sequence cannot be read locally.
    if !verify && !tx_json.contains_key(jss::Sequence) {
        return Err(Status::missing_field_error("tx_json.Sequence"));
    }
    let app = ctx.runtime.app();
    let account_root = if verify {
        let root = app.and_then(|app| open_ledger_account_root(app, &source));
        if root.is_none() {
            return Err(Status::new(RpcErrorCode::SrcActNotFound));
        }
        root
    } else {
        None
    };

    if !tx_json.contains_key(jss::Sequence) {
        let seq = if tx_json.contains_key("TicketSequence") {
            0
        } else {
            // TxQ::nextQueuableSeq on the open ledger.
            let probe = STTx::new(protocol::TxType::PAYMENT, |tx| {
                tx.set_account_id(get_field_by_symbol("sfAccount"), source);
            });
            app.and_then(|app| app.network_ops_next_account_seq_for_tx(&probe))
                .ok_or_else(|| Status::new(RpcErrorCode::SrcActNotFound))?
        };
        tx_json.insert(
            jss::Sequence.to_owned(),
            JsonValue::Unsigned(u64::from(seq)),
        );
    }
    if !tx_json.contains_key(jss::NetworkID) {
        let network_id = app.map(|app| app.network_id()).unwrap_or(0);
        if network_id > 1024 {
            tx_json.insert(
                jss::NetworkID.to_owned(),
                JsonValue::String(network_id.to_string()),
            );
        }
    }
    check_fee(ctx, params, &mut tx_json, verify)?;
    check_payment(ctx, params, &mut tx_json, verify)?;

    if tx_json.contains_key(jss::Signers) {
        return Err(Status::new(RpcErrorCode::AlreadyMultisig));
    }
    // With a signature target the account and secret need not belong together.
    if verify && signature_target.is_none() {
        if let Some(delegate) = tx_json.get("Delegate") {
            let delegate = match delegate {
                JsonValue::String(text) => parse_base58_account_id(text),
                _ => None,
            }
            .ok_or_else(|| {
                Status::with_message(
                    RpcErrorCode::SrcActMalformed,
                    invalid_field_message("tx_json.Delegate"),
                )
            })?;
            let delegate_root = app.and_then(|app| open_ledger_account_root(app, &delegate));
            let Some(delegate_root) = delegate_root else {
                return Err(Status::new(RpcErrorCode::DelegateActNotFound));
            };
            acct_matches_pub_key(Some(&delegate_root), delegate, &public_key)?;
        } else {
            acct_matches_pub_key(account_root.as_ref(), source, &public_key)?;
        }
    }

    let parsed = STParsedJSONObject::new("tx_json", &JsonValue::Object(tx_json));
    let Some(object) = parsed.object else {
        let mut status = Status::new(RpcErrorCode::InvalidParams);
        if let JsonValue::Object(mut error) = parsed.error
            && let Some(JsonValue::String(message)) = error.remove("error_message")
        {
            status = Status::with_message(RpcErrorCode::InvalidParams, message);
        }
        return Err(status);
    };
    let mut st_tx = STTx::from_stobject(object);
    let target_field = signature_field(signature_role);
    if let Some(field) = target_field
        && !st_tx.is_field_present(field)
    {
        st_tx.set_field_object(field, STObject::make_inner_object(field));
    }
    signer_target_object(&mut st_tx, target_field).set_field_vl(
        get_field_by_symbol("sfSigningPubKey"),
        public_key.as_bytes(),
    );
    protocol::passes_local_checks(&st_tx).map_err(Status::make_param_error)?;
    st_tx
        .sign_with_role(
            &public_key,
            &secret_key,
            signature_role,
            &signing_rules(ctx),
        )
        .map_err(|_| Status::new(RpcErrorCode::Internal))?;
    Ok(st_tx)
}

/// rippled `doSign`, `doSignFor` and `doSubmit` with a secret: only admin
/// callers may sign unless `[signing_support]` is set (`Config::canSign`).
pub fn check_signing_supported<Runtime: RpcRuntime, Source>(
    ctx: &RpcRequestContext<'_, Source, Runtime>,
) -> Result<(), Status> {
    if ctx.role != Role::Admin && !ctx.runtime.app().is_some_and(|app| app.can_sign()) {
        return Err(Status::with_message(
            RpcErrorCode::NotSupported,
            "Signing is not supported by this server.",
        ));
    }
    Ok(())
}

/// rippled `transactionSign`.
pub fn transaction_sign<Runtime: RpcRuntime, Source>(
    ctx: &RpcRequestContext<'_, Source, Runtime>,
) -> Result<JsonValue, Status> {
    tracing::debug!(target: "rpc", method = "sign", "RPC request received");
    let JsonValue::Object(params) = &ctx.params else {
        return Err(Status::new(RpcErrorCode::InvalidParams));
    };
    let st_tx = transaction_pre_process(ctx, params)?;
    Ok(JsonValue::Object(transaction_format_result(
        &st_tx,
        ctx.api_version,
    )))
}

pub fn transaction_sign_for<Runtime: RpcRuntime, Source>(
    ctx: &RpcRequestContext<'_, Source, Runtime>,
) -> Result<JsonValue, Status> {
    tracing::debug!(target: "rpc", method = "sign_for", "RPC request received");
    let JsonValue::Object(params) = &ctx.params else {
        return Err(Status::new(RpcErrorCode::InvalidParams));
    };

    let signer_account = params
        .get("account")
        .and_then(JsonValue::as_str)
        .ok_or_else(|| {
            Status::with_message(
                RpcErrorCode::InvalidParams,
                missing_field_message("account"),
            )
        })?;
    let signer_account_id = parse_base58_account_id(signer_account).ok_or_else(|| {
        Status::with_message(
            RpcErrorCode::InvalidParams,
            invalid_field_message("account"),
        )
    })?;

    let mut tx_json = params.get(jss::tx_json).cloned().ok_or_else(|| {
        Status::with_message(
            RpcErrorCode::InvalidParams,
            missing_field_message(jss::tx_json),
        )
    })?;
    let JsonValue::Object(ref mut tx_object) = tx_json else {
        return Err(Status::with_message(
            RpcErrorCode::InvalidParams,
            expected_field_message(jss::tx_json, "object"),
        ));
    };
    let app_network_id = ctx.runtime.app().map(|app| app.network_id()).unwrap_or(0);
    // Auto-fill NetworkID for private networks (ID > 1024), matching rippled's autofill
    if app_network_id > 1024 && !tx_object.contains_key(jss::NetworkID) {
        tx_object.insert(
            jss::NetworkID.to_owned(),
            JsonValue::Unsigned(u64::from(app_network_id)),
        );
    }
    check_transaction_sign_for_network_id(tx_object, app_network_id)?;

    tx_object
        .entry(jss::SigningPubKey.to_owned())
        .or_insert_with(|| JsonValue::String(String::new()));

    if !tx_object.contains_key(jss::Sequence) {
        return Err(Status::with_message(
            RpcErrorCode::InvalidParams,
            missing_field_message("tx_json.Sequence"),
        ));
    }
    if !params.contains_key(jss::signature_target)
        && tx_object
            .get(jss::SigningPubKey)
            .and_then(JsonValue::as_str)
            .is_some_and(|value| !value.is_empty())
    {
        return Err(Status::with_message(
            RpcErrorCode::InvalidParams,
            "When multi-signing 'tx_json.SigningPubKey' must be empty.",
        ));
    }

    let mut st_tx = parse_sttx_from_json_value(&tx_json)?;
    let (public_key, secret_key) = keypair_for_signature(params, ctx.api_version)?;
    let signing_rules = signing_rules(ctx);
    let signature_role = parse_signature_target(params)?.unwrap_or(SignatureRole::Transaction);

    let signing_data = build_multi_signing_data_with_prefix(
        &st_tx.clone_as_object(),
        signer_account_id,
        signing_prefix(signature_role, true, &signing_rules),
    );
    let signature = sign(&public_key, &secret_key, signing_data.data())
        .map_err(|_| Status::new(RpcErrorCode::Internal))?;

    let signing_for_id = st_tx.get_initiator();
    let mut signers = signer_target_object(&mut st_tx, signature_field(signature_role))
        .get_field_array(get_field_by_symbol("sfSigners"))
        .iter()
        .cloned()
        .collect::<Vec<_>>();

    let mut signer = STObject::make_inner_object(get_field_by_symbol("sfSigner"));
    signer.set_account_id(get_field_by_symbol("sfAccount"), signer_account_id);
    signer.set_field_vl(
        get_field_by_symbol("sfSigningPubKey"),
        public_key.as_bytes(),
    );
    signer.set_field_vl(get_field_by_symbol("sfTxnSignature"), &signature);
    signers.push(signer);
    signers.sort_by_key(|entry| entry.get_account_id(get_field_by_symbol("sfAccount")));

    if signers.windows(2).any(|pair| {
        pair[0].get_account_id(get_field_by_symbol("sfAccount"))
            == pair[1].get_account_id(get_field_by_symbol("sfAccount"))
    }) {
        return Err(Status::with_message(
            RpcErrorCode::InvalidParams,
            "Duplicate Signers:Signer:Account entries are not allowed.",
        ));
    }
    if signers
        .iter()
        .any(|entry| entry.get_account_id(get_field_by_symbol("sfAccount")) == signing_for_id)
    {
        return Err(Status::with_message(
            RpcErrorCode::InvalidParams,
            format!(
                "A Signer may not be the transaction's Account ({}).",
                protocol::to_base58(signing_for_id)
            ),
        ));
    }

    let mut signers_array = STArray::new(get_field_by_symbol("sfSigners"));
    for signer in signers {
        signers_array.push_back(signer);
    }
    signer_target_object(&mut st_tx, signature_field(signature_role))
        .set_field_array(get_field_by_symbol("sfSigners"), signers_array);

    let mut result = transaction_format_result(&st_tx, ctx.api_version);
    result.insert(
        jss::deprecated.to_string(),
        JsonValue::String(
            "This command has been deprecated and will be removed in a future version of the server. Please migrate to a standalone signing tool.".to_owned(),
        ),
    );
    Ok(JsonValue::Object(result))
}

fn check_transaction_sign_for_network_id(
    tx_object: &BTreeMap<String, JsonValue>,
    app_network_id: u32,
) -> Result<(), Status> {
    if app_network_id <= 1024 {
        return Ok(());
    }

    let Some(network_id) = tx_object.get(jss::NetworkID) else {
        return Err(Status::with_message(
            RpcErrorCode::InvalidParams,
            missing_field_message("tx_json.NetworkID"),
        ));
    };

    if network_id.as_u64() != Some(u64::from(app_network_id)) {
        return Err(Status::with_message(
            RpcErrorCode::InvalidParams,
            invalid_field_message("tx_json.NetworkID"),
        ));
    }

    Ok(())
}

#[cfg(test)]
mod transaction_sign_for_network_id_tests {
    use super::*;

    #[test]
    fn network_id_is_required_and_must_match_for_networks_above_1024() {
        let empty = BTreeMap::new();
        assert_eq!(
            check_transaction_sign_for_network_id(&empty, 21338),
            Err(Status::with_message(
                RpcErrorCode::InvalidParams,
                missing_field_message("tx_json.NetworkID")
            ))
        );

        let wrong = BTreeMap::from([(jss::NetworkID.to_owned(), JsonValue::Unsigned(21337))]);
        assert_eq!(
            check_transaction_sign_for_network_id(&wrong, 21338),
            Err(Status::with_message(
                RpcErrorCode::InvalidParams,
                invalid_field_message("tx_json.NetworkID")
            ))
        );

        let correct = BTreeMap::from([(jss::NetworkID.to_owned(), JsonValue::Unsigned(21338))]);
        assert!(check_transaction_sign_for_network_id(&correct, 21338).is_ok());
    }
}

pub fn get_tx_json_from_params(params: &JsonValue) -> Result<JsonValue, Status> {
    let JsonValue::Object(map) = params else {
        return Err(Status::new(RpcErrorCode::InvalidParams));
    };

    if let Some(blob) = map.get(jss::tx_blob) {
        if map.contains_key(jss::tx_json) {
            return Err(Status::with_message(
                RpcErrorCode::InvalidParams,
                "Can only include one of `tx_blob` and `tx_json`.",
            ));
        }
        let JsonValue::String(hex_str) = blob else {
            return Err(Status::new(RpcErrorCode::InvalidParams));
        };
        let bytes = hex::decode(hex_str).map_err(|_| Status::new(RpcErrorCode::InvalidParams))?;
        let mut iter = SerialIter::new(&bytes);
        let obj = STObject::from_serial_iter(&mut iter, sf_generic(), 0);
        Ok(obj.json(JsonOptions::new(0)))
    } else if let Some(tx_json) = map.get(jss::tx_json) {
        if !matches!(tx_json, JsonValue::Object(_)) {
            return Err(Status::new(RpcErrorCode::InvalidParams));
        }
        Ok(tx_json.clone())
    } else {
        Err(Status::with_message(
            RpcErrorCode::InvalidParams,
            "Neither `tx_blob` nor `tx_json` included.",
        ))
    }
}

pub fn parse_sttx_from_params(params: &JsonValue) -> Result<STTx, Status> {
    let JsonValue::Object(map) = params else {
        return Err(Status::new(RpcErrorCode::InvalidParams));
    };

    if map.contains_key(jss::tx_blob) && map.contains_key(jss::tx_json) {
        return Err(Status::with_message(
            RpcErrorCode::InvalidParams,
            "Can only include one of `tx_blob` and `tx_json`.",
        ));
    }

    if let Some(blob) = map.get(jss::tx_blob) {
        let JsonValue::String(hex_str) = blob else {
            return Err(Status::new(RpcErrorCode::InvalidParams));
        };
        let bytes = hex::decode(hex_str).map_err(|_| Status::new(RpcErrorCode::InvalidParams))?;
        let mut iter = SerialIter::new(&bytes);
        return std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            STTx::from_serial_iter(&mut iter)
        }))
        .map_err(|_| Status::new(RpcErrorCode::InvalidParams));
    }

    parse_sttx_from_json_value(&get_tx_json_from_params(params)?)
}

pub fn channel_authorize<Runtime: RpcRuntime, Source>(
    ctx: &RpcRequestContext<'_, Source, Runtime>,
) -> Result<JsonValue, Status> {
    tracing::debug!(target: "rpc", method = "channel_authorize", "RPC request received");
    let JsonValue::Object(params) = &ctx.params else {
        return Err(Status::new(RpcErrorCode::InvalidParams));
    };

    if !params.contains_key(jss::channel_id) {
        return Err(Status::with_message(
            RpcErrorCode::InvalidParams,
            missing_field_message(jss::channel_id),
        ));
    }
    if !params.contains_key(jss::amount) {
        return Err(Status::with_message(
            RpcErrorCode::InvalidParams,
            missing_field_message(jss::amount),
        ));
    }
    if !params.contains_key(jss::key_type) && !params.contains_key(jss::secret) {
        return Err(Status::with_message(
            RpcErrorCode::InvalidParams,
            missing_field_message(jss::secret),
        ));
    }

    let channel_id = params
        .get(jss::channel_id)
        .and_then(JsonValue::as_str)
        .ok_or_else(|| Status::new(RpcErrorCode::ChannelMalformed))?;
    let amount = params
        .get(jss::amount)
        .and_then(JsonValue::as_str)
        .ok_or_else(|| Status::new(RpcErrorCode::ChannelAmtMalformed))?;

    let (public_key, secret_key) = keypair_for_signature(params, ctx.api_version)?;
    let channel_id =
        Uint256::from_hex(channel_id).map_err(|_| Status::new(RpcErrorCode::ChannelMalformed))?;
    let amount = to_uint64(amount).ok_or_else(|| Status::new(RpcErrorCode::ChannelAmtMalformed))?;
    let signature = sign(
        &public_key,
        &secret_key,
        &serialize_pay_chan_authorization(&channel_id, amount),
    )
    .map_err(|_| Status::new(RpcErrorCode::Internal))?;

    let mut ret = BTreeMap::new();
    ret.insert(
        jss::signature.to_string(),
        JsonValue::String(str_hex(&signature)),
    );
    Ok(JsonValue::Object(ret))
}

/// rippled Simulate.cpp `autofillSignature`: empty `SigningPubKey` and
/// `TxnSignature` (also per multisig signer); a signed transaction is
/// rejected with `transactionSigned`.
fn autofill_signature(
    sig_object: &mut BTreeMap<String, JsonValue>,
    field_prefix: &str,
) -> Result<(), Status> {
    sig_object
        .entry(jss::SigningPubKey.to_owned())
        .or_insert_with(|| JsonValue::String(String::new()));

    if let Some(signers) = sig_object.get_mut(jss::Signers) {
        let JsonValue::Array(signers) = signers else {
            return Err(Status::invalid_field_error(format!(
                "{field_prefix}.Signers"
            )));
        };
        for (index, signer) in signers.iter_mut().enumerate() {
            let Some(JsonValue::Object(signer)) = (match signer {
                JsonValue::Object(outer) => outer.get_mut(jss::Signer),
                _ => None,
            }) else {
                return Err(Status::invalid_field_error(format!(
                    "{field_prefix}.Signers[{index}]"
                )));
            };
            signer
                .entry(jss::SigningPubKey.to_owned())
                .or_insert_with(|| JsonValue::String(String::new()));
            check_unsigned(signer)?;
        }
    }

    check_unsigned(sig_object)
}

fn check_unsigned(sig_object: &mut BTreeMap<String, JsonValue>) -> Result<(), Status> {
    match sig_object.get(jss::TxnSignature) {
        None => {
            sig_object.insert(
                jss::TxnSignature.to_owned(),
                JsonValue::String(String::new()),
            );
            Ok(())
        }
        Some(JsonValue::String(signature)) if signature.is_empty() => Ok(()),
        Some(_) => Err(Status::new(RpcErrorCode::TxSigned)),
    }
}

/// rippled Simulate.cpp `autofillTx`: signature fields, `Sequence`
/// (`TxQ::nextQueuableSeq` on the open ledger, 0 with a ticket),
/// `NetworkID` above 1024, then `Fee` (`getCurrentNetworkFee`, capped at 10x
/// the transaction's base fee).
pub fn autofill_tx<Runtime: RpcRuntime>(
    tx_json: &mut protocol::JsonValue,
    ctx: &RpcRequestContext<'_, SimulateSource, Runtime>,
) -> Result<(), Status> {
    let JsonValue::Object(map) = tx_json else {
        return Ok(());
    };
    autofill_signature(map, "tx")?;
    if let Some(sponsor) = map.get_mut("SponsorSignature") {
        let JsonValue::Object(sponsor) = sponsor else {
            return Err(Status::expected_field_error("SponsorSignature", "object"));
        };
        autofill_signature(sponsor, "tx.SponsorSignature")?;
    }

    let app = ctx.runtime.app();
    if !map.contains_key(jss::Sequence) {
        let account = match map.get(jss::Account) {
            Some(JsonValue::String(account)) => {
                parse_base58_account_id(account).ok_or_else(|| {
                    Status::with_message(
                        RpcErrorCode::SrcActMalformed,
                        invalid_field_message("tx.Account"),
                    )
                })?
            }
            _ => return Err(Status::invalid_field_error("tx.Account")),
        };
        let seq = if map.contains_key("TicketSequence") {
            0
        } else {
            match app {
                Some(app) => {
                    let probe = STTx::new(protocol::TxType::PAYMENT, |tx| {
                        tx.set_account_id(get_field_by_symbol("sfAccount"), account);
                    });
                    app.network_ops_next_account_seq_for_tx(&probe)
                        .ok_or_else(|| Status::new(RpcErrorCode::SrcActNotFound))?
                }
                None => 1,
            }
        };
        map.insert(
            jss::Sequence.to_owned(),
            JsonValue::Unsigned(u64::from(seq)),
        );
    }

    if !map.contains_key(jss::NetworkID) {
        let network_id = app.map(|app| app.network_id()).unwrap_or(0);
        if network_id > 1024 {
            map.insert(
                jss::NetworkID.to_owned(),
                JsonValue::String(network_id.to_string()),
            );
        }
    }

    if !map.contains_key(jss::Fee) {
        let fee = match app {
            Some(app) => {
                // rippled getTxFee parses a copy with Fee 0 so the fee
                // estimator sees the full transaction shape.
                let mut probe = map.clone();
                probe.insert(jss::Fee.to_owned(), JsonValue::String("0".to_owned()));
                let (fee, fee_default) = match parse_sttx_from_json_value(&JsonValue::Object(probe))
                {
                    Ok(probe) => app.current_network_fee_drops(&probe, is_unlimited(ctx.role)),
                    Err(_) => {
                        let reference = app.open_ledger().current().base_fee_drops;
                        let fee = app
                            .current_network_fee_drops(
                                &STTx::new(protocol::TxType::PAYMENT, |_| {}),
                                is_unlimited(ctx.role),
                            )
                            .0;
                        (fee, reference)
                    }
                };
                let limit = fee_default.saturating_mul(10);
                if fee > limit {
                    return Err(Status::with_message(
                        RpcErrorCode::HighFee,
                        format!("Fee of {fee} exceeds the requested tx limit of {limit}"),
                    ));
                }
                fee
            }
            None => 10,
        };
        map.insert(jss::Fee.to_owned(), JsonValue::String(fee.to_string()));
    }
    Ok(())
}

pub fn simulate_txn<Runtime: RpcRuntime>(
    ctx: &RpcRequestContext<'_, SimulateSource, Runtime>,
    tx: &STTx,
) -> Result<JsonValue, Status> {
    tracing::debug!(target: "rpc", method = "simulate", "RPC request received");
    let mut ret = BTreeMap::new();
    let binary = ctx
        .params
        .get(jss::binary)
        .and_then(|value| match value {
            JsonValue::Bool(value) => Some(*value),
            _ => None,
        })
        .unwrap_or(false);
    let mut simulation_meta_blob = None;
    ret.insert(jss::applied.to_string(), JsonValue::Bool(false));

    // The real application owns TxQ and its open-ledger/fee state. Route
    // simulation through that canonical admission boundary instead of calling
    // the transactor shell directly. Parity: ../rippled/src/xrpld/rpc/handlers/
    // transaction/Simulate.cpp::simulateTxn invokes TxQ::apply(TapDryRun).
    if let Some(app) = ctx.runtime.app() {
        // ApplicationRoot::simulate_transaction clones its persistent OpenView
        // sandbox. The runtime ledger is only the immutable fallback for a
        // node that has not accepted any open-ledger mutations yet; rebuilding
        // `Ledger::from_previous` here would discard sequence/balance changes.
        // Parity: ../rippled/src/xrpld/rpc/handlers/transaction/Simulate.cpp::
        // simulateTxn copies OpenLedger::current() before TxQ::apply(TapDryRun).
        let ledger = ctx
            .runtime
            .current_ledger_for_simulation()
            .ok_or_else(|| Status::new(RpcErrorCode::NotSynced))?;
        let outcome = app.simulate_transaction(ledger, Arc::new(tx.clone()));
        ret.insert(
            jss::applied.to_string(),
            JsonValue::Bool(outcome.result.applied),
        );
        ret.insert(
            jss::engine_result.to_string(),
            JsonValue::String(protocol::trans_token(outcome.result.ter).to_owned()),
        );
        ret.insert(
            jss::engine_result_code.to_string(),
            JsonValue::Signed(outcome.result.ter.to_int() as i64),
        );
        ret.insert(
            "engine_result_message".to_string(),
            JsonValue::String(protocol::trans_human(outcome.result.ter).to_owned()),
        );
        ret.insert(
            jss::ledger_index.to_string(),
            JsonValue::Unsigned(u64::from(outcome.ledger_seq)),
        );

        if let Some(mut metadata) = outcome.metadata {
            if binary {
                let mut serializer = Serializer::default();
                metadata.add_raw(&mut serializer, outcome.result.ter, 0);
                ret.insert(
                    "meta_blob".to_string(),
                    JsonValue::String(hex::encode(serializer.data())),
                );
            } else {
                let mut meta = metadata.get_json(JsonOptions::new(0));
                insert_all_synthetic_in_json(
                    &mut meta,
                    outcome.ledger_seq,
                    Some(outcome.close_time),
                    tx,
                    &metadata,
                );
                ret.insert("meta".to_string(), meta);
            }
        } else {
            ret.insert(
                "meta".to_string(),
                JsonValue::Object(BTreeMap::from([
                    ("AffectedNodes".to_owned(), JsonValue::Array(Vec::new())),
                    (
                        "TransactionResult".to_owned(),
                        JsonValue::String(protocol::trans_token(outcome.result.ter).to_owned()),
                    ),
                ])),
            );
        }

        if binary {
            ret.insert(
                jss::tx_blob.to_string(),
                JsonValue::String(hex::encode(tx.get_serializer().data())),
            );
        } else {
            ret.insert(jss::tx_json.to_string(), tx.json(JsonOptions::new(0)));
        }
        return Ok(JsonValue::Object(ret));
    }

    // Keep the generic fallback for lightweight RPC runtimes that do not own
    // an ApplicationRoot/TxQ (unit harnesses and isolated handler tests).
    if let Some(ledger) = ctx.runtime.current_ledger_for_simulation() {
        let ledger_seq = ledger.header().seq;
        let close_time = ledger.header().close_time;
        let mut parent = ledger::ApplyViewImpl::new(Arc::clone(&ledger), tx::ApplyFlags::NONE);
        let rules = parent.rules();
        let mut view = ledger::FlowSandbox::new_with_flags(&mut parent, tx::ApplyFlags::DRY_RUN);
        // Minimal RPC runtimes have no ApplicationRoot/TxQ owner. Keep their
        // fallback on the app-level canonical dry-run boundary.
        let (result, delivered_amount) =
            match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                app::apply_simulated_transaction(&mut view, tx)
            })) {
                Ok((result, delivered_amount)) => (result, delivered_amount),
                Err(_) => {
                    ret.insert(
                        jss::engine_result.to_string(),
                        JsonValue::String("telLOCAL_ERROR".to_string()),
                    );
                    ret.insert(jss::engine_result_code.to_string(), JsonValue::Signed(-399));
                    return Ok(JsonValue::Object(ret));
                }
            };

        ret.insert(
            jss::engine_result.to_string(),
            JsonValue::String(protocol::trans_token(result).to_owned()),
        );
        ret.insert(
            jss::engine_result_code.to_string(),
            JsonValue::Signed(result.to_int() as i64),
        );
        ret.insert(
            "engine_result_message".to_string(),
            JsonValue::String(protocol::trans_human(result).to_string()),
        );
        ret.insert(
            jss::ledger_index.to_string(),
            JsonValue::Unsigned(u64::from(ledger_seq)),
        );

        // Build one typed metadata object from the actual dry-run changes.
        // JSON and binary simulation responses must render the same metadata.
        if is_tes_success(result) || protocol::is_tec_claim(result) {
            let mut transaction_meta = view
                .to_tx_meta(
                    tx.get_transaction_id(),
                    ledger_seq,
                    delivered_amount,
                    &rules,
                )
                .map_err(|error| {
                    Status::with_message(
                        RpcErrorCode::Internal,
                        format!("simulation metadata failed: {error:?}"),
                    )
                })?;
            let mut serializer = Serializer::default();
            transaction_meta.add_raw(&mut serializer, result, 0);

            let mut meta = transaction_meta.get_json(JsonOptions::new(0));
            insert_all_synthetic_in_json(
                &mut meta,
                ledger_seq,
                Some(close_time),
                tx,
                &transaction_meta,
            );
            simulation_meta_blob = Some(hex::encode(serializer.data()));
            ret.insert("meta".to_string(), meta);
        } else {
            let mut meta = BTreeMap::new();
            meta.insert("AffectedNodes".to_owned(), JsonValue::Array(Vec::new()));
            meta.insert(
                "TransactionResult".to_owned(),
                JsonValue::String(format!("{:?}", result)),
            );
            ret.insert("meta".to_string(), JsonValue::Object(meta));
        }
    } else {
        ret.insert(
            jss::engine_result.to_string(),
            JsonValue::String("tesSUCCESS".to_string()),
        );
        ret.insert(jss::engine_result_code.to_string(), JsonValue::Signed(0));
        ret.insert(
            "engine_result_message".to_string(),
            JsonValue::String(
                "The transaction was applied. Only final in a validated ledger.".to_string(),
            ),
        );
    }

    if !binary {
        ret.insert(jss::tx_json.to_string(), tx.json(JsonOptions::new(0)));
    } else {
        ret.insert(
            jss::tx_blob.to_string(),
            JsonValue::String(hex::encode(tx.get_serializer().data())),
        );
        if let Some(meta_blob) = simulation_meta_blob {
            ret.insert("meta_blob".to_string(), JsonValue::String(meta_blob));
        }
    }

    Ok(JsonValue::Object(ret))
}

/// Decode only xrpl.js's legacy Ed25519 seed encoding.
///
/// rippled's `parseXrplLibSeed` accepts the raw Base58 payload prefix E1 4B,
/// followed by exactly 16 seed bytes. A normal XRPL family seed must not be
/// accepted here: it falls through to normal seed handling and therefore
/// retains the default Secp256k1 key type.
pub fn parse_xrpl_lib_seed(s: &str) -> Option<Seed> {
    decode_base58_token_multibyte(s, &[0xE1, 0x4B])
        .filter(|bytes| bytes.len() == 16)
        .and_then(|bytes| Seed::from_slice(&bytes).ok())
}

/// rippled `getSeedFromRPC`: exactly one of `passphrase`, `seed` (a Base58
/// family seed) or `seed_hex` (32 hex digits).
pub fn get_seed_from_rpc(params: &JsonValue) -> Result<Seed, Status> {
    let JsonValue::Object(map) = params else {
        return Err(Status::new(RpcErrorCode::InvalidParams));
    };
    let present: Vec<&str> = [jss::passphrase, jss::seed, jss::seed_hex]
        .into_iter()
        .filter(|name| map.contains_key(*name))
        .collect();
    let [seed_type] = present[..] else {
        return Err(Status::make_param_error(
            "Exactly one of the following must be specified: passphrase, seed or seed_hex",
        ));
    };
    let Some(JsonValue::String(text)) = map.get(seed_type) else {
        return Err(Status::expected_field_error(seed_type, "string"));
    };
    let seed = if seed_type == jss::passphrase {
        protocol::parse_generic_seed(text, true)
    } else if seed_type == jss::seed {
        protocol::decode_base58_token(text, protocol::TokenType::FamilySeed)
            .and_then(|bytes| Seed::from_slice(&bytes).ok())
    } else {
        // UInt128::parseHex: 32 hex digits, or "0".
        basics::base_uint::Uint128::from_hex(text)
            .ok()
            .and_then(|value| Seed::from_slice(value.data()).ok())
    };
    seed.ok_or_else(|| Status::new(RpcErrorCode::BadSeed))
}

/// rippled `json::Value::asBool`: rpc-spec reads boolean options this way,
/// so any non-empty string, non-zero number or non-empty container is true.
pub fn json_as_bool(value: &JsonValue) -> bool {
    match value {
        JsonValue::Bool(value) => *value,
        JsonValue::Signed(value) => *value != 0,
        JsonValue::Unsigned(value) => *value != 0,
        JsonValue::String(value) => !value.is_empty(),
        JsonValue::Array(values) => !values.is_empty(),
        JsonValue::Object(values) => !values.is_empty(),
        // Raw holds pre-serialized server output; it never appears in a request.
        JsonValue::Raw(_) | JsonValue::Null => false,
    }
}

pub fn read_limit_field(params: &JsonValue, role: Role, range: LimitRange) -> Result<u32, Status> {
    let JsonValue::Object(object) = params else {
        return Ok(range.r_default);
    };

    let Some(limit_value) = object.get("limit") else {
        return Ok(range.r_default);
    };

    if matches!(limit_value, JsonValue::Null) {
        return Ok(range.r_default);
    }

    let mut limit = match limit_value {
        JsonValue::Unsigned(value) => u32::try_from(*value)
            .map_err(|_| Status::expected_field_error("limit", "unsigned integer"))?,
        JsonValue::Signed(value) if *value >= 0 => u32::try_from(*value as u64)
            .map_err(|_| Status::expected_field_error("limit", "unsigned integer"))?,
        _ => return Err(Status::expected_field_error("limit", "unsigned integer")),
    };

    if limit == 0 {
        return Err(Status::invalid_field_error("limit"));
    }

    if !is_unlimited(role) {
        limit = limit.clamp(range.rmin, range.rmax);
    }

    Ok(limit)
}

pub fn read_limit_field_with_cap(
    params: &JsonValue,
    role: Role,
    default_limit: u32,
    cap: u32,
) -> Result<u32, Status> {
    let JsonValue::Object(object) = params else {
        return Ok(default_limit);
    };

    let Some(limit_value) = object.get("limit") else {
        return Ok(default_limit);
    };

    if matches!(limit_value, JsonValue::Null) {
        return Ok(default_limit);
    }

    let mut limit = match limit_value {
        JsonValue::Unsigned(value) => u32::try_from(*value)
            .map_err(|_| Status::expected_field_error("limit", "unsigned integer"))?,
        JsonValue::Signed(value) if *value >= 0 => u32::try_from(*value as u64)
            .map_err(|_| Status::expected_field_error("limit", "unsigned integer"))?,
        _ => return Err(Status::expected_field_error("limit", "unsigned integer")),
    };

    if limit == 0 {
        return Err(Status::invalid_field_error("limit"));
    }

    if !is_unlimited(role) {
        limit = limit.min(cap);
    }

    Ok(limit)
}

pub fn choose_ledger_entry_type(params: &JsonValue) -> Result<Option<LedgerEntryType>, Status> {
    let JsonValue::Object(object) = params else {
        return Ok(None);
    };

    let Some(type_value) = object.get("type") else {
        return Ok(None);
    };

    let JsonValue::String(filter) = type_value else {
        return Err(Status::with_message(
            RpcErrorCode::InvalidParams,
            "Invalid field 'type', not string.",
        ));
    };

    LedgerFormats::get_instance()
        .iter()
        .find_map(|item| {
            let canonical_match = item.name().eq_ignore_ascii_case(filter);
            let rpc_match = item.metadata().rpc_name == filter;
            (canonical_match || rpc_match).then(|| item.format_type())
        })
        .map(Some)
        .ok_or_else(|| Status::with_message(RpcErrorCode::InvalidParams, "Invalid field 'type'."))
}

fn parse_sttx_from_json_value(tx_json: &JsonValue) -> Result<STTx, Status> {
    // Strip display-only / computed fields that are not protocol SFields.
    // sign_for and tx results include these in their tx_json output, but
    // they must be removed before re-parsing for submit_multisigned.
    let cleaned = match tx_json {
        JsonValue::Object(map) => {
            let mut m = map.clone();
            m.remove(jss::DeliverMax);
            m.remove(jss::hash);
            JsonValue::Object(m)
        }
        other => other.clone(),
    };

    let parsed = STParsedJSONObject::new("tx_json", &cleaned);
    if let Some(object) = parsed.object {
        return Ok(STTx::from_stobject(object));
    }

    let mut status = Status::new(RpcErrorCode::InvalidParams);
    if let JsonValue::Object(mut error_map) = parsed.error {
        if let Some(JsonValue::String(message)) = error_map.remove("error_message") {
            status = Status::with_message(RpcErrorCode::InvalidParams, message);
        }
    }
    Err(status)
}

/// rippled `keypairForSignature` (RPCHelpers.cpp).
fn keypair_for_signature(
    params: &BTreeMap<String, JsonValue>,
    api_version: u32,
) -> Result<(PublicKey, SecretKey), Status> {
    let has_key_type = params.contains_key(jss::key_type);
    let present: Vec<&str> = [jss::passphrase, jss::secret, jss::seed, jss::seed_hex]
        .into_iter()
        .filter(|name| params.contains_key(*name))
        .collect();
    let Some(&secret_type) = present.last() else {
        return Err(Status::missing_field_error(jss::secret));
    };
    if present.len() > 1 {
        return Err(Status::make_param_error(
            "Exactly one of the following must be specified: passphrase, secret, seed or seed_hex",
        ));
    }

    let mut key_type = None;
    if let Some(value) = params.get(jss::key_type) {
        let JsonValue::String(value) = value else {
            return Err(Status::expected_field_error(jss::key_type, "string"));
        };
        key_type = Some(key_type_from_string(value).ok_or_else(|| {
            if api_version > 1 {
                Status::new(RpcErrorCode::BadKeyType)
            } else {
                Status::invalid_field_error(jss::key_type)
            }
        })?);
        if secret_type == jss::secret {
            return Err(Status::make_param_error(
                "The secret field is not allowed if key_type is used.",
            ));
        }
    }

    // xrpl.js encodes Ed25519 seeds in its own way; detect them.
    let mut seed = None;
    if secret_type != jss::seed_hex
        && let Some(JsonValue::String(text)) = params.get(secret_type)
        && let Some(lib_seed) = parse_xrpl_lib_seed(text)
    {
        if key_type.unwrap_or(KeyType::Ed25519) != KeyType::Ed25519 {
            return Err(Status::with_message(
                RpcErrorCode::BadSeed,
                "Specified seed is for an Ed25519 wallet.",
            ));
        }
        key_type = Some(KeyType::Ed25519);
        seed = Some(lib_seed);
    }
    let key_type = key_type.unwrap_or(KeyType::Secp256k1);

    let seed = match seed {
        Some(seed) => seed,
        None if has_key_type => get_seed_from_rpc(&JsonValue::Object(params.clone()))?,
        None => {
            // Without key_type rippled reads `secret` whatever was given.
            let Some(JsonValue::String(secret)) = params.get(jss::secret) else {
                return Err(Status::expected_field_error(jss::secret, "string"));
            };
            protocol::parse_generic_seed(secret, true).ok_or_else(|| {
                Status::with_message(RpcErrorCode::BadSeed, invalid_field_message(secret_type))
            })?
        }
    };
    protocol::generate_key_pair(key_type, &seed).map_err(|_| Status::new(RpcErrorCode::Internal))
}

fn signing_rules<Runtime: RpcRuntime, Source>(
    ctx: &RpcRequestContext<'_, Source, Runtime>,
) -> protocol::Rules {
    ctx.runtime
        .app()
        .map(|app| app.current_open_ledger_rules())
        .unwrap_or_default()
}

fn parse_signature_target(
    params: &BTreeMap<String, JsonValue>,
) -> Result<Option<SignatureRole>, Status> {
    let Some(value) = params.get(jss::signature_target) else {
        return Ok(None);
    };
    let JsonValue::String(target_name) = value else {
        return Err(Status::with_message(
            RpcErrorCode::InvalidParams,
            invalid_field_message(jss::signature_target),
        ));
    };
    let field = get_field_by_name(target_name);
    signature_role(field).map(Some).ok_or_else(|| {
        Status::with_message(
            RpcErrorCode::InvalidParams,
            invalid_field_message(jss::signature_target),
        )
    })
}

fn signer_target_object<'a>(
    st_tx: &'a mut STTx,
    signature_target: Option<&'static protocol::SField>,
) -> &'a mut STObject {
    if let Some(target) = signature_target {
        st_tx.peek_field_object(target)
    } else {
        st_tx
    }
}

fn transaction_format_result(st_tx: &STTx, api_version: u32) -> BTreeMap<String, JsonValue> {
    let mut tx_json = if api_version > 1 {
        st_tx.json(JsonOptions::DISABLE_API_PRIOR_V2)
    } else {
        st_tx.json(JsonOptions::NONE)
    };
    insert_deliver_max(&mut tx_json, st_tx.get_txn_type(), api_version);

    let mut result = BTreeMap::new();
    result.insert(jss::tx_json.to_string(), tx_json);
    if api_version > 1 {
        result.insert(
            jss::hash.to_string(),
            JsonValue::String(st_tx.get_transaction_id().to_string()),
        );
    }
    result.insert(
        jss::tx_blob.to_string(),
        JsonValue::String(hex::encode(st_tx.get_serializer().data())),
    );
    result
}
