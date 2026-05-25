//! Narrow helper for the `SHAMapStoreImp::clearSql(...)` batch delete loop.
//!
//! The reference code uses a simple sequence:
//! - discover the minimum ledger sequence in a table,
//! - do nothing when the table is already empty or already aligned,
//! - delete in bounded batches,
//! - check for stop/health after each batch,
//! - and sleep between batches when more work remains.
//!
//! This module keeps that behavior explicit and testable through callbacks
//! rather than pretending the full relational database owner is already ported.

use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClearSqlOutcome {
    Completed,
    Stopped,
}

pub fn clear_sql_batches<GetMinSeq, DeleteBeforeSeq, ShouldStop, Sleep>(
    last_rotated: u32,
    delete_batch: u32,
    back_off: Duration,
    mut get_min_seq: GetMinSeq,
    mut delete_before_seq: DeleteBeforeSeq,
    mut should_stop: ShouldStop,
    mut sleep: Sleep,
) -> ClearSqlOutcome
where
    GetMinSeq: FnMut() -> Option<u32>,
    DeleteBeforeSeq: FnMut(u32),
    ShouldStop: FnMut() -> bool,
    Sleep: FnMut(Duration),
{
    let Some(mut min_seq) = get_min_seq() else {
        return ClearSqlOutcome::Completed;
    };

    if should_stop() {
        return ClearSqlOutcome::Stopped;
    }

    if min_seq >= last_rotated || delete_batch == 0 {
        return ClearSqlOutcome::Completed;
    }

    while min_seq < last_rotated {
        min_seq = min_seq.saturating_add(delete_batch).min(last_rotated);
        delete_before_seq(min_seq);

        if should_stop() {
            return ClearSqlOutcome::Stopped;
        }

        if min_seq < last_rotated {
            sleep(back_off);

            if should_stop() {
                return ClearSqlOutcome::Stopped;
            }
        }
    }

    ClearSqlOutcome::Completed
}

#[cfg(test)]
mod tests {
    use super::{ClearSqlOutcome, clear_sql_batches};
    use std::cell::Cell;
    use std::time::Duration;

    #[test]
    fn clear_sql_batches_reports_stop_before_aligned_short_circuit() {
        let stop_calls = Cell::new(0usize);
        let outcome = clear_sql_batches(
            900,
            100,
            Duration::from_secs(0),
            || Some(900),
            |_min_seq| panic!("should not delete when already aligned"),
            || {
                stop_calls.set(stop_calls.get() + 1);
                true
            },
            |_| panic!("should not sleep when already aligned"),
        );

        assert_eq!(outcome, ClearSqlOutcome::Stopped);
        assert_eq!(stop_calls.get(), 1);
    }
}
