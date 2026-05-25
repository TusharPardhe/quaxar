//! `AccountID` helpers ported from `xrpl/protocol/AccountID.h`.

use basics::base_uint::BaseUInt;
use bs58::Alphabet;
use sha2::{Digest, Sha256};
use std::sync::OnceLock;

use crate::ripesha;

const XRPL_BASE58_ALPHABET: &[u8; 58] =
    b"rpshnaf39wBUDNEGHJKLM4PQRST7VWXYZ2bcdeCg65jkm8oFqi1tuvAxyz";
const ACCOUNT_ID_TOKEN_TYPE: u8 = 0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct AccountIdTag;

pub type AccountId = BaseUInt<20, AccountIdTag>;
pub type AccountID = AccountId;

pub fn calc_account_id(public_key: &[u8]) -> AccountID {
    let digest = ripesha(public_key);
    AccountID::from_slice(&digest).expect("ripemd160 width should match AccountID")
}

pub fn to_base58(account_id: AccountID) -> String {
    let mut payload = Vec::with_capacity(1 + AccountID::size() + 4);
    payload.push(ACCOUNT_ID_TOKEN_TYPE);
    payload.extend_from_slice(account_id.data());
    payload.extend_from_slice(&checksum(&payload));
    bs58::encode(payload)
        .with_alphabet(xrpl_base58_alphabet())
        .into_string()
}

pub fn parse_base58_account_id(value: &str) -> Option<AccountID> {
    let decoded = bs58::decode(value)
        .with_alphabet(xrpl_base58_alphabet())
        .into_vec()
        .ok()?;
    if decoded.len() != 1 + AccountID::size() + 4 {
        return None;
    }
    if decoded[0] != ACCOUNT_ID_TOKEN_TYPE {
        return None;
    }

    let checksum_offset = 1 + AccountID::size();
    let expected = checksum(&decoded[..checksum_offset]);
    if decoded[checksum_offset..] != expected {
        return None;
    }

    AccountID::from_slice(&decoded[1..checksum_offset])
}

pub fn xrp_account() -> AccountID {
    AccountID::zero()
}

pub fn no_account() -> AccountID {
    AccountID::from_u64(1)
}

pub fn to_issuer(out: &mut AccountID, value: &str) -> bool {
    if out.parse_hex(value) {
        return true;
    }

    match parse_base58_account_id(value) {
        Some(account) => {
            *out = account;
            true
        }
        None => false,
    }
}

fn checksum(message: &[u8]) -> [u8; 4] {
    let first = Sha256::digest(message);
    let second = Sha256::digest(first);
    let mut checksum = [0u8; 4];
    checksum.copy_from_slice(&second[..4]);
    checksum
}

fn xrpl_base58_alphabet() -> &'static Alphabet {
    static ALPHABET: OnceLock<Alphabet> = OnceLock::new();
    ALPHABET.get_or_init(|| {
        Alphabet::new(XRPL_BASE58_ALPHABET).expect("XRPL base58 alphabet should remain valid")
    })
}

#[cfg(test)]
mod tests {
    use super::{
        AccountID, calc_account_id, no_account, parse_base58_account_id, to_base58, xrp_account,
    };
    use crate::genesis_public_key;

    #[test]
    fn base58_zero_and_genesis_vectors() {
        assert_eq!(to_base58(AccountID::zero()), "rrrrrrrrrrrrrrrrrrrrrhoLvTp");
        assert_eq!(
            to_base58(calc_account_id(&genesis_public_key())),
            "rHb9CJAWyB4rj91VRWn96DkukG4bwdtyTh"
        );
        assert_eq!(
            parse_base58_account_id("rHb9CJAWyB4rj91VRWn96DkukG4bwdtyTh"),
            Some(calc_account_id(&genesis_public_key()))
        );
        assert_eq!(xrp_account(), AccountID::zero());
        assert_eq!(no_account(), AccountID::from_u64(1));
    }
}
