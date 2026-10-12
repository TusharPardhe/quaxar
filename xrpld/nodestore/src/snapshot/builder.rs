//! Streaming bottom-up SHAMap construction from sorted leaves.
//!
//! A SHAMap is a 16-ary radix tree keyed by 256-bit hashes, and an inner
//! node's hash and stored bytes depend only on its 16 child hashes. So the
//! whole tree can be rebuilt from its leaves alone, which lets a snapshot omit
//! inner nodes entirely. The Ethereum `snap` protocol makes the same move:
//! "retrieve and verify all the account and storage data without downloading
//! intermediate Merkle trie nodes. The final state trie is reassembled
//! locally."
//!
//! Canonical shape (rippled `SHAMap::addGiveItem`): there is no path
//! compression. When two keys share `k` leading nibbles, an inner node exists at
//! every depth `0..=k`, and the two leaves hang under the inner node at depth
//! `k`. For sorted keys this means each leaf hangs at depth
//! `max(lcp(prev, key), lcp(key, next))`, and an inner node at depth `d` exists
//! exactly where at least two keys share a `d`-nibble prefix.
//!
//! The builder keeps one open inner node per depth along the current key's
//! path (at most 64), so memory is O(depth) regardless of the number of
//! leaves, the same idea as bulk-loading a B+-tree bottom-up from sorted input.
//! A node is emitted as soon as no later key can fall beneath it.
//!
//! Hashing follows quaxar's (and rippled's) node encoding exactly: a node's
//! hash is the first 32 bytes of SHA-512 over its prefix-serialized bytes,
//! `prefix || data || key` for a leaf and `prefix || 16 child hashes` for an
//! inner node (empty branches as zero). Leaves are prepared independently of
//! the tree ([`prepare_leaf`]) so their hashing can run in parallel.
//!
//! Every node is keyed by its own recomputed content hash, so any corrupted or
//! substituted leaf changes the rebuilt root. Comparing the rebuilt root with
//! the ledger header's `account_hash` / `tx_hash` therefore verifies the whole
//! tree in the same pass that builds it.

use basics::base_uint::Uint256;
use sha2::{Digest, Sha512};
use shamap::nodes::tree_node::{BRANCH_FACTOR, SHAMapNodeType};

/// Leaf depth in nibbles (64 nibbles = 256 bits).
const LEAF_DEPTH: usize = 64;

/// Smallest SHAMap item payload (mirrors the private `MIN_SHAMAP_ITEM_BYTES`
/// the SHAMap leaf constructor asserts).
const MIN_ITEM_BYTES: usize = 12;

/// Hash prefixes (big-endian `u32`), identical to `xrpl/shamap` tree_node.rs.
const HASH_PREFIX_INNER_NODE: u32 = 0x4D49_4E00; // "MIN\0"
const HASH_PREFIX_LEAF_NODE: u32 = 0x4D4C_4E00; // "MLN\0" account state
const HASH_PREFIX_TX_NODE: u32 = 0x534E_4400; // "SND\0" transaction + metadata

/// A node produced by the rebuild: content hash and prefix-serialized bytes
/// exactly as the NodeStore stores them.
pub struct BuiltNode {
    pub hash: Uint256,
    pub data: Vec<u8>,
}

fn sha512_half(bytes: &[u8]) -> Uint256 {
    let digest = Sha512::digest(bytes);
    let mut out = [0u8; 32];
    out.copy_from_slice(&digest[..32]);
    Uint256::from_array(out)
}

/// Serialize and hash one leaf. Independent of tree position, so callers may
/// run it in parallel. Supports the two leaf kinds a ledger holds.
pub fn prepare_leaf(
    leaf_type: SHAMapNodeType,
    key: &Uint256,
    data: &[u8],
) -> Result<BuiltNode, String> {
    let prefix = match leaf_type {
        SHAMapNodeType::AccountState => HASH_PREFIX_LEAF_NODE,
        SHAMapNodeType::TransactionMd => HASH_PREFIX_TX_NODE,
        other => return Err(format!("unsupported snapshot leaf type {other:?}")),
    };
    if data.len() < MIN_ITEM_BYTES {
        return Err(format!("leaf {key} payload below minimum size"));
    }
    let mut bytes = Vec::with_capacity(4 + data.len() + 32);
    bytes.extend_from_slice(&prefix.to_be_bytes());
    bytes.extend_from_slice(data);
    bytes.extend_from_slice(key.data());
    Ok(BuiltNode {
        hash: sha512_half(&bytes),
        data: bytes,
    })
}

fn inner_node(children: &[Uint256; BRANCH_FACTOR]) -> BuiltNode {
    let mut bytes = Vec::with_capacity(4 + BRANCH_FACTOR * 32);
    bytes.extend_from_slice(&HASH_PREFIX_INNER_NODE.to_be_bytes());
    for child in children {
        bytes.extend_from_slice(child.data());
    }
    BuiltNode {
        hash: sha512_half(&bytes),
        data: bytes,
    }
}

fn nibble(key: &Uint256, depth: usize) -> usize {
    let byte = key.data()[depth / 2] as usize;
    if depth % 2 == 0 { byte >> 4 } else { byte & 0x0F }
}

/// Number of leading nibbles two distinct keys share (always < 64).
fn common_prefix(a: &Uint256, b: &Uint256) -> usize {
    let (a, b) = (a.data(), b.data());
    for i in 0..32 {
        let x = a[i] ^ b[i];
        if x != 0 {
            return i * 2 + usize::from(x & 0xF0 == 0);
        }
    }
    LEAF_DEPTH
}

/// One open inner node on the current path. `path_key` is any key beneath it,
/// used to pick its branch in the parent when it closes.
struct Open {
    path_key: Uint256,
    children: [Uint256; BRANCH_FACTOR],
}

/// Builds the inner nodes of a SHAMap from leaf (key, hash) pairs supplied in
/// strictly ascending key order. Leaf nodes themselves come from
/// [`prepare_leaf`]; the builder emits only inner nodes.
pub struct SortedLeafBuilder {
    /// Open inner nodes; `path[d]` is the open node at depth `d`.
    path: Vec<Open>,
    /// The most recent leaf, waiting for the next key to fix its depth.
    pending: Option<(Uint256, Uint256)>, // (key, leaf hash)
    /// lcp between the pending leaf and the leaf before it.
    pending_prev_lcp: usize,
    leaves: u64,
    inners: u64,
}

impl Default for SortedLeafBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl SortedLeafBuilder {
    pub fn new() -> Self {
        Self {
            path: Vec::with_capacity(LEAF_DEPTH),
            pending: None,
            pending_prev_lcp: 0,
            leaves: 0,
            inners: 0,
        }
    }

    pub fn leaves(&self) -> u64 {
        self.leaves
    }

    pub fn inners(&self) -> u64 {
        self.inners
    }

    /// Add the next leaf by key and leaf hash. `emit` receives each inner node
    /// once it is final.
    pub fn push(
        &mut self,
        key: Uint256,
        leaf_hash: Uint256,
        emit: &mut dyn FnMut(BuiltNode) -> Result<(), String>,
    ) -> Result<(), String> {
        let lcp_with_prev = match self.pending {
            Some((prev, _)) if key <= prev => {
                return Err(format!("leaf keys not strictly ascending at {key}"));
            }
            Some((prev, _)) => common_prefix(&prev, &key),
            None => 0,
        };
        self.leaves += 1;
        if let Some((prev_key, prev_hash)) = self.pending.take() {
            // The pending leaf hangs at the deeper of its shared prefix with the
            // leaf before it and with this one.
            let depth = self.pending_prev_lcp.max(lcp_with_prev);
            self.place_leaf(prev_key, prev_hash, depth);
            // Nodes deeper than the split from the new key can get no more
            // children; what remains open lies on the new key's path.
            self.close_deeper_than(lcp_with_prev, emit)?;
        }
        self.pending = Some((key, leaf_hash));
        self.pending_prev_lcp = lcp_with_prev;
        Ok(())
    }

    /// Open inner nodes down to `depth` along `key`'s path and store the leaf
    /// hash in the depth-`depth` node. Every node already open lies on `key`'s
    /// path (maintained by `push`) and is never deeper than `depth`.
    fn place_leaf(&mut self, key: Uint256, leaf_hash: Uint256, depth: usize) {
        debug_assert!(self.path.len() <= depth + 1);
        while self.path.len() <= depth {
            self.path.push(Open {
                path_key: key,
                children: [Uint256::default(); BRANCH_FACTOR],
            });
        }
        let branch = nibble(&key, depth);
        debug_assert!(self.path[depth].children[branch].is_zero());
        self.path[depth].children[branch] = leaf_hash;
    }

    /// Emit open inner nodes deeper than `depth`, attaching each to its parent
    /// at the branch its path key takes at the parent's depth.
    fn close_deeper_than(
        &mut self,
        depth: usize,
        emit: &mut dyn FnMut(BuiltNode) -> Result<(), String>,
    ) -> Result<(), String> {
        while self.path.len() > depth + 1 {
            let open = self.path.pop().expect("non-empty path");
            let node = inner_node(&open.children);
            let hash = node.hash;
            emit(node)?;
            self.inners += 1;
            let parent_depth = self.path.len() - 1;
            let branch = nibble(&open.path_key, parent_depth);
            let parent = self.path.last_mut().expect("closed node has a parent");
            debug_assert!(parent.children[branch].is_zero());
            parent.children[branch] = hash;
        }
        Ok(())
    }

    /// Finish the tree and return its root hash (zero for an empty map).
    pub fn finish(
        mut self,
        emit: &mut dyn FnMut(BuiltNode) -> Result<(), String>,
    ) -> Result<Uint256, String> {
        let Some((key, leaf_hash)) = self.pending.take() else {
            return Ok(Uint256::default());
        };
        // The last leaf hangs at its shared prefix with the leaf before it (a
        // single-leaf map hangs it directly under the depth-0 root).
        self.place_leaf(key, leaf_hash, self.pending_prev_lcp);
        self.close_deeper_than(0, emit)?;
        let root = self.path.pop().expect("root is open");
        debug_assert!(self.path.is_empty());
        let node = inner_node(&root.children);
        let hash = node.hash;
        emit(node)?;
        self.inners += 1;
        Ok(hash)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use basics::intrusive_pointer::SharedIntrusive;
    use shamap::nodes::item::SHAMapItem;
    use shamap::nodes::tree_node::SHAMapTreeNode;
    use shamap::operations::mutation::add_item;
    use std::collections::BTreeMap;

    fn key(seed: u64) -> Uint256 {
        // splitmix64 spread into 32 bytes: uniform, distinct keys.
        let mut bytes = [0u8; 32];
        let mut z = seed;
        for chunk in bytes.chunks_mut(8) {
            z = z.wrapping_add(0x9E37_79B9_7F4A_7C15);
            let mut x = z;
            x = (x ^ (x >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
            x = (x ^ (x >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
            chunk.copy_from_slice(&(x ^ (x >> 31)).to_be_bytes());
        }
        Uint256::from_array(bytes)
    }

    /// Post-order rehash: `add_item` leaves ancestor hashes dirty.
    fn rehash(node: &SharedIntrusive<SHAMapTreeNode>) {
        if !node.is_inner() {
            return;
        }
        for branch in 0..BRANCH_FACTOR {
            if let Some(child) = node.get_child(branch) {
                rehash(&child);
            }
        }
        node.update_hash_deep();
    }

    /// Every node in the reference tree, as (hash, serialized bytes).
    fn collect(node: &SharedIntrusive<SHAMapTreeNode>, out: &mut BTreeMap<Uint256, Vec<u8>>) {
        out.insert(
            *node.get_hash().as_uint256(),
            node.serialize_with_prefix().expect("serialize"),
        );
        if node.is_inner() {
            for branch in 0..BRANCH_FACTOR {
                if let Some(child) = node.get_child(branch) {
                    collect(&child, out);
                }
            }
        }
    }

    /// Root hash and every node from quaxar's own SHAMap insert path.
    fn reference(
        leaves: &[(Uint256, Vec<u8>)],
        ty: SHAMapNodeType,
    ) -> (Uint256, BTreeMap<Uint256, Vec<u8>>) {
        let root = SHAMapTreeNode::new_inner(1);
        for (k, v) in leaves {
            add_item(&root, ty, SHAMapItem::new(*k, v.clone())).expect("insert");
        }
        rehash(&root);
        let mut nodes = BTreeMap::new();
        collect(&root, &mut nodes);
        (*root.get_hash().as_uint256(), nodes)
    }

    fn build(
        leaves: &[(Uint256, Vec<u8>)],
        ty: SHAMapNodeType,
    ) -> (Uint256, BTreeMap<Uint256, Vec<u8>>, SortedLeafBuilderCounts) {
        let mut sorted = leaves.to_vec();
        sorted.sort_by(|a, b| a.0.cmp(&b.0));
        let mut nodes = BTreeMap::new();
        let mut b = SortedLeafBuilder::new();
        for (k, v) in &sorted {
            let leaf = prepare_leaf(ty, k, v).expect("leaf");
            let hash = leaf.hash;
            nodes.insert(leaf.hash, leaf.data);
            b.push(*k, hash, &mut |n| {
                nodes.insert(n.hash, n.data);
                Ok(())
            })
            .expect("push");
        }
        let (l, _) = (b.leaves(), b.inners());
        let mut inner_count = 0;
        let root = {
            let mut emit = |n: BuiltNode| {
                nodes.insert(n.hash, n.data);
                Ok(())
            };
            let before = b.inners();
            let r = b.finish(&mut emit).expect("finish");
            inner_count += before;
            r
        };
        (root, nodes, SortedLeafBuilderCounts { leaves: l, _inners: inner_count })
    }

    struct SortedLeafBuilderCounts {
        leaves: u64,
        _inners: u64,
    }

    fn leaves(n: u64) -> Vec<(Uint256, Vec<u8>)> {
        (0..n)
            .map(|i| (key(i), vec![(i % 251) as u8; 20 + (i % 7) as usize]))
            .collect()
    }

    fn assert_matches_reference(input: &[(Uint256, Vec<u8>)], ty: SHAMapNodeType) {
        let (want_root, want_nodes) = reference(input, ty);
        let (got_root, got_nodes, counts) = build(input, ty);
        assert_eq!(got_root, want_root, "rebuilt root must equal SHAMap root");
        assert_eq!(got_nodes, want_nodes, "every node, byte for byte");
        assert_eq!(counts.leaves, input.len() as u64);
    }

    #[test]
    fn single_leaf_map_matches_shamap() {
        assert_matches_reference(&leaves(1), SHAMapNodeType::AccountState);
    }

    #[test]
    fn random_maps_match_shamap_root_and_all_nodes() {
        for n in [2u64, 3, 17, 256, 5_000] {
            assert_matches_reference(&leaves(n), SHAMapNodeType::AccountState);
        }
        assert_matches_reference(&leaves(300), SHAMapNodeType::TransactionMd);
    }

    #[test]
    fn deep_shared_prefixes_build_full_inner_chains() {
        // Keys that agree on many leading nibbles force long inner-node chains
        // (rippled has no path compression).
        let mut input = Vec::new();
        for i in 0..40u8 {
            let mut bytes = [0xABu8; 32];
            bytes[20] = i / 8;
            bytes[31] = i;
            input.push((Uint256::from_array(bytes), vec![i; 16]));
        }
        assert_matches_reference(&input, SHAMapNodeType::AccountState);
    }

    #[test]
    fn empty_map_has_zero_root() {
        let mut emit = |_: BuiltNode| Ok(());
        let root = SortedLeafBuilder::new().finish(&mut emit).unwrap();
        assert!(root.is_zero());
    }

    #[test]
    fn unsorted_duplicate_or_short_leaves_are_rejected_not_panicked() {
        let mut emit = |_: BuiltNode| Ok(());
        let (lo, hi) = (key(1).min(key(2)), key(1).max(key(2)));
        let mut b = SortedLeafBuilder::new();
        b.push(hi, key(9), &mut emit).unwrap();
        assert!(b.push(lo, key(9), &mut emit).is_err(), "descending");

        let mut b = SortedLeafBuilder::new();
        b.push(key(5), key(9), &mut emit).unwrap();
        assert!(b.push(key(5), key(9), &mut emit).is_err(), "duplicate");

        assert!(prepare_leaf(SHAMapNodeType::AccountState, &key(7), &[1; 3]).is_err());
        assert!(prepare_leaf(SHAMapNodeType::Inner, &key(7), &[1; 16]).is_err());
    }
}
