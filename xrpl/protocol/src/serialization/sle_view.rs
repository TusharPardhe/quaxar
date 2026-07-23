//! Zero-copy SLE (Serialized Ledger Entry) view.
//!
//! `SleView` borrows raw serialized bytes and provides field access without
//! deserialization or allocation. Fields are located by scanning type/field
//! headers per the XRPL binary serialization format.

/// Field type IDs matching `SerializedTypeId`.
const STI_UINT32: i32 = 2;
const STI_AMOUNT: i32 = 6;

/// Field indices from the protocol sfield specs.
const SF_SEQUENCE_FIELD: i32 = 4; // UInt32, field_value=4
const SF_OWNER_COUNT_FIELD: i32 = 13; // UInt32, field_value=13
const SF_BALANCE_FIELD: i32 = 2; // Amount, field_value=2

/// Raw 64-bit Amount value decoded from wire format.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Amount(pub u64);

/// Zero-copy view over a serialized SLE blob.
#[derive(Debug)]
pub struct SleView<'a> {
    data: &'a [u8],
}

impl<'a> SleView<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Self { data }
    }

    /// Decode the Balance field (Amount type, field_value=2).
    pub fn balance(&self) -> Option<Amount> {
        let offset = self.find_field(STI_AMOUNT, SF_BALANCE_FIELD)?;
        if offset + 8 > self.data.len() {
            return None;
        }
        let val = u64::from_be_bytes(self.data[offset..offset + 8].try_into().ok()?);
        Some(Amount(val))
    }

    /// Decode the Sequence field (UInt32, field_value=4).
    pub fn sequence(&self) -> Option<u32> {
        let offset = self.find_field(STI_UINT32, SF_SEQUENCE_FIELD)?;
        if offset + 4 > self.data.len() {
            return None;
        }
        Some(u32::from_be_bytes(
            self.data[offset..offset + 4].try_into().ok()?,
        ))
    }

    /// Decode the OwnerCount field (UInt32, field_value=13).
    pub fn owner_count(&self) -> Option<u32> {
        let offset = self.find_field(STI_UINT32, SF_OWNER_COUNT_FIELD)?;
        if offset + 4 > self.data.len() {
            return None;
        }
        Some(u32::from_be_bytes(
            self.data[offset..offset + 4].try_into().ok()?,
        ))
    }

    /// Scan from the beginning of the serialized data to find the field with
    /// the given type_id and field_id. Returns the byte offset of the field
    /// value (past the header).
    fn find_field(&self, target_type: i32, target_field: i32) -> Option<usize> {
        let mut pos = 0;
        let data = self.data;

        while pos < data.len() {
            let (type_id, field_id, header_len) = Self::decode_field_header(data, pos)?;
            pos += header_len;

            if type_id == target_type && field_id == target_field {
                return Some(pos);
            }

            // Skip the field value based on type.
            pos += Self::field_data_length(data, pos, type_id)?;
        }
        None
    }

    /// Decode a field header at position `pos`. Returns (type_id, field_id, header_byte_count).
    fn decode_field_header(data: &[u8], pos: usize) -> Option<(i32, i32, usize)> {
        if pos >= data.len() {
            return None;
        }
        let byte = data[pos] as i32;
        let mut type_id = byte >> 4;
        let mut field_id = byte & 0x0F;
        let mut consumed = 1;

        if type_id == 0 {
            if pos + consumed >= data.len() {
                return None;
            }
            type_id = data[pos + consumed] as i32;
            consumed += 1;
        }

        if field_id == 0 {
            if pos + consumed >= data.len() {
                return None;
            }
            field_id = data[pos + consumed] as i32;
            consumed += 1;
        }

        Some((type_id, field_id, consumed))
    }

    /// Return the byte length of a field value given the type_id.
    fn field_data_length(data: &[u8], pos: usize, type_id: i32) -> Option<usize> {
        match type_id {
            16 => Some(1), // UInt8
            1 => Some(2),  // UInt16
            2 => Some(4),  // UInt32
            3 => Some(8),  // UInt64
            4 => Some(16), // UInt128
            5 => Some(32), // UInt256
            6 => {
                // Amount type: discriminate by top bit of first byte.
                // - Native XRP: bit 62 clear (top byte & 0x80 == 0x40 pattern) → 8 bytes
                // - IOU: bit 62 set → 48 bytes (8 amount + 20 currency + 20 issuer)
                // - MPT: first byte == 0x03 → 41 bytes (1 + 8 + 32)
                if pos >= data.len() {
                    return None;
                }
                let first = data[pos];
                if first == 0x03 {
                    // MPT amount
                    Some(41)
                } else if first & 0x80 != 0 {
                    // IOU amount (bit 63 set = non-native)
                    Some(48)
                } else {
                    // Native XRP amount
                    Some(8)
                }
            }
            17 => Some(20), // UInt160
            20 => Some(12), // UInt96
            21 => Some(24), // UInt192
            22 => Some(48), // UInt384
            23 => Some(64), // UInt512
            // Variable-length types (VL, Account, etc.) — decode VL prefix.
            7 | 8 | 18 | 19 | 24 | 25 | 26 => Self::decode_vl_length(data, pos),
            // Object (14) — scan until end-of-object marker (0xE1).
            14 => Self::scan_object_length(data, pos),
            // Array (15) — scan until end-of-array marker (0xF1).
            15 => Self::scan_array_length(data, pos),
            _ => None,
        }
    }

    /// Decode a variable-length prefix and return total consumed bytes (prefix + payload).
    fn decode_vl_length(data: &[u8], pos: usize) -> Option<usize> {
        if pos >= data.len() {
            return None;
        }
        let b1 = data[pos] as usize;
        if b1 <= 192 {
            Some(1 + b1)
        } else if b1 <= 240 {
            if pos + 1 >= data.len() {
                return None;
            }
            let b2 = data[pos + 1] as usize;
            let len = 193 + ((b1 - 193) * 256) + b2;
            Some(2 + len)
        } else if b1 <= 254 {
            if pos + 2 >= data.len() {
                return None;
            }
            let b2 = data[pos + 1] as usize;
            let b3 = data[pos + 2] as usize;
            let len = 12481 + ((b1 - 241) * 65536) + (b2 * 256) + b3;
            Some(3 + len)
        } else {
            None
        }
    }

    /// Scan past an STObject (type 14) until end-of-object marker byte 0xE1.
    fn scan_object_length(data: &[u8], start: usize) -> Option<usize> {
        let mut pos = start;
        while pos < data.len() {
            if data[pos] == 0xE1 {
                return Some(pos - start + 1);
            }
            let (type_id, _field_id, header_len) = Self::decode_field_header(data, pos)?;
            pos += header_len;
            pos += Self::field_data_length(data, pos, type_id)?;
        }
        None
    }

    /// Scan past an STArray (type 15) until end-of-array marker byte 0xF1.
    fn scan_array_length(data: &[u8], start: usize) -> Option<usize> {
        let mut pos = start;
        while pos < data.len() {
            if data[pos] == 0xF1 {
                return Some(pos - start + 1);
            }
            let (type_id, _field_id, header_len) = Self::decode_field_header(data, pos)?;
            pos += header_len;
            pos += Self::field_data_length(data, pos, type_id)?;
        }
        None
    }
}
