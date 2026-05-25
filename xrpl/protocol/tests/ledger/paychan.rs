//! Integration tests that pin the current `PayChan.h` authorization message
//! shape to the C++ behavior.

use basics::base_uint::Uint256;
use protocol::{PAYMENT_CHANNEL_CLAIM_HASH_PREFIX, serialize_pay_chan_authorization};

#[test]
fn paychan_authorization_serialization_prefix_and_layout() {
    let channel_id =
        Uint256::from_hex("0123456789ABCDEFFEDCBA98765432100123456789ABCDEFFEDCBA9876543210")
            .expect("expected channel id should parse");
    let amount_drops = 0x0102_0304_0506_0708_u64;

    let serialized = serialize_pay_chan_authorization(&channel_id, amount_drops);

    assert_eq!(serialized.len(), 44);
    assert_eq!(
        &serialized[..4],
        &PAYMENT_CHANNEL_CLAIM_HASH_PREFIX.to_be_bytes()
    );
    assert_eq!(&serialized[4..36], channel_id.data());
    assert_eq!(&serialized[36..], &amount_drops.to_be_bytes());
}

#[test]
fn paychan_authorization_prefix_matches_current_cpp_constant() {
    assert_eq!(PAYMENT_CHANNEL_CLAIM_HASH_PREFIX, 0x434c_4d00);
    assert_eq!(
        PAYMENT_CHANNEL_CLAIM_HASH_PREFIX.to_be_bytes(),
        [0x43, 0x4c, 0x4d, 0x00]
    );
}
