//! Rust port of `xrpl/basics/random.h`.
//!
//! This module keeps the important reference contracts:
//! - a deterministic default non-cryptographic engine type,
//! - a thread-local default PRNG handle,
//! - closed-interval integer sampling helpers,
//! - byte and boolean helpers built on top of integer sampling.
//!
//! Rust does not support reference-style function overloading, so the helper family
//! is split into explicit names like `rand_int_range_with` and `rand_int_to`.

use rand::RngCore;
use rand::distributions::uniform::SampleUniform;
use rand::distributions::{Distribution, Uniform};
use std::cell::RefCell;
use std::fmt;
use std::sync::{Mutex, OnceLock};

/// Error raised when attempting to seed the engine with zero.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InvalidSeed;

impl fmt::Display for InvalidSeed {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("invalid seed")
    }
}

/// Exact port of `beast::xor_shift_engine`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct XorShiftEngine {
    state: [u64; 2],
}

impl XorShiftEngine {
    pub const DEFAULT_SEED: u64 = 1977;

    pub fn try_new(seed: u64) -> Result<Self, InvalidSeed> {
        let mut engine = Self { state: [0, 0] };
        engine.try_seed(seed)?;
        Ok(engine)
    }

    pub fn new(seed: u64) -> Self {
        Self::try_new(seed).expect("XorShiftEngine seed must be non-zero")
    }

    pub fn try_seed(&mut self, seed: u64) -> Result<(), InvalidSeed> {
        if seed == 0 {
            return Err(InvalidSeed);
        }

        self.state[0] = murmurhash3(seed);
        self.state[1] = murmurhash3(self.state[0]);
        Ok(())
    }

    pub fn seed(&mut self, seed: u64) {
        self.try_seed(seed)
            .expect("XorShiftEngine seed must be non-zero");
    }

    pub const fn min() -> u64 {
        u64::MIN
    }

    pub const fn max() -> u64 {
        u64::MAX
    }
}

impl Default for XorShiftEngine {
    fn default() -> Self {
        Self::new(Self::DEFAULT_SEED)
    }
}

impl RngCore for XorShiftEngine {
    fn next_u32(&mut self) -> u32 {
        self.next_u64() as u32
    }

    fn next_u64(&mut self) -> u64 {
        let mut s1 = self.state[0];
        let s0 = self.state[1];
        self.state[0] = s0;
        s1 ^= s1 << 23;
        let next = s1 ^ s0 ^ (s1 >> 17) ^ (s0 >> 26);
        self.state[1] = next;
        next.wrapping_add(s0)
    }

    fn fill_bytes(&mut self, dest: &mut [u8]) {
        let mut chunks = dest.chunks_exact_mut(8);
        for chunk in &mut chunks {
            chunk.copy_from_slice(&self.next_u64().to_le_bytes());
        }

        let remainder = chunks.into_remainder();
        if !remainder.is_empty() {
            let tail = self.next_u64().to_le_bytes();
            remainder.copy_from_slice(&tail[..remainder.len()]);
        }
    }

    fn try_fill_bytes(&mut self, dest: &mut [u8]) -> Result<(), rand::Error> {
        self.fill_bytes(dest);
        Ok(())
    }
}

fn murmurhash3(mut value: u64) -> u64 {
    value ^= value >> 33;
    value = value.wrapping_mul(0xff51_afd7_ed55_8ccd);
    value ^= value >> 33;
    value = value.wrapping_mul(0xc4ce_b9fe_1a85_ec53);
    value ^ (value >> 33)
}

thread_local! {
    static DEFAULT_PRNG_ENGINE: RefCell<Option<XorShiftEngine>> = const { RefCell::new(None) };
}

static SEEDER: OnceLock<Mutex<XorShiftEngine>> = OnceLock::new();

#[derive(Clone, Copy, Debug, Default)]
pub struct DefaultPrng;

pub fn default_prng() -> DefaultPrng {
    DefaultPrng
}

impl RngCore for DefaultPrng {
    fn next_u32(&mut self) -> u32 {
        with_default_engine(|engine| engine.next_u32())
    }

    fn next_u64(&mut self) -> u64 {
        with_default_engine(|engine| engine.next_u64())
    }

    fn fill_bytes(&mut self, dest: &mut [u8]) {
        with_default_engine(|engine| engine.fill_bytes(dest));
    }

    fn try_fill_bytes(&mut self, dest: &mut [u8]) -> Result<(), rand::Error> {
        with_default_engine(|engine| engine.try_fill_bytes(dest))
    }
}

pub fn rand_int_range_with<R, T>(engine: &mut R, min: T, max: T) -> T
where
    R: RngCore + ?Sized,
    T: SampleUniform + PartialOrd + Copy,
{
    assert!(max > min, "xrpl::rand_int : max over min inputs");
    Uniform::new_inclusive(min, max).sample(engine)
}

pub fn rand_int_range<T>(min: T, max: T) -> T
where
    T: SampleUniform + PartialOrd + Copy,
{
    let mut engine = default_prng();
    rand_int_range_with(&mut engine, min, max)
}

pub fn rand_int_to_with<R, T>(engine: &mut R, max: T) -> T
where
    R: RngCore + ?Sized,
    T: SampleUniform + PartialOrd + Copy + Default,
{
    rand_int_range_with(engine, T::default(), max)
}

pub fn rand_int_to<T>(max: T) -> T
where
    T: SampleUniform + PartialOrd + Copy + Default,
{
    let mut engine = default_prng();
    rand_int_to_with(&mut engine, max)
}

pub fn rand_int_full_with<R, T>(engine: &mut R) -> T
where
    R: RngCore + ?Sized,
    T: SampleUniform + PartialOrd + Copy + Default + BoundedInteger,
{
    rand_int_to_with(engine, T::max_value())
}

pub fn rand_int_full<T>() -> T
where
    T: SampleUniform + PartialOrd + Copy + Default + BoundedInteger,
{
    let mut engine = default_prng();
    rand_int_full_with(&mut engine)
}

pub fn rand_byte_with<R>(engine: &mut R) -> u8
where
    R: RngCore + ?Sized,
{
    rand_int_range_with(engine, u8::MIN, u8::MAX)
}

pub fn rand_byte() -> u8 {
    let mut engine = default_prng();
    rand_byte_with(&mut engine)
}

pub fn rand_bool_with<R>(engine: &mut R) -> bool
where
    R: RngCore + ?Sized,
{
    rand_int_to_with(engine, 1u8) == 1
}

pub fn rand_bool() -> bool {
    let mut engine = default_prng();
    rand_bool_with(&mut engine)
}

pub trait BoundedInteger {
    fn max_value() -> Self;
}

macro_rules! impl_bounded_integer {
    ($($ty:ty),* $(,)?) => {
        $(
            impl BoundedInteger for $ty {
                fn max_value() -> Self {
                    <$ty>::MAX
                }
            }
        )*
    };
}

impl_bounded_integer!(
    u8, u16, u32, u64, u128, usize, i8, i16, i32, i64, i128, isize
);

fn with_default_engine<T>(f: impl FnOnce(&mut XorShiftEngine) -> T) -> T {
    DEFAULT_PRNG_ENGINE.with(|cell| {
        let mut slot = cell.borrow_mut();
        let engine = slot.get_or_insert_with(seed_thread_local_engine);
        f(engine)
    })
}

fn seed_thread_local_engine() -> XorShiftEngine {
    let mut seeder = seeder()
        .lock()
        .expect("default PRNG seeder mutex must not be poisoned");
    let seed = rand_int_range_with(&mut *seeder, 1u64, u64::MAX);
    XorShiftEngine::new(seed)
}

fn seeder() -> &'static Mutex<XorShiftEngine> {
    SEEDER.get_or_init(|| {
        let seed = rand::random::<u64>().max(1);
        Mutex::new(XorShiftEngine::new(seed))
    })
}

#[cfg(test)]
mod tests {
    use super::{
        DefaultPrng, InvalidSeed, XorShiftEngine, default_prng, rand_bool_with, rand_byte_with,
        rand_int_full_with, rand_int_range_with, rand_int_to_with,
    };
    use rand::RngCore;
    use std::collections::HashSet;

    #[test]
    fn xor_shift_engine_reference_sequence_for_default_seed() {
        let mut rng = XorShiftEngine::new(1977);
        let expected = [
            3_238_484_970_499_989_659,
            18_388_379_945_704_714_460,
            6_940_444_787_602_738_416,
            10_403_977_402_942_764_970,
            5_603_373_218_716_299_795,
        ];

        for value in expected {
            assert_eq!(rng.next_u64(), value);
        }
    }

    #[test]
    fn xor_shift_engine_reference_sequence_for_seed_one() {
        let mut rng = XorShiftEngine::new(1);
        let expected = [
            3_787_875_997_830_008_111,
            7_110_081_793_310_507_210,
            15_613_677_437_415_376_376,
            6_427_185_811_373_906_065,
            17_589_699_481_160_154_643,
        ];

        for value in expected {
            assert_eq!(rng.next_u64(), value);
        }
    }

    #[test]
    fn zero_seed_is_rejected() {
        assert_eq!(XorShiftEngine::try_new(0), Err(InvalidSeed));
    }

    #[test]
    fn rand_int_helpers_stay_within_closed_ranges() {
        let mut rng = XorShiftEngine::new(1977);

        for _ in 0..256 {
            let ranged = rand_int_range_with(&mut rng, -5i32, 15i32);
            assert!((-5..=15).contains(&ranged));

            let to = rand_int_to_with(&mut rng, 7u32);
            assert!((0..=7).contains(&to));

            let full: u32 = rand_int_full_with(&mut rng);
            let _ = full;
        }
    }

    #[test]
    fn rand_byte_and_rand_bool_match_reference_role() {
        let mut rng = XorShiftEngine::new(1977);
        let mut seen = HashSet::new();

        for _ in 0..256 {
            seen.insert(rand_bool_with(&mut rng));
            let byte = rand_byte_with(&mut rng);
            let _ = byte;
        }

        assert!(seen.contains(&true));
        assert!(seen.contains(&false));
    }

    #[test]
    fn default_prng_is_a_rng_handle() {
        let mut rng = default_prng();
        let first = rng.next_u64();
        let second = rng.next_u64();

        assert_ne!(first, second);
    }

    #[test]
    fn default_prng_handle_is_zero_sized() {
        assert_eq!(std::mem::size_of::<DefaultPrng>(), 0);
    }
}
