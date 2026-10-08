//! Synchronous validation ingress, matching rippled's
//! `PeerImp::checkValidation -> NetworkOPsImp::recvValidation` path.
//!
//! A received `TMValidation` is processed directly on the `JtValidationT`/
//! `JtValidationUt` job thread via [`process_validation_inline`]: the
//! signature is verified, the validation is fed into `NetworkOPs`-equivalent
//! ingress (`receive_validation_to_network_ops_with_accept`, which calls
//! `Validations::add` + `updateTrie` and `checkAccept`), and it is relayed per
//! the resulting report.
//!
//! There is deliberately no intermediate event queue or forwarder thread:
//! the overlay invokes the installed validation router synchronously on
//! receipt (`QueuedInbound::on_validation` early-returns once a router is
//! set), so a trusted validation reaches the validation trie before the next
//! consensus `getPrevLedger` read. An earlier design routed validations
//! through a shared `ConsensusEvent` event loop that also handled ledger
//! completion; that extra hop could delay a trusted validation past the
//! consensus query, transiently skewing the preferred ledger and demoting the
//! node out of `full`. Ledger-completion bookkeeping is owned by the
//! acquisition/durable-handoff completion path, not by this module.

use overlay::{Overlay, QueuedValidation};
use protocol::STValidation;

use crate::state::application_root::ApplicationRoot;

/// Process a single received validation inline: verify its signature, feed it
/// into `NetworkOPs`-equivalent ingress
/// (`receive_validation_to_network_ops_with_accept`), and relay it per the
/// resulting report.
///
/// Runs synchronously on the validation job thread, matching rippled
/// `PeerImp::checkValidation -> recvValidation`. Running it here (rather than
/// forwarding onto a shared event loop) ensures a trusted validation reaches
/// `Validations::add`/`updateTrie` before the next consensus `getPrevLedger`
/// read, keeping the node's preferred ledger aligned with the network
/// regardless of node type.
pub fn process_validation_inline(app: &ApplicationRoot, mut queued: Box<QueuedValidation>) {
    let Some(mut validation) = queued
        .validation
        .take()
        .or_else(|| parse_validation(&queued.message.validation))
    else {
        tracing::warn!(target: "consensus", peer = ?queued.peer_id, "dropped malformed validation");
        return;
    };
    if !validation.is_valid() {
        // Match rippled PeerImp::checkValidation:
        // charge(Resource::kFeeInvalidSignature, desc)
        if let Some(overlay_rt) = app.overlay_runtime() {
            if let Some(peer) = overlay_rt.overlay().find_peer_by_short_id(queued.peer_id) {
                peer.charge(
                    (*resource::FEE_INVALID_SIGNATURE).clone(),
                    "validation invalid signature".to_owned(),
                );
            }
        }
        tracing::warn!(target: "consensus", peer = ?queued.peer_id, "dropped invalid validation signature");
        return;
    }
    let report = app.receive_validation_to_network_ops_with_accept(&mut validation, "peer", app);
    // Matches the reference's relay decision: after processing, relay the
    // validation to other peers if the report indicates it should be relayed
    // (trusted validations are always relayed; untrusted only when
    // relay_untrusted_validations is configured), or if it came from a
    // cluster peer.
    if let Some(report) = report {
        let relay_from_cluster = app.overlay_runtime().is_some_and(|overlay_rt| {
            overlay_rt
                .overlay()
                .find_peer_by_short_id(queued.peer_id)
                .is_some_and(|peer| peer.cluster())
        });
        if report.relay || relay_from_cluster {
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
