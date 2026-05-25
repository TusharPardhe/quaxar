//! Build metadata helpers from `xrpl/protocol/BuildInfo.*`.

use std::sync::OnceLock;

use crate::system_name;

pub const VERSION_STRING: &str = "3.2.0-b0";
const IMPLEMENTATION_VERSION_IDENTIFIER: u64 = 0x183B_0000_0000_0000;
const IMPLEMENTATION_VERSION_IDENTIFIER_MASK: u64 = 0xFFFF_0000_0000_0000;

pub fn get_version_string() -> &'static str {
    VERSION_STRING
}

pub fn get_full_version_string() -> &'static str {
    static FULL: OnceLock<String> = OnceLock::new();
    FULL.get_or_init(|| format!("{}-{}", system_name(), get_version_string()))
}

pub fn encode_software_version(version: &str) -> u64 {
    let mut encoded = IMPLEMENTATION_VERSION_IDENTIFIER;
    let (core, pre_release) = version
        .split_once('-')
        .map_or((version, ""), |(core, pre)| (core, pre));
    let mut pieces = core.split('.');
    let major = pieces.next().and_then(|v| v.parse::<u8>().ok());
    let minor = pieces.next().and_then(|v| v.parse::<u8>().ok());
    let patch = pieces.next().and_then(|v| v.parse::<u8>().ok());

    if let Some(major) = major {
        encoded |= u64::from(major) << 40;
    }
    if let Some(minor) = minor {
        encoded |= u64::from(minor) << 32;
    }
    if let Some(patch) = patch {
        encoded |= u64::from(patch) << 24;
    }

    if pre_release.is_empty() {
        encoded |= 0xC0_0000;
        return encoded;
    }

    if let Some(value) = parse_prerelease(pre_release, "rc", 0x80) {
        encoded |= u64::from(value) << 16;
    } else if let Some(value) = parse_prerelease(pre_release, "b", 0x40) {
        encoded |= u64::from(value) << 16;
    }

    encoded
}

fn parse_prerelease(input: &str, prefix: &str, key: u8) -> Option<u8> {
    let number = input.strip_prefix(prefix)?.parse::<u8>().ok()?;
    (number <= 63).then_some(number + key)
}

pub fn get_encoded_version() -> u64 {
    encode_software_version(get_version_string())
}

pub fn is_xrpld_version(version: u64) -> bool {
    (version & IMPLEMENTATION_VERSION_IDENTIFIER_MASK) == IMPLEMENTATION_VERSION_IDENTIFIER
}

pub fn is_newer_version(version: u64) -> bool {
    is_xrpld_version(version) && version > get_encoded_version()
}

#[cfg(test)]
mod tests {
    use super::{VERSION_STRING, encode_software_version, get_encoded_version, is_xrpld_version};

    #[test]
    fn build_info_encodes_current_version_shape() {
        assert_eq!(VERSION_STRING, "3.2.0-b0");
        assert!(is_xrpld_version(get_encoded_version()));
        assert_eq!(encode_software_version("3.2.0"), 0x183B_0302_00C0_0000);
    }
}
