//! Integration tests that pin the narrowed Rust `LedgerStateFix.cpp`
//! transactor shell to the current C++ behavior.

use std::{cell::Cell, collections::BTreeMap, sync::Arc};

use basics::base_uint::{Uint160, Uint256};
use ledger::{Fees, LedgerHeader, ReadView, ReadViewTx, Rules, ViewError};
use protocol::{
    AccountID, LedgerEntryType, STLedgerEntry, STTx, Ter, TxType, get_field_by_symbol,
    quality_from_key, trans_token,
};
use tx::{
    LedgerStateFixType, run_ledger_state_fix_do_apply, run_ledger_state_fix_preclaim,
    run_ledger_state_fix_preflight, run_ledger_state_fix_read_view_preclaim,
};

#[test]
fn ledger_state_fix_preflight_requires_owner_for_nft_page_link() {
    assert_eq!(
        run_ledger_state_fix_preflight(LedgerStateFixType::NfTokenPageLink, false),
        Ter::TEM_INVALID
    );
    assert_eq!(
        run_ledger_state_fix_preflight(LedgerStateFixType::NfTokenPageLink, true),
        Ter::TES_SUCCESS
    );
}

#[derive(Debug, Default)]
struct TestReadView {
    entries: BTreeMap<Uint256, Arc<STLedgerEntry>>,
    fail_reads: bool,
}

impl TestReadView {
    fn insert(&mut self, entry: STLedgerEntry) {
        self.entries.insert(*entry.key(), Arc::new(entry));
    }
}

impl ReadView for TestReadView {
    fn open(&self) -> bool {
        false
    }

    fn header(&self) -> LedgerHeader {
        LedgerHeader::default()
    }

    fn fees(&self) -> Fees {
        Fees::default()
    }

    fn rules(&self) -> Rules {
        Rules::default()
    }

    fn exists(&self, keylet: protocol::Keylet) -> Result<bool, ViewError> {
        Ok(self.entries.contains_key(&keylet.key))
    }

    fn succ(&self, _: Uint256, _: Option<Uint256>) -> Result<Option<Uint256>, ViewError> {
        Ok(None)
    }

    fn read(&self, keylet: protocol::Keylet) -> Result<Option<Arc<STLedgerEntry>>, ViewError> {
        if self.fail_reads {
            return Err(ViewError::Conversion("test read failure".to_owned()));
        }
        Ok(self.entries.get(&keylet.key).cloned())
    }

    fn sles(&self) -> Result<Vec<Arc<STLedgerEntry>>, ViewError> {
        Ok(self.entries.values().cloned().collect())
    }

    fn tx_exists(&self, _: Uint256) -> Result<bool, ViewError> {
        Ok(false)
    }

    fn tx_read(&self, _: Uint256) -> Result<Option<ReadViewTx>, ViewError> {
        Ok(None)
    }

    fn txs(&self) -> Result<Vec<ReadViewTx>, ViewError> {
        Ok(Vec::new())
    }
}

fn sf(name: &str) -> &'static protocol::SField {
    get_field_by_symbol(name)
}

fn account(fill: u8) -> AccountID {
    AccountID::from_array([fill; 20])
}

fn account_root(account: AccountID) -> STLedgerEntry {
    let mut entry = STLedgerEntry::from_type_and_key(
        LedgerEntryType::AccountRoot,
        protocol::account_keylet(Uint160::from_void(account.data())).key,
    );
    entry.set_account_id(sf("sfAccount"), account);
    entry
}

fn ledger_state_fix_tx(fix_type: u16, owner: AccountID, directory: Uint256) -> STTx {
    STTx::new(TxType::LEDGER_STATE_FIX, |tx| {
        tx.set_field_u16(sf("sfLedgerFixType"), fix_type);
        tx.set_account_id(sf("sfOwner"), owner);
        tx.set_field_h256(sf("sfBookDirectory"), directory);
    })
}

fn directory(key: Uint256, exchange_rate: Option<u64>) -> STLedgerEntry {
    let mut entry = STLedgerEntry::from_type_and_key(LedgerEntryType::DirectoryNode, key);
    if let Some(exchange_rate) = exchange_rate {
        entry.set_field_u64(sf("sfExchangeRate"), exchange_rate);
    }
    entry
}

#[test]
fn ledger_state_fix_read_view_preclaim_preserves_nft_and_book_ter_ordering() {
    let owner = account(1);
    let directory_key = Uint256::from_array([7; 32]);
    let nft = ledger_state_fix_tx(1, owner, directory_key);
    let book = ledger_state_fix_tx(2, owner, directory_key);
    let mut view = TestReadView::default();

    assert_eq!(
        run_ledger_state_fix_read_view_preclaim(&view, &nft, TxType::LEDGER_STATE_FIX),
        Some(Ter::TEC_OBJECT_NOT_FOUND)
    );
    view.insert(account_root(owner));
    assert_eq!(
        run_ledger_state_fix_read_view_preclaim(&view, &nft, TxType::LEDGER_STATE_FIX),
        Some(Ter::TES_SUCCESS)
    );
    assert_eq!(
        run_ledger_state_fix_read_view_preclaim(&view, &book, TxType::LEDGER_STATE_FIX),
        Some(Ter::TEC_OBJECT_NOT_FOUND),
        "a missing directory wins before ExchangeRate checks"
    );

    view.insert(directory(directory_key, None));
    assert_eq!(
        run_ledger_state_fix_read_view_preclaim(&view, &book, TxType::LEDGER_STATE_FIX),
        Some(Ter::TEC_NO_PERMISSION),
        "a directory without ExchangeRate is not eligible for repair"
    );

    view.insert(directory(
        directory_key,
        Some(quality_from_key(directory_key)),
    ));
    assert_eq!(
        run_ledger_state_fix_read_view_preclaim(&view, &book, TxType::LEDGER_STATE_FIX),
        Some(Ter::TEC_NO_PERMISSION),
        "an already-correct ExchangeRate is not eligible for repair"
    );

    view.insert(directory(directory_key, Some(1)));
    assert_eq!(
        run_ledger_state_fix_read_view_preclaim(&view, &book, TxType::LEDGER_STATE_FIX),
        Some(Ter::TES_SUCCESS)
    );
    assert_eq!(
        view.entries.len(),
        2,
        "preclaim must not mutate the ReadView"
    );
}

#[test]
fn ledger_state_fix_read_view_preclaim_has_no_default_success_or_read_error_fallback() {
    let owner = account(2);
    let tx = ledger_state_fix_tx(1, owner, Uint256::default());
    let error_view = TestReadView {
        fail_reads: true,
        ..Default::default()
    };

    assert_eq!(
        run_ledger_state_fix_read_view_preclaim(&error_view, &tx, TxType::LEDGER_STATE_FIX),
        Some(Ter::TEF_BAD_LEDGER)
    );
    assert_eq!(
        run_ledger_state_fix_read_view_preclaim(
            &TestReadView::default(),
            &STTx::new(TxType::PAYMENT, |_| {}),
            TxType::PAYMENT,
        ),
        None,
        "unowned transaction types must never receive a permissive success"
    );
}

#[test]
fn ledger_state_fix_preflight_rejects_unknown_fix_type_codes() {
    let zero = run_ledger_state_fix_preflight(LedgerStateFixType::from(0), true);
    let two_hundred = run_ledger_state_fix_preflight(LedgerStateFixType::from(200), true);

    assert_eq!(zero, Ter::TEF_INVALID_LEDGER_FIX_TYPE);
    assert_eq!(two_hundred, Ter::TEF_INVALID_LEDGER_FIX_TYPE);
    assert_eq!(trans_token(zero), "tefINVALID_LEDGER_FIX_TYPE");
}

#[test]
fn ledger_state_fix_preclaim_requires_owner_account() {
    let missing = run_ledger_state_fix_preclaim(LedgerStateFixType::NfTokenPageLink, false);
    let present = run_ledger_state_fix_preclaim(LedgerStateFixType::NfTokenPageLink, true);

    assert_eq!(missing, Ter::TEC_OBJECT_NOT_FOUND);
    assert_eq!(present, Ter::TES_SUCCESS);
}

#[test]
fn ledger_state_fix_do_apply_maps_repair_result() {
    let called = Cell::new(false);
    let success = run_ledger_state_fix_do_apply(LedgerStateFixType::NfTokenPageLink, || {
        called.set(true);
        true
    });
    let failure = run_ledger_state_fix_do_apply(LedgerStateFixType::NfTokenPageLink, || false);

    assert!(called.get());
    assert_eq!(success, Ter::TES_SUCCESS);
    assert_eq!(failure, Ter::TEC_FAILED_PROCESSING);
    assert_eq!(trans_token(failure), "tecFAILED_PROCESSING");
}
