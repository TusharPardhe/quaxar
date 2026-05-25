use ledger::ApplyView;
use protocol::{STTx, Ter};
use tx::run_batch_do_apply;

///
/// The reference doApply() for Batch literally returns tesSUCCESS — the inner
/// transactions are applied separately by the outer framework in
/// `applyBatchTransactions`, not inside doApply. The real Batch logic
/// lives in preflight (validates inner txs) and checkSign (verifies
/// batch signatures), both handled by the tx crate.
pub fn apply_batch<V: ApplyView>(_view: &mut V, _sttx: &STTx) -> Ter {
    run_batch_do_apply()
}
