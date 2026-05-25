//! RPC status helpers ported from `xrpld/rpc/Status.*`.

#![allow(dead_code)]

use std::collections::BTreeMap;

use protocol::JsonValue;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RpcErrorCode {
    BadSyntax,
    TooBusy,
    Internal,
    WrongNetwork,
    NoPermission,
    NoEvents,
    NotEnabled,
    NotSupported,
    NoNetwork,
    NotSynced,
    LedgerIndexesInvalid,
    LedgerIndexMalformed,
    LedgerNotFound,
    LedgerNotValidated,
    TxnNotFound,
    InvalidParams,
    UnknownCommand,
    NoPathRequest,
    NotImpl,
    DbDeserialization,
    ExcessiveLedgerRange,
    InvalidLedgerRange,
    ActNotFound,
    ActMalformed,
    ChannelMalformed,
    ChannelAmtMalformed,
    PublicMalformed,
    BadFeature,
    BadMarket,
    SrcActNotFound,
    SrcCurMalformed,
    SrcIsrMalformed,
    DstActNotFound,
    DstAmtMalformed,
    DstIsrMalformed,
    ObjectNotFound,
    IssueMalformed,
    DomainMalformed,
    EntryNotFound,
    UnexpectedLedgerType,
    BadCredentials,
    StreamMalformed,
    NotStandalone,
    NotImplemented,
}

impl RpcErrorCode {
    pub const fn code(self) -> i32 {
        match self {
            Self::BadSyntax => 1,
            Self::TooBusy => 9,
            Self::Internal => 73,
            Self::WrongNetwork => 4,
            Self::NoPermission => 6,
            Self::NoEvents => 7,
            Self::NotEnabled => 12,
            Self::NotSupported => 75,
            Self::NoNetwork => 17,
            Self::NotSynced => 18,
            Self::LedgerIndexesInvalid => 57,
            Self::LedgerIndexMalformed => 58,
            Self::LedgerNotFound => 21,
            Self::LedgerNotValidated => 22,
            Self::TxnNotFound => 29,
            Self::InvalidParams => 31,
            Self::UnknownCommand => 32,
            Self::NoPathRequest => 33,
            Self::NotImpl => 74,
            Self::DbDeserialization => 77,
            Self::ExcessiveLedgerRange => 78,
            Self::InvalidLedgerRange => 79,
            Self::ActNotFound => 19,
            Self::ActMalformed => 35,
            Self::ChannelMalformed => 45,
            Self::ChannelAmtMalformed => 46,
            Self::PublicMalformed => 62,
            Self::BadFeature => 40,
            Self::BadMarket => 42,
            Self::SrcActNotFound => 67,
            Self::SrcCurMalformed => 69,
            Self::SrcIsrMalformed => 70,
            Self::DstActNotFound => 50,
            Self::DstAmtMalformed => 51,
            Self::DstIsrMalformed => 53,
            Self::ObjectNotFound => 92,
            Self::IssueMalformed => 93,
            Self::DomainMalformed => 97,
            Self::EntryNotFound => 98,
            Self::UnexpectedLedgerType => 99,
            Self::BadCredentials => 95,
            Self::StreamMalformed => 71,
            Self::NotStandalone => 76,
            Self::NotImplemented => 74, // Same as NotImpl
        }
    }

    pub const fn token(self) -> &'static str {
        match self {
            Self::BadSyntax => "badSyntax",
            Self::TooBusy => "tooBusy",
            Self::Internal => "internal",
            Self::WrongNetwork => "wrongNetwork",
            Self::NoPermission => "noPermission",
            Self::NoEvents => "noEvents",
            Self::NotEnabled => "notEnabled",
            Self::NotSupported => "notSupported",
            Self::NoNetwork => "noNetwork",
            Self::NotSynced => "notSynced",
            Self::LedgerIndexesInvalid => "lgrIdxsInvalid",
            Self::LedgerIndexMalformed => "lgrIdxMalformed",
            Self::LedgerNotFound => "lgrNotFound",
            Self::LedgerNotValidated => "lgrNotValidated",
            Self::TxnNotFound => "txnNotFound",
            Self::InvalidParams => "invalidParams",
            Self::UnknownCommand => "unknownCmd",
            Self::NoPathRequest => "noPathRequest",
            Self::NotImpl => "notImpl",
            Self::DbDeserialization => "dbDeserialization",
            Self::ExcessiveLedgerRange => "excessiveLgrRange",
            Self::InvalidLedgerRange => "invalidLgrRange",
            Self::ActNotFound => "actNotFound",
            Self::ActMalformed => "actMalformed",
            Self::ChannelMalformed => "channelMalformed",
            Self::ChannelAmtMalformed => "channelAmtMalformed",
            Self::PublicMalformed => "publicMalformed",
            Self::BadFeature => "badFeature",
            Self::BadMarket => "badMarket",
            Self::SrcActNotFound => "srcActNotFound",
            Self::SrcCurMalformed => "srcCurMalformed",
            Self::SrcIsrMalformed => "srcIsrMalformed",
            Self::DstActNotFound => "dstActNotFound",
            Self::DstAmtMalformed => "dstAmtMalformed",
            Self::DstIsrMalformed => "dstIsrMalformed",
            Self::ObjectNotFound => "objectNotFound",
            Self::IssueMalformed => "issueMalformed",
            Self::DomainMalformed => "domainMalformed",
            Self::EntryNotFound => "entryNotFound",
            Self::UnexpectedLedgerType => "unexpectedLedgerType",
            Self::BadCredentials => "badCredentials",
            Self::StreamMalformed => "malformedStream",
            Self::NotStandalone => "notStandalone",
            Self::NotImplemented => "notImplemented",
        }
    }

    pub const fn message(self) -> &'static str {
        match self {
            Self::BadSyntax => "Syntax error.",
            Self::TooBusy => "The server is too busy to help you now.",
            Self::Internal => "Internal error.",
            Self::WrongNetwork => "Wrong network.",
            Self::NoPermission => "You don't have permission for this command.",
            Self::NoEvents => "Current transport does not support events.",
            Self::NotEnabled => "Not enabled in configuration.",
            Self::NotSupported => "Operation not supported.",
            Self::NoNetwork | Self::NotSynced => "Not synced to the network.",
            Self::LedgerIndexesInvalid => "Ledger indexes invalid.",
            Self::LedgerIndexMalformed => "Ledger index malformed.",
            Self::LedgerNotFound => "Ledger not found.",
            Self::LedgerNotValidated => "Ledger not validated.",
            Self::TxnNotFound => "Transaction not found.",
            Self::InvalidParams => "Invalid parameters.",
            Self::UnknownCommand => "Unknown method.",
            Self::NoPathRequest => "No pathfinding request in progress.",
            Self::NotImpl => "Not implemented.",
            Self::DbDeserialization => "Database deserialization error.",
            Self::ExcessiveLedgerRange => "Ledger range exceeds 1000.",
            Self::InvalidLedgerRange => "Ledger range is invalid.",
            Self::ActNotFound => "Account not found.",
            Self::ActMalformed => "Account malformed.",
            Self::ChannelMalformed => "Payment channel is malformed.",
            Self::ChannelAmtMalformed => "Payment channel amount is malformed.",
            Self::PublicMalformed => "Public key is malformed.",
            Self::BadFeature => "Feature unknown or invalid.",
            Self::BadMarket => "No such market.",
            Self::SrcActNotFound => "Source account not found.",
            Self::SrcCurMalformed => "Source currency is malformed.",
            Self::SrcIsrMalformed => "Source issuer is malformed.",
            Self::DstActNotFound => "Destination account not found.",
            Self::DstAmtMalformed => "Destination amount/currency/issuer is malformed.",
            Self::DstIsrMalformed => "Destination issuer is malformed.",
            Self::ObjectNotFound => "The requested object was not found.",
            Self::IssueMalformed => "Issue is malformed.",
            Self::DomainMalformed => "Domain is malformed.",
            Self::EntryNotFound => "Entry not found.",
            Self::UnexpectedLedgerType => "Unexpected ledger type.",
            Self::BadCredentials => "Credentials do not exist, are not accepted, or have expired.",
            Self::StreamMalformed => "Stream malformed.",
            Self::NotStandalone => "Operation only allowed in standalone mode.",
            Self::NotImplemented => "Not implemented.",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Status {
    code: Option<RpcErrorCode>,
    messages: Vec<String>,
}

impl Default for Status {
    fn default() -> Self {
        Self::OK
    }
}

impl From<RpcErrorCode> for Status {
    fn from(value: RpcErrorCode) -> Self {
        Self::new(value)
    }
}

pub type RpcStatus = Status;

impl Status {
    pub const OK: Self = Self {
        code: None,
        messages: Vec::new(),
    };

    pub fn new(code: RpcErrorCode) -> Self {
        Self {
            code: Some(code),
            messages: Vec::new(),
        }
    }

    pub fn with_message(code: RpcErrorCode, message: impl Into<String>) -> Self {
        Self {
            code: Some(code),
            messages: vec![message.into()],
        }
    }

    pub fn make_param_error(message: impl Into<String>) -> Self {
        Self::with_message(RpcErrorCode::InvalidParams, message)
    }

    pub fn expected_field_message(name: impl AsRef<str>, ty: impl AsRef<str>) -> String {
        format!("Invalid field '{}', not {}.", name.as_ref(), ty.as_ref())
    }

    pub fn missing_field_message(name: impl AsRef<str>) -> String {
        format!("Missing field '{}'.", name.as_ref())
    }

    pub fn invalid_field_message(name: impl AsRef<str>) -> String {
        format!("Invalid field '{}'.", name.as_ref())
    }

    pub fn expected_field_error(name: impl AsRef<str>, ty: impl AsRef<str>) -> Self {
        Self::make_param_error(Self::expected_field_message(name, ty))
    }

    pub fn missing_field_error(name: impl AsRef<str>) -> Self {
        Self::make_param_error(Self::missing_field_message(name))
    }

    pub fn invalid_field_error(name: impl AsRef<str>) -> Self {
        Self::make_param_error(Self::invalid_field_message(name))
    }

    pub fn is_ok(&self) -> bool {
        self.code.is_none()
    }

    pub fn error_code(&self) -> Option<RpcErrorCode> {
        self.code
    }

    pub fn messages(&self) -> &[String] {
        &self.messages
    }

    pub fn message(&self) -> String {
        self.messages.join("/")
    }

    pub fn code_string(&self) -> String {
        let Some(code) = self.code else {
            return String::new();
        };
        format!("{}: {}", code.token(), code.message())
    }

    pub fn inject(&self, value: &mut JsonValue) {
        let Some(code) = self.code else {
            return;
        };

        let object = ensure_object(value);
        object.insert(
            "error".to_owned(),
            JsonValue::String(code.token().to_owned()),
        );
        object.insert(
            "error_code".to_owned(),
            JsonValue::Signed(i64::from(code.code())),
        );
        object.insert(
            "error_message".to_owned(),
            JsonValue::String(if self.messages.is_empty() {
                code.message().to_owned()
            } else {
                self.message()
            }),
        );
    }

    pub fn fill_json(&self, value: &mut JsonValue) {
        let Some(code) = self.code else {
            return;
        };

        let error = ensure_object_field(value, "error");
        error.insert("code".to_owned(), JsonValue::Signed(i64::from(code.code())));
        error.insert("message".to_owned(), JsonValue::String(self.code_string()));
        if !self.messages.is_empty() {
            error.insert(
                "data".to_owned(),
                JsonValue::Array(
                    self.messages
                        .iter()
                        .cloned()
                        .map(JsonValue::String)
                        .collect(),
                ),
            );
        }
    }

    pub fn to_status_string(&self) -> String {
        if self.is_ok() {
            String::new()
        } else {
            format!("{}:{}", self.code_string(), self.message())
        }
    }
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

fn ensure_object_field<'a>(
    value: &'a mut JsonValue,
    field: &str,
) -> &'a mut BTreeMap<String, JsonValue> {
    let object = ensure_object(value);
    let field_value = object
        .entry(field.to_owned())
        .or_insert_with(|| JsonValue::Object(BTreeMap::new()));
    if !matches!(field_value, JsonValue::Object(_)) {
        *field_value = JsonValue::Object(BTreeMap::new());
    }
    let JsonValue::Object(object) = field_value else {
        unreachable!("json field should be an object");
    };
    object
}
