//! Rust port of the current `xrpl::base64_*` behavior.
//!
//! Important Rust note:
//! - reference `std::string` can hold arbitrary bytes.
//! - Rust `String` must be valid UTF-8.
//!
//! Because decoded base64 may produce arbitrary bytes, the decode path returns
//! `Vec<u8>` rather than `String`. This is a more accurate Rust model for the
//! underlying data.

const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

fn encoded_size(n: usize) -> usize {
    4 * n.div_ceil(3)
}

fn decoded_size(n: usize) -> usize {
    ((n / 4) * 3) + 2
}

fn decode_value(byte: u8) -> Option<u8> {
    match byte {
        b'A'..=b'Z' => Some(byte - b'A'),
        b'a'..=b'z' => Some(byte - b'a' + 26),
        b'0'..=b'9' => Some(byte - b'0' + 52),
        b'+' => Some(62),
        b'/' => Some(63),
        _ => None,
    }
}

/// Encode bytes as padded base64, matching the the reference implementation behavior.
pub fn base64_encode(data: &[u8]) -> String {
    let mut out = String::with_capacity(encoded_size(data.len()));

    for chunk in data.chunks_exact(3) {
        out.push(ALPHABET[((chunk[0] & 0xfc) >> 2) as usize] as char);
        out.push(ALPHABET[(((chunk[0] & 0x03) << 4) + ((chunk[1] & 0xf0) >> 4)) as usize] as char);
        out.push(ALPHABET[(((chunk[2] & 0xc0) >> 6) + ((chunk[1] & 0x0f) << 2)) as usize] as char);
        out.push(ALPHABET[(chunk[2] & 0x3f) as usize] as char);
    }

    let remainder = data.chunks_exact(3).remainder();
    match remainder {
        [a, b] => {
            out.push(ALPHABET[((a & 0xfc) >> 2) as usize] as char);
            out.push(ALPHABET[(((a & 0x03) << 4) + ((b & 0xf0) >> 4)) as usize] as char);
            out.push(ALPHABET[((b & 0x0f) << 2) as usize] as char);
            out.push('=');
        }
        [a] => {
            out.push(ALPHABET[((a & 0xfc) >> 2) as usize] as char);
            out.push(ALPHABET[((a & 0x03) << 4) as usize] as char);
            out.push('=');
            out.push('=');
        }
        [] => {}
        _ => unreachable!("chunks_exact(3) remainder can only be length 0, 1, or 2"),
    }

    out
}

/// Convenience helper mirroring the inline string overload in the reference header.
pub fn base64_encode_str(data: &str) -> String {
    base64_encode(data.as_bytes())
}

/// Decode base64 bytes, stopping on `=` or the first invalid character.
///
/// This matches the the reference implementation behavior used by the existing tests.
pub fn base64_decode(data: &str) -> Vec<u8> {
    let input = data.as_bytes();
    let mut out = Vec::with_capacity(decoded_size(input.len()));
    let mut c4 = [0u8; 4];
    let mut i = 0usize;
    let mut index = 0usize;

    while index < input.len() && input[index] != b'=' {
        let Some(value) = decode_value(input[index]) else {
            break;
        };

        index += 1;
        c4[i] = value;
        i += 1;

        if i == 4 {
            out.push((c4[0] << 2) + ((c4[1] & 0x30) >> 4));
            out.push(((c4[1] & 0x0f) << 4) + ((c4[2] & 0x3c) >> 2));
            out.push(((c4[2] & 0x03) << 6) + c4[3]);
            i = 0;
        }
    }

    if i != 0 {
        let c3 = [
            (c4[0] << 2) + ((c4[1] & 0x30) >> 4),
            ((c4[1] & 0x0f) << 4) + ((c4[2] & 0x3c) >> 2),
            ((c4[2] & 0x03) << 6) + c4[3],
        ];

        for byte in c3.iter().take(i - 1) {
            out.push(*byte);
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::{base64_decode, base64_encode_str};

    fn check(input: &str, output: &str) {
        let encoded = base64_encode_str(input);
        assert_eq!(encoded, output);
        assert_eq!(base64_decode(&encoded), input.as_bytes());
    }

    #[test]
    fn matches_cpp_base64_examples() {
        check("", "");
        check("f", "Zg==");
        check("fo", "Zm8=");
        check("foo", "Zm9v");
        check("foob", "Zm9vYg==");
        check("fooba", "Zm9vYmE=");
        check("foobar", "Zm9vYmFy");

        check(
            "Man is distinguished, not only by his reason, but by this singular passion from other animals, which is a lust of the mind, that by a perseverance of delight in the continued and indefatigable generation of knowledge, exceeds the short vehemence of any carnal pleasure.",
            "TWFuIGlzIGRpc3Rpbmd1aXNoZWQsIG5vdCBvbmx5IGJ5IGhpcyByZWFzb24sIGJ1dCBieSB0aGlzIHNpbmd1bGFyIHBhc3Npb24gZnJvbSBvdGhlciBhbmltYWxzLCB3aGljaCBpcyBhIGx1c3Qgb2YgdGhlIG1pbmQsIHRoYXQgYnkgYSBwZXJzZXZlcmFuY2Ugb2YgZGVsaWdodCBpbiB0aGUgY29udGludWVkIGFuZCBpbmRlZmF0aWdhYmxlIGdlbmVyYXRpb24gb2Yga25vd2xlZGdlLCBleGNlZWRzIHRoZSBzaG9ydCB2ZWhlbWVuY2Ugb2YgYW55IGNhcm5hbCBwbGVhc3VyZS4=",
        );

        let not_base64 = "not_base64!!";
        let truncated = "not";
        assert_eq!(base64_decode(not_base64), base64_decode(truncated));
    }
}
