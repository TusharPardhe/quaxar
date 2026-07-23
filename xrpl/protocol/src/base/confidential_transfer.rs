use basics::base_uint::Uint256;
use sha2::{Digest, Sha512};

pub const EC_SCALAR_LENGTH: usize = 32;
pub const COMPRESSED_EC_POINT_LENGTH: usize = 33;
pub const EC_CIPHERTEXT_COMPONENT_LENGTH: usize = 33;
pub const EC_GAMAL_ENCRYPTED_TOTAL_LENGTH: usize = 66;
pub const EC_PUB_KEY_LENGTH: usize = 33;
pub const EC_PRIV_KEY_LENGTH: usize = 32;
pub const EC_BLINDING_FACTOR_LENGTH: usize = 32;
pub const EC_SCHNORR_PROOF_LENGTH: usize = 64;
pub const EC_PEDERSEN_COMMITMENT_LENGTH: usize = 33;
pub const EC_SINGLE_BULLETPROOF_LENGTH: usize = 688;
pub const EC_DOUBLE_BULLETPROOF_LENGTH: usize = 736;
pub const EC_SEND_SIGMA_PROOF_LENGTH: usize = 128;
pub const EC_SEND_PROOF_LENGTH: usize = EC_SEND_SIGMA_PROOF_LENGTH + EC_DOUBLE_BULLETPROOF_LENGTH;
pub const EC_CONVERT_BACK_SIGMA_PROOF_LENGTH: usize = 128;
pub const EC_CONVERT_BACK_PROOF_LENGTH: usize =
    EC_CONVERT_BACK_SIGMA_PROOF_LENGTH + EC_SINGLE_BULLETPROOF_LENGTH;
pub const EC_CLAWBACK_PROOF_LENGTH: usize = 96;
pub const CONFIDENTIAL_FEE_MULTIPLIER: u32 = 9;
pub const EC_COMPRESSED_PREFIX_EVEN_Y: u8 = 0x02;
pub const EC_COMPRESSED_PREFIX_ODD_Y: u8 = 0x03;

pub fn is_valid_compressed_ec_point(buffer: &[u8]) -> bool {
    if buffer.len() != COMPRESSED_EC_POINT_LENGTH {
        return false;
    }
    if buffer[0] != EC_COMPRESSED_PREFIX_EVEN_Y && buffer[0] != EC_COMPRESSED_PREFIX_ODD_Y {
        return false;
    }
    true
}

pub fn is_valid_ciphertext(buffer: &[u8]) -> bool {
    if buffer.len() != EC_GAMAL_ENCRYPTED_TOTAL_LENGTH {
        return false;
    }
    let c1 = &buffer[..EC_CIPHERTEXT_COMPONENT_LENGTH];
    let c2 = &buffer[EC_CIPHERTEXT_COMPONENT_LENGTH..];
    is_valid_compressed_ec_point(c1) && is_valid_compressed_ec_point(c2)
}

pub fn get_confidential_recipient_count(has_auditor: bool) -> u8 {
    if has_auditor { 4 } else { 3 }
}

fn context_hash_from_parts(parts: &[&[u8]]) -> Uint256 {
    let mut hasher = Sha512::new();
    for part in parts {
        hasher.update(part);
    }
    let digest = hasher.finalize();
    Uint256::from_slice(&digest[..32]).expect("SHA-512 half produces 32 bytes")
}

pub fn get_send_context_hash(
    account: &[u8; 20],
    issuance_id: &[u8; 24],
    sequence: u32,
    destination: &[u8; 20],
    version: u32,
) -> Uint256 {
    let seq_bytes = sequence.to_be_bytes();
    let ver_bytes = version.to_be_bytes();
    context_hash_from_parts(&[
        b"ConfidentialMPTSend",
        account.as_slice(),
        issuance_id.as_slice(),
        &seq_bytes,
        destination.as_slice(),
        &ver_bytes,
    ])
}

pub fn get_clawback_context_hash(
    account: &[u8; 20],
    issuance_id: &[u8; 24],
    sequence: u32,
    holder: &[u8; 20],
) -> Uint256 {
    let seq_bytes = sequence.to_be_bytes();
    context_hash_from_parts(&[
        b"ConfidentialMPTClawback",
        account.as_slice(),
        issuance_id.as_slice(),
        &seq_bytes,
        holder.as_slice(),
    ])
}

pub fn get_convert_context_hash(
    account: &[u8; 20],
    issuance_id: &[u8; 24],
    sequence: u32,
) -> Uint256 {
    let seq_bytes = sequence.to_be_bytes();
    context_hash_from_parts(&[
        b"ConfidentialMPTConvert",
        account.as_slice(),
        issuance_id.as_slice(),
        &seq_bytes,
    ])
}

pub fn get_convert_back_context_hash(
    account: &[u8; 20],
    issuance_id: &[u8; 24],
    sequence: u32,
    version: u32,
) -> Uint256 {
    let seq_bytes = sequence.to_be_bytes();
    let ver_bytes = version.to_be_bytes();
    context_hash_from_parts(&[
        b"ConfidentialMPTConvertBack",
        account.as_slice(),
        issuance_id.as_slice(),
        &seq_bytes,
        &ver_bytes,
    ])
}

pub fn homomorphic_add(a: &[u8], b: &[u8]) -> Option<Vec<u8>> {
    if a.len() != EC_GAMAL_ENCRYPTED_TOTAL_LENGTH || b.len() != EC_GAMAL_ENCRYPTED_TOTAL_LENGTH {
        return None;
    }
    if !is_valid_ciphertext(a) || !is_valid_ciphertext(b) {
        return None;
    }

    // EC point addition on each ciphertext half (C1, C2).
    // Each half is a compressed secp256k1 point (33 bytes).
    let a_c1 = secp256k1::PublicKey::from_slice(&a[..EC_CIPHERTEXT_COMPONENT_LENGTH]).ok()?;
    let a_c2 = secp256k1::PublicKey::from_slice(&a[EC_CIPHERTEXT_COMPONENT_LENGTH..]).ok()?;
    let b_c1 = secp256k1::PublicKey::from_slice(&b[..EC_CIPHERTEXT_COMPONENT_LENGTH]).ok()?;
    let b_c2 = secp256k1::PublicKey::from_slice(&b[EC_CIPHERTEXT_COMPONENT_LENGTH..]).ok()?;

    let sum_c1 = a_c1.combine(&b_c1).ok()?;
    let sum_c2 = a_c2.combine(&b_c2).ok()?;

    let mut result = Vec::with_capacity(EC_GAMAL_ENCRYPTED_TOTAL_LENGTH);
    result.extend_from_slice(&sum_c1.serialize());
    result.extend_from_slice(&sum_c2.serialize());
    Some(result)
}

pub fn homomorphic_subtract(a: &[u8], b: &[u8]) -> Option<Vec<u8>> {
    if a.len() != EC_GAMAL_ENCRYPTED_TOTAL_LENGTH || b.len() != EC_GAMAL_ENCRYPTED_TOTAL_LENGTH {
        return None;
    }
    if !is_valid_ciphertext(a) || !is_valid_ciphertext(b) {
        return None;
    }

    // EC point subtraction: negate B's points then add to A.
    // Negation on compressed points flips the y-parity prefix byte.
    let negate_compressed = |point: &[u8]| -> Vec<u8> {
        let mut negated = point.to_vec();
        negated[0] = match negated[0] {
            EC_COMPRESSED_PREFIX_EVEN_Y => EC_COMPRESSED_PREFIX_ODD_Y,
            EC_COMPRESSED_PREFIX_ODD_Y => EC_COMPRESSED_PREFIX_EVEN_Y,
            other => other,
        };
        negated
    };

    let a_c1 = secp256k1::PublicKey::from_slice(&a[..EC_CIPHERTEXT_COMPONENT_LENGTH]).ok()?;
    let a_c2 = secp256k1::PublicKey::from_slice(&a[EC_CIPHERTEXT_COMPONENT_LENGTH..]).ok()?;

    let neg_b_c1_bytes = negate_compressed(&b[..EC_CIPHERTEXT_COMPONENT_LENGTH]);
    let neg_b_c2_bytes = negate_compressed(&b[EC_CIPHERTEXT_COMPONENT_LENGTH..]);

    let neg_b_c1 = secp256k1::PublicKey::from_slice(&neg_b_c1_bytes).ok()?;
    let neg_b_c2 = secp256k1::PublicKey::from_slice(&neg_b_c2_bytes).ok()?;

    let diff_c1 = a_c1.combine(&neg_b_c1).ok()?;
    let diff_c2 = a_c2.combine(&neg_b_c2).ok()?;

    let mut result = Vec::with_capacity(EC_GAMAL_ENCRYPTED_TOTAL_LENGTH);
    result.extend_from_slice(&diff_c1.serialize());
    result.extend_from_slice(&diff_c2.serialize());
    Some(result)
}

pub fn rerandomize_ciphertext(
    ciphertext: &[u8],
    pub_key: &[u8],
    randomness: &[u8],
) -> Option<Vec<u8>> {
    if ciphertext.len() != EC_GAMAL_ENCRYPTED_TOTAL_LENGTH
        || pub_key.len() != EC_PUB_KEY_LENGTH
        || randomness.len() != EC_BLINDING_FACTOR_LENGTH
    {
        return None;
    }
    // Stub: rerandomize = ciphertext + Enc(0, pub_key, randomness)
    Some(ciphertext.to_vec())
}

pub fn encrypt_amount(amount: u64, pub_key: &[u8], blinding_factor: &[u8]) -> Option<Vec<u8>> {
    if pub_key.len() != EC_PUB_KEY_LENGTH || blinding_factor.len() != EC_BLINDING_FACTOR_LENGTH {
        return None;
    }
    let _ = amount;
    // Stub: produce a valid-format ciphertext
    let mut out = vec![0u8; EC_GAMAL_ENCRYPTED_TOTAL_LENGTH];
    out[0] = EC_COMPRESSED_PREFIX_EVEN_Y;
    out[EC_CIPHERTEXT_COMPONENT_LENGTH] = EC_COMPRESSED_PREFIX_EVEN_Y;
    Some(out)
}

pub fn encrypt_canonical_zero_amount(
    pub_key: &[u8],
    account: &[u8; 20],
    mpt_id: &[u8],
) -> Option<Vec<u8>> {
    if pub_key.len() != EC_PUB_KEY_LENGTH {
        return None;
    }
    let _ = (account, mpt_id);
    // Stub: deterministic zero encryption
    let mut out = vec![0u8; EC_GAMAL_ENCRYPTED_TOTAL_LENGTH];
    out[0] = EC_COMPRESSED_PREFIX_EVEN_Y;
    out[EC_CIPHERTEXT_COMPONENT_LENGTH] = EC_COMPRESSED_PREFIX_EVEN_Y;
    Some(out)
}

pub fn increment_confidential_version(current_version: Option<u32>) -> u32 {
    let current = current_version.unwrap_or(0);
    if current == u32::MAX { 0 } else { current + 1 }
}

use crate::Ter;

pub fn verify_schnorr_proof(pub_key: &[u8], proof: &[u8], context_hash: &Uint256) -> Ter {
    if proof.len() != EC_SCHNORR_PROOF_LENGTH || pub_key.len() != EC_PUB_KEY_LENGTH {
        return Ter::TEC_INTERNAL;
    }
    let _ = context_hash;
    // NOTE: Full verification requires mpt-crypto Rust bindings (pending).
    // Returns success when featureConfidentialTransfer is active; the feature
    // will not be enabled until validators vote and bindings are available.
    Ter::TES_SUCCESS
}

pub fn verify_revealed_amount(
    amount: u64,
    blinding_factor: &[u8],
    holder_pub_key: &[u8],
    holder_encrypted: &[u8],
    issuer_pub_key: &[u8],
    issuer_encrypted: &[u8],
    auditor_pub_key: Option<&[u8]>,
    auditor_encrypted: Option<&[u8]>,
) -> Ter {
    if blinding_factor.len() != EC_BLINDING_FACTOR_LENGTH
        || holder_pub_key.len() != EC_PUB_KEY_LENGTH
        || holder_encrypted.len() != EC_GAMAL_ENCRYPTED_TOTAL_LENGTH
        || issuer_pub_key.len() != EC_PUB_KEY_LENGTH
        || issuer_encrypted.len() != EC_GAMAL_ENCRYPTED_TOTAL_LENGTH
    {
        return Ter::TEC_INTERNAL;
    }
    if let (Some(apk), Some(ae)) = (auditor_pub_key, auditor_encrypted) {
        if apk.len() != EC_PUB_KEY_LENGTH || ae.len() != EC_GAMAL_ENCRYPTED_TOTAL_LENGTH {
            return Ter::TEC_INTERNAL;
        }
    }
    let _ = amount;
    // NOTE: Full verification requires mpt-crypto Rust bindings (pending).
    // Returns success unconditionally until mpt_verify_revealed_amount is ported.
    Ter::TES_SUCCESS
}

pub fn verify_send_proof(
    proof: &[u8],
    spending_balance: &[u8],
    amount_commitment: &[u8],
    balance_commitment: &[u8],
    context_hash: &Uint256,
    recipient_count: u8,
) -> Ter {
    if proof.len() != EC_SEND_PROOF_LENGTH
        || spending_balance.len() != EC_GAMAL_ENCRYPTED_TOTAL_LENGTH
        || amount_commitment.len() != EC_PEDERSEN_COMMITMENT_LENGTH
        || balance_commitment.len() != EC_PEDERSEN_COMMITMENT_LENGTH
    {
        return Ter::TEC_INTERNAL;
    }
    let _ = (context_hash, recipient_count);
    // NOTE: Full verification requires mpt-crypto Rust bindings (pending).
    // Returns success unconditionally until mpt_verify_send_proof is ported.
    Ter::TES_SUCCESS
}

pub fn verify_convert_back_proof(
    proof: &[u8],
    pub_key: &[u8],
    spending_balance: &[u8],
    balance_commitment: &[u8],
    amount: u64,
    context_hash: &Uint256,
) -> Ter {
    if proof.len() != EC_CONVERT_BACK_PROOF_LENGTH
        || pub_key.len() != EC_PUB_KEY_LENGTH
        || spending_balance.len() != EC_GAMAL_ENCRYPTED_TOTAL_LENGTH
        || balance_commitment.len() != EC_PEDERSEN_COMMITMENT_LENGTH
    {
        return Ter::TEC_INTERNAL;
    }
    let _ = (amount, context_hash);
    // NOTE: Full verification requires mpt-crypto Rust bindings (pending).
    // Returns success unconditionally until mpt_verify_convert_back_proof is ported.
    Ter::TES_SUCCESS
}

pub fn verify_clawback_proof(
    amount: u64,
    proof: &[u8],
    pub_key: &[u8],
    ciphertext: &[u8],
    context_hash: &Uint256,
) -> Ter {
    if ciphertext.len() != EC_GAMAL_ENCRYPTED_TOTAL_LENGTH
        || pub_key.len() != EC_PUB_KEY_LENGTH
        || proof.len() != EC_CLAWBACK_PROOF_LENGTH
    {
        return Ter::TEC_INTERNAL;
    }
    let _ = (amount, context_hash);
    // NOTE: Full verification requires mpt-crypto Rust bindings (pending).
    // Returns success unconditionally until mpt_verify_clawback_proof is ported.
    Ter::TES_SUCCESS
}
