//! Typed overlay message routing above the wire codec.

use crate::message::{
    ProtocolMessage, ProtocolPayload, TmCluster, TmEndpoints, TmGetLedger, TmGetObjectByHash,
    TmHaveTransactionSet, TmHaveTransactions, TmLedgerData, TmManifests, TmPing,
    TmProofPathRequest, TmProofPathResponse, TmProposeSet, TmReplayDeltaRequest,
    TmReplayDeltaResponse, TmSquelch, TmStatusChange, TmTransaction, TmTransactions, TmValidation,
    TmValidatorList, TmValidatorListCollection,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RouteAction {
    Continue,
    Stop,
}

pub trait MessageRouter {
    fn on_manifests(&mut self, _message: &TmManifests) -> RouteAction {
        RouteAction::Continue
    }
    fn on_ping(&mut self, _message: &TmPing) -> RouteAction {
        RouteAction::Continue
    }
    fn on_cluster(&mut self, _message: &TmCluster) -> RouteAction {
        RouteAction::Continue
    }
    fn on_endpoints(&mut self, _message: &TmEndpoints) -> RouteAction {
        RouteAction::Continue
    }
    fn on_transaction(&mut self, _message: &TmTransaction) -> RouteAction {
        RouteAction::Continue
    }
    fn on_get_ledger(&mut self, _message: &TmGetLedger) -> RouteAction {
        RouteAction::Continue
    }
    fn on_ledger_data(&mut self, _message: &TmLedgerData) -> RouteAction {
        RouteAction::Continue
    }
    fn on_propose_ledger(&mut self, _message: &TmProposeSet) -> RouteAction {
        RouteAction::Continue
    }
    fn on_status_change(&mut self, _message: &TmStatusChange) -> RouteAction {
        RouteAction::Continue
    }
    fn on_have_set(&mut self, _message: &TmHaveTransactionSet) -> RouteAction {
        RouteAction::Continue
    }
    fn on_validation(&mut self, _message: &TmValidation) -> RouteAction {
        RouteAction::Continue
    }
    fn on_validator_list(&mut self, _message: &TmValidatorList) -> RouteAction {
        RouteAction::Continue
    }
    fn on_validator_list_collection(
        &mut self,
        _message: &TmValidatorListCollection,
    ) -> RouteAction {
        RouteAction::Continue
    }
    fn on_get_objects(&mut self, _message: &TmGetObjectByHash) -> RouteAction {
        RouteAction::Continue
    }
    fn on_have_transactions(&mut self, _message: &TmHaveTransactions) -> RouteAction {
        RouteAction::Continue
    }
    fn on_transactions(&mut self, _message: &TmTransactions) -> RouteAction {
        RouteAction::Continue
    }
    fn on_squelch(&mut self, _message: &TmSquelch) -> RouteAction {
        RouteAction::Continue
    }
    fn on_proof_path_request(&mut self, _message: &TmProofPathRequest) -> RouteAction {
        RouteAction::Continue
    }
    fn on_proof_path_response(&mut self, _message: &TmProofPathResponse) -> RouteAction {
        RouteAction::Continue
    }
    fn on_replay_delta_request(&mut self, _message: &TmReplayDeltaRequest) -> RouteAction {
        RouteAction::Continue
    }
    fn on_replay_delta_response(&mut self, _message: &TmReplayDeltaResponse) -> RouteAction {
        RouteAction::Continue
    }
}

pub fn route_message(router: &mut impl MessageRouter, message: &ProtocolMessage) -> RouteAction {
    match &message.payload {
        ProtocolPayload::Manifests(message) => router.on_manifests(message),
        ProtocolPayload::Ping(message) => router.on_ping(message),
        ProtocolPayload::Cluster(message) => router.on_cluster(message),
        ProtocolPayload::Endpoints(message) => router.on_endpoints(message),
        ProtocolPayload::Transaction(message) => router.on_transaction(message),
        ProtocolPayload::GetLedger(message) => router.on_get_ledger(message),
        ProtocolPayload::LedgerData(message) => router.on_ledger_data(message),
        ProtocolPayload::ProposeLedger(message) => router.on_propose_ledger(message),
        ProtocolPayload::StatusChange(message) => router.on_status_change(message),
        ProtocolPayload::HaveSet(message) => router.on_have_set(message),
        ProtocolPayload::Validation(message) => router.on_validation(message),
        ProtocolPayload::ValidatorList(message) => router.on_validator_list(message),
        ProtocolPayload::ValidatorListCollection(message) => {
            router.on_validator_list_collection(message)
        }
        ProtocolPayload::GetObjects(message) => router.on_get_objects(message),
        ProtocolPayload::HaveTransactions(message) => router.on_have_transactions(message),
        ProtocolPayload::Transactions(message) => router.on_transactions(message),
        ProtocolPayload::Squelch(message) => router.on_squelch(message),
        ProtocolPayload::ProofPathRequest(message) => router.on_proof_path_request(message),
        ProtocolPayload::ProofPathResponse(message) => router.on_proof_path_response(message),
        ProtocolPayload::ReplayDeltaRequest(message) => router.on_replay_delta_request(message),
        ProtocolPayload::ReplayDeltaResponse(message) => router.on_replay_delta_response(message),
    }
}

#[cfg(test)]
mod tests {
    use super::{MessageRouter, RouteAction, route_message};
    use crate::message::{ProtocolMessage, ProtocolPayload, TmStatusChange, TmTransaction};

    #[derive(Default)]
    struct Router {
        tx_seen: bool,
        status_seen: bool,
    }

    impl MessageRouter for Router {
        fn on_transaction(&mut self, _message: &TmTransaction) -> RouteAction {
            self.tx_seen = true;
            RouteAction::Continue
        }

        fn on_status_change(&mut self, _message: &TmStatusChange) -> RouteAction {
            self.status_seen = true;
            RouteAction::Stop
        }
    }

    #[test]
    fn routes_transaction_and_status_messages() {
        let tx = ProtocolMessage::new(ProtocolPayload::Transaction(TmTransaction {
            raw_transaction: vec![1, 2, 3],
            status: 1,
            receive_timestamp: None,
            deferred: None,
        }));
        let status = ProtocolMessage::new(ProtocolPayload::StatusChange(TmStatusChange {
            new_status: None,
            new_event: Some(3),
            ledger_seq: Some(10),
            ledger_hash: None,
            ledger_hash_previous: None,
            network_time: Some(20),
            first_seq: None,
            last_seq: None,
        }));

        let mut router = Router::default();
        assert_eq!(route_message(&mut router, &tx), RouteAction::Continue);
        assert_eq!(route_message(&mut router, &status), RouteAction::Stop);
        assert!(router.tx_seen);
        assert!(router.status_seen);
    }
}
