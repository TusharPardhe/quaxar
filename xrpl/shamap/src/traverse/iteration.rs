//! Current `SHAMap` ordered leaf walk helpers.

use crate::family::{MissingNodeReporter, SHAMapFamily, SHAMapNodeFetcher};
use crate::node_id::{SHAMapNodeId, select_branch};
use crate::search::{
    NodePathEntry, walk_towards_key_with_path, walk_towards_key_with_path_and_family,
};
use crate::traversal::{TraversalError, descend_throw, descend_throw_with_family};
use crate::tree_node::{BRANCH_FACTOR, SHAMapTreeNode};
use basics::base_uint::Uint256;
use basics::intrusive_pointer::SharedIntrusive;
use basics::sha_map_hash::SHAMapHash;
use basics::tagged_cache::CacheClock;
use std::hash::BuildHasher;

pub fn peek_first_item<F>(
    root: &SharedIntrusive<SHAMapTreeNode>,
    stack: &mut Vec<NodePathEntry>,
    backed: bool,
    fetch: &mut F,
) -> Result<Option<SharedIntrusive<SHAMapTreeNode>>, TraversalError>
where
    F: FnMut(SHAMapHash) -> Option<SharedIntrusive<SHAMapTreeNode>>,
{
    stack.clear();
    let leaf = first_below(root, SHAMapNodeId::default(), stack, backed, fetch)?;
    if leaf.is_none() {
        stack.clear();
    }
    Ok(leaf)
}

pub fn peek_first_item_with_family<CLOCK, S, FB, F, MR, NS>(
    root: &SharedIntrusive<SHAMapTreeNode>,
    stack: &mut Vec<NodePathEntry>,
    backed: bool,
    ledger_seq: u32,
    family: &SHAMapFamily<CLOCK, S, FB, F, MR, NS>,
) -> Result<Option<SharedIntrusive<SHAMapTreeNode>>, TraversalError>
where
    CLOCK: CacheClock,
    S: BuildHasher + Clone,
    F: SHAMapNodeFetcher,
    MR: MissingNodeReporter,
{
    stack.clear();
    let leaf = first_below_with_family(
        root,
        SHAMapNodeId::default(),
        stack,
        backed,
        ledger_seq,
        family,
    )?;
    if leaf.is_none() {
        stack.clear();
    }
    Ok(leaf)
}

pub fn peek_next_item<F>(
    id: Uint256,
    stack: &mut Vec<NodePathEntry>,
    backed: bool,
    fetch: &mut F,
) -> Result<Option<SharedIntrusive<SHAMapTreeNode>>, TraversalError>
where
    F: FnMut(SHAMapHash) -> Option<SharedIntrusive<SHAMapTreeNode>>,
{
    assert!(
        !stack.is_empty(),
        "peek_next_item requires a non-empty stack from peek_first_item"
    );
    assert!(
        stack
            .last()
            .expect("stack was checked as non-empty")
            .node
            .is_leaf(),
        "peek_next_item expects the stack to end with a leaf"
    );

    stack.pop();
    while let Some(entry) = stack.last().cloned() {
        assert!(
            entry.node.is_inner(),
            "non-leaf stack entries must be inner"
        );
        let start_branch = select_branch(entry.node_id, id) + 1;
        for branch in start_branch..BRANCH_FACTOR {
            if entry.node.is_empty_branch(branch) {
                continue;
            }

            let child = descend_throw(&entry.node, branch, backed, fetch)?
                .expect("non-empty branches should resolve to a child or error");
            let child_id = entry
                .node_id
                .get_child_node_id(branch)
                .expect("branch selection must stay within SHAMap depth bounds");
            return first_below(&child, child_id, stack, backed, fetch);
        }
        stack.pop();
    }

    Ok(None)
}

pub fn peek_next_item_with_family<CLOCK, S, FB, F, MR, NS>(
    id: Uint256,
    stack: &mut Vec<NodePathEntry>,
    backed: bool,
    ledger_seq: u32,
    family: &SHAMapFamily<CLOCK, S, FB, F, MR, NS>,
) -> Result<Option<SharedIntrusive<SHAMapTreeNode>>, TraversalError>
where
    CLOCK: CacheClock,
    S: BuildHasher + Clone,
    F: SHAMapNodeFetcher,
    MR: MissingNodeReporter,
{
    assert!(
        !stack.is_empty(),
        "peek_next_item requires a non-empty stack from peek_first_item"
    );
    assert!(
        stack
            .last()
            .expect("stack was checked as non-empty")
            .node
            .is_leaf(),
        "peek_next_item expects the stack to end with a leaf"
    );

    stack.pop();
    while let Some(entry) = stack.last().cloned() {
        assert!(
            entry.node.is_inner(),
            "non-leaf stack entries must be inner"
        );
        let start_branch = select_branch(entry.node_id, id) + 1;
        for branch in start_branch..BRANCH_FACTOR {
            if entry.node.is_empty_branch(branch) {
                continue;
            }

            let child = descend_throw_with_family(&entry.node, branch, backed, ledger_seq, family)?
                .expect("non-empty branches should resolve to a child or error");
            let child_id = entry
                .node_id
                .get_child_node_id(branch)
                .expect("branch selection must stay within SHAMap depth bounds");
            return first_below_with_family(&child, child_id, stack, backed, ledger_seq, family);
        }
        stack.pop();
    }

    Ok(None)
}

pub fn upper_bound<F>(
    root: &SharedIntrusive<SHAMapTreeNode>,
    id: Uint256,
    backed: bool,
    fetch: &mut F,
) -> Result<Option<SharedIntrusive<SHAMapTreeNode>>, TraversalError>
where
    F: FnMut(SHAMapHash) -> Option<SharedIntrusive<SHAMapTreeNode>>,
{
    let (_, mut stack) = walk_towards_key_with_path(root, id, backed, fetch)?;
    while let Some(entry) = stack.last().cloned() {
        if entry.node.is_leaf() {
            if entry.node.peek_item().is_some_and(|item| item.key() > id) {
                return Ok(Some(entry.node));
            }
        } else {
            let start_branch = select_branch(entry.node_id, id) + 1;
            for branch in start_branch..BRANCH_FACTOR {
                if entry.node.is_empty_branch(branch) {
                    continue;
                }

                let child = descend_throw(&entry.node, branch, backed, fetch)?
                    .expect("non-empty branches should resolve to a child or error");
                let child_id = entry
                    .node_id
                    .get_child_node_id(branch)
                    .expect("branch selection must stay within SHAMap depth bounds");
                let mut below_stack = Vec::new();
                return first_below(&child, child_id, &mut below_stack, backed, fetch);
            }
        }
        stack.pop();
    }

    Ok(None)
}

pub fn upper_bound_with_family<CLOCK, S, FB, F, MR, NS>(
    root: &SharedIntrusive<SHAMapTreeNode>,
    id: Uint256,
    backed: bool,
    ledger_seq: u32,
    family: &SHAMapFamily<CLOCK, S, FB, F, MR, NS>,
) -> Result<Option<SharedIntrusive<SHAMapTreeNode>>, TraversalError>
where
    CLOCK: CacheClock,
    S: BuildHasher + Clone,
    F: SHAMapNodeFetcher,
    MR: MissingNodeReporter,
{
    let (_, mut stack) =
        walk_towards_key_with_path_and_family(root, id, backed, ledger_seq, family)?;
    while let Some(entry) = stack.last().cloned() {
        if entry.node.is_leaf() {
            if entry.node.peek_item().is_some_and(|item| item.key() > id) {
                return Ok(Some(entry.node));
            }
        } else {
            let start_branch = select_branch(entry.node_id, id) + 1;
            for branch in start_branch..BRANCH_FACTOR {
                if entry.node.is_empty_branch(branch) {
                    continue;
                }

                let child =
                    descend_throw_with_family(&entry.node, branch, backed, ledger_seq, family)?
                        .expect("non-empty branches should resolve to a child or error");
                let child_id = entry
                    .node_id
                    .get_child_node_id(branch)
                    .expect("branch selection must stay within SHAMap depth bounds");
                let mut below_stack = Vec::new();
                return first_below_with_family(
                    &child,
                    child_id,
                    &mut below_stack,
                    backed,
                    ledger_seq,
                    family,
                );
            }
        }
        stack.pop();
    }

    Ok(None)
}

pub fn lower_bound<F>(
    root: &SharedIntrusive<SHAMapTreeNode>,
    id: Uint256,
    backed: bool,
    fetch: &mut F,
) -> Result<Option<SharedIntrusive<SHAMapTreeNode>>, TraversalError>
where
    F: FnMut(SHAMapHash) -> Option<SharedIntrusive<SHAMapTreeNode>>,
{
    let (_, mut stack) = walk_towards_key_with_path(root, id, backed, fetch)?;
    while let Some(entry) = stack.last().cloned() {
        if entry.node.is_leaf() {
            if entry.node.peek_item().is_some_and(|item| item.key() < id) {
                return Ok(Some(entry.node));
            }
        } else {
            let start_branch = select_branch(entry.node_id, id);
            for branch in (0..start_branch).rev() {
                if entry.node.is_empty_branch(branch) {
                    continue;
                }

                let child = descend_throw(&entry.node, branch, backed, fetch)?
                    .expect("non-empty branches should resolve to a child or error");
                let child_id = entry
                    .node_id
                    .get_child_node_id(branch)
                    .expect("branch selection must stay within SHAMap depth bounds");
                let mut below_stack = Vec::new();
                return last_below(&child, child_id, &mut below_stack, backed, fetch);
            }
        }
        stack.pop();
    }

    Ok(None)
}

pub fn lower_bound_with_family<CLOCK, S, FB, F, MR, NS>(
    root: &SharedIntrusive<SHAMapTreeNode>,
    id: Uint256,
    backed: bool,
    ledger_seq: u32,
    family: &SHAMapFamily<CLOCK, S, FB, F, MR, NS>,
) -> Result<Option<SharedIntrusive<SHAMapTreeNode>>, TraversalError>
where
    CLOCK: CacheClock,
    S: BuildHasher + Clone,
    F: SHAMapNodeFetcher,
    MR: MissingNodeReporter,
{
    let (_, mut stack) =
        walk_towards_key_with_path_and_family(root, id, backed, ledger_seq, family)?;
    while let Some(entry) = stack.last().cloned() {
        if entry.node.is_leaf() {
            if entry.node.peek_item().is_some_and(|item| item.key() < id) {
                return Ok(Some(entry.node));
            }
        } else {
            let start_branch = select_branch(entry.node_id, id);
            for branch in (0..start_branch).rev() {
                if entry.node.is_empty_branch(branch) {
                    continue;
                }

                let child =
                    descend_throw_with_family(&entry.node, branch, backed, ledger_seq, family)?
                        .expect("non-empty branches should resolve to a child or error");
                let child_id = entry
                    .node_id
                    .get_child_node_id(branch)
                    .expect("branch selection must stay within SHAMap depth bounds");
                let mut below_stack = Vec::new();
                return last_below_with_family(
                    &child,
                    child_id,
                    &mut below_stack,
                    backed,
                    ledger_seq,
                    family,
                );
            }
        }
        stack.pop();
    }

    Ok(None)
}

fn first_below<F>(
    root: &SharedIntrusive<SHAMapTreeNode>,
    root_id: SHAMapNodeId,
    stack: &mut Vec<NodePathEntry>,
    backed: bool,
    fetch: &mut F,
) -> Result<Option<SharedIntrusive<SHAMapTreeNode>>, TraversalError>
where
    F: FnMut(SHAMapHash) -> Option<SharedIntrusive<SHAMapTreeNode>>,
{
    extreme_below(root, root_id, stack, backed, fetch, true)
}

fn first_below_with_family<CLOCK, S, FB, F, MR, NS>(
    root: &SharedIntrusive<SHAMapTreeNode>,
    root_id: SHAMapNodeId,
    stack: &mut Vec<NodePathEntry>,
    backed: bool,
    ledger_seq: u32,
    family: &SHAMapFamily<CLOCK, S, FB, F, MR, NS>,
) -> Result<Option<SharedIntrusive<SHAMapTreeNode>>, TraversalError>
where
    CLOCK: CacheClock,
    S: BuildHasher + Clone,
    F: SHAMapNodeFetcher,
    MR: MissingNodeReporter,
{
    extreme_below_with_family(root, root_id, stack, backed, ledger_seq, family, true)
}

fn last_below<F>(
    root: &SharedIntrusive<SHAMapTreeNode>,
    root_id: SHAMapNodeId,
    stack: &mut Vec<NodePathEntry>,
    backed: bool,
    fetch: &mut F,
) -> Result<Option<SharedIntrusive<SHAMapTreeNode>>, TraversalError>
where
    F: FnMut(SHAMapHash) -> Option<SharedIntrusive<SHAMapTreeNode>>,
{
    extreme_below(root, root_id, stack, backed, fetch, false)
}

fn last_below_with_family<CLOCK, S, FB, F, MR, NS>(
    root: &SharedIntrusive<SHAMapTreeNode>,
    root_id: SHAMapNodeId,
    stack: &mut Vec<NodePathEntry>,
    backed: bool,
    ledger_seq: u32,
    family: &SHAMapFamily<CLOCK, S, FB, F, MR, NS>,
) -> Result<Option<SharedIntrusive<SHAMapTreeNode>>, TraversalError>
where
    CLOCK: CacheClock,
    S: BuildHasher + Clone,
    F: SHAMapNodeFetcher,
    MR: MissingNodeReporter,
{
    extreme_below_with_family(root, root_id, stack, backed, ledger_seq, family, false)
}

fn extreme_below<F>(
    root: &SharedIntrusive<SHAMapTreeNode>,
    root_id: SHAMapNodeId,
    stack: &mut Vec<NodePathEntry>,
    backed: bool,
    fetch: &mut F,
    forward: bool,
) -> Result<Option<SharedIntrusive<SHAMapTreeNode>>, TraversalError>
where
    F: FnMut(SHAMapHash) -> Option<SharedIntrusive<SHAMapTreeNode>>,
{
    let mut node = root.clone();
    let mut node_id = root_id;

    loop {
        stack.push(NodePathEntry {
            node: node.clone(),
            node_id,
        });

        if node.is_leaf() {
            return Ok(Some(node));
        }

        let next = if forward {
            next_child_in_order(&node, node_id, backed, fetch)?
        } else {
            previous_child_in_order(&node, node_id, backed, fetch)?
        };
        let Some((child, child_id)) = next else {
            return Ok(None);
        };
        node = child;
        node_id = child_id;
    }
}

fn extreme_below_with_family<CLOCK, S, FB, F, MR, NS>(
    root: &SharedIntrusive<SHAMapTreeNode>,
    root_id: SHAMapNodeId,
    stack: &mut Vec<NodePathEntry>,
    backed: bool,
    ledger_seq: u32,
    family: &SHAMapFamily<CLOCK, S, FB, F, MR, NS>,
    forward: bool,
) -> Result<Option<SharedIntrusive<SHAMapTreeNode>>, TraversalError>
where
    CLOCK: CacheClock,
    S: BuildHasher + Clone,
    F: SHAMapNodeFetcher,
    MR: MissingNodeReporter,
{
    let mut node = root.clone();
    let mut node_id = root_id;

    loop {
        stack.push(NodePathEntry {
            node: node.clone(),
            node_id,
        });

        if node.is_leaf() {
            return Ok(Some(node));
        }

        let next = if forward {
            next_child_in_order_with_family(&node, node_id, backed, ledger_seq, family)?
        } else {
            previous_child_in_order_with_family(&node, node_id, backed, ledger_seq, family)?
        };
        let Some((child, child_id)) = next else {
            return Ok(None);
        };
        node = child;
        node_id = child_id;
    }
}

fn next_child_in_order<F>(
    node: &SharedIntrusive<SHAMapTreeNode>,
    node_id: SHAMapNodeId,
    backed: bool,
    fetch: &mut F,
) -> Result<Option<(SharedIntrusive<SHAMapTreeNode>, SHAMapNodeId)>, TraversalError>
where
    F: FnMut(SHAMapHash) -> Option<SharedIntrusive<SHAMapTreeNode>>,
{
    for branch in 0..BRANCH_FACTOR {
        if node.is_empty_branch(branch) {
            continue;
        }

        let child = descend_throw(node, branch, backed, fetch)?
            .expect("non-empty branches should resolve to a child or error");
        let child_id = node_id
            .get_child_node_id(branch)
            .expect("branch selection must stay within SHAMap depth bounds");
        return Ok(Some((child, child_id)));
    }

    Ok(None)
}

fn next_child_in_order_with_family<CLOCK, S, FB, F, MR, NS>(
    node: &SharedIntrusive<SHAMapTreeNode>,
    node_id: SHAMapNodeId,
    backed: bool,
    ledger_seq: u32,
    family: &SHAMapFamily<CLOCK, S, FB, F, MR, NS>,
) -> Result<Option<(SharedIntrusive<SHAMapTreeNode>, SHAMapNodeId)>, TraversalError>
where
    CLOCK: CacheClock,
    S: BuildHasher + Clone,
    F: SHAMapNodeFetcher,
    MR: MissingNodeReporter,
{
    for branch in 0..BRANCH_FACTOR {
        if node.is_empty_branch(branch) {
            continue;
        }

        let child = descend_throw_with_family(node, branch, backed, ledger_seq, family)?
            .expect("non-empty branches should resolve to a child or error");
        let child_id = node_id
            .get_child_node_id(branch)
            .expect("branch selection must stay within SHAMap depth bounds");
        return Ok(Some((child, child_id)));
    }

    Ok(None)
}

fn previous_child_in_order<F>(
    node: &SharedIntrusive<SHAMapTreeNode>,
    node_id: SHAMapNodeId,
    backed: bool,
    fetch: &mut F,
) -> Result<Option<(SharedIntrusive<SHAMapTreeNode>, SHAMapNodeId)>, TraversalError>
where
    F: FnMut(SHAMapHash) -> Option<SharedIntrusive<SHAMapTreeNode>>,
{
    for branch in (0..BRANCH_FACTOR).rev() {
        if node.is_empty_branch(branch) {
            continue;
        }

        let child = descend_throw(node, branch, backed, fetch)?
            .expect("non-empty branches should resolve to a child or error");
        let child_id = node_id
            .get_child_node_id(branch)
            .expect("branch selection must stay within SHAMap depth bounds");
        return Ok(Some((child, child_id)));
    }

    Ok(None)
}

fn previous_child_in_order_with_family<CLOCK, S, FB, F, MR, NS>(
    node: &SharedIntrusive<SHAMapTreeNode>,
    node_id: SHAMapNodeId,
    backed: bool,
    ledger_seq: u32,
    family: &SHAMapFamily<CLOCK, S, FB, F, MR, NS>,
) -> Result<Option<(SharedIntrusive<SHAMapTreeNode>, SHAMapNodeId)>, TraversalError>
where
    CLOCK: CacheClock,
    S: BuildHasher + Clone,
    F: SHAMapNodeFetcher,
    MR: MissingNodeReporter,
{
    for branch in (0..BRANCH_FACTOR).rev() {
        if node.is_empty_branch(branch) {
            continue;
        }

        let child = descend_throw_with_family(node, branch, backed, ledger_seq, family)?
            .expect("non-empty branches should resolve to a child or error");
        let child_id = node_id
            .get_child_node_id(branch)
            .expect("branch selection must stay within SHAMap depth bounds");
        return Ok(Some((child, child_id)));
    }

    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::{lower_bound, peek_first_item, peek_next_item, upper_bound};
    use crate::item::SHAMapItem;
    use crate::search::NodePathEntry;
    use crate::tree_node::{SHAMapNodeType, SHAMapTreeNode};
    use basics::base_uint::Uint256;
    use basics::intrusive_pointer::{SharedIntrusive, make_shared_intrusive};
    use basics::sha_map_hash::SHAMapHash;

    fn key(hex: &str) -> Uint256 {
        Uint256::from_hex(hex).expect("hex should parse")
    }

    fn sample_hash(fill: u8) -> SHAMapHash {
        SHAMapHash::new(Uint256::from_array([fill; 32]))
    }

    fn same_node(
        left: &SharedIntrusive<SHAMapTreeNode>,
        right: &SharedIntrusive<SHAMapTreeNode>,
    ) -> bool {
        std::ptr::eq(&**left, &**right)
    }

    type ThreeLeafRoot = (
        SharedIntrusive<SHAMapTreeNode>,
        SharedIntrusive<SHAMapTreeNode>,
        SharedIntrusive<SHAMapTreeNode>,
        SharedIntrusive<SHAMapTreeNode>,
        Uint256,
        Uint256,
        Uint256,
    );

    fn build_three_leaf_root() -> ThreeLeafRoot {
        let low_key = key("1000000000000000000000000000000000000000000000000000000000000000");
        let mid_key = key("4000000000000000000000000000000000000000000000000000000000000000");
        let high_key = key("9000000000000000000000000000000000000000000000000000000000000000");

        let low_leaf = make_shared_intrusive(SHAMapTreeNode::new_leaf(
            SHAMapNodeType::AccountState,
            SHAMapItem::new(low_key, vec![1; 12]),
            0,
        ));
        let mid_leaf = make_shared_intrusive(SHAMapTreeNode::new_leaf(
            SHAMapNodeType::AccountState,
            SHAMapItem::new(mid_key, vec![2; 12]),
            0,
        ));
        let high_leaf = make_shared_intrusive(SHAMapTreeNode::new_leaf(
            SHAMapNodeType::AccountState,
            SHAMapItem::new(high_key, vec![3; 12]),
            0,
        ));
        let root = make_shared_intrusive(SHAMapTreeNode::new_inner(1));
        root.set_child_hash(1, low_leaf.get_hash());
        root.share_child(1, &low_leaf);
        root.set_child_hash(4, mid_leaf.get_hash());
        root.share_child(4, &mid_leaf);
        root.set_child_hash(9, high_leaf.get_hash());
        root.share_child(9, &high_leaf);

        (
            root, low_leaf, mid_leaf, high_leaf, low_key, mid_key, high_key,
        )
    }

    #[test]
    fn peek_first_and_next_item_walk_loaded_leaves_in_order() {
        let (root, low_leaf, mid_leaf, high_leaf, low_key, mid_key, _) = build_three_leaf_root();
        let mut stack: Vec<NodePathEntry> = Vec::new();

        let first = peek_first_item(&root, &mut stack, false, &mut |_| None)
            .expect("first loaded leaf lookup should succeed")
            .expect("first leaf should exist");
        assert!(same_node(&first, &low_leaf));

        let second = peek_next_item(low_key, &mut stack, false, &mut |_| None)
            .expect("next leaf lookup should succeed")
            .expect("second leaf should exist");
        assert!(same_node(&second, &mid_leaf));

        let third = peek_next_item(mid_key, &mut stack, false, &mut |_| None)
            .expect("third leaf lookup should succeed")
            .expect("third leaf should exist");
        assert!(same_node(&third, &high_leaf));

        let end = peek_next_item(
            third
                .peek_item()
                .expect("third leaf should have an item")
                .key(),
            &mut stack,
            false,
            &mut |_| None,
        )
        .expect("iteration end should not error");
        assert!(end.is_none());
    }

    #[test]
    fn upper_and_lower_bound_match_current_cpp_roles() {
        let (root, _low_leaf, mid_leaf, high_leaf, low_key, _mid_key, high_key) =
            build_three_leaf_root();
        let between_mid_and_high =
            key("7000000000000000000000000000000000000000000000000000000000000000");

        let upper = upper_bound(&root, between_mid_and_high, false, &mut |_| None)
            .expect("upper bound should succeed")
            .expect("higher leaf should exist");
        assert!(same_node(&upper, &high_leaf));

        let lower = lower_bound(&root, between_mid_and_high, false, &mut |_| None)
            .expect("lower bound should succeed")
            .expect("lower leaf should exist");
        assert!(same_node(&lower, &mid_leaf));

        let exact_upper = upper_bound(&root, low_key, false, &mut |_| None)
            .expect("exact upper bound should succeed")
            .expect("next leaf should exist");
        assert!(same_node(&exact_upper, &mid_leaf));

        let exact_lower = lower_bound(&root, high_key, false, &mut |_| None)
            .expect("exact lower bound should succeed")
            .expect("previous leaf should exist");
        assert!(same_node(&exact_lower, &mid_leaf));

        let no_lower = lower_bound(&root, low_key, false, &mut |_| None)
            .expect("empty predecessor search should succeed");
        assert!(no_lower.is_none());
    }

    #[test]
    fn peek_first_item_fetches_missing_children_when_backed() {
        let key = key("2000000000000000000000000000000000000000000000000000000000000000");
        let leaf = make_shared_intrusive(SHAMapTreeNode::new_leaf_with_hash(
            SHAMapNodeType::AccountState,
            SHAMapItem::new(key, vec![9; 12]),
            0,
            sample_hash(0xAB),
        ));
        let root = make_shared_intrusive(SHAMapTreeNode::new_inner(1));
        root.set_child_hash(2, sample_hash(0xAB));

        let mut stack = Vec::new();
        let mut fetch_calls = 0;
        let first = peek_first_item(&root, &mut stack, true, &mut |_| {
            fetch_calls += 1;
            Some(leaf.clone())
        })
        .expect("backed first lookup should succeed")
        .expect("fetched first leaf should exist");

        assert!(same_node(&first, &leaf));
        assert_eq!(fetch_calls, 1);
        assert!(root.get_child(2).is_some());
    }
}
