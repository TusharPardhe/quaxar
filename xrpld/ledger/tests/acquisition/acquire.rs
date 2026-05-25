use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::sync::Arc;

use basics::base_uint::Uint256;
use ledger::{
    Fees, InboundLedgerReason, Ledger, LedgerConfig, LedgerDeltaAcquire, SkipListAcquire,
    TransactionAcquire,
};
use overlay::{Peer, PeerImp, PeerSet, ProtocolPayload, SimplePeerSet};
use protocol::{FeatureSet, PublicKey};
use shamap::{node_id::SHAMapNodeId, sync::SHAMapAddNode};

fn test_peer(id: u32) -> Arc<PeerImp> {
    PeerImp::new(
        id,
        SocketAddr::from(([127, 0, 0, 1], 5100 + id as u16)),
        PublicKey::from_bytes([0x02; 33]),
        format!("peer-{id}"),
    )
}

fn config() -> LedgerConfig {
    LedgerConfig::new(
        Fees {
            base: 10,
            reserve: 20,
            increment: 30,
        },
        FeatureSet::new([]),
    )
}

#[test]
fn transaction_acquire_requests_root_candidate() {
    let hash = Uint256::from_array([0xAA; 32]);
    let peer = test_peer(1);
    peer.record_tx_set(hash);
    let peer_dyn: Arc<dyn Peer> = peer.clone();

    let peer_set: Arc<dyn PeerSet> = Arc::new(SimplePeerSet::new(vec![peer_dyn]));
    let mut acquire = TransactionAcquire::new(hash, peer_set);
    acquire.init(1);

    let queued = peer.queued_messages();
    assert_eq!(queued.len(), 1);

    match &queued[0].protocol().payload {
        ProtocolPayload::GetLedger(message) => {
            assert_eq!(message.itype, 3);
            assert_eq!(message.ledger_hash.as_deref(), Some(hash.data().as_slice()));
            assert_eq!(message.node_i_ds.len(), 1);
            assert_eq!(message.query_depth, Some(3));
        }
        other => panic!("expected TMGetLedger request, got {other:?}"),
    }
}

#[test]
fn transaction_acquire_ignores_bad_root_node() {
    let hash = Uint256::from_array([0xAB; 32]);
    let peer_set: Arc<dyn PeerSet> = Arc::new(SimplePeerSet::new(Vec::new()));
    let mut acquire = TransactionAcquire::new(hash, peer_set);

    let result = acquire.take_nodes(&[(SHAMapNodeId::default(), vec![0xDE, 0xAD])], None);

    assert_eq!(result, SHAMapAddNode::useful());
    assert!(!acquire.has_root());
    assert!(!acquire.is_failed());
}

#[test]
fn skip_list_acquire_prefers_local_ledger_skip_list() {
    let genesis = Ledger::create_genesis(false, &config(), []).expect("genesis should build");
    let mut next = Ledger::from_previous(&genesis, 10);
    next.update_skip_list().expect("skip list should update");
    let next = Arc::new(next);

    let mut acquire = SkipListAcquire::new(
        *next.header().hash.as_uint256(),
        Arc::new(SimplePeerSet::new(Vec::new())),
    );

    acquire.init(
        1,
        &mut |hash| {
            if hash == *next.header().hash.as_uint256() {
                Some(Arc::clone(&next))
            } else {
                None
            }
        },
        &mut |_, _, _| panic!("local ledger path should not fall back"),
    );

    let data = acquire.get_data().expect("skip list should be available");
    assert!(acquire.is_complete());
    assert_eq!(data.ledger_seq, next.header().seq);
    assert_eq!(data.skip_list, vec![*genesis.header().hash.as_uint256()]);
}

#[test]
fn delta_acquire_builds_replay_and_surfaces_reasons() {
    let cfg = config();
    let parent = Arc::new(Ledger::create_genesis(false, &cfg, []).expect("genesis should build"));
    let replay = Arc::new(Ledger::from_previous(&parent, 10));
    let hash = *replay.header().hash.as_uint256();

    let mut delta = LedgerDeltaAcquire::new(
        hash,
        replay.header().seq,
        Arc::new(SimplePeerSet::new(Vec::new())),
    );
    delta.add_data_reason(InboundLedgerReason::Generic);
    delta.process_data(replay.header(), BTreeMap::new(), &cfg);

    let built = delta
        .try_build(&parent, &mut |replay_data| {
            Ok::<Arc<Ledger>, ()>(Arc::clone(replay_data.replay()))
        })
        .expect("replay builder should succeed")
        .expect("verified replay should build immediately");

    assert_eq!(built.header().hash, replay.header().hash);
    assert_eq!(
        delta.drain_ready_reasons(),
        vec![InboundLedgerReason::Generic]
    );
}

#[test]
fn replay_acquires_ignore_featureless_peers() {
    let ledger_hash = Uint256::from_array([0xCC; 32]);
    let ledger_seq = 11;
    let peer = test_peer(3);
    peer.record_ledger(ledger_hash, 0);
    peer.record_ledger(ledger_hash, ledger_seq);
    let peer_dyn: Arc<dyn Peer> = peer.clone();
    let peer_set: Arc<dyn PeerSet> = Arc::new(SimplePeerSet::new(vec![peer_dyn]));

    let mut skip_acquire = SkipListAcquire::new(ledger_hash, Arc::clone(&peer_set));
    let mut lookup_ledger = |_| None;
    let mut fallback_calls = 0usize;
    {
        let mut fallback = |_, _, _| fallback_calls += 1;
        skip_acquire.init(1, &mut lookup_ledger, &mut fallback);
        skip_acquire.invoke_on_timer(&mut lookup_ledger, &mut fallback);
    }

    assert_eq!(fallback_calls, 0);
    assert!(!skip_acquire.is_fallback());
    assert!(peer.queued_messages().is_empty());

    let mut delta_acquire = LedgerDeltaAcquire::new(ledger_hash, ledger_seq, Arc::clone(&peer_set));
    {
        let mut fallback = |_, _, _| fallback_calls += 1;
        delta_acquire.init(1, &mut lookup_ledger, &mut fallback);
        delta_acquire.invoke_on_timer(&mut lookup_ledger, &mut fallback);
    }

    assert_eq!(fallback_calls, 0);
    assert!(!delta_acquire.is_complete());
    assert!(!delta_acquire.is_failed());
    assert!(peer.queued_messages().is_empty());
}

#[test]
fn transaction_and_replay_acquires_stop_shutdown() {
    let hash = Uint256::from_array([0xBB; 32]);
    let peer = test_peer(2);
    let peer_dyn: Arc<dyn Peer> = peer.clone();
    let peer_set: Arc<dyn PeerSet> = Arc::new(SimplePeerSet::new(vec![peer_dyn]));

    let mut tx_acquire = TransactionAcquire::new(hash, Arc::clone(&peer_set));
    tx_acquire.stop();
    tx_acquire.init(1);
    assert!(tx_acquire.is_stopped());
    assert!(tx_acquire.is_done());
    assert!(peer.queued_messages().is_empty());

    let mut skip_acquire = SkipListAcquire::new(hash, Arc::clone(&peer_set));
    skip_acquire.stop();
    skip_acquire.init(
        1,
        &mut |_| panic!("stopped skip-list acquire should not consult local ledger"),
        &mut |_, _, _| panic!("stopped skip-list acquire should not fall back"),
    );
    assert!(skip_acquire.is_stopped());
    assert!(skip_acquire.is_done());

    let cfg = config();
    let parent = Arc::new(Ledger::create_genesis(false, &cfg, []).expect("genesis should build"));
    let replay = Arc::new(Ledger::from_previous(&parent, 10));
    let mut delta = LedgerDeltaAcquire::new(
        *replay.header().hash.as_uint256(),
        replay.header().seq,
        Arc::new(SimplePeerSet::new(Vec::new())),
    );
    delta.stop();
    delta.add_data_reason(InboundLedgerReason::Generic);
    delta.process_data(replay.header(), BTreeMap::new(), &cfg);
    assert!(delta.is_stopped());
    assert!(delta.is_done());
    assert!(
        delta
            .try_build(&parent, &mut |replay_data| Ok::<Arc<Ledger>, ()>(
                Arc::clone(replay_data.replay())
            ))
            .expect("stopped delta build should not fail")
            .is_none()
    );
}
