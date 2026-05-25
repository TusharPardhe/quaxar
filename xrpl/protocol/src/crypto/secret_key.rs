//! Protocol secret key surface used by transaction builders and signing helpers.

use crate::{KeyType, PublicKey, Seed, sha512_half, sha512_half_secure};

pub const SECRET_KEY_LENGTH: usize = 32;

#[derive(Clone)]
pub struct SecretKey([u8; SECRET_KEY_LENGTH]);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecretKeyError {
    InvalidLength,
    InvalidSecret,
    KeyGenerationFailed,
}

impl SecretKey {
    pub fn from_slice(slice: &[u8]) -> Result<Self, SecretKeyError> {
        if slice.len() != SECRET_KEY_LENGTH {
            return Err(SecretKeyError::InvalidLength);
        }

        let mut bytes = [0u8; SECRET_KEY_LENGTH];
        bytes.copy_from_slice(slice);
        Ok(Self(bytes))
    }

    pub const fn from_bytes(bytes: [u8; SECRET_KEY_LENGTH]) -> Self {
        Self(bytes)
    }

    pub const fn as_bytes(&self) -> &[u8; SECRET_KEY_LENGTH] {
        &self.0
    }

    pub fn to_hex(&self) -> String {
        basics::str_hex::str_hex(self.0)
    }
}

impl std::fmt::Debug for SecretKey {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("SecretKey(..)")
    }
}

impl Drop for SecretKey {
    fn drop(&mut self) {
        self.0.fill(0);
    }
}

pub fn derive_public_key(
    key_type: KeyType,
    secret_key: &SecretKey,
) -> Result<PublicKey, SecretKeyError> {
    match key_type {
        KeyType::Secp256k1 => {
            let secret = secp256k1::SecretKey::from_byte_array(*secret_key.as_bytes())
                .map_err(|_| SecretKeyError::InvalidSecret)?;
            let secp = secp256k1::Secp256k1::new();
            Ok(PublicKey::from_bytes(
                secp256k1::PublicKey::from_secret_key(&secp, &secret).serialize(),
            ))
        }
        KeyType::Ed25519 => {
            let signing_key = ed25519_dalek::SigningKey::from_bytes(secret_key.as_bytes());
            let mut public = [0u8; crate::PUBLIC_KEY_LENGTH];
            public[0] = 0xED;
            public[1..].copy_from_slice(&signing_key.verifying_key().to_bytes());
            Ok(PublicKey::from_bytes(public))
        }
    }
}

pub fn generate_secret_key(key_type: KeyType, seed: &Seed) -> Result<SecretKey, SecretKeyError> {
    match key_type {
        KeyType::Ed25519 => SecretKey::from_slice(sha512_half_secure(seed.as_slice()).data()),
        KeyType::Secp256k1 => derive_deterministic_root_key(seed),
    }
}

fn derive_deterministic_root_key(seed: &Seed) -> Result<SecretKey, SecretKeyError> {
    let mut buffer = [0u8; 20];
    buffer[..16].copy_from_slice(seed.as_slice());

    for seq in 0u32..128 {
        buffer[16..].copy_from_slice(&seq.to_be_bytes());
        let candidate = sha512_half(buffer);
        if secp256k1::SecretKey::from_byte_array(*candidate.data()).is_ok() {
            return SecretKey::from_slice(candidate.data());
        }
    }

    Err(SecretKeyError::KeyGenerationFailed)
}

#[cfg(test)]
mod tests {
    use crate::{
        KeyType, TokenType, derive_public_key, parse_base58_seed, parse_base58_with_type,
        sha512_half_secure,
    };

    use super::generate_secret_key;

    #[test]
    fn generate_secp256k1_secret_key_seed_vectors() {
        let seed = parse_base58_seed("snoPBrXtMeMyMHUVTgbuqAfg1SUTb")
            .expect("family seed vector should parse");
        let secret =
            generate_secret_key(KeyType::Secp256k1, &seed).expect("seed should derive secret");
        let public =
            derive_public_key(KeyType::Secp256k1, &secret).expect("public key should derive");

        assert_eq!(
            public.to_node_public_base58(),
            "n94a1u4jAz288pZLtw6yFWVbi89YamiC6JBXPVUj5zmExe5fTVg9"
        );
        assert_eq!(
            secret.to_hex(),
            parse_base58_with_type::<crate::SecretKey>(
                TokenType::NodePrivate,
                "pnen77YEeUd4fFKG7iycBWcwKpTaeFRkW2WFostaATy1DSupwXe"
            )
            .expect("node private vector should parse")
            .to_hex()
        );
    }

    #[test]
    fn generate_ed25519_secret_key_matches_secure_seed_hash_contract() {
        let seed = parse_base58_seed("snoPBrXtMeMyMHUVTgbuqAfg1SUTb")
            .expect("family seed vector should parse");
        let secret =
            generate_secret_key(KeyType::Ed25519, &seed).expect("seed should derive secret");

        assert_eq!(
            *secret.as_bytes(),
            *sha512_half_secure(seed.as_slice()).data()
        );
    }
}
