//! `xrpl/shamap/SHAMapItem.h` compatibility surface.

use basics::base_uint::Uint256;
use basics::blob::Blob;
use basics::byte_utilities::megabytes;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SHAMapItem {
    key: Uint256,
    data: Blob,
}

impl SHAMapItem {
    pub fn new(key: Uint256, data: impl Into<Blob>) -> Self {
        let data = data.into();
        assert!(
            data.len() <= megabytes::<usize>(16),
            "SHAMapItem data must not exceed 16 MB"
        );

        Self { key, data }
    }

    pub fn key(&self) -> Uint256 {
        self.key
    }

    pub fn size(&self) -> usize {
        self.data.len()
    }

    pub fn data(&self) -> &[u8] {
        &self.data
    }
}
