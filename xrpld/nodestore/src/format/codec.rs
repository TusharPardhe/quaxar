use crate::{NodeObject, NodeObjectType};
use basics::base_uint::Uint256;
use basics::blob::Blob;
use lz4_flex::block::{
    CompressError, DecompressError, compress_into, decompress_into, get_maximum_output_size,
};
use protocol::HashPrefix;
use std::sync::Arc;

const INNER_NODE_SIZE: usize = 525;
const INNER_NODE_HASH_BYTES: usize = 16 * 32;
const INNER_NODE_MASK_BYTES: usize = 2;
const INNER_NODE_HEADER_BYTES: usize = 13;
const ENCODED_BLOB_PREFIX_BYTES: usize = 9;
const ENCODED_BLOB_INLINE_BYTES: usize = align_up(
    ENCODED_BLOB_PREFIX_BYTES + 1024,
    std::mem::align_of::<u32>(),
);

const fn align_up(value: usize, alignment: usize) -> usize {
    let remainder = value % alignment;
    if remainder == 0 {
        value
    } else {
        value + (alignment - remainder)
    }
}

fn read_u16_be(bytes: &[u8]) -> u16 {
    u16::from_be_bytes(bytes[..2].try_into().expect("u16 field must fit"))
}

fn write_u16_be(out: &mut [u8], value: u16) {
    out[..2].copy_from_slice(&value.to_be_bytes());
}

fn read_u32_be(bytes: &[u8]) -> u32 {
    u32::from_be_bytes(bytes[..4].try_into().expect("u32 field must fit"))
}

fn write_u32_be(out: &mut [u8], value: u32) {
    out[..4].copy_from_slice(&value.to_be_bytes());
}

fn lz4_error_to_string(error: DecompressError) -> String {
    format!("lz4_decompress: {error}")
}

fn lz4_compress_error_to_string(error: CompressError) -> String {
    format!("lz4 compress: {error}")
}

fn lz4_decompress(input: &[u8]) -> Result<Vec<u8>, String> {
    if input.len() > i32::MAX as usize {
        return Err("lz4_decompress: integer overflow (input)".to_owned());
    }

    let mut out_size = 0usize;
    let n = read_varint(input, &mut out_size);
    if n == 0 || n >= input.len() {
        return Err("lz4_decompress: invalid blob".to_owned());
    }
    if out_size == 0 || out_size > i32::MAX as usize {
        return Err("lz4_decompress: integer overflow (output)".to_owned());
    }

    let mut output = vec![0u8; out_size];
    let written = decompress_into(&input[n..], &mut output).map_err(lz4_error_to_string)?;
    if written != out_size {
        return Err("lz4_decompress: LZ4_decompress_safe".to_owned());
    }
    Ok(output)
}

fn lz4_compress(input: &[u8]) -> Result<Vec<u8>, String> {
    let mut out = vec![0u8; size_varint(input.len()) + get_maximum_output_size(input.len())];
    let size_prefix = write_varint(&mut out, input.len());
    let compressed =
        compress_into(input, &mut out[size_prefix..]).map_err(lz4_compress_error_to_string)?;
    out.truncate(size_prefix + compressed);
    Ok(out)
}

pub fn read_varint(buf: &[u8], value: &mut usize) -> usize {
    if buf.is_empty() {
        return 0;
    }

    *value = 0;
    let mut n = 0usize;
    while buf[n] & 0x80 != 0 {
        n += 1;
        if n >= buf.len() {
            return 0;
        }
    }

    n += 1;
    if n > buf.len() {
        return 0;
    }

    if n == 1 && buf[0] == 0 {
        *value = 0;
        return 1;
    }

    let used = n;
    while n > 0 {
        n -= 1;
        let digit = (buf[n] & 0x7f) as usize;
        let previous = *value;
        let next = value.wrapping_mul(127).wrapping_add(digit);
        if next <= previous {
            return 0;
        }
        *value = next;
    }

    used
}

pub fn size_varint(mut value: usize) -> usize {
    let mut size = 0usize;
    loop {
        value /= 127;
        size += 1;
        if value == 0 {
            return size;
        }
    }
}

pub fn write_varint(buf: &mut [u8], mut value: usize) -> usize {
    let required = size_varint(value);
    assert!(
        buf.len() >= required,
        "write_varint buffer must hold the encoded value"
    );

    let mut written = 0usize;
    loop {
        let mut digit = (value % 127) as u8;
        value /= 127;
        if value != 0 {
            digit |= 0x80;
        }
        buf[written] = digit;
        written += 1;
        if value == 0 {
            return written;
        }
    }
}

#[allow(clippy::large_enum_variant)]
enum EncodedBlobStorage {
    Inline([u8; ENCODED_BLOB_INLINE_BYTES]),
    Heap(Box<[u8]>),
}

impl EncodedBlobStorage {
    fn as_slice(&self, size: usize) -> &[u8] {
        match self {
            Self::Inline(bytes) => &bytes[..size],
            Self::Heap(bytes) => &bytes[..size],
        }
    }

    fn as_mut_slice(&mut self, size: usize) -> &mut [u8] {
        match self {
            Self::Inline(bytes) => &mut bytes[..size],
            Self::Heap(bytes) => &mut bytes[..size],
        }
    }
}

pub struct EncodedBlob {
    key: [u8; NodeObject::KEY_BYTES],
    size: u32,
    storage: EncodedBlobStorage,
}

impl EncodedBlob {
    pub fn new(object: &NodeObject) -> Self {
        let size = object
            .data()
            .len()
            .checked_add(ENCODED_BLOB_PREFIX_BYTES)
            .expect("EncodedBlob size must fit usize");
        let size_u32 = u32::try_from(size).expect("EncodedBlob size must fit u32");

        let storage = if size <= ENCODED_BLOB_INLINE_BYTES {
            EncodedBlobStorage::Inline([0u8; ENCODED_BLOB_INLINE_BYTES])
        } else {
            EncodedBlobStorage::Heap(vec![0u8; size].into_boxed_slice())
        };

        let mut blob = Self {
            key: *object.hash().data(),
            size: size_u32,
            storage,
        };

        let payload = blob.storage.as_mut_slice(size);
        payload[..8].fill(0);
        payload[8] = object.object_type() as u8;
        payload[9..].copy_from_slice(object.data());
        blob
    }

    pub fn get_key(&self) -> &[u8; NodeObject::KEY_BYTES] {
        &self.key
    }

    pub fn get_size(&self) -> usize {
        self.size as usize
    }

    pub fn get_data(&self) -> &[u8] {
        self.storage.as_slice(self.size as usize)
    }
}

pub struct DecodedBlob {
    success: bool,
    key: [u8; NodeObject::KEY_BYTES],
    object_type: NodeObjectType,
    object_data: Blob,
}

impl DecodedBlob {
    pub fn new(key: &[u8], value: &[u8]) -> Self {
        let key: [u8; NodeObject::KEY_BYTES] = key[..NodeObject::KEY_BYTES]
            .try_into()
            .expect("DecodedBlob keys must be 32 bytes");

        let mut object_type = NodeObjectType::Unknown;
        let mut success = false;
        let mut object_data = Vec::new();

        if value.len() > 8
            && let Ok(parsed) = NodeObjectType::try_from(value[8])
        {
            object_type = parsed;
        }

        if value.len() > 9 {
            object_data = value[9..].to_vec();
            success = matches!(
                value[8],
                x if x == NodeObjectType::Unknown as u8
                    || x == NodeObjectType::Ledger as u8
                    || x == NodeObjectType::AccountNode as u8
                    || x == NodeObjectType::TransactionNode as u8
            );
        }

        Self {
            success,
            key,
            object_type,
            object_data,
        }
    }

    pub fn was_ok(&self) -> bool {
        self.success
    }

    pub fn create_object(&self) -> Arc<NodeObject> {
        assert!(
            self.success,
            "xrpl::NodeStore::DecodedBlob::createObject : valid object type"
        );

        NodeObject::create_object(
            self.object_type,
            self.object_data.clone(),
            Uint256::from_array(self.key),
        )
    }
}

pub fn nodeobject_decompress(input: &[u8]) -> Result<Vec<u8>, String> {
    let mut codec_type = 0usize;
    let varint_bytes = read_varint(input, &mut codec_type);
    if varint_bytes == 0 {
        return Err("nodeobject decompress".to_owned());
    }

    let remaining = &input[varint_bytes..];
    match codec_type {
        0 => Ok(remaining.to_vec()),
        1 => lz4_decompress(remaining),
        2 => {
            if remaining.len() < INNER_NODE_MASK_BYTES + 32 {
                return Err(format!(
                    "nodeobject codec v1: short inner node size: in_size = {} hs = {}",
                    remaining.len(),
                    INNER_NODE_MASK_BYTES
                ));
            }

            let mut in_size = remaining.len() - INNER_NODE_MASK_BYTES;
            let mask = read_u16_be(remaining);
            if mask == 0 {
                return Err("nodeobject codec v1: empty inner node".to_owned());
            }

            let mut out = vec![0u8; INNER_NODE_SIZE];
            write_u32_be(&mut out[0..4], 0);
            write_u32_be(&mut out[4..8], 0);
            out[8] = NodeObjectType::Unknown as u8;
            write_u32_be(&mut out[9..13], HashPrefix::InnerNode.as_u32());

            let mut source_offset = INNER_NODE_MASK_BYTES;
            let mut bit = 0x8000u16;
            for slot in 0..16 {
                let target = INNER_NODE_HEADER_BYTES + slot * 32;
                if mask & bit != 0 {
                    if in_size < 32 {
                        return Err(format!(
                            "nodeobject codec v1: short inner node subsize: in_size = {} i = {}",
                            in_size,
                            15 - slot
                        ));
                    }
                    out[target..target + 32]
                        .copy_from_slice(&remaining[source_offset..source_offset + 32]);
                    source_offset += 32;
                    in_size -= 32;
                } else {
                    out[target..target + 32].fill(0);
                }
                bit >>= 1;
            }

            if in_size > 0 {
                return Err(format!(
                    "nodeobject codec v1: long inner node, in_size = {in_size}"
                ));
            }

            Ok(out)
        }
        3 => {
            if remaining.len() != INNER_NODE_HASH_BYTES {
                return Err(format!(
                    "nodeobject codec v1: short full inner node, in_size = {}",
                    remaining.len()
                ));
            }

            let mut out = vec![0u8; INNER_NODE_SIZE];
            write_u32_be(&mut out[0..4], 0);
            write_u32_be(&mut out[4..8], 0);
            out[8] = NodeObjectType::Unknown as u8;
            write_u32_be(&mut out[9..13], HashPrefix::InnerNode.as_u32());
            out[INNER_NODE_HEADER_BYTES..].copy_from_slice(remaining);
            Ok(out)
        }
        _ => Err(format!("nodeobject codec: bad type={codec_type}")),
    }
}

pub fn nodeobject_compress(input: &[u8]) -> Result<Vec<u8>, String> {
    if input.len() == INNER_NODE_SIZE {
        let prefix = read_u32_be(&input[9..13]);
        if prefix == HashPrefix::InnerNode.as_u32() {
            let hashes = &input[INNER_NODE_HEADER_BYTES..];
            let mut packed = [0u8; INNER_NODE_HASH_BYTES];
            let mut count = 0usize;
            let mut mask = 0u16;

            for (slot, hash) in hashes.chunks_exact(32).enumerate() {
                let bit = 0x8000u16 >> slot;
                if hash.iter().all(|byte| *byte == 0) {
                    continue;
                }
                packed[count * 32..(count + 1) * 32].copy_from_slice(hash);
                mask |= bit;
                count += 1;
            }

            if count < 16 {
                let codec_type = 2usize;
                let mut out =
                    vec![0u8; size_varint(codec_type) + INNER_NODE_MASK_BYTES + count * 32];
                let type_bytes = write_varint(&mut out, codec_type);
                write_u16_be(&mut out[type_bytes..type_bytes + 2], mask);
                out[type_bytes + 2..].copy_from_slice(&packed[..count * 32]);
                return Ok(out);
            }

            let codec_type = 3usize;
            let mut out = vec![0u8; size_varint(codec_type) + count * 32];
            let type_bytes = write_varint(&mut out, codec_type);
            out[type_bytes..].copy_from_slice(&packed[..count * 32]);
            return Ok(out);
        }
    }

    let codec_type = 1usize;
    let compressed = lz4_compress(input)?;
    let mut out = vec![0u8; size_varint(codec_type) + compressed.len()];
    let type_bytes = write_varint(&mut out, codec_type);
    out[type_bytes..].copy_from_slice(&compressed);
    Ok(out)
}

pub fn filter_inner(input: &mut [u8]) {
    if input.len() != INNER_NODE_SIZE {
        return;
    }

    if read_u32_be(&input[9..13]) == HashPrefix::InnerNode.as_u32() {
        write_u32_be(&mut input[0..4], 0);
        write_u32_be(&mut input[4..8], 0);
        input[8] = NodeObjectType::Unknown as u8;
    }
}

#[cfg(test)]
mod tests {
    use super::{
        DecodedBlob, EncodedBlob, filter_inner, nodeobject_compress, nodeobject_decompress,
        read_u32_be, read_varint, size_varint, write_u32_be, write_varint,
    };
    use crate::{NodeObject, NodeObjectType};
    use basics::base_uint::Uint256;
    use protocol::HashPrefix;

    fn sample_hash(fill: u8) -> Uint256 {
        Uint256::from_array([fill; 32])
    }

    fn make_inner_node(occupied: &[usize]) -> Vec<u8> {
        let mut bytes = vec![0u8; 525];
        write_u32_be(&mut bytes[0..4], 0x0102_0304);
        write_u32_be(&mut bytes[4..8], 0x0506_0708);
        bytes[8] = NodeObjectType::TransactionNode as u8;
        write_u32_be(&mut bytes[9..13], HashPrefix::InnerNode.as_u32());

        for &slot in occupied {
            let start = 13 + slot * 32;
            for (offset, byte) in bytes[start..start + 32].iter_mut().enumerate() {
                *byte = (slot as u8).wrapping_add(offset as u8).wrapping_add(1);
            }
        }

        bytes
    }

    #[test]
    fn base_127_varint_round_trips() {
        for value in [0usize, 1, 126, 127, 128, 16_128, 2_048_191] {
            let mut encoded = [0u8; 16];
            let written = write_varint(&mut encoded, value);
            assert_eq!(written, size_varint(value));

            let mut decoded = usize::MAX;
            let used = read_varint(&encoded[..written], &mut decoded);
            assert_eq!(used, written);
            assert_eq!(decoded, value);
        }
    }

    #[test]
    fn base_127_varint_rejects_noncanonical_zero_and_overflow() {
        let mut value = usize::MAX;
        assert_eq!(read_varint(&[0x80, 0x00], &mut value), 0);
        assert_eq!(
            read_varint(
                &[0x81, 0x81, 0x81, 0x81, 0x81, 0x81, 0x81, 0x81, 0x81, 0x00],
                &mut value
            ),
            0
        );
    }

    #[test]
    fn encoded_and_decoded_blob_round_trip() {
        let object = NodeObject::new(
            NodeObjectType::Ledger,
            vec![1, 2, 3, 4, 5, 6],
            sample_hash(0x44),
        );
        let encoded = EncodedBlob::new(&object);
        assert_eq!(encoded.get_key(), object.hash().data());
        assert_eq!(encoded.get_size(), 15);
        assert_eq!(encoded.get_data()[8], NodeObjectType::Ledger as u8);

        let decoded = DecodedBlob::new(encoded.get_key(), encoded.get_data());
        assert!(decoded.was_ok());
        let recreated = decoded.create_object();
        assert_eq!(recreated.object_type(), NodeObjectType::Ledger);
        assert_eq!(recreated.hash(), object.hash());
        assert_eq!(recreated.data(), object.data());
    }

    #[test]
    fn decoded_blob_rejects_unknown_object_types() {
        let key = [0x11; 32];
        let mut value = vec![0u8; 10];
        value[8] = 2;
        value[9] = 0xaa;

        let decoded = DecodedBlob::new(&key, &value);
        assert!(!decoded.was_ok());
    }

    #[test]
    fn inner_node_codec_uses_sparse_v1_layout() {
        let mut original = make_inner_node(&[0, 3, 15]);
        let compressed =
            nodeobject_compress(&original).expect("inner node compression should work");
        assert_eq!(compressed[0], 2);

        filter_inner(&mut original);
        let decompressed =
            nodeobject_decompress(&compressed).expect("inner node decompression should work");
        assert_eq!(decompressed, original);
    }

    #[test]
    fn inner_node_codec_uses_full_v1_layout_for_16_children() {
        let occupied: Vec<usize> = (0..16).collect();
        let mut original = make_inner_node(&occupied);
        let compressed = nodeobject_compress(&original).expect("full inner node compression");
        assert_eq!(compressed[0], 3);
        assert_eq!(compressed.len(), 1 + 16 * 32);

        filter_inner(&mut original);
        let decompressed = nodeobject_decompress(&compressed).expect("full inner node decompress");
        assert_eq!(decompressed, original);
    }

    #[test]
    fn filter_inner_only_zeroes_the_mutable_prefix_fields() {
        let mut bytes = make_inner_node(&[1]);
        let original_hash_block = bytes[13..].to_vec();

        filter_inner(&mut bytes);

        assert_eq!(read_u32_be(&bytes[0..4]), 0);
        assert_eq!(read_u32_be(&bytes[4..8]), 0);
        assert_eq!(bytes[8], NodeObjectType::Unknown as u8);
        assert_eq!(read_u32_be(&bytes[9..13]), HashPrefix::InnerNode.as_u32());
        assert_eq!(&bytes[13..], original_hash_block.as_slice());
    }

    #[test]
    fn lz4_codec_round_trips_non_inner_payloads() {
        let payload: Vec<u8> = (0..2048).map(|i| (i % 251) as u8).collect();
        let compressed = nodeobject_compress(&payload).expect("lz4 compression should work");
        assert_eq!(compressed[0], 1);
        let decompressed =
            nodeobject_decompress(&compressed).expect("lz4 decompression should work");
        assert_eq!(decompressed, payload);
    }
}
