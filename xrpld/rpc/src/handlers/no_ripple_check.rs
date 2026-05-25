//! Read-only `no_ripple_check` RPC slice.
//!
//! This ports the the reference implementation account/default-ripple and trustline scan onto
//! the existing Rust ledger helpers without inventing an Application or live
//! fee-track runtime seam. The scan, role parsing, error handling, and
//! transaction shaping are kept explicit; the fee drops used for recommended
//! transactions come from a narrow source trait seam because the current RPC
//! layer does not expose the reference `FeeTrack` owner graph.

use std::collections::BTreeMap;

use basics::base_uint::Uint256;
use protocol::{
    AccountID, JsonOptions, JsonValue, LedgerEntryType, STAmount, STLedgerEntry, StBase, TxType,
    XRPAmount, currency_to_string, get_field_by_symbol, lsfDefaultRipple, lsfHighNoRipple,
    lsfLowNoRipple, parse_base58_account_id, tfClearNoRipple, tfSetNoRipple, to_base58,
};

use crate::commands::rpc_helpers::{
    expected_field_error, invalid_field_error, read_limit_field, rpc_error,
};
use crate::handlers::ledger_lookup::{
    LedgerLookupContext, LedgerLookupLedger, LedgerLookupSource, RpcRole, RpcStatus,
    lookup_ledger_with_result,
};
use crate::state::tuning::Tuning;
use crate::status::RpcErrorCode;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NoRippleCheckRequest<'a> {
    pub params: &'a JsonValue,
    pub api_version: u32,
    pub role: RpcRole,
}

pub trait NoRippleCheckSource: LedgerLookupSource {
    fn read_account_root(
        &self,
        ledger: &LedgerLookupLedger,
        account_id: AccountID,
    ) -> Option<STLedgerEntry>;

    fn read_owner_dir_page(
        &self,
        ledger: &LedgerLookupLedger,
        account_id: AccountID,
        page_index: u64,
    ) -> Option<STLedgerEntry>;

    fn read_child_entry(
        &self,
        ledger: &LedgerLookupLedger,
        entry_index: Uint256,
    ) -> Option<STLedgerEntry>;

    fn transaction_fee_drops(&self, ledger: &LedgerLookupLedger) -> u64;
}

fn ensure_object(value: &mut JsonValue) -> &mut BTreeMap<String, JsonValue> {
    if !matches!(value, JsonValue::Object(_)) {
        *value = JsonValue::Object(BTreeMap::new());
    }

    let JsonValue::Object(object) = value else {
        unreachable!("json value should be an object");
    };
    object
}

fn account_string(params: &JsonValue) -> Result<String, JsonValue> {
    let JsonValue::Object(object) = params else {
        return Err(crate::commands::rpc_helpers::missing_field_error("account"));
    };

    let Some(account) = object.get("account") else {
        return Err(crate::commands::rpc_helpers::missing_field_error("account"));
    };
    let JsonValue::String(account) = account else {
        return Err(invalid_field_error("account"));
    };

    Ok(account.clone())
}

fn role_string(params: &JsonValue) -> Result<String, JsonValue> {
    let JsonValue::Object(object) = params else {
        return Err(crate::commands::rpc_helpers::missing_field_error("role"));
    };

    let Some(role) = object.get("role") else {
        return Err(crate::commands::rpc_helpers::missing_field_error("role"));
    };
    let JsonValue::String(role) = role else {
        return Err(invalid_field_error("role"));
    };

    Ok(role.clone())
}

fn transactions_requested(params: &JsonValue, api_version: u32) -> Result<bool, JsonValue> {
    let JsonValue::Object(object) = params else {
        return Ok(false);
    };

    let Some(value) = object.get("transactions") else {
        return Ok(false);
    };

    if api_version > 1 && !matches!(value, JsonValue::Bool(_)) {
        return Err(expected_field_error("transactions", "bool"));
    }

    Ok(match value {
        JsonValue::Bool(flag) => *flag,
        JsonValue::String(text) => !text.is_empty(),
        JsonValue::Signed(value) => *value != 0,
        JsonValue::Unsigned(value) => *value != 0,
        JsonValue::Null => false,
        JsonValue::Array(values) => !values.is_empty(),
        JsonValue::Object(values) => !values.is_empty(),
    })
}

fn json_to_error(status: RpcStatus) -> JsonValue {
    let mut error = JsonValue::Object(BTreeMap::new());
    status.inject(&mut error);
    error
}

fn is_account_gateway(role: &str) -> Option<bool> {
    match role {
        "gateway" => Some(true),
        "user" => Some(false),
        _ => None,
    }
}

fn for_each_item_after<S, F>(
    source: &S,
    ledger: &LedgerLookupLedger,
    account_id: AccountID,
    after: Uint256,
    hint: u64,
    mut limit: u32,
    mut f: F,
) -> bool
where
    S: NoRippleCheckSource,
    F: FnMut(&STLedgerEntry) -> bool,
{
    let mut current_index = 0u64;

    if !after.is_zero() {
        let hint_index = hint;
        if let Some(page) = source.read_owner_dir_page(ledger, account_id, hint_index)
            && page
                .get_field_v256(get_field_by_symbol("sfIndexes"))
                .value()
                .iter()
                .any(|entry| *entry == after)
        {
            current_index = hint_index;
        }

        let mut found = false;
        loop {
            let Some(owner_dir) = source.read_owner_dir_page(ledger, account_id, current_index)
            else {
                return found;
            };

            for entry in owner_dir
                .get_field_v256(get_field_by_symbol("sfIndexes"))
                .value()
                .iter()
                .copied()
            {
                if !found {
                    if entry == after {
                        found = true;
                    }
                    continue;
                }

                let Some(sle) = source.read_child_entry(ledger, entry) else {
                    return false;
                };

                let keep = f(&sle);
                if keep && limit <= 1 {
                    return true;
                }
                if keep {
                    limit -= 1;
                }
            }

            let next = owner_dir.get_field_u64(get_field_by_symbol("sfIndexNext"));
            if next == 0 {
                return found;
            }
            current_index = next;
        }
    }

    loop {
        let Some(owner_dir) = source.read_owner_dir_page(ledger, account_id, current_index) else {
            return true;
        };

        for entry in owner_dir
            .get_field_v256(get_field_by_symbol("sfIndexes"))
            .value()
            .iter()
            .copied()
        {
            let Some(sle) = source.read_child_entry(ledger, entry) else {
                return false;
            };

            let keep = f(&sle);
            if keep && limit <= 1 {
                return true;
            }
            if keep {
                limit -= 1;
            }
        }

        let next = owner_dir.get_field_u64(get_field_by_symbol("sfIndexNext"));
        if next == 0 {
            return true;
        }
        current_index = next;
    }
}

fn append_transaction(
    txs: &mut Vec<JsonValue>,
    tx_type: TxType,
    account_id: AccountID,
    sequence: &mut u32,
    fee_drops: u64,
    mut fields: BTreeMap<String, JsonValue>,
) {
    let fee_drops = i64::try_from(fee_drops).unwrap_or(i64::MAX);
    let mut tx = BTreeMap::new();
    tx.insert(
        "TransactionType".to_owned(),
        JsonValue::String(tx_type.format_name().expect("known tx format").to_owned()),
    );
    tx.insert(
        "Sequence".to_owned(),
        JsonValue::Unsigned(u64::from(*sequence)),
    );
    *sequence = sequence.saturating_add(1);
    tx.insert(
        "Account".to_owned(),
        JsonValue::String(to_base58(account_id)),
    );
    tx.insert(
        "Fee".to_owned(),
        XRPAmount::from_drops(fee_drops).json_clipped(),
    );
    tx.append(&mut fields);
    txs.push(JsonValue::Object(tx));
}

fn append_account_set_transaction(
    txs: &mut Vec<JsonValue>,
    account_id: AccountID,
    sequence: &mut u32,
    fee_drops: u64,
) {
    let mut fields = BTreeMap::new();
    fields.insert("SetFlag".to_owned(), JsonValue::Unsigned(8));
    append_transaction(
        txs,
        TxType::ACCOUNT_SET,
        account_id,
        sequence,
        fee_drops,
        fields,
    );
}

fn append_trust_set_transaction(
    txs: &mut Vec<JsonValue>,
    account_id: AccountID,
    sequence: &mut u32,
    fee_drops: u64,
    limit_amount: STAmount,
    flags: u32,
) {
    let mut fields = BTreeMap::new();
    fields.insert(
        "LimitAmount".to_owned(),
        limit_amount.json(JsonOptions::NONE),
    );
    fields.insert("Flags".to_owned(), JsonValue::Unsigned(u64::from(flags)));
    append_transaction(
        txs,
        TxType::TRUST_SET,
        account_id,
        sequence,
        fee_drops,
        fields,
    );
}

fn maybe_report_default_ripple(
    role_gateway: bool,
    transactions: bool,
    txs: &mut Vec<JsonValue>,
    account_id: AccountID,
    sequence: &mut u32,
    fee_drops: u64,
    result: &mut JsonValue,
    b_default_ripple: bool,
) {
    let problems = ensure_object(result)
        .entry("problems".to_owned())
        .or_insert_with(|| JsonValue::Array(Vec::new()));
    let JsonValue::Array(problems) = problems else {
        unreachable!("problems should be an array");
    };

    if b_default_ripple && !role_gateway {
        problems.push(JsonValue::String(
            "You appear to have set your default ripple flag even though you are not a gateway. This is not recommended unless you are experimenting"
                .to_owned(),
        ));
    } else if role_gateway && !b_default_ripple {
        problems.push(JsonValue::String(
            "You should immediately set your default ripple flag".to_owned(),
        ));
        if transactions {
            append_account_set_transaction(txs, account_id, sequence, fee_drops);
        }
    }
}

fn maybe_report_trustline_problem(
    role_gateway: bool,
    transactions: bool,
    txs: &mut Vec<JsonValue>,
    account_id: AccountID,
    sequence: &mut u32,
    fee_drops: u64,
    line: &STLedgerEntry,
    result: &mut JsonValue,
) -> bool {
    if line.get_type() != LedgerEntryType::RippleState {
        return false;
    }

    let b_low = account_id
        == line
            .get_field_amount(get_field_by_symbol("sfLowLimit"))
            .issue()
            .account;
    let b_no_ripple = line.get_field_u32(get_field_by_symbol("sfFlags"))
        & if b_low {
            lsfLowNoRipple
        } else {
            lsfHighNoRipple
        }
        != 0;

    let needs_fix = if b_no_ripple && role_gateway {
        Some("clear")
    } else if !role_gateway && !b_no_ripple {
        Some("probably set")
    } else {
        None
    };

    let Some(problem_verb) = needs_fix else {
        return false;
    };

    let peer_limit = line.get_field_amount(if b_low {
        get_field_by_symbol("sfHighLimit")
    } else {
        get_field_by_symbol("sfLowLimit")
    });
    let peer = peer_limit.issue().account;

    let problems = ensure_object(result)
        .entry("problems".to_owned())
        .or_insert_with(|| JsonValue::Array(Vec::new()));
    let JsonValue::Array(problems) = problems else {
        unreachable!("problems should be an array");
    };
    problems.push(JsonValue::String(format!(
        "You should {problem_verb} the no ripple flag on your {} line to {}",
        currency_to_string(peer_limit.issue().currency),
        to_base58(peer)
    )));

    if transactions {
        let mut limit_amount = line.get_field_amount(if b_low {
            get_field_by_symbol("sfLowLimit")
        } else {
            get_field_by_symbol("sfHighLimit")
        });
        limit_amount.set_issuer(peer);

        append_trust_set_transaction(
            txs,
            account_id,
            sequence,
            fee_drops,
            limit_amount,
            if b_no_ripple {
                tfClearNoRipple
            } else {
                tfSetNoRipple
            },
        );
    }

    true
}

pub fn do_no_ripple_check<S: NoRippleCheckSource>(
    request: &NoRippleCheckRequest<'_>,
    source: &S,
) -> JsonValue {
    tracing::trace!(target: "rpc", method = "noripple_check", "noripple_check query");
    let account_text = match account_string(request.params) {
        Ok(account) => account,
        Err(error) => return error,
    };

    let role_text = match role_string(request.params) {
        Ok(role) => role,
        Err(error) => return error,
    };

    let role_gateway = match is_account_gateway(&role_text) {
        Some(value) => value,
        None => return invalid_field_error("role"),
    };

    let limit = match read_limit_field(request.params, request.role, Tuning::NO_RIPPLE_CHECK) {
        Ok(limit) => limit,
        Err(status) => return json_to_error(status),
    };

    let transactions = match transactions_requested(request.params, request.api_version) {
        Ok(value) => value,
        Err(error) => return error,
    };

    let context = LedgerLookupContext {
        params: request.params,
        source,
        api_version: request.api_version,
        role: request.role,
    };

    let (ledger, mut result) = match lookup_ledger_with_result(&context) {
        Ok(result) => result,
        Err(status) => return json_to_error(status),
    };

    let Some(account_id) = parse_base58_account_id(&account_text) else {
        crate::commands::rpc_helpers::inject_error(RpcErrorCode::ActMalformed, &mut result);
        return result;
    };

    let Some(account_root) = source.read_account_root(&ledger, account_id) else {
        return rpc_error(RpcErrorCode::ActNotFound);
    };

    let mut sequence = account_root.get_field_u32(get_field_by_symbol("sfSequence"));
    let fee_drops = if transactions {
        source.transaction_fee_drops(&ledger)
    } else {
        0
    };

    let mut txs = Vec::new();
    let b_default_ripple = account_root.is_flag(lsfDefaultRipple);
    ensure_object(&mut result).insert("problems".to_owned(), JsonValue::Array(Vec::new()));

    maybe_report_default_ripple(
        role_gateway,
        transactions,
        &mut txs,
        account_id,
        &mut sequence,
        fee_drops,
        &mut result,
        b_default_ripple,
    );

    let _ = for_each_item_after(
        source,
        &ledger,
        account_id,
        Uint256::zero(),
        0,
        limit,
        |sle| {
            maybe_report_trustline_problem(
                role_gateway,
                transactions,
                &mut txs,
                account_id,
                &mut sequence,
                fee_drops,
                sle,
                &mut result,
            )
        },
    );

    let object = ensure_object(&mut result);
    object.insert(
        "account".to_owned(),
        JsonValue::String(to_base58(account_id)),
    );
    if transactions {
        object.insert("transactions".to_owned(), JsonValue::Array(txs));
    }
    result
}
