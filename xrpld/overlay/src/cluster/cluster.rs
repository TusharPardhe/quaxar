//! Overlay cluster membership and state.

use std::collections::BTreeMap;
use std::sync::Mutex;
use std::time::SystemTime;

use protocol::{PublicKey, parse_base58_node_public};

use crate::cluster_node::ClusterNode;

#[derive(Debug, Default)]
pub struct Cluster {
    nodes: Mutex<BTreeMap<PublicKey, ClusterNode>>,
}

impl Cluster {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn member(&self, node: PublicKey) -> Option<String> {
        self.nodes
            .lock()
            .expect("cluster lock")
            .get(&node)
            .map(|node| node.name().to_owned())
    }

    pub fn size(&self) -> usize {
        self.nodes.lock().expect("cluster lock").len()
    }

    pub fn update(
        &self,
        identity: PublicKey,
        name: impl Into<String>,
        load_fee: u32,
        report_time: SystemTime,
    ) -> bool {
        let mut nodes = self.nodes.lock().expect("cluster lock");
        let mut name = name.into();
        if let Some(existing) = nodes.get(&identity) {
            if report_time <= existing.report_time() {
                return false;
            }
            if name.is_empty() {
                name = existing.name().to_owned();
            }
        }
        nodes.insert(
            identity,
            ClusterNode::new(identity, name, load_fee, report_time),
        );
        true
    }

    pub fn for_each(&self, mut visitor: impl FnMut(&ClusterNode)) {
        let snapshot = self
            .nodes
            .lock()
            .expect("cluster lock")
            .values()
            .cloned()
            .collect::<Vec<_>>();
        for node in &snapshot {
            visitor(node);
        }
    }

    pub fn load(&self, entries: &[String]) -> bool {
        for entry in entries {
            let trimmed = entry.trim();
            if trimmed.is_empty() {
                return false;
            }
            let mut parts = trimmed.split_whitespace();
            let Some(identity) = parts.next() else {
                return false;
            };
            let Some(public_key_bytes) = parse_base58_node_public(identity) else {
                return false;
            };
            let Ok(public_key) = PublicKey::from_slice(&public_key_bytes) else {
                return false;
            };
            if self.member(public_key).is_some() {
                continue;
            }
            let name = parts.collect::<Vec<_>>().join(" ");
            self.update(public_key, name, 0, SystemTime::UNIX_EPOCH);
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, SystemTime};

    use protocol::{KeyType, SecretKey, derive_public_key};

    use super::Cluster;

    #[test]
    fn cluster_update_ignores_stale_reports() {
        let secret = SecretKey::from_bytes([5u8; 32]);
        let public = derive_public_key(KeyType::Secp256k1, &secret).expect("public key");
        let cluster = Cluster::new();
        let now = SystemTime::now();

        assert!(cluster.update(public, "alpha", 10, now));
        assert!(!cluster.update(public, "beta", 20, now - Duration::from_secs(1)));
        assert_eq!(cluster.member(public), Some("alpha".to_owned()));
    }
}
