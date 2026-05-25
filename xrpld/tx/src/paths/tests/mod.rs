//! Tests for path discovery and ranking

#[cfg(test)]
mod tests {
    use crate::paths::search::Pathfinder;
    use crate::paths::node::{Path, PathNode};
    use protocol::AccountID;

    #[test]
    fn test_pathfinder_initialization() {
        let pf = Pathfinder::new(10, 5);
        assert_eq!(pf.max_paths, 10);
        assert_eq!(pf.max_hops, 5);
    }

    #[test]
    fn test_path_node_equality() {
        let acc1 = AccountID::default();
        let acc2 = AccountID::default();
        let node1 = PathNode::Account(acc1);
        let node2 = PathNode::Account(acc1);
        assert_eq!(node1, node2);
        assert_ne!(node1, PathNode::Account(acc2));
    }

    #[test]
    fn test_path_construction() {
        let mut path = Path::new();
        let node = PathNode::Account(AccountID::default());
        path.add_node(node.clone());
        assert_eq!(path.nodes.len(), 1);
        assert_eq!(path.nodes[0], node);
    }
}
