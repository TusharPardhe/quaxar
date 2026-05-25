//! Compatibility re-export for `xrpl/basics/KeyCache.h`.

use crate::base_uint::Uint256;
use crate::hardened_hash::HardenedHashBuilder;

pub use crate::tagged_cache::{ManualClock, MonotonicClock};

pub type KeyCache<C = MonotonicClock, S = HardenedHashBuilder> =
    crate::tagged_cache::KeyCache<Uint256, C, S>;
