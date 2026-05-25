//! Tests for the ledger entry RPC handler.

use std::collections::BTreeMap;

use basics::{
    base_uint::{Uint160, Uint256},
    str_hex::str_hex,
};
use protocol::{
    AccountID, BridgeBuilder, Currency, Issue, JsonOptions, JsonValue, LedgerEntryType, MPTID,
    STArray, STLedgerEntry, Serializer, StBase, XChainOwnedClaimIDBuilder,
    XChainOwnedCreateAccountClaimIDBuilder, account_keylet, amendments_key,
    deposit_preauth_credentials_keylet, get_field_by_symbol, line, offer_keylet, owner_dir_keylet,
    page_keylet, parse_base58_account_id, to_base58,
};
use sha2::{Digest, Sha512};

use rpc::{LedgerEntryRequest, LedgerEntrySource, do_ledger_entry};

pub(super) fn object(entries: impl IntoIterator<Item = (&'static str, JsonValue)>) -> JsonValue {
    JsonValue::Object(
        entries
            .into_iter()
            .map(|(key, value)| (key.to_owned(), value))
            .collect::<BTreeMap<_, _>>(),
    )
}

#[derive(Debug, Clone)]
pub(super) struct FakeSource {
    current: Option<rpc::LedgerLookupLedger>,
    closed: Option<rpc::LedgerLookupLedger>,
    validated: Option<rpc::LedgerLookupLedger>,
    by_seq: BTreeMap<u32, rpc::LedgerLookupLedger>,
    entries: BTreeMap<Uint256, STLedgerEntry>,
}

impl rpc::LedgerLookupSource for FakeSource {
    fn get_ledger_by_hash(&self, hash: Uint256) -> Option<rpc::LedgerLookupLedger> {
        self.by_seq
            .values()
            .copied()
            .find(|ledger| ledger.hash == hash)
    }

    fn get_ledger_by_seq(&self, seq: u32) -> Option<rpc::LedgerLookupLedger> {
        self.by_seq.get(&seq).copied()
    }

    fn get_current_ledger(&self) -> Option<rpc::LedgerLookupLedger> {
        self.current
    }

    fn get_closed_ledger(&self) -> Option<rpc::LedgerLookupLedger> {
        self.closed
    }

    fn get_validated_ledger(&self) -> Option<rpc::LedgerLookupLedger> {
        self.validated
    }

    fn get_valid_ledger_index(&self) -> u32 {
        self.validated.map(|ledger| ledger.seq).unwrap_or(0)
    }

    fn get_validated_ledger_age(&self) -> std::time::Duration {
        std::time::Duration::from_secs(1)
    }

    fn is_validated(&self, ledger: &rpc::LedgerLookupLedger) -> bool {
        self.validated
            .is_some_and(|validated| validated.seq == ledger.seq)
    }
}

impl LedgerEntrySource for FakeSource {
    fn read_ledger_entry(
        &self,
        _ledger: &rpc::LedgerLookupLedger,
        entry_index: Uint256,
    ) -> Option<STLedgerEntry> {
        self.entries.get(&entry_index).cloned()
    }
}

pub(super) fn account(fill: u8) -> AccountID {
    AccountID::from_array([fill; 20])
}

pub(super) fn account160(account: AccountID) -> Uint160 {
    Uint160::from_slice(account.data()).expect("account width")
}

pub(super) fn currency(fill: u8) -> Currency {
    Currency::from_array([fill; 20])
}

pub(super) fn closed_ledger() -> rpc::LedgerLookupLedger {
    rpc::LedgerLookupLedger {
        hash: Uint256::from_array([0x44; 32]),
        seq: 9,
        open: false,
    }
}

pub(super) fn open_ledger() -> rpc::LedgerLookupLedger {
    rpc::LedgerLookupLedger {
        hash: Uint256::from_array([0x55; 32]),
        seq: 10,
        open: true,
    }
}

pub(super) fn source_with(entries: Vec<STLedgerEntry>) -> FakeSource {
    FakeSource {
        current: Some(open_ledger()),
        closed: Some(closed_ledger()),
        validated: Some(closed_ledger()),
        by_seq: BTreeMap::from([(9, closed_ledger()), (10, open_ledger())]),
        entries: entries
            .into_iter()
            .map(|entry| (*entry.key(), entry))
            .collect(),
    }
}

pub(super) fn sha512_half(parts: &[&[u8]]) -> Uint256 {
    let mut hasher = Sha512::new();
    for part in parts {
        hasher.update(part);
    }
    let digest = hasher.finalize();
    Uint256::from_slice(&digest[..32]).expect("SHA-512 half output must contain 32 bytes")
}

pub(super) fn index_hash_with_slices(namespace: u16, slices: &[&[u8]]) -> Uint256 {
    let mut hasher = Sha512::new();
    hasher.update(namespace.to_be_bytes());
    for slice in slices {
        hasher.update(slice);
    }
    let digest = hasher.finalize();
    Uint256::from_slice(&digest[..32]).expect("SHA-512 half output must contain 32 bytes")
}

pub(super) fn bridge_key(
    locking_chain_door: AccountID,
    locking_chain_currency: Currency,
    issuing_chain_door: AccountID,
    issuing_chain_currency: Currency,
    locking_chain_send: bool,
) -> Uint256 {
    let (door, currency) = if locking_chain_send {
        (locking_chain_door, locking_chain_currency)
    } else {
        (issuing_chain_door, issuing_chain_currency)
    };

    index_hash_with_slices(b'H' as u16, &[door.data(), currency.data()])
}

pub(super) fn xchain_claim_id_key(
    locking_chain_door: AccountID,
    locking_chain_issue: Issue,
    issuing_chain_door: AccountID,
    issuing_chain_issue: Issue,
    seq: u32,
) -> Uint256 {
    index_hash_with_slices(
        b'Q' as u16,
        &[
            locking_chain_door.data(),
            locking_chain_issue.account.data(),
            locking_chain_issue.currency.data(),
            issuing_chain_door.data(),
            issuing_chain_issue.account.data(),
            issuing_chain_issue.currency.data(),
            &u64::from(seq).to_be_bytes(),
        ],
    )
}

pub(super) fn xchain_create_account_claim_id_key(
    locking_chain_door: AccountID,
    locking_chain_issue: Issue,
    issuing_chain_door: AccountID,
    issuing_chain_issue: Issue,
    seq: u32,
) -> Uint256 {
    index_hash_with_slices(
        b'K' as u16,
        &[
            locking_chain_door.data(),
            locking_chain_issue.account.data(),
            locking_chain_issue.currency.data(),
            issuing_chain_door.data(),
            issuing_chain_issue.account.data(),
            issuing_chain_issue.currency.data(),
            &u64::from(seq).to_be_bytes(),
        ],
    )
}

pub(super) fn mpt_issuance_key(mpt_id: MPTID) -> Uint256 {
    index_hash_with_slices(b'~' as u16, &[mpt_id.data()])
}

pub(super) fn mptoken_key(issuance_key: Uint256, holder: AccountID) -> Uint256 {
    index_hash_with_slices(b't' as u16, &[issuance_key.data(), holder.data()])
}

pub(super) fn deposit_preauth_credentials_key(
    owner: AccountID,
    creds: &[(AccountID, &[u8])],
) -> Uint256 {
    let mut hashes: Vec<Uint256> = creds
        .iter()
        .map(|(issuer, cred_type)| sha512_half(&[issuer.data(), cred_type]))
        .collect();
    hashes.sort();
    hashes.dedup();
    deposit_preauth_credentials_keylet(account160(owner), &hashes).key
}

pub(super) fn request(params: JsonValue, api_version: u32) -> LedgerEntryRequest<'static> {
    let params = Box::leak(Box::new(params));
    LedgerEntryRequest {
        params,
        api_version,
        role: rpc::RpcRole::User,
    }
}

pub(super) fn serialize_hex(entry: &STLedgerEntry) -> String {
    let mut serializer = Serializer::new(256);
    entry.add(&mut serializer);
    str_hex(serializer.data())
}

pub(super) fn json_object(value: &JsonValue) -> &BTreeMap<String, JsonValue> {
    let JsonValue::Object(object) = value else {
        panic!("expected object");
    };
    object
}

#[track_caller]
pub(super) fn assert_result_index(result: &JsonValue, expected: Uint256) {
    let JsonValue::Object(object) = result else {
        panic!("expected object");
    };
    assert_eq!(
        object.get("index"),
        Some(&JsonValue::String(expected.to_string()))
    );
}

#[track_caller]
pub(super) fn assert_error(result: &JsonValue, expected: &str) {
    let JsonValue::Object(object) = result else {
        panic!("expected object");
    };
    assert_eq!(
        object.get("error"),
        Some(&JsonValue::String(expected.to_owned()))
    );
}

mod bridge;
mod individual;
mod selectors;
