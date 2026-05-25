//! Protocol public key surface used by transaction builders and signing helpers.

use crate::{KeyType, encode_node_public_base58};

pub const PUBLIC_KEY_LENGTH: usize = 33;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PublicKey([u8; PUBLIC_KEY_LENGTH]);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PublicKeyError {
    InvalidLength,
    UnknownKeyType,
}

impl PublicKey {
    pub fn from_slice(slice: &[u8]) -> Result<Self, PublicKeyError> {
        if slice.len() != PUBLIC_KEY_LENGTH {
            return Err(PublicKeyError::InvalidLength);
        }

        let mut bytes = [0u8; PUBLIC_KEY_LENGTH];
        bytes.copy_from_slice(slice);
        let key = Self(bytes);
        key.key_type().ok_or(PublicKeyError::UnknownKeyType)?;
        Ok(key)
    }

    pub const fn from_bytes(bytes: [u8; PUBLIC_KEY_LENGTH]) -> Self {
        Self(bytes)
    }

    pub const fn as_bytes(&self) -> &[u8; PUBLIC_KEY_LENGTH] {
        &self.0
    }

    pub fn to_bytes(self) -> [u8; PUBLIC_KEY_LENGTH] {
        self.0
    }

    pub const fn key_type(&self) -> Option<KeyType> {
        match self.0[0] {
            0x02 | 0x03 => Some(KeyType::Secp256k1),
            0xED => Some(KeyType::Ed25519),
            _ => None,
        }
    }

    pub fn to_node_public_base58(self) -> String {
        encode_node_public_base58(self.0)
    }

    pub fn to_hex(self) -> String {
        basics::str_hex::str_hex(self.0)
    }
}

impl AsRef<[u8]> for PublicKey {
    fn as_ref(&self) -> &[u8] {
        self.as_bytes()
    }
}

impl std::fmt::Display for PublicKey {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.to_hex())
    }
}
