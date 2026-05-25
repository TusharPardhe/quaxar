//! Single-candidate `TxQ::erase(FeeMultiSet iterator)` helper.
//!
//! This ports the deterministic owner-visible postconditions of removing one
//! fee-ordered candidate: erase it from the fee-order view first, then remove
//! the matching entry from the account-local map, and return the next
//! fee-ordered candidate.

use std::collections::BTreeMap;

use protocol::SeqProxy;

use crate::TxQAccount;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct FeeQueueKey<Account> {
    pub account: Account,
    pub seq_proxy: SeqProxy,
}

impl<Account> FeeQueueKey<Account> {
    pub const fn new(account: Account, seq_proxy: SeqProxy) -> Self {
        Self { account, seq_proxy }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EraseCandidateResult<Account> {
    pub removed: FeeQueueKey<Account>,
    pub next_fee_candidate: Option<FeeQueueKey<Account>>,
}

pub fn erase_fee_candidate<Account, T>(
    accounts: &mut BTreeMap<Account, TxQAccount<Account, T>>,
    fee_order: &mut Vec<FeeQueueKey<Account>>,
    candidate_index: usize,
) -> EraseCandidateResult<Account>
where
    Account: Ord + Clone,
{
    let removed = fee_order
        .get(candidate_index)
        .cloned()
        .expect("xrpl::TxQ::erase : candidate found in byFee");
    let txq_account = accounts
        .get_mut(&removed.account)
        .expect("xrpl::TxQ::erase : account found");

    let removed = fee_order.remove(candidate_index);
    let next_fee_candidate = fee_order.get(candidate_index).cloned();

    let found = txq_account.remove(removed.seq_proxy);
    assert!(found, "xrpl::TxQ::erase : account removed");

    EraseCandidateResult {
        removed,
        next_fee_candidate,
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use protocol::SeqProxy;

    use super::{EraseCandidateResult, FeeQueueKey, erase_fee_candidate};
    use crate::{MaybeTxCore, TxConsequences, TxQAccount};

    #[test]
    fn erase_fee_candidate_removes_from_fee_order_and_account_and_returns_next() {
        let mut first = TxQAccount::new("a");
        first.add(
            SeqProxy::sequence(5),
            MaybeTxCore::new("a5", TxConsequences::new(1, SeqProxy::sequence(5))),
        );

        let mut second = TxQAccount::new("b");
        second.add(
            SeqProxy::sequence(7),
            MaybeTxCore::new("b7", TxConsequences::new(1, SeqProxy::sequence(7))),
        );

        let mut accounts = BTreeMap::from([("a", first), ("b", second)]);
        let mut fee_order = vec![
            FeeQueueKey::new("a", SeqProxy::sequence(5)),
            FeeQueueKey::new("b", SeqProxy::sequence(7)),
        ];

        let result = erase_fee_candidate(&mut accounts, &mut fee_order, 0);

        assert_eq!(
            result,
            EraseCandidateResult {
                removed: FeeQueueKey::new("a", SeqProxy::sequence(5)),
                next_fee_candidate: Some(FeeQueueKey::new("b", SeqProxy::sequence(7))),
            }
        );
        assert_eq!(
            fee_order,
            vec![FeeQueueKey::new("b", SeqProxy::sequence(7))]
        );
        assert!(accounts.contains_key("a"));
        assert!(accounts.get("a").is_some_and(TxQAccount::empty));
    }

    #[test]
    fn erase_fee_candidate_returns_none_when_last_fee_entry_is_removed() {
        let mut account = TxQAccount::new("a");
        account.add(
            SeqProxy::ticket(9),
            MaybeTxCore::new("a9", TxConsequences::new(1, SeqProxy::ticket(9))),
        );

        let mut accounts = BTreeMap::from([("a", account)]);
        let mut fee_order = vec![FeeQueueKey::new("a", SeqProxy::ticket(9))];

        let result = erase_fee_candidate(&mut accounts, &mut fee_order, 0);

        assert_eq!(
            result,
            EraseCandidateResult {
                removed: FeeQueueKey::new("a", SeqProxy::ticket(9)),
                next_fee_candidate: None,
            }
        );
        assert!(fee_order.is_empty());
        assert!(accounts.get("a").is_some_and(TxQAccount::empty));
    }

    #[test]
    #[should_panic(expected = "xrpl::TxQ::erase : account removed")]
    fn erase_fee_candidate_requires_account_entry_to_exist_for_removed_seq() {
        let accounts = BTreeMap::from([("a", TxQAccount::<&str, &str>::new("a"))]);
        let mut accounts = accounts;
        let mut fee_order = vec![FeeQueueKey::new("a", SeqProxy::sequence(5))];

        let _ = erase_fee_candidate(&mut accounts, &mut fee_order, 0);
    }
}
