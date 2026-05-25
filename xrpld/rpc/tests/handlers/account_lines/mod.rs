//! Tests for the account_lines RPC handler.

//! Tests for the account lines RPC handler.

#![allow(clippy::too_many_arguments)]

use std::{collections::BTreeMap, time::Duration};

use basics::base_uint::{Uint160, Uint256};
use protocol::{
    AccountID, Issue, JsonValue, LedgerEntryType, STAmount, STLedgerEntry, STVector256,
    account_keylet, currency_from_string, get_field_by_symbol, line, lsfLowAuth, lsfLowReserve,
    owner_dir_keylet, page_keylet, to_base58,
};
use rpc::Role;
use rpc::{AccountLinesRequest, AccountLinesSource, do_account_lines};
use rpc::{LedgerLookupLedger, LedgerLookupSource};

#[derive(Debug, Default)]
struct FakeSource {
    ledger: Option<LedgerLookupLedger>,
    account_roots: BTreeMap<AccountID, STLedgerEntry>,
    owner_pages: BTreeMap<(AccountID, u64), STLedgerEntry>,
    children: BTreeMap<Uint256, STLedgerEntry>,
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

impl AccountLinesSource for FakeSource {
    fn read_account_root(
        &self,
        _ledger: &LedgerLookupLedger,
        account_id: AccountID,
    ) -> Option<STLedgerEntry> {
        self.account_roots.get(&account_id).cloned()
    }

    fn read_owner_dir_page(
        &self,
        _ledger: &LedgerLookupLedger,
        account_id: AccountID,
        page_index: u64,
    ) -> Option<STLedgerEntry> {
        self.owner_pages.get(&(account_id, page_index)).cloned()
    }

    fn read_child_entry(
        &self,
        _ledger: &LedgerLookupLedger,
        entry_index: Uint256,
    ) -> Option<STLedgerEntry> {
        self.children.get(&entry_index).cloned()
    }
}

pub(super) fn sample_account(fill: u8) -> AccountID {
    AccountID::from_array([fill; 20])
}

pub(super) fn sample_hash(fill: u8) -> Uint256 {
    Uint256::from_array([fill; 32])
}

pub(super) fn object(params: impl IntoIterator<Item = (&'static str, JsonValue)>) -> JsonValue {
    JsonValue::Object(
        params
            .into_iter()
            .map(|(key, value)| (key.to_owned(), value))
            .collect(),
    )
}

pub(super) fn closed_ledger() -> LedgerLookupLedger {
    LedgerLookupLedger {
        hash: sample_hash(0xAA),
        seq: 101,
        open: false,
    }
}

pub(super) fn make_account_root(account: AccountID) -> STLedgerEntry {
    let account_key = Uint160::from_slice(account.data()).expect("account width");
    let mut sle = STLedgerEntry::from_type_and_key(
        LedgerEntryType::AccountRoot,
        account_keylet(account_key).key,
    );
    sle.set_account_id(get_field_by_symbol("sfAccount"), account);
    sle.set_field_u32(get_field_by_symbol("sfSequence"), 7);
    sle.set_field_u32(get_field_by_symbol("sfOwnerCount"), 2);
    sle
}

pub(super) fn make_owner_page(
    account: AccountID,
    page_index: u64,
    entries: &[Uint256],
    next: u64,
) -> STLedgerEntry {
    let root = owner_dir_keylet(Uint160::from_slice(account.data()).expect("account width"));
    let keylet = if page_index == 0 {
        root
    } else {
        page_keylet(root, page_index)
    };
    let mut page = STLedgerEntry::new(keylet);
    let mut indexes = STVector256::with_field(get_field_by_symbol("sfIndexes"));
    for entry in entries {
        indexes.push_back(*entry);
    }
    page.set_field_v256(get_field_by_symbol("sfIndexes"), indexes);
    if next != 0 {
        page.set_field_u64(get_field_by_symbol("sfIndexNext"), next);
    }
    page
}

pub(super) fn make_trust_line(
    account: AccountID,
    peer: AccountID,
    currency: &str,
    balance_value: u64,
    balance_negative: bool,
    flags: u32,
    low_node: u64,
    high_node: u64,
) -> STLedgerEntry {
    let (low, high) = if account < peer {
        (account, peer)
    } else {
        (peer, account)
    };
    let currency = currency_from_string(currency);
    let mut sle = STLedgerEntry::new(line(low, high, currency));
    sle.set_field_amount(
        get_field_by_symbol("sfBalance"),
        STAmount::new_with_asset(
            get_field_by_symbol("sfBalance"),
            Issue::new(currency, low),
            balance_value,
            0,
            balance_negative,
        ),
    );
    sle.set_field_amount(
        get_field_by_symbol("sfLowLimit"),
        STAmount::new_with_asset(
            get_field_by_symbol("sfLowLimit"),
            Issue::new(currency, low),
            500,
            0,
            false,
        ),
    );
    sle.set_field_amount(
        get_field_by_symbol("sfHighLimit"),
        STAmount::new_with_asset(
            get_field_by_symbol("sfHighLimit"),
            Issue::new(currency, high),
            800,
            0,
            false,
        ),
    );
    sle.set_field_u32(get_field_by_symbol("sfFlags"), flags);
    sle.set_field_u64(get_field_by_symbol("sfLowNode"), low_node);
    sle.set_field_u64(get_field_by_symbol("sfHighNode"), high_node);
    sle.set_field_u32(get_field_by_symbol("sfLowQualityIn"), 11);
    sle.set_field_u32(get_field_by_symbol("sfLowQualityOut"), 12);
    sle.set_field_u32(get_field_by_symbol("sfHighQualityIn"), 21);
    sle.set_field_u32(get_field_by_symbol("sfHighQualityOut"), 22);
    sle
}

mod pagination;
mod validation;
