//! Rust port of `xrpl::CachedSLEs` from `xrpl/ledger/CachedSLEs.h`.

use basics::base_uint::Uint256;
use basics::tagged_cache::{MonotonicClock, TaggedCache};

pub type CachedSles<C = MonotonicClock> = TaggedCache<Uint256, protocol::STLedgerEntry, C>;
