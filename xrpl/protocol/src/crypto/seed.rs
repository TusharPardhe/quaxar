//! Seed helpers from `xrpl/protocol/Seed.*`.

use basics::{base_uint::BaseUInt, string_utilities::str_unhex};
use rfc1751::{FromRfc1751, ToRfc1751};

use crate::{
    PublicKey, SecretKey, Sha512HalfHasherS, TokenType, decode_base58_token, encode_base58_token,
    parse_base58_account_id, parse_base58_node_public, parse_base58_with_type,
};

#[derive(Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Seed([u8; 16]);

impl Seed {
    pub fn from_slice(slice: &[u8]) -> Result<Self, &'static str> {
        if slice.len() != 16 {
            return Err("Seed::from_slice: invalid size");
        }
        let mut bytes = [0u8; 16];
        bytes.copy_from_slice(slice);
        Ok(Self(bytes))
    }

    pub fn from_uint128(seed: BaseUInt<16>) -> Self {
        Self(*seed.data())
    }

    pub const fn data(&self) -> &[u8; 16] {
        &self.0
    }

    pub const fn as_slice(&self) -> &[u8] {
        &self.0
    }

    pub const fn size(&self) -> usize {
        self.0.len()
    }

    pub fn iter(&self) -> std::slice::Iter<'_, u8> {
        self.0.iter()
    }
}

impl std::fmt::Debug for Seed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Seed(..)")
    }
}

impl Drop for Seed {
    fn drop(&mut self) {
        self.0.fill(0);
    }
}

impl crate::tokens::Base58Token for Seed {
    const TOKEN_TYPE: crate::TokenType = crate::TokenType::FamilySeed;

    fn from_token_bytes(bytes: &[u8]) -> Option<Self> {
        Self::from_slice(bytes).ok()
    }

    fn to_token_bytes(&self) -> Vec<u8> {
        self.0.to_vec()
    }
}

impl AsRef<[u8]> for Seed {
    fn as_ref(&self) -> &[u8] {
        self.as_slice()
    }
}

impl<'a> IntoIterator for &'a Seed {
    type Item = &'a u8;
    type IntoIter = std::slice::Iter<'a, u8>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

pub fn random_seed() -> Seed {
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes).expect("OS randomness must be available");
    Seed(bytes)
}

pub fn generate_seed(passphrase: &str) -> Seed {
    let mut hasher = Sha512HalfHasherS::new();
    hasher.write(passphrase.as_bytes());
    let digest = hasher.result();
    Seed::from_slice(&digest.data()[..16]).expect("sha512 half should yield at least 16 bytes")
}

pub fn parse_base58_seed(value: &str) -> Option<Seed> {
    if let Some(bytes) = decode_base58_token(value, TokenType::FamilySeed) {
        return Seed::from_slice(&bytes).ok();
    }
    if let Some(bytes) = crate::tokens::decode_base58_token_multibyte(value, &[1, 225, 75]) {
        return Seed::from_slice(&bytes).ok();
    }
    None
}

pub fn parse_generic_seed(value: &str, rfc1751_enabled: bool) -> Option<Seed> {
    if value.is_empty() {
        return None;
    }

    if parse_base58_account_id(value).is_some()
        || parse_base58_node_public(value).is_some()
        || parse_base58_with_type::<PublicKey>(TokenType::AccountPublic, value).is_some()
        || parse_base58_with_type::<SecretKey>(TokenType::NodePrivate, value).is_some()
        || parse_base58_with_type::<SecretKey>(TokenType::AccountSecret, value).is_some()
    {
        return None;
    }

    if let Some(bytes) = str_unhex(value)
        && bytes.len() == 16
    {
        return Seed::from_slice(&bytes).ok();
    }

    if let Some(seed) = parse_base58_seed(value) {
        return Some(seed);
    }

    if rfc1751_enabled
        && let Ok(english) = value.from_rfc1751()
        && english.len() == 16
    {
        let mut reversed = english;
        reversed.reverse();
        if let Ok(seed) = Seed::from_slice(&reversed) {
            return Some(seed);
        }
    }

    Some(generate_seed(value))
}

pub fn to_base58(seed: &Seed) -> String {
    encode_base58_token(TokenType::FamilySeed, seed.data())
}

pub fn seed_as_1751(seed: &Seed) -> String {
    let mut reversed = seed.data().to_vec();
    reversed.reverse();
    reversed
        .as_slice()
        .to_rfc1751()
        .expect("seed should always encode to RFC1751")
}

#[cfg(test)]
mod tests {
    use super::{
        Seed, generate_seed, parse_base58_seed, parse_generic_seed, seed_as_1751, to_base58,
    };

    #[test]
    fn seed_base58_round_trips() {
        let seed = generate_seed("masterpassphrase");
        let encoded = to_base58(&seed);
        assert_eq!(parse_base58_seed(&encoded), Some(seed.clone()));
        assert!(parse_generic_seed(&encoded, true).is_some());
        assert!(Seed::from_slice(&[0; 15]).is_err());
    }

    #[test]
    fn seed_rfc1751_round_trips() {
        let seed = generate_seed("masterpassphrase");
        let english = seed_as_1751(&seed);
        assert_eq!(parse_generic_seed(&english, true), Some(seed));
    }

    #[test]
    fn seed_passphrase_vectors() {
        assert_eq!(
            to_base58(&generate_seed("masterpassphrase")),
            "snoPBrXtMeMyMHUVTgbuqAfg1SUTb"
        );
        assert_eq!(
            to_base58(&generate_seed("Non-Random Passphrase")),
            "snMKnVku798EnBwUfxeSD8953sLYA"
        );
        assert_eq!(
            to_base58(&generate_seed("cookies excitement hand public")),
            "sspUXGrmjQhq6mgc24jiRuevZiwKT"
        );
    }

    #[test]
    fn parse_generic_seed_rejects_known_non_seed_base58_tokens() {
        for value in [
            "n94a1u4jAz288pZLtw6yFWVbi89YamiC6JBXPVUj5zmExe5fTVg9",
            "pnen77YEeUd4fFKG7iycBWcwKpTaeFRkW2WFostaATy1DSupwXe",
            "rHb9CJAWyB4rj91VRWn96DkukG4bwdtyTh",
            "aBQG8RQAzjs1eTKFEAQXr2gS4utcDiEC9wmi7pfUPTi27VCahwgw",
            "p9JfM6HHi64m6mvB6v5k7G2b1cXzGmYiCNJf6GHPKvFTWdeRVjh",
            "nHUeeJCSY2dM71oxM8Cgjouf5ekTuev2mwDpc374aLMxzDLXNmjf",
            "paKv46LztLqK3GaKz1rG2nQGN6M4JLyRtxFBYFTw4wAVHtGys36",
            "rGWrZyQqhTp9Xu7G5Pkayo7bXjH4k4QYpf",
            "aKGheSBjmCsKJVuLNKRAKpZXT6wpk2FCuEZAXJupXgdAxX5THCqR",
            "pwDQjwEhbUBmPuEjFpEG75bFhv2obkCB7NxQsfFxM7xGHBMVPu9",
        ] {
            assert_eq!(parse_generic_seed(value, true), None, "{value}");
        }
    }

    #[test]
    fn parse_generic_seed_invalid_rfc1751_falls_back_to_generate_seed() {
        let text = "THIS IS NOT VALID RFC1751";
        assert_eq!(parse_generic_seed(text, true), Some(generate_seed(text)));
        assert_eq!(parse_generic_seed(text, false), Some(generate_seed(text)));
    }
}

#[cfg(test)]
mod tests_parsing {
    use super::*;
    #[test]
    fn test_sed_parsing() {
        let s = "sEd7ZFrv3FDZM6W8zcBL7qDuBTVwGHP";
        let seed = parse_base58_seed(s);
        assert!(seed.is_some(), "Should parse sEd seed");
    }
}
