//! `xrpld/app/ledger/LocalTxs.*` compatibility surface.

use std::sync::{Arc, Mutex};

use basics::base_uint::{Uint160, Uint256};
use protocol::{
    AccountID, STTx, SeqProxy, account_keylet, get_field_by_symbol, ticket_keylet_from_seq_proxy,
};
use shamap::traversal::TraversalError;

use crate::{CanonicalTXSet, Ledger};

#[derive(Debug, Clone)]
struct LocalTx {
    txn: Arc<STTx>,
    expire: u32,
    id: Uint256,
    account: AccountID,
    seq_proxy: SeqProxy,
}

impl LocalTx {
    fn new(index: u32, txn: Arc<STTx>) -> Self {
        let seq_proxy = txn.get_seq_proxy();
        let mut expire = index.wrapping_add(LocalTxs::HOLD_LEDGERS);
        let last_ledger_sequence = get_field_by_symbol("sfLastLedgerSequence");
        if txn.is_field_present(last_ledger_sequence) {
            expire = expire.min(txn.get_field_u32(last_ledger_sequence).wrapping_add(1));
        }

        Self {
            id: txn.get_transaction_id(),
            account: txn.get_account_id(get_field_by_symbol("sfAccount")),
            txn,
            expire,
            seq_proxy,
        }
    }

    fn is_expired(&self, ledger_index: u32) -> bool {
        ledger_index > self.expire
    }
}

fn raw_account_id(account: AccountID) -> Uint160 {
    Uint160::from_slice(account.data()).expect("AccountID width should match Uint160")
}

#[derive(Debug, Default)]
pub struct LocalTxs {
    txns: Mutex<Vec<LocalTx>>,
}

impl LocalTxs {
    pub const HOLD_LEDGERS: u32 = 5;

    pub fn new() -> Self {
        Self::default()
    }

    pub fn push_back(&self, index: u32, txn: Arc<STTx>) {
        self.txns
            .lock()
            .expect("local txs mutex must not be poisoned")
            .push(LocalTx::new(index, txn));
    }

    pub fn get_tx_set(&self) -> CanonicalTXSet {
        let txns = self
            .txns
            .lock()
            .expect("local txs mutex must not be poisoned");
        let mut set = CanonicalTXSet::new(Uint256::zero());
        for tx in txns.iter() {
            set.insert(Arc::clone(&tx.txn));
        }
        set
    }

    pub fn sweep(&self, view: &Ledger) -> Result<(), TraversalError> {
        let ledger_index = view.header().seq;
        let mut txns = self
            .txns
            .lock()
            .expect("local txs mutex must not be poisoned");
        let mut kept = Vec::with_capacity(txns.len());

        for txn in txns.drain(..) {
            if should_keep_tx(&txn, ledger_index, view)? {
                kept.push(txn);
            }
        }

        *txns = kept;
        Ok(())
    }

    pub fn size(&self) -> usize {
        self.txns
            .lock()
            .expect("local txs mutex must not be poisoned")
            .len()
    }
}

fn should_keep_tx(txn: &LocalTx, ledger_index: u32, view: &Ledger) -> Result<bool, TraversalError> {
    if txn.is_expired(ledger_index) {
        return Ok(false);
    }

    if view.tx_exists(txn.id) {
        return Ok(false);
    }

    let Some(account_root) = view.read(account_keylet(raw_account_id(txn.account)))? else {
        return Ok(true);
    };

    let account_seq =
        SeqProxy::sequence(account_root.get_field_u32(get_field_by_symbol("sfSequence")));
    if txn.seq_proxy.is_seq() {
        return Ok(account_seq <= txn.seq_proxy);
    }

    if account_seq.value() <= txn.seq_proxy.value() {
        return Ok(true);
    }

    view.exists_keylet(ticket_keylet_from_seq_proxy(
        raw_account_id(txn.account),
        txn.seq_proxy,
    ))
}
