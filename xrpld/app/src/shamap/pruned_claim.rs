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
use basics::sha_map_hash::SHAMapHash;
use ledger::Ledger;
use nodestore::ClaimDelta;
use shamap::tree_node::SHAMapTreeNode;

/// Resolves a node by hash when a validated ledger's tree is backed and a
/// subtree is not resident. Production supplies a node-store reader.
pub type ClaimNodeFetcher =
    dyn Fn(SHAMapHash) -> Option<SharedIntrusive<SHAMapTreeNode>> + Send + Sync;

/// Collect every node hash present under `want` but not under `have`, using
/// the shared difference walk. `have` of `None` collects the whole `want`
/// tree, which is only used for a ledger's own (small) transaction tree.
///
/// Both trees are treated as backed: any child that is not resident is read
/// from the node store through `fetch`. A traversal error (a node neither
/// resident nor fetchable) is returned instead of yielding a partial set,
/// because claiming a partial delta would leave live nodes unclaimed.
fn collect_new_nodes(
    want: &SharedIntrusive<SHAMapTreeNode>,
    have: Option<&SharedIntrusive<SHAMapTreeNode>>,
    fetch: &ClaimNodeFetcher,
) -> Result<Vec<Uint256>, String> {
    if want.get_hash().is_zero() {
        return Ok(Vec::new());
    }
    let mut want_fetch = |hash| fetch(hash);
    let mut have_fetch = |hash| fetch(hash);
    let mut out = Vec::new();
    shamap::difference::visit_differences(
        want,
        have,
        true,
        &mut want_fetch,
        true,
        &mut have_fetch,
        &mut |node: &SharedIntrusive<SHAMapTreeNode>| {
            out.push(*node.get_hash().as_uint256());
            true
        },
    )
    .map_err(|error| format!("claim diff traversal failed: {error:?}"))?;
    Ok(out)
}

/// Build the [`ClaimDelta`] for `ledger` relative to the previously claimed
/// state root `claimed_state`.
///
/// - `new_state` / `dead_state`: the symmetric difference of the state trees,
///   so shared subtrees (unchanged accounts) are visited in neither direction.
///   For consecutive ledgers this touches only the few changed paths.
/// - `owned`: every node of this ledger's transaction tree. These are unique
///   to the ledger and die when it leaves the window.
///
/// `claimed_state` is the root of the last claimed state tree, either the
/// resident previous ledger's root or the persisted claimed root read back
/// from the store after a restart. Any traversal failure aborts the delta: the
/// caller skips this claim and the next ledger re-diffs across the gap.
pub fn compute_claim_delta(
    ledger: &Ledger,
    claimed_state: &SharedIntrusive<SHAMapTreeNode>,
    state_root_hash: Uint256,
    fetch: &ClaimNodeFetcher,
) -> Result<ClaimDelta, String> {
    let state_root = ledger.state_map().root();
    let new_state = collect_new_nodes(&state_root, Some(claimed_state), fetch)?;
    let dead_state = collect_new_nodes(claimed_state, Some(&state_root), fetch)?;
    let owned = collect_new_nodes(&ledger.tx_map().root(), None, fetch)?;
    Ok(ClaimDelta {
        seq: ledger.header().seq,
        state_root: state_root_hash,
        new_state,
        dead_state,
        owned,
    })
}

// Behavioural coverage for compute_claim_delta runs in the shamap-store
// integration tests, which build real state and transaction trees through the
// SHAMap family fixtures.
