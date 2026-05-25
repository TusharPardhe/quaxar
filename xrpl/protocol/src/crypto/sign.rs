//! Signing helpers mirroring `xrpl/protocol/Sign.*`.

use basics::base_uint::Uint256;
use ed25519_dalek::{Signer, Verifier};
use secp256k1::{Message, Secp256k1, ecdsa::Signature as EcdsaSignature};
use sha2::{Digest, Sha512};

use crate::{
    AccountID, HashPrefix, KeyType, PublicKey, SField, STObject, SecretKey, Serializer,
    get_field_by_symbol,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SignError {
    KeyTypeMismatch,
    InvalidSecretKey,
    InvalidPublicKey,
}

pub fn sign(
    public_key: &PublicKey,
    secret_key: &SecretKey,
    message: &[u8],
) -> Result<Vec<u8>, SignError> {
    match public_key.key_type() {
        Some(KeyType::Secp256k1) => sign_secp256k1(public_key, secret_key, message),
        Some(KeyType::Ed25519) => Ok(sign_ed25519(public_key, secret_key, message)),
        None => Err(SignError::InvalidPublicKey),
    }
}

pub fn sign_digest(
    public_key: &PublicKey,
    secret_key: &SecretKey,
    digest: Uint256,
) -> Result<Vec<u8>, SignError> {
    match public_key.key_type() {
        Some(KeyType::Secp256k1) => sign_secp256k1_digest(public_key, secret_key, *digest.data()),
        Some(KeyType::Ed25519) => Err(SignError::KeyTypeMismatch),
        None => Err(SignError::InvalidPublicKey),
    }
}

pub fn verify(public_key: &PublicKey, message: &[u8], signature: &[u8]) -> bool {
    match public_key.key_type() {
        Some(KeyType::Secp256k1) => verify_secp256k1(public_key, message, signature),
        Some(KeyType::Ed25519) => verify_ed25519(public_key, message, signature),
        None => false,
    }
}

pub fn verify_digest(
    public_key: &PublicKey,
    digest: Uint256,
    signature: &[u8],
    must_be_fully_canonical: bool,
) -> bool {
    if public_key.key_type() != Some(KeyType::Secp256k1) {
        return false;
    }

    let Ok(public_key) = secp256k1::PublicKey::from_slice(public_key.as_bytes()) else {
        return false;
    };
    let Ok(mut signature_imp) = EcdsaSignature::from_der(signature) else {
        return false;
    };

    signature_imp.normalize_s();
    if must_be_fully_canonical && signature_imp.serialize_der().as_ref() != signature {
        return false;
    }

    let message = Message::from_digest(*digest.data());
    Secp256k1::verification_only()
        .verify_ecdsa(message, &signature_imp, &public_key)
        .is_ok()
}

pub fn sign_st_object(
    object: &mut STObject,
    prefix: HashPrefix,
    public_key: &PublicKey,
    secret_key: &SecretKey,
    sig_field: &'static SField,
) -> Result<(), SignError> {
    object.set_field_vl(
        get_field_by_symbol("sfSigningPubKey"),
        public_key.as_bytes(),
    );
    let mut serializer = Serializer::default();
    serializer.add32_prefix(prefix);
    object.add_without_signing_fields(&mut serializer);
    let signature = sign(public_key, secret_key, serializer.data())?;
    object.set_field_vl(sig_field, &signature);
    Ok(())
}

pub fn verify_st_object(
    object: &STObject,
    prefix: HashPrefix,
    public_key: &PublicKey,
    sig_field: &'static SField,
) -> bool {
    if !object.is_field_present(sig_field) {
        return false;
    }

    let mut serializer = Serializer::default();
    serializer.add32_prefix(prefix);
    object.add_without_signing_fields(&mut serializer);
    verify(
        public_key,
        serializer.data(),
        &object.get_field_vl(sig_field),
    )
}

pub fn build_multi_signing_data(object: &STObject, signing_id: AccountID) -> Serializer {
    let mut serializer = start_multi_signing_data(object);
    finish_multi_signing_data(signing_id, &mut serializer);
    serializer
}

pub fn start_multi_signing_data(object: &STObject) -> Serializer {
    let mut serializer = Serializer::default();
    serializer.add32_prefix(HashPrefix::TxMultiSign);
    object.add_without_signing_fields(&mut serializer);
    serializer
}

pub fn finish_multi_signing_data(signing_id: AccountID, serializer: &mut Serializer) {
    serializer.add_bit_string(signing_id);
}

fn sign_secp256k1(
    public_key: &PublicKey,
    secret_key: &SecretKey,
    message: &[u8],
) -> Result<Vec<u8>, SignError> {
    let digest = sha512_half(message);
    sign_secp256k1_digest(public_key, secret_key, digest)
}

fn sign_secp256k1_digest(
    public_key: &PublicKey,
    secret_key: &SecretKey,
    digest: [u8; 32],
) -> Result<Vec<u8>, SignError> {
    let secret = secp256k1::SecretKey::from_byte_array(*secret_key.as_bytes())
        .map_err(|_| SignError::InvalidSecretKey)?;
    let secp = Secp256k1::new();
    let derived = secp256k1::PublicKey::from_secret_key(&secp, &secret).serialize();
    if derived != *public_key.as_bytes() {
        return Err(SignError::KeyTypeMismatch);
    }

    let message = Message::from_digest(digest);
    Ok(secp.sign_ecdsa(message, &secret).serialize_der().to_vec())
}

fn sign_ed25519(public_key: &PublicKey, secret_key: &SecretKey, message: &[u8]) -> Vec<u8> {
    let signing_key = ed25519_dalek::SigningKey::from_bytes(secret_key.as_bytes());
    let mut derived = [0u8; crate::PUBLIC_KEY_LENGTH];
    derived[0] = 0xED;
    derived[1..].copy_from_slice(&signing_key.verifying_key().to_bytes());
    assert_eq!(
        derived,
        *public_key.as_bytes(),
        "ed25519 public key must match secret key"
    );
    signing_key.sign(message).to_bytes().to_vec()
}

fn verify_secp256k1(public_key: &PublicKey, message: &[u8], signature: &[u8]) -> bool {
    let digest = sha512_half(message);
    verify_digest(public_key, Uint256::from_array(digest), signature, true)
}

fn verify_ed25519(public_key: &PublicKey, message: &[u8], signature: &[u8]) -> bool {
    if public_key.as_bytes()[0] != 0xED {
        return false;
    }

    let Ok(verifying_key) = ed25519_dalek::VerifyingKey::from_bytes(
        &public_key.as_bytes()[1..]
            .try_into()
            .expect("ed25519 public key width"),
    ) else {
        return false;
    };
    let Ok(signature) = ed25519_dalek::Signature::from_slice(signature) else {
        return false;
    };
    verifying_key.verify(message, &signature).is_ok()
}

fn sha512_half(message: &[u8]) -> [u8; 32] {
    let digest = Sha512::digest(message);
    let mut half = [0u8; 32];
    half.copy_from_slice(&digest[..32]);
    half
}

#[cfg(test)]
mod tests {
    use basics::base_uint::Uint256;

    use crate::{HashPrefix, STObject, get_field_by_symbol};

    use super::{
        AccountID, KeyType, SecretKey, build_multi_signing_data, finish_multi_signing_data, sign,
        sign_digest, sign_st_object, start_multi_signing_data, verify, verify_digest,
        verify_st_object,
    };

    #[test]
    fn secp256k1_sign_and_verify_round_trip() {
        let secret = SecretKey::from_bytes([7u8; 32]);
        let public =
            crate::derive_public_key(KeyType::Secp256k1, &secret).expect("valid secp256k1 key");
        let signature = sign(&public, &secret, b"sign me").expect("signature");
        assert!(verify(&public, b"sign me", &signature));
        assert!(!verify(&public, b"tampered", &signature));
    }

    #[test]
    fn secp256k1_sign_digest_and_verify_digest_round_trip() {
        let secret = SecretKey::from_bytes([5u8; 32]);
        let public =
            crate::derive_public_key(KeyType::Secp256k1, &secret).expect("valid secp256k1 key");
        let digest =
            Uint256::from_hex("2A2B2C2D2E2F30313233343536373839404142434445464748494A4B4C4D4E4F")
                .expect("digest hex should parse");
        let signature = sign_digest(&public, &secret, digest).expect("digest signature");

        assert!(verify_digest(&public, digest, &signature, true));
        assert!(!verify_digest(
            &public,
            Uint256::from_hex("AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA")
                .expect("digest hex should parse"),
            &signature,
            true,
        ));
    }

    #[test]
    fn ed25519_sign_and_verify_round_trip() {
        let secret = SecretKey::from_bytes([9u8; 32]);
        let public =
            crate::derive_public_key(KeyType::Ed25519, &secret).expect("valid ed25519 key");
        let signature = sign(&public, &secret, b"sign me").expect("signature");
        assert!(verify(&public, b"sign me", &signature));
        assert!(!verify(&public, b"tampered", &signature));
    }

    #[test]
    fn verify_digest_normalizes_high_s_only_when_requested() {
        let public = crate::PublicKey::from_bytes([
            0x03, 0x1E, 0xE9, 0x9D, 0x2B, 0x78, 0x6A, 0xB3, 0xB0, 0x99, 0x13, 0x25, 0xF2, 0xDE,
            0x84, 0x89, 0x24, 0x6A, 0x6A, 0x3F, 0xDB, 0x70, 0x0F, 0x6D, 0x05, 0x11, 0xB1, 0xD8,
            0x0C, 0xF5, 0xF4, 0xCD, 0x43,
        ]);
        let digest =
            Uint256::from_hex("A4965CA63B7D8562736CEEC36DFA5A11BF426EB65BE8EA3F7A49AE363032DA0D")
                .expect("digest hex should parse");
        let signature = basics::string_utilities::str_unhex(
            "3046022100839C1FBC5304DE944F697C9F4B1D01D1FAEBA32D751C0F7ACB21AC8A0F436A72022100E89BD46BB3A5A62ADC679F659B7CE876D83EE297C7A5587B2011C4FCC72EAB45",
        )
        .expect("signature hex should decode");

        assert!(!verify_digest(&public, digest, &signature, true));
        assert!(verify_digest(&public, digest, &signature, false));
    }

    #[test]
    fn sign_st_object_sets_signing_fields() {
        let mut object = STObject::new(get_field_by_symbol("sfTransaction"));
        object.set_field_u16(get_field_by_symbol("sfTransactionType"), 0);
        object.set_account_id(get_field_by_symbol("sfAccount"), AccountID::from_u64(1));
        object.set_field_u32(get_field_by_symbol("sfSequence"), 1);
        object.set_field_amount(
            get_field_by_symbol("sfFee"),
            crate::STAmount::from_xrp_amount(crate::XRPAmount::from_drops(10)),
        );

        let secret = SecretKey::from_bytes([3u8; 32]);
        let public =
            crate::derive_public_key(KeyType::Secp256k1, &secret).expect("valid secp256k1 key");
        sign_st_object(
            &mut object,
            HashPrefix::TxSign,
            &public,
            &secret,
            get_field_by_symbol("sfTxnSignature"),
        )
        .expect("sign object");

        assert_eq!(
            object.get_field_vl(get_field_by_symbol("sfSigningPubKey")),
            public.as_bytes().to_vec()
        );
        assert!(verify_st_object(
            &object,
            HashPrefix::TxSign,
            &public,
            get_field_by_symbol("sfTxnSignature")
        ));
    }

    #[test]
    fn multi_signing_data_appends_signer_account() {
        let object = STObject::new(get_field_by_symbol("sfTransaction"));
        let signer = AccountID::from_u64(9);
        let start = start_multi_signing_data(&object);
        let mut finished = start.clone();
        finish_multi_signing_data(signer, &mut finished);
        let combined = build_multi_signing_data(&object, signer);
        assert_eq!(finished.data(), combined.data());
        assert!(finished.size() > start.size());
    }
}
