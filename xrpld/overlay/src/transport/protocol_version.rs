//! `ProtocolVersion` support matching the current XRPL overlay version
//! negotiation rules.

use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ProtocolVersion {
    pub major: u16,
    pub minor: u16,
}

impl ProtocolVersion {
    pub const fn new(major: u16, minor: u16) -> Self {
        Self { major, minor }
    }
}

impl fmt::Display for ProtocolVersion {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "XRPL/{}.{}", self.major, self.minor)
    }
}

pub const SUPPORTED_PROTOCOL_LIST: [ProtocolVersion; 2] =
    [ProtocolVersion::new(2, 1), ProtocolVersion::new(2, 2)];

pub fn parse_protocol_versions(value: &str) -> Vec<ProtocolVersion> {
    let mut result = value
        .split(',')
        .filter_map(|token| parse_protocol_version_token(token.trim()))
        .collect::<Vec<_>>();
    result.sort_unstable();
    result.dedup();
    result
}

pub fn negotiate_protocol_version(
    versions: impl IntoIterator<Item = ProtocolVersion>,
) -> Option<ProtocolVersion> {
    versions
        .into_iter()
        .filter(|version| is_protocol_supported(*version))
        .max()
}

pub fn supported_protocol_versions() -> String {
    SUPPORTED_PROTOCOL_LIST
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(", ")
}

pub fn is_protocol_supported(version: ProtocolVersion) -> bool {
    SUPPORTED_PROTOCOL_LIST.contains(&version)
}

fn parse_protocol_version_token(token: &str) -> Option<ProtocolVersion> {
    let version = token.strip_prefix("XRPL/")?;
    let (major, minor) = version.split_once('.')?;
    if major.starts_with('0') && major.len() > 1 {
        return None;
    }
    if minor.starts_with('0') && minor.len() > 1 {
        return None;
    }
    let major = major.parse::<u16>().ok()?;
    let minor = minor.parse::<u16>().ok()?;
    (major >= 2).then_some(ProtocolVersion::new(major, minor))
}

#[cfg(test)]
mod tests {
    use super::{
        ProtocolVersion, is_protocol_supported, negotiate_protocol_version,
        parse_protocol_versions, supported_protocol_versions,
    };

    #[test]
    fn parse_versions_shape() {
        assert_eq!(
            parse_protocol_versions("XRPL/2.2, XRPL/2.1, XRPL/2.2, XRPL/1.9, bad"),
            vec![ProtocolVersion::new(2, 1), ProtocolVersion::new(2, 2)]
        );
        assert_eq!(
            parse_protocol_versions("XRPL/2.01"),
            Vec::<ProtocolVersion>::new()
        );
    }

    #[test]
    fn negotiate_versions_picks_highest_supported() {
        let versions = vec![
            ProtocolVersion::new(3, 0),
            ProtocolVersion::new(2, 1),
            ProtocolVersion::new(2, 2),
        ];
        assert_eq!(
            negotiate_protocol_version(versions),
            Some(ProtocolVersion::new(2, 2))
        );
        assert_eq!(
            negotiate_protocol_version(vec![
                ProtocolVersion::new(2, 2),
                ProtocolVersion::new(2, 1)
            ]),
            Some(ProtocolVersion::new(2, 2))
        );
        assert!(is_protocol_supported(ProtocolVersion::new(2, 1)));
        assert_eq!(supported_protocol_versions(), "XRPL/2.1, XRPL/2.2");
    }
}
