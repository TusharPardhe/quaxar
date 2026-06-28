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
        KeyType::Secp256k1 => derive_deterministic_account_key(seed),
    }
}

/// Generate a ROOT secret key (no account tweak). Used for VALIDATOR keys
/// where rippled uses the root key directly (matching rippled's validator identity).
pub fn generate_root_secret_key(
    key_type: KeyType,
    seed: &Seed,
) -> Result<SecretKey, SecretKeyError> {
    match key_type {
        KeyType::Ed25519 => SecretKey::from_slice(sha512_half_secure(seed.as_slice()).data()),
        KeyType::Secp256k1 => derive_deterministic_root_key(seed),
    }
}

fn derive_deterministic_account_key(seed: &Seed) -> Result<SecretKey, SecretKeyError> {
    let root = derive_deterministic_root_key(seed)?;
    // Derive account keypair: root → generator → account (matching rippled's
    // generateKeyPair which calls deriveKeypair → root + tweak(generator, 0))
    let root_sk = secp256k1::SecretKey::from_byte_array(*root.as_bytes())
        .map_err(|_| SecretKeyError::KeyGenerationFailed)?;
    let secp = secp256k1::Secp256k1::new();
    let generator = secp256k1::PublicKey::from_secret_key(&secp, &root_sk).serialize();
    let tweak = calculate_account_tweak(generator, 0).ok_or(SecretKeyError::KeyGenerationFailed)?;
    let scalar = secp256k1::scalar::Scalar::from_be_bytes(tweak)
        .map_err(|_| SecretKeyError::KeyGenerationFailed)?;
    let account_sk = root_sk
        .add_tweak(&scalar)
        .map_err(|_| SecretKeyError::KeyGenerationFailed)?;
    SecretKey::from_slice(&account_sk.secret_bytes())
}

fn calculate_account_tweak(generator: [u8; 33], ordinal: u32) -> Option<[u8; 32]> {
    let mut buffer = [0u8; 41];
    buffer[..33].copy_from_slice(&generator);
    buffer[33..37].copy_from_slice(&ordinal.to_be_bytes());

    for subseq in 0u32..128 {
        buffer[37..41].copy_from_slice(&subseq.to_be_bytes());
        let candidate = sha512_half_secure(buffer);
        if *candidate.data() != [0u8; 32]
            && secp256k1::SecretKey::from_byte_array(*candidate.data()).is_ok()
        {
            return Some(*candidate.data());
        }
    }
    None
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

        // Verified against rippled wallet_propose for seed snoPBrXtMeMyMHUVTgbuqAfg1SUTb:
        // public_key_hex = 0330E7FC9D56BB25D6893BA3F317AE5BCF33B3291BD63DB32654A313222F7FD020
        // account_id = rHb9CJAWyB4rj91VRWn96DkukG4bwdtyTh
        let expected: [u8; 33] = [
            0x03, 0x30, 0xE7, 0xFC, 0x9D, 0x56, 0xBB, 0x25, 0xD6, 0x89, 0x3B, 0xA3, 0xF3, 0x17,
            0xAE, 0x5B, 0xCF, 0x33, 0xB3, 0x29, 0x1B, 0xD6, 0x3D, 0xB3, 0x26, 0x54, 0xA3, 0x13,
            0x22, 0x2F, 0x7F, 0xD0, 0x20,
        ];
        assert_eq!(public.as_bytes(), &expected);
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
