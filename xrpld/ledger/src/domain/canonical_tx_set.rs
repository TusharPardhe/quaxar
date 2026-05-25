//! `xrpl/ledger/CanonicalTXSet.*` compatibility surface.

use std::{collections::BTreeMap, sync::Arc};

use basics::base_uint::Uint256;
use protocol::{AccountID, LedgerHash, STTx, SeqProxy};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct CanonicalTxSetKey {
    account: Uint256,
    seq_proxy: SeqProxy,
    tx_id: Uint256,
}

#[derive(Debug, Clone, Default)]
pub struct CanonicalTXSet {
    map: BTreeMap<CanonicalTxSetKey, Arc<STTx>>,
    salt: LedgerHash,
}

impl CanonicalTXSet {
    pub fn new(salt: LedgerHash) -> Self {
        Self {
            map: BTreeMap::new(),
            salt,
        }
    }

    pub fn insert(&mut self, txn: Arc<STTx>) {
        let key = CanonicalTxSetKey {
            account: self
                .account_key(txn.get_account_id(protocol::get_field_by_symbol("sfAccount"))),
            seq_proxy: txn.get_seq_proxy(),
            tx_id: txn.get_transaction_id(),
        };
        self.map.entry(key).or_insert(txn);
    }

    pub fn pop_acct_transaction(&mut self, tx: &Arc<STTx>) -> Option<Arc<STTx>> {
        let effective_account =
            self.account_key(tx.get_account_id(protocol::get_field_by_symbol("sfAccount")));
        let seq_proxy = tx.get_seq_proxy();
        let after = CanonicalTxSetKey {
            account: effective_account,
            seq_proxy,
            tx_id: Uint256::zero(),
        };

        let next = self
            .map
            .range(after..)
            .next()
            .map(|(key, value)| (*key, Arc::clone(value)));
        let (key, candidate) = next?;

        if key.account != effective_account {
            return None;
        }

        let candidate_seq = candidate.get_seq_proxy();
        if candidate_seq.is_seq() && candidate_seq.value() != seq_proxy.value().wrapping_add(1) {
            return None;
        }

        self.map.remove(&key)
    }

    pub fn reset(&mut self, salt: LedgerHash) {
        self.salt = salt;
        self.map.clear();
    }

    pub fn iter(&self) -> impl Iterator<Item = &Arc<STTx>> {
        self.map.values()
    }

    pub fn drain_ordered(&mut self) -> Vec<Arc<STTx>> {
        std::mem::take(&mut self.map).into_values().collect()
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    pub fn key(&self) -> LedgerHash {
        self.salt
    }

    fn account_key(&self, account: AccountID) -> Uint256 {
        let mut raw = [0u8; 32];
        raw[..AccountID::size()].copy_from_slice(account.data());
        Uint256::from_array(raw) ^ self.salt
    }
}
