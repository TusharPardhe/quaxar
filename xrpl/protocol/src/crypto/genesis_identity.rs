//! Deterministic genesis-identity helpers for genesis-ledger construction.
//!
//! This module provides the secp256k1 passphrase-to-account derivation used by
//! the genesis-ledger constructor and related compatibility tests.

use basics::base_uint::Uint160;
use ripemd::Ripemd160;
use secp256k1::{PublicKey, Scalar, Secp256k1, SecretKey};
use sha2::{Digest, Sha256, Sha512};

pub const GENESIS_PASSPHRASE: &str = "masterpassphrase";

pub fn genesis_account_id() -> Uint160 {
    derive_secp256k1_account_id_from_passphrase(GENESIS_PASSPHRASE, 0)
        .expect("current XRPL genesis account derivation must stay valid")
}

pub fn genesis_public_key() -> [u8; 33] {
    derive_secp256k1_public_key_from_passphrase(GENESIS_PASSPHRASE, 0)
        .expect("current XRPL genesis public-key derivation must stay valid")
}

pub fn derive_secp256k1_account_id_from_passphrase(
    passphrase: &str,
    ordinal: u32,
) -> Option<Uint160> {
    let public_key = derive_secp256k1_public_key_from_passphrase(passphrase, ordinal)?;
    Some(account_id_from_public_key_bytes(public_key))
}

pub fn derive_secp256k1_public_key_from_passphrase(
    passphrase: &str,
    ordinal: u32,
) -> Option<[u8; 33]> {
    let seed = seed_from_passphrase(passphrase);
    derive_secp256k1_public_key_from_seed(seed, ordinal)
}

fn seed_from_passphrase(passphrase: &str) -> [u8; 16] {
    let mut hasher = Sha512::new();
    hasher.update(passphrase.as_bytes());
    let digest = hasher.finalize();
    digest[..16]
        .try_into()
        .expect("seed slices must contain exactly 16 bytes")
}

fn derive_secp256k1_public_key_from_seed(seed: [u8; 16], ordinal: u32) -> Option<[u8; 33]> {
    let secp = Secp256k1::new();
    let root_bytes = derive_deterministic_root_key(seed)?;
    let root = SecretKey::from_byte_array(root_bytes).ok()?;
    let generator = PublicKey::from_secret_key(&secp, &root).serialize();
    let tweak_bytes = calculate_tweak(generator, ordinal)?;
    let tweak = Scalar::from_be_bytes(tweak_bytes).ok()?;
    let secret = root.add_tweak(&tweak).ok()?;
    Some(PublicKey::from_secret_key(&secp, &secret).serialize())
}

fn derive_deterministic_root_key(seed: [u8; 16]) -> Option<[u8; 32]> {
    let mut buffer = [0u8; 20];
    buffer[..16].copy_from_slice(&seed);

    for seq in 0..128u32 {
        buffer[16..].copy_from_slice(&seq.to_be_bytes());
        let candidate = sha512_half(buffer);
        if SecretKey::from_byte_array(candidate).is_ok() {
            return Some(candidate);
        }
    }

    None
}

fn calculate_tweak(generator: [u8; 33], ordinal: u32) -> Option<[u8; 32]> {
    let mut buffer = [0u8; 41];
    buffer[..33].copy_from_slice(&generator);
    buffer[33..37].copy_from_slice(&ordinal.to_be_bytes());

    for subseq in 0..128u32 {
        buffer[37..41].copy_from_slice(&subseq.to_be_bytes());
        let tweak = sha512_half(buffer);
        if tweak != [0; 32] && SecretKey::from_byte_array(tweak).is_ok() {
            return Some(tweak);
        }
    }

    None
}

fn sha512_half<const N: usize>(bytes: [u8; N]) -> [u8; 32] {
    let mut hasher = Sha512::new();
    hasher.update(bytes);
    let digest = hasher.finalize();
    digest[..32]
        .try_into()
        .expect("SHA-512 half output must contain exactly 32 bytes")
}

fn account_id_from_public_key_bytes(public_key: [u8; 33]) -> Uint160 {
    let sha = Sha256::digest(public_key);
    let ripe = Ripemd160::digest(sha);
    Uint160::from_slice(&ripe).expect("RIPEMD-160 output must contain 20 bytes")
}

#[cfg(test)]
mod tests {
    use super::{
        GENESIS_PASSPHRASE, derive_secp256k1_account_id_from_passphrase,
        derive_secp256k1_public_key_from_passphrase, genesis_account_id, genesis_public_key,
        seed_from_passphrase,
    };
    use basics::base_uint::{Uint160, to_string};

    #[test]
    fn genesis_seed_matches_current_cpp_passphrase_hash_prefix() {
        assert_eq!(
            seed_from_passphrase(GENESIS_PASSPHRASE),
            [
                0xDE, 0xDC, 0xE9, 0xCE, 0x67, 0xB4, 0x51, 0xD8, 0x52, 0xFD, 0x4E, 0x84, 0x6F, 0xCD,
                0xE3, 0x1C,
            ]
        );
    }

    #[test]
    fn genesis_public_key_matches_current_cpp_vector() {
        assert_eq!(
            to_string(
                &Uint160::from_slice(&genesis_public_key()[..20])
                    .expect("public-key prefix slice should fit in Uint160 for test display")
            ),
            "0330E7FC9D56BB25D6893BA3F317AE5BCF33B329"
        );
        assert_eq!(
            genesis_public_key(),
            [
                0x03, 0x30, 0xE7, 0xFC, 0x9D, 0x56, 0xBB, 0x25, 0xD6, 0x89, 0x3B, 0xA3, 0xF3, 0x17,
                0xAE, 0x5B, 0xCF, 0x33, 0xB3, 0x29, 0x1B, 0xD6, 0x3D, 0xB3, 0x26, 0x54, 0xA3, 0x13,
                0x22, 0x2F, 0x7F, 0xD0, 0x20,
            ]
        );
    }

    #[test]
    fn genesis_account_id_matches_current_cpp_vector() {
        let expected = Uint160::from_hex("B5F762798A53D543A014CAF8B297CFF8F2F937E8")
            .expect("expected genesis account id should parse");

        assert_eq!(genesis_account_id(), expected);
        assert_eq!(
            derive_secp256k1_account_id_from_passphrase(GENESIS_PASSPHRASE, 0),
            Some(expected)
        );
    }

    #[test]
    fn other_passphrases_produce_different_accounts() {
        let genesis = genesis_account_id();
        let other = derive_secp256k1_account_id_from_passphrase("otherpassphrase", 0)
            .expect("other passphrase should still derive");

        assert_ne!(other, genesis);
        assert_eq!(
            derive_secp256k1_public_key_from_passphrase("otherpassphrase", 0)
                .expect("other passphrase should derive a public key")
                .len(),
            33
        );
    }
}
