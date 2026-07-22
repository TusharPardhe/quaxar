//! The consensus event-loop driver: a dedicated background thread that
//! processes incoming validations and newly-completed ledgers, feeding
//! them into `NetworkOPs`-equivalent validation ingress
//! (`receive_validation_to_network_ops_with_accept`) and ledger-history
//! bookkeeping. Ported from the event-driven portions of
//! `NetworkOPsImp::recvValidation` and `LedgerMaster::checkAccept`'s
//! newly-completed-ledger handling.
//!
//! `bootstrap.rs` constructs the channel via [`consensus_event_channel`],
//! spawns the loop via [`spawn_event_loop`], and forwards events into it
//! from two sources: a dedicated validation-forwarder thread (bridging the
//! overlay's `SyncSender<()>` notify pattern into [`ConsensusEvent::Validation`])
//! and the main bootstrap loop's `storeLedger` handling (emitting
//! [`ConsensusEvent::LedgerDone`] whenever an `InboundLedger` or shared
//! ledger-completion channel produces a new ledger).

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};

use overlay::QueuedValidation;
use protocol::STValidation;

use crate::ledger::inbound_ledgers::InboundLedgers;
use crate::state::application_root::ApplicationRoot;

/// An event fed into the consensus driver's event loop.
pub enum ConsensusEvent {
    /// A validation received from a peer, still in wire form (its
    /// suppression id and originating peer are carried alongside the raw
    /// `TMValidation` payload for dedup/relay bookkeeping upstream).
    Validation(QueuedValidation),
    /// A ledger has finished acquiring/building and is ready for
    /// `checkAccept`-style promotion to validated, if it has sufficient
    /// validation support.
    LedgerDone(Arc<ledger::Ledger>),
}

/// Construct the channel used to feed [`ConsensusEvent`]s into
/// [`spawn_event_loop`]'s background thread.
pub fn consensus_event_channel() -> (Sender<ConsensusEvent>, Receiver<ConsensusEvent>) {
    channel()
}

/// Parse a wire-format validation payload (`TMValidation.validation`) into
/// an `STValidation`, resolving the signer's node id via `calc_node_id`
/// (matching the reference's default lookup when no local manifest cache
/// override applies). Returns `None` on malformed input, matching the
/// reference's `invalid_argument` catch-and-drop behavior in
/// `NetworkOPsImp::recvValidation`.
fn parse_validation(bytes: &[u8]) -> Option<STValidation> {
    let mut sit = protocol::SerialIter::new(bytes);
    match STValidation::from_serial_iter_default_node_id(&mut sit, true) {
        Ok(v) => Some(v),
        Err(err) => {
            tracing::warn!(target: "consensus", ?err, "validation parse failed");
            None
        }
    }
}

/// Spawn the consensus event-loop background thread. Processes
/// [`ConsensusEvent`]s from `event_rx` until `stop` is set, dispatching
/// validations into `NetworkOPs`-equivalent ingress and newly-completed
/// ledgers into ledger-history bookkeeping.
pub fn spawn_event_loop(
    app: ApplicationRoot,
    shared_inbound: Arc<InboundLedgers>,
    event_rx: Receiver<ConsensusEvent>,
    stop: Arc<AtomicBool>,
) {
    std::thread::Builder::new()
        .name("consensus-event-loop".into())
        .spawn(move || {
            let _ = &shared_inbound;
            while !stop.load(Ordering::Acquire) {
                let event = match event_rx.recv_timeout(std::time::Duration::from_millis(200)) {
                    Ok(event) => event,
                    Err(std::sync::mpsc::RecvTimeoutError::Timeout) => continue,
                    Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
                };

                match event {
                    ConsensusEvent::Validation(queued) => {
                        let Some(mut validation) = parse_validation(&queued.message.validation) else {
                            tracing::warn!(target: "consensus", peer = ?queued.peer_id, "dropped malformed validation");
                            continue;
                        };
                        let report = app.receive_validation_to_network_ops_with_accept(&mut validation, "peer", &app);
                        // Matches the reference's relay decision: after
                        // processing, relay the validation to other peers
                        // if the report indicates it should be relayed
                        // (trusted validations are always relayed;
                        // untrusted only when relay_untrusted_validations
                        // is configured).
                        if let Some(report) = report {
                            if report.relay {
                                if let Some(overlay_rt) = app.overlay_runtime() {
                                    overlay_rt.overlay().relay_validation(
                                        queued.message.clone(),
                                        queued.suppression,
                                        *validation.get_signer_public(),
                                    );
                                }
                            }
                        }
                    }
                    ConsensusEvent::LedgerDone(ledger) => {
                        if let Some(runtime) = app.ledger_master_runtime() {
                            runtime.ledger_master().ledger_history().insert(Arc::clone(&ledger), true);
                            // Matches rippled's storeLedger() → checkAccept(ledger):
                            // once a newly-acquired ledger is stored in history,
                            // immediately check whether it has reached quorum.
                            // Without this, the ledger sits in history until
                            // the periodic bootstrap check finds it (~50ms),
                            // or worse, if the acquiring map promotion already
                            // happened before this event fires, the trie has
                            // the entry but check_accept was never triggered.
                            app.check_accept_hash_seq(
                                *ledger.header().hash.as_uint256(),
                                ledger.header().seq,
                            );
                        }
                        // Matches rippled's Validations::onLedger(ledger):
                        // register the ledger in the validations adaptor's
                        // local cache so subsequent trie operations
                        // (updateTrie → acquire) find it immediately without
                        // falling through to the slower ledger_history lookup.
                        app.validations().register_ledger(&ledger);
                    }
                }
            }
        })
        .expect("spawn consensus-event-loop thread");
}
