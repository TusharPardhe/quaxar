//! Cluster-node state carried by the overlay cluster list.

use std::time::SystemTime;

use protocol::PublicKey;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClusterNode {
    identity: PublicKey,
    name: String,
    load_fee: u32,
    report_time: SystemTime,
}

impl ClusterNode {
    pub fn new(
        identity: PublicKey,
        name: impl Into<String>,
        load_fee: u32,
        report_time: SystemTime,
    ) -> Self {
        Self {
            identity,
            name: name.into(),
            load_fee,
            report_time,
        }
    }

    pub fn identity(&self) -> PublicKey {
        self.identity
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn load_fee(&self) -> u32 {
        self.load_fee
    }

    pub fn report_time(&self) -> SystemTime {
        self.report_time
    }
}
