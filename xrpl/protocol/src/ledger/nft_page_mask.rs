//! NFT page mask constant from `xrpl/protocol/nftPageMask.h`.

use basics::base_uint::Uint256;

pub fn page_mask() -> Uint256 {
    Uint256::from_hex("0000000000000000000000000000000000000000FFFFFFFFFFFFFFFFFFFFFFFF")
        .expect("NFT page mask hex should remain valid")
}
