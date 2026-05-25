use std::sync::Arc;

use basics::base_uint::Uint256;
use ledger::CanonicalTXSet;
use protocol::{AccountID, STAmount, STTx, SeqProxy, TxType, get_field_by_symbol};

fn account(hex: &str) -> AccountID {
    AccountID::from_hex(hex).expect("account hex should parse")
}

fn payment_tx(
    source: AccountID,
    destination: AccountID,
    sequence: u32,
    ticket_sequence: Option<u32>,
    fee_drops: u64,
) -> Arc<STTx> {
    Arc::new(STTx::new(TxType::PAYMENT, |tx| {
        tx.set_account_id(get_field_by_symbol("sfAccount"), source);
        tx.set_account_id(get_field_by_symbol("sfDestination"), destination);
        tx.set_field_amount(
            get_field_by_symbol("sfAmount"),
            STAmount::new_native(1_000_000, false),
        );
        tx.set_field_amount(
            get_field_by_symbol("sfFee"),
            STAmount::new_native(fee_drops, false),
        );
        tx.set_field_u32(get_field_by_symbol("sfSequence"), sequence);
        if let Some(ticket_sequence) = ticket_sequence {
            tx.set_field_u32(get_field_by_symbol("sfTicketSequence"), ticket_sequence);
        }
    }))
}

fn tx_ids(set: &CanonicalTXSet) -> Vec<Uint256> {
    set.iter().map(|tx| tx.get_transaction_id()).collect()
}

#[test]
fn ledger_canonical_tx_set_orders_sequences_before_tickets() {
    let source = account("1111111111111111111111111111111111111111");
    let destination = account("AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA");
    let seq = payment_tx(source, destination, 2, None, 10);
    let ticket = payment_tx(source, destination, 0, Some(1), 11);

    let mut set = CanonicalTXSet::new(Uint256::zero());
    set.insert(Arc::clone(&ticket));
    set.insert(Arc::clone(&seq));

    let ordered: Vec<SeqProxy> = set.iter().map(|tx| tx.get_seq_proxy()).collect();
    assert_eq!(ordered, vec![SeqProxy::sequence(2), SeqProxy::ticket(1)]);
}

#[test]
fn ledger_canonical_tx_set_orders_same_account_sequences() {
    let source = account("1111111111111111111111111111111111111111");
    let destination = account("AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA");

    let first = payment_tx(source, destination, 1, None, 10);
    let second = payment_tx(source, destination, 2, None, 11);
    let third = payment_tx(source, destination, 3, None, 12);

    let mut set = CanonicalTXSet::new(Uint256::zero());
    set.insert(Arc::clone(&third));
    set.insert(Arc::clone(&first));
    set.insert(Arc::clone(&second));

    let ordered: Vec<SeqProxy> = set.iter().map(|tx| tx.get_seq_proxy()).collect();
    assert_eq!(
        ordered,
        vec![
            SeqProxy::sequence(1),
            SeqProxy::sequence(2),
            SeqProxy::sequence(3)
        ]
    );
}

#[test]
fn ledger_canonical_tx_set_orders_same_account_tickets() {
    let source = account("1111111111111111111111111111111111111111");
    let destination = account("AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA");

    let first = payment_tx(source, destination, 0, Some(9), 10);
    let second = payment_tx(source, destination, 0, Some(2), 11);
    let third = payment_tx(source, destination, 0, Some(5), 12);

    let mut set = CanonicalTXSet::new(Uint256::zero());
    set.insert(Arc::clone(&first));
    set.insert(Arc::clone(&second));
    set.insert(Arc::clone(&third));

    let ordered: Vec<SeqProxy> = set.iter().map(|tx| tx.get_seq_proxy()).collect();
    assert_eq!(
        ordered,
        vec![
            SeqProxy::ticket(2),
            SeqProxy::ticket(5),
            SeqProxy::ticket(9)
        ]
    );
}

#[test]
fn ledger_canonical_tx_set_uses_txid_tiebreak_for_equal_account_and_seqproxy() {
    let source = account("1111111111111111111111111111111111111111");
    let left = payment_tx(
        source,
        account("AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"),
        7,
        None,
        10,
    );
    let right = payment_tx(
        source,
        account("BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB"),
        7,
        None,
        10,
    );

    let expected = {
        let mut ids = vec![left.get_transaction_id(), right.get_transaction_id()];
        ids.sort();
        ids
    };

    let mut set = CanonicalTXSet::new(Uint256::zero());
    set.insert(Arc::clone(&right));
    set.insert(Arc::clone(&left));

    assert_eq!(tx_ids(&set), expected);
}

#[test]
fn ledger_canonical_tx_set_salts_account_order() {
    let lower = payment_tx(
        account("0000000000000000000000000000000000000001"),
        account("AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"),
        1,
        None,
        10,
    );
    let higher = payment_tx(
        account("FFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFF"),
        account("BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB"),
        1,
        None,
        10,
    );

    let mut zero_salt = CanonicalTXSet::new(Uint256::zero());
    zero_salt.insert(Arc::clone(&higher));
    zero_salt.insert(Arc::clone(&lower));

    let mut salted =
        CanonicalTXSet::new(Uint256::from_hex(&"F".repeat(64)).expect("salt should parse"));
    salted.insert(Arc::clone(&higher));
    salted.insert(Arc::clone(&lower));

    assert_ne!(tx_ids(&zero_salt), tx_ids(&salted));
}

#[test]
fn ledger_canonical_tx_set_duplicate_insert_does_not_replace_or_grow() {
    let tx = payment_tx(
        account("1111111111111111111111111111111111111111"),
        account("AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"),
        1,
        None,
        10,
    );

    let mut set = CanonicalTXSet::new(Uint256::zero());
    set.insert(Arc::clone(&tx));
    set.insert(Arc::clone(&tx));

    assert_eq!(set.len(), 1);
}

#[test]
fn ledger_canonical_tx_set_reset_clears_entries_and_replaces_salt() {
    let tx = payment_tx(
        account("1111111111111111111111111111111111111111"),
        account("AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"),
        1,
        None,
        10,
    );
    let mut set = CanonicalTXSet::new(Uint256::zero());
    set.insert(tx);

    let new_salt = Uint256::from_u64(9);
    set.reset(new_salt);

    assert!(set.is_empty());
    assert_eq!(set.key(), new_salt);
}

#[test]
fn ledger_canonical_tx_set_pop_acct_transaction_returns_exact_next_sequence() {
    let source = account("1111111111111111111111111111111111111111");
    let current = payment_tx(
        source,
        account("AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"),
        5,
        None,
        10,
    );
    let next = payment_tx(
        source,
        account("BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB"),
        6,
        None,
        11,
    );

    let mut set = CanonicalTXSet::new(Uint256::zero());
    set.insert(Arc::clone(&next));

    let popped = set
        .pop_acct_transaction(&current)
        .expect("next sequence should pop");
    assert_eq!(popped.get_transaction_id(), next.get_transaction_id());
    assert!(set.is_empty());
}

#[test]
fn ledger_canonical_tx_set_pop_acct_transaction_returns_lowest_ticket_after_sequences() {
    let source = account("1111111111111111111111111111111111111111");
    let current = payment_tx(
        source,
        account("AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"),
        5,
        None,
        10,
    );
    let ticket_low = payment_tx(
        source,
        account("BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB"),
        0,
        Some(2),
        11,
    );
    let ticket_high = payment_tx(
        source,
        account("CCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCC"),
        0,
        Some(7),
        12,
    );

    let mut set = CanonicalTXSet::new(Uint256::zero());
    set.insert(Arc::clone(&ticket_high));
    set.insert(Arc::clone(&ticket_low));

    let popped = set
        .pop_acct_transaction(&current)
        .expect("lowest ticket should pop");
    assert_eq!(popped.get_transaction_id(), ticket_low.get_transaction_id());
}

#[test]
fn ledger_canonical_tx_set_pop_acct_transaction_blocks_on_sequence_gap() {
    let source = account("1111111111111111111111111111111111111111");
    let current = payment_tx(
        source,
        account("AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"),
        5,
        None,
        10,
    );
    let gapped_sequence = payment_tx(
        source,
        account("BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB"),
        7,
        None,
        11,
    );
    let ticket = payment_tx(
        source,
        account("CCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCC"),
        0,
        Some(1),
        12,
    );

    let mut set = CanonicalTXSet::new(Uint256::zero());
    set.insert(Arc::clone(&gapped_sequence));
    set.insert(Arc::clone(&ticket));

    assert!(set.pop_acct_transaction(&current).is_none());
    assert_eq!(set.len(), 2);
}
