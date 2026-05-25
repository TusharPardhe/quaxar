use protocol::{AccountID, Issue, STAmount};

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum PathNode {
    Account(AccountID),
    OrderBook(Issue),
}

#[derive(Clone, Debug, Default)]
pub struct Path {
    pub nodes: Vec<PathNode>,
    pub quality: STAmount,
    pub liquidity: STAmount,
}

impl Path {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_node(&mut self, node: PathNode) {
        self.nodes.push(node);
    }
}
