use crate::{
    SHAMapStoreHealthStatus, SHAMapStoreRotationDecision, initialize_last_rotated, rotation_ready,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SHAMapStoreRunLoopStep {
    pub validated_seq: u32,
    pub decision: SHAMapStoreRotationDecision,
}

pub fn runloop_step(
    validated_seq: u32,
    last_rotated: u32,
    delete_interval: u32,
    can_delete: u32,
    health: SHAMapStoreHealthStatus,
) -> SHAMapStoreRunLoopStep {
    let last_rotated = initialize_last_rotated(last_rotated, validated_seq);
    SHAMapStoreRunLoopStep {
        validated_seq,
        decision: SHAMapStoreRotationDecision {
            last_rotated,
            ready_to_rotate: rotation_ready(
                validated_seq,
                last_rotated,
                delete_interval,
                can_delete,
                health,
            ),
        },
    }
}
