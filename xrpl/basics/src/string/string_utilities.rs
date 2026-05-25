//! Rust port of `xrpl/basics/StringUtilities.h`.

use crate::blob::Blob;
use crate::str_hex::str_hex;
use regex::Regex;
use std::net::{Ipv4Addr, Ipv6Addr};
use std::sync::OnceLock;

/// Format arbitrary binary data as an SQLite blob literal.
pub fn sql_blob_literal(blob: &Blob) -> String {
    let mut output = String::with_capacity(blob.len() * 2 + 3);
    output.push('X');
    output.push('\'');
    output.push_str(&str_hex(blob));
    output.push('\'');
    output
}

pub fn str_unhex_iter<I>(str_size: usize, iter: I) -> Option<Blob>
where
    I: IntoIterator<Item = char>,
{
    let mut output = Blob::with_capacity(str_size.div_ceil(2));
    let mut iter = iter.into_iter();

    if str_size & 1 == 1 {
        let high = decode_hex_digit(iter.next()?)?;
        output.push(high as u8);
    }

    while let Some(high_char) = iter.next() {
        let high = decode_hex_digit(high_char)?;
        let low = decode_hex_digit(iter.next()?)?;
        output.push(((high << 4) | low) as u8);
    }

    Some(output)
}

pub fn str_unhex(input: &str) -> Option<Blob> {
    str_unhex_iter(input.len(), input.chars())
}

pub fn str_view_unhex(input: &str) -> Option<Blob> {
    str_unhex(input)
}

#[derive(Debug, Clone, Default)]
pub struct ParsedUrl {
    pub scheme: String,
    pub username: String,
    pub password: String,
    pub domain: String,
    pub port: Option<u16>,
    pub path: String,
}

impl PartialEq for ParsedUrl {
    fn eq(&self, other: &Self) -> bool {
        self.scheme == other.scheme
            && self.domain == other.domain
            && self.port == other.port
            && self.path == other.path
    }
}

impl Eq for ParsedUrl {}

pub fn parse_url(input: &str) -> Option<ParsedUrl> {
    let captures = url_regex().captures(input)?;

    let scheme = captures.get(1)?.as_str().to_ascii_lowercase();
    let username = captures
        .get(2)
        .map_or_else(String::new, |capture| capture.as_str().to_owned());
    let password = captures
        .get(3)
        .map_or_else(String::new, |capture| capture.as_str().to_owned());
    let domain_raw = captures
        .get(4)
        .map_or_else(String::new, |capture| capture.as_str().to_owned());
    let domain = normalize_domain(&domain_raw);

    let port = match captures.get(5) {
        Some(capture) => match capture.as_str().parse::<u16>() {
            Ok(value) => Some(value),
            Err(_) => return None,
        },
        None => None,
    };
    if port == Some(0) {
        return None;
    }

    let path = captures
        .get(6)
        .map_or_else(String::new, |capture| capture.as_str().to_owned());

    Some(ParsedUrl {
        scheme,
        username,
        password,
        domain,
        port,
        path,
    })
}

pub fn parse_url_into(output: &mut ParsedUrl, input: &str) -> bool {
    if let Some(parsed) = parse_url(input) {
        *output = parsed;
        true
    } else {
        false
    }
}

pub fn trim_whitespace(input: String) -> String {
    trim_ascii_whitespace(&input).to_owned()
}

pub fn to_uint64(input: &str) -> Option<u64> {
    input.parse::<u64>().ok()
}

pub fn is_properly_formed_toml_domain(domain: &str) -> bool {
    if domain.len() < 4 || domain.len() > 128 {
        return false;
    }

    let mut labels = domain.split('.').peekable();
    if labels.peek().is_none() {
        return false;
    }

    let mut seen_tld = false;
    let mut segment_count = 0usize;
    while let Some(label) = labels.next() {
        segment_count += 1;
        let is_last = labels.peek().is_none();
        if label.is_empty() || label.len() > 63 {
            return false;
        }

        if is_last {
            seen_tld = true;
            if label.len() < 2 || !label.chars().all(|ch| ch.is_ascii_alphabetic()) {
                return false;
            }
            continue;
        }

        if label.starts_with('-') || label.ends_with('-') {
            return false;
        }

        if !label
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '-')
        {
            return false;
        }
    }

    seen_tld && segment_count >= 2
}

fn decode_hex_digit(ch: char) -> Option<u8> {
    match ch {
        '0'..='9' => Some((ch as u8) - b'0'),
        'A'..='F' => Some((ch as u8) - b'A' + 10),
        'a'..='f' => Some((ch as u8) - b'a' + 10),
        _ => None,
    }
}

fn trim_ascii_whitespace(input: &str) -> &str {
    let bytes = input.as_bytes();
    let mut start = 0usize;
    let mut end = bytes.len();

    while start < end && is_ascii_whitespace(bytes[start]) {
        start += 1;
    }

    while end > start && is_ascii_whitespace(bytes[end - 1]) {
        end -= 1;
    }

    &input[start..end]
}

const fn is_ascii_whitespace(byte: u8) -> bool {
    matches!(byte, b' ' | b'\t' | b'\n' | b'\x0b' | b'\x0c' | b'\r')
}

fn url_regex() -> &'static Regex {
    static URL_REGEX: OnceLock<Regex> = OnceLock::new();
    URL_REGEX.get_or_init(|| {
        Regex::new(
            r"(?i)^\s*([a-z][\-+\.a-z0-9]*):\/\/(?:([^:@/]*?)(?::([^@/]*?))?@)?([0-9:]*[0-9]|\[[^]]+\]|[^:/?#]*?)(?::([0-9]+))?(\/.*)?\s*$",
        )
        .expect("URL parsing regex should compile")
    })
}

fn normalize_domain(domain: &str) -> String {
    if let Some(stripped) = domain
        .strip_prefix('[')
        .and_then(|value| value.strip_suffix(']'))
        && let Ok(ipv6) = stripped.parse::<Ipv6Addr>()
    {
        return ipv6.to_string();
    }

    if let Ok(ipv4) = domain.parse::<Ipv4Addr>() {
        return ipv4.to_string();
    }

    if let Ok(ipv6) = domain.parse::<Ipv6Addr>() {
        if should_use_mixed_ipv6_rendering(ipv6) {
            return to_mixed_ipv6_string(ipv6);
        }
        return ipv6.to_string();
    }

    domain.to_owned()
}

fn should_use_mixed_ipv6_rendering(ipv6: Ipv6Addr) -> bool {
    let segments = ipv6.segments();
    segments[..6] == [0, 0, 0, 0, 0, 0] && (segments[6] != 0 || segments[7] > 0x00ff)
}

fn to_mixed_ipv6_string(ipv6: Ipv6Addr) -> String {
    let octets = ipv6.octets();
    format!(
        "::{}.{}.{}.{}",
        octets[12], octets[13], octets[14], octets[15]
    )
}

#[cfg(test)]
mod tests {
    use super::{
        ParsedUrl, is_properly_formed_toml_domain, parse_url, parse_url_into, sql_blob_literal,
        str_unhex, str_view_unhex, to_uint64, trim_whitespace,
    };

    #[test]
    fn str_unhex_examples() {
        assert_eq!(str_unhex("526970706c6544").unwrap(), b"RippleD");
        assert_eq!(str_unhex("A").unwrap(), b"\n");
        assert_eq!(str_unhex("0A").unwrap(), b"\n");
        assert_eq!(str_unhex("D0A").unwrap(), b"\r\n");
        assert_eq!(str_unhex("0D0A").unwrap(), b"\r\n");
        assert_eq!(str_unhex("200D0A").unwrap(), b" \r\n");
        assert_eq!(str_unhex("282A2B2C2D2E2F29").unwrap(), b"(*+,-./)");
        assert!(str_unhex("123X").is_none());
        assert!(str_unhex("V").is_none());
        assert!(str_unhex("XRP").is_none());
        assert_eq!(str_view_unhex("0D0A").unwrap(), b"\r\n");
    }

    #[test]
    fn sql_blob_literal_wraps_uppercase_hex() {
        assert_eq!(
            sql_blob_literal(&vec![0xde, 0xad, 0xbe, 0xef]),
            "X'DEADBEEF'"
        );
    }

    #[test]
    fn parse_url_cases() {
        let cases = [
            (
                "scheme://",
                ParsedUrl {
                    scheme: "scheme".into(),
                    username: String::new(),
                    password: String::new(),
                    domain: String::new(),
                    port: None,
                    path: String::new(),
                },
            ),
            (
                "scheme:///",
                ParsedUrl {
                    scheme: "scheme".into(),
                    username: String::new(),
                    password: String::new(),
                    domain: String::new(),
                    port: None,
                    path: "/".into(),
                },
            ),
            (
                "lower://domain",
                ParsedUrl {
                    scheme: "lower".into(),
                    username: String::new(),
                    password: String::new(),
                    domain: "domain".into(),
                    port: None,
                    path: String::new(),
                },
            ),
            (
                "UPPER://domain:234/",
                ParsedUrl {
                    scheme: "upper".into(),
                    username: String::new(),
                    password: String::new(),
                    domain: "domain".into(),
                    port: Some(234),
                    path: "/".into(),
                },
            ),
            (
                "Mixed://domain/path",
                ParsedUrl {
                    scheme: "mixed".into(),
                    username: String::new(),
                    password: String::new(),
                    domain: "domain".into(),
                    port: None,
                    path: "/path".into(),
                },
            ),
            (
                "scheme://[::1]:123/path",
                ParsedUrl {
                    scheme: "scheme".into(),
                    username: String::new(),
                    password: String::new(),
                    domain: "::1".into(),
                    port: Some(123),
                    path: "/path".into(),
                },
            ),
            (
                "scheme://user:pass@domain:123/abc:321",
                ParsedUrl {
                    scheme: "scheme".into(),
                    username: "user".into(),
                    password: "pass".into(),
                    domain: "domain".into(),
                    port: Some(123),
                    path: "/abc:321".into(),
                },
            ),
            (
                "scheme://:999/",
                ParsedUrl {
                    scheme: "scheme".into(),
                    username: String::new(),
                    password: String::new(),
                    domain: ":999".into(),
                    port: None,
                    path: "/".into(),
                },
            ),
            (
                "http://::1:1234/validators",
                ParsedUrl {
                    scheme: "http".into(),
                    username: String::new(),
                    password: String::new(),
                    domain: "::0.1.18.52".into(),
                    port: None,
                    path: "/validators".into(),
                },
            ),
        ];

        for (input, expected) in cases {
            assert_eq!(parse_url(input).unwrap(), expected);
        }

        let mut output = ParsedUrl::default();
        assert!(parse_url_into(&mut output, "scheme://domain/abc:321"));
        assert_eq!(output.scheme, "scheme");
        assert_eq!(output.domain, "domain");
        assert_eq!(output.path, "/abc:321");
    }

    #[test]
    fn parse_url_rejects_cpp_failure_cases() {
        let too_many_colons = format!("s://{}", ":".repeat(8192));

        for input in [
            "",
            "nonsense",
            "://",
            ":///",
            "scheme://user:pass@domain:65536/abc:321",
            "UPPER://domain:23498765/",
            "UPPER://domain:0/",
            "UPPER://domain:+7/",
            "UPPER://domain:-7234/",
            "UPPER://domain:@#$56!/",
            too_many_colons.as_str(),
        ] {
            assert!(parse_url(input).is_none(), "{input}");
        }
    }

    #[test]
    fn trim_and_uint_helpers_match_expected_role() {
        assert_eq!(trim_whitespace("  hello \n".to_owned()), "hello");
        assert_eq!(
            trim_whitespace("\u{00a0} hello \u{00a0}".to_owned()),
            "\u{00a0} hello \u{00a0}"
        );
        assert_eq!(to_uint64("42"), Some(42));
        assert_eq!(to_uint64("18446744073709551615"), Some(u64::MAX));
        assert_eq!(to_uint64("-1"), None);
        assert_eq!(to_uint64(" 7 "), None);
        assert_eq!(to_uint64("abc"), None);
    }

    #[test]
    fn toml_domain_check_matches_expected_shape() {
        assert!(is_properly_formed_toml_domain("example.com"));
        assert!(is_properly_formed_toml_domain("xrpl-validators.org"));
        assert!(!is_properly_formed_toml_domain("a.b"));
        assert!(!is_properly_formed_toml_domain("-example.com"));
        assert!(!is_properly_formed_toml_domain("example-.com"));
        assert!(!is_properly_formed_toml_domain("example.123"));
        assert!(!is_properly_formed_toml_domain("example"));
    }
}
