//! Tests for the SubscriptionManager (subscribe, unsubscribe, publish).

use std::collections::BTreeMap;

use protocol::JsonValue;
use rpc::{InfoSub, RpcRole, SubscriptionManager, SubscriptionMessage, SubscriptionStream};

fn object(entries: impl IntoIterator<Item = (&'static str, JsonValue)>) -> JsonValue {
    JsonValue::Object(
        entries
            .into_iter()
            .map(|(key, value)| (key.to_owned(), value))
            .collect::<BTreeMap<_, _>>(),
    )
}

#[test]
fn subscription_manager_fans_out_named_stream_events_streams() {
    let manager = SubscriptionManager::new();
    let mut session = InfoSub::with_identity(RpcRole::Identified, "alice", "203.0.113.9");

    let mut receiver = manager
        .subscribe(&mut session, SubscriptionStream::Transactions)
        .expect("stream exists");
    assert!(session.is_subscribed(SubscriptionStream::Transactions));

    let payload = object([
        ("ledger_index", JsonValue::Unsigned(42)),
        ("validated", JsonValue::Bool(true)),
    ]);
    let sent = manager
        .publish(SubscriptionStream::Transactions, payload.clone())
        .expect("publish succeeds");
    assert_eq!(sent, 1);

    let message = receiver.try_recv().expect("message should be queued");
    assert_eq!(
        message,
        SubscriptionMessage {
            stream: SubscriptionStream::Transactions,
            payload,
        }
    );

    assert!(manager.unsubscribe(&mut session, SubscriptionStream::Transactions));
    assert!(!session.is_subscribed(SubscriptionStream::Transactions));
}

#[test]
fn subscription_manager_subscribe_and_unsubscribe_tracking() {
    let manager = SubscriptionManager::new();
    let mut session = InfoSub::new(RpcRole::Admin);

    assert!(!session.is_subscribed(SubscriptionStream::Ledger));
    assert!(!session.is_subscribed(SubscriptionStream::Transactions));
    assert!(!session.is_subscribed(SubscriptionStream::Server));

    let _ = manager.subscribe(&mut session, SubscriptionStream::Ledger);
    assert!(session.is_subscribed(SubscriptionStream::Ledger));
    assert!(!session.is_subscribed(SubscriptionStream::Transactions));

    let _ = manager.subscribe(&mut session, SubscriptionStream::Transactions);
    assert!(session.is_subscribed(SubscriptionStream::Transactions));

    manager.unsubscribe(&mut session, SubscriptionStream::Ledger);
    assert!(!session.is_subscribed(SubscriptionStream::Ledger));
    assert!(session.is_subscribed(SubscriptionStream::Transactions));

    manager.unsubscribe(&mut session, SubscriptionStream::Transactions);
    assert!(!session.is_subscribed(SubscriptionStream::Transactions));
}

#[test]
fn subscription_manager_multiple_streams_independent() {
    let manager = SubscriptionManager::new();
    let mut session = InfoSub::new(RpcRole::Admin);

    let _ = manager.subscribe(&mut session, SubscriptionStream::Ledger);
    let _ = manager.subscribe(&mut session, SubscriptionStream::Transactions);
    let _ = manager.subscribe(&mut session, SubscriptionStream::Server);

    assert!(session.is_subscribed(SubscriptionStream::Ledger));
    assert!(session.is_subscribed(SubscriptionStream::Transactions));
    assert!(session.is_subscribed(SubscriptionStream::Server));

    manager.unsubscribe(&mut session, SubscriptionStream::Ledger);
    assert!(!session.is_subscribed(SubscriptionStream::Ledger));
    assert!(session.is_subscribed(SubscriptionStream::Transactions));
    assert!(session.is_subscribed(SubscriptionStream::Server));
}
