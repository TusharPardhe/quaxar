//! Validates that the Ledger struct can hold state, build SHAMap roots, and produce
//! deterministic hashes — proving the full ledger pipeline works end-to-end.

use basics::base_uint::Uint160;
use basics::sha_map_hash::SHAMapHash;
use ledger::{Ledger, LedgerHeader, StateBatchOp};
use protocol::{
    AccountID, Keylet, STAmount, STLedgerEntry, account_keylet, calculate_ledger_hash,
    get_field_by_symbol,
};

fn account(fill: u8) -> AccountID {
    AccountID::from_hex(&format!("{fill:02x}").repeat(20)).unwrap()
}

fn raw_id(id: AccountID) -> Uint160 {
    Uint160::from_slice(id.data()).expect("account width")
}

fn account_key(id: AccountID) -> Keylet {
    account_keylet(raw_id(id))
}

fn make_account_root_bytes(account_id: AccountID, balance: u64) -> Vec<u8> {
    let keylet = account_key(account_id);
    let mut sle = STLedgerEntry::new(keylet);
    sle.set_account_id(get_field_by_symbol("sfAccount"), account_id);
    sle.set_field_amount(
        get_field_by_symbol("sfBalance"),
        STAmount::new_native(balance, false),
    );
    sle.set_field_u32(get_field_by_symbol("sfSequence"), 1);
    sle.set_field_u32(get_field_by_symbol("sfOwnerCount"), 0);
    sle.get_serializer().data().to_vec()
}

/// Test: A ledger can hold account state entries and produce a non-zero state root hash.
#[test]
fn ledger_holds_state_and_produces_root_hash() {
    let mut ledger = Ledger::new(
        LedgerHeader {
            seq: 100,
            drops: 100_000_000_000,
            close_time: 700_000_000,
            close_time_resolution: 10,
            ..LedgerHeader::default()
        },
        false,
    );

    let alice = account(0x11);
    let bob = account(0x22);

    let alice_key = account_key(alice).key;
    let bob_key = account_key(bob).key;

    ledger
        .apply_state_batch(&[
            (
                StateBatchOp::Insert,
                alice_key,
                make_account_root_bytes(alice, 50_000_000_000),
            ),
            (
                StateBatchOp::Insert,
                bob_key,
                make_account_root_bytes(bob, 50_000_000_000),
            ),
        ])
        .expect("insert should succeed");

    ledger.set_immutable(true);

    assert_ne!(
        ledger.header().account_hash,
        SHAMapHash::default(),
        "State root hash must be non-zero after inserting entries"
    );

    let hash = calculate_ledger_hash(&ledger.header());
    assert_ne!(hash, SHAMapHash::default(), "Ledger hash must be non-zero");
}

/// Test: Same state entries always produce the same root hash (deterministic).
#[test]
fn ledger_hash_is_deterministic() {
    let build = || {
        let mut ledger = Ledger::new(
            LedgerHeader {
                seq: 200,
                drops: 99_000_000_000,
                close_time: 750_000_000,
                close_time_resolution: 10,
                ..LedgerHeader::default()
            },
            false,
        );

        let alice = account(0xAA);
        let bob = account(0xBB);
        let carol = account(0xCC);

        ledger
            .apply_state_batch(&[
                (
                    StateBatchOp::Insert,
                    account_key(alice).key,
                    make_account_root_bytes(alice, 33_000_000_000),
                ),
                (
                    StateBatchOp::Insert,
                    account_key(bob).key,
                    make_account_root_bytes(bob, 33_000_000_000),
                ),
                (
                    StateBatchOp::Insert,
                    account_key(carol).key,
                    make_account_root_bytes(carol, 33_000_000_000),
                ),
            ])
            .expect("insert should succeed");

        ledger.set_immutable(true);
        calculate_ledger_hash(&ledger.header())
    };

    let hash1 = build();
    let hash2 = build();
    let hash3 = build();

    assert_eq!(hash1, hash2, "Ledger hash must be deterministic");
    assert_eq!(hash2, hash3, "Ledger hash must be deterministic");
}

/// Test: Different state produces different hashes.
#[test]
fn different_state_produces_different_hash() {
    let build_with_balance = |balance: u64| {
        let mut ledger = Ledger::new(
            LedgerHeader {
                seq: 300,
                drops: balance,
                close_time: 800_000_000,
                close_time_resolution: 10,
                ..LedgerHeader::default()
            },
            false,
        );

        let alice = account(0xDD);
        ledger
            .apply_state_batch(&[(
                StateBatchOp::Insert,
                account_key(alice).key,
                make_account_root_bytes(alice, balance),
            )])
            .expect("insert");

        ledger.set_immutable(true);
        calculate_ledger_hash(&ledger.header())
    };

    let hash_a = build_with_balance(1_000_000_000);
    let hash_b = build_with_balance(2_000_000_000);

    assert_ne!(
        hash_a, hash_b,
        "Different balances must produce different hashes"
    );
}

/// Test: State entries can be read back after insertion.
#[test]
fn ledger_state_entries_readable_after_insert() {
    let mut ledger = Ledger::new(
        LedgerHeader {
            seq: 400,
            drops: 100_000_000_000,
            close_time: 850_000_000,
            close_time_resolution: 10,
            ..LedgerHeader::default()
        },
        false,
    );

    let alice = account(0xEE);
    let alice_key = account_key(alice);

    ledger
        .apply_state_batch(&[(
            StateBatchOp::Insert,
            alice_key.key,
            make_account_root_bytes(alice, 77_777_777_777),
        )])
        .expect("insert");

    // Read it back
    let read_back = ledger.read(alice_key).expect("read should not error");
    assert!(read_back.is_some(), "Inserted entry must be readable");

    let sle = read_back.unwrap();
    let balance = sle.get_field_amount(get_field_by_symbol("sfBalance"));
    assert_eq!(balance.xrp().drops(), 77_777_777_777i64);
}

/// Test: Ledger exists check works.
#[test]
fn ledger_exists_check() {
    let mut ledger = Ledger::new(
        LedgerHeader {
            seq: 500,
            ..LedgerHeader::default()
        },
        false,
    );

    let alice = account(0xFF);
    let bob = account(0x01);

    ledger
        .apply_state_batch(&[(
            StateBatchOp::Insert,
            account_key(alice).key,
            make_account_root_bytes(alice, 1_000_000_000),
        )])
        .expect("insert");

    assert!(ledger.exists_keylet(account_key(alice)).unwrap_or(false));
    assert!(!ledger.exists_keylet(account_key(bob)).unwrap_or(true));
}
