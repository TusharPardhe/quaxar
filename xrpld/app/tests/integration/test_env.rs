//! TestEnv — full integration test environment mirroring C++ `test::jtx::Env`.

use std::sync::Arc;

use basics::base_uint::{Uint160, Uint256};
use ledger::{ApplyView, ApplyViewImpl, Fees, Ledger, LedgerHeader, ReadView};
use protocol::{
    AccountID, ApplyFlags, Asset, Currency, IOUAmount, Issue, LedgerEntryType, Rules, STAmount,
    STLedgerEntry, STTx, Ter, TxType, XRPAmount, account_keylet, get_field_by_symbol,
    owner_dir_keylet, xrp_issue,
};
use shamap::item::SHAMapItem;
use shamap::mutation::MutableTree;
use shamap::sync::{SHAMapType, SyncState, SyncTree};
use shamap::tree_node::SHAMapNodeType;

use super::test_account::TestAccount;
use app::state::transactor_dispatcher::handle_real_dispatch;

/// Full integration test environment — equivalent to C++ `Env`.
pub struct TestEnv {
    ledger: Ledger,
    view: ApplyViewImpl<Ledger>,
    accounts: Vec<TestAccount>,
    features: Vec<Uint256>,
    close_time: u32,
}

impl TestEnv {
    /// Create a new test environment with default features.
    pub fn new() -> Self {
        Self::with_features(vec![])
    }

    /// Create a new test environment with specific amendment features enabled.
    pub fn with_features(features: Vec<Uint256>) -> Self {
        let header = LedgerHeader {
            seq: 1,
            drops: 100_000_000_000_000_000,
            close_time: 0,
            parent_close_time: 0,
            ..LedgerHeader::default()
        };
        let tree = MutableTree::new(1);
        let state_map = SyncTree::from_root_with_type(
            tree.root(),
            SHAMapType::State,
            false,
            1,
            SyncState::Immutable,
        );
        let mut ledger = Ledger::from_maps(
            header,
            state_map,
            SyncTree::new_with_type(SHAMapType::Transaction, false, 1),
        );
        ledger.set_fees(Fees {
            base: 10,
            reserve: 200_000,
            increment: 50_000,
        });
        if !features.is_empty() {
            ledger.set_rules(Rules::new(features.iter().copied()));
        }
        let view = ApplyViewImpl::new(Arc::new(ledger.clone()), ApplyFlags::NONE);
        Self {
            ledger,
            view,
            accounts: Vec::new(),
            features,
            close_time: 0,
        }
    }

    /// Get or create a named account.
    pub fn account(&mut self, name: &str) -> TestAccount {
        if let Some(acct) = self.accounts.iter().find(|a| a.name == name) {
            return acct.clone();
        }
        let acct = TestAccount::new(name);
        self.accounts.push(acct.clone());
        acct
    }

    /// Fund an account with XRP (creates the account if it doesn't exist).
    pub fn fund(&mut self, amount: XRPAmount, account: &TestAccount) {
        let keylet = account_keylet(account.id_160());
        let mut entry = STLedgerEntry::from_type_and_key(LedgerEntryType::AccountRoot, keylet.key);
        entry.set_account_id(get_field_by_symbol("sfAccount"), account.id);
        entry.set_field_u32(get_field_by_symbol("sfSequence"), 1);
        entry.set_field_amount(
            get_field_by_symbol("sfBalance"),
            STAmount::from_xrp_amount(amount),
        );
        entry.set_field_u32(get_field_by_symbol("sfOwnerCount"), 0);
        entry.set_field_u32(get_field_by_symbol("sfFlags"), 0);
        let _ = self.view.insert(Arc::new(entry));
    }

    /// Fund an account with XRP and set DefaultRipple flag.
    pub fn fund_with_default_ripple(&mut self, amount: XRPAmount, account: &TestAccount) {
        let keylet = account_keylet(account.id_160());
        let mut entry = STLedgerEntry::from_type_and_key(LedgerEntryType::AccountRoot, keylet.key);
        entry.set_account_id(get_field_by_symbol("sfAccount"), account.id);
        entry.set_field_u32(get_field_by_symbol("sfSequence"), 1);
        entry.set_field_amount(
            get_field_by_symbol("sfBalance"),
            STAmount::from_xrp_amount(amount),
        );
        entry.set_field_u32(get_field_by_symbol("sfOwnerCount"), 0);
        entry.set_field_u32(get_field_by_symbol("sfFlags"), protocol::lsfDefaultRipple);
        let _ = self.view.insert(Arc::new(entry));
    }

    /// Submit a transaction and assert the expected result.
    pub fn apply(&mut self, tx: STTx, expected: Ter) -> Ter {
        let tx_type = tx.get_txn_type();
        let result = handle_real_dispatch(&mut self.view, &tx, tx_type, None);
        assert_eq!(
            result, expected,
            "Transaction {:?} expected {:?} but got {:?}",
            tx_type, expected, result
        );
        result
    }

    /// Submit a transaction and return the result without asserting.
    pub fn try_apply(&mut self, tx: STTx) -> Ter {
        let tx_type = tx.get_txn_type();
        handle_real_dispatch(&mut self.view, &tx, tx_type, None)
    }

    /// Close the current ledger (advance time and sequence).
    pub fn close(&mut self) {
        self.close_time += 10;
        // In a full implementation, this would rebuild the ledger from the view's
        // state table. For now, the view persists state across closes.
    }

    /// Get the XRP balance of an account.
    pub fn balance(&self, account: &TestAccount) -> i64 {
        let keylet = account_keylet(account.id_160());
        self.view
            .read(keylet)
            .ok()
            .flatten()
            .map(|sle| sle.get_field_amount(get_field_by_symbol("sfBalance")).xrp().drops())
            .unwrap_or(0)
    }

    /// Get the owner count of an account.
    pub fn owner_count(&self, account: &TestAccount) -> u32 {
        let keylet = account_keylet(account.id_160());
        self.view
            .read(keylet)
            .ok()
            .flatten()
            .map(|sle| sle.get_field_u32(get_field_by_symbol("sfOwnerCount")))
            .unwrap_or(0)
    }

    /// Get the sequence number of an account.
    pub fn seq(&self, account: &TestAccount) -> u32 {
        let keylet = account_keylet(account.id_160());
        self.view
            .read(keylet)
            .ok()
            .flatten()
            .map(|sle| sle.get_field_u32(get_field_by_symbol("sfSequence")))
            .unwrap_or(0)
    }

    /// Check if a ledger entry exists.
    pub fn exists(&self, keylet: protocol::Keylet) -> bool {
        self.view.exists(keylet).unwrap_or(false)
    }

    /// Read a ledger entry.
    pub fn read(&self, keylet: protocol::Keylet) -> Option<Arc<STLedgerEntry>> {
        self.view.read(keylet).ok().flatten()
    }

    /// Get the current ledger fees.
    pub fn fees(&self) -> Fees {
        self.view.fees()
    }

    /// Get the current rules.
    pub fn rules(&self) -> Rules {
        self.view.rules()
    }

    /// Convert the current view state into a standalone Ledger for RPC queries.
    pub fn to_ledger(&self) -> Ledger {
        let mut ledger = self.ledger.clone();
        self.view
            .table()
            .apply(&mut ledger)
            .expect("state table should apply to ledger");
        ledger
    }
}

/// Helper: create XRP amount from drops.
pub fn xrp(drops: i64) -> XRPAmount {
    XRPAmount::from_drops(drops * 1_000_000)
}

/// Helper: create XRP drops amount.
pub fn drops(amount: i64) -> XRPAmount {
    XRPAmount::from_drops(amount)
}
