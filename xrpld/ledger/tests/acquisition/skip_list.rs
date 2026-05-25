use basics::base_uint::Uint256;
use ledger::{Fees, Ledger, LedgerConfig, LedgerJournal};
use protocol::{FeatureSet, decode_ledger_hashes_entry, skip_keylet, skip_keylet_for_ledger};

#[derive(Debug, Default)]
struct RecordingLedgerJournal {
    warns: std::sync::Mutex<Vec<String>>,
}

impl RecordingLedgerJournal {
    fn warns(&self) -> Vec<String> {
        self.warns
            .lock()
            .expect("ledger journal mutex must not be poisoned")
            .clone()
    }
}

impl LedgerJournal for RecordingLedgerJournal {
    fn info(&self, _message: &str) {}

    fn warn(&self, message: &str) {
        self.warns
            .lock()
            .expect("ledger journal mutex must not be poisoned")
            .push(message.to_owned());
    }
}

fn sample_ledger_config(features: impl IntoIterator<Item = Uint256>) -> LedgerConfig {
    LedgerConfig::new(
        Fees {
            base: 10,
            reserve: 20,
            increment: 30,
        },
        FeatureSet::new(features),
    )
}

#[test]
fn update_skip_list_records_previous_hash_in_short_list() {
    let config = sample_ledger_config([]);
    let genesis = Ledger::create_genesis(false, &config, []).expect("genesis ledger should build");
    let mut next = Ledger::from_previous(&genesis, 10);

    next.update_skip_list()
        .expect("skip-list update should write the short list");

    let (short_list, _) = next
        .state_map()
        .peek_item_with_hash(skip_keylet().key, &mut |_| None)
        .expect("skip-list read should succeed")
        .expect("short skip-list entry should exist");
    let decoded =
        decode_ledger_hashes_entry(short_list.data()).expect("short skip-list entry should decode");

    assert_eq!(decoded.last_ledger_sequence, Some(genesis.header().seq));
    assert_eq!(decoded.hashes, vec![*genesis.header().hash.as_uint256()]);

    let journal = RecordingLedgerJournal::default();
    assert_eq!(
        next.hash_of_seq(next.header().seq, &journal),
        Some(next.header().hash)
    );
    assert_eq!(
        next.hash_of_seq(genesis.header().seq, &journal),
        Some(genesis.header().hash)
    );
    assert!(journal.warns().is_empty());
}

#[test]
fn update_skip_list_rolls_short_list_and_keeps_long_bucket() {
    let config = sample_ledger_config([]);
    let genesis = Ledger::create_genesis(false, &config, []).expect("genesis ledger should build");
    let mut history = vec![genesis];

    for close_time in 1..=512u32 {
        let mut next = Ledger::from_previous(
            history
                .last()
                .expect("history should contain a previous ledger"),
            close_time,
        );
        next.update_skip_list()
            .expect("skip-list update should succeed across the history build");
        history.push(next);
    }

    let latest = history
        .last()
        .expect("history should contain the latest ledger");
    assert_eq!(latest.header().seq, 513);

    let (short_list, _) = latest
        .state_map()
        .peek_item_with_hash(skip_keylet().key, &mut |_| None)
        .expect("short skip-list read should succeed")
        .expect("short skip-list entry should exist");
    let short_decoded =
        decode_ledger_hashes_entry(short_list.data()).expect("short skip-list should decode");
    assert_eq!(short_decoded.last_ledger_sequence, Some(512));
    assert_eq!(short_decoded.hashes.len(), 256);
    assert_eq!(
        short_decoded
            .hashes
            .first()
            .copied()
            .expect("rolled short list should retain 256 hashes"),
        *history[256].header().hash.as_uint256()
    );
    assert_eq!(
        short_decoded
            .hashes
            .last()
            .copied()
            .expect("rolled short list should retain the parent hash"),
        *history[511].header().hash.as_uint256()
    );

    let (long_list, _) = latest
        .state_map()
        .peek_item_with_hash(skip_keylet_for_ledger(256).key, &mut |_| None)
        .expect("long skip-list read should succeed")
        .expect("long skip-list entry should exist");
    let long_decoded =
        decode_ledger_hashes_entry(long_list.data()).expect("long skip-list should decode");
    assert_eq!(long_decoded.last_ledger_sequence, Some(512));
    assert_eq!(
        long_decoded.hashes,
        vec![
            *history[255].header().hash.as_uint256(),
            *history[511].header().hash.as_uint256(),
        ]
    );

    let journal = RecordingLedgerJournal::default();
    assert_eq!(
        latest.hash_of_seq(512, &journal),
        Some(history[511].header().hash)
    );
    assert_eq!(
        latest.hash_of_seq(257, &journal),
        Some(history[256].header().hash)
    );
    assert_eq!(
        latest.hash_of_seq(256, &journal),
        Some(history[255].header().hash)
    );
    assert_eq!(latest.hash_of_seq(255, &journal), None);
    assert_eq!(latest.hash_of_seq(514, &journal), None);
}
