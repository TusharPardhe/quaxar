//! `validation_create` handler port from `xrpld/rpc/handlers/admin/keygen/the reference source`.

use crate::state::context::{RpcRequestContext, RpcRuntime};
use crate::status::{RpcErrorCode, Status};
use protocol::{
    JsonValue, KeyType, Seed, TokenType, derive_public_key, encode_base58_token,
    generate_secret_key, jss, random_seed, seed_as_1751,
};
use std::collections::BTreeMap;

pub struct ValidationCreateSource;

fn validation_seed(params: &JsonValue) -> Option<Seed> {
    let JsonValue::Object(map) = params else {
        return Some(random_seed());
    };
    if let Some(JsonValue::String(secret)) = map.get(jss::secret) {
        protocol::parse_generic_seed(secret, false)
    } else {
        Some(random_seed())
    }
}

pub fn do_validation_create<Runtime: RpcRuntime>(
    ctx: &RpcRequestContext<'_, ValidationCreateSource, Runtime>,
) -> Result<JsonValue, Status> {
    let seed =
        validation_seed(ctx.params).ok_or_else(|| Status::new(RpcErrorCode::InvalidParams))?;

    let private_key = generate_secret_key(KeyType::Secp256k1, &seed)
        .map_err(|_| Status::new(RpcErrorCode::Internal))?;
    let public_key = derive_public_key(KeyType::Secp256k1, &private_key)
        .map_err(|_| Status::new(RpcErrorCode::Internal))?;

    let mut obj = BTreeMap::new();
    obj.insert(
        jss::validation_public_key.to_string(),
        JsonValue::String(encode_base58_token(
            TokenType::NodePublic,
            public_key.as_bytes(),
        )),
    );
    obj.insert(
        jss::validation_private_key.to_string(),
        JsonValue::String(encode_base58_token(
            TokenType::NodePrivate,
            private_key.as_bytes(),
        )),
    );
    obj.insert(
        jss::validation_seed.to_string(),
        JsonValue::String(encode_base58_token(TokenType::FamilySeed, seed.data())),
    );
    obj.insert(
        jss::validation_key.to_string(),
        JsonValue::String(seed_as_1751(&seed)),
    );

    Ok(JsonValue::Object(obj))
}
