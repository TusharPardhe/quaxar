use serde_json::{Map, Value, json};
use std::collections::BTreeMap;
use std::fmt::{self, Display};

pub trait LedgerHistory: Clone {
    type Id: Copy + Default + Ord + Eq;

    fn make_genesis() -> Self;
    fn seq(&self) -> u32;
    fn id(&self) -> Self::Id;
    fn ancestor(&self, seq: u32) -> Self::Id;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpanTip<L: LedgerHistory> {
    pub seq: u32,
    pub id: L::Id,
    pub(crate) ledger: L,
}

impl<L: LedgerHistory> SpanTip<L> {
    pub fn ancestor(&self, seq: u32) -> L::Id {
        assert!(seq <= self.seq, "xrpl::SpanTip::ancestor : valid input");
        self.ledger.ancestor(seq)
    }

    pub fn ledger(&self) -> &L {
        &self.ledger
    }
}

pub fn mismatch<L: LedgerHistory>(a: &L, b: &L) -> u32 {
    let lower = min_seq(a).max(min_seq(b));
    let upper = a.seq().min(b.seq());

    let mut curr = upper;
    while curr != 0 && curr >= lower && a.ancestor(curr) != b.ancestor(curr) {
        curr -= 1;
    }

    if curr < lower { 1 } else { curr + 1 }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Span<L: LedgerHistory> {
    start: u32,
    end: u32,
    ledger: L,
}

impl<L: LedgerHistory> Default for Span<L> {
    fn default() -> Self {
        let ledger = L::make_genesis();
        assert_eq!(ledger.seq(), 0, "xrpl::Span::Span : ledger is genesis");
        Self {
            start: 0,
            end: 1,
            ledger,
        }
    }
}

impl<L: LedgerHistory> Span<L> {
    fn new(ledger: L) -> Self {
        Self {
            start: 0,
            end: ledger.seq() + 1,
            ledger,
        }
    }

    fn from(&self, spot: u32) -> Option<Self> {
        self.sub(spot, self.end)
    }

    fn before(&self, spot: u32) -> Option<Self> {
        self.sub(self.start, spot)
    }

    fn start_id(&self) -> L::Id {
        self.ledger.ancestor(self.start)
    }

    fn diff(&self, other: &L) -> u32 {
        self.clamp(mismatch(&self.ledger, other))
    }

    fn tip(&self) -> SpanTip<L> {
        let tip_seq = self.end - 1;
        SpanTip {
            seq: tip_seq,
            id: self.ledger.ancestor(tip_seq),
            ledger: self.ledger.clone(),
        }
    }

    fn merge(left: &Self, right: &Self) -> Self {
        if left.end < right.end {
            Self {
                start: left.start.min(right.start),
                end: right.end,
                ledger: right.ledger.clone(),
            }
        } else {
            Self {
                start: left.start.min(right.start),
                end: left.end,
                ledger: left.ledger.clone(),
            }
        }
    }

    fn clamp(&self, value: u32) -> u32 {
        value.clamp(self.start, self.end)
    }

    fn sub(&self, from: u32, to: u32) -> Option<Self> {
        let new_from = self.clamp(from);
        let new_to = self.clamp(to);
        (new_from < new_to).then(|| Self {
            start: new_from,
            end: new_to,
            ledger: self.ledger.clone(),
        })
    }
}

impl<L> Display for Span<L>
where
    L: LedgerHistory,
    L::Id: Display,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}[{},{}]", self.tip().id, self.start, self.end)
    }
}

#[derive(Debug, Clone)]
struct Node<L: LedgerHistory> {
    span: Span<L>,
    tip_support: u32,
    branch_support: u32,
    children: Vec<usize>,
    parent: Option<usize>,
    live: bool,
}

impl<L: LedgerHistory> Default for Node<L> {
    fn default() -> Self {
        Self {
            span: Span::default(),
            tip_support: 0,
            branch_support: 0,
            children: Vec::new(),
            parent: None,
            live: true,
        }
    }
}

impl<L: LedgerHistory> Node<L> {
    fn with_span(span: Span<L>) -> Self {
        Self {
            span,
            tip_support: 0,
            branch_support: 0,
            children: Vec::new(),
            parent: None,
            live: true,
        }
    }

    fn json(&self, arena: &[Node<L>]) -> Value
    where
        L::Id: Display,
    {
        let mut value = Map::new();
        value.insert("span".to_owned(), Value::String(self.span.to_string()));
        value.insert(
            "startID".to_owned(),
            Value::String(self.span.start_id().to_string()),
        );
        value.insert("seq".to_owned(), json!(self.span.tip().seq));
        value.insert("tipSupport".to_owned(), json!(self.tip_support));
        value.insert("branchSupport".to_owned(), json!(self.branch_support));
        if !self.children.is_empty() {
            value.insert(
                "children".to_owned(),
                Value::Array(
                    self.children
                        .iter()
                        .filter_map(|child| arena.get(*child))
                        .filter(|child| child.live)
                        .map(|child| child.json(arena))
                        .collect(),
                ),
            );
        }
        Value::Object(value)
    }
}

#[derive(Debug, Clone)]
pub struct LedgerTrie<L: LedgerHistory> {
    nodes: Vec<Node<L>>,
    root: usize,
    seq_support: BTreeMap<u32, u32>,
}

impl<L: LedgerHistory> Default for LedgerTrie<L> {
    fn default() -> Self {
        Self::new()
    }
}

impl<L: LedgerHistory> LedgerTrie<L> {
    pub fn new() -> Self {
        Self {
            nodes: vec![Node::default()],
            root: 0,
            seq_support: BTreeMap::new(),
        }
    }

    pub fn insert(&mut self, ledger: L, count: u32) {
        let (loc, diff_seq) = self.find(&ledger);
        let mut inc_node = loc;

        let prefix = self.nodes[loc].span.before(diff_seq);
        let old_suffix = self.nodes[loc].span.from(diff_seq);
        let new_suffix = Span::new(ledger.clone()).from(diff_seq);

        if let Some(old_suffix) = old_suffix {
            let moved_children = std::mem::take(&mut self.nodes[loc].children);
            let mut new_node = Node::with_span(old_suffix);
            new_node.tip_support = self.nodes[loc].tip_support;
            new_node.branch_support = self.nodes[loc].branch_support;
            new_node.children = moved_children;
            new_node.parent = Some(loc);
            let new_idx = self.alloc_node(new_node);
            for child_idx in self.nodes[new_idx].children.clone() {
                self.nodes[child_idx].parent = Some(new_idx);
            }

            self.nodes[loc].span = prefix.expect("xrpl::LedgerTrie::insert : prefix is set");
            self.nodes[loc].children.push(new_idx);
            self.nodes[loc].tip_support = 0;
        }

        if let Some(new_suffix) = new_suffix {
            let mut new_node = Node::with_span(new_suffix);
            new_node.parent = Some(loc);
            let new_idx = self.alloc_node(new_node);
            self.nodes[loc].children.push(new_idx);
            inc_node = new_idx;
        }

        self.nodes[inc_node].tip_support += count;
        let mut cursor = Some(inc_node);
        while let Some(index) = cursor {
            self.nodes[index].branch_support += count;
            cursor = self.nodes[index].parent;
        }

        *self.seq_support.entry(ledger.seq()).or_default() += count;
    }

    pub fn remove(&mut self, ledger: &L, count: u32) -> bool {
        let Some(mut loc) = self.find_by_ledger_id(ledger, self.root) else {
            return false;
        };
        if self.nodes[loc].tip_support == 0 {
            return false;
        }

        let count = count.min(self.nodes[loc].tip_support);
        self.nodes[loc].tip_support -= count;

        let seq = ledger.seq();
        let entry = self
            .seq_support
            .get_mut(&seq)
            .expect("xrpl::LedgerTrie::remove : valid input ledger");
        assert!(
            *entry >= count,
            "xrpl::LedgerTrie::remove : valid input ledger"
        );
        *entry -= count;
        if *entry == 0 {
            self.seq_support.remove(&seq);
        }

        let mut cursor = Some(loc);
        while let Some(index) = cursor {
            self.nodes[index].branch_support -= count;
            cursor = self.nodes[index].parent;
        }

        while self.nodes[loc].tip_support == 0 && loc != self.root {
            let parent = self.nodes[loc].parent.expect("non-root nodes have parents");
            let child_count = self.nodes[loc].children.len();
            if child_count == 0 {
                self.detach_child(parent, loc);
                self.nodes[loc].live = false;
            } else if child_count == 1 {
                let child = self.nodes[loc].children[0];
                let merged = Span::merge(&self.nodes[loc].span, &self.nodes[child].span);
                self.nodes[child].span = merged;
                self.nodes[child].parent = Some(parent);
                self.nodes[parent].children.push(child);
                self.detach_child(parent, loc);
                self.nodes[loc].children.clear();
                self.nodes[loc].live = false;
            } else {
                break;
            }
            loc = parent;
        }

        true
    }

    pub fn tip_support(&self, ledger: &L) -> u32 {
        self.find_by_ledger_id(ledger, self.root)
            .map(|index| self.nodes[index].tip_support)
            .unwrap_or(0)
    }

    pub fn branch_support(&self, ledger: &L) -> u32 {
        let loc = self.find_by_ledger_id(ledger, self.root).or_else(|| {
            let (loc, diff_seq) = self.find(ledger);
            (diff_seq > ledger.seq() && ledger.seq() < self.nodes[loc].span.end).then_some(loc)
        });

        loc.map(|index| self.nodes[index].branch_support)
            .unwrap_or(0)
    }

    pub fn get_preferred(&self, largest_issued: u32) -> Option<SpanTip<L>> {
        if self.empty() {
            return None;
        }

        let mut curr = self.root;
        let mut done = false;
        let mut uncommitted = 0u32;
        let seq_support = self.seq_support.iter().collect::<Vec<_>>();
        let mut uncommitted_it = 0usize;

        while !done {
            let span = &self.nodes[curr].span;
            let mut next_seq = span.start + 1;
            while uncommitted_it < seq_support.len()
                && *seq_support[uncommitted_it].0 < next_seq.max(largest_issued)
            {
                uncommitted += *seq_support[uncommitted_it].1;
                uncommitted_it += 1;
            }

            while next_seq < span.end && self.nodes[curr].branch_support > uncommitted {
                if uncommitted_it < seq_support.len() && *seq_support[uncommitted_it].0 < span.end {
                    next_seq = *seq_support[uncommitted_it].0 + 1;
                    uncommitted += *seq_support[uncommitted_it].1;
                    uncommitted_it += 1;
                } else {
                    next_seq = span.end;
                }
            }

            if next_seq < span.end {
                return span.before(next_seq).map(|span| span.tip());
            }

            let children = self.nodes[curr]
                .children
                .iter()
                .copied()
                .filter(|child| self.nodes[*child].live)
                .collect::<Vec<_>>();

            let (best, margin) = if children.len() == 1 {
                let child = children[0];
                (Some(child), self.nodes[child].branch_support)
            } else if !children.is_empty() {
                let mut ordered = children;
                ordered.sort_by(|left, right| {
                    (
                        self.nodes[*right].branch_support,
                        self.nodes[*right].span.start_id(),
                    )
                        .cmp(&(
                            self.nodes[*left].branch_support,
                            self.nodes[*left].span.start_id(),
                        ))
                });

                let best = ordered[0];
                let second = ordered[1];
                let mut margin =
                    self.nodes[best].branch_support - self.nodes[second].branch_support;
                if self.nodes[best].span.start_id() > self.nodes[second].span.start_id() {
                    margin += 1;
                }
                (Some(best), margin)
            } else {
                (None, 0)
            };

            if let Some(best) = best
                && (margin > uncommitted || uncommitted == 0)
            {
                curr = best;
            } else {
                done = true;
            }
        }

        Some(self.nodes[curr].span.tip())
    }

    pub fn empty(&self) -> bool {
        self.nodes[self.root].branch_support == 0
    }

    pub fn get_json(&self) -> Value
    where
        L::Id: Display,
    {
        let mut value = Map::new();
        value.insert("trie".to_owned(), self.nodes[self.root].json(&self.nodes));
        let seq_support = self
            .seq_support
            .iter()
            .map(|(seq, support)| (seq.to_string(), json!(support)))
            .collect::<Map<String, Value>>();
        value.insert("seq_support".to_owned(), Value::Object(seq_support));
        Value::Object(value)
    }

    pub fn check_invariants(&self) -> bool {
        let mut expected_seq_support = BTreeMap::<u32, u32>::new();
        let mut stack = vec![self.root];

        while let Some(curr) = stack.pop() {
            if !self.nodes[curr].live {
                continue;
            }

            if curr != self.root
                && self.nodes[curr].tip_support == 0
                && self.nodes[curr].children.len() < 2
            {
                return false;
            }

            let mut support = self.nodes[curr].tip_support;
            if self.nodes[curr].tip_support != 0 {
                *expected_seq_support
                    .entry(self.nodes[curr].span.end - 1)
                    .or_default() += self.nodes[curr].tip_support;
            }

            for child in &self.nodes[curr].children {
                if !self.nodes[*child].live {
                    continue;
                }
                if self.nodes[*child].parent != Some(curr) {
                    return false;
                }
                support += self.nodes[*child].branch_support;
                stack.push(*child);
            }

            if support != self.nodes[curr].branch_support {
                return false;
            }
        }

        expected_seq_support == self.seq_support
    }

    fn alloc_node(&mut self, node: Node<L>) -> usize {
        self.nodes.push(node);
        self.nodes.len() - 1
    }

    fn find(&self, ledger: &L) -> (usize, u32) {
        let mut curr = self.root;
        let mut pos = self.nodes[curr].span.diff(ledger);
        let mut done = false;

        while !done && pos == self.nodes[curr].span.end {
            done = true;
            for child in &self.nodes[curr].children {
                if !self.nodes[*child].live {
                    continue;
                }
                let child_pos = self.nodes[*child].span.diff(ledger);
                if child_pos > pos {
                    done = false;
                    pos = child_pos;
                    curr = *child;
                    break;
                }
            }
        }

        (curr, pos)
    }

    fn find_by_ledger_id(&self, ledger: &L, parent: usize) -> Option<usize> {
        if !self.nodes[parent].live {
            return None;
        }
        if self.nodes[parent].span.tip().id == ledger.id() {
            return Some(parent);
        }
        for child in &self.nodes[parent].children {
            if let Some(found) = self.find_by_ledger_id(ledger, *child) {
                return Some(found);
            }
        }
        None
    }

    fn detach_child(&mut self, parent: usize, child: usize) {
        let children = &mut self.nodes[parent].children;
        let pos = children
            .iter()
            .position(|candidate| *candidate == child)
            .expect("xrpl::Node::erase : valid input");
        children.swap_remove(pos);
    }
}

fn min_seq<L: LedgerHistory>(ledger: &L) -> u32 {
    let seq = ledger.seq();
    for candidate in 0..=seq {
        if candidate == seq || ledger.ancestor(candidate) != L::Id::default() {
            return candidate;
        }
    }
    seq
}

#[cfg(test)]
mod tests {
    use super::{LedgerHistory, LedgerTrie, mismatch};

    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
    struct TestLedger {
        seq: u32,
        id: u32,
        ancestors: [u32; 8],
    }

    impl TestLedger {
        fn new(seq: u32, id: u32, ancestors: [u32; 8]) -> Self {
            Self { seq, id, ancestors }
        }
    }

    impl LedgerHistory for TestLedger {
        type Id = u32;

        fn make_genesis() -> Self {
            Self::new(0, 0, [0; 8])
        }

        fn seq(&self) -> u32 {
            self.seq
        }

        fn id(&self) -> Self::Id {
            self.id
        }

        fn ancestor(&self, seq: u32) -> Self::Id {
            if seq == self.seq {
                return self.id;
            }
            self.ancestors
                .get(seq as usize)
                .copied()
                .unwrap_or_default()
        }
    }

    fn ledger_a() -> TestLedger {
        TestLedger::new(3, 30, [0, 10, 20, 0, 0, 0, 0, 0])
    }

    fn ledger_b() -> TestLedger {
        TestLedger::new(3, 31, [0, 10, 21, 0, 0, 0, 0, 0])
    }

    fn ledger_c() -> TestLedger {
        TestLedger::new(4, 40, [0, 10, 20, 30, 0, 0, 0, 0])
    }

    #[test]
    fn mismatch_returns_first_possible_divergence() {
        assert_eq!(mismatch(&ledger_a(), &ledger_c()), 4);
        assert_eq!(mismatch(&ledger_a(), &ledger_b()), 2);
    }

    #[test]
    fn trie_tracks_tip_and_branch_support() {
        let a = ledger_a();
        let b = ledger_b();
        let c = ledger_c();
        let mut trie = LedgerTrie::new();

        trie.insert(a, 1);
        trie.insert(b, 2);
        trie.insert(c, 3);

        assert_eq!(trie.tip_support(&a), 1);
        assert_eq!(trie.tip_support(&b), 2);
        assert_eq!(trie.tip_support(&c), 3);
        assert_eq!(trie.branch_support(&a), 4);
        assert_eq!(trie.branch_support(&b), 2);
        assert!(trie.check_invariants());

        assert!(trie.remove(&c, 2));
        assert_eq!(trie.tip_support(&c), 1);
        assert_eq!(trie.branch_support(&a), 2);
        assert!(trie.check_invariants());
    }

    #[test]
    fn trie_preferred_ledger_uses_branch_margin_and_uncommitted_rules() {
        let a = ledger_a();
        let b = ledger_b();
        let c = ledger_c();
        let mut trie = LedgerTrie::new();

        trie.insert(a, 1);
        trie.insert(b, 2);
        trie.insert(c, 5);

        let preferred = trie.get_preferred(4).expect("preferred tip");
        assert_eq!(preferred.seq, 4);
        assert_eq!(preferred.id, 40);

        trie.remove(&c, 5);
        let preferred = trie.get_preferred(4).expect("preferred tip");
        assert_eq!(preferred.seq, 0);
        assert_eq!(preferred.id, 0);

        let preferred = trie.get_preferred(3).expect("preferred tip");
        assert_eq!(preferred.seq, 3);
        assert_eq!(preferred.id, 31);
    }

    #[test]
    fn trie_json_contains_support_snapshots() {
        let mut trie = LedgerTrie::new();
        trie.insert(ledger_a(), 1);

        let json = trie.get_json();
        assert_eq!(json["trie"]["tipSupport"], 0);
        assert_eq!(json["seq_support"]["3"], 1);
    }
}
