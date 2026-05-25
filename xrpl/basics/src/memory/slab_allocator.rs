//! Compatibility wrapper for `xrpl/basics/SlabAllocator.h`.
//!
//! The reference surface is a small slab allocator with fixed-size blocks and a
//! higher-level set that chooses the smallest allocator capable of satisfying
//! a request. This Rust port keeps the same public shape and the same
//! allocation/deallocation contracts, while using Rust ownership to clean up
//! the backing memory automatically.

use std::alloc::{Layout, alloc, dealloc};
use std::any::type_name;
use std::error::Error;
use std::fmt;
use std::marker::PhantomData;
use std::mem::size_of;
use std::ptr::null_mut;
use std::sync::Mutex;

const MAX_ALLOCATORS: usize = 64;

fn align_up(value: usize, align: usize) -> usize {
    debug_assert!(align.is_power_of_two());
    value.saturating_add(align - 1) & !(align - 1)
}

#[derive(Debug)]
struct SlabBlock {
    memory: std::ptr::NonNull<u8>,
    layout: Layout,
    free_list: Vec<*mut u8>,
}

impl SlabBlock {
    fn new(slab_size: usize, item_size: usize, item_alignment: usize) -> Option<Self> {
        if slab_size == 0 {
            return None;
        }

        let allocation_alignment = item_alignment.max(size_of::<usize>());
        let allocation_size = align_up(slab_size, allocation_alignment);
        if allocation_size < item_size {
            return None;
        }

        let layout = Layout::from_size_align(allocation_size, allocation_alignment).ok()?;
        let memory = {
            // SAFETY: The layout is valid and non-zero sized.
            let ptr = unsafe { alloc(layout) };
            std::ptr::NonNull::new(ptr)?
        };

        let mut free_list = Vec::new();
        let start = memory.as_ptr() as usize;
        let end = start.saturating_add(allocation_size);
        let mut cursor = start;
        while cursor.saturating_add(item_size) <= end {
            free_list.push(cursor as *mut u8);
            cursor += item_size;
        }

        if free_list.is_empty() {
            // SAFETY: The memory came from `alloc` with the same layout.
            unsafe { dealloc(memory.as_ptr(), layout) };
            return None;
        }

        Some(Self {
            memory,
            layout,
            free_list,
        })
    }

    fn owns(&self, ptr: *const u8) -> bool {
        if ptr.is_null() {
            return false;
        }

        let start = self.memory.as_ptr() as usize;
        let end = start + self.layout.size();
        let addr = ptr as usize;
        (start..end).contains(&addr)
    }

    fn allocate(&mut self) -> *mut u8 {
        self.free_list.pop().unwrap_or(null_mut())
    }

    fn deallocate(&mut self, ptr: *mut u8) {
        debug_assert!(self.owns(ptr));
        self.free_list.push(ptr);
    }
}

impl Drop for SlabBlock {
    fn drop(&mut self) {
        // SAFETY: `memory` was allocated with this exact layout.
        unsafe { dealloc(self.memory.as_ptr(), self.layout) };
    }
}

#[derive(Debug)]
struct SlabAllocatorState {
    blocks: Vec<SlabBlock>,
}

/// A slab allocator for fixed-size objects of `Type`.
#[derive(Debug)]
pub struct SlabAllocator<Type> {
    item_alignment: usize,
    item_size: usize,
    slab_size: usize,
    state: Mutex<SlabAllocatorState>,
    _marker: PhantomData<fn() -> Type>,
}

impl<Type> SlabAllocator<Type> {
    /// Construct a slab allocator.
    ///
    /// `extra` is added to `size_of::<Type>()` before alignment. `alloc`
    /// controls the size of each backing slab. A zero `align` means "use the
    /// natural alignment of `Type`".
    pub fn new(extra: usize, alloc: usize, align: usize) -> Self {
        assert!(
            size_of::<Type>() >= size_of::<*const u8>(),
            "SlabAllocator: the requested object must be larger than a pointer."
        );
        assert!(
            matches!(std::mem::align_of::<Type>(), 4 | 8),
            "SlabAllocator: the requested object must have alignment 4 or 8."
        );

        let item_alignment = if align == 0 {
            std::mem::align_of::<Type>()
        } else {
            align
        };
        assert!(
            item_alignment.is_power_of_two(),
            "xrpl::SlabAllocator::SlabAllocator : valid alignment"
        );

        let item_size = align_up(size_of::<Type>().saturating_add(extra), item_alignment);
        Self {
            item_alignment,
            item_size,
            slab_size: alloc,
            state: Mutex::new(SlabAllocatorState { blocks: Vec::new() }),
            _marker: PhantomData,
        }
    }

    /// Returns the size of the memory block returned by this allocator.
    pub fn size(&self) -> usize {
        self.item_size
    }

    /// Returns a suitably aligned pointer, if one is available.
    pub fn allocate(&self) -> *mut u8 {
        let mut state = self.state.lock().expect("slab allocator mutex poisoned");

        for block in &mut state.blocks {
            let ptr = block.allocate();
            if !ptr.is_null() {
                return ptr;
            }
        }

        let Some(mut block) = SlabBlock::new(self.slab_size, self.item_size, self.item_alignment)
        else {
            return null_mut();
        };

        let ptr = block.allocate();
        if ptr.is_null() {
            return null_mut();
        }

        state.blocks.push(block);
        ptr
    }

    /// Returns the memory block to the allocator.
    pub fn deallocate(&self, ptr: *mut u8) -> bool {
        assert!(
            !ptr.is_null(),
            "xrpl::SlabAllocator::SlabAllocator::deallocate : non-null input"
        );

        let mut state = self.state.lock().expect("slab allocator mutex poisoned");
        for block in &mut state.blocks {
            if block.owns(ptr) {
                block.deallocate(ptr);
                return true;
            }
        }

        false
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SlabConfig {
    extra: usize,
    alloc: usize,
    align: usize,
}

impl SlabConfig {
    pub const fn new(extra: usize, alloc: usize) -> Self {
        Self {
            extra,
            alloc,
            align: 0,
        }
    }

    pub const fn with_align(extra: usize, alloc: usize, align: usize) -> Self {
        Self {
            extra,
            alloc,
            align,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SlabAllocatorSetError {
    DuplicateSlabSize { type_name: &'static str },
    TooManyAllocators { count: usize, max: usize },
}

impl fmt::Display for SlabAllocatorSetError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DuplicateSlabSize { type_name } => {
                write!(f, "SlabAllocatorSet<{type_name}>: duplicate slab size")
            }
            Self::TooManyAllocators { count, max } => {
                write!(
                    f,
                    "SlabAllocatorSet: {count} allocators exceeds capacity {max}"
                )
            }
        }
    }
}

impl Error for SlabAllocatorSetError {}

/// A collection of slab allocators of various sizes for a given type.
#[derive(Debug)]
pub struct SlabAllocatorSet<Type> {
    allocators: Vec<SlabAllocator<Type>>,
    max_size: usize,
}

impl<Type> SlabAllocatorSet<Type> {
    pub fn new(mut cfg: Vec<SlabConfig>) -> Result<Self, SlabAllocatorSetError> {
        cfg.sort_by_key(|c| c.extra);

        if cfg.len() > MAX_ALLOCATORS {
            return Err(SlabAllocatorSetError::TooManyAllocators {
                count: cfg.len(),
                max: MAX_ALLOCATORS,
            });
        }

        if cfg.windows(2).any(|pair| pair[0].extra == pair[1].extra) {
            return Err(SlabAllocatorSetError::DuplicateSlabSize {
                type_name: type_name::<Type>(),
            });
        }

        let mut max_size = 0;
        let mut allocators = Vec::with_capacity(cfg.len());
        for config in cfg {
            let allocator = SlabAllocator::<Type>::new(config.extra, config.alloc, config.align);
            max_size = max_size.max(allocator.size());
            allocators.push(allocator);
        }

        Ok(Self {
            allocators,
            max_size,
        })
    }

    pub fn allocate(&self, extra: usize) -> *mut u8 {
        let requested = size_of::<Type>().saturating_add(extra);
        if requested > self.max_size {
            return null_mut();
        }

        for allocator in &self.allocators {
            if allocator.size() >= requested {
                let ptr = allocator.allocate();
                if !ptr.is_null() {
                    return ptr;
                }
            }
        }

        null_mut()
    }

    pub fn deallocate(&self, ptr: *mut u8) -> bool {
        for allocator in &self.allocators {
            if allocator.deallocate(ptr) {
                return true;
            }
        }

        false
    }
}
