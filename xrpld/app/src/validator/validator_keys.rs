use protocol::{
    AccountID, KeyType, PublicKey, SecretKey, TokenType, calc_account_id, derive_public_key,
    generate_root_secret_key, parse_base58_seed, parse_base58_with_type,
};

use crate::state::manifest::{deserialize_manifest, load_validator_token};

pub type NodeId = AccountID;

#[derive(Debug, Clone)]
pub struct Keys {
    pub master_public_key: PublicKey,
    pub public_key: PublicKey,
    pub secret_key: SecretKey,
}

#[derive(Debug, Clone)]
pub struct ValidatorKeys {
    pub keys: Option<Keys>,
    pub node_id: NodeId,
    pub manifest: String,
    pub sequence: u32,
    config_invalid: bool,
}

impl Default for ValidatorKeys {
    fn default() -> Self {
        Self {
            keys: None,
            node_id: NodeId::zero(),
            manifest: String::new(),
            sequence: 0,
            config_invalid: false,
        }
    }
}

impl ValidatorKeys {
    pub fn from_sources(validation_seed: Option<&str>, validator_token: Option<&[String]>) -> Self {
        let mut result = Self::default();

        if validation_seed.is_some() && validator_token.is_some() {
            result.config_invalid = true;
            return result;
        }

        if let Some(token_lines) = validator_token {
            let Some(token) = load_validator_token(token_lines.iter().map(String::as_str)) else {
                result.config_invalid = true;
                return result;
            };

            let Ok(public_key) = derive_public_key(KeyType::Secp256k1, &token.validation_secret)
            else {
                result.config_invalid = true;
                return result;
            };

            let manifest_bytes = basics::base64::base64_decode(&token.manifest);
            let Some(manifest) = deserialize_manifest(&manifest_bytes) else {
                result.config_invalid = true;
                return result;
            };

            if manifest.signing_key != Some(public_key) {
                result.config_invalid = true;
                return result;
            }

            result.node_id = calc_node_id(&manifest.master_key);
            result.sequence = manifest.sequence;
            result.manifest = token.manifest;
            result.keys = Some(Keys {
                master_public_key: manifest.master_key,
                public_key,
                secret_key: token.validation_secret,
            });
            return result;
        }

        if let Some(seed) = validation_seed {
            let Some(seed) = parse_base58_seed(seed) else {
                result.config_invalid = true;
                return result;
            };

            let Ok(secret_key) = generate_root_secret_key(KeyType::Secp256k1, &seed) else {
                result.config_invalid = true;
                return result;
            };

            let Ok(public_key) = derive_public_key(KeyType::Secp256k1, &secret_key) else {
                result.config_invalid = true;
                return result;
            };

            result.node_id = calc_node_id(&public_key);
            result.keys = Some(Keys {
                master_public_key: public_key,
                public_key,
                secret_key,
            });
        }

        result
    }

    pub const fn config_invalid(&self) -> bool {
        self.config_invalid
    }
}

pub fn calc_node_id(public_key: &PublicKey) -> NodeId {
    calc_account_id(public_key.as_bytes())
}

pub fn parse_node_private_base58(value: &str) -> Option<SecretKey> {
    parse_base58_with_type(TokenType::NodePrivate, value)
}
