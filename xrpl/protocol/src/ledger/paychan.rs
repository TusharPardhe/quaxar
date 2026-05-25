//! `xrpl/protocol/PayChan.h` compatibility surface.
//!
//! This keeps the current payment-channel claim hash prefix and the exact
//! authorization message byte layout caller-visible.

use basics::base_uint::Uint256;

pub const PAYMENT_CHANNEL_CLAIM_HASH_PREFIX: u32 = 0x434c_4d00;

pub fn serialize_pay_chan_authorization(channel_id: &Uint256, amount_drops: u64) -> Vec<u8> {
    let mut message = Vec::with_capacity(4 + Uint256::BYTES + 8);
    message.extend_from_slice(&PAYMENT_CHANNEL_CLAIM_HASH_PREFIX.to_be_bytes());
    message.extend_from_slice(channel_id.data());
    message.extend_from_slice(&amount_drops.to_be_bytes());
    message
}

#[cfg(test)]
mod tests {
    use super::{PAYMENT_CHANNEL_CLAIM_HASH_PREFIX, serialize_pay_chan_authorization};
    use basics::base_uint::Uint256;

    #[test]
    fn payment_channel_claim_hash_prefix() {
        assert_eq!(PAYMENT_CHANNEL_CLAIM_HASH_PREFIX, 0x434c_4d00);
        assert_eq!(
            PAYMENT_CHANNEL_CLAIM_HASH_PREFIX.to_be_bytes(),
            [0x43, 0x4c, 0x4d, 0x00]
        );
    }

    #[test]
    fn serialize_pay_chan_authorization_byte_layout() {
        let channel_id =
            Uint256::from_hex("0123456789ABCDEFFEDCBA98765432100123456789ABCDEFFEDCBA9876543210")
                .expect("channel id should parse");

        let bytes = serialize_pay_chan_authorization(&channel_id, 0x0102_0304_0506_0708);

        let mut expected = Vec::with_capacity(44);
        expected.extend_from_slice(&0x434c_4d00_u32.to_be_bytes());
        expected.extend_from_slice(channel_id.data());
        expected.extend_from_slice(&0x0102_0304_0506_0708_u64.to_be_bytes());

        assert_eq!(bytes, expected);
        assert_eq!(bytes.len(), 44);
    }

    #[test]
    fn serialize_pay_chan_authorization_keeps_channel_bytes_unchanged() {
        let channel_id = Uint256::from_array([0xAB; Uint256::BYTES]);

        let bytes = serialize_pay_chan_authorization(&channel_id, 42);

        assert_eq!(&bytes[4..36], channel_id.data());
        assert_eq!(&bytes[36..], 42_u64.to_be_bytes());
    }
}
