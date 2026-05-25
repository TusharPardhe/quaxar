//! Digest helpers from `xrpl/protocol/digest.h`.

use basics::base_uint::Uint256;
use ripemd::{Digest as RipemdDigest, Ripemd160};
use sha2::{Sha256, Sha512};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EndianOrder {
    Native,
    Big,
}

pub type Ripemd160Digest = [u8; 20];
pub type Sha256Digest = [u8; 32];
pub type Sha512Digest = [u8; 64];

fn secure_erase_value<T>(value: &mut T) {
    // Match the reference secure_erase intent for digest state buffers on drop.
    unsafe {
        std::ptr::write_bytes(value as *mut T as *mut u8, 0, std::mem::size_of::<T>());
    }
}

#[derive(Clone)]
pub struct OpensslRipemd160Hasher {
    state: Ripemd160,
}

impl OpensslRipemd160Hasher {
    pub const ENDIAN: EndianOrder = EndianOrder::Native;

    pub fn new() -> Self {
        Self {
            state: Ripemd160::new(),
        }
    }

    pub fn update(&mut self, data: impl AsRef<[u8]>) {
        self.state.update(data.as_ref());
    }

    pub fn write(&mut self, data: impl AsRef<[u8]>) {
        self.update(data);
    }

    pub fn result(self) -> Ripemd160Digest {
        self.state.finalize().into()
    }
}

impl Default for OpensslRipemd160Hasher {
    fn default() -> Self {
        Self::new()
    }
}

impl From<OpensslRipemd160Hasher> for Ripemd160Digest {
    fn from(value: OpensslRipemd160Hasher) -> Self {
        value.result()
    }
}

#[derive(Clone)]
pub struct OpensslSha512Hasher {
    state: Sha512,
}

impl OpensslSha512Hasher {
    pub const ENDIAN: EndianOrder = EndianOrder::Native;

    pub fn new() -> Self {
        Self {
            state: Sha512::new(),
        }
    }

    pub fn update(&mut self, data: impl AsRef<[u8]>) {
        self.state.update(data.as_ref());
    }

    pub fn write(&mut self, data: impl AsRef<[u8]>) {
        self.update(data);
    }

    pub fn result(self) -> Sha512Digest {
        self.state.finalize().into()
    }
}

impl Default for OpensslSha512Hasher {
    fn default() -> Self {
        Self::new()
    }
}

impl From<OpensslSha512Hasher> for Sha512Digest {
    fn from(value: OpensslSha512Hasher) -> Self {
        value.result()
    }
}

#[derive(Clone)]
pub struct OpensslSha256Hasher {
    state: Sha256,
}

impl OpensslSha256Hasher {
    pub const ENDIAN: EndianOrder = EndianOrder::Native;

    pub fn new() -> Self {
        Self {
            state: Sha256::new(),
        }
    }

    pub fn update(&mut self, data: impl AsRef<[u8]>) {
        self.state.update(data.as_ref());
    }

    pub fn write(&mut self, data: impl AsRef<[u8]>) {
        self.update(data);
    }

    pub fn result(self) -> Sha256Digest {
        self.state.finalize().into()
    }
}

impl Default for OpensslSha256Hasher {
    fn default() -> Self {
        Self::new()
    }
}

impl From<OpensslSha256Hasher> for Sha256Digest {
    fn from(value: OpensslSha256Hasher) -> Self {
        value.result()
    }
}

pub type Ripemd160Hasher = OpensslRipemd160Hasher;
pub type Sha256Hasher = OpensslSha256Hasher;
pub type Sha512Hasher = OpensslSha512Hasher;

#[derive(Clone)]
pub struct RipeshaHasher {
    state: Sha256Hasher,
}

impl RipeshaHasher {
    pub const ENDIAN: EndianOrder = EndianOrder::Native;

    pub fn new() -> Self {
        Self {
            state: Sha256Hasher::new(),
        }
    }

    pub fn update(&mut self, data: impl AsRef<[u8]>) {
        self.state.update(data.as_ref());
    }

    pub fn write(&mut self, data: impl AsRef<[u8]>) {
        self.update(data);
    }

    pub fn result(self) -> Ripemd160Digest {
        ripemd160_digest(self.state.result())
    }
}

impl Default for RipeshaHasher {
    fn default() -> Self {
        Self::new()
    }
}

impl From<RipeshaHasher> for Ripemd160Digest {
    fn from(value: RipeshaHasher) -> Self {
        value.result()
    }
}

#[derive(Clone)]
pub struct BasicSha512HalfHasher<const SECURE: bool> {
    state: Sha512Hasher,
    scratch: Sha512Digest,
}

impl<const SECURE: bool> BasicSha512HalfHasher<SECURE> {
    pub const ENDIAN: EndianOrder = EndianOrder::Big;

    pub fn new() -> Self {
        Self {
            state: Sha512Hasher::new(),
            scratch: [0; 64],
        }
    }

    pub fn update(&mut self, data: impl AsRef<[u8]>) {
        self.state.update(data.as_ref());
    }

    pub fn write(&mut self, data: impl AsRef<[u8]>) {
        self.update(data);
    }

    pub fn result(mut self) -> Uint256 {
        self.scratch = std::mem::take(&mut self.state).result();
        Uint256::from_slice(&self.scratch[..32]).expect("sha512 half width")
    }
}

impl<const SECURE: bool> Default for BasicSha512HalfHasher<SECURE> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const SECURE: bool> From<BasicSha512HalfHasher<SECURE>> for Uint256 {
    fn from(value: BasicSha512HalfHasher<SECURE>) -> Self {
        value.result()
    }
}

impl<const SECURE: bool> Drop for BasicSha512HalfHasher<SECURE> {
    fn drop(&mut self) {
        if SECURE {
            self.scratch.fill(0);
            secure_erase_value(&mut self.state);
        }
    }
}

pub type Sha512HalfHasher = BasicSha512HalfHasher<false>;
pub type Sha512HalfHasherS = BasicSha512HalfHasher<true>;

pub fn ripemd160_digest(data: impl AsRef<[u8]>) -> Ripemd160Digest {
    let mut hasher = Ripemd160Hasher::new();
    hasher.update(data);
    hasher.result()
}

pub fn sha256_digest(data: impl AsRef<[u8]>) -> Sha256Digest {
    let mut hasher = Sha256Hasher::new();
    hasher.update(data);
    hasher.result()
}

pub fn sha512_digest(data: impl AsRef<[u8]>) -> Sha512Digest {
    let mut hasher = Sha512Hasher::new();
    hasher.update(data);
    hasher.result()
}

pub fn ripesha(data: impl AsRef<[u8]>) -> Ripemd160Digest {
    let mut hasher = RipeshaHasher::new();
    hasher.update(data);
    hasher.result()
}

pub fn sha512_half(data: impl AsRef<[u8]>) -> Uint256 {
    let mut hasher = Sha512HalfHasher::new();
    hasher.update(data);
    hasher.result()
}

pub fn sha512_half_slices(parts: &[&[u8]]) -> Uint256 {
    let mut hasher = Sha512HalfHasher::new();
    for part in parts {
        hasher.write(part);
    }
    hasher.result()
}

pub fn sha512_half_secure(data: impl AsRef<[u8]>) -> Uint256 {
    let mut hasher = Sha512HalfHasherS::new();
    hasher.update(data);
    hasher.result()
}

pub fn calculate_ledger_object_id(prefix: crate::HashPrefix, data: impl AsRef<[u8]>) -> Uint256 {
    let mut hasher = Sha512HalfHasher::new();
    hasher.write(prefix.as_u32().to_be_bytes());
    hasher.write(data);
    hasher.result()
}

#[cfg(test)]
mod tests {
    use basics::base_uint::Uint256;

    use super::{
        EndianOrder, OpensslSha256Hasher, OpensslSha512Hasher, Ripemd160Digest, RipeshaHasher,
        Sha256Digest, Sha512Digest, Sha512HalfHasher, sha256_digest, sha512_digest, sha512_half,
        sha512_half_secure, sha512_half_slices,
    };

    #[test]
    fn digest_hashers_match_function_helpers() {
        let payload = b"xrpl";

        let mut sha256 = OpensslSha256Hasher::new();
        sha256.write(&payload[..2]);
        sha256.write(&payload[2..]);
        assert_eq!(Sha256Digest::from(sha256), sha256_digest(payload));

        let mut ripesha = RipeshaHasher::new();
        ripesha.write(payload);
        assert_eq!(Ripemd160Digest::from(ripesha), super::ripesha(payload));

        let mut sha512 = OpensslSha512Hasher::new();
        sha512.write(payload);
        assert_eq!(Sha512Digest::from(sha512), sha512_digest(payload));

        let mut half = Sha512HalfHasher::new();
        half.write(payload);
        assert_eq!(Uint256::from(half), sha512_half(payload));
        assert_eq!(
            sha512_half_slices(&[&payload[..2], &payload[2..]]),
            sha512_half(payload)
        );
        assert_eq!(sha512_half_secure(payload), sha512_half(payload));
        assert_eq!(Sha512HalfHasher::ENDIAN, EndianOrder::Big);
    }
}
