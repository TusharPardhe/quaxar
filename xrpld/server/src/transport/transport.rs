use protocol::JsonValue;

use crate::session::{RequestMetadata, WSSession};

#[derive(Debug, Clone, PartialEq)]
pub struct RpcError {
    pub code: i32,
    pub token: String,
    pub message: String,
    pub data: Option<JsonValue>,
}

impl RpcError {
    pub fn new(code: i32, token: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code,
            token: token.into(),
            message: message.into(),
            data: None,
        }
    }
}

impl From<rpc::RpcErrorCode> for RpcError {
    fn from(value: rpc::RpcErrorCode) -> Self {
        Self::new(value.code(), value.token(), value.message())
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum RpcReply {
    Result(JsonValue),
    Error(RpcError),
}

impl RpcReply {
    pub fn result(value: JsonValue) -> Self {
        Self::Result(value)
    }

    pub fn error(code: rpc::RpcErrorCode, message: impl Into<String>) -> Self {
        let mut error = RpcError::from(code);
        error.message = message.into();
        Self::Error(error)
    }

    pub fn with_meta(self, key: impl Into<String>, value: JsonValue) -> Self {
        match self {
            Self::Result(JsonValue::Object(mut object)) => {
                object.insert(key.into(), value);
                Self::Result(JsonValue::Object(object))
            }
            other => other,
        }
    }
}

pub struct RpcRequest<'a> {
    pub method: &'a str,
    pub params: &'a JsonValue,
    pub metadata: &'a RequestMetadata,
    pub session: Option<&'a WSSession>,
}

pub trait RpcDispatcher: Send + Sync {
    fn dispatch(&self, request: RpcRequest<'_>) -> RpcReply;
}
