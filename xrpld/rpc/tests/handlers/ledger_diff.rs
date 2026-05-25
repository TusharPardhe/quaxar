//! Tests for the ledger diff RPC handler.

use std::collections::BTreeMap;

use basics::{base_uint::Uint256, sha_map_hash::SHAMapHash};
use ledger::{Ledger, LedgerHeader};
use rpc::{
    LedgerDiffError, LedgerDiffObject, LedgerDiffRequest, LedgerDiffResolved, LedgerDiffSource,
    LedgerDiffSpecifier, do_ledger_diff,
};
use shamap::{item::SHAMapItem, mutation::MutableTree, sync::SyncTree, tree_node::SHAMapNodeType};

#[derive(Debug)]
struct StoredLedger {
    ledger: Ledger,
    validated: bool,
}

#[derive(Debug, Default)]
struct FakeLedgerDiffSource {
    ledgers: BTreeMap<u32, StoredLedger>,
    hashes: BTreeMap<SHAMapHash, u32>,
    current: Option<u32>,
    closed: Option<u32>,
    validated: Option<u32>,
}

impl FakeLedgerDiffSource {
    fn insert(&mut self, ledger: Ledger, validated: bool) {
        let header = ledger.header();
        self.hashes.insert(header.hash, header.seq);
        self.ledgers
            .insert(header.seq, StoredLedger { ledger, validated });
    }

    fn current(mut self, seq: u32) -> Self {
        self.current = Some(seq);
        self
    }

    fn closed(mut self, seq: u32) -> Self {
        self.closed = Some(seq);
        self
    }

    fn validated(mut self, seq: u32) -> Self {
        self.validated = Some(seq);
        self
    }

    fn resolve_seq(&self, seq: u32) -> Option<LedgerDiffResolved<'_>> {
        let stored = self.ledgers.get(&seq)?;
        Some(if stored.validated {
            LedgerDiffResolved::Ledger(&stored.ledger)
        } else {
            LedgerDiffResolved::NotValidated
        })
    }
}

impl LedgerDiffSource for FakeLedgerDiffSource {
    fn ledger_from_specifier(
        &self,
        specifier: LedgerDiffSpecifier,
    ) -> Option<LedgerDiffResolved<'_>> {
        match specifier {
            LedgerDiffSpecifier::Sequence(seq) => self.resolve_seq(seq),
            LedgerDiffSpecifier::Hash(hash) => self
                .hashes
                .get(&hash)
                .and_then(|seq| self.resolve_seq(*seq)),
            LedgerDiffSpecifier::Current => self.current.and_then(|seq| self.resolve_seq(seq)),
            LedgerDiffSpecifier::Closed => self.closed.and_then(|seq| self.resolve_seq(seq)),
            LedgerDiffSpecifier::Validated => self.validated.and_then(|seq| self.resolve_seq(seq)),
        }
    }
}

fn hash(fill: u8) -> SHAMapHash {
    SHAMapHash::new(Uint256::from_array([fill; 32]))
}

fn key(fill: u8) -> Uint256 {
    Uint256::from_array([fill; 32])
}

fn payload(fill: u8) -> Vec<u8> {
    (0..16).map(|offset| fill.wrapping_add(offset)).collect()
}

fn build_state_map_with_items(items: &[(Uint256, Vec<u8>)], ledger_seq: u32) -> SyncTree {
    let mut tree = MutableTree::new(1);
    for (key, payload) in items {
        tree.add_item(
            SHAMapNodeType::AccountState,
            SHAMapItem::new(*key, payload.clone()),
        )
        .expect("state map item insertion should succeed");
    }

    SyncTree::from_root_with_type(
        tree.root(),
        shamap::sync::SHAMapType::State,
        false,
        ledger_seq,
        shamap::sync::SyncState::Immutable,
    )
}

fn empty_tx_map(ledger_seq: u32) -> SyncTree {
    SyncTree::new_with_type(shamap::sync::SHAMapType::Transaction, false, ledger_seq)
}

fn build_ledger(seq: u32, hash_fill: u8, items: &[(Uint256, Vec<u8>)]) -> Ledger {
    Ledger::from_maps(
        LedgerHeader {
            seq,
            hash: hash(hash_fill),
            ..LedgerHeader::default()
        },
        build_state_map_with_items(items, seq),
        empty_tx_map(seq),
    )
}

#[test]
fn ledger_diff_returns_empty_for_identical_ledgers() {
    let base = build_ledger(10, 0x11, &[(key(1), payload(1)), (key(2), payload(2))]);
    let mut source = FakeLedgerDiffSource::default();
    source.insert(base, true);
    let source = source.current(10).closed(10).validated(10);

    let result = do_ledger_diff(
        LedgerDiffRequest {
            base_ledger: LedgerDiffSpecifier::Current,
            desired_ledger: LedgerDiffSpecifier::Validated,
            include_blobs: false,
        },
        &source,
    )
    .expect("identical ledgers should diff successfully");

    assert!(result.ledger_objects.is_empty());
}

#[test]
fn ledger_diff_omits_blobs_when_disabled() {
    let base = build_ledger(20, 0x21, &[(key(1), payload(1)), (key(2), payload(2))]);
    let desired = build_ledger(21, 0x22, &[(key(1), payload(9)), (key(3), payload(3))]);

    let mut source = FakeLedgerDiffSource::default();
    source.insert(base, true);
    source.insert(desired, true);

    let result = do_ledger_diff(
        LedgerDiffRequest {
            base_ledger: LedgerDiffSpecifier::Sequence(20),
            desired_ledger: LedgerDiffSpecifier::Sequence(21),
            include_blobs: false,
        },
        &source,
    )
    .expect("ledger diff should succeed");

    assert_eq!(
        result.ledger_objects,
        vec![
            LedgerDiffObject {
                key: key(1),
                data: None,
            },
            LedgerDiffObject {
                key: key(2),
                data: None,
            },
            LedgerDiffObject {
                key: key(3),
                data: None,
            },
        ]
    );
}

#[test]
fn ledger_diff_includes_desired_blobs_for_created_and_modified_only() {
    let base = build_ledger(30, 0x31, &[(key(1), payload(1)), (key(2), payload(2))]);
    let desired = build_ledger(31, 0x32, &[(key(1), payload(9)), (key(3), payload(3))]);
    let desired_hash = desired.header().hash;

    let mut source = FakeLedgerDiffSource::default();
    source.insert(base, true);
    source.insert(desired, true);

    let result = do_ledger_diff(
        LedgerDiffRequest {
            base_ledger: LedgerDiffSpecifier::Sequence(30),
            desired_ledger: LedgerDiffSpecifier::Hash(desired_hash),
            include_blobs: true,
        },
        &source,
    )
    .expect("ledger diff should succeed");

    assert_eq!(
        result.ledger_objects,
        vec![
            LedgerDiffObject {
                key: key(1),
                data: Some(payload(9)),
            },
            LedgerDiffObject {
                key: key(2),
                data: None,
            },
            LedgerDiffObject {
                key: key(3),
                data: Some(payload(3)),
            },
        ]
    );
}

#[test]
fn ledger_diff_reports_missing_base_ledger() {
    let source = FakeLedgerDiffSource::default();

    let error = do_ledger_diff(
        LedgerDiffRequest {
            base_ledger: LedgerDiffSpecifier::Sequence(1),
            desired_ledger: LedgerDiffSpecifier::Sequence(2),
            include_blobs: false,
        },
        &source,
    )
    .expect_err("missing base ledger should fail");

    assert!(matches!(error, LedgerDiffError::BaseLedgerNotFound));
}

#[test]
fn ledger_diff_reports_unvalidated_ledgers() {
    let base = build_ledger(40, 0x41, &[(key(1), payload(1))]);
    let desired = build_ledger(41, 0x42, &[(key(1), payload(2))]);

    let mut source = FakeLedgerDiffSource::default();
    source.insert(base, false);
    source.insert(desired, false);

    let base_error = do_ledger_diff(
        LedgerDiffRequest {
            base_ledger: LedgerDiffSpecifier::Sequence(40),
            desired_ledger: LedgerDiffSpecifier::Sequence(41),
            include_blobs: false,
        },
        &source,
    )
    .expect_err("unvalidated base ledger should fail");
    assert!(matches!(
        base_error,
        LedgerDiffError::BaseLedgerNotValidated
    ));

    let base = build_ledger(50, 0x51, &[(key(1), payload(1))]);
    let desired = build_ledger(51, 0x52, &[(key(1), payload(2))]);
    let mut source = FakeLedgerDiffSource::default();
    source.insert(base, true);
    source.insert(desired, false);

    let desired_error = do_ledger_diff(
        LedgerDiffRequest {
            base_ledger: LedgerDiffSpecifier::Sequence(50),
            desired_ledger: LedgerDiffSpecifier::Sequence(51),
            include_blobs: false,
        },
        &source,
    )
    .expect_err("unvalidated desired ledger should fail");
    assert!(matches!(
        desired_error,
        LedgerDiffError::DesiredLedgerNotValidated
    ));
}
