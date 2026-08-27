use basics::base_uint::Uint256;
use libloading::Library;
use sha2::{Digest, Sha256};
use std::path::PathBuf;
use std::sync::OnceLock;

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
pub const EC_DOUBLE_BULLETPROOF_LENGTH: usize = 754;
pub const EC_SEND_SIGMA_PROOF_LENGTH: usize = 192;
pub const EC_SEND_PROOF_LENGTH: usize = EC_SEND_SIGMA_PROOF_LENGTH + EC_DOUBLE_BULLETPROOF_LENGTH;
pub const EC_CONVERT_BACK_SIGMA_PROOF_LENGTH: usize = 128;
pub const EC_CONVERT_BACK_PROOF_LENGTH: usize =
    EC_CONVERT_BACK_SIGMA_PROOF_LENGTH + EC_SINGLE_BULLETPROOF_LENGTH;
pub const EC_CLAWBACK_PROOF_LENGTH: usize = 64;
pub const CONFIDENTIAL_FEE_MULTIPLIER: u32 = 9;
pub const EC_COMPRESSED_PREFIX_EVEN_Y: u8 = 0x02;
pub const EC_COMPRESSED_PREFIX_ODD_Y: u8 = 0x03;

#[repr(C)]
#[derive(Clone, Copy)]
struct MptAccountId {
    bytes: [u8; 20],
}

#[repr(C)]
#[derive(Clone, Copy)]
struct MptIssuanceId {
    bytes: [u8; 24],
}

#[repr(C)]
#[derive(Clone, Copy)]
struct MptParticipant {
    pubkey: [u8; EC_PUB_KEY_LENGTH],
    ciphertext: [u8; EC_GAMAL_ENCRYPTED_TOTAL_LENGTH],
}

type GetConvertContextHash = unsafe extern "C" fn(MptAccountId, MptIssuanceId, u32, *mut u8) -> i32;
type GetConvertBackContextHash =
    unsafe extern "C" fn(MptAccountId, MptIssuanceId, u32, u32, *mut u8) -> i32;
type GetSendContextHash =
    unsafe extern "C" fn(MptAccountId, MptIssuanceId, u32, MptAccountId, u32, *mut u8) -> i32;
type GetClawbackContextHash =
    unsafe extern "C" fn(MptAccountId, MptIssuanceId, u32, MptAccountId, *mut u8) -> i32;
type EncryptAmount = unsafe extern "C" fn(u64, *const u8, *const u8, *mut u8) -> i32;
type VerifyRevealedAmount = unsafe extern "C" fn(
    u64,
    *const u8,
    *const MptParticipant,
    *const MptParticipant,
    *const MptParticipant,
) -> i32;
type VerifyConvertProof = unsafe extern "C" fn(*const u8, *const u8, *const u8) -> i32;
type VerifyConvertBackProof =
    unsafe extern "C" fn(*const u8, *const u8, *const u8, *const u8, u64, *const u8) -> i32;
type VerifySendProof = unsafe extern "C" fn(
    *const u8,
    *const MptParticipant,
    u8,
    *const u8,
    *const u8,
    *const u8,
    *const u8,
) -> i32;
type VerifyClawbackProof =
    unsafe extern "C" fn(*const u8, u64, *const u8, *const u8, *const u8) -> i32;

struct MptCrypto {
    // Function pointers remain valid only while the library is resident.
    _library: Library,
    get_convert_context_hash: GetConvertContextHash,
    get_convert_back_context_hash: GetConvertBackContextHash,
    get_send_context_hash: GetSendContextHash,
    get_clawback_context_hash: GetClawbackContextHash,
    encrypt_amount: EncryptAmount,
    verify_revealed_amount: VerifyRevealedAmount,
    verify_convert_proof: VerifyConvertProof,
    verify_convert_back_proof: VerifyConvertBackProof,
    verify_send_proof: VerifySendProof,
    verify_clawback_proof: VerifyClawbackProof,
}

static MPT_CRYPTO: OnceLock<Result<MptCrypto, String>> = OnceLock::new();

fn library_candidates() -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    if let Some(path) = std::env::var_os("QUAXAR_MPT_CRYPTO_LIBRARY") {
        candidates.push(PathBuf::from(path));
    }
    #[cfg(target_os = "macos")]
    candidates.extend(["libmpt-crypto.1.dylib", "libmpt-crypto.dylib"].map(PathBuf::from));
    #[cfg(target_os = "linux")]
    candidates.extend(["libmpt-crypto.so.1", "libmpt-crypto.so"].map(PathBuf::from));
    #[cfg(target_os = "windows")]
    candidates.push(PathBuf::from("mpt-crypto.dll"));
    candidates
}

unsafe fn symbol<T: Copy>(library: &Library, name: &'static [u8]) -> Result<T, String> {
    // SAFETY: every requested symbol is declared by mpt-crypto/1.0.2's public
    // C header. The load-time vector below rejects an ABI-incompatible library.
    unsafe { library.get::<T>(name) }
        .map(|value| *value)
        .map_err(|error| format!("missing {}: {error}", String::from_utf8_lossy(name)))
}

fn load_mpt_crypto() -> Result<MptCrypto, String> {
    load_mpt_crypto_from(library_candidates())
}

fn load_mpt_crypto_from(candidates: Vec<PathBuf>) -> Result<MptCrypto, String> {
    let mut failures = Vec::new();
    for path in candidates {
        // SAFETY: loading executes no Quaxar-owned pointer dereferences. All
        // symbols are checked before the handle is published.
        let library = match unsafe { Library::new(&path) } {
            Ok(library) => library,
            Err(error) => {
                failures.push(format!("{}: {error}", path.display()));
                continue;
            }
        };
        let loaded = (|| unsafe {
            let crypto = MptCrypto {
                get_convert_context_hash: symbol(&library, b"mpt_get_convert_context_hash\0")?,
                get_convert_back_context_hash: symbol(
                    &library,
                    b"mpt_get_convert_back_context_hash\0",
                )?,
                get_send_context_hash: symbol(&library, b"mpt_get_send_context_hash\0")?,
                get_clawback_context_hash: symbol(&library, b"mpt_get_clawback_context_hash\0")?,
                encrypt_amount: symbol(&library, b"mpt_encrypt_amount\0")?,
                verify_revealed_amount: symbol(&library, b"mpt_verify_revealed_amount\0")?,
                verify_convert_proof: symbol(&library, b"mpt_verify_convert_proof\0")?,
                verify_convert_back_proof: symbol(&library, b"mpt_verify_convert_back_proof\0")?,
                verify_send_proof: symbol(&library, b"mpt_verify_send_proof\0")?,
                verify_clawback_proof: symbol(&library, b"mpt_verify_clawback_proof\0")?,
                _library: library,
            };

            // Pinned 1.0.2 ABI vector. Besides checking byte order, this calls
            // a by-value struct ABI that would expose an incompatible layout.
            if !crypto.context_vectors_match() {
                return Err("mpt-crypto context-hash ABI self-test failed".to_owned());
            }
            Ok(crypto)
        })();
        match loaded {
            Ok(crypto) => return Ok(crypto),
            Err(error) => failures.push(format!("{}: {error}", path.display())),
        }
    }
    Err(format!(
        "official mpt-crypto/1.0.2 is unavailable ({})",
        failures.join("; ")
    ))
}

impl MptCrypto {
    unsafe fn context_vectors_match(&self) -> bool {
        let account = MptAccountId {
            bytes: std::array::from_fn(|index| index as u8),
        };
        let issuance = MptIssuanceId {
            bytes: std::array::from_fn(|index| index as u8),
        };
        let other = MptAccountId {
            bytes: std::array::from_fn(|index| (index + 20) as u8),
        };
        let mut actual = [[0u8; 32]; 4];
        // SAFETY: each official ABI writes exactly 32 bytes.
        let ok = unsafe {
            (self.get_convert_context_hash)(account, issuance, 0x0102_0304, actual[0].as_mut_ptr())
                == 0
                && (self.get_convert_back_context_hash)(
                    account,
                    issuance,
                    0x0102_0304,
                    0x0506_0708,
                    actual[1].as_mut_ptr(),
                ) == 0
                && (self.get_send_context_hash)(
                    account,
                    issuance,
                    0x0102_0304,
                    other,
                    0x0506_0708,
                    actual[2].as_mut_ptr(),
                ) == 0
                && (self.get_clawback_context_hash)(
                    account,
                    issuance,
                    0x0102_0304,
                    other,
                    actual[3].as_mut_ptr(),
                ) == 0
        };
        ok && actual
            == [
                decode_vector("233f4a8552793b09d0480dc74674f60fa6cf90ea7554431e5036f4ee2e832d87"),
                decode_vector("911d49d6c86e4128aeb1b655c605f8b81a3620176618ee7ee224cc78cbadb78d"),
                decode_vector("bc6c0ff25c4ff58c35c52c2eb4ff00f222b0f8bd6d3bde52f0a88e7c095c73fc"),
                decode_vector("212634d39916295282e4956125cd39e962da71ee54cf8a27e1d89f5e58c39c9e"),
            ]
    }
}

fn decode_vector(value: &str) -> [u8; 32] {
    let mut output = [0u8; 32];
    for (index, pair) in value.as_bytes().chunks_exact(2).enumerate() {
        let nibble = |byte: u8| match byte {
            b'0'..=b'9' => byte - b'0',
            b'a'..=b'f' => byte - b'a' + 10,
            _ => unreachable!("pinned lowercase hex vector"),
        };
        output[index] = (nibble(pair[0]) << 4) | nibble(pair[1]);
    }
    output
}

fn mpt_crypto() -> Result<&'static MptCrypto, &'static str> {
    MPT_CRYPTO
        .get_or_init(load_mpt_crypto)
        .as_ref()
        .map_err(String::as_str)
}

pub fn confidential_crypto_available() -> bool {
    mpt_crypto().is_ok()
}

pub fn confidential_crypto_unavailable_reason() -> Option<&'static str> {
    mpt_crypto().err()
}

fn confidential_crypto_required_at_startup(
    required_by_release: bool,
    explicitly_configured: bool,
) -> bool {
    required_by_release || explicitly_configured
}

/// Validate the optional native consensus dependency before application
/// components start. A release which advertises ConfidentialTransfer support
/// must have the pinned ABI available. An explicitly configured library is
/// also mandatory even while the amendment remains gated, so a misspelled or
/// ABI-incompatible deployment path cannot be silently ignored.
pub fn validate_confidential_crypto_startup(required_by_release: bool) -> Result<(), String> {
    let explicitly_configured = std::env::var_os("QUAXAR_MPT_CRYPTO_LIBRARY").is_some();
    if !confidential_crypto_required_at_startup(required_by_release, explicitly_configured) {
        return Ok(());
    }
    mpt_crypto().map(|_| ()).map_err(|reason| {
        format!(
            "ConfidentialTransfer requires official mpt-crypto/1.0.2, but startup capability validation failed: {reason}"
        )
    })
}

pub fn is_valid_compressed_ec_point(buffer: &[u8]) -> bool {
    if buffer.len() != COMPRESSED_EC_POINT_LENGTH {
        return false;
    }
    if buffer[0] != EC_COMPRESSED_PREFIX_EVEN_Y && buffer[0] != EC_COMPRESSED_PREFIX_ODD_Y {
        return false;
    }
    // Prefix and width alone do not establish that the x-coordinate encodes a
    // point on secp256k1.  mpt-crypto rejects malformed curve points, so the
    // compatibility layer must parse the point as well.
    secp256k1::PublicKey::from_slice(buffer).is_ok()
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

pub fn get_send_context_hash(
    account: &[u8; 20],
    issuance_id: &[u8; 24],
    sequence: u32,
    destination: &[u8; 20],
    version: u32,
) -> Option<Uint256> {
    let crypto = mpt_crypto().ok()?;
    let mut output = [0u8; 32];
    // SAFETY: all by-value wrappers and the output width match the official ABI.
    let result = unsafe {
        (crypto.get_send_context_hash)(
            MptAccountId { bytes: *account },
            MptIssuanceId {
                bytes: *issuance_id,
            },
            sequence,
            MptAccountId {
                bytes: *destination,
            },
            version,
            output.as_mut_ptr(),
        )
    };
    (result == 0).then(|| Uint256::from_void(&output))
}

pub fn get_clawback_context_hash(
    account: &[u8; 20],
    issuance_id: &[u8; 24],
    sequence: u32,
    holder: &[u8; 20],
) -> Option<Uint256> {
    let crypto = mpt_crypto().ok()?;
    let mut output = [0u8; 32];
    // SAFETY: all by-value wrappers and the output width match the official ABI.
    let result = unsafe {
        (crypto.get_clawback_context_hash)(
            MptAccountId { bytes: *account },
            MptIssuanceId {
                bytes: *issuance_id,
            },
            sequence,
            MptAccountId { bytes: *holder },
            output.as_mut_ptr(),
        )
    };
    (result == 0).then(|| Uint256::from_void(&output))
}

pub fn get_convert_context_hash(
    account: &[u8; 20],
    issuance_id: &[u8; 24],
    sequence: u32,
) -> Option<Uint256> {
    let crypto = mpt_crypto().ok()?;
    let mut output = [0u8; 32];
    // SAFETY: all by-value wrappers and the output width match the official ABI.
    let result = unsafe {
        (crypto.get_convert_context_hash)(
            MptAccountId { bytes: *account },
            MptIssuanceId {
                bytes: *issuance_id,
            },
            sequence,
            output.as_mut_ptr(),
        )
    };
    (result == 0).then(|| Uint256::from_void(&output))
}

pub fn get_convert_back_context_hash(
    account: &[u8; 20],
    issuance_id: &[u8; 24],
    sequence: u32,
    version: u32,
) -> Option<Uint256> {
    let crypto = mpt_crypto().ok()?;
    let mut output = [0u8; 32];
    // SAFETY: all by-value wrappers and the output width match the official ABI.
    let result = unsafe {
        (crypto.get_convert_back_context_hash)(
            MptAccountId { bytes: *account },
            MptIssuanceId {
                bytes: *issuance_id,
            },
            sequence,
            version,
            output.as_mut_ptr(),
        )
    };
    (result == 0).then(|| Uint256::from_void(&output))
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
    let zero = encrypt_amount(0, pub_key, randomness)?;
    homomorphic_add(ciphertext, &zero)
}

pub fn encrypt_amount(amount: u64, pub_key: &[u8], blinding_factor: &[u8]) -> Option<Vec<u8>> {
    if pub_key.len() != EC_PUB_KEY_LENGTH
        || blinding_factor.len() != EC_BLINDING_FACTOR_LENGTH
        || !is_valid_compressed_ec_point(pub_key)
    {
        return None;
    }
    let crypto = mpt_crypto().ok()?;
    let mut ciphertext = vec![0u8; EC_GAMAL_ENCRYPTED_TOTAL_LENGTH];
    // SAFETY: every slice was width-checked above and the output has the ABI's
    // exact fixed capacity. The library passed its load-time ABI self-test.
    let result = unsafe {
        (crypto.encrypt_amount)(
            amount,
            pub_key.as_ptr(),
            blinding_factor.as_ptr(),
            ciphertext.as_mut_ptr(),
        )
    };
    (result == 0).then_some(ciphertext)
}

pub fn encrypt_canonical_zero_amount(
    pub_key: &[u8],
    account: &[u8; 20],
    mpt_id: &[u8],
) -> Option<Vec<u8>> {
    if pub_key.len() != EC_PUB_KEY_LENGTH
        || mpt_id.len() != 24
        || !is_valid_compressed_ec_point(pub_key)
    {
        return None;
    }
    // Exact port of mpt-crypto/1.0.2 generate_canonical_encrypted_zero:
    // SHA-256("EncZero" || AccountID || MPTokenIssuanceID), followed by
    // rejection sampling through chained SHA-256 until the scalar is valid.
    let mut hasher = Sha256::new();
    hasher.update(b"EncZero");
    hasher.update(account);
    hasher.update(mpt_id);
    let mut scalar: [u8; EC_SCALAR_LENGTH] = hasher.finalize().into();
    loop {
        if secp256k1::SecretKey::from_byte_array(scalar).is_ok() {
            break;
        }
        scalar = Sha256::digest(scalar).into();
    }
    encrypt_amount(0, pub_key, &scalar)
}

pub fn increment_confidential_version(current_version: Option<u32>) -> u32 {
    let current = current_version.unwrap_or(0);
    if current == u32::MAX { 0 } else { current + 1 }
}

use crate::Ter;

fn participant(pub_key: &[u8], ciphertext: &[u8]) -> Option<MptParticipant> {
    Some(MptParticipant {
        pubkey: pub_key.try_into().ok()?,
        ciphertext: ciphertext.try_into().ok()?,
    })
}

fn proof_result(result: i32) -> Ter {
    if result == 0 {
        Ter::TES_SUCCESS
    } else {
        Ter::TEC_BAD_PROOF
    }
}

pub fn verify_schnorr_proof(pub_key: &[u8], proof: &[u8], context_hash: &Uint256) -> Ter {
    if proof.len() != EC_SCHNORR_PROOF_LENGTH || pub_key.len() != EC_PUB_KEY_LENGTH {
        return Ter::TEC_INTERNAL;
    }
    let Ok(crypto) = mpt_crypto() else {
        return Ter::TEC_INTERNAL;
    };
    // SAFETY: widths are checked above and pointers remain live for the call.
    proof_result(unsafe {
        (crypto.verify_convert_proof)(
            proof.as_ptr(),
            pub_key.as_ptr(),
            context_hash.data().as_ptr(),
        )
    })
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
    if auditor_pub_key.is_some() != auditor_encrypted.is_some() {
        return Ter::TEC_INTERNAL;
    }
    let Some(holder) = participant(holder_pub_key, holder_encrypted) else {
        return Ter::TEC_INTERNAL;
    };
    let Some(issuer) = participant(issuer_pub_key, issuer_encrypted) else {
        return Ter::TEC_INTERNAL;
    };
    let auditor = match (auditor_pub_key, auditor_encrypted) {
        (Some(pub_key), Some(ciphertext)) => match participant(pub_key, ciphertext) {
            Some(value) => Some(value),
            None => return Ter::TEC_INTERNAL,
        },
        (None, None) => None,
        _ => return Ter::TEC_INTERNAL,
    };
    let Ok(crypto) = mpt_crypto() else {
        return Ter::TEC_INTERNAL;
    };
    // SAFETY: fixed-size C representations own their backing arrays.
    proof_result(unsafe {
        (crypto.verify_revealed_amount)(
            amount,
            blinding_factor.as_ptr(),
            &holder,
            &issuer,
            auditor
                .as_ref()
                .map_or(std::ptr::null(), |value| value as *const _),
        )
    })
}

pub fn verify_send_proof(
    proof: &[u8],
    recipients: &[(&[u8], &[u8])],
    spending_balance: &[u8],
    amount_commitment: &[u8],
    balance_commitment: &[u8],
    context_hash: &Uint256,
) -> Ter {
    if proof.len() != EC_SEND_PROOF_LENGTH
        || spending_balance.len() != EC_GAMAL_ENCRYPTED_TOTAL_LENGTH
        || amount_commitment.len() != EC_PEDERSEN_COMMITMENT_LENGTH
        || balance_commitment.len() != EC_PEDERSEN_COMMITMENT_LENGTH
        || !(recipients.len() == 3 || recipients.len() == 4)
    {
        return Ter::TEC_INTERNAL;
    }
    let Some(participants) = recipients
        .iter()
        .map(|(pub_key, ciphertext)| participant(pub_key, ciphertext))
        .collect::<Option<Vec<_>>>()
    else {
        return Ter::TEC_INTERNAL;
    };
    let Ok(crypto) = mpt_crypto() else {
        return Ter::TEC_INTERNAL;
    };
    // SAFETY: all proof/participant/commitment widths and count are checked.
    proof_result(unsafe {
        (crypto.verify_send_proof)(
            proof.as_ptr(),
            participants.as_ptr(),
            participants.len() as u8,
            spending_balance.as_ptr(),
            amount_commitment.as_ptr(),
            balance_commitment.as_ptr(),
            context_hash.data().as_ptr(),
        )
    })
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
    let Ok(crypto) = mpt_crypto() else {
        return Ter::TEC_INTERNAL;
    };
    // SAFETY: all fixed-width inputs were validated above.
    proof_result(unsafe {
        (crypto.verify_convert_back_proof)(
            proof.as_ptr(),
            pub_key.as_ptr(),
            spending_balance.as_ptr(),
            balance_commitment.as_ptr(),
            amount,
            context_hash.data().as_ptr(),
        )
    })
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
    let Ok(crypto) = mpt_crypto() else {
        return Ter::TEC_INTERNAL;
    };
    // SAFETY: all fixed-width inputs were validated above.
    proof_result(unsafe {
        (crypto.verify_clawback_proof)(
            proof.as_ptr(),
            amount,
            pub_key.as_ptr(),
            ciphertext.as_ptr(),
            context_hash.data().as_ptr(),
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[repr(C)]
    struct MptPedersenProofParams {
        pedersen_commitment: [u8; EC_PEDERSEN_COMMITMENT_LENGTH],
        amount: u64,
        ciphertext: [u8; EC_GAMAL_ENCRYPTED_TOTAL_LENGTH],
        blinding_factor: [u8; EC_BLINDING_FACTOR_LENGTH],
    }

    #[repr(C)]
    struct SecpPublicKey {
        data: [u8; 64],
    }

    struct OfficialProofGenerators {
        _library: Library,
        convert: unsafe extern "C" fn(*const u8, *const u8, *const u8, *mut u8) -> i32,
        pedersen: unsafe extern "C" fn(u64, *const u8, *mut u8) -> i32,
        send: unsafe extern "C" fn(
            *const u8,
            *const u8,
            u64,
            *const MptParticipant,
            usize,
            *const u8,
            *const u8,
            *const u8,
            *const MptPedersenProofParams,
            *mut u8,
            *mut usize,
        ) -> i32,
        convert_back: unsafe extern "C" fn(
            *const u8,
            *const u8,
            *const u8,
            u64,
            *const MptPedersenProofParams,
            *mut u8,
        ) -> i32,
        clawback:
            unsafe extern "C" fn(*const u8, *const u8, *const u8, u64, *const u8, *mut u8) -> i32,
        context: unsafe extern "C" fn() -> *mut std::ffi::c_void,
        parse_public: unsafe extern "C" fn(
            *const std::ffi::c_void,
            *mut SecpPublicKey,
            *const u8,
            usize,
        ) -> i32,
        canonical_zero: unsafe extern "C" fn(
            *const std::ffi::c_void,
            *mut SecpPublicKey,
            *mut SecpPublicKey,
            *const SecpPublicKey,
            *const u8,
            *const u8,
        ) -> i32,
        serialize_public: unsafe extern "C" fn(
            *const std::ffi::c_void,
            *mut u8,
            *mut usize,
            *const SecpPublicKey,
            u32,
        ) -> i32,
    }

    fn official_proof_generators() -> Option<OfficialProofGenerators> {
        let path = std::env::var_os("QUAXAR_MPT_CRYPTO_LIBRARY")?;
        // SAFETY: this is the operator-selected official test library. Every
        // generator is resolved with its mpt-crypto/1.0.2 public C signature.
        let library = unsafe { Library::new(path) }.expect("configured official test library");
        Some(unsafe {
            OfficialProofGenerators {
                convert: symbol(&library, b"mpt_get_convert_proof\0").expect("convert generator"),
                pedersen: symbol(&library, b"mpt_get_pedersen_commitment\0")
                    .expect("pedersen generator"),
                send: symbol(&library, b"mpt_get_confidential_send_proof\0")
                    .expect("send generator"),
                convert_back: symbol(&library, b"mpt_get_convert_back_proof\0")
                    .expect("convert-back generator"),
                clawback: symbol(&library, b"mpt_get_clawback_proof\0")
                    .expect("clawback generator"),
                context: symbol(&library, b"mpt_secp256k1_context\0").expect("shared context"),
                parse_public: symbol(&library, b"secp256k1_ec_pubkey_parse\0")
                    .expect("public-key parser"),
                canonical_zero: symbol(&library, b"generate_canonical_encrypted_zero\0")
                    .expect("canonical-zero generator"),
                serialize_public: symbol(&library, b"secp256k1_ec_pubkey_serialize\0")
                    .expect("public-key serializer"),
                _library: library,
            }
        })
    }

    fn keypair(secret: [u8; 32]) -> ([u8; 32], [u8; 33]) {
        let public = secp256k1::PublicKey::from_secret_key(
            &secp256k1::Secp256k1::new(),
            &secp256k1::SecretKey::from_byte_array(secret).expect("valid scalar"),
        )
        .serialize();
        (secret, public)
    }

    fn generator() -> Vec<u8> {
        secp256k1::PublicKey::from_secret_key(
            &secp256k1::Secp256k1::new(),
            &secp256k1::SecretKey::from_byte_array([1; 32]).expect("valid scalar"),
        )
        .serialize()
        .to_vec()
    }

    #[test]
    fn compressed_point_requires_an_on_curve_encoding() {
        assert!(is_valid_compressed_ec_point(&generator()));
        let mut invalid = vec![0xff; COMPRESSED_EC_POINT_LENGTH];
        invalid[0] = EC_COMPRESSED_PREFIX_EVEN_Y;
        assert!(!is_valid_compressed_ec_point(&invalid));
    }

    #[test]
    fn confidential_crypto_malformed_inputs_fail_closed_in_every_environment() {
        let key = generator();
        let ciphertext = [key.clone(), key.clone()].concat();
        let scalar = vec![1; EC_BLINDING_FACTOR_LENGTH];
        let context = Uint256::zero();
        let invalid_key = vec![0; EC_PUB_KEY_LENGTH];
        assert!(rerandomize_ciphertext(&ciphertext, &invalid_key, &scalar).is_none());
        assert!(encrypt_amount(1, &invalid_key, &scalar).is_none());
        assert!(encrypt_canonical_zero_amount(&invalid_key, &[0; 20], &[0; 24]).is_none());
        for result in [
            verify_schnorr_proof(&key, &[0; EC_SCHNORR_PROOF_LENGTH], &context),
            verify_revealed_amount(1, &scalar, &key, &ciphertext, &key, &ciphertext, None, None),
            verify_send_proof(
                &[0; EC_SEND_PROOF_LENGTH],
                &[
                    (&key, &ciphertext),
                    (&key, &ciphertext),
                    (&key, &ciphertext),
                ],
                &ciphertext,
                &key,
                &key,
                &context,
            ),
            verify_convert_back_proof(
                &[0; EC_CONVERT_BACK_PROOF_LENGTH],
                &key,
                &ciphertext,
                &key,
                1,
                &context,
            ),
            verify_clawback_proof(
                1,
                &[0; EC_CLAWBACK_PROOF_LENGTH],
                &key,
                &ciphertext,
                &context,
            ),
        ] {
            assert_ne!(result, Ter::TES_SUCCESS);
        }
    }

    #[test]
    fn explicit_missing_library_candidate_is_deterministically_unavailable() {
        let result = load_mpt_crypto_from(vec![PathBuf::from(
            "/quaxar-test/does-not-exist/libmpt-crypto.so",
        )]);
        assert!(result.is_err());
    }

    #[test]
    fn startup_capability_policy_never_silently_ignores_a_required_or_configured_library() {
        assert!(!confidential_crypto_required_at_startup(false, false));
        assert!(confidential_crypto_required_at_startup(true, false));
        assert!(confidential_crypto_required_at_startup(false, true));
        assert!(confidential_crypto_required_at_startup(true, true));
    }

    #[test]
    fn proof_sizes_match_official_mpt_crypto_1_0_2() {
        assert_eq!(EC_DOUBLE_BULLETPROOF_LENGTH, 754);
        assert_eq!(EC_SEND_SIGMA_PROOF_LENGTH, 192);
        assert_eq!(EC_SEND_PROOF_LENGTH, 946);
        assert_eq!(EC_CONVERT_BACK_PROOF_LENGTH, 816);
        assert_eq!(EC_CLAWBACK_PROOF_LENGTH, 64);
    }

    #[test]
    fn configured_official_abi_passes_positive_context_and_encryption_vectors() {
        if std::env::var_os("QUAXAR_MPT_CRYPTO_LIBRARY").is_none() {
            return;
        }
        assert!(
            confidential_crypto_available(),
            "configured official mpt-crypto library must load and pass its ABI self-test: {:?}",
            confidential_crypto_unavailable_reason()
        );

        let account = std::array::from_fn(|index| index as u8);
        let issuance = std::array::from_fn(|index| index as u8);
        let other = std::array::from_fn(|index| (index + 20) as u8);
        let expected = [
            "233f4a8552793b09d0480dc74674f60fa6cf90ea7554431e5036f4ee2e832d87",
            "911d49d6c86e4128aeb1b655c605f8b81a3620176618ee7ee224cc78cbadb78d",
            "bc6c0ff25c4ff58c35c52c2eb4ff00f222b0f8bd6d3bde52f0a88e7c095c73fc",
            "212634d39916295282e4956125cd39e962da71ee54cf8a27e1d89f5e58c39c9e",
        ]
        .map(decode_vector)
        .map(Uint256::from_array);
        assert_eq!(
            get_convert_context_hash(&account, &issuance, 0x0102_0304),
            Some(expected[0])
        );
        assert_eq!(
            get_convert_back_context_hash(&account, &issuance, 0x0102_0304, 0x0506_0708),
            Some(expected[1])
        );
        assert_eq!(
            get_send_context_hash(&account, &issuance, 0x0102_0304, &other, 0x0506_0708),
            Some(expected[2])
        );
        assert_eq!(
            get_clawback_context_hash(&account, &issuance, 0x0102_0304, &other),
            Some(expected[3])
        );

        let key = generator();
        let first = encrypt_canonical_zero_amount(&key, &account, &issuance)
            .expect("official ABI must encrypt canonical zero");
        let second = encrypt_canonical_zero_amount(&key, &account, &issuance)
            .expect("canonical zero encryption must be deterministic");
        assert_eq!(first, second);
        assert!(is_valid_ciphertext(&first));
    }

    #[test]
    fn configured_official_abi_generates_and_verifies_every_proof_family() {
        let Some(generators) = official_proof_generators() else {
            eprintln!("skipping official positive-proof vectors: no configured ABI library");
            return;
        };
        assert!(confidential_crypto_available());

        let context = Uint256::from_array([0x5a; 32]);
        let (holder_private, holder_public) = keypair([1; 32]);
        let (issuer_private, issuer_public) = keypair([2; 32]);
        let (_, destination_public) = keypair([3; 32]);
        let amount = 30u64;

        // Canonical zero is serialized into consensus state. Compare the
        // portable Rust path byte-for-byte with the exact low-level primitive
        // used by pinned rippled, not merely for decryptability/determinism.
        let account = [0x31; 20];
        let issuance = [0x32; 24];
        let portable_zero =
            encrypt_canonical_zero_amount(&holder_public, &account, &issuance).unwrap();
        let official_context = unsafe { (generators.context)() };
        assert!(!official_context.is_null());
        let mut parsed = SecpPublicKey { data: [0; 64] };
        let mut c1 = SecpPublicKey { data: [0; 64] };
        let mut c2 = SecpPublicKey { data: [0; 64] };
        assert_eq!(
            unsafe {
                (generators.parse_public)(
                    official_context,
                    &mut parsed,
                    holder_public.as_ptr(),
                    holder_public.len(),
                )
            },
            1
        );
        assert_eq!(
            unsafe {
                (generators.canonical_zero)(
                    official_context,
                    &mut c1,
                    &mut c2,
                    &parsed,
                    account.as_ptr(),
                    issuance.as_ptr(),
                )
            },
            1
        );
        let mut official_zero = [0u8; EC_GAMAL_ENCRYPTED_TOTAL_LENGTH];
        for (offset, point) in [(0, &c1), (EC_CIPHERTEXT_COMPONENT_LENGTH, &c2)] {
            let mut length = EC_CIPHERTEXT_COMPONENT_LENGTH;
            assert_eq!(
                unsafe {
                    (generators.serialize_public)(
                        official_context,
                        official_zero[offset..].as_mut_ptr(),
                        &mut length,
                        point,
                        258, // SECP256K1_EC_COMPRESSED
                    )
                },
                1
            );
            assert_eq!(length, EC_CIPHERTEXT_COMPONENT_LENGTH);
        }
        assert_eq!(portable_zero, official_zero);

        let mut convert_proof = [0u8; EC_SCHNORR_PROOF_LENGTH];
        // SAFETY: all buffers exactly match the public 1.0.2 ABI widths.
        assert_eq!(
            unsafe {
                (generators.convert)(
                    holder_public.as_ptr(),
                    holder_private.as_ptr(),
                    context.data().as_ptr(),
                    convert_proof.as_mut_ptr(),
                )
            },
            0
        );
        assert_eq!(
            verify_schnorr_proof(&holder_public, &convert_proof, &context),
            Ter::TES_SUCCESS
        );

        let transfer_blinding = [4u8; EC_BLINDING_FACTOR_LENGTH];
        let holder_amount = encrypt_amount(amount, &holder_public, &transfer_blinding).unwrap();
        let issuer_amount = encrypt_amount(amount, &issuer_public, &transfer_blinding).unwrap();
        assert_eq!(
            verify_revealed_amount(
                amount,
                &transfer_blinding,
                &holder_public,
                &holder_amount,
                &issuer_public,
                &issuer_amount,
                None,
                None,
            ),
            Ter::TES_SUCCESS
        );

        let balance = 100u64;
        let balance_blinding = [5u8; EC_BLINDING_FACTOR_LENGTH];
        let spending = encrypt_amount(balance, &holder_public, &balance_blinding).unwrap();
        let mut balance_commitment = [0u8; EC_PEDERSEN_COMMITMENT_LENGTH];
        assert_eq!(
            unsafe {
                (generators.pedersen)(
                    balance,
                    balance_blinding.as_ptr(),
                    balance_commitment.as_mut_ptr(),
                )
            },
            0
        );
        let balance_params = MptPedersenProofParams {
            pedersen_commitment: balance_commitment,
            amount: balance,
            ciphertext: spending.clone().try_into().unwrap(),
            blinding_factor: balance_blinding,
        };

        let mut convert_back_proof = [0u8; EC_CONVERT_BACK_PROOF_LENGTH];
        assert_eq!(
            unsafe {
                (generators.convert_back)(
                    holder_private.as_ptr(),
                    holder_public.as_ptr(),
                    context.data().as_ptr(),
                    amount,
                    &balance_params,
                    convert_back_proof.as_mut_ptr(),
                )
            },
            0
        );
        assert_eq!(
            verify_convert_back_proof(
                &convert_back_proof,
                &holder_public,
                &spending,
                &balance_commitment,
                amount,
                &context,
            ),
            Ter::TES_SUCCESS
        );

        let mut clawback_proof = [0u8; EC_CLAWBACK_PROOF_LENGTH];
        assert_eq!(
            unsafe {
                (generators.clawback)(
                    issuer_private.as_ptr(),
                    issuer_public.as_ptr(),
                    context.data().as_ptr(),
                    amount,
                    issuer_amount.as_ptr(),
                    clawback_proof.as_mut_ptr(),
                )
            },
            0
        );
        assert_eq!(
            verify_clawback_proof(
                amount,
                &clawback_proof,
                &issuer_public,
                &issuer_amount,
                &context,
            ),
            Ter::TES_SUCCESS
        );

        let destination_amount =
            encrypt_amount(amount, &destination_public, &transfer_blinding).unwrap();
        let participants = [
            participant(&holder_public, &holder_amount).unwrap(),
            participant(&destination_public, &destination_amount).unwrap(),
            participant(&issuer_public, &issuer_amount).unwrap(),
        ];
        let mut amount_commitment = [0u8; EC_PEDERSEN_COMMITMENT_LENGTH];
        assert_eq!(
            unsafe {
                (generators.pedersen)(
                    amount,
                    transfer_blinding.as_ptr(),
                    amount_commitment.as_mut_ptr(),
                )
            },
            0
        );
        let mut send_proof = [0u8; EC_SEND_PROOF_LENGTH];
        let mut send_proof_len = send_proof.len();
        assert_eq!(
            unsafe {
                (generators.send)(
                    holder_private.as_ptr(),
                    holder_public.as_ptr(),
                    amount,
                    participants.as_ptr(),
                    participants.len(),
                    transfer_blinding.as_ptr(),
                    context.data().as_ptr(),
                    amount_commitment.as_ptr(),
                    &balance_params,
                    send_proof.as_mut_ptr(),
                    &mut send_proof_len,
                )
            },
            0
        );
        assert_eq!(send_proof_len, EC_SEND_PROOF_LENGTH);
        assert_eq!(
            verify_send_proof(
                &send_proof,
                &[
                    (&holder_public, holder_amount.as_slice()),
                    (&destination_public, destination_amount.as_slice()),
                    (&issuer_public, issuer_amount.as_slice()),
                ],
                &spending,
                &amount_commitment,
                &balance_commitment,
                &context,
            ),
            Ter::TES_SUCCESS
        );
    }
}
