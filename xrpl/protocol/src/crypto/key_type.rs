//! `KeyType` port from `xrpl/protocol/KeyType.h`.

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum KeyType {
    Secp256k1,
    Ed25519,
}

impl std::str::FromStr for KeyType {
    type Err = ();

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "secp256k1" => Ok(Self::Secp256k1),
            "ed25519" => Ok(Self::Ed25519),
            _ => Err(()),
        }
    }
}

impl KeyType {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Secp256k1 => "secp256k1",
            Self::Ed25519 => "ed25519",
        }
    }
}

impl std::fmt::Display for KeyType {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}
