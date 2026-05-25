use ledger::{
    Ledger, LedgerHeader, get_next_ledger_time_resolution, is_flag_ledger, is_voting_ledger,
    round_close_time,
};
use shamap::sync::{SHAMapType, SyncState};

#[test]
fn ledger_new_matches_narrow_cpp_map_roles() {
    let ledger = Ledger::new(
        LedgerHeader {
            seq: 500,
            ..LedgerHeader::default()
        },
        true,
    );

    assert_eq!(ledger.state_map().map_type(), SHAMapType::State);
    assert_eq!(ledger.tx_map().map_type(), SHAMapType::Transaction);
    assert_eq!(ledger.state_map().state(), SyncState::Modifying);
    assert_eq!(ledger.tx_map().state(), SyncState::Modifying);
}

#[test]
fn ledger_timing_helpers_match_current_cpp_examples() {
    assert_eq!(get_next_ledger_time_resolution(30, true, 8), 20);
    assert_eq!(get_next_ledger_time_resolution(20, true, 16), 10);
    assert_eq!(get_next_ledger_time_resolution(10, true, 24), 10);
    assert_eq!(get_next_ledger_time_resolution(30, false, 1), 60);
    assert_eq!(get_next_ledger_time_resolution(60, false, 2), 90);
    assert_eq!(get_next_ledger_time_resolution(120, false, 3), 120);

    assert_eq!(round_close_time(0, 30), 0);
    assert_eq!(round_close_time(29, 60), 0);
    assert_eq!(round_close_time(30, 1), 30);
    assert_eq!(round_close_time(31, 60), 60);
    assert_eq!(round_close_time(30, 60), 60);
    assert_eq!(round_close_time(59, 60), 60);
    assert_eq!(round_close_time(60, 60), 60);
    assert_eq!(round_close_time(61, 60), 60);
}

#[test]
fn ledger_timing_and_voting_helpers_wrap_u32_max_unsigned_arithmetic() {
    let previous = Ledger::new(
        LedgerHeader {
            seq: u32::MAX,
            close_time: 200,
            close_time_resolution: 30,
            close_flags: 0,
            ..LedgerHeader::default()
        },
        true,
    );

    let next = Ledger::from_previous(&previous, 31);

    assert_eq!(get_next_ledger_time_resolution(30, true, 0), 20);
    assert_eq!(next.header().seq, 0);
    assert_eq!(next.header().parent_hash, previous.header().hash);
    assert_eq!(next.header().close_time_resolution, 20);
    assert_eq!(next.header().close_time, 220);
    assert!(!next.is_voting_ledger());
}

#[test]
fn flag_and_voting_ledger_helpers_match_current_cpp_interval_rules() {
    assert!(is_flag_ledger(256));
    assert!(!is_flag_ledger(255));
    assert!(is_voting_ledger(256));
    assert!(!is_voting_ledger(255));

    let flag_ledger = Ledger::new(
        LedgerHeader {
            seq: 256,
            ..LedgerHeader::default()
        },
        false,
    );
    assert!(flag_ledger.is_flag_ledger());
    assert!(!flag_ledger.is_voting_ledger());

    let voting_ledger = Ledger::new(
        LedgerHeader {
            seq: 255,
            ..LedgerHeader::default()
        },
        false,
    );
    assert!(voting_ledger.is_voting_ledger());
    assert!(!voting_ledger.is_flag_ledger());
}
