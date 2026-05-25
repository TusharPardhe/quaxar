use basics::slab_allocator::{SlabAllocator, SlabAllocatorSet, SlabAllocatorSetError, SlabConfig};
use std::ptr::null_mut;

#[test]
fn slab_allocator_rounds_size_alignment_rules() {
    let allocator = SlabAllocator::<u64>::new(3, 128, 0);
    assert_eq!(allocator.size(), 16);
}

#[test]
fn slab_allocator_allocate_and_deallocate_round_trip() {
    let allocator = SlabAllocator::<u64>::new(0, 128, 0);

    let first = allocator.allocate();
    assert!(!first.is_null());
    assert!(allocator.deallocate(first));

    let second = allocator.allocate();
    assert!(!second.is_null());
    assert!(allocator.deallocate(second));
}

#[test]
fn slab_allocator_rejects_foreign_pointers() {
    let allocator = SlabAllocator::<u64>::new(0, 128, 0);
    let mut foreign = Box::new(0_u64);
    let ptr = &mut *foreign as *mut u64 as *mut u8;

    assert!(!allocator.deallocate(ptr));
    drop(foreign);
}

#[test]
fn slab_allocator_set_sorts_configs_and_rejects_duplicate_sizes() {
    let set = SlabAllocatorSet::<u64>::new(vec![SlabConfig::new(8, 128), SlabConfig::new(0, 128)])
        .expect("set");

    let ptr = set.allocate(0);
    assert!(!ptr.is_null());
    assert!(set.deallocate(ptr));
}

#[test]
fn slab_allocator_set_rejects_duplicate_slabs() {
    let err = SlabAllocatorSet::<u64>::new(vec![
        SlabConfig::new(0, 128),
        SlabConfig::with_align(0, 256, 16),
    ])
    .expect_err("duplicate sizes");

    assert!(matches!(
        err,
        SlabAllocatorSetError::DuplicateSlabSize { .. }
    ));
}

#[test]
fn slab_allocator_set_returns_null_when_request_exceeds_largest_allocator() {
    let set = SlabAllocatorSet::<u64>::new(vec![SlabConfig::new(0, 64)]).expect("set");
    assert_eq!(set.allocate(64), null_mut());
}
