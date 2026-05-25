//! Tests for the account objects RPC handler.

//! Tests for the account objects RPC handler.

use std::{
    collections::{BTreeMap, HashMap},
    time::Duration,
};

use basics::base_uint::{Uint160, Uint256};
use basics::sha_map_hash::SHAMapHash;
use protocol::{
    AccountID, JsonValue, Keylet, LedgerEntryType, STArray, STLedgerEntry, STObject, STVector256,
    account_keylet, child_keylet, get_field_by_symbol, nft_page_keylet, nft_page_min_keylet,
    owner_dir_keylet, to_base58,
};
use rpc::Role;
use rpc::{AccountObjectsRequest, do_account_objects};
use rpc::{AccountObjectsView, collect_account_nfts};
use rpc::{LedgerLookupLedger, LedgerLookupSource};
use shamap::traversal::TraversalError;

#[derive(Debug, Default)]
struct FakeSource {
    ledger: Option<LedgerLookupLedger>,
    entries: HashMap<Keylet, STLedgerEntry>,
    fail_on_read: Option<Keylet>,
    fail_on_succ: Option<Uint256>,
}

impl LedgerLookupSource for FakeSource {
    fn get_ledger_by_hash(&self, hash: Uint256) -> Option<LedgerLookupLedger> {
        self.ledger.filter(|ledger| ledger.hash == hash)
    }

    fn get_ledger_by_seq(&self, seq: u32) -> Option<LedgerLookupLedger> {
        self.ledger.filter(|ledger| ledger.seq == seq)
    }

    fn get_current_ledger(&self) -> Option<LedgerLookupLedger> {
        self.ledger
    }

    fn get_closed_ledger(&self) -> Option<LedgerLookupLedger> {
        self.ledger.filter(|ledger| !ledger.open)
    }

    fn get_validated_ledger(&self) -> Option<LedgerLookupLedger> {
        self.ledger.filter(|ledger| !ledger.open)
    }

    fn get_valid_ledger_index(&self) -> u32 {
        self.ledger.map(|ledger| ledger.seq).unwrap_or_default()
    }

    fn get_validated_ledger_age(&self) -> Duration {
        Duration::from_secs(1)
    }

    fn is_validated(&self, ledger: &LedgerLookupLedger) -> bool {
        !ledger.open && self.ledger == Some(*ledger)
    }
}

impl AccountObjectsView for FakeSource {
    fn read_entry(&self, keylet: Keylet) -> Result<Option<STLedgerEntry>, TraversalError> {
        if self.fail_on_read == Some(keylet) {
            return Err(TraversalError::MissingNode(SHAMapHash::new(keylet.key)));
        }

        Ok(self.entries.get(&keylet).cloned())
    }

    fn succ_key(
        &self,
        key: Uint256,
        last: Option<Uint256>,
    ) -> Result<Option<Uint256>, TraversalError> {
        if self.fail_on_succ == Some(key) {
            return Err(TraversalError::MissingNode(SHAMapHash::new(key)));
        }

        Ok(self
            .entries
            .keys()
            .filter(|keylet| keylet.entry_type == LedgerEntryType::NFTokenPage)
            .map(|keylet| keylet.key)
            .filter(|candidate| *candidate > key)
            .filter(|candidate| last.map(|bound| *candidate < bound).unwrap_or(true))
            .min())
    }
}

pub(super) fn object(entries: impl IntoIterator<Item = (&'static str, JsonValue)>) -> JsonValue {
    JsonValue::Object(
        entries
            .into_iter()
            .map(|(key, value)| (key.to_owned(), value))
            .collect::<BTreeMap<_, _>>(),
    )
}

pub(super) fn error_fields(value: &JsonValue) -> (&str, i64, &str) {
    let JsonValue::Object(object) = value else {
        panic!("expected error object");
    };
    let JsonValue::String(error) = object.get("error").expect("error") else {
        panic!("expected error string");
    };
    let JsonValue::Signed(code) = object.get("error_code").expect("error_code") else {
        panic!("expected error code");
    };
    let JsonValue::String(message) = object.get("error_message").expect("error_message") else {
        panic!("expected error message");
    };
    (error, *code, message)
}

pub(super) fn sample_account(fill: u8) -> AccountID {
    AccountID::from_array([fill; 20])
}

pub(super) fn sample_hash(fill: u8) -> Uint256 {
    Uint256::from_array([fill; 32])
}

fn closed_ledger() -> LedgerLookupLedger {
    LedgerLookupLedger {
        hash: sample_hash(0xAB),
        seq: 91,
        open: false,
    }
}

pub(super) fn account_root_key(account: AccountID) -> Keylet {
    account_keylet(Uint160::from_slice(account.data()).expect("account width"))
}

pub(super) fn owner_root_key(account: AccountID) -> Keylet {
    owner_dir_keylet(Uint160::from_slice(account.data()).expect("account width"))
}

pub(super) fn make_account_root(account: AccountID) -> STLedgerEntry {
    STLedgerEntry::from_type_and_key(LedgerEntryType::AccountRoot, account_root_key(account).key)
}

pub(super) fn make_owner_dir_page(
    account: AccountID,
    entries: &[Uint256],
    next: Option<u64>,
) -> STLedgerEntry {
    let root = owner_root_key(account);
    let mut page = STLedgerEntry::new(root);
    let mut indexes = STVector256::with_field(get_field_by_symbol("sfIndexes"));
    for entry in entries {
        indexes.push_back(*entry);
    }
    page.set_field_v256(get_field_by_symbol("sfIndexes"), indexes);
    if let Some(next) = next {
        page.set_field_u64(get_field_by_symbol("sfIndexNext"), next);
    }
    page
}

pub(super) fn make_offer_entry(key: Uint256) -> STLedgerEntry {
    STLedgerEntry::from_type_and_key(LedgerEntryType::Offer, key)
}

pub(super) fn make_check_entry(key: Uint256) -> STLedgerEntry {
    STLedgerEntry::from_type_and_key(LedgerEntryType::Check, key)
}

pub(super) fn make_nft_id(
    flags: u16,
    fee: u16,
    issuer: AccountID,
    taxon: u32,
    serial: u32,
) -> Uint256 {
    let cipher = taxon ^ ((384_160_001u32.wrapping_mul(serial)).wrapping_add(2_459));
    let mut bytes = [0u8; 32];
    bytes[..2].copy_from_slice(&flags.to_be_bytes());
    bytes[2..4].copy_from_slice(&fee.to_be_bytes());
    bytes[4..24].copy_from_slice(issuer.data());
    bytes[24..28].copy_from_slice(&cipher.to_be_bytes());
    bytes[28..32].copy_from_slice(&serial.to_be_bytes());
    Uint256::from_array(bytes)
}

pub(super) fn make_nft_entry(nft_id: Uint256) -> STObject {
    let mut nft = STObject::make_inner_object(get_field_by_symbol("sfNFToken"));
    nft.set_field_h256(get_field_by_symbol("sfNFTokenID"), nft_id);
    nft
}

pub(super) fn make_nft_page(
    key: Uint256,
    token_ids: &[Uint256],
    next_page_min: Option<Uint256>,
) -> STLedgerEntry {
    let mut page = STLedgerEntry::from_type_and_key(LedgerEntryType::NFTokenPage, key);
    let mut tokens = STArray::new(get_field_by_symbol("sfNFTokens"));
    for token_id in token_ids {
        tokens.push_back(make_nft_entry(*token_id));
    }
    page.set_field_array(get_field_by_symbol("sfNFTokens"), tokens);
    if let Some(next_page_min) = next_page_min {
        page.set_field_h256(get_field_by_symbol("sfNextPageMin"), next_page_min);
    }
    page
}

mod pagination;
mod validation;
