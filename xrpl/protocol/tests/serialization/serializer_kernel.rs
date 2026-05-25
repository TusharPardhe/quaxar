use basics::{
    base_uint::{Uint128, Uint256},
    buffer::Buffer,
    slice::Slice,
};
use protocol::{HashPrefix, SerialIter, Serializer};
use std::panic::{AssertUnwindSafe, catch_unwind};

fn panic_message(payload: Box<dyn std::any::Any + Send>) -> String {
    if let Some(message) = payload.downcast_ref::<String>() {
        return message.clone();
    }
    if let Some(message) = payload.downcast_ref::<&'static str>() {
        return (*message).to_string();
    }
    "non-string panic payload".to_string()
}

#[test]
fn serializer_integer_and_hash_prefix_writes_match_cpp_wire_order() {
    let mut serializer = Serializer::default();
    serializer.add8(0xAB);
    serializer.add16(0xCDEF);
    serializer.add32(0x0123_4567);
    serializer.add32_prefix(HashPrefix::LedgerMaster);
    serializer.add64(0x89AB_CDEF_0123_4567);
    serializer.add_integer(-2i32);

    assert_eq!(
        serializer,
        vec![
            0xAB, 0xCD, 0xEF, 0x01, 0x23, 0x45, 0x67, b'L', b'W', b'R', 0x00, 0x89, 0xAB, 0xCD,
            0xEF, 0x01, 0x23, 0x45, 0x67, 0xFF, 0xFF, 0xFF, 0xFE,
        ]
    );
}

#[test]
fn serializer_field_id_encodings_match_cpp_layout_rules() {
    let mut serializer = Serializer::default();
    serializer.add_field_id(2, 4);
    serializer.add_field_id(2, 17);
    serializer.add_field_id(17, 4);
    serializer.add_field_id(17, 18);

    assert_eq!(
        serializer,
        vec![0x24, 0x20, 0x11, 0x04, 0x11, 0x00, 0x11, 0x12]
    );
}

#[test]
fn serializer_vl_prefix_boundaries_match_cpp() {
    let mut one = Serializer::default();
    one.add_vl(vec![0u8; 192]);
    assert_eq!(one.data()[0], 192);

    let mut two = Serializer::default();
    two.add_vl(vec![0u8; 193]);
    assert_eq!(&two.data()[..2], &[193, 0]);

    let mut two_max = Serializer::default();
    two_max.add_vl(vec![0u8; 12_480]);
    assert_eq!(&two_max.data()[..2], &[240, 255]);

    let mut three = Serializer::default();
    three.add_vl(vec![0u8; 12_481]);
    assert_eq!(&three.data()[..3], &[241, 0, 0]);

    let mut three_max = Serializer::default();
    three_max.add_vl(vec![0u8; 918_744]);
    assert_eq!(&three_max.data()[..3], &[254, 212, 23]);

    let overflow = catch_unwind(AssertUnwindSafe(|| {
        let mut serializer = Serializer::default();
        serializer.add_vl(vec![0u8; 918_745]);
    }))
    .expect_err("oversized VL should panic");
    assert_eq!(panic_message(overflow), "lenlen");
}

#[test]
fn serializer_decode_helpers_match_cpp_boundaries() {
    assert_eq!(Serializer::decode_length_length(0), 1);
    assert_eq!(Serializer::decode_length_length(192), 1);
    assert_eq!(Serializer::decode_length_length(193), 2);
    assert_eq!(Serializer::decode_length_length(240), 2);
    assert_eq!(Serializer::decode_length_length(241), 3);
    assert_eq!(Serializer::decode_length_length(254), 3);

    assert_eq!(Serializer::decode_vl_length_1(0), 0);
    assert_eq!(Serializer::decode_vl_length_1(192), 192);
    assert_eq!(Serializer::decode_vl_length_2(193, 0), 193);
    assert_eq!(Serializer::decode_vl_length_2(240, 255), 12_480);
    assert_eq!(Serializer::decode_vl_length_3(241, 0, 0), 12_481);
    assert_eq!(Serializer::decode_vl_length_3(254, 212, 23), 918_744);

    let invalid = catch_unwind(AssertUnwindSafe(|| Serializer::decode_length_length(255)))
        .expect_err("255 should be rejected");
    assert_eq!(panic_message(invalid), "b1>254");
}

#[test]
fn serializer_read_helpers_match_cpp_behavior() {
    let bytes = [
        0xAA, 0x12, 0x34, 0x56, 0x78, 0xFE, 0xDC, 0xBA, 0x98, 0x11, 0x22, 0x33, 0x44,
    ];
    let serializer = Serializer::from_bytes(bytes);

    let mut b = 0;
    assert!(serializer.get8(&mut b, 0));
    assert_eq!(b, 0xAA);
    assert!(!serializer.get8(&mut b, bytes.len()));

    let mut u32_value = 0u32;
    assert!(serializer.get_integer(&mut u32_value, 1));
    assert_eq!(u32_value, 0x1234_5678);

    let mut i32_value = 0i32;
    assert!(serializer.get_integer(&mut i32_value, 5));
    assert_eq!(i32_value, -19088744);

    let mut uint128 = Uint128::default();
    let bit_serializer =
        Serializer::from_bytes([1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16]);
    assert!(bit_serializer.get_bit_string(&mut uint128, 0));
    assert_eq!(
        uint128,
        Uint128::from_slice(&[1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16])
            .expect("uint128")
    );
}

#[test]
fn serial_iter_cursor_and_integer_reads_match_cpp() {
    let bytes = [
        0xAB, 0xCD, 0xEF, 0xFE, 0xDC, 0xBA, 0x98, 0x76, 0x54, 0x32, 0x10,
    ];
    let mut iter = SerialIter::new(&bytes);

    assert_eq!(iter.get8(), 0xAB);
    assert_eq!(iter.get16(), 0xCDEF);
    assert_eq!(iter.geti32(), -19088744);
    assert_eq!(iter.get_bytes_left(), 4);
    assert_eq!(iter.get32(), 0x7654_3210);
    assert!(iter.empty());

    iter.reset();
    assert_eq!(iter.get_bytes_left(), bytes.len() as i32);
    iter.skip(3);
    assert_eq!(iter.get32(), 0xFEDC_BA98);
}

#[test]
fn serial_iter_field_id_and_vl_reads_match_cpp() {
    let bytes = [0x24, 0x20, 0x11, 0x03, b'a', b'b', b'c'];
    let mut iter = SerialIter::new(&bytes);

    let mut type_id = 0;
    let mut field_id = 0;
    iter.get_field_id(&mut type_id, &mut field_id);
    assert_eq!((type_id, field_id), (2, 4));

    iter.get_field_id(&mut type_id, &mut field_id);
    assert_eq!((type_id, field_id), (2, 17));

    assert_eq!(iter.get_vl_data_length(), 3);
    assert_eq!(iter.get_slice(3), Slice::new(b"abc"));
    assert!(iter.empty());
}

#[test]
fn serial_iter_rejects_non_canonical_expanded_field_ids() {
    // Invalid expanded type (< 16): returns -1 instead of panicking
    let mut iter = SerialIter::new(&[0x04, 0x0F]);
    let mut type_id = 0;
    let mut field_id = 0;
    iter.get_field_id(&mut type_id, &mut field_id);
    assert_eq!(type_id, -1, "invalid expanded type should return -1");

    // Invalid expanded name (< 16): returns -1 instead of panicking
    let mut iter = SerialIter::new(&[0x20, 0x0F]);
    let mut type_id = 0;
    let mut field_id = 0;
    iter.get_field_id(&mut type_id, &mut field_id);
    assert_eq!(field_id, -1, "invalid expanded name should return -1");
}

#[test]
fn serial_iter_bitstrings_and_vl_buffer_preserve_bytes() {
    let uint256_bytes: Vec<u8> = (1u8..=32).collect();
    let mut iter = SerialIter::new(&uint256_bytes);
    let uint256 = iter.get256();
    assert_eq!(
        uint256,
        Uint256::from_slice(&uint256_bytes).expect("uint256")
    );

    let mut vl_iter = SerialIter::new(&[3, 0xAA, 0xBB, 0xCC]);
    let buffer: Buffer = vl_iter.get_vl_buffer();
    assert_eq!(buffer.data(), &[0xAA, 0xBB, 0xCC]);
}

#[test]
fn serializer_sha512_half_and_chop_match_cpp_behavior() {
    let mut serializer = Serializer::from_bytes(b"abc");
    let hash = serializer.get_sha512_half();
    assert_eq!(
        hash,
        Uint256::from_hex("DDAF35A193617ABACC417349AE20413112E6FA4E89A97EA20A9EEEE64B55D39A")
            .expect("sha512 half")
    );

    assert!(serializer.chop(1));
    assert_eq!(serializer, b"ab".to_vec());
    assert!(!serializer.chop(3));
}
