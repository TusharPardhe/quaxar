//! Validates that the node state machine transitions correctly:
//! Disconnected → Connected → Syncing → Tracking → Full
//!
//! This proves the node will properly advance through states when
//! participating in the network.

use app::{
    NetworkOpsOperatingMode, SharedNetworkOpsState, normalize_operating_mode_for_validated_age,
};
use std::time::Duration;

/// Test: State ordering is correct (Disconnected < Connected < Syncing < Tracking < Full)
#[test]
fn operating_mode_ordering() {
    assert!(NetworkOpsOperatingMode::Disconnected < NetworkOpsOperatingMode::Connected);
    assert!(NetworkOpsOperatingMode::Connected < NetworkOpsOperatingMode::Syncing);
    assert!(NetworkOpsOperatingMode::Syncing < NetworkOpsOperatingMode::Tracking);
    assert!(NetworkOpsOperatingMode::Tracking < NetworkOpsOperatingMode::Full);
}

/// Test: SharedNetworkOpsState tracks mode transitions correctly.
#[test]
fn shared_state_tracks_mode_transitions() {
    let state = SharedNetworkOpsState::new(NetworkOpsOperatingMode::Disconnected);

    assert_eq!(
        state.operating_mode(),
        NetworkOpsOperatingMode::Disconnected
    );

    state.set_operating_mode(NetworkOpsOperatingMode::Connected);
    assert_eq!(state.operating_mode(), NetworkOpsOperatingMode::Connected);

    state.set_operating_mode(NetworkOpsOperatingMode::Syncing);
    assert_eq!(state.operating_mode(), NetworkOpsOperatingMode::Syncing);

    state.set_operating_mode(NetworkOpsOperatingMode::Tracking);
    assert_eq!(state.operating_mode(), NetworkOpsOperatingMode::Tracking);

    state.set_operating_mode(NetworkOpsOperatingMode::Full);
    assert_eq!(state.operating_mode(), NetworkOpsOperatingMode::Full);
}

/// Test: Connected + fresh validated ledger → Syncing
#[test]
fn connected_with_fresh_ledger_becomes_syncing() {
    let fresh_age = Duration::from_secs(59); // < 60 seconds threshold
    let result = normalize_operating_mode_for_validated_age(
        NetworkOpsOperatingMode::Connected,
        fresh_age,
        false,
    );
    assert_eq!(result, NetworkOpsOperatingMode::Syncing);
}

/// Test: Syncing + stale validated ledger → Connected
#[test]
fn syncing_with_stale_ledger_becomes_connected() {
    let stale_age = Duration::from_secs(600); // > 5 minutes
    let result = normalize_operating_mode_for_validated_age(
        NetworkOpsOperatingMode::Syncing,
        stale_age,
        false,
    );
    assert_eq!(result, NetworkOpsOperatingMode::Connected);
}

/// Test: Any mode above Connected + blocked → Connected
#[test]
fn blocked_node_drops_to_connected() {
    for mode in [
        NetworkOpsOperatingMode::Syncing,
        NetworkOpsOperatingMode::Tracking,
        NetworkOpsOperatingMode::Full,
    ] {
        let result =
            normalize_operating_mode_for_validated_age(mode, Duration::from_secs(30), true);
        assert_eq!(
            result,
            NetworkOpsOperatingMode::Connected,
            "{:?} should drop to Connected when blocked",
            mode
        );
    }
}

/// Test: Full mode stays Full when not blocked and ledger is fresh
#[test]
fn full_stays_full_when_healthy() {
    let result = normalize_operating_mode_for_validated_age(
        NetworkOpsOperatingMode::Full,
        Duration::from_secs(30),
        false,
    );
    assert_eq!(result, NetworkOpsOperatingMode::Full);
}

/// Test: Tracking stays Tracking when not blocked
#[test]
fn tracking_stays_tracking_when_healthy() {
    let result = normalize_operating_mode_for_validated_age(
        NetworkOpsOperatingMode::Tracking,
        Duration::from_secs(30),
        false,
    );
    assert_eq!(result, NetworkOpsOperatingMode::Tracking);
}

/// Test: Disconnected is never promoted by normalize alone
#[test]
fn disconnected_stays_disconnected() {
    let result = normalize_operating_mode_for_validated_age(
        NetworkOpsOperatingMode::Disconnected,
        Duration::from_secs(1),
        false,
    );
    assert_eq!(result, NetworkOpsOperatingMode::Disconnected);
}

/// Test: Mode string representations match expected values
#[test]
fn mode_string_representations() {
    assert_eq!(
        NetworkOpsOperatingMode::Disconnected.as_str(),
        "disconnected"
    );
    assert_eq!(NetworkOpsOperatingMode::Connected.as_str(), "connected");
    assert_eq!(NetworkOpsOperatingMode::Syncing.as_str(), "syncing");
    assert_eq!(NetworkOpsOperatingMode::Tracking.as_str(), "tracking");
    assert_eq!(NetworkOpsOperatingMode::Full.as_str(), "full");
}
