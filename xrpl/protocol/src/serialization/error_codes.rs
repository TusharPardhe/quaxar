//! RPC error-code catalog from `xrpl/protocol/ErrorCodes.*`.

use std::collections::BTreeMap;

use crate::JsonValue;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ErrorInfo {
    pub code: i32,
    pub token: &'static str,
    pub message: &'static str,
    pub http_status: i32,
}

macro_rules! rpc_codes {
    ($($name:ident = $value:expr,)+) => {
        $(#[allow(non_upper_case_globals)] pub const $name: i32 = $value;)+
    };
}

rpc_codes! {
    rpcUNKNOWN = -1,
    rpcSUCCESS = 0,
    rpcBAD_SYNTAX = 1,
    rpcJSON_RPC = 2,
    rpcFORBIDDEN = 3,
    rpcWRONG_NETWORK = 4,
    rpcNO_PERMISSION = 6,
    rpcNO_EVENTS = 7,
    rpcTOO_BUSY = 9,
    rpcSLOW_DOWN = 10,
    rpcHIGH_FEE = 11,
    rpcNOT_ENABLED = 12,
    rpcNOT_READY = 13,
    rpcAMENDMENT_BLOCKED = 14,
    rpcNO_CLOSED = 15,
    rpcNO_CURRENT = 16,
    rpcNO_NETWORK = 17,
    rpcNOT_SYNCED = 18,
    rpcACT_NOT_FOUND = 19,
    rpcLGR_NOT_FOUND = 21,
    rpcLGR_NOT_VALIDATED = 22,
    rpcMASTER_DISABLED = 23,
    rpcTXN_NOT_FOUND = 29,
    rpcINVALID_HOTWALLET = 30,
    rpcINVALID_PARAMS = 31,
    rpcUNKNOWN_COMMAND = 32,
    rpcNO_PF_REQUEST = 33,
    rpcACT_MALFORMED = 35,
    rpcALREADY_MULTISIG = 36,
    rpcALREADY_SINGLE_SIG = 37,
    rpcBAD_FEATURE = 40,
    rpcBAD_ISSUER = 41,
    rpcBAD_MARKET = 42,
    rpcBAD_SECRET = 43,
    rpcBAD_SEED = 44,
    rpcCHANNEL_MALFORMED = 45,
    rpcCHANNEL_AMT_MALFORMED = 46,
    rpcCOMMAND_MISSING = 47,
    rpcDST_ACT_MALFORMED = 48,
    rpcDST_ACT_MISSING = 49,
    rpcDST_ACT_NOT_FOUND = 50,
    rpcDST_AMT_MALFORMED = 51,
    rpcDST_AMT_MISSING = 52,
    rpcDST_ISR_MALFORMED = 53,
    rpcLGR_IDXS_INVALID = 57,
    rpcLGR_IDX_MALFORMED = 58,
    rpcPUBLIC_MALFORMED = 62,
    rpcSIGNING_MALFORMED = 63,
    rpcSENDMAX_MALFORMED = 64,
    rpcSRC_ACT_MALFORMED = 65,
    rpcSRC_ACT_MISSING = 66,
    rpcSRC_ACT_NOT_FOUND = 67,
    rpcDELEGATE_ACT_NOT_FOUND = 68,
    rpcSRC_CUR_MALFORMED = 69,
    rpcSRC_ISR_MALFORMED = 70,
    rpcSTREAM_MALFORMED = 71,
    rpcATX_DEPRECATED = 72,
    rpcINTERNAL = 73,
    rpcNOT_IMPL = 74,
    rpcNOT_SUPPORTED = 75,
    rpcBAD_KEY_TYPE = 76,
    rpcDB_DESERIALIZATION = 77,
    rpcEXCESSIVE_LGR_RANGE = 78,
    rpcINVALID_LGR_RANGE = 79,
    rpcEXPIRED_VALIDATOR_LIST = 80,
    rpcREPORTING_UNSUPPORTED = 91,
    rpcOBJECT_NOT_FOUND = 92,
    rpcISSUE_MALFORMED = 93,
    rpcORACLE_MALFORMED = 94,
    rpcBAD_CREDENTIALS = 95,
    rpcTX_SIGNED = 96,
    rpcDOMAIN_MALFORMED = 97,
    rpcENTRY_NOT_FOUND = 98,
    rpcUNEXPECTED_LEDGER_TYPE = 99,
}

macro_rules! warning_codes {
    ($($name:ident = $value:expr,)+) => {
        $(#[allow(non_upper_case_globals)] pub const $name: i32 = $value;)+
    };
}

warning_codes! {
    warnRPC_UNSUPPORTED_MAJORITY = 1001,
    warnRPC_AMENDMENT_BLOCKED = 1002,
    warnRPC_EXPIRED_VALIDATOR_LIST = 1003,
    warnRPC_FIELDS_DEPRECATED = 2004,
}

const UNKNOWN_ERROR: ErrorInfo = ErrorInfo {
    code: rpcUNKNOWN,
    token: "unknown",
    message: "An unknown error code.",
    http_status: 200,
};

const ERROR_INFOS: &[ErrorInfo] = &[
    ErrorInfo {
        code: rpcACT_MALFORMED,
        token: "actMalformed",
        message: "Account malformed.",
        http_status: 200,
    },
    ErrorInfo {
        code: rpcACT_NOT_FOUND,
        token: "actNotFound",
        message: "Account not found.",
        http_status: 200,
    },
    ErrorInfo {
        code: rpcALREADY_MULTISIG,
        token: "alreadyMultisig",
        message: "Already multisigned.",
        http_status: 200,
    },
    ErrorInfo {
        code: rpcALREADY_SINGLE_SIG,
        token: "alreadySingleSig",
        message: "Already single-signed.",
        http_status: 200,
    },
    ErrorInfo {
        code: rpcAMENDMENT_BLOCKED,
        token: "amendmentBlocked",
        message: "Amendment blocked, need upgrade.",
        http_status: 503,
    },
    ErrorInfo {
        code: rpcEXPIRED_VALIDATOR_LIST,
        token: "unlBlocked",
        message: "Validator list expired.",
        http_status: 503,
    },
    ErrorInfo {
        code: rpcATX_DEPRECATED,
        token: "deprecated",
        message: "Use the new API or specify a ledger range.",
        http_status: 400,
    },
    ErrorInfo {
        code: rpcBAD_KEY_TYPE,
        token: "badKeyType",
        message: "Bad key type.",
        http_status: 400,
    },
    ErrorInfo {
        code: rpcBAD_FEATURE,
        token: "badFeature",
        message: "Feature unknown or invalid.",
        http_status: 500,
    },
    ErrorInfo {
        code: rpcBAD_ISSUER,
        token: "badIssuer",
        message: "Issuer account malformed.",
        http_status: 400,
    },
    ErrorInfo {
        code: rpcBAD_MARKET,
        token: "badMarket",
        message: "No such market.",
        http_status: 404,
    },
    ErrorInfo {
        code: rpcBAD_SECRET,
        token: "badSecret",
        message: "Secret does not match account.",
        http_status: 403,
    },
    ErrorInfo {
        code: rpcBAD_SEED,
        token: "badSeed",
        message: "Disallowed seed.",
        http_status: 403,
    },
    ErrorInfo {
        code: rpcBAD_SYNTAX,
        token: "badSyntax",
        message: "Syntax error.",
        http_status: 400,
    },
    ErrorInfo {
        code: rpcCHANNEL_MALFORMED,
        token: "channelMalformed",
        message: "Payment channel is malformed.",
        http_status: 400,
    },
    ErrorInfo {
        code: rpcCHANNEL_AMT_MALFORMED,
        token: "channelAmtMalformed",
        message: "Payment channel amount is malformed.",
        http_status: 400,
    },
    ErrorInfo {
        code: rpcCOMMAND_MISSING,
        token: "commandMissing",
        message: "Missing command entry.",
        http_status: 400,
    },
    ErrorInfo {
        code: rpcDB_DESERIALIZATION,
        token: "dbDeserialization",
        message: "Database deserialization error.",
        http_status: 502,
    },
    ErrorInfo {
        code: rpcDST_ACT_MALFORMED,
        token: "dstActMalformed",
        message: "Destination account is malformed.",
        http_status: 400,
    },
    ErrorInfo {
        code: rpcDST_ACT_MISSING,
        token: "dstActMissing",
        message: "Destination account not provided.",
        http_status: 400,
    },
    ErrorInfo {
        code: rpcDST_ACT_NOT_FOUND,
        token: "dstActNotFound",
        message: "Destination account not found.",
        http_status: 404,
    },
    ErrorInfo {
        code: rpcDST_AMT_MALFORMED,
        token: "dstAmtMalformed",
        message: "Destination amount/currency/issuer is malformed.",
        http_status: 400,
    },
    ErrorInfo {
        code: rpcDST_AMT_MISSING,
        token: "dstAmtMissing",
        message: "Destination amount/currency/issuer is missing.",
        http_status: 400,
    },
    ErrorInfo {
        code: rpcDST_ISR_MALFORMED,
        token: "dstIsrMalformed",
        message: "Destination issuer is malformed.",
        http_status: 400,
    },
    ErrorInfo {
        code: rpcEXCESSIVE_LGR_RANGE,
        token: "excessiveLgrRange",
        message: "Ledger range exceeds 1000.",
        http_status: 400,
    },
    ErrorInfo {
        code: rpcFORBIDDEN,
        token: "forbidden",
        message: "Bad credentials.",
        http_status: 403,
    },
    ErrorInfo {
        code: rpcHIGH_FEE,
        token: "highFee",
        message: "Current transaction fee exceeds your limit.",
        http_status: 402,
    },
    ErrorInfo {
        code: rpcINTERNAL,
        token: "internal",
        message: "Internal error.",
        http_status: 500,
    },
    ErrorInfo {
        code: rpcINVALID_LGR_RANGE,
        token: "invalidLgrRange",
        message: "Ledger range is invalid.",
        http_status: 400,
    },
    ErrorInfo {
        code: rpcINVALID_PARAMS,
        token: "invalidParams",
        message: "Invalid parameters.",
        http_status: 400,
    },
    ErrorInfo {
        code: rpcINVALID_HOTWALLET,
        token: "invalidHotWallet",
        message: "Invalid hotwallet.",
        http_status: 400,
    },
    ErrorInfo {
        code: rpcISSUE_MALFORMED,
        token: "issueMalformed",
        message: "Issue is malformed.",
        http_status: 400,
    },
    ErrorInfo {
        code: rpcJSON_RPC,
        token: "json_rpc",
        message: "JSON-RPC transport error.",
        http_status: 500,
    },
    ErrorInfo {
        code: rpcLGR_IDXS_INVALID,
        token: "lgrIdxsInvalid",
        message: "Ledger indexes invalid.",
        http_status: 400,
    },
    ErrorInfo {
        code: rpcLGR_IDX_MALFORMED,
        token: "lgrIdxMalformed",
        message: "Ledger index malformed.",
        http_status: 400,
    },
    ErrorInfo {
        code: rpcLGR_NOT_FOUND,
        token: "lgrNotFound",
        message: "Ledger not found.",
        http_status: 404,
    },
    ErrorInfo {
        code: rpcLGR_NOT_VALIDATED,
        token: "lgrNotValidated",
        message: "Ledger not validated.",
        http_status: 202,
    },
    ErrorInfo {
        code: rpcMASTER_DISABLED,
        token: "masterDisabled",
        message: "Master key is disabled.",
        http_status: 403,
    },
    ErrorInfo {
        code: rpcNOT_ENABLED,
        token: "notEnabled",
        message: "Not enabled in configuration.",
        http_status: 501,
    },
    ErrorInfo {
        code: rpcNOT_IMPL,
        token: "notImpl",
        message: "Not implemented.",
        http_status: 501,
    },
    ErrorInfo {
        code: rpcNOT_READY,
        token: "notReady",
        message: "Not ready to handle this request.",
        http_status: 503,
    },
    ErrorInfo {
        code: rpcNOT_SUPPORTED,
        token: "notSupported",
        message: "Operation not supported.",
        http_status: 501,
    },
    ErrorInfo {
        code: rpcNO_CLOSED,
        token: "noClosed",
        message: "Closed ledger is unavailable.",
        http_status: 503,
    },
    ErrorInfo {
        code: rpcNO_CURRENT,
        token: "noCurrent",
        message: "Current ledger is unavailable.",
        http_status: 503,
    },
    ErrorInfo {
        code: rpcNOT_SYNCED,
        token: "notSynced",
        message: "Not synced to the network.",
        http_status: 503,
    },
    ErrorInfo {
        code: rpcNO_EVENTS,
        token: "noEvents",
        message: "Current transport does not support events.",
        http_status: 405,
    },
    ErrorInfo {
        code: rpcNO_NETWORK,
        token: "noNetwork",
        message: "Not synced to the network.",
        http_status: 503,
    },
    ErrorInfo {
        code: rpcWRONG_NETWORK,
        token: "wrongNetwork",
        message: "Wrong network.",
        http_status: 503,
    },
    ErrorInfo {
        code: rpcNO_PERMISSION,
        token: "noPermission",
        message: "You don't have permission for this command.",
        http_status: 401,
    },
    ErrorInfo {
        code: rpcNO_PF_REQUEST,
        token: "noPathRequest",
        message: "No pathfinding request in progress.",
        http_status: 404,
    },
    ErrorInfo {
        code: rpcOBJECT_NOT_FOUND,
        token: "objectNotFound",
        message: "The requested object was not found.",
        http_status: 404,
    },
    ErrorInfo {
        code: rpcPUBLIC_MALFORMED,
        token: "publicMalformed",
        message: "Public key is malformed.",
        http_status: 400,
    },
    ErrorInfo {
        code: rpcSENDMAX_MALFORMED,
        token: "sendMaxMalformed",
        message: "SendMax amount malformed.",
        http_status: 400,
    },
    ErrorInfo {
        code: rpcSIGNING_MALFORMED,
        token: "signingMalformed",
        message: "Signing of transaction is malformed.",
        http_status: 400,
    },
    ErrorInfo {
        code: rpcSLOW_DOWN,
        token: "slowDown",
        message: "You are placing too much load on the server.",
        http_status: 429,
    },
    ErrorInfo {
        code: rpcSRC_ACT_MALFORMED,
        token: "srcActMalformed",
        message: "Source account is malformed.",
        http_status: 400,
    },
    ErrorInfo {
        code: rpcSRC_ACT_MISSING,
        token: "srcActMissing",
        message: "Source account not provided.",
        http_status: 400,
    },
    ErrorInfo {
        code: rpcSRC_ACT_NOT_FOUND,
        token: "srcActNotFound",
        message: "Source account not found.",
        http_status: 404,
    },
    ErrorInfo {
        code: rpcDELEGATE_ACT_NOT_FOUND,
        token: "delegateActNotFound",
        message: "Delegate account not found.",
        http_status: 404,
    },
    ErrorInfo {
        code: rpcSRC_CUR_MALFORMED,
        token: "srcCurMalformed",
        message: "Source currency is malformed.",
        http_status: 400,
    },
    ErrorInfo {
        code: rpcSRC_ISR_MALFORMED,
        token: "srcIsrMalformed",
        message: "Source issuer is malformed.",
        http_status: 400,
    },
    ErrorInfo {
        code: rpcSTREAM_MALFORMED,
        token: "malformedStream",
        message: "Stream malformed.",
        http_status: 400,
    },
    ErrorInfo {
        code: rpcTOO_BUSY,
        token: "tooBusy",
        message: "The server is too busy to help you now.",
        http_status: 503,
    },
    ErrorInfo {
        code: rpcTXN_NOT_FOUND,
        token: "txnNotFound",
        message: "Transaction not found.",
        http_status: 404,
    },
    ErrorInfo {
        code: rpcUNKNOWN_COMMAND,
        token: "unknownCmd",
        message: "Unknown method.",
        http_status: 405,
    },
    ErrorInfo {
        code: rpcORACLE_MALFORMED,
        token: "oracleMalformed",
        message: "Oracle request is malformed.",
        http_status: 400,
    },
    ErrorInfo {
        code: rpcBAD_CREDENTIALS,
        token: "badCredentials",
        message: "Credentials do not exist, are not accepted, or have expired.",
        http_status: 400,
    },
    ErrorInfo {
        code: rpcTX_SIGNED,
        token: "transactionSigned",
        message: "Transaction should not be signed.",
        http_status: 400,
    },
    ErrorInfo {
        code: rpcDOMAIN_MALFORMED,
        token: "domainMalformed",
        message: "Domain is malformed.",
        http_status: 400,
    },
    ErrorInfo {
        code: rpcENTRY_NOT_FOUND,
        token: "entryNotFound",
        message: "Entry not found.",
        http_status: 400,
    },
    ErrorInfo {
        code: rpcUNEXPECTED_LEDGER_TYPE,
        token: "unexpectedLedgerType",
        message: "Unexpected ledger type.",
        http_status: 400,
    },
];

pub fn get_error_info(code: i32) -> &'static ErrorInfo {
    ERROR_INFOS
        .iter()
        .find(|info| info.code == code)
        .unwrap_or(&UNKNOWN_ERROR)
}

pub fn inject_error(code: i32, json: &mut JsonValue) {
    inject_error_with_message(code, get_error_info(code).message, json);
}

pub fn inject_error_with_message(code: i32, message: &str, json: &mut JsonValue) {
    let info = get_error_info(code);
    let JsonValue::Object(object) = json else {
        *json = JsonValue::Object(BTreeMap::new());
        inject_error_with_message(code, message, json);
        return;
    };
    object.insert("error".to_owned(), JsonValue::String(info.token.to_owned()));
    object.insert(
        "error_code".to_owned(),
        JsonValue::Signed(i64::from(info.code)),
    );
    object.insert(
        "error_message".to_owned(),
        JsonValue::String(message.to_owned()),
    );
}

pub fn make_error(code: i32) -> JsonValue {
    let mut json = JsonValue::Object(BTreeMap::new());
    inject_error(code, &mut json);
    json
}

pub fn make_error_with_message(code: i32, message: &str) -> JsonValue {
    let mut json = JsonValue::Object(BTreeMap::new());
    inject_error_with_message(code, message, &mut json);
    json
}

pub fn make_param_error(message: &str) -> JsonValue {
    make_error_with_message(rpcINVALID_PARAMS, message)
}

pub fn missing_field_message(name: &str) -> String {
    format!("Missing field '{name}'.")
}

pub fn missing_field_error(name: &str) -> JsonValue {
    make_param_error(&missing_field_message(name))
}

pub fn object_field_message(name: &str) -> String {
    format!("Invalid field '{name}', not object.")
}

pub fn object_field_error(name: &str) -> JsonValue {
    make_param_error(&object_field_message(name))
}

pub fn contains_error(json: &JsonValue) -> bool {
    matches!(json, JsonValue::Object(object) if object.contains_key("error"))
}

pub fn error_code_http_status(code: i32) -> i32 {
    get_error_info(code).http_status
}

pub fn rpc_error_string(json: &JsonValue) -> String {
    let JsonValue::Object(object) = json else {
        return String::new();
    };
    let token = match object.get("error") {
        Some(JsonValue::String(value)) => value.as_str(),
        _ => "",
    };
    let message = match object.get("error_message") {
        Some(JsonValue::String(value)) => value.as_str(),
        _ => "",
    };
    format!("{token}{message}")
}

#[cfg(test)]
mod tests {
    use super::{contains_error, error_code_http_status, make_error, rpcBAD_SYNTAX};

    #[test]
    fn error_catalog_exposes_current_tokens() {
        let json = make_error(rpcBAD_SYNTAX);
        assert!(contains_error(&json));
        assert_eq!(error_code_http_status(rpcBAD_SYNTAX), 400);
    }
}
