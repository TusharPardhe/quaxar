//! `wallet_propose` handler port from `xrpld/rpc/handlers/admin/keygen/the reference source`.

use crate::commands::rpc_helpers::{get_seed_from_rpc, parse_xrpl_lib_seed};
use crate::state::context::{RpcRequestContext, RpcRuntime};
use crate::status::{RpcErrorCode, Status};
use protocol::{
    JsonValue, KeyType, TokenType, calc_account_id, derive_public_key, encode_base58_token,
    generate_secret_key, jss, random_seed, seed_as_1751,
};
use std::collections::BTreeMap;

pub struct WalletProposeSource;

fn estimate_entropy(input: &str) -> f64 {
    let mut freq = BTreeMap::new();
    for c in input.chars() {
        *freq.entry(c).or_insert(0.0) += 1.0;
    }

    let len = input.len() as f64;
    let mut se = 0.0;
    for f in freq.values() {
        let x = f / len;
        se += x * x.log2();
    }

    (-se * len).floor()
}

pub fn do_wallet_propose<Runtime: RpcRuntime>(
    ctx: &RpcRequestContext<'_, WalletProposeSource, Runtime>,
) -> Result<JsonValue, Status> {
    tracing::debug!(target: "rpc", method = "wallet_propose", "RPC request received");
    wallet_propose(ctx.params)
}

pub fn key_type_from_string(s: &str) -> Option<KeyType> {
    match s.to_lowercase().as_str() {
        "secp256k1" => Some(KeyType::Secp256k1),
        "ed25519" => Some(KeyType::Ed25519),
        _ => None,
    }
}

pub fn wallet_propose(params: &JsonValue) -> Result<JsonValue, Status> {
    let JsonValue::Object(map) = params else {
        return Err(Status::new(RpcErrorCode::InvalidParams));
    };

    let mut key_type = None;
    if let Some(val) = map.get(jss::key_type) {
        let JsonValue::String(s) = val else {
            return Err(Status::expected_field_error(jss::key_type, "string"));
        };
        key_type =
            Some(key_type_from_string(s).ok_or_else(|| Status::new(RpcErrorCode::InvalidParams))?);
    }

    let mut seed = None;
    let mut lib_seed = false;

    if let Some(JsonValue::String(passphrase)) = map.get(jss::passphrase) {
        seed = parse_xrpl_lib_seed(passphrase);
    } else if let Some(JsonValue::String(s)) = map.get(jss::seed) {
        seed = parse_xrpl_lib_seed(s);
    }

    if let Some(s) = seed {
        lib_seed = true;
        if let Some(kt) = key_type {
            if kt != KeyType::Ed25519 {
                return Err(Status::new(RpcErrorCode::InvalidParams));
            }
        }
        key_type = Some(KeyType::Ed25519);
        seed = Some(s);
    }

    if seed.is_none() {
        if map.contains_key(jss::passphrase)
            || map.contains_key(jss::seed)
            || map.contains_key(jss::seed_hex)
        {
            seed = Some(get_seed_from_rpc(params)?);
        } else {
            seed = Some(random_seed());
        }
    }

    let kt = key_type.unwrap_or(KeyType::Secp256k1);
    let s = seed.unwrap();
    let private_key =
        generate_secret_key(kt, &s).map_err(|_| Status::new(RpcErrorCode::Internal))?;
    let public_key =
        derive_public_key(kt, &private_key).map_err(|_| Status::new(RpcErrorCode::Internal))?;

    let mut obj = BTreeMap::new();
    let seed_1751 = seed_as_1751(&s);
    let seed_hex = hex::encode(s.data());
    let seed_base58 = encode_base58_token(TokenType::FamilySeed, s.data());

    obj.insert(
        jss::master_seed.to_string(),
        JsonValue::String(seed_base58.clone()),
    );
    obj.insert(
        jss::master_seed_hex.to_string(),
        JsonValue::String(seed_hex.clone()),
    );
    obj.insert(
        jss::master_key.to_string(),
        JsonValue::String(seed_1751.clone()),
    );
    obj.insert(
        jss::account_id.to_string(),
        JsonValue::String(encode_base58_token(
            TokenType::AccountID,
            calc_account_id(public_key.as_bytes()).data(),
        )),
    );
    obj.insert(
        jss::public_key.to_string(),
        JsonValue::String(encode_base58_token(
            TokenType::AccountPublic,
            public_key.as_bytes(),
        )),
    );
    obj.insert(jss::key_type.to_string(), JsonValue::String(kt.to_string()));
    obj.insert(
        jss::public_key_hex.to_string(),
        JsonValue::String(hex::encode(public_key.as_bytes())),
    );

    if !lib_seed {
        if let Some(JsonValue::String(passphrase)) = map.get(jss::passphrase) {
            if passphrase != &seed_1751 && passphrase != &seed_base58 && passphrase != &seed_hex {
                if estimate_entropy(passphrase) < 80.0 {
                    obj.insert(jss::warning.to_string(), JsonValue::String("This wallet was generated using a user-supplied passphrase that has low entropy and is vulnerable to brute-force attacks.".to_string()));
                } else {
                    obj.insert(jss::warning.to_string(), JsonValue::String("This wallet was generated using a user-supplied passphrase. It may be vulnerable to brute-force attacks.".to_string()));
                }
            }
        }
    }

    Ok(JsonValue::Object(obj))
}
