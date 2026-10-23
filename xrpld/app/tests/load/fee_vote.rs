use std::sync::Arc;

use app::{FeeSetup, FeeVote, FeeVoteJournal, FeeVoteLedgerView, ShamapVoteTxSet};
use basics::tagged_cache::MonotonicClock;
use ledger::Fees;
use protocol::{
    KeyType, STAmount, STTx, STValidation, SecretKey, SerialIter, TxType,
    VF_FULL_VALIDATION, calc_node_id, derive_public_key, get_field_by_symbol,
};
use shamap::{storage::StorageTree, tree_node_cache::TreeNodeCache};
use time::Duration;

#[derive(Default)]
struct RecordingJournal {
    info: std::sync::Mutex<Vec<String>>,
    warn: std::sync::Mutex<Vec<String>>,
}

impl FeeVoteJournal for RecordingJournal {
    fn info(&self, message: &str) {
        self.info
            .lock()
            .expect("fee vote info mutex must not be poisoned")
            .push(message.to_owned());
    }

    fn warn(&self, message: &str) {
        self.warn
            .lock()
            .expect("fee vote warn mutex must not be poisoned")
            .push(message.to_owned());
    }
}

struct FakeLedger {
    fees: Fees,
    seq: u32,
    is_flag_ledger: bool,
}

impl FeeVoteLedgerView for FakeLedger {
    fn fees(&self) -> Fees {
        self.fees
    }

    fn seq(&self) -> u32 {
        self.seq
    }

    fn is_flag_ledger(&self) -> bool {
        self.is_flag_ledger
    }
}

fn validator(seed: u8) -> (SecretKey, protocol::PublicKey) {
    let secret = SecretKey::from_bytes([seed; 32]);
    let public = derive_public_key(KeyType::Secp256k1, &secret).expect("validator public key");
    (secret, public)
}

fn validation_with_fill(
    seed: u8,
    ledger_seq: u32,
    trusted: bool,
    fill: impl FnOnce(&mut STValidation),
) -> STValidation {
    let (secret, public) = validator(seed);
    let mut validation =
        STValidation::new_signed(100, &public, calc_node_id(&public), &secret, |validation| {
            validation.set_field_h256(
                get_field_by_symbol("sfLedgerHash"),
                basics::base_uint::Uint256::from_u64(u64::from(seed) + 1),
            );
            validation.set_field_u32(get_field_by_symbol("sfLedgerSequence"), ledger_seq);
            validation.set_flag(VF_FULL_VALIDATION);
            fill(validation);
        })
        .expect("validation should sign");
    if trusted {
        validation.set_trusted();
    } else {
        validation.set_untrusted();
    }
    validation
}

fn fee_vote_setup() -> FeeSetup {
    FeeSetup {
        reference_fee: protocol::XRPAmount::from_drops(42),
        account_reserve: protocol::XRPAmount::from_drops(1_234_567),
        owner_reserve: protocol::XRPAmount::from_drops(7_654_321),
    }
}

#[test]
fn fee_vote_validation_uses_xrp_fee_fields() {
    let journal = RecordingJournal::default();
    let fee_vote = FeeVote::new(fee_vote_setup(), journal);
    let mut validation = validation_with_fill(1, 255, true, |_| {});

    fee_vote.do_validation(
        Fees {
            base: 10,
            reserve: 200_000,
            increment: 50_000,
        },
        &mut validation,
    );

    assert_eq!(
        validation.get_field_amount(get_field_by_symbol("sfBaseFeeDrops")),
        STAmount::from_xrp_amount(protocol::XRPAmount::from_drops(42))
    );
    assert_eq!(
        validation.get_field_amount(get_field_by_symbol("sfReserveBaseDrops")),
        STAmount::from_xrp_amount(protocol::XRPAmount::from_drops(1_234_567))
    );
    assert_eq!(
        validation.get_field_amount(get_field_by_symbol("sfReserveIncrementDrops")),
        STAmount::from_xrp_amount(protocol::XRPAmount::from_drops(7_654_321))
    );
    assert!(!validation.is_field_present(get_field_by_symbol("sfBaseFee")));
}

#[test]
fn fee_vote_inserts_a_single_fee_transaction_for_trusted_xrp_votes() {
    let fee_vote = FeeVote::new(fee_vote_setup(), RecordingJournal::default());
    let ledger = FakeLedger {
        fees: Fees {
            base: 10,
            reserve: 200_000,
            increment: 50_000,
        },
        seq: 256,
        is_flag_ledger: true,
    };

    let validations = vec![
        validation_with_fill(3, 256, true, |validation| {
            validation.set_field_amount(
                get_field_by_symbol("sfBaseFeeDrops"),
                STAmount::from_xrp_amount(protocol::XRPAmount::from_drops(42)),
            );
            validation.set_field_amount(
                get_field_by_symbol("sfReserveBaseDrops"),
                STAmount::from_xrp_amount(protocol::XRPAmount::from_drops(1_234_567)),
            );
            validation.set_field_amount(
                get_field_by_symbol("sfReserveIncrementDrops"),
                STAmount::from_xrp_amount(protocol::XRPAmount::from_drops(7_654_321)),
            );
        }),
        validation_with_fill(4, 256, false, |validation| {
            validation.set_field_amount(
                get_field_by_symbol("sfBaseFeeDrops"),
                STAmount::from_xrp_amount(protocol::XRPAmount::from_drops(99)),
            );
        }),
    ];
    let mut txs = Vec::<STTx>::new();

    fee_vote.do_voting(&ledger, &validations, &mut txs);

    assert_eq!(txs.len(), 1);
    let tx = &txs[0];
    assert_eq!(tx.get_txn_type(), TxType::FEE);
    assert_eq!(
        tx.get_field_u32(get_field_by_symbol("sfLedgerSequence")),
        257
    );
    assert_eq!(
        tx.get_field_amount(get_field_by_symbol("sfBaseFeeDrops")),
        STAmount::from_xrp_amount(protocol::XRPAmount::from_drops(42))
    );
    assert_eq!(
        tx.get_field_amount(get_field_by_symbol("sfReserveBaseDrops")),
        STAmount::from_xrp_amount(protocol::XRPAmount::from_drops(1_234_567))
    );
    assert_eq!(
        tx.get_field_amount(get_field_by_symbol("sfReserveIncrementDrops")),
        STAmount::from_xrp_amount(protocol::XRPAmount::from_drops(7_654_321))
    );
}

#[test]
fn fee_vote_ignores_invalid_or_untrusted_votes_() {
    let fee_vote = FeeVote::new(
        FeeSetup {
            reference_fee: protocol::XRPAmount::from_drops(30),
            account_reserve: protocol::XRPAmount::from_drops(300),
            owner_reserve: protocol::XRPAmount::from_drops(30),
        },
        RecordingJournal::default(),
    );
    let ledger = FakeLedger {
        fees: Fees {
            base: 10,
            reserve: 200,
            increment: 20,
        },
        seq: 512,
        is_flag_ledger: true,
    };
    let validations = vec![
        validation_with_fill(5, 512, true, |validation| {
            validation.set_field_amount(get_field_by_symbol("sfBaseFeeDrops"), STAmount::from_xrp_amount(protocol::XRPAmount::from_drops(30)));
            validation.set_field_amount(get_field_by_symbol("sfReserveBaseDrops"), STAmount::from_xrp_amount(protocol::XRPAmount::from_drops(300)));
            validation.set_field_amount(get_field_by_symbol("sfReserveIncrementDrops"), STAmount::from_xrp_amount(protocol::XRPAmount::from_drops(30)));
        }),
        validation_with_fill(6, 512, true, |validation| {
            // A non-native amount is not a usable fee vote (rippled doVote: noVote).
            validation.set_field_amount(
                get_field_by_symbol("sfBaseFeeDrops"),
                STAmount::from_iou_amount(
                    get_field_by_symbol("sfBaseFeeDrops"),
                    protocol::IOUAmount::from_parts(99, 0).expect("iou"),
                    protocol::Issue::new(protocol::currency_from_string("USD"), protocol::AccountID::from_array([9; 20])),
                ),
            );
        }),
        validation_with_fill(7, 512, false, |validation| {
            validation.set_field_amount(get_field_by_symbol("sfBaseFeeDrops"), STAmount::from_xrp_amount(protocol::XRPAmount::from_drops(99)));
            validation.set_field_amount(get_field_by_symbol("sfReserveBaseDrops"), STAmount::from_xrp_amount(protocol::XRPAmount::from_drops(999)));
            validation.set_field_amount(get_field_by_symbol("sfReserveIncrementDrops"), STAmount::from_xrp_amount(protocol::XRPAmount::from_drops(999)));
        }),
    ];
    let mut txs = Vec::<STTx>::new();

    fee_vote.do_voting(&ledger, &validations, &mut txs);

    assert_eq!(txs.len(), 1);
    let tx = &txs[0];
    assert_eq!(tx.get_field_amount(get_field_by_symbol("sfBaseFeeDrops")), STAmount::from_xrp_amount(protocol::XRPAmount::from_drops(30)));
    assert_eq!(tx.get_field_amount(get_field_by_symbol("sfReserveBaseDrops")), STAmount::from_xrp_amount(protocol::XRPAmount::from_drops(300)));
    assert_eq!(
        tx.get_field_amount(get_field_by_symbol("sfReserveIncrementDrops")),
        STAmount::from_xrp_amount(protocol::XRPAmount::from_drops(30))
    );
    assert!(!tx.is_field_present(get_field_by_symbol("sfBaseFee")));
}

#[test]
fn shamap_vote_tx_set_stores_fee_vote_transactions_in_real_shamap() {
    let fee_vote = FeeVote::new(fee_vote_setup(), RecordingJournal::default());
    let ledger = FakeLedger {
        fees: Fees {
            base: 10,
            reserve: 200_000,
            increment: 50_000,
        },
        seq: 256,
        is_flag_ledger: true,
    };
    let validations = vec![validation_with_fill(8, 256, true, |validation| {
        validation.set_field_amount(
            get_field_by_symbol("sfBaseFeeDrops"),
            STAmount::from_xrp_amount(protocol::XRPAmount::from_drops(42)),
        );
        validation.set_field_amount(
            get_field_by_symbol("sfReserveBaseDrops"),
            STAmount::from_xrp_amount(protocol::XRPAmount::from_drops(1_234_567)),
        );
        validation.set_field_amount(
            get_field_by_symbol("sfReserveIncrementDrops"),
            STAmount::from_xrp_amount(protocol::XRPAmount::from_drops(7_654_321)),
        );
    })];

    let cache = Arc::new(TreeNodeCache::<MonotonicClock>::new(
        "FeeVoteTxSet",
        128,
        Duration::seconds(60),
        MonotonicClock::default(),
    ));
    let mut tree = StorageTree::new(1, false, 256, cache);
    {
        let mut tx_set = ShamapVoteTxSet::new(&mut tree);
        fee_vote.do_voting(&ledger, &validations, &mut tx_set);
    }

    let mut txs = Vec::new();
    tree.visit_leaves(&mut |_| None, &mut |item| {
        let mut serial = SerialIter::new(item.data());
        txs.push(STTx::from_serial_iter(&mut serial));
    })
    .expect("fee vote transaction tree should be readable");

    assert_eq!(txs.len(), 1);
    assert_eq!(txs[0].get_txn_type(), TxType::FEE);
}
