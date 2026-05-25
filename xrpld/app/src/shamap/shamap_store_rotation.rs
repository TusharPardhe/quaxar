use crate::SHAMapStoreHealthStatus;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SHAMapStoreRotationDecision {
    pub last_rotated: u32,
    pub ready_to_rotate: bool,
}

pub fn initialize_last_rotated(last_rotated: u32, validated_seq: u32) -> u32 {
    if last_rotated == 0 {
        validated_seq
    } else {
        last_rotated
    }
}

pub fn rotation_ready(
    validated_seq: u32,
    last_rotated: u32,
    delete_interval: u32,
    can_delete: u32,
    health: SHAMapStoreHealthStatus,
) -> bool {
    delete_interval != 0
        && validated_seq >= last_rotated.saturating_add(delete_interval)
        && can_delete >= last_rotated.saturating_sub(1)
        && health == SHAMapStoreHealthStatus::KeepGoing
}

#[cfg(test)]
mod tests {
    use super::{initialize_last_rotated, rotation_ready};
    use crate::SHAMapStoreHealthStatus;

    #[test]
    fn rotation_helpers_match_cpp_ready_to_rotate_rules() {
        assert_eq!(initialize_last_rotated(0, 900), 900);
        assert_eq!(initialize_last_rotated(800, 900), 800);
        assert!(rotation_ready(
            1024,
            768,
            256,
            767,
            SHAMapStoreHealthStatus::KeepGoing
        ));
        assert!(!rotation_ready(
            1024,
            768,
            256,
            766,
            SHAMapStoreHealthStatus::KeepGoing
        ));
    }
}
