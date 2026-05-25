//! `STValidation` owner port from `xrpl/protocol/STValidation.*`.

use std::{
    ops::{Deref, DerefMut},
    sync::OnceLock,
};

use basics::base_uint::Uint256;

use crate::{
    HashPrefix, JsonOptions, JsonValue, KeyType, NodeId, PublicKey, SOEStyle, SOElement,
    SOTemplate, STObject, SecretKey, SerialIter, Serializer, SignError, StBase, StBaseCore,
    calc_node_id, get_field_by_symbol, sign_digest, verify_digest,
};

pub const VF_FULL_VALIDATION: u32 = 0x0000_0001;
pub const VF_FULLY_CANONICAL_SIG: u32 = 0x8000_0000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StValidationError {
    InvalidPublicKey,
    InvalidSignature,
    ZeroNodeId,
    KeyTypeMismatch,
    SignFailed(SignError),
    MissingRequiredField(&'static str),
}

impl From<SignError> for StValidationError {
    fn from(value: SignError) -> Self {
        Self::SignFailed(value)
    }
}

#[derive(Debug)]
pub struct STValidation {
    object: STObject,
    trusted: bool,
    valid: OnceLock<bool>,
    signing_pub_key: PublicKey,
    node_id: NodeId,
    seen_time: u32,
}

impl Clone for STValidation {
    fn clone(&self) -> Self {
        let valid = OnceLock::new();
        if let Some(cached) = self.valid.get().copied() {
            let _ = valid.set(cached);
        }

        Self {
            object: self.object.clone(),
            trusted: self.trusted,
            valid,
            signing_pub_key: self.signing_pub_key,
            node_id: self.node_id,
            seen_time: self.seen_time,
        }
    }
}

impl PartialEq for STValidation {
    fn eq(&self, other: &Self) -> bool {
        self.object == other.object
            && self.trusted == other.trusted
            && self.signing_pub_key == other.signing_pub_key
            && self.node_id == other.node_id
            && self.seen_time == other.seen_time
    }
}

impl Eq for STValidation {}

impl STValidation {
    pub fn from_serial_iter<LookupNodeId>(
        sit: &mut SerialIter<'_>,
        lookup_node_id: LookupNodeId,
        check_signature: bool,
    ) -> Result<Self, StValidationError>
    where
        LookupNodeId: FnOnce(&PublicKey) -> NodeId,
    {
        let mut object = STObject::from_serial_iter(sit, get_field_by_symbol("sfValidation"), 0);
        object.apply_template(validation_format());

        let signing_pub_key = parse_validation_signing_pub_key(&object)?;
        let node_id = lookup_node_id(&signing_pub_key);
        if node_id.is_zero() {
            return Err(StValidationError::ZeroNodeId);
        }

        let validation = Self {
            object,
            trusted: false,
            valid: OnceLock::new(),
            signing_pub_key,
            node_id,
            seen_time: 0,
        };

        if check_signature && !validation.is_valid() {
            return Err(StValidationError::InvalidSignature);
        }

        Ok(validation)
    }

    pub fn from_serial_iter_default_node_id(
        sit: &mut SerialIter<'_>,
        check_signature: bool,
    ) -> Result<Self, StValidationError> {
        Self::from_serial_iter(sit, calc_node_id, check_signature)
    }

    pub fn new_signed<F>(
        sign_time: u32,
        public_key: &PublicKey,
        node_id: NodeId,
        secret_key: &SecretKey,
        fill: F,
    ) -> Result<Self, StValidationError>
    where
        F: FnOnce(&mut Self),
    {
        if public_key.key_type() != Some(KeyType::Secp256k1) {
            return Err(StValidationError::KeyTypeMismatch);
        }
        if node_id.is_zero() {
            return Err(StValidationError::ZeroNodeId);
        }

        let mut validation = Self {
            object: STObject::with_template(
                validation_format(),
                get_field_by_symbol("sfValidation"),
            ),
            trusted: false,
            valid: OnceLock::new(),
            signing_pub_key: *public_key,
            node_id,
            seen_time: sign_time,
        };

        validation.set_field_vl(
            get_field_by_symbol("sfSigningPubKey"),
            public_key.as_bytes(),
        );
        validation.set_field_u32(get_field_by_symbol("sfSigningTime"), sign_time);

        fill(&mut validation);

        validation.set_flag(VF_FULLY_CANONICAL_SIG);
        let signature = sign_digest(public_key, secret_key, validation.get_signing_hash())?;
        validation.set_field_vl(get_field_by_symbol("sfSignature"), &signature);
        validation.set_trusted();

        for element in validation_format().iter() {
            if element.style() == SOEStyle::Required
                && !validation.is_field_present(element.sfield())
            {
                return Err(StValidationError::MissingRequiredField(
                    element.sfield().name(),
                ));
            }
        }

        let _ = validation.valid.set(true);
        Ok(validation)
    }

    pub fn get_ledger_hash(&self) -> Uint256 {
        self.get_field_h256(get_field_by_symbol("sfLedgerHash"))
    }

    pub fn get_consensus_hash(&self) -> Uint256 {
        self.get_field_h256(get_field_by_symbol("sfConsensusHash"))
    }

    pub fn get_sign_time(&self) -> u32 {
        self.get_field_u32(get_field_by_symbol("sfSigningTime"))
    }

    pub fn get_seen_time(&self) -> u32 {
        self.seen_time
    }

    pub fn get_signer_public(&self) -> &PublicKey {
        &self.signing_pub_key
    }

    pub fn get_node_id(&self) -> NodeId {
        self.node_id
    }

    pub fn is_valid(&self) -> bool {
        *self.valid.get_or_init(|| {
            verify_digest(
                &self.signing_pub_key,
                self.get_signing_hash(),
                &self.get_signature(),
                (self.get_flags() & VF_FULLY_CANONICAL_SIG) != 0,
            )
        })
    }

    pub fn is_full(&self) -> bool {
        (self.get_flags() & VF_FULL_VALIDATION) != 0
    }

    pub fn is_trusted(&self) -> bool {
        self.trusted
    }

    pub fn get_signing_hash(&self) -> Uint256 {
        self.object.get_signing_hash(HashPrefix::Validation)
    }

    pub fn set_trusted(&mut self) {
        self.trusted = true;
    }

    pub fn set_untrusted(&mut self) {
        self.trusted = false;
    }

    pub fn set_seen(&mut self, seen_time: u32) {
        self.seen_time = seen_time;
    }

    pub fn get_serialized(&self) -> Vec<u8> {
        let mut serializer = Serializer::default();
        self.add(&mut serializer);
        serializer.data().to_vec()
    }

    pub fn get_signature(&self) -> Vec<u8> {
        self.get_field_vl(get_field_by_symbol("sfSignature"))
    }

    pub fn render(&self) -> String {
        format!(
            "validation:  ledger_hash: {} consensus_hash: {} sign_time: {} seen_time: {} signer_public_key: {} node_id: {} is_valid: {} is_full: {} is_trusted: {} signing_hash: {} base58: {}",
            self.get_ledger_hash(),
            self.get_consensus_hash(),
            self.get_sign_time(),
            self.get_seen_time(),
            self.get_signer_public(),
            self.get_node_id(),
            self.is_valid(),
            self.is_full(),
            self.is_trusted(),
            self.get_signing_hash(),
            self.get_signer_public().to_node_public_base58(),
        )
    }
}

impl Deref for STValidation {
    type Target = STObject;

    fn deref(&self) -> &Self::Target {
        &self.object
    }
}

impl DerefMut for STValidation {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.object
    }
}

impl StBase for STValidation {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }

    fn core(&self) -> &StBaseCore {
        self.object.core()
    }

    fn core_mut(&mut self) -> &mut StBaseCore {
        self.object.core_mut()
    }

    fn stype(&self) -> crate::SerializedTypeId {
        self.object.stype()
    }

    fn full_text(&self) -> String {
        self.object.full_text()
    }

    fn text(&self) -> String {
        self.object.text()
    }

    fn json(&self, options: JsonOptions) -> JsonValue {
        self.object.json(options)
    }

    fn add(&self, serializer: &mut Serializer) {
        self.object.add(serializer);
    }

    fn is_equivalent(&self, other: &dyn StBase) -> bool {
        let Some(other) = other.as_any().downcast_ref::<Self>() else {
            return false;
        };
        self.object.is_equivalent(&other.object)
    }

    fn is_default(&self) -> bool {
        self.object.is_default()
    }
}

fn parse_validation_signing_pub_key(object: &STObject) -> Result<PublicKey, StValidationError> {
    let signing_pub_key =
        PublicKey::from_slice(&object.get_field_vl(get_field_by_symbol("sfSigningPubKey")))
            .map_err(|_| StValidationError::InvalidPublicKey)?;

    if signing_pub_key.key_type() != Some(KeyType::Secp256k1) {
        return Err(StValidationError::InvalidPublicKey);
    }

    Ok(signing_pub_key)
}

fn validation_format() -> &'static SOTemplate {
    static FORMAT: OnceLock<SOTemplate> = OnceLock::new();
    FORMAT.get_or_init(|| {
        SOTemplate::new(
            vec![
                SOElement::new(get_field_by_symbol("sfFlags"), SOEStyle::Required)
                    .expect("validation flags field should be useful"),
                SOElement::new(get_field_by_symbol("sfLedgerHash"), SOEStyle::Required)
                    .expect("validation ledger hash field should be useful"),
                SOElement::new(get_field_by_symbol("sfLedgerSequence"), SOEStyle::Required)
                    .expect("validation ledger sequence field should be useful"),
                SOElement::new(get_field_by_symbol("sfCloseTime"), SOEStyle::Optional)
                    .expect("validation close time field should be useful"),
                SOElement::new(get_field_by_symbol("sfLoadFee"), SOEStyle::Optional)
                    .expect("validation load fee field should be useful"),
                SOElement::new(get_field_by_symbol("sfAmendments"), SOEStyle::Optional)
                    .expect("validation amendments field should be useful"),
                SOElement::new(get_field_by_symbol("sfBaseFee"), SOEStyle::Optional)
                    .expect("validation base fee field should be useful"),
                SOElement::new(get_field_by_symbol("sfReserveBase"), SOEStyle::Optional)
                    .expect("validation reserve base field should be useful"),
                SOElement::new(
                    get_field_by_symbol("sfReserveIncrement"),
                    SOEStyle::Optional,
                )
                .expect("validation reserve increment field should be useful"),
                SOElement::new(get_field_by_symbol("sfSigningTime"), SOEStyle::Required)
                    .expect("validation signing time field should be useful"),
                SOElement::new(get_field_by_symbol("sfSigningPubKey"), SOEStyle::Required)
                    .expect("validation signing pub key field should be useful"),
                SOElement::new(get_field_by_symbol("sfSignature"), SOEStyle::Required)
                    .expect("validation signature field should be useful"),
                SOElement::new(get_field_by_symbol("sfConsensusHash"), SOEStyle::Optional)
                    .expect("validation consensus hash field should be useful"),
                SOElement::new(get_field_by_symbol("sfCookie"), SOEStyle::Default)
                    .expect("validation cookie field should be useful"),
                SOElement::new(get_field_by_symbol("sfValidatedHash"), SOEStyle::Optional)
                    .expect("validation validated hash field should be useful"),
                SOElement::new(get_field_by_symbol("sfServerVersion"), SOEStyle::Optional)
                    .expect("validation server version field should be useful"),
                SOElement::new(get_field_by_symbol("sfBaseFeeDrops"), SOEStyle::Optional)
                    .expect("validation base fee drops field should be useful"),
                SOElement::new(
                    get_field_by_symbol("sfReserveBaseDrops"),
                    SOEStyle::Optional,
                )
                .expect("validation reserve base drops field should be useful"),
                SOElement::new(
                    get_field_by_symbol("sfReserveIncrementDrops"),
                    SOEStyle::Optional,
                )
                .expect("validation reserve increment drops field should be useful"),
            ],
            Vec::new(),
        )
        .expect("validation template should build")
    })
}
