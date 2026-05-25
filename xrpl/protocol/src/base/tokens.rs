//! Base58 token helpers from `xrpl/protocol/tokens.*`.

use std::sync::OnceLock;

use bs58::Alphabet;
use sha2::{Digest, Sha256};

use crate::{AccountID, PublicKey, SecretKey};

const XRPL_BASE58_ALPHABET: &[u8; 58] =
    b"rpshnaf39wBUDNEGHJKLM4PQRST7VWXYZ2bcdeCg65jkm8oFqi1tuvAxyz";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum TokenType {
    None = 1,
    NodePublic = 28,
    NodePrivate = 32,
    AccountID = 0,
    AccountPublic = 35,
    AccountSecret = 34,
    FamilyGenerator = 41,
    FamilySeed = 33,
}

pub trait Base58Token: Sized {
    const TOKEN_TYPE: TokenType;
    fn from_token_bytes(bytes: &[u8]) -> Option<Self>;
    fn to_token_bytes(&self) -> Vec<u8>;
}

pub trait TypedBase58Token: Sized {
    fn from_typed_token_bytes(token_type: TokenType, bytes: &[u8]) -> Option<Self>;
}

impl Base58Token for AccountID {
    const TOKEN_TYPE: TokenType = TokenType::AccountID;

    fn from_token_bytes(bytes: &[u8]) -> Option<Self> {
        AccountID::from_slice(bytes)
    }

    fn to_token_bytes(&self) -> Vec<u8> {
        self.data().to_vec()
    }
}

impl Base58Token for PublicKey {
    const TOKEN_TYPE: TokenType = TokenType::AccountPublic;

    fn from_token_bytes(bytes: &[u8]) -> Option<Self> {
        PublicKey::from_slice(bytes).ok()
    }

    fn to_token_bytes(&self) -> Vec<u8> {
        self.as_bytes().to_vec()
    }
}

impl TypedBase58Token for PublicKey {
    fn from_typed_token_bytes(token_type: TokenType, bytes: &[u8]) -> Option<Self> {
        matches!(token_type, TokenType::AccountPublic | TokenType::NodePublic)
            .then(|| PublicKey::from_slice(bytes).ok())
            .flatten()
    }
}

impl Base58Token for SecretKey {
    const TOKEN_TYPE: TokenType = TokenType::AccountSecret;

    fn from_token_bytes(bytes: &[u8]) -> Option<Self> {
        SecretKey::from_slice(bytes).ok()
    }

    fn to_token_bytes(&self) -> Vec<u8> {
        self.as_bytes().to_vec()
    }
}

impl TypedBase58Token for SecretKey {
    fn from_typed_token_bytes(token_type: TokenType, bytes: &[u8]) -> Option<Self> {
        matches!(
            token_type,
            TokenType::AccountSecret | TokenType::NodePrivate
        )
        .then(|| SecretKey::from_slice(bytes).ok())
        .flatten()
    }
}

pub fn parse_base58<T: Base58Token>(value: &str) -> Option<T> {
    let bytes = decode_base58_token(value, T::TOKEN_TYPE)?;
    T::from_token_bytes(&bytes)
}

pub fn parse_base58_with_type<T: TypedBase58Token>(
    token_type: TokenType,
    value: &str,
) -> Option<T> {
    let bytes = decode_base58_token(value, token_type)?;
    T::from_typed_token_bytes(token_type, &bytes)
}

pub fn encode_base58_token(token_type: TokenType, token: &[u8]) -> String {
    let mut payload = Vec::with_capacity(1 + token.len() + 4);
    payload.push(token_type as u8);
    payload.extend_from_slice(token);
    payload.extend_from_slice(&checksum(&payload));
    bs58::encode(payload)
        .with_alphabet(xrpl_base58_alphabet())
        .into_string()
}

pub fn decode_base58_token(value: &str, token_type: TokenType) -> Option<Vec<u8>> {
    decode_base58_token_multibyte(value, &[token_type as u8])
}

pub fn decode_base58_token_multibyte(value: &str, prefix: &[u8]) -> Option<Vec<u8>> {
    let decoded = bs58::decode(value)
        .with_alphabet(xrpl_base58_alphabet())
        .into_vec()
        .ok()?;
    if decoded.len() < prefix.len() + 4 || !decoded.starts_with(prefix) {
        return None;
    }
    let payload_len = decoded.len() - 4;
    let expected = checksum(&decoded[..payload_len]);
    if decoded[payload_len..] != expected {
        return None;
    }
    Some(decoded[prefix.len()..payload_len].to_vec())
}

fn checksum(message: &[u8]) -> [u8; 4] {
    let first = Sha256::digest(message);
    let second = Sha256::digest(first);
    let mut checksum = [0u8; 4];
    checksum.copy_from_slice(&second[..4]);
    checksum
}

fn xrpl_base58_alphabet() -> &'static Alphabet {
    static ALPHABET: OnceLock<Alphabet> = OnceLock::new();
    ALPHABET.get_or_init(|| {
        Alphabet::new(XRPL_BASE58_ALPHABET).expect("XRPL base58 alphabet should remain valid")
    })
}
