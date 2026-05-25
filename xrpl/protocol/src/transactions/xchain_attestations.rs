//! Cross-chain attestation helpers from `xrpl/protocol/XChainAttestations.*`.

use basics::buffer::Buffer;

use crate::json_get_or_throw::{JsonGetOrThrowError, get_optional, get_or_throw};
use crate::{
    AccountID, JsonValue, PublicKey, STAmount, STArray, STObject, STXChainBridge, SecretKey,
    Serializer, StBase, get_field_by_symbol, is_legal_amount_signed, sign, verify,
};

pub mod attestations {
    use super::*;

    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct AttestationBase {
        pub attestation_signer_account: AccountID,
        pub public_key: PublicKey,
        pub signature: Buffer,
        pub sending_account: AccountID,
        pub sending_amount: STAmount,
        pub reward_account: AccountID,
        pub was_locking_chain_send: bool,
    }

    impl AttestationBase {
        #[allow(clippy::too_many_arguments)]
        pub fn new(
            attestation_signer_account: AccountID,
            public_key: PublicKey,
            signature: Buffer,
            sending_account: AccountID,
            sending_amount: STAmount,
            reward_account: AccountID,
            was_locking_chain_send: bool,
        ) -> Self {
            Self {
                attestation_signer_account,
                public_key,
                signature,
                sending_account,
                sending_amount,
                reward_account,
                was_locking_chain_send,
            }
        }

        pub fn from_st_object(object: &STObject) -> Self {
            Self {
                attestation_signer_account: object
                    .get_account_id(get_field_by_symbol("sfAttestationSignerAccount")),
                public_key: PublicKey::from_slice(
                    &object.get_field_vl(get_field_by_symbol("sfPublicKey")),
                )
                .expect("attestation public key should be valid"),
                signature: Buffer::from_bytes(
                    &object.get_field_vl(get_field_by_symbol("sfSignature")),
                ),
                sending_account: object.get_account_id(get_field_by_symbol("sfAccount")),
                sending_amount: object.get_field_amount(get_field_by_symbol("sfAmount")),
                reward_account: object
                    .get_account_id(get_field_by_symbol("sfAttestationRewardAccount")),
                was_locking_chain_send: object
                    .get_field_u8(get_field_by_symbol("sfWasLockingChainSend"))
                    != 0,
            }
        }

        pub fn from_json_value(value: &JsonValue) -> Result<Self, JsonGetOrThrowError> {
            Ok(Self {
                attestation_signer_account: get_or_throw(
                    value,
                    get_field_by_symbol("sfAttestationSignerAccount"),
                )?,
                public_key: get_or_throw(value, get_field_by_symbol("sfPublicKey"))?,
                signature: get_or_throw(value, get_field_by_symbol("sfSignature"))?,
                sending_account: get_or_throw(value, get_field_by_symbol("sfAccount"))?,
                sending_amount: get_or_throw(value, get_field_by_symbol("sfAmount"))?,
                reward_account: get_or_throw(
                    value,
                    get_field_by_symbol("sfAttestationRewardAccount"),
                )?,
                was_locking_chain_send: get_or_throw(
                    value,
                    get_field_by_symbol("sfWasLockingChainSend"),
                )?,
            })
        }

        pub fn add_helper(&self, object: &mut STObject) {
            object.set_account_id(
                get_field_by_symbol("sfAttestationSignerAccount"),
                self.attestation_signer_account,
            );
            object.set_field_vl(
                get_field_by_symbol("sfPublicKey"),
                self.public_key.as_bytes(),
            );
            object.set_field_vl(get_field_by_symbol("sfSignature"), self.signature.data());
            object.set_account_id(get_field_by_symbol("sfAccount"), self.sending_account);
            object.set_field_amount(get_field_by_symbol("sfAmount"), self.sending_amount.clone());
            object.set_account_id(
                get_field_by_symbol("sfAttestationRewardAccount"),
                self.reward_account,
            );
            object.set_field_u8(
                get_field_by_symbol("sfWasLockingChainSend"),
                u8::from(self.was_locking_chain_send),
            );
        }
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct AttestationClaim {
        pub base: AttestationBase,
        pub claim_id: u64,
        pub dst: Option<AccountID>,
    }

    impl AttestationClaim {
        #[allow(clippy::too_many_arguments)]
        pub fn new(
            attestation_signer_account: AccountID,
            public_key: PublicKey,
            signature: Buffer,
            sending_account: AccountID,
            sending_amount: STAmount,
            reward_account: AccountID,
            was_locking_chain_send: bool,
            claim_id: u64,
            dst: Option<AccountID>,
        ) -> Self {
            Self {
                base: AttestationBase::new(
                    attestation_signer_account,
                    public_key,
                    signature,
                    sending_account,
                    sending_amount,
                    reward_account,
                    was_locking_chain_send,
                ),
                claim_id,
                dst,
            }
        }

        #[allow(clippy::too_many_arguments)]
        pub fn signed(
            bridge: &STXChainBridge,
            attestation_signer_account: AccountID,
            public_key: PublicKey,
            secret_key: &SecretKey,
            sending_account: AccountID,
            sending_amount: STAmount,
            reward_account: AccountID,
            was_locking_chain_send: bool,
            claim_id: u64,
            dst: Option<AccountID>,
        ) -> Result<Self, crate::SignError> {
            let mut claim = Self::new(
                attestation_signer_account,
                public_key,
                Buffer::new(),
                sending_account,
                sending_amount,
                reward_account,
                was_locking_chain_send,
                claim_id,
                dst,
            );
            let message = claim.message(bridge);
            claim.base.signature =
                Buffer::from_bytes(&sign(&claim.base.public_key, secret_key, &message)?);
            Ok(claim)
        }

        pub fn from_st_object(object: &STObject) -> Self {
            Self {
                base: AttestationBase::from_st_object(object),
                claim_id: object.get_field_u64(get_field_by_symbol("sfXChainClaimID")),
                dst: object
                    .is_field_present(get_field_by_symbol("sfDestination"))
                    .then(|| object.get_account_id(get_field_by_symbol("sfDestination"))),
            }
        }

        pub fn from_json_value(value: &JsonValue) -> Result<Self, JsonGetOrThrowError> {
            Ok(Self {
                base: AttestationBase::from_json_value(value)?,
                claim_id: get_or_throw(value, get_field_by_symbol("sfXChainClaimID"))?,
                dst: get_optional(value, get_field_by_symbol("sfDestination")),
            })
        }

        pub fn to_st_object(&self) -> STObject {
            let mut object = STObject::make_inner_object(get_field_by_symbol(
                "sfXChainClaimAttestationCollectionElement",
            ));
            self.base.add_helper(&mut object);
            object.set_field_u64(get_field_by_symbol("sfXChainClaimID"), self.claim_id);
            if let Some(dst) = self.dst {
                object.set_account_id(get_field_by_symbol("sfDestination"), dst);
            }
            object
        }

        pub fn message(&self, bridge: &STXChainBridge) -> Vec<u8> {
            Self::message_for(
                bridge,
                self.base.sending_account,
                self.base.sending_amount.clone(),
                self.base.reward_account,
                self.base.was_locking_chain_send,
                self.claim_id,
                self.dst,
            )
        }

        #[allow(clippy::too_many_arguments)]
        pub fn message_for(
            bridge: &STXChainBridge,
            sending_account: AccountID,
            sending_amount: STAmount,
            reward_account: AccountID,
            was_locking_chain_send: bool,
            claim_id: u64,
            dst: Option<AccountID>,
        ) -> Vec<u8> {
            let mut object = STObject::new(get_field_by_symbol("sfGeneric"));
            object.set_field_u64(get_field_by_symbol("sfXChainClaimID"), claim_id);
            object.set_field_amount(get_field_by_symbol("sfAmount"), sending_amount);
            if let Some(dst) = dst {
                object.set_account_id(get_field_by_symbol("sfDestination"), dst);
            }
            object.set_account_id(get_field_by_symbol("sfOtherChainSource"), sending_account);
            object.set_account_id(
                get_field_by_symbol("sfAttestationRewardAccount"),
                reward_account,
            );
            object.set_field_u8(
                get_field_by_symbol("sfWasLockingChainSend"),
                u8::from(was_locking_chain_send),
            );
            object.set_field_xchain_bridge(get_field_by_symbol("sfXChainBridge"), bridge.clone());

            let mut serializer = Serializer::new(0);
            object.add(&mut serializer);
            serializer.get_data()
        }

        pub fn verify(&self, bridge: &STXChainBridge) -> bool {
            verify(
                &self.base.public_key,
                &self.message(bridge),
                self.base.signature.data(),
            )
        }

        pub fn valid_amounts(&self) -> bool {
            !self.base.sending_amount.native()
                || is_legal_amount_signed(self.base.sending_amount.xrp())
        }
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct AttestationCreateAccount {
        pub base: AttestationBase,
        pub create_count: u64,
        pub to_create: AccountID,
        pub reward_amount: STAmount,
    }

    impl AttestationCreateAccount {
        #[allow(clippy::too_many_arguments)]
        pub fn new(
            attestation_signer_account: AccountID,
            public_key: PublicKey,
            signature: Buffer,
            sending_account: AccountID,
            sending_amount: STAmount,
            reward_amount: STAmount,
            reward_account: AccountID,
            was_locking_chain_send: bool,
            create_count: u64,
            to_create: AccountID,
        ) -> Self {
            Self {
                base: AttestationBase::new(
                    attestation_signer_account,
                    public_key,
                    signature,
                    sending_account,
                    sending_amount,
                    reward_account,
                    was_locking_chain_send,
                ),
                create_count,
                to_create,
                reward_amount,
            }
        }

        #[allow(clippy::too_many_arguments)]
        pub fn signed(
            bridge: &STXChainBridge,
            attestation_signer_account: AccountID,
            public_key: PublicKey,
            secret_key: &SecretKey,
            sending_account: AccountID,
            sending_amount: STAmount,
            reward_amount: STAmount,
            reward_account: AccountID,
            was_locking_chain_send: bool,
            create_count: u64,
            to_create: AccountID,
        ) -> Result<Self, crate::SignError> {
            let mut attestation = Self::new(
                attestation_signer_account,
                public_key,
                Buffer::new(),
                sending_account,
                sending_amount,
                reward_amount,
                reward_account,
                was_locking_chain_send,
                create_count,
                to_create,
            );
            let message = attestation.message(bridge);
            attestation.base.signature =
                Buffer::from_bytes(&sign(&attestation.base.public_key, secret_key, &message)?);
            Ok(attestation)
        }

        pub fn from_st_object(object: &STObject) -> Self {
            Self {
                base: AttestationBase::from_st_object(object),
                create_count: object
                    .get_field_u64(get_field_by_symbol("sfXChainAccountCreateCount")),
                to_create: object.get_account_id(get_field_by_symbol("sfDestination")),
                reward_amount: object.get_field_amount(get_field_by_symbol("sfSignatureReward")),
            }
        }

        pub fn from_json_value(value: &JsonValue) -> Result<Self, JsonGetOrThrowError> {
            Ok(Self {
                base: AttestationBase::from_json_value(value)?,
                create_count: get_or_throw(
                    value,
                    get_field_by_symbol("sfXChainAccountCreateCount"),
                )?,
                to_create: get_or_throw(value, get_field_by_symbol("sfDestination"))?,
                reward_amount: get_or_throw(value, get_field_by_symbol("sfSignatureReward"))?,
            })
        }

        pub fn to_st_object(&self) -> STObject {
            let mut object = STObject::make_inner_object(get_field_by_symbol(
                "sfXChainCreateAccountAttestationCollectionElement",
            ));
            self.base.add_helper(&mut object);
            object.set_field_u64(
                get_field_by_symbol("sfXChainAccountCreateCount"),
                self.create_count,
            );
            object.set_account_id(get_field_by_symbol("sfDestination"), self.to_create);
            object.set_field_amount(
                get_field_by_symbol("sfSignatureReward"),
                self.reward_amount.clone(),
            );
            object
        }

        pub fn message(&self, bridge: &STXChainBridge) -> Vec<u8> {
            Self::message_for(
                bridge,
                self.base.sending_account,
                self.base.sending_amount.clone(),
                self.reward_amount.clone(),
                self.base.reward_account,
                self.base.was_locking_chain_send,
                self.create_count,
                self.to_create,
            )
        }

        #[allow(clippy::too_many_arguments)]
        pub fn message_for(
            bridge: &STXChainBridge,
            sending_account: AccountID,
            sending_amount: STAmount,
            reward_amount: STAmount,
            reward_account: AccountID,
            was_locking_chain_send: bool,
            create_count: u64,
            dst: AccountID,
        ) -> Vec<u8> {
            let mut object = STObject::new(get_field_by_symbol("sfGeneric"));
            object.set_field_u64(
                get_field_by_symbol("sfXChainAccountCreateCount"),
                create_count,
            );
            object.set_field_amount(get_field_by_symbol("sfAmount"), sending_amount);
            object.set_field_amount(get_field_by_symbol("sfSignatureReward"), reward_amount);
            object.set_account_id(get_field_by_symbol("sfDestination"), dst);
            object.set_account_id(get_field_by_symbol("sfOtherChainSource"), sending_account);
            object.set_account_id(
                get_field_by_symbol("sfAttestationRewardAccount"),
                reward_account,
            );
            object.set_field_u8(
                get_field_by_symbol("sfWasLockingChainSend"),
                u8::from(was_locking_chain_send),
            );
            object.set_field_xchain_bridge(get_field_by_symbol("sfXChainBridge"), bridge.clone());

            let mut serializer = Serializer::new(0);
            object.add(&mut serializer);
            serializer.get_data()
        }

        pub fn verify(&self, bridge: &STXChainBridge) -> bool {
            verify(
                &self.base.public_key,
                &self.message(bridge),
                self.base.signature.data(),
            )
        }

        pub fn valid_amounts(&self) -> bool {
            (!self.base.sending_amount.native()
                || is_legal_amount_signed(self.base.sending_amount.xrp()))
                && (!self.reward_amount.native()
                    || is_legal_amount_signed(self.reward_amount.xrp()))
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AttestationMatch {
    NonDstMismatch,
    MatchExceptDst,
    Match,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct XChainClaimAttestation {
    pub key_account: AccountID,
    pub public_key: PublicKey,
    pub amount: STAmount,
    pub reward_account: AccountID,
    pub was_locking_chain_send: bool,
    pub dst: Option<AccountID>,
}

impl XChainClaimAttestation {
    pub fn from_signed(attestation: &attestations::AttestationClaim) -> Self {
        Self {
            key_account: attestation.base.attestation_signer_account,
            public_key: attestation.base.public_key,
            amount: attestation.base.sending_amount.clone(),
            reward_account: attestation.base.reward_account,
            was_locking_chain_send: attestation.base.was_locking_chain_send,
            dst: attestation.dst,
        }
    }

    pub fn from_st_object(object: &STObject) -> Self {
        Self {
            key_account: object.get_account_id(get_field_by_symbol("sfAttestationSignerAccount")),
            public_key: PublicKey::from_slice(
                &object.get_field_vl(get_field_by_symbol("sfPublicKey")),
            )
            .expect("claim proof public key should be valid"),
            amount: object.get_field_amount(get_field_by_symbol("sfAmount")),
            reward_account: object
                .get_account_id(get_field_by_symbol("sfAttestationRewardAccount")),
            was_locking_chain_send: object
                .get_field_u8(get_field_by_symbol("sfWasLockingChainSend"))
                != 0,
            dst: object
                .is_field_present(get_field_by_symbol("sfDestination"))
                .then(|| object.get_account_id(get_field_by_symbol("sfDestination"))),
        }
    }

    pub fn from_json_value(value: &JsonValue) -> Result<Self, JsonGetOrThrowError> {
        Ok(Self {
            key_account: get_or_throw(value, get_field_by_symbol("sfAttestationSignerAccount"))?,
            public_key: get_or_throw(value, get_field_by_symbol("sfPublicKey"))?,
            amount: get_or_throw(value, get_field_by_symbol("sfAmount"))?,
            reward_account: get_or_throw(value, get_field_by_symbol("sfAttestationRewardAccount"))?,
            was_locking_chain_send: get_or_throw(
                value,
                get_field_by_symbol("sfWasLockingChainSend"),
            )?,
            dst: get_optional(value, get_field_by_symbol("sfDestination")),
        })
    }

    pub fn to_st_object(&self) -> STObject {
        let mut object = STObject::make_inner_object(get_field_by_symbol("sfXChainClaimProofSig"));
        object.set_account_id(
            get_field_by_symbol("sfAttestationSignerAccount"),
            self.key_account,
        );
        object.set_field_vl(
            get_field_by_symbol("sfPublicKey"),
            self.public_key.as_bytes(),
        );
        object.set_field_amount(get_field_by_symbol("sfAmount"), self.amount.clone());
        object.set_account_id(
            get_field_by_symbol("sfAttestationRewardAccount"),
            self.reward_account,
        );
        object.set_field_u8(
            get_field_by_symbol("sfWasLockingChainSend"),
            u8::from(self.was_locking_chain_send),
        );
        if let Some(dst) = self.dst {
            object.set_account_id(get_field_by_symbol("sfDestination"), dst);
        }
        object
    }

    pub fn match_fields(
        &self,
        amount: &STAmount,
        was_locking_chain_send: bool,
        dst: Option<AccountID>,
    ) -> AttestationMatch {
        if self.amount != *amount || self.was_locking_chain_send != was_locking_chain_send {
            return AttestationMatch::NonDstMismatch;
        }
        if self.dst != dst {
            return AttestationMatch::MatchExceptDst;
        }
        AttestationMatch::Match
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct XChainCreateAccountAttestation {
    pub key_account: AccountID,
    pub public_key: PublicKey,
    pub amount: STAmount,
    pub reward_amount: STAmount,
    pub reward_account: AccountID,
    pub was_locking_chain_send: bool,
    pub dst: AccountID,
}

impl XChainCreateAccountAttestation {
    pub fn from_signed(attestation: &attestations::AttestationCreateAccount) -> Self {
        Self {
            key_account: attestation.base.attestation_signer_account,
            public_key: attestation.base.public_key,
            amount: attestation.base.sending_amount.clone(),
            reward_amount: attestation.reward_amount.clone(),
            reward_account: attestation.base.reward_account,
            was_locking_chain_send: attestation.base.was_locking_chain_send,
            dst: attestation.to_create,
        }
    }

    pub fn from_st_object(object: &STObject) -> Self {
        Self {
            key_account: object.get_account_id(get_field_by_symbol("sfAttestationSignerAccount")),
            public_key: PublicKey::from_slice(
                &object.get_field_vl(get_field_by_symbol("sfPublicKey")),
            )
            .expect("create-account proof public key should be valid"),
            amount: object.get_field_amount(get_field_by_symbol("sfAmount")),
            reward_amount: object.get_field_amount(get_field_by_symbol("sfSignatureReward")),
            reward_account: object
                .get_account_id(get_field_by_symbol("sfAttestationRewardAccount")),
            was_locking_chain_send: object
                .get_field_u8(get_field_by_symbol("sfWasLockingChainSend"))
                != 0,
            dst: object.get_account_id(get_field_by_symbol("sfDestination")),
        }
    }

    pub fn from_json_value(value: &JsonValue) -> Result<Self, JsonGetOrThrowError> {
        Ok(Self {
            key_account: get_or_throw(value, get_field_by_symbol("sfAttestationSignerAccount"))?,
            public_key: get_or_throw(value, get_field_by_symbol("sfPublicKey"))?,
            amount: get_or_throw(value, get_field_by_symbol("sfAmount"))?,
            reward_amount: get_or_throw(value, get_field_by_symbol("sfSignatureReward"))?,
            reward_account: get_or_throw(value, get_field_by_symbol("sfAttestationRewardAccount"))?,
            was_locking_chain_send: get_or_throw(
                value,
                get_field_by_symbol("sfWasLockingChainSend"),
            )?,
            dst: get_or_throw(value, get_field_by_symbol("sfDestination"))?,
        })
    }

    pub fn to_st_object(&self) -> STObject {
        let mut object =
            STObject::make_inner_object(get_field_by_symbol("sfXChainCreateAccountProofSig"));
        object.set_account_id(
            get_field_by_symbol("sfAttestationSignerAccount"),
            self.key_account,
        );
        object.set_field_vl(
            get_field_by_symbol("sfPublicKey"),
            self.public_key.as_bytes(),
        );
        object.set_field_amount(get_field_by_symbol("sfAmount"), self.amount.clone());
        object.set_field_amount(
            get_field_by_symbol("sfSignatureReward"),
            self.reward_amount.clone(),
        );
        object.set_account_id(
            get_field_by_symbol("sfAttestationRewardAccount"),
            self.reward_account,
        );
        object.set_field_u8(
            get_field_by_symbol("sfWasLockingChainSend"),
            u8::from(self.was_locking_chain_send),
        );
        object.set_account_id(get_field_by_symbol("sfDestination"), self.dst);
        object
    }

    pub fn match_fields(
        &self,
        amount: &STAmount,
        reward_amount: &STAmount,
        was_locking_chain_send: bool,
        dst: AccountID,
    ) -> AttestationMatch {
        if self.amount != *amount
            || self.reward_amount != *reward_amount
            || self.was_locking_chain_send != was_locking_chain_send
        {
            return AttestationMatch::NonDstMismatch;
        }
        if self.dst != dst {
            return AttestationMatch::MatchExceptDst;
        }
        AttestationMatch::Match
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct XChainAttestationsBase<T> {
    attestations: Vec<T>,
}

impl<T> XChainAttestationsBase<T> {
    const MAX_ATTESTATIONS: usize = 256;

    pub fn new(attestations: Vec<T>) -> Self {
        Self { attestations }
    }

    pub fn attestations(&self) -> &[T] {
        &self.attestations
    }

    pub fn attestations_mut(&mut self) -> &mut [T] {
        &mut self.attestations
    }

    pub fn emplace_back(&mut self, attestation: T) {
        self.attestations.push(attestation);
    }

    pub fn erase_if(&mut self, mut predicate: impl FnMut(&T) -> bool) -> usize {
        let before = self.attestations.len();
        self.attestations
            .retain(|attestation| !predicate(attestation));
        before - self.attestations.len()
    }

    pub fn size(&self) -> usize {
        self.attestations.len()
    }

    pub fn empty(&self) -> bool {
        self.attestations.is_empty()
    }
}

impl<T> XChainAttestationsBase<T>
where
    T: Clone,
{
    pub fn from_json_value(
        value: &JsonValue,
        parse: impl Fn(&JsonValue) -> Result<T, JsonGetOrThrowError>,
    ) -> Result<Self, String> {
        let JsonValue::Object(object) = value else {
            return Err(
                "XChainAttestationsBase can only be specified with an object Json value".to_owned(),
            );
        };
        let Some(JsonValue::Array(entries)) = object.get(crate::jss::attestations) else {
            return Err("Missing json key: attestations".to_owned());
        };
        if entries.len() > Self::MAX_ATTESTATIONS {
            return Err("XChainAttestationsBase exceeded max number of attestations".to_owned());
        }
        let mut attestations = Vec::with_capacity(entries.len());
        for entry in entries {
            attestations.push(parse(entry).map_err(|error| error.to_string())?);
        }
        Ok(Self::new(attestations))
    }

    pub fn from_st_array(array: &STArray, parse: impl Fn(&STObject) -> T) -> Result<Self, String> {
        if array.len() > Self::MAX_ATTESTATIONS {
            return Err("XChainAttestationsBase exceeded max number of attestations".to_owned());
        }
        Ok(Self::new(array.iter().map(parse).collect()))
    }
}

impl XChainAttestationsBase<XChainClaimAttestation> {
    pub fn to_st_array(&self) -> STArray {
        let mut array = STArray::new(get_field_by_symbol("sfXChainClaimAttestations"));
        for attestation in &self.attestations {
            array.push_back(attestation.to_st_object());
        }
        array
    }
}

impl XChainAttestationsBase<XChainCreateAccountAttestation> {
    pub fn to_st_array(&self) -> STArray {
        let mut array = STArray::new(get_field_by_symbol("sfXChainCreateAccountAttestations"));
        for attestation in &self.attestations {
            array.push_back(attestation.to_st_object());
        }
        array
    }
}

pub type XChainClaimAttestations = XChainAttestationsBase<XChainClaimAttestation>;
pub type XChainCreateAccountAttestations = XChainAttestationsBase<XChainCreateAccountAttestation>;

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::{XChainAttestationsBase, XChainClaimAttestation, attestations};
    use crate::{
        JsonOptions, JsonValue, KeyType, STAmount, STXChainBridge, SecretKey, StBase, XRPAmount,
        derive_public_key, parse_base58_account_id, xrp_issue,
    };

    fn account(value: &str) -> crate::AccountID {
        parse_base58_account_id(value).expect("valid base58 account")
    }

    #[test]
    fn signed_claim_attestation_round_trips_and_verifies() {
        let secret = SecretKey::from_bytes([7u8; crate::SECRET_KEY_LENGTH]);
        let public = derive_public_key(KeyType::Ed25519, &secret).expect("valid deterministic key");
        let bridge = STXChainBridge::from_parts(
            account("rHb9CJAWyB4rj91VRWn96DkukG4bwdtyTh"),
            xrp_issue(),
            account("rrrrrrrrrrrrrrrrrrrrBZbvji"),
            xrp_issue(),
        );

        let signed = attestations::AttestationClaim::signed(
            &bridge,
            account("rHb9CJAWyB4rj91VRWn96DkukG4bwdtyTh"),
            public,
            &secret,
            account("rrrrrrrrrrrrrrrrrrrrBZbvji"),
            STAmount::from_xrp_amount(XRPAmount::from_drops(10)),
            account("rHb9CJAWyB4rj91VRWn96DkukG4bwdtyTh"),
            true,
            7,
            None,
        )
        .expect("signature should succeed");

        assert!(signed.verify(&bridge));
        assert!(signed.valid_amounts());

        let proof = XChainClaimAttestation::from_signed(&signed);
        let json = JsonValue::Object(BTreeMap::from([(
            "attestations".to_owned(),
            JsonValue::Array(vec![proof.to_st_object().json(JsonOptions::NONE)]),
        )]));

        let parsed =
            XChainAttestationsBase::from_json_value(&json, XChainClaimAttestation::from_json_value)
                .expect("collection should parse");

        assert_eq!(parsed.size(), 1);
        assert_eq!(parsed.to_st_array().len(), 1);
    }
}
