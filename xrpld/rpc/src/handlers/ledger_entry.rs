//! Narrow `ledger_entry` RPC handler slice.
//!
//! This ports the the reference implementation selector/lookup/result-shaping behavior onto the
//! existing Rust ledger lookup and ledger-entry helper surfaces. It keeps the
//! implementation honest: if a selector can be derived from the current Rust
//! protocol surface, it is ported here; otherwise the handler returns the same
//! style of reference-shaped invalid-params error instead of inventing a wider
//! runtime seam.

use std::collections::BTreeMap;

use basics::{
    base_uint::{Uint160, Uint256},
    str_hex::str_hex,
};
use protocol::{
    JsonValue, LedgerEntryType, STLedgerEntry, Serializer, StBase, account_keylet, did_keylet,
};

use crate::{
    LedgerLookupContext, LedgerLookupLedger, LedgerLookupSource, RpcErrorCode, RpcRole, RpcStatus,
    inject_error, lookup_ledger_with_result,
};

use crate::handlers::ledger_entry_helpers as helpers;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LedgerEntryRequest<'a> {
    pub params: &'a JsonValue,
    pub api_version: u32,
    pub role: RpcRole,
}

pub trait LedgerEntrySource: LedgerLookupSource {
    fn read_ledger_entry(
        &self,
        ledger: &LedgerLookupLedger,
        entry_index: Uint256,
    ) -> Option<STLedgerEntry>;
}

#[derive(Debug, Clone, Copy)]
struct Selector {
    field: &'static str,
    expected_type: LedgerEntryType,
    parser: fn(&JsonValue, &'static str, u32) -> Result<Uint256, JsonValue>,
    use_full_params: bool,
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

fn json_from_status(status: RpcStatus) -> JsonValue {
    let mut json = JsonValue::Object(BTreeMap::new());
    status.inject(&mut json);
    json
}

fn parse_mpt_issuance(
    params: &JsonValue,
    field: &'static str,
    _api_version: u32,
) -> Result<Uint256, JsonValue> {
    helpers::parse_mpt_issuance(params, field)
}

fn parse_mptoken(
    params: &JsonValue,
    field: &'static str,
    _api_version: u32,
) -> Result<Uint256, JsonValue> {
    helpers::parse_mptoken(params, field)
}

fn parse_deposit_preauth(
    params: &JsonValue,
    field: &'static str,
    _api_version: u32,
) -> Result<Uint256, JsonValue> {
    helpers::parse_deposit_preauth_account(params, field)
}

macro_rules! wrap_selector_2arg {
    ($name:ident, $helper:path) => {
        fn $name(
            params: &JsonValue,
            field: &'static str,
            _api_version: u32,
        ) -> Result<Uint256, JsonValue> {
            $helper(params, field)
        }
    };
}

macro_rules! wrap_selector_3arg {
    ($name:ident, $helper:path) => {
        fn $name(
            params: &JsonValue,
            field: &'static str,
            api_version: u32,
        ) -> Result<Uint256, JsonValue> {
            $helper(params, field, api_version)
        }
    };
}

wrap_selector_2arg!(parse_nftoken_offer_selector, helpers::parse_nftoken_offer);
wrap_selector_2arg!(parse_check_selector, helpers::parse_check);
wrap_selector_2arg!(parse_nftoken_page_selector, helpers::parse_nftoken_page);
wrap_selector_2arg!(parse_signer_list_selector, helpers::parse_signer_list);
wrap_selector_2arg!(parse_ticket_selector, helpers::parse_ticket);
wrap_selector_2arg!(parse_offer_selector, helpers::parse_offer);
wrap_selector_2arg!(parse_ripple_state_selector, helpers::parse_ripple_state);
wrap_selector_2arg!(parse_escrow_selector, helpers::parse_escrow);
wrap_selector_2arg!(parse_pay_channel_selector, helpers::parse_pay_channel);
wrap_selector_2arg!(parse_amm_selector, helpers::parse_amm);
wrap_selector_2arg!(parse_oracle_selector, helpers::parse_oracle);
wrap_selector_2arg!(parse_credential_selector, helpers::parse_credential);
wrap_selector_2arg!(
    parse_permissioned_domain_selector,
    helpers::parse_permissioned_domain
);
wrap_selector_2arg!(parse_delegate_selector, helpers::parse_delegate);
wrap_selector_2arg!(parse_vault_selector, helpers::parse_vault);
wrap_selector_2arg!(parse_loan_broker_selector, helpers::parse_loan_broker);
wrap_selector_2arg!(parse_loan_selector, helpers::parse_loan);
wrap_selector_3arg!(parse_negative_unl_selector, helpers::parse_negative_unl);
wrap_selector_3arg!(parse_directory_node_selector, helpers::parse_directory_node);
wrap_selector_3arg!(parse_amendments_selector, helpers::parse_amendments);
wrap_selector_3arg!(parse_ledger_hashes_selector, helpers::parse_ledger_hashes);
wrap_selector_3arg!(parse_fee_settings_selector, helpers::parse_fee_settings);
wrap_selector_3arg!(parse_index_selector, helpers::parse_index);

fn parse_bridge(
    params: &JsonValue,
    field: &'static str,
    _api_version: u32,
) -> Result<Uint256, JsonValue> {
    helpers::parse_bridge(params, field)
}

fn parse_xchain_owned_claim_id(
    params: &JsonValue,
    field: &'static str,
    _api_version: u32,
) -> Result<Uint256, JsonValue> {
    helpers::parse_xchain_owned_claim_id(params, field)
}

fn parse_xchain_owned_create_account_claim_id(
    params: &JsonValue,
    field: &'static str,
    _api_version: u32,
) -> Result<Uint256, JsonValue> {
    helpers::parse_xchain_owned_create_account_claim_id(params, field)
}

fn parse_did_selector(
    params: &JsonValue,
    field: &'static str,
    _api_version: u32,
) -> Result<Uint256, JsonValue> {
    let Some(account) = helpers::parse_account_id(params) else {
        return Err(helpers::invalid_field_error(
            "malformedAddress",
            field,
            "AccountID",
        ));
    };

    Ok(did_keylet(Uint160::from_slice(account.data()).expect("account width")).key)
}

fn parse_account_root_selector(
    params: &JsonValue,
    field: &'static str,
    _api_version: u32,
) -> Result<Uint256, JsonValue> {
    let Some(account) = helpers::parse_account_id(params) else {
        return Err(helpers::invalid_field_error(
            "malformedAddress",
            field,
            "AccountID",
        ));
    };

    Ok(account_keylet(Uint160::from_slice(account.data()).expect("account width")).key)
}

fn parse_selection(
    params: &JsonValue,
    selector: &Selector,
    api_version: u32,
) -> Result<Uint256, JsonValue> {
    let value = if selector.use_full_params {
        params
    } else {
        let JsonValue::Object(object) = params else {
            return Err(helpers::missing_field_error(selector.field));
        };
        object
            .get(selector.field)
            .ok_or_else(|| helpers::missing_field_error(selector.field))?
    };

    (selector.parser)(value, selector.field, api_version)
}

fn selector_count(params: &JsonValue) -> usize {
    let JsonValue::Object(object) = params else {
        return 0;
    };

    selectors()
        .iter()
        .filter(|selector| object.contains_key(selector.field))
        .count()
}

fn selectors() -> &'static [Selector] {
    static SELECTORS: &[Selector] = &[
        Selector {
            field: "nft_offer",
            expected_type: LedgerEntryType::NFTokenOffer,
            parser: parse_nftoken_offer_selector,
            use_full_params: false,
        },
        Selector {
            field: "check",
            expected_type: LedgerEntryType::Check,
            parser: parse_check_selector,
            use_full_params: false,
        },
        Selector {
            field: "did",
            expected_type: LedgerEntryType::DID,
            parser: parse_did_selector,
            use_full_params: false,
        },
        Selector {
            field: "nunl",
            expected_type: LedgerEntryType::NegativeUnl,
            parser: parse_negative_unl_selector,
            use_full_params: false,
        },
        Selector {
            field: "nft_page",
            expected_type: LedgerEntryType::NFTokenPage,
            parser: parse_nftoken_page_selector,
            use_full_params: false,
        },
        Selector {
            field: "signer_list",
            expected_type: LedgerEntryType::SignerList,
            parser: parse_signer_list_selector,
            use_full_params: false,
        },
        Selector {
            field: "ticket",
            expected_type: LedgerEntryType::Ticket,
            parser: parse_ticket_selector,
            use_full_params: false,
        },
        Selector {
            field: "account",
            expected_type: LedgerEntryType::AccountRoot,
            parser: parse_account_root_selector,
            use_full_params: false,
        },
        Selector {
            field: "directory",
            expected_type: LedgerEntryType::DirectoryNode,
            parser: parse_directory_node_selector,
            use_full_params: false,
        },
        Selector {
            field: "amendments",
            expected_type: LedgerEntryType::Amendments,
            parser: parse_amendments_selector,
            use_full_params: false,
        },
        Selector {
            field: "hashes",
            expected_type: LedgerEntryType::LedgerHashes,
            parser: parse_ledger_hashes_selector,
            use_full_params: false,
        },
        Selector {
            field: "bridge",
            expected_type: LedgerEntryType::Bridge,
            parser: parse_bridge,
            use_full_params: true,
        },
        Selector {
            field: "offer",
            expected_type: LedgerEntryType::Offer,
            parser: parse_offer_selector,
            use_full_params: false,
        },
        Selector {
            field: "deposit_preauth",
            expected_type: LedgerEntryType::DepositPreauth,
            parser: parse_deposit_preauth,
            use_full_params: false,
        },
        Selector {
            field: "xchain_owned_claim_id",
            expected_type: LedgerEntryType::XChainOwnedClaimId,
            parser: parse_xchain_owned_claim_id,
            use_full_params: false,
        },
        Selector {
            field: "state",
            expected_type: LedgerEntryType::RippleState,
            parser: parse_ripple_state_selector,
            use_full_params: false,
        },
        Selector {
            field: "fee",
            expected_type: LedgerEntryType::FeeSettings,
            parser: parse_fee_settings_selector,
            use_full_params: false,
        },
        Selector {
            field: "xchain_owned_create_account_claim_id",
            expected_type: LedgerEntryType::XChainOwnedCreateAccountClaimId,
            parser: parse_xchain_owned_create_account_claim_id,
            use_full_params: false,
        },
        Selector {
            field: "escrow",
            expected_type: LedgerEntryType::Escrow,
            parser: parse_escrow_selector,
            use_full_params: false,
        },
        Selector {
            field: "payment_channel",
            expected_type: LedgerEntryType::PayChannel,
            parser: parse_pay_channel_selector,
            use_full_params: false,
        },
        Selector {
            field: "amm",
            expected_type: LedgerEntryType::AMM,
            parser: parse_amm_selector,
            use_full_params: false,
        },
        Selector {
            field: "mpt_issuance",
            expected_type: LedgerEntryType::MPTokenIssuance,
            parser: parse_mpt_issuance,
            use_full_params: false,
        },
        Selector {
            field: "mptoken",
            expected_type: LedgerEntryType::MPToken,
            parser: parse_mptoken,
            use_full_params: false,
        },
        Selector {
            field: "oracle",
            expected_type: LedgerEntryType::Oracle,
            parser: parse_oracle_selector,
            use_full_params: false,
        },
        Selector {
            field: "credential",
            expected_type: LedgerEntryType::Credential,
            parser: parse_credential_selector,
            use_full_params: false,
        },
        Selector {
            field: "permissioned_domain",
            expected_type: LedgerEntryType::PermissionedDomain,
            parser: parse_permissioned_domain_selector,
            use_full_params: false,
        },
        Selector {
            field: "delegate",
            expected_type: LedgerEntryType::Delegate,
            parser: parse_delegate_selector,
            use_full_params: false,
        },
        Selector {
            field: "vault",
            expected_type: LedgerEntryType::Vault,
            parser: parse_vault_selector,
            use_full_params: false,
        },
        Selector {
            field: "loan_broker",
            expected_type: LedgerEntryType::LoanBroker,
            parser: parse_loan_broker_selector,
            use_full_params: false,
        },
        Selector {
            field: "loan",
            expected_type: LedgerEntryType::Loan,
            parser: parse_loan_selector,
            use_full_params: false,
        },
        Selector {
            field: "index",
            expected_type: LedgerEntryType::Any,
            parser: parse_index_selector,
            use_full_params: false,
        },
        Selector {
            field: "account_root",
            expected_type: LedgerEntryType::AccountRoot,
            parser: parse_account_root_selector,
            use_full_params: false,
        },
        Selector {
            field: "ripple_state",
            expected_type: LedgerEntryType::RippleState,
            parser: parse_ripple_state_selector,
            use_full_params: false,
        },
    ];

    SELECTORS
}

fn inject_named_error(result: &mut JsonValue, error: &str) {
    let object = ensure_object(result);
    object.insert("error".to_owned(), JsonValue::String(error.to_owned()));
}

fn set_index(result: &mut JsonValue, index: Uint256) {
    let object = ensure_object(result);
    object.insert("index".to_owned(), JsonValue::String(index.to_string()));
}

pub fn do_ledger_entry<S: LedgerEntrySource>(
    request: &LedgerEntryRequest<'_>,
    source: &S,
) -> JsonValue {
    tracing::trace!(target: "rpc", method = "ledger_entry", "ledger_entry query");
    if selector_count(request.params) > 1 {
        return json_from_status(RpcStatus::make_param_error("Too many fields provided."));
    }

    let context = LedgerLookupContext {
        params: request.params,
        source,
        api_version: request.api_version,
        role: request.role,
    };

    let (ledger, mut result) = match lookup_ledger_with_result(&context) {
        Ok(result) => result,
        Err(status) => return json_from_status(status),
    };

    let Some(selector) = selectors().iter().find(|selector| match request.params {
        JsonValue::Object(object) => object.contains_key(selector.field),
        _ => false,
    }) else {
        if request.api_version < 2 {
            inject_named_error(&mut result, "unknownOption");
            return result;
        }

        return json_from_status(RpcStatus::make_param_error(
            "No ledger_entry params provided.",
        ));
    };

    let node_index = match parse_selection(request.params, selector, request.api_version) {
        Ok(index) => index,
        Err(error) => return error,
    };

    set_index(&mut result, node_index);

    if node_index.is_zero() {
        inject_error(RpcErrorCode::EntryNotFound, &mut result);
        return result;
    }

    let Some(node) = source.read_ledger_entry(&ledger, node_index) else {
        inject_error(RpcErrorCode::EntryNotFound, &mut result);
        return result;
    };

    if selector.expected_type != LedgerEntryType::Any && node.get_type() != selector.expected_type {
        inject_error(RpcErrorCode::UnexpectedLedgerType, &mut result);
        return result;
    }

    let binary = matches!(
        request.params,
        JsonValue::Object(object) if matches!(object.get("binary"), Some(JsonValue::Bool(true)))
    );

    if binary {
        let mut serializer = Serializer::new(256);
        node.add(&mut serializer);
        let object = ensure_object(&mut result);
        object.insert(
            "node_binary".to_owned(),
            JsonValue::String(str_hex(serializer.data())),
        );
    } else {
        let object = ensure_object(&mut result);
        object.insert("node".to_owned(), helpers::render_ledger_entry_json(&node));
    }

    result
}
