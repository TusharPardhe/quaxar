use crate::network::network_ops::{NetworkOpsOperatingMode, SharedNetworkOpsState};
use crate::state::stop_tree::StopTree;
use ledger::LedgerMasterCaughtUp;

pub const SERVER_OKAY_STOPPING_REASON: &str = "Server is shutting down";
pub const SERVER_OKAY_NEED_NETWORK_LEDGER_REASON: &str = "Not synchronized with network yet";
pub const SERVER_OKAY_AMENDMENT_BLOCKED_REASON: &str = "Server version too old";
pub const SERVER_OKAY_UNL_BLOCKED_REASON: &str = "No valid validator list available";
pub const SERVER_OKAY_NOT_SYNCED_REASON: &str = "Not synchronized with network";
pub const SERVER_OKAY_TOO_MUCH_LOAD_REASON: &str = "Too much load";

pub fn server_okay(
    elb_support: bool,
    stop_tree: &StopTree,
    network_ops_state: &SharedNetworkOpsState,
    ledger_master: LedgerMasterCaughtUp,
    local_load: bool,
) -> Result<(), &'static str> {
    if !elb_support {
        return Ok(());
    }

    if stop_tree.is_stopping() {
        return Err(SERVER_OKAY_STOPPING_REASON);
    }

    if network_ops_state.need_network_ledger() {
        return Err(SERVER_OKAY_NEED_NETWORK_LEDGER_REASON);
    }

    if network_ops_state.amendment_blocked() {
        return Err(SERVER_OKAY_AMENDMENT_BLOCKED_REASON);
    }

    if network_ops_state.unl_blocked() {
        return Err(SERVER_OKAY_UNL_BLOCKED_REASON);
    }

    if network_ops_state.operating_mode() < NetworkOpsOperatingMode::Syncing {
        return Err(SERVER_OKAY_NOT_SYNCED_REASON);
    }

    if let LedgerMasterCaughtUp::No { reason } = ledger_master {
        return Err(reason);
    }

    if local_load {
        return Err(SERVER_OKAY_TOO_MUCH_LOAD_REASON);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        SERVER_OKAY_AMENDMENT_BLOCKED_REASON, SERVER_OKAY_NEED_NETWORK_LEDGER_REASON,
        SERVER_OKAY_NOT_SYNCED_REASON, SERVER_OKAY_STOPPING_REASON,
        SERVER_OKAY_TOO_MUCH_LOAD_REASON, SERVER_OKAY_UNL_BLOCKED_REASON, server_okay,
    };
    use crate::network::network_ops::{NetworkOpsOperatingMode, SharedNetworkOpsState};
    use crate::state::stop_tree::StopTree;
    use ledger::LedgerMasterCaughtUp;

    #[test]
    fn server_okay_matches_current_application_gate_order() {
        let stop_tree = StopTree::new("application");
        let state = SharedNetworkOpsState::new(NetworkOpsOperatingMode::Disconnected);

        assert_eq!(
            server_okay(true, &stop_tree, &state, LedgerMasterCaughtUp::Yes, false),
            Err(SERVER_OKAY_NOT_SYNCED_REASON)
        );

        state.set_need_network_ledger(true);
        assert_eq!(
            server_okay(true, &stop_tree, &state, LedgerMasterCaughtUp::Yes, false),
            Err(SERVER_OKAY_NEED_NETWORK_LEDGER_REASON)
        );

        state.set_need_network_ledger(false);
        state.set_amendment_blocked(true);
        assert_eq!(
            server_okay(true, &stop_tree, &state, LedgerMasterCaughtUp::Yes, false),
            Err(SERVER_OKAY_AMENDMENT_BLOCKED_REASON)
        );

        state.set_amendment_blocked(false);
        state.set_unl_blocked(true);
        assert_eq!(
            server_okay(true, &stop_tree, &state, LedgerMasterCaughtUp::Yes, false),
            Err(SERVER_OKAY_UNL_BLOCKED_REASON)
        );
    }

    #[test]
    fn server_okay_uses_ledger_master_and_load_after_sync_checks() {
        let stop_tree = StopTree::new("application");
        let state = SharedNetworkOpsState::new(NetworkOpsOperatingMode::Full);

        assert_eq!(
            server_okay(
                true,
                &stop_tree,
                &state,
                LedgerMasterCaughtUp::No {
                    reason: "No published ledger",
                },
                false
            ),
            Err("No published ledger")
        );

        assert_eq!(
            server_okay(true, &stop_tree, &state, LedgerMasterCaughtUp::Yes, true),
            Err(SERVER_OKAY_TOO_MUCH_LOAD_REASON)
        );

        assert_eq!(
            server_okay(true, &stop_tree, &state, LedgerMasterCaughtUp::Yes, false),
            Ok(())
        );
    }

    #[test]
    fn server_okay_short_circuits_for_disabled_elb_and_stopping_state() {
        let stop_tree = StopTree::new("application");
        assert!(stop_tree.signal_stop("shutdown"));
        let state = SharedNetworkOpsState::new(NetworkOpsOperatingMode::Full);

        assert_eq!(
            server_okay(true, &stop_tree, &state, LedgerMasterCaughtUp::Yes, false),
            Err(SERVER_OKAY_STOPPING_REASON)
        );

        assert_eq!(
            server_okay(
                false,
                &stop_tree,
                &state,
                LedgerMasterCaughtUp::No {
                    reason: "No published ledger",
                },
                true
            ),
            Ok(())
        );
    }
}
