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
    with_base58(account_id, str::to_owned)
}

/// Call `f` with the classic address of `account_id`.
///
/// Addresses repeat heavily within a response (an owner across its offers,
/// an issuer across trust lines and amounts), and each encoding costs a
/// double SHA-256 checksum plus a base58 conversion. A small per-thread
/// direct-mapped memo of the encoding (a pure function of the ID) turns
/// repeats into a 20-byte compare. Account IDs are RIPEMD-160 outputs, so
/// their low bytes index the table uniformly.
pub fn with_base58<R>(account_id: AccountID, f: impl FnOnce(&str) -> R) -> R {
    const SLOTS: usize = 1024;
    #[derive(Clone, Copy)]
    struct Slot {
        id: [u8; 20],
        len: u8,
        text: [u8; 35],
    }
    thread_local! {
        static MEMO: std::cell::RefCell<Vec<Slot>> = const { std::cell::RefCell::new(Vec::new()) };
    }

    let id: [u8; 20] = *account_id.data();
    let index = (usize::from(id[18]) << 8 | usize::from(id[19])) % SLOTS;
    MEMO.with(|memo| {
        let mut memo = memo.borrow_mut();
        if memo.is_empty() {
            memo.resize(
                SLOTS,
                Slot {
                    id: [0; 20],
                    len: 0,
                    text: [0; 35],
                },
            );
        }
        let slot = &mut memo[index];
        if slot.len == 0 || slot.id != id {
            let encoded = crate::base::b58_fast::encode_token(ACCOUNT_ID_TOKEN_TYPE, &id);
            debug_assert!(encoded.len() <= 35);
            slot.id = id;
            slot.len = encoded.len() as u8;
            slot.text[..encoded.len()].copy_from_slice(encoded.as_bytes());
        }
        let text = std::str::from_utf8(&slot.text[..usize::from(slot.len)])
            .expect("base58 alphabet is ASCII");
        f(text)
    })
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
    fn memoized_addresses_match_direct_encoding_including_slot_collisions() {
        let mut state = 0x0BAD_C0DE_1234_5678_u64;
        let mut ids = Vec::new();
        for _ in 0..5_000 {
            let mut bytes = [0_u8; 20];
            for byte in &mut bytes {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                *byte = state as u8;
            }
            // Force collisions on the memo index (last two bytes).
            if ids.len() % 3 == 0 {
                bytes[18] = 0;
                bytes[19] = 7;
            }
            ids.push(AccountID::from_array(bytes));
        }
        for round in 0..2 {
            for id in &ids {
                let expected = crate::base::b58_fast::encode_token(0, id.data());
                assert_eq!(to_base58(*id), expected, "round {round}");
            }
        }
    }

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
