use app::LoadFeeControl;
use app::{LOAD_FEE_NORMAL, SharedLoadFeeTrack};

#[test]
fn load_fee_track_raise_and_lower_local_fee() {
    let track = SharedLoadFeeTrack::new();

    assert!(!track.raise_local_fee());
    assert_eq!(track.local_fee(), LOAD_FEE_NORMAL);

    assert!(track.raise_local_fee());
    assert_eq!(track.local_fee(), 320);
    assert!(track.is_loaded_local());

    assert!(track.lower_local_fee());
    assert_eq!(track.local_fee(), LOAD_FEE_NORMAL);
    assert!(!track.is_loaded_local());
}

#[test]
fn load_fee_track_scaling_factors_match_cpp() {
    let track = SharedLoadFeeTrack::new();
    track.set_remote_fee(400);
    track.set_cluster_fee(384);

    assert_eq!(track.load_factor(), 400);
    assert_eq!(track.scaling_factors(), (400, 400));
    assert!(track.is_loaded_cluster());

    assert!(!track.raise_local_fee());
    assert!(track.raise_local_fee());
    assert_eq!(track.local_fee(), 500);
    assert_eq!(track.load_factor(), 500);
    assert_eq!(track.scaling_factors(), (500, 400));
    assert!(track.is_loaded_cluster());
}
