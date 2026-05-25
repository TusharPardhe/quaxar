pub trait NetworkIDService: Send + Sync {
    fn get_network_id(&self) -> u32;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FixedNetworkIdService {
    network_id: u32,
}

impl FixedNetworkIdService {
    pub const fn new(network_id: u32) -> Self {
        Self { network_id }
    }
}

impl NetworkIDService for FixedNetworkIdService {
    fn get_network_id(&self) -> u32 {
        self.network_id
    }
}

#[cfg(test)]
mod tests {
    use super::{FixedNetworkIdService, NetworkIDService};

    #[test]
    fn fixed_network_id_service_returns_the_configured_network_id() {
        let service = FixedNetworkIdService::new(1_025);

        assert_eq!(service.get_network_id(), 1_025);
    }
}
