//! Serializer and SerialIter ported from `xrpl/protocol/Serializer.*`.

use std::fmt;

use basics::{
    base_uint::{BaseUInt, Uint128, Uint160, Uint192, Uint256},
    blob::Blob,
    buffer::Buffer,
    slice::Slice,
};
use sha2::{Digest, Sha512};

use crate::{HashPrefix, SerializedTypeId};

pub trait SerializerInteger: Copy {
    const BYTES: usize;

    fn write_be(self, out: &mut Blob);
    fn read_be(bytes: &[u8]) -> Self;
}

impl SerializerInteger for u8 {
    const BYTES: usize = 1;

    fn write_be(self, out: &mut Blob) {
        out.push(self);
    }

    fn read_be(bytes: &[u8]) -> Self {
        bytes[0]
    }
}

impl SerializerInteger for u16 {
    const BYTES: usize = 2;

    fn write_be(self, out: &mut Blob) {
        out.extend_from_slice(&self.to_be_bytes());
    }

    fn read_be(bytes: &[u8]) -> Self {
        Self::from_be_bytes(bytes.try_into().expect("u16 byte width"))
    }
}

impl SerializerInteger for u32 {
    const BYTES: usize = 4;

    fn write_be(self, out: &mut Blob) {
        out.extend_from_slice(&self.to_be_bytes());
    }

    fn read_be(bytes: &[u8]) -> Self {
        Self::from_be_bytes(bytes.try_into().expect("u32 byte width"))
    }
}

impl SerializerInteger for i32 {
    const BYTES: usize = 4;

    fn write_be(self, out: &mut Blob) {
        out.extend_from_slice(&self.to_be_bytes());
    }

    fn read_be(bytes: &[u8]) -> Self {
        Self::from_be_bytes(bytes.try_into().expect("i32 byte width"))
    }
}

impl SerializerInteger for u64 {
    const BYTES: usize = 8;

    fn write_be(self, out: &mut Blob) {
        out.extend_from_slice(&self.to_be_bytes());
    }

    fn read_be(bytes: &[u8]) -> Self {
        Self::from_be_bytes(bytes.try_into().expect("u64 byte width"))
    }
}

impl SerializerInteger for i64 {
    const BYTES: usize = 8;

    fn write_be(self, out: &mut Blob) {
        out.extend_from_slice(&self.to_be_bytes());
    }

    fn read_be(bytes: &[u8]) -> Self {
        Self::from_be_bytes(bytes.try_into().expect("i64 byte width"))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Serializer {
    data: Blob,
}

impl Serializer {
    const DEFAULT_CAPACITY: usize = 256;

    pub fn new(capacity_hint: usize) -> Self {
        Self {
            data: Blob::with_capacity(capacity_hint),
        }
    }

    pub fn from_bytes(bytes: impl AsRef<[u8]>) -> Self {
        Self {
            data: bytes.as_ref().to_vec(),
        }
    }

    pub fn slice(&self) -> Slice<'_> {
        Slice::new(&self.data)
    }

    pub fn size(&self) -> usize {
        self.data.len()
    }

    pub fn data(&self) -> &[u8] {
        &self.data
    }

    pub fn data_ptr(&self) -> *const u8 {
        self.data.as_ptr()
    }

    pub fn add8(&mut self, value: u8) -> i32 {
        let ret = self.data.len() as i32;
        self.data.push(value);
        ret
    }

    pub fn add16(&mut self, value: u16) -> i32 {
        let ret = self.data.len() as i32;
        self.data.extend_from_slice(&value.to_be_bytes());
        ret
    }

    pub fn add32(&mut self, value: u32) -> i32 {
        let ret = self.data.len() as i32;
        self.data.extend_from_slice(&value.to_be_bytes());
        ret
    }

    pub fn add32_prefix(&mut self, prefix: HashPrefix) -> i32 {
        self.add32(prefix.as_u32())
    }

    pub fn add64(&mut self, value: u64) -> i32 {
        let ret = self.data.len() as i32;
        self.data.extend_from_slice(&value.to_be_bytes());
        ret
    }

    pub fn add_integer<T: SerializerInteger>(&mut self, value: T) -> i32 {
        let ret = self.data.len() as i32;
        value.write_be(&mut self.data);
        ret
    }

    pub fn add_bit_string<const BYTES: usize, Tag>(&mut self, value: BaseUInt<BYTES, Tag>) -> i32 {
        self.add_raw(value.data())
    }

    pub fn add_raw(&mut self, bytes: impl AsRef<[u8]>) -> i32 {
        let ret = self.data.len() as i32;
        self.data.extend_from_slice(bytes.as_ref());
        ret
    }

    pub fn add_raw_slice(&mut self, slice: Slice<'_>) -> i32 {
        self.add_raw(slice.data())
    }

    pub fn add_raw_serializer(&mut self, serializer: &Self) -> i32 {
        self.add_raw(serializer.data())
    }

    pub fn add_vl(&mut self, bytes: impl AsRef<[u8]>) -> i32 {
        let bytes = bytes.as_ref();
        let ret = self.add_encoded(bytes.len());
        if !bytes.is_empty() {
            self.add_raw(bytes);
        }
        ret
    }

    pub fn add_vl_slice(&mut self, slice: Slice<'_>) -> i32 {
        self.add_vl(slice.data())
    }

    pub fn add_vl_chunks<I, B>(&mut self, chunks: I, len: usize) -> i32
    where
        I: IntoIterator<Item = B>,
        B: AsRef<[u8]>,
    {
        let ret = self.add_encoded(len);
        let mut remaining = len;
        for chunk in chunks {
            let bytes = chunk.as_ref();
            self.add_raw(bytes);
            remaining = remaining
                .checked_sub(bytes.len())
                .expect("xrpl::Serializer::addVL : length matches distance");
        }
        assert_eq!(
            remaining, 0,
            "xrpl::Serializer::addVL : length matches distance"
        );
        ret
    }

    pub fn get8(&self, out: &mut i32, offset: usize) -> bool {
        match self.data.get(offset) {
            Some(byte) => {
                *out = i32::from(*byte);
                true
            }
            None => false,
        }
    }

    pub fn get_integer<T: SerializerInteger>(&self, out: &mut T, offset: usize) -> bool {
        if offset + T::BYTES > self.data.len() {
            return false;
        }
        *out = T::read_be(&self.data[offset..offset + T::BYTES]);
        true
    }

    pub fn get_bit_string<const BYTES: usize, Tag>(
        &self,
        out: &mut BaseUInt<BYTES, Tag>,
        offset: usize,
    ) -> bool {
        if offset + BYTES > self.data.len() {
            return false;
        }
        *out = BaseUInt::from_slice(&self.data[offset..offset + BYTES]).expect("fixed width slice");
        true
    }

    pub fn add_field_id(&mut self, type_: i32, name: i32) -> i32 {
        if !(1..256).contains(&type_) || !(1..256).contains(&name) {
            return 0;
        }

        let ret = self.data.len() as i32;
        if type_ < 16 {
            if name < 16 {
                self.data.push(((type_ << 4) | name) as u8);
            } else {
                self.data.push((type_ << 4) as u8);
                self.data.push(name as u8);
            }
        } else if name < 16 {
            self.data.push(name as u8);
            self.data.push(type_ as u8);
        } else {
            self.data.push(0);
            self.data.push(type_ as u8);
            self.data.push(name as u8);
        }
        ret
    }

    pub fn add_field_type_id(&mut self, type_: SerializedTypeId, name: i32) -> i32 {
        self.add_field_id(type_.as_i32(), name)
    }

    pub fn get_sha512_half(&self) -> Uint256 {
        let digest = Sha512::digest(&self.data);
        Uint256::from_slice(&digest[..32]).expect("sha512 half width")
    }

    pub fn peek_data(&self) -> &Blob {
        &self.data
    }

    pub fn get_data(&self) -> Blob {
        self.data.clone()
    }

    pub fn mod_data(&mut self) -> &mut Blob {
        &mut self.data
    }

    pub fn get_data_length(&self) -> i32 {
        self.data.len() as i32
    }

    pub fn get_length(&self) -> i32 {
        self.data.len() as i32
    }

    pub fn erase(&mut self) {
        self.data.clear();
    }

    pub fn chop(&mut self, bytes: usize) -> bool {
        if bytes > self.data.len() {
            return false;
        }
        self.data.truncate(self.data.len() - bytes);
        true
    }

    pub fn reserve(&mut self, capacity: usize) {
        if capacity > self.data.capacity() {
            self.data.reserve(capacity - self.data.capacity());
        }
    }

    pub fn resize(&mut self, size: usize) {
        self.data.resize(size, 0);
    }

    pub fn capacity(&self) -> usize {
        self.data.capacity()
    }

    pub fn decode_length_length(b1: i32) -> i32 {
        if b1 < 0 {
            panic!("b1<0");
        }
        if b1 <= 192 {
            return 1;
        }
        if b1 <= 240 {
            return 2;
        }
        if b1 <= 254 {
            return 3;
        }
        panic!("b1>254");
    }

    pub fn decode_vl_length_1(b1: i32) -> i32 {
        if b1 < 0 {
            panic!("b1<0");
        }
        if b1 > 254 {
            panic!("b1>254");
        }
        b1
    }

    pub fn decode_vl_length_2(b1: i32, b2: i32) -> i32 {
        if b1 < 193 {
            panic!("b1<193");
        }
        if b1 > 240 {
            panic!("b1>240");
        }
        193 + ((b1 - 193) * 256) + b2
    }

    pub fn decode_vl_length_3(b1: i32, b2: i32, b3: i32) -> i32 {
        if b1 < 241 {
            panic!("b1<241");
        }
        if b1 > 254 {
            panic!("b1>254");
        }
        12481 + ((b1 - 241) * 65536) + (b2 * 256) + b3
    }

    fn encode_length_length(length: usize) -> usize {
        if length <= 192 {
            return 1;
        }
        if length <= 12_480 {
            return 2;
        }
        if length <= 918_744 {
            return 3;
        }
        panic!("len>918744");
    }

    fn add_encoded(&mut self, length: usize) -> i32 {
        let ret = self.data.len() as i32;
        if length <= 192 {
            self.data.push(length as u8);
        } else if length <= 12_480 {
            let adjusted = length - 193;
            self.data.push((193 + (adjusted >> 8)) as u8);
            self.data.push((adjusted & 0xff) as u8);
        } else if length <= 918_744 {
            let adjusted = length - 12_481;
            self.data.push((241 + (adjusted >> 16)) as u8);
            self.data.push(((adjusted >> 8) & 0xff) as u8);
            self.data.push((adjusted & 0xff) as u8);
        } else {
            panic!("lenlen");
        }

        debug_assert_eq!(
            self.data.len(),
            ret as usize + Self::encode_length_length(length)
        );
        ret
    }
}

impl Default for Serializer {
    fn default() -> Self {
        Self::new(Self::DEFAULT_CAPACITY)
    }
}

impl PartialEq<Blob> for Serializer {
    fn eq(&self, other: &Blob) -> bool {
        &self.data == other
    }
}

impl PartialEq<Serializer> for Blob {
    fn eq(&self, other: &Serializer) -> bool {
        self == &other.data
    }
}

#[derive(Clone, Copy)]
pub struct SerialIter<'a> {
    data: &'a [u8],
    used: usize,
}

impl<'a> fmt::Debug for SerialIter<'a> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SerialIter")
            .field("used", &self.used)
            .field("remain", &self.get_bytes_left())
            .finish()
    }
}

impl<'a> SerialIter<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Self { data, used: 0 }
    }

    pub fn from_slice(slice: Slice<'a>) -> Self {
        Self::new(slice.data())
    }

    pub fn empty(&self) -> bool {
        self.used == self.data.len()
    }

    pub fn reset(&mut self) {
        self.used = 0;
    }

    pub fn get_bytes_left(&self) -> i32 {
        (self.data.len() - self.used) as i32
    }

    pub fn skip(&mut self, length: usize) {
        if self.data.len() - self.used < length {
            self.used = self.data.len();
            return;
        }
        self.used += length;
    }

    pub fn get8(&mut self) -> u8 {
        if self.data.len().saturating_sub(self.used) < 1 {
            tracing::trace!(target: "protocol", "SerialIter underflow — returning default");
            return 0;
        }
        let value = self.data[self.used];
        self.used += 1;
        value
    }

    pub fn get16(&mut self) -> u16 {
        self.read_integer::<u16>("invalid SerialIter get16")
    }

    pub fn get32(&mut self) -> u32 {
        self.read_integer::<u32>("invalid SerialIter get32")
    }

    pub fn geti32(&mut self) -> i32 {
        self.read_integer::<i32>("invalid SerialIter geti32")
    }

    pub fn get64(&mut self) -> u64 {
        self.read_integer::<u64>("invalid SerialIter get64")
    }

    pub fn geti64(&mut self) -> i64 {
        self.read_integer::<i64>("invalid SerialIter geti64")
    }

    pub fn get_bit_string<const BYTES: usize, Tag>(&mut self) -> BaseUInt<BYTES, Tag> {
        if self.data.len().saturating_sub(self.used) < BYTES {
            return BaseUInt::default();
        }
        let value = BaseUInt::from_slice(&self.data[self.used..self.used + BYTES])
            .expect("fixed width slice");
        self.used += BYTES;
        value
    }

    pub fn get128(&mut self) -> Uint128 {
        self.get_bit_string()
    }

    pub fn get160(&mut self) -> Uint160 {
        self.get_bit_string()
    }

    pub fn get192(&mut self) -> Uint192 {
        self.get_bit_string()
    }

    pub fn get256(&mut self) -> Uint256 {
        self.get_bit_string()
    }

    pub fn get_field_id(&mut self, type_out: &mut i32, name_out: &mut i32) {
        *type_out = i32::from(self.get8());
        *name_out = *type_out & 15;
        *type_out >>= 4;

        if *type_out == 0 {
            *type_out = i32::from(self.get8());
            if *type_out < 16 {
                *type_out = -1;
                return;
            }
        }

        if *name_out == 0 {
            *name_out = i32::from(self.get8());
            if *name_out < 16 {
                *name_out = -1;
            }
        }
    }

    pub fn get_vl_data_length(&mut self) -> i32 {
        let b1 = i32::from(self.get8());
        let len_len = Serializer::decode_length_length(b1);
        match len_len {
            1 => Serializer::decode_vl_length_1(b1),
            2 => {
                let b2 = i32::from(self.get8());
                Serializer::decode_vl_length_2(b1, b2)
            }
            3 => {
                let b2 = i32::from(self.get8());
                let b3 = i32::from(self.get8());
                Serializer::decode_vl_length_3(b1, b2, b3)
            }
            _ => unreachable!("decode_length_length only returns 1..=3"),
        }
    }

    pub fn get_slice(&mut self, bytes: usize) -> Slice<'a> {
        if bytes > self.data.len().saturating_sub(self.used) {
            return Slice::new(&[]);
        }
        let start = self.used;
        self.used += bytes;
        Slice::new(&self.data[start..start + bytes])
    }

    pub fn get_raw(&mut self, size: usize) -> Blob {
        self.get_raw_helper_blob(size)
    }

    pub fn get_vl(&mut self) -> Blob {
        let length = self.get_vl_data_length() as usize;
        self.get_raw(length)
    }

    pub fn get_vl_buffer(&mut self) -> Buffer {
        let length = self.get_vl_data_length() as usize;
        self.get_raw_helper_buffer(length)
    }

    fn read_integer<T: SerializerInteger>(&mut self, _message: &'static str) -> T {
        let bytes = T::BYTES;
        if self.data.len().saturating_sub(self.used) < bytes {
            tracing::trace!(target: "protocol", "SerialIter underflow — returning default");
            return T::read_be(&vec![0u8; bytes]);
        }
        let value = T::read_be(&self.data[self.used..self.used + bytes]);
        self.used += bytes;
        value
    }

    fn get_raw_helper_blob(&mut self, size: usize) -> Blob {
        if self.data.len() - self.used < size {
            panic!("invalid SerialIter getRaw");
        }
        let result = self.data[self.used..self.used + size].to_vec();
        self.used += size;
        result
    }

    fn get_raw_helper_buffer(&mut self, size: usize) -> Buffer {
        if self.data.len().saturating_sub(self.used) < size {
            self.used = self.data.len();
            return Buffer::from_bytes(&[]);
        }
        let result = Buffer::from_bytes(&self.data[self.used..self.used + size]);
        self.used += size;
        result
    }
}

impl<'a> From<&'a [u8]> for SerialIter<'a> {
    fn from(value: &'a [u8]) -> Self {
        Self::new(value)
    }
}

impl<'a, const N: usize> From<&'a [u8; N]> for SerialIter<'a> {
    fn from(value: &'a [u8; N]) -> Self {
        Self::new(value)
    }
}
