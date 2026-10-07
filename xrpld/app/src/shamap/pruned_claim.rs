//! Bridge from the SHAMap view of a validated ledger to the pruned node
//! store's [`ClaimDelta`].
//!
//! On each validated ledger the pruned store needs three node-hash sets: the
//! state nodes this ledger adds relative to the previously claimed state, the
//! state nodes it retires, and the nodes it owns outright (its transaction
//! tree and header). This module computes them with the existing
//! `visit_differences` diff primitive, so the pruned index stays SHAMap-
//! agnostic and only ever receives hashes.
//!
//! The diff is taken against the previously *claimed* state, not necessarily
//! the parent ledger: a gap (missed ledgers) is handled by diffing directly
//! across it, which yields a larger but still correct delta.

use basics::base_uint::Uint256;
use basics::memory::intrusive_pointer::SharedIntrusive;
use ledger::Ledger;
use nodestore::ClaimDelta;
use shamap::sync::SyncTree;
use shamap::tree_node::SHAMapTreeNode;

/// Collect every node hash present in `want` but not in `have`, using the
/// shared difference walk. `have` of `None` collects the whole `want` tree
/// (used for owned tx-tree nodes and the first claim's anchor).
fn collect_new_nodes(want: &SyncTree, have: Option<&SyncTree>) -> Vec<Uint256> {
    let want_root = want.root();
    if want_root.get_hash().is_zero() {
        return Vec::new();
    }
    let have_root = have.map(SyncTree::root);
    let have_backed = have.is_some_and(SyncTree::backed);
    // The trees of a just-validated ledger are fully resident, so the fetch
    // callbacks never need the store; return None and let the walk use the
    // linked children it already holds.
    let mut want_fetch = |_hash| None;
    let mut have_fetch = |_hash| None;
    let mut out = Vec::new();
    let _ = shamap::difference::visit_differences(
        &want_root,
        have_root.as_ref(),
        want.backed(),
        &mut want_fetch,
        have_backed,
        &mut have_fetch,
        &mut |node: &SharedIntrusive<SHAMapTreeNode>| {
            out.push(*node.get_hash().as_uint256());
            true
        },
    );
    out
}

/// Build the [`ClaimDelta`] for `ledger` relative to the previously claimed
/// state tree `claimed_state`.
///
/// - `new_state` / `dead_state`: the symmetric difference of the state trees,
///   so shared subtrees (unchanged accounts) are visited in neither direction.
/// - `owned`: every node of this ledger's transaction tree. These are unique
///   to the ledger and die when it leaves the window, so the index records
///   their death at `seq + 1`.
///
/// `claimed_state` is `None` for the anchor (first claim after sync), making
/// the whole state tree new and nothing dead.
pub fn compute_claim_delta(
    ledger: &Ledger,
    claimed_state: Option<&SyncTree>,
    state_root_hash: Uint256,
) -> ClaimDelta {
    let new_state = collect_new_nodes(ledger.state_map(), claimed_state);
    let dead_state = match claimed_state {
        Some(prev) => collect_new_nodes(prev, Some(ledger.state_map())),
        None => Vec::new(),
    };
    let owned = collect_new_nodes(ledger.tx_map(), None);
    ClaimDelta {
        seq: ledger.header().seq,
        state_root: state_root_hash,
        new_state,
        dead_state,
        owned,
    }
}

// Behavioural coverage for compute_claim_delta runs in the shamap-store
// integration tests, which build real state and transaction trees through the
// SHAMap family fixtures. SyncTree cannot be constructed meaningfully in a
// unit test without that wiring, so there is no unit test module here.
