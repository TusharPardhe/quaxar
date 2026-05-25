//! Deterministic `TxQAccount` sequencing helper in `xrpld/app/misc/TxQ`.
//!
//! This helper ports the per-account ordered queue shape that is driven by
//! `SeqProxy` and `TxConsequences`.

use std::collections::BTreeMap;

use protocol::SeqProxy;

use crate::TxConsequences;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MaybeTxCore<T> {
    pub payload: T,
    pub consequences: TxConsequences,
}

impl<T> MaybeTxCore<T> {
    pub const fn new(payload: T, consequences: TxConsequences) -> Self {
        Self {
            payload,
            consequences,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TxQAccount<Account, T> {
    pub account: Account,
    pub transactions: BTreeMap<SeqProxy, MaybeTxCore<T>>,
    pub retry_penalty: bool,
    pub drop_penalty: bool,
}

impl<Account, T> TxQAccount<Account, T> {
    pub fn new(account: Account) -> Self {
        Self {
            account,
            transactions: BTreeMap::new(),
            retry_penalty: false,
            drop_penalty: false,
        }
    }

    pub fn get_txn_count(&self) -> usize {
        self.transactions.len()
    }

    pub fn empty(&self) -> bool {
        self.transactions.is_empty()
    }

    /// Mirrors the the reference implementation helper exactly:
    /// - if a strictly previous entry exists, return it;
    /// - otherwise return the first same-or-greater entry, if any.
    pub fn get_prev_tx(&self, seq_proxy: SeqProxy) -> Option<(&SeqProxy, &MaybeTxCore<T>)> {
        self.transactions
            .range(..seq_proxy)
            .next_back()
            .or_else(|| self.transactions.range(seq_proxy..).next())
    }

    pub fn add(&mut self, seq_proxy: SeqProxy, txn: MaybeTxCore<T>) -> &mut MaybeTxCore<T> {
        let inserted = self.transactions.insert(seq_proxy, txn);
        assert!(
            inserted.is_none(),
            "xrpl::TxQ::TxQAccount::add : emplace succeeded"
        );

        self.transactions
            .get_mut(&seq_proxy)
            .expect("inserted transaction must exist")
    }

    pub fn remove(&mut self, seq_proxy: SeqProxy) -> bool {
        self.transactions.remove(&seq_proxy).is_some()
    }

    /// Returns the first sequence hole for a new sequence-based transaction.
    ///
    /// This mirrors the current `nextQueuableSeqImpl(...)` behavior over the
    /// already-known account queue.
    pub fn next_queuable_seq(&self, account_seq_proxy: SeqProxy) -> SeqProxy {
        let mut queued = self.transactions.range(account_seq_proxy..);

        let Some((seq_proxy, txn)) = queued.next() else {
            return account_seq_proxy;
        };

        if !seq_proxy.is_seq() || *seq_proxy != account_seq_proxy {
            return account_seq_proxy;
        }

        let mut attempt = txn.consequences.following_seq();
        for (queued_seq, queued_txn) in queued {
            if attempt < *queued_seq {
                break;
            }

            attempt = queued_txn.consequences.following_seq();
        }
        attempt
    }
}

#[cfg(test)]
mod tests {
    use super::{MaybeTxCore, TxQAccount};
    use crate::TxConsequences;
    use protocol::SeqProxy;

    #[test]
    fn get_prev_tx_matches_current_cpp_iterator_shape() {
        let mut account = TxQAccount::new("acct");
        account.add(
            SeqProxy::sequence(5),
            MaybeTxCore::new("a", TxConsequences::new(1, SeqProxy::sequence(5))),
        );
        account.add(
            SeqProxy::sequence(7),
            MaybeTxCore::new("b", TxConsequences::new(1, SeqProxy::sequence(7))),
        );
        account.add(
            SeqProxy::ticket(3),
            MaybeTxCore::new("c", TxConsequences::new(1, SeqProxy::ticket(3))),
        );

        assert_eq!(
            account.get_prev_tx(SeqProxy::sequence(4)).map(|(k, _)| *k),
            Some(SeqProxy::sequence(5))
        );
        assert_eq!(
            account.get_prev_tx(SeqProxy::sequence(5)).map(|(k, _)| *k),
            Some(SeqProxy::sequence(5))
        );
        assert_eq!(
            account.get_prev_tx(SeqProxy::sequence(6)).map(|(k, _)| *k),
            Some(SeqProxy::sequence(5))
        );
        assert_eq!(
            account.get_prev_tx(SeqProxy::sequence(7)).map(|(k, _)| *k),
            Some(SeqProxy::sequence(5))
        );
        assert_eq!(
            account.get_prev_tx(SeqProxy::ticket(9)).map(|(k, _)| *k),
            Some(SeqProxy::ticket(3))
        );
    }

    #[test]
    fn add_remove_and_count_match_current_cpp_roles() {
        let mut account = TxQAccount::new("acct");
        assert!(account.empty());

        let inserted = account.add(
            SeqProxy::sequence(9),
            MaybeTxCore::new("payload", TxConsequences::new(3, SeqProxy::sequence(9))),
        );
        inserted.payload = "updated";

        assert_eq!(account.get_txn_count(), 1);
        assert!(!account.empty());
        assert!(account.remove(SeqProxy::sequence(9)));
        assert!(!account.remove(SeqProxy::sequence(9)));
        assert!(account.empty());
    }

    #[test]
    fn next_queuable_seq_matches_current_cpp_gap_rules() {
        let mut account = TxQAccount::new("acct");
        account.add(
            SeqProxy::sequence(5),
            MaybeTxCore::new(
                "s5",
                TxConsequences::with_sequences_consumed(1, SeqProxy::sequence(5), 1),
            ),
        );
        account.add(
            SeqProxy::sequence(6),
            MaybeTxCore::new(
                "s6",
                TxConsequences::with_sequences_consumed(1, SeqProxy::sequence(6), 1),
            ),
        );
        account.add(
            SeqProxy::sequence(8),
            MaybeTxCore::new(
                "s8",
                TxConsequences::with_sequences_consumed(1, SeqProxy::sequence(8), 1),
            ),
        );

        assert_eq!(
            account.next_queuable_seq(SeqProxy::sequence(5)),
            SeqProxy::sequence(7)
        );
        assert_eq!(
            account.next_queuable_seq(SeqProxy::sequence(4)),
            SeqProxy::sequence(4)
        );
        assert_eq!(
            account.next_queuable_seq(SeqProxy::sequence(9)),
            SeqProxy::sequence(9)
        );
    }

    #[test]
    fn next_queuable_seq_respects_multi_sequence_consumers_and_ticket_fronts() {
        let mut account = TxQAccount::new("acct");
        account.add(
            SeqProxy::ticket(2),
            MaybeTxCore::new("ticket", TxConsequences::new(1, SeqProxy::ticket(2))),
        );
        account.add(
            SeqProxy::sequence(5),
            MaybeTxCore::new(
                "batch",
                TxConsequences::with_sequences_consumed(1, SeqProxy::sequence(5), 3),
            ),
        );

        assert_eq!(
            account.next_queuable_seq(SeqProxy::sequence(4)),
            SeqProxy::sequence(4)
        );
        assert_eq!(
            account.next_queuable_seq(SeqProxy::sequence(5)),
            SeqProxy::sequence(8)
        );
    }
}
