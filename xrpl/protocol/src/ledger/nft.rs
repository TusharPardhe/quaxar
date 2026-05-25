//! Narrow NFT helpers from `xrpl/protocol/nft.h`.

use basics::{base_uint::Uint256, tagged_integer::TaggedInteger};

use crate::AccountID;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct TaxonTag;

pub type Taxon = TaggedInteger<u32, TaxonTag>;

pub const FLAG_BURNABLE: u16 = 0x0001;
pub const FLAG_ONLY_XRP: u16 = 0x0002;
pub const FLAG_CREATE_TRUST_LINES: u16 = 0x0004;
pub const FLAG_TRANSFERABLE: u16 = 0x0008;
pub const FLAG_MUTABLE: u16 = 0x0010;

pub fn to_taxon(value: u32) -> Taxon {
    Taxon::new(value)
}

pub fn to_u32(taxon: Taxon) -> u32 {
    taxon.value()
}

pub fn get_flags(id: Uint256) -> u16 {
    u16::from_be_bytes(id.data()[..2].try_into().expect("flags width"))
}

pub fn get_transfer_fee(id: Uint256) -> u16 {
    u16::from_be_bytes(id.data()[2..4].try_into().expect("fee width"))
}

pub fn get_serial(id: Uint256) -> u32 {
    u32::from_be_bytes(id.data()[28..32].try_into().expect("serial width"))
}

pub fn ciphered_taxon(token_seq: u32, taxon: Taxon) -> Taxon {
    taxon ^ Taxon::new((384_160_001u32.wrapping_mul(token_seq)).wrapping_add(2459))
}

pub fn get_taxon(id: Uint256) -> Taxon {
    let taxon = u32::from_be_bytes(id.data()[24..28].try_into().expect("taxon width"));
    ciphered_taxon(get_serial(id), Taxon::new(taxon))
}

pub fn get_issuer(id: Uint256) -> AccountID {
    AccountID::from_slice(&id.data()[4..24]).expect("issuer width")
}
