use consensus::{ConsensusParms, DisputedTx};

#[derive(Debug, Clone, PartialEq, Eq)]
struct MockTx {
    id: u32,
}

#[test]
fn dispute_stalled_matches_non_proposing_threshold_logic() {
    let parms = ConsensusParms::default();
    let mut dispute = DisputedTx::new(MockTx { id: 1 }, 1u32, false);

    for peer in 1..=5 {
        assert!(dispute.set_vote(peer, true));
    }
    assert!(dispute.set_vote(6, false));

    for _ in 0..8 {
        let _ = dispute.update_vote(200, false, &parms);
    }

    assert!(dispute.stalled(&parms, false, 0));
}
