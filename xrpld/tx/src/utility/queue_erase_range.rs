//! Range-erase helper behind `TxQ::erase(txQAccount, begin, end)`.
//!
//! This ports the deterministic account-ordered removal semantics while
//! preserving the reference helper order.

use protocol::SeqProxy;

use crate::TxQAccount;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EraseRangeResult {
    pub removed_seq_proxies: Vec<SeqProxy>,
    pub next_seq_proxy: Option<SeqProxy>,
}

pub fn erase_account_range<Account, T>(
    account: &mut TxQAccount<Account, T>,
    begin: SeqProxy,
    end_exclusive: Option<SeqProxy>,
) -> EraseRangeResult {
    let removed_seq_proxies = account
        .transactions
        .range(begin..)
        .take_while(|(seq_proxy, _)| end_exclusive.is_none_or(|end| **seq_proxy < end))
        .map(|(seq_proxy, _)| *seq_proxy)
        .collect::<Vec<_>>();

    for seq_proxy in &removed_seq_proxies {
        let removed = account.transactions.remove(seq_proxy);
        assert!(
            removed.is_some(),
            "xrpl::TxQ::erase : account range entry removed"
        );
    }

    let next_seq_proxy = account
        .transactions
        .range(begin..)
        .next()
        .map(|(seq, _)| *seq);

    EraseRangeResult {
        removed_seq_proxies,
        next_seq_proxy,
    }
}

#[cfg(test)]
mod tests {
    use protocol::SeqProxy;

    use super::{EraseRangeResult, erase_account_range};
    use crate::{MaybeTxCore, TxConsequences, TxQAccount};

    #[test]
    fn erase_account_range_removes_half_open_range_and_returns_next_key() {
        let mut account = TxQAccount::new("acct");
        account.add(
            SeqProxy::sequence(5),
            MaybeTxCore::new("s5", TxConsequences::new(1, SeqProxy::sequence(5))),
        );
        account.add(
            SeqProxy::sequence(6),
            MaybeTxCore::new("s6", TxConsequences::new(1, SeqProxy::sequence(6))),
        );
        account.add(
            SeqProxy::sequence(8),
            MaybeTxCore::new("s8", TxConsequences::new(1, SeqProxy::sequence(8))),
        );

        let result = erase_account_range(
            &mut account,
            SeqProxy::sequence(5),
            Some(SeqProxy::sequence(8)),
        );

        assert_eq!(
            result,
            EraseRangeResult {
                removed_seq_proxies: vec![SeqProxy::sequence(5), SeqProxy::sequence(6)],
                next_seq_proxy: Some(SeqProxy::sequence(8)),
            }
        );
        assert_eq!(account.get_txn_count(), 1);
        assert!(account.transactions.contains_key(&SeqProxy::sequence(8)));
    }

    #[test]
    fn erase_account_range_supports_empty_ranges() {
        let mut account = TxQAccount::new("acct");
        account.add(
            SeqProxy::sequence(5),
            MaybeTxCore::new("s5", TxConsequences::new(1, SeqProxy::sequence(5))),
        );
        account.add(
            SeqProxy::sequence(8),
            MaybeTxCore::new("s8", TxConsequences::new(1, SeqProxy::sequence(8))),
        );

        let result = erase_account_range(
            &mut account,
            SeqProxy::sequence(8),
            Some(SeqProxy::sequence(8)),
        );

        assert_eq!(
            result,
            EraseRangeResult {
                removed_seq_proxies: Vec::new(),
                next_seq_proxy: Some(SeqProxy::sequence(8)),
            }
        );
        assert_eq!(account.get_txn_count(), 2);
    }

    #[test]
    fn erase_account_range_can_remove_to_the_end() {
        let mut account = TxQAccount::new("acct");
        account.add(
            SeqProxy::ticket(2),
            MaybeTxCore::new("t2", TxConsequences::new(1, SeqProxy::ticket(2))),
        );
        account.add(
            SeqProxy::sequence(5),
            MaybeTxCore::new("s5", TxConsequences::new(1, SeqProxy::sequence(5))),
        );

        let result = erase_account_range(&mut account, SeqProxy::sequence(5), None);

        assert_eq!(
            result,
            EraseRangeResult {
                removed_seq_proxies: vec![SeqProxy::sequence(5), SeqProxy::ticket(2)],
                next_seq_proxy: None,
            }
        );
        assert!(account.empty());
    }
}
