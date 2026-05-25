use basics::base_uint::Uint256;
use basics::sha_map_hash::SHAMapHash;

pub fn sample_uint256(fill: u8) -> Uint256 {
    Uint256::from_array([fill; 32])
}

pub fn sample_hash(fill: u8) -> SHAMapHash {
    SHAMapHash::new(sample_uint256(fill))
}
