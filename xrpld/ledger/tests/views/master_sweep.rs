use ledger::{LedgerMasterSweepTarget, sweep_ledger_master_like};
use std::sync::Mutex;

#[derive(Default)]
struct RecordingSweepTarget {
    calls: Mutex<u32>,
}

impl RecordingSweepTarget {
    fn calls(&self) -> u32 {
        *self.calls.lock().expect("calls mutex poisoned")
    }
}

impl LedgerMasterSweepTarget for RecordingSweepTarget {
    fn sweep(&self) {
        *self.calls.lock().expect("calls mutex poisoned") += 1;
    }
}

#[test]
fn master_sweep_calls_history_and_fetch_pack_once_each() {
    let history = RecordingSweepTarget::default();
    let fetch_pack = RecordingSweepTarget::default();

    sweep_ledger_master_like(&history, &fetch_pack);

    assert_eq!(history.calls(), 1);
    assert_eq!(fetch_pack.calls(), 1);
}
