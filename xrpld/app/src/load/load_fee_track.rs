//! Narrow `LoadFeeTrack` port for the current app-runtime shell.

use crate::load::load_manager::LoadFeeControl;
use std::cmp::max;
use std::sync::Mutex;

pub const LOAD_FEE_NORMAL: u32 = 256;
pub const LOAD_FEE_INC_FRACTION: u32 = 4;
pub const LOAD_FEE_DEC_FRACTION: u32 = 4;
pub const LOAD_FEE_MAX: u32 = LOAD_FEE_NORMAL * 1_000_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LoadFeeTrackSnapshot {
    pub local_fee: u32,
    pub remote_fee: u32,
    pub cluster_fee: u32,
    pub raise_count: u32,
}

#[derive(Debug)]
struct LoadFeeTrackInner {
    local_fee: u32,
    remote_fee: u32,
    cluster_fee: u32,
    raise_count: u32,
}

#[derive(Debug)]
pub struct SharedLoadFeeTrack {
    inner: Mutex<LoadFeeTrackInner>,
}

impl Default for SharedLoadFeeTrack {
    fn default() -> Self {
        Self::new()
    }
}

impl SharedLoadFeeTrack {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(LoadFeeTrackInner {
                local_fee: LOAD_FEE_NORMAL,
                remote_fee: LOAD_FEE_NORMAL,
                cluster_fee: LOAD_FEE_NORMAL,
                raise_count: 0,
            }),
        }
    }

    pub const fn load_base(&self) -> u32 {
        LOAD_FEE_NORMAL
    }

    pub fn snapshot(&self) -> LoadFeeTrackSnapshot {
        let inner = self.inner.lock().expect("load fee track mutex");
        LoadFeeTrackSnapshot {
            local_fee: inner.local_fee,
            remote_fee: inner.remote_fee,
            cluster_fee: inner.cluster_fee,
            raise_count: inner.raise_count,
        }
    }

    pub fn set_remote_fee(&self, fee: u32) {
        self.inner.lock().expect("load fee track mutex").remote_fee = fee;
    }

    pub fn remote_fee(&self) -> u32 {
        self.inner.lock().expect("load fee track mutex").remote_fee
    }

    pub fn local_fee(&self) -> u32 {
        self.inner.lock().expect("load fee track mutex").local_fee
    }

    pub fn set_cluster_fee(&self, fee: u32) {
        self.inner.lock().expect("load fee track mutex").cluster_fee = fee;
    }

    pub fn cluster_fee(&self) -> u32 {
        self.inner.lock().expect("load fee track mutex").cluster_fee
    }

    pub fn load_factor(&self) -> u32 {
        let inner = self.inner.lock().expect("load fee track mutex");
        max(max(inner.cluster_fee, inner.local_fee), inner.remote_fee)
    }

    pub fn scaling_factors(&self) -> (u32, u32) {
        let inner = self.inner.lock().expect("load fee track mutex");
        (
            max(inner.local_fee, inner.remote_fee),
            max(inner.remote_fee, inner.cluster_fee),
        )
    }

    pub fn is_loaded_local(&self) -> bool {
        let inner = self.inner.lock().expect("load fee track mutex");
        inner.raise_count != 0 || inner.local_fee != LOAD_FEE_NORMAL
    }

    pub fn is_loaded_cluster(&self) -> bool {
        let inner = self.inner.lock().expect("load fee track mutex");
        inner.raise_count != 0
            || inner.local_fee != LOAD_FEE_NORMAL
            || inner.cluster_fee != LOAD_FEE_NORMAL
    }
}

impl LoadFeeControl for SharedLoadFeeTrack {
    fn raise_local_fee(&self) -> bool {
        let mut inner = self.inner.lock().expect("load fee track mutex");
        inner.raise_count = inner.raise_count.saturating_add(1);
        if inner.raise_count < 2 {
            return false;
        }

        let original = inner.local_fee;
        inner.local_fee = max(inner.local_fee, inner.remote_fee);
        inner.local_fee = inner
            .local_fee
            .saturating_add(inner.local_fee / LOAD_FEE_INC_FRACTION);
        inner.local_fee = inner.local_fee.min(LOAD_FEE_MAX);
        if original != inner.local_fee {
            tracing::info!(target: "network", fee_level = inner.local_fee, "Fee escalation triggered");
        }
        original != inner.local_fee
    }

    fn lower_local_fee(&self) -> bool {
        let mut inner = self.inner.lock().expect("load fee track mutex");
        let original = inner.local_fee;
        inner.raise_count = 0;
        inner.local_fee = inner
            .local_fee
            .saturating_sub(inner.local_fee / LOAD_FEE_DEC_FRACTION)
            .max(LOAD_FEE_NORMAL);
        original != inner.local_fee
    }
}

#[cfg(test)]
mod tests {
    use super::{LOAD_FEE_NORMAL, SharedLoadFeeTrack};
    use crate::load::load_manager::LoadFeeControl;

    #[test]
    fn load_fee_track_matches_raise_and_lower_rules() {
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
    fn load_fee_track_uses_remote_and_cluster_factors() {
        let track = SharedLoadFeeTrack::new();
        track.set_remote_fee(400);
        track.set_cluster_fee(384);

        assert_eq!(track.load_factor(), 400);
        assert_eq!(track.scaling_factors(), (400, 400));

        assert!(!track.raise_local_fee());
        assert!(track.raise_local_fee());
        assert_eq!(track.local_fee(), 500);
        assert_eq!(track.load_factor(), 500);
        assert!(track.is_loaded_cluster());
    }
}
