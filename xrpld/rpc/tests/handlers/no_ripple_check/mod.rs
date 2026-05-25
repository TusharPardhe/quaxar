//! Tests for the no ripple check RPC handler.

//! Tests for the no ripple check RPC handler.

#![allow(clippy::too_many_arguments)]

use std::{collections::BTreeMap, time::Duration};

use basics::base_uint::{Uint160, Uint256};
use protocol::{
    AccountID, Issue, JsonValue, LedgerEntryType, STAmount, STLedgerEntry, STVector256,
    currency_from_string, get_field_by_symbol, line, lsfDefaultRipple, lsfLowNoRipple,
    owner_dir_keylet, tfClearNoRipple, tfSetNoRipple, to_base58,
};
use rpc::{
    LedgerLookupLedger, LedgerLookupSource, NoRippleCheckRequest, NoRippleCheckSource, RpcRole,
    do_no_ripple_check,
};

#[derive(Debug, Default)]
struct FakeSource {
    ledger: Option<LedgerLookupLedger>,
    account_roots: BTreeMap<AccountID, STLedgerEntry>,
    owner_pages: BTreeMap<(AccountID, u64), STLedgerEntry>,
    children: BTreeMap<Uint256, STLedgerEntry>,
    fee_drops: u64,
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

impl NoRippleCheckSource for FakeSource {
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

    fn transaction_fee_drops(&self, _ledger: &LedgerLookupLedger) -> u64 {
        self.fee_drops
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

pub(super) fn make_account_root(account: AccountID, default_ripple: bool) -> STLedgerEntry {
    let account_key = Uint160::from_slice(account.data()).expect("account width");
    let mut sle = STLedgerEntry::from_type_and_key(
        LedgerEntryType::AccountRoot,
        protocol::account_keylet(account_key).key,
    );
    sle.set_account_id(get_field_by_symbol("sfAccount"), account);
    sle.set_field_u32(get_field_by_symbol("sfSequence"), 7);
    if default_ripple {
        sle.set_field_u32(get_field_by_symbol("sfFlags"), lsfDefaultRipple);
    }
    sle
}

pub(super) fn make_owner_page(account: AccountID, entries: &[Uint256]) -> STLedgerEntry {
    let root = owner_dir_keylet(Uint160::from_slice(account.data()).expect("account width"));
    let mut page = STLedgerEntry::new(root);
    let mut indexes = STVector256::with_field(get_field_by_symbol("sfIndexes"));
    for entry in entries {
        indexes.push_back(*entry);
    }
    page.set_field_v256(get_field_by_symbol("sfIndexes"), indexes);
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
    sle
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

mod detection;
mod recommendations;
