//! Ancestry trie of ledgers.
//!
//! Ported from rippled's `LedgerTrie.h`. A compressed trie tree that tracks
//! validation support of recent ledgers based on their ancestry, used to
//! compute the network's "preferred" ledger when validators disagree.
//!
//! The reference implementation uses `std::unique_ptr<Node>` children with
//! raw `Node*` parent pointers. Rust's ownership model doesn't allow that
//! pattern safely, so this port uses an index-based arena: nodes live in a
//! single `Vec<Node<L>>`, and children/parent links are `usize` indices into
//! that vec. The algorithm itself — `find`, `insert`, `remove`,
//! `get_preferred` — is ported line-for-line; only the pointer plumbing
//! differs.

use std::collections::BTreeMap;

/// A ledger and its ancestry, as required by the trie.
///
/// Mirrors the reference's implicit `Ledger` concept: a lightweight,
/// cheaply-copyable type with a sequence number, an identity, and the
/// ability to look up an ancestor's id at any earlier sequence.
pub trait TrieLedger: Clone {
    type Seq: Copy + Ord + Default + std::ops::Add<u32, Output = Self::Seq> + std::ops::Sub<u32, Output = Self::Seq>;
    type Id: Copy + Eq + Ord + Default;

    /// The genesis ledger: prefixes all other ledgers, at `Seq` zero.
    fn genesis() -> Self;

    /// This ledger's sequence number.
    fn seq(&self) -> Self::Seq;

    /// The id of this ledger's ancestor at sequence `s`, or the zero id if
    /// unknown. `s` must be `<= self.seq()`.
    fn ancestor(&self, s: Self::Seq) -> Self::Id;

    /// The sequence number of the first ancestor at which `self` and
    /// `other` disagree (or the min of their sequences + 1 if they agree
    /// everywhere in common). Matches the reference's free function
    /// `mismatch(ledgerA, ledgerB)`.
    fn mismatch(&self, other: &Self) -> Self::Seq;
}

/// The tip of a [`Span`]: a specific ledger and enough of its ancestry to
/// answer `ancestor` queries. Matches the reference `SpanTip<Ledger>`.
#[derive(Debug, Clone)]
pub struct SpanTip<L: TrieLedger> {
    pub seq: L::Seq,
    pub id: L::Id,
    ledger: L,
}

impl<L: TrieLedger> SpanTip<L> {
    /// The id of an ancestor of this tip. `s` must be `<= self.seq`.
    pub fn ancestor(&self, s: L::Seq) -> L::Id {
        debug_assert!(s <= self.seq, "SpanTip::ancestor: s must not exceed seq");
        self.ledger.ancestor(s)
    }
}

/// A half-open span `[start, end)` of a ledger's ancestry.
/// Matches `ledger_trie_detail::Span<Ledger>`.
#[derive(Debug, Clone)]
struct Span<L: TrieLedger> {
    start: L::Seq,
    end: L::Seq,
    ledger: L,
}

impl<L: TrieLedger> Span<L> {
    fn genesis() -> Self {
        let ledger = L::genesis();
        let start = ledger.seq();
        Self {
            start,
            end: start + 1,
            ledger,
        }
    }

    fn from_ledger(ledger: L, zero: L::Seq) -> Self {
        let end = ledger.seq() + 1;
        Self {
            // Reference: `Seq start_{0}` is a default member initializer,
            // NOT the ledger's own seq -- a freshly constructed span always
            // starts at the trie's zero/genesis sequence and is narrowed by
            // `before`/`from` (`sub`) as insertion splits it.
            start: zero,
            end,
            ledger,
        }
    }

    fn clamp(&self, val: L::Seq) -> L::Seq {
        val.max(self.start).min(self.end)
    }

    fn sub(&self, from: L::Seq, to: L::Seq) -> Option<Self> {
        let new_from = self.clamp(from);
        let new_to = self.clamp(to);
        if new_from < new_to {
            Some(Self {
                start: new_from,
                end: new_to,
                ledger: self.ledger.clone(),
            })
        } else {
            None
        }
    }

    /// The span from `[spot, end)`. `from()`.
    fn from(&self, spot: L::Seq) -> Option<Self> {
        self.sub(spot, self.end)
    }

    /// The span from `[start, spot)`. `before()`.
    fn before(&self, spot: L::Seq) -> Option<Self> {
        self.sub(self.start, spot)
    }

    /// The id of the ledger that starts this span. `startID()`.
    fn start_id(&self) -> L::Id {
        self.ledger.ancestor(self.start)
    }

    /// The sequence of the first difference between this span's ledger and
    /// `other`, clamped to this span's bounds. `diff()`.
    fn diff(&self, other: &L) -> L::Seq {
        self.clamp(self.ledger.mismatch(other))
    }

    /// The tip of this span. `tip()`.
    fn tip(&self) -> SpanTip<L> {
        let tip_seq = self.end - 1;
        SpanTip {
            seq: tip_seq,
            id: self.ledger.ancestor(tip_seq),
            ledger: self.ledger.clone(),
        }
    }

    /// Merge two overlapping/adjacent spans, keeping the ledger from the
    /// higher-sequence span (which has the fuller ancestry). Matches the
    /// reference's `friend Span merge(a, b)`.
    fn merge(a: &Self, b: &Self) -> Self {
        if a.end < b.end {
            Self {
                start: a.start.min(b.start),
                end: b.end,
                ledger: b.ledger.clone(),
            }
        } else {
            Self {
                start: a.start.min(b.start),
                end: a.end,
                ledger: a.ledger.clone(),
            }
        }
    }
}

/// A node in the trie. Matches `ledger_trie_detail::Node<Ledger>`, with
/// `children`/`parent` stored as arena indices instead of owning pointers.
struct Node<L: TrieLedger> {
    span: Span<L>,
    tip_support: u32,
    branch_support: u32,
    children: Vec<usize>,
    parent: Option<usize>,
    /// Marks a node as logically removed from the arena. Since the arena
    /// never physically deallocates slots (indices must stay stable), a
    /// freed node is tombstoned here instead. `find`/`getPreferred` treat
    /// tombstoned nodes as absent by skipping them wherever the reference
    /// would have followed a now-dangling child pointer.
    live: bool,
}

impl<L: TrieLedger> Node<L> {
    fn root() -> Self {
        Self {
            span: Span::genesis(),
            tip_support: 0,
            branch_support: 0,
            children: Vec::new(),
            parent: None,
            live: true,
        }
    }

    /// Matches the reference's `explicit Node(Ledger const& l)` constructor.
    /// Kept for API completeness/parity even though, as in the reference,
    /// `insert`/`remove` build nodes via the `Span`-based constructor
    /// instead.
    #[allow(dead_code)]
    fn from_ledger(ledger: &L) -> Self {
        Self {
            span: Span::from_ledger(ledger.clone(), L::Seq::default()),
            tip_support: 1,
            branch_support: 1,
            children: Vec::new(),
            parent: None,
            live: true,
        }
    }

    fn from_span(span: Span<L>) -> Self {
        Self {
            span,
            tip_support: 0,
            branch_support: 0,
            children: Vec::new(),
            parent: None,
            live: true,
        }
    }
}

const ROOT: usize = 0;

/// Ancestry trie of ledgers. Matches `LedgerTrie<Ledger>`.
pub struct LedgerTrie<L: TrieLedger> {
    nodes: Vec<Node<L>>,
    /// Count of tip support for each sequence number. `seqSupport_`.
    seq_support: BTreeMap<L::Seq, u32>,
}

impl<L: TrieLedger> LedgerTrie<L> {
    pub fn new() -> Self {
        Self {
            nodes: vec![Node::root()],
            seq_support: BTreeMap::new(),
        }
    }

    fn alloc(&mut self, node: Node<L>) -> usize {
        self.nodes.push(node);
        self.nodes.len() - 1
    }

    fn erase_child(&mut self, parent: usize, child: usize) {
        self.nodes[parent].children.retain(|&c| c != child);
        self.nodes[child].live = false;
    }

    /// Find the node with the longest common ancestry with `ledger`.
    /// Returns `(node_index, diff_seq)`. Matches `find()`.
    fn find(&self, ledger: &L) -> (usize, L::Seq) {
        let mut curr = ROOT;
        let mut pos = self.nodes[curr].span.diff(ledger);
        let mut done = false;

        while !done && pos == self.nodes[curr].span.end {
            done = true;
            for &child in &self.nodes[curr].children {
                if !self.nodes[child].live {
                    continue;
                }
                let child_pos = self.nodes[child].span.diff(ledger);
                if child_pos > pos {
                    done = false;
                    pos = child_pos;
                    curr = child;
                    break;
                }
            }
        }
        (curr, pos)
    }

    /// Find the node with an exact ledger-id match to `ledger`'s tip.
    /// O(n). Matches `findByLedgerID()`.
    fn find_by_ledger_id(&self, ledger_id: L::Id) -> Option<usize> {
        self.nodes
            .iter()
            .enumerate()
            .find(|(idx, n)| n.live && (*idx == ROOT || n.tip_support > 0 || true) && n.span.tip().id == ledger_id)
            .map(|(idx, _)| idx)
    }

    /// Insert and/or increment support for `ledger`. Matches `insert()`.
    pub fn insert(&mut self, ledger: &L, count: u32) {
        let (loc, diff_seq) = self.find(ledger);

        let prefix = self.nodes[loc].span.before(diff_seq);
        let old_suffix = self.nodes[loc].span.from(diff_seq);
        let new_suffix = Span::from_ledger(ledger.clone(), L::Seq::default()).from(diff_seq);

        let mut inc_node = loc;

        if let Some(old_suffix) = old_suffix {
            // Have `abcdef -> ...`, inserting `abc`: becomes
            // `abc -> def -> ...`.
            let mut new_node = Node::from_span(old_suffix);
            new_node.tip_support = self.nodes[loc].tip_support;
            new_node.branch_support = self.nodes[loc].branch_support;
            new_node.children = std::mem::take(&mut self.nodes[loc].children);
            let new_node_idx = self.alloc(new_node);
            let moved_children = self.nodes[new_node_idx].children.clone();
            for child in moved_children {
                self.nodes[child].parent = Some(new_node_idx);
            }

            self.nodes[loc].span = prefix.expect("prefix must exist when old_suffix exists");
            self.nodes[new_node_idx].parent = Some(loc);
            self.nodes[loc].children.push(new_node_idx);
            self.nodes[loc].tip_support = 0;
        }

        if let Some(new_suffix) = new_suffix {
            // Have `abc -> ...`, inserting `abcdef`: becomes
            // `abc -> ...` with a new sibling branch `abc -> def`.
            let mut new_node = Node::from_span(new_suffix);
            new_node.parent = Some(loc);
            let new_node_idx = self.alloc(new_node);
            inc_node = new_node_idx;
            self.nodes[loc].children.push(new_node_idx);
        }

        self.nodes[inc_node].tip_support += count;
        let mut walk = Some(inc_node);
        while let Some(idx) = walk {
            self.nodes[idx].branch_support += count;
            walk = self.nodes[idx].parent;
        }

        *self.seq_support.entry(ledger.seq()).or_insert(0) += count;
    }

    /// Decrease support for `ledger`, removing and compressing nodes where
    /// possible. Returns whether a matching node was found and
    /// decremented. Matches `remove()`.
    pub fn remove(&mut self, ledger_id: L::Id, ledger_seq: L::Seq, count: u32) -> bool {
        let Some(mut loc) = self.find_by_ledger_id(ledger_id) else {
            return false;
        };
        if self.nodes[loc].tip_support == 0 {
            return false;
        }

        let count = count.min(self.nodes[loc].tip_support);
        self.nodes[loc].tip_support -= count;

        if let Some(support) = self.seq_support.get_mut(&ledger_seq) {
            *support -= count;
            if *support == 0 {
                self.seq_support.remove(&ledger_seq);
            }
        }

        let mut walk = Some(loc);
        while let Some(idx) = walk {
            self.nodes[idx].branch_support -= count;
            walk = self.nodes[idx].parent;
        }

        while self.nodes[loc].tip_support == 0 && loc != ROOT {
            let parent = self.nodes[loc].parent.expect("non-root node must have a parent");
            let live_children: Vec<usize> =
                self.nodes[loc].children.iter().copied().filter(|&c| self.nodes[c].live).collect();

            if live_children.is_empty() {
                self.erase_child(parent, loc);
            } else if live_children.len() == 1 {
                let child = live_children[0];
                let merged = Span::merge(&self.nodes[loc].span, &self.nodes[child].span);
                self.nodes[child].span = merged;
                self.nodes[child].parent = Some(parent);
                self.nodes[parent].children.push(child);
                self.erase_child(parent, loc);
            } else {
                break;
            }
            loc = parent;
        }
        true
    }

    /// Tip support for the exact ledger identified by `ledger_id`.
    /// Matches `tipSupport()`.
    pub fn tip_support(&self, ledger_id: L::Id) -> u32 {
        self.find_by_ledger_id(ledger_id).map(|idx| self.nodes[idx].tip_support).unwrap_or(0)
    }

    /// Branch support (this ledger or any descendant) for `ledger`.
    /// Matches `branchSupport()`.
    pub fn branch_support(&self, ledger: &L) -> u32 {
        if let Some(idx) = self.find_by_ledger_id(ledger.ancestor(ledger.seq())) {
            return self.nodes[idx].branch_support;
        }
        let (loc, diff_seq) = self.find(ledger);
        if diff_seq > ledger.seq() && ledger.seq() < self.nodes[loc].span.end {
            self.nodes[loc].branch_support
        } else {
            0
        }
    }

    /// Whether the trie is tracking any ledgers. Matches `empty()`.
    pub fn is_empty(&self) -> bool {
        self.nodes[ROOT].branch_support == 0
    }

    /// The network's preferred ledger, given `largest_issued` (the
    /// sequence number of the largest validation this node has issued).
    /// Matches `getPreferred()`.
    pub fn get_preferred(&self, largest_issued: L::Seq) -> Option<SpanTip<L>> {
        if self.is_empty() {
            return None;
        }

        let mut curr = ROOT;
        let mut done = false;
        let mut uncommitted: u32 = 0;
        let seq_support: Vec<(L::Seq, u32)> = self.seq_support.iter().map(|(k, v)| (*k, *v)).collect();
        let mut uncommitted_idx = 0usize;

        while !done {
            {
                let span = &self.nodes[curr].span;
                let mut next_seq = span.start + 1;

                while uncommitted_idx < seq_support.len()
                    && seq_support[uncommitted_idx].0 < next_seq.max(largest_issued)
                {
                    uncommitted += seq_support[uncommitted_idx].1;
                    uncommitted_idx += 1;
                }

                while next_seq < span.end && self.nodes[curr].branch_support > uncommitted {
                    if uncommitted_idx < seq_support.len() && seq_support[uncommitted_idx].0 < span.end {
                        next_seq = seq_support[uncommitted_idx].0 + 1;
                        uncommitted += seq_support[uncommitted_idx].1;
                        uncommitted_idx += 1;
                    } else {
                        next_seq = span.end;
                    }
                }

                if next_seq < span.end {
                    return span.before(next_seq).map(|s| s.tip());
                }
            }

            let live_children: Vec<usize> =
                self.nodes[curr].children.iter().copied().filter(|&c| self.nodes[c].live).collect();

            let mut best: Option<usize> = None;
            let mut margin: u32 = 0;

            if live_children.len() == 1 {
                best = Some(live_children[0]);
                margin = self.nodes[live_children[0]].branch_support;
            } else if !live_children.is_empty() {
                let mut ordered = live_children.clone();
                ordered.sort_by(|&a, &b| {
                    let a_key = (self.nodes[a].branch_support, self.nodes[a].span.start_id());
                    let b_key = (self.nodes[b].branch_support, self.nodes[b].span.start_id());
                    b_key.cmp(&a_key)
                });
                let first = ordered[0];
                let second = ordered[1];
                best = Some(first);
                margin = self.nodes[first].branch_support - self.nodes[second].branch_support;
                if self.nodes[first].span.start_id() > self.nodes[second].span.start_id() {
                    margin += 1;
                }
            }

            if let Some(best_idx) = best {
                if margin > uncommitted || uncommitted == 0 {
                    curr = best_idx;
                } else {
                    done = true;
                }
            } else {
                done = true;
            }
        }
        Some(self.nodes[curr].span.tip())
    }

    /// Check the compressed-trie and support invariants. Matches
    /// `checkInvariants()`. Intended for use in tests/debug assertions.
    pub fn check_invariants(&self) -> bool {
        let mut expected: BTreeMap<L::Seq, u32> = BTreeMap::new();
        let mut stack = vec![ROOT];
        while let Some(curr) = stack.pop() {
            let node = &self.nodes[curr];
            if !node.live && curr != ROOT {
                continue;
            }
            let live_children: Vec<usize> = node.children.iter().copied().filter(|&c| self.nodes[c].live).collect();

            if curr != ROOT && node.tip_support == 0 && live_children.len() < 2 {
                return false;
            }

            let mut support = node.tip_support;
            if node.tip_support != 0 {
                *expected.entry(node.span.end - 1).or_insert(0) += node.tip_support;
            }
            for &child in &live_children {
                if self.nodes[child].parent != Some(curr) {
                    return false;
                }
                support += self.nodes[child].branch_support;
                stack.push(child);
            }
            if support != node.branch_support {
                return false;
            }
        }
        expected == self.seq_support
    }
}

impl<L: TrieLedger> Default for LedgerTrie<L> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A minimal `TrieLedger` for tests: a chain identified by a `Vec<u8>`
    /// "history" where `history[i]` is the id of the ancestor at seq `i`.
    #[derive(Debug, Clone, PartialEq, Eq)]
    struct TestLedger {
        history: Vec<u8>,
    }

    impl TestLedger {
        fn genesis() -> Self {
            Self { history: vec![0] }
        }

        fn child(&self, id: u8) -> Self {
            let mut history = self.history.clone();
            history.push(id);
            Self { history }
        }
    }

    impl TrieLedger for TestLedger {
        type Seq = u32;
        type Id = u8;

        fn genesis() -> Self {
            TestLedger::genesis()
        }

        fn seq(&self) -> u32 {
            (self.history.len() - 1) as u32
        }

        fn ancestor(&self, s: u32) -> u8 {
            self.history.get(s as usize).copied().unwrap_or(0)
        }

        fn mismatch(&self, other: &Self) -> u32 {
            let max_check = self.seq().min(other.seq()) + 1;
            for s in 0..max_check {
                if self.ancestor(s) != other.ancestor(s) {
                    return s;
                }
            }
            max_check
        }
    }

    #[test]
    fn empty_trie_has_no_preferred_ledger() {
        let trie: LedgerTrie<TestLedger> = LedgerTrie::new();
        assert!(trie.is_empty());
        assert!(trie.get_preferred(0).is_none());
    }

    #[test]
    fn insert_single_chain_prefers_its_tip() {
        let mut trie = LedgerTrie::new();
        let genesis = TestLedger::genesis();
        let a = genesis.child(1);
        let b = a.child(2);

        trie.insert(&b, 1);
        assert!(trie.check_invariants());
        assert_eq!(trie.tip_support(2), 1);
        assert_eq!(trie.branch_support(&b), 1);

        let preferred = trie.get_preferred(0).expect("trie is non-empty");
        assert_eq!(preferred.id, 2);
        assert_eq!(preferred.seq, 2);
    }

    #[test]
    fn insert_diverging_chains_prefers_higher_branch_support() {
        let mut trie = LedgerTrie::new();
        let genesis = TestLedger::genesis();
        let a = genesis.child(1);
        let b1 = a.child(2);
        let b2 = a.child(3);

        // Two validators on b1, one on b2: b1 should be preferred.
        trie.insert(&b1, 1);
        trie.insert(&b1, 1);
        trie.insert(&b2, 1);
        assert!(trie.check_invariants());

        let preferred = trie.get_preferred(0).expect("trie is non-empty");
        assert_eq!(preferred.id, 2);
    }

    #[test]
    fn remove_decrements_and_compresses() {
        let mut trie = LedgerTrie::new();
        let genesis = TestLedger::genesis();
        let a = genesis.child(1);
        let b = a.child(2);

        trie.insert(&b, 2);
        assert_eq!(trie.tip_support(2), 2);

        assert!(trie.remove(2, 2, 1));
        assert_eq!(trie.tip_support(2), 1);
        assert!(trie.check_invariants());

        assert!(trie.remove(2, 2, 1));
        assert_eq!(trie.tip_support(2), 0);
        assert!(trie.check_invariants());

        // Removing a ledger with no support is a no-op returning false.
        assert!(!trie.remove(2, 2, 1));
    }

    #[test]
    fn branch_support_counts_descendants() {
        let mut trie = LedgerTrie::new();
        let genesis = TestLedger::genesis();
        let a = genesis.child(1);
        let b = a.child(2);
        let c = b.child(3);

        trie.insert(&c, 1);
        assert!(trie.check_invariants());

        // `a` is a proper prefix of the inserted chain, so its branch
        // support should reflect the descendant's support.
        assert_eq!(trie.branch_support(&a), 1);
        assert_eq!(trie.branch_support(&b), 1);
        assert_eq!(trie.branch_support(&c), 1);
    }

    #[test]
    fn uncommitted_support_defers_preference_until_resolved() {
        let mut trie = LedgerTrie::new();
        let genesis = TestLedger::genesis();
        let a = genesis.child(1);
        let b = a.child(2);

        // Only one validator has moved past genesis, but largest_issued=5
        // manufactures 1 unit of "uncommitted" support (from seq_support's
        // entry at seq=2, since 2 < max(next_seq, largest_issued)=5) that
        // could theoretically still land on a not-yet-seen alternative
        // branch. With margin(=1) not exceeding uncommitted(=1), the walk
        // correctly stops at the root rather than speculatively preferring
        // the only known branch -- matching the reference's conservative
        // "don't switch until you know an alternative can't catch up"
        // design intent.
        trie.insert(&b, 1);
        let preferred = trie.get_preferred(5).expect("trie is non-empty");
        assert_eq!(preferred.id, 0);
        assert_eq!(preferred.seq, 0);

        // With largest_issued at or below what's actually been seen, there
        // is no manufactured uncommitted weight, so the lone branch is
        // correctly preferred.
        let preferred_no_uncommitted = trie.get_preferred(0).expect("trie is non-empty");
        assert_eq!(preferred_no_uncommitted.id, 2);
    }
}
