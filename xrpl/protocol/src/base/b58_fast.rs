//! Fast base58 encoding for XRPL tokens, ported from rippled's `b58_fast`
//! (`src/libxrpl/protocol/tokens.cpp`, `include/xrpl/protocol/detail/b58_utils.h`).
//!
//! The textbook algorithm converts base 256 -> base 58 one digit at a time
//! over a big integer. This converts base 256 -> base 2^64 (trivial), then
//! base 2^64 -> base 58^10 (multi-precision, but on 64-bit limbs), then
//! base 58^10 -> base 58 (trivial). 58^10 is the largest power of 58 that
//! fits in a `u64`. rippled reports 10-15x over the reference algorithm.
//!
//! Output is identical to standard base58 (each leading zero byte becomes the
//! first alphabet character), so it is a drop-in replacement for `bs58`.

/// XRPL base58 alphabet.
pub const ALPHABET: &[u8; 58] = b"rpshnaf39wBUDNEGHJKLM4PQRST7VWXYZ2bcdeCg65jkm8oFqi1tuvAxyz";

/// Largest input `b58_fast` accepts: 33-byte node public key + 1 type byte +
/// 4 checksum bytes.
pub const MAX_INPUT: usize = 38;

/// ceil(38 * 8 / log2(58)) = 52 digits.
pub const MAX_OUTPUT: usize = 52;

const B58_10: u64 = 430_804_206_899_405_824; // 58^10
const B58_5: u32 = 656_356_768; // 58^5

// Division of a 128-bit value by the constant 58^10 via a precomputed
// reciprocal (Moller & Granlund, "Improved division by invariant integers",
// IEEE Trans. Computers 60(2), 2011, Algorithm 4; GMP's udiv_qrnnd_preinv).
// Avoids the software `__udivti3` call a plain `u128 / u64` compiles to.
const NORM_SHIFT: u32 = B58_10.leading_zeros();
const NORM_D: u64 = B58_10 << NORM_SHIFT;
const RECIPROCAL: u64 = ((u128::MAX / NORM_D as u128) - (1_u128 << 64)) as u64;

/// `(rem * 2^64 + limb) / 58^10` and its remainder, given `rem < 58^10`.
#[inline(always)]
fn div_rem_b58_10(rem: u64, limb: u64) -> (u64, u64) {
    debug_assert!(rem < B58_10);
    // Normalize so the divisor's top bit is set.
    let u1 = (rem << NORM_SHIFT) | (limb >> (64 - NORM_SHIFT));
    let u0 = limb << NORM_SHIFT;
    let product =
        u128::from(RECIPROCAL) * u128::from(u1) + ((u128::from(u1) << 64) | u128::from(u0));
    let mut q1 = ((product >> 64) as u64).wrapping_add(1);
    let q0 = product as u64;
    let mut r = u0.wrapping_sub(q1.wrapping_mul(NORM_D));
    if r > q0 {
        q1 = q1.wrapping_sub(1);
        r = r.wrapping_add(NORM_D);
    }
    if r >= NORM_D {
        q1 += 1;
        r -= NORM_D;
    }
    (q1, r >> NORM_SHIFT)
}

/// Divide the little-endian (lowest limb first) big integer `limbs` in place
/// by 58^10, returning the remainder (`inplaceBigintDivRem`).
#[inline]
fn div_rem_in_place(limbs: &mut [u64]) -> u64 {
    let mut rem = 0_u64;
    for limb in limbs.iter_mut().rev() {
        let (quotient, remainder) = div_rem_b58_10(rem, *limb);
        *limb = quotient;
        rem = remainder;
    }
    rem
}

/// Base 58^10 coefficient -> 10 base-58 digits, most significant first
/// (`b5810ToB58Be`). Split into two base-58^5 halves so each half uses
/// 32-bit constant division, which the compiler lowers to multiplications.
#[inline]
fn b58_10_digits(value: u64) -> [u8; 10] {
    debug_assert!(value < B58_10);
    let mut hi = (value / u64::from(B58_5)) as u32;
    let mut lo = (value % u64::from(B58_5)) as u32;
    let mut digits = [0_u8; 10];
    for i in (5..10).rev() {
        digits[i] = (lo % 58) as u8;
        lo /= 58;
    }
    for i in (0..5).rev() {
        digits[i] = (hi % 58) as u8;
        hi /= 58;
    }
    digits
}

/// Encode `input` (at most [`MAX_INPUT`] bytes) into `out`, returning the
/// number of characters written. Returns `None` if `input` is too long.
pub fn encode_into(input: &[u8], out: &mut [u8; MAX_OUTPUT]) -> Option<usize> {
    if input.len() > MAX_INPUT {
        return None;
    }
    let zeros = input.iter().take_while(|byte| **byte == 0).count();
    let rest = &input[zeros..];

    // Big-endian bytes -> little-endian u64 limbs (lowest limb first).
    let mut limbs = [0_u64; 5];
    let mut num_limbs = 0;
    let mut end = rest.len();
    while end > 0 {
        let start = end.saturating_sub(8);
        let mut limb = 0_u64;
        for byte in &rest[start..end] {
            limb = (limb << 8) | u64::from(*byte);
        }
        limbs[num_limbs] = limb;
        num_limbs += 1;
        end = start;
    }

    // Base 2^64 -> base 58^10 coefficients, lowest first.
    let mut coeffs = [0_u64; 6];
    let mut num_coeffs = 0;
    let mut top = num_limbs;
    while top > 0 {
        coeffs[num_coeffs] = div_rem_in_place(&mut limbs[..top]);
        num_coeffs += 1;
        while top > 0 && limbs[top - 1] == 0 {
            top -= 1;
        }
    }

    out[..zeros].fill(ALPHABET[0]);
    let mut pos = zeros;
    let mut leading = true;
    for coeff in coeffs[..num_coeffs].iter().rev() {
        if leading && *coeff == 0 {
            continue;
        }
        let digits = b58_10_digits(*coeff);
        let skip = if leading {
            leading = false;
            digits.iter().take_while(|digit| **digit == 0).count()
        } else {
            0
        };
        for digit in &digits[skip..] {
            out[pos] = ALPHABET[usize::from(*digit)];
            pos += 1;
        }
    }
    Some(pos)
}

/// Encode `input` as an XRPL-alphabet base58 string. Inputs longer than
/// [`MAX_INPUT`] fall back to the generic implementation.
pub fn encode(input: &[u8]) -> String {
    let mut buf = [0_u8; MAX_OUTPUT];
    match encode_into(input, &mut buf) {
        Some(len) => {
            let mut text = String::with_capacity(len);
            // Every byte comes from the ASCII alphabet.
            text.push_str(std::str::from_utf8(&buf[..len]).expect("base58 alphabet is ASCII"));
            text
        }
        None => encode_generic(input),
    }
}

fn encode_generic(input: &[u8]) -> String {
    static ALPHABET_BS58: std::sync::OnceLock<bs58::Alphabet> = std::sync::OnceLock::new();
    let alphabet = ALPHABET_BS58
        .get_or_init(|| bs58::Alphabet::new(ALPHABET).expect("XRPL base58 alphabet is valid"));
    bs58::encode(input).with_alphabet(alphabet).into_string()
}

/// `type || token || checksum(type || token)` as a base58 string, with the
/// payload built on the stack (`encodeBase58Token`).
pub fn encode_token(token_type: u8, token: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let len = 1 + token.len() + 4;
    if len > MAX_INPUT {
        let mut payload = Vec::with_capacity(len);
        payload.push(token_type);
        payload.extend_from_slice(token);
        let digest = Sha256::digest(Sha256::digest(&payload));
        payload.extend_from_slice(&digest[..4]);
        return encode_generic(&payload);
    }
    let mut payload = [0_u8; MAX_INPUT];
    payload[0] = token_type;
    payload[1..1 + token.len()].copy_from_slice(token);
    let digest = Sha256::digest(Sha256::digest(&payload[..1 + token.len()]));
    payload[1 + token.len()..len].copy_from_slice(&digest[..4]);
    encode(&payload[..len])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reference(input: &[u8]) -> String {
        encode_generic(input)
    }

    struct XorShift(u64);

    impl XorShift {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }
    }

    #[test]
    fn matches_reference_for_random_inputs_of_every_length() {
        let mut rng = XorShift(0x2545_F491_4F6C_DD1D);
        for len in 0..=MAX_INPUT {
            for _ in 0..2_000 {
                let mut input: Vec<u8> = (0..len).map(|_| rng.next() as u8).collect();
                // Exercise leading-zero runs of every size.
                let zeros = (rng.next() as usize) % (len + 1);
                input[..zeros].fill(0);
                assert_eq!(encode(&input), reference(&input), "{input:02X?}");
            }
        }
    }

    #[test]
    fn matches_reference_on_edge_values() {
        let mut cases: Vec<Vec<u8>> = vec![
            vec![],
            vec![0],
            vec![0; MAX_INPUT],
            vec![0xFF; MAX_INPUT],
            vec![1],
            vec![57],
            vec![58],
        ];
        for len in 1..=MAX_INPUT {
            cases.push(vec![0xFF; len]);
            let mut one = vec![0; len];
            one[len - 1] = 1;
            cases.push(one);
            let mut high = vec![0; len];
            high[0] = 0x80;
            cases.push(high);
        }
        // Values around 58^10 and 58^5 boundaries.
        for value in [
            B58_10 - 1,
            B58_10,
            B58_10 + 1,
            u64::from(B58_5) - 1,
            u64::from(B58_5),
        ] {
            cases.push(value.to_be_bytes().to_vec());
        }
        for case in cases {
            assert_eq!(encode(&case), reference(&case), "{case:02X?}");
        }
    }

    #[test]
    fn reciprocal_division_matches_hardware_division() {
        let mut rng = XorShift(0xD1B5_4A32_D192_ED03);
        let divisor = u128::from(B58_10);
        let mut check = |rem: u64, limb: u64| {
            let num = (u128::from(rem) << 64) | u128::from(limb);
            let (q, r) = div_rem_b58_10(rem, limb);
            assert_eq!(u128::from(q), num / divisor, "{rem} {limb}");
            assert_eq!(u128::from(r), num % divisor, "{rem} {limb}");
        };
        for rem in [0, 1, B58_10 - 1, B58_10 / 2] {
            for limb in [0, 1, u64::MAX, u64::MAX - 1, B58_10, B58_10 - 1, 1 << 63] {
                check(rem, limb);
            }
        }
        for _ in 0..1_000_000 {
            check(rng.next() % B58_10, rng.next());
        }
    }

    #[test]
    fn long_inputs_fall_back() {
        let input = vec![0xAB; MAX_INPUT + 10];
        assert_eq!(encode(&input), reference(&input));
    }

    #[test]
    fn known_xrpl_vectors() {
        // Genesis account (rippled AccountID tests / docs).
        let genesis = [
            0xB5, 0xF7, 0x62, 0x79, 0x8A, 0x53, 0xD5, 0x43, 0xA0, 0x14, 0xCA, 0xF8, 0xB2, 0x97,
            0xCF, 0xF8, 0xF2, 0xF9, 0x37, 0xE8,
        ];
        assert_eq!(
            encode_token(0, &genesis),
            "rHb9CJAWyB4rj91VRWn96DkukG4bwdtyTh"
        );
        // Account zero and account one (well-known XRPL special accounts).
        assert_eq!(encode_token(0, &[0; 20]), "rrrrrrrrrrrrrrrrrrrrrhoLvTp");
        let mut one = [0_u8; 20];
        one[19] = 1;
        assert_eq!(encode_token(0, &one), "rrrrrrrrrrrrrrrrrrrrBZbvji");
    }
}
