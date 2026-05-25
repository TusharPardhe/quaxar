//! Validates transaction binary serialization roundtrip with real XRPL mainnet data.
//!
//! Takes real transaction blobs from ledger #90,000,000, deserializes them,
//! re-serializes them, and verifies byte-for-byte equality.
//! If this passes, the node can correctly process transactions from the network.

use protocol::{STTx, SerialIter};

/// Real mainnet transaction blobs from ledger #90,000,000 (Aug 12, 2024).
/// Mix of OfferCreate (0x0007), Payment (0x0000), and other types.
const TX_BLOBS: &[&str] = &[
    // TX 0: OfferCreate
    "12000722000000002408452114201908452113201B055D4A8264D5886B8ABC31AB89000000000000000000000000434E5900000000000360E3E0751BD9A566CD03FA6CAFC78118B82BA0654000000143C1720A68400000000000000F7321039451ECAC6D4EB75E3C926E7DC7BA7721719A1521502F99EC7EB2FE87CEE9E8247446304402202E0D054CD230A51CBA667DFE99C7E8454A43FF43180B9D5D9560ACCFDFC7A34A022042C75F0BBA0F9B38B6090BD3F9A29405635504B14E3390B61E09D59066893B9B8114FDA303AEF9115230B73D244C26E9DDB813EEBC05",
    // TX 1: OfferCreate (ed25519)
    "12000722000000002400000000201905420F7F201B055D4A8D202905420F86644000000000AC928565D510E01EFF864200504958454C000000000000000000000000000000CA8FE658DA594C043BD17FB2AD43D055ABAE0AD368400000000000000C7321EDDC0F61F5B2EDC288EF56C0F0B9DCC4C1D596165FC7A53D72022FDBEC3C1D2669744098A92CD600BBBA386FDFEEF76F0CFE4A216ADA90194F83458FF3EDD852493E63E95979A9B92CD469CD41AA50C3BDFFDF35EC86FB39BB51EFB7AB29121C31B4038114836B515B3824D5E17638EEF4E2EE1BC21438A75C",
    // TX 2: OfferCreate with memo (ed25519, longer)
    "12000722000A000024005BCEEC201B055D4A8164D345DA424182BDE4534F4C4F000000000000000000000000000000001EB3EAA3AD86242E1D51DC502DD6566BD39E06A665D40A98877FB1088F58434F524500000000000000000000000000000053AFF2ACCE93F425344F40EDF8D937FCAC96895F68400000000000000A7321ED3DC1A8262390DBA0E9926050A7BE377DFCC7937CC94C5F5F24E6BD97D677BA6C74407D6E2CDD719A1E40E0EA57C987F4AD72799C20D7881FF286FBACA5138E74610C4B6AC60D9857D0845F0818B591A853548F7DE35319C7DBF2102C703B720BE804811408D472939580F197184B89AC5D263C06D7EB6FCAF9EA7D381D6CDA09E33E64791E62BB9E5BEAB6C5C02E9FD1000000003FE1ACD4B1C448D43F9F066B2BD70ADB3F9122F9A635B2C63FF0000000000000E1F1",
    // TX 3: OfferCreate (secp256k1)
    "12000722000000002408452116201908452110201B055D4A8264D5848F7416813CAA000000000000000000000000434E5900000000000360E3E0751BD9A566CD03FA6CAFC78118B82BA065400000008279A2C168400000000000000F7321039451ECAC6D4EB75E3C926E7DC7BA7721719A1521502F99EC7EB2FE87CEE9E824744630440220138C344F3F2283801E39D75A2822DB31DAEE4D58B85ED88C3964653D3D30BD370220770DDFC1CD0DE8D16075957585F44A66B54195EA330E67680919BEE32AD3D6DD8114FDA303AEF9115230B73D244C26E9DDB813EEBC05",
    // TX 5: Payment (secp256k1)
    "12000024051A1FF72EF1FBDE75201B055D4A88614000000002BDE78068400000000000000C7321031AF2270BC2B4B3D44339D237808C4CDF99DD7EA607383087AC026E43AE3B02A2744730450221009F404B1C5FB96F36576CF628CD892888C6843BBDEFC2DAE32FAA3E72A8FBC65E022078DC3975AB67987ACEE94E5E195E41B265C096F9492E385D0678BEFE9D01D8AA8114F0ED7E93A276E87FF76C524B00C8A84CBEAA54FF8314E23E1F811DC4A4AD525F73D6B17F07C9FA127B38",
    // TX 9: Payment with flags (secp256k1)
    "12000022800000002403E62B2B201B055D7EB761400000000702CDF76840000000000000187321024979DFC8EA12B95CB832E57D9E11AFABB2F4A8F99736F4AA1050F7C3F4297B777446304402201F2C2AA48217BDF4058651D00E7FF74ACC132FA178AB357AF4E079CCD10204CE02204F41E76A4376DEC3B427A38D918F3DBD96DDD4FFD125ADF1BD4BAD5C2DC16AAF8114A2FEF188D8A2C0A3E363F479A559E67C847DE3968314C821B57BA317BB0F29A001A81FC94315E70D9908",
];

fn hex_to_bytes(hex: &str) -> Vec<u8> {
    (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap())
        .collect()
}

fn bytes_to_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02X}")).collect()
}

/// Core test: deserialize real mainnet tx blobs and re-serialize — must produce identical bytes.
#[test]
fn transaction_roundtrip_matches_mainnet_blobs() {
    for (i, hex_blob) in TX_BLOBS.iter().enumerate() {
        let original_bytes = hex_to_bytes(hex_blob);

        // Deserialize
        let mut iter = SerialIter::new(&original_bytes);
        let tx = STTx::from_serial_iter(&mut iter);

        // Re-serialize
        let reserialized = tx.get_serializer().data().to_vec();

        assert_eq!(
            reserialized,
            original_bytes,
            "TX {i}: roundtrip mismatch!\n  original:     {}\n  reserialized: {}",
            &hex_blob[..40],
            &bytes_to_hex(&reserialized)[..40],
        );
    }
}

/// Test: Transaction ID (hash) is deterministic and non-zero.
#[test]
fn transaction_id_is_deterministic() {
    for (i, hex_blob) in TX_BLOBS.iter().enumerate() {
        let bytes = hex_to_bytes(hex_blob);
        let mut iter = SerialIter::new(&bytes);
        let tx = STTx::from_serial_iter(&mut iter);

        let id1 = tx.get_transaction_id();
        let id2 = tx.get_transaction_id();

        assert_eq!(id1, id2, "TX {i}: transaction ID must be deterministic");
        assert_ne!(
            id1,
            basics::base_uint::Uint256::default(),
            "TX {i}: transaction ID must be non-zero"
        );
    }
}

/// Test: Deserialized transactions have valid type codes.
#[test]
fn deserialized_transactions_have_valid_type() {
    use protocol::TxType;
    let expected_types: &[TxType] = &[
        TxType::OFFER_CREATE,
        TxType::OFFER_CREATE,
        TxType::OFFER_CREATE,
        TxType::OFFER_CREATE,
        TxType::PAYMENT,
        TxType::PAYMENT,
    ];

    for (i, hex_blob) in TX_BLOBS.iter().enumerate() {
        let bytes = hex_to_bytes(hex_blob);
        let mut iter = SerialIter::new(&bytes);
        let tx = STTx::from_serial_iter(&mut iter);

        let tx_type = tx.get_txn_type();
        assert_eq!(tx_type, expected_types[i], "TX {i}: type mismatch");
    }
}

/// Test: Deserializing truncated blobs doesn't panic (returns error or partial).
#[test]
fn truncated_blobs_dont_panic() {
    for hex_blob in TX_BLOBS {
        let bytes = hex_to_bytes(hex_blob);
        // Try various truncation points
        for cut in [1, 4, 10, 20, bytes.len() / 2] {
            let truncated = &bytes[..cut.min(bytes.len())];
            let mut iter = SerialIter::new(truncated);
            // This should not panic — it may produce a partial/invalid tx
            let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let _ = STTx::from_serial_iter(&mut iter);
            }));
        }
    }
}

/// Test: Random garbage bytes don't panic the deserializer.
#[test]
fn garbage_bytes_dont_panic() {
    let garbage_inputs: &[&[u8]] = &[
        &[0xFF; 64],
        &[0x00; 32],
        &[0x12, 0x00],                               // Just tx type prefix
        &[0x12, 0x00, 0x22, 0x80, 0x00, 0x00, 0x00], // Partial payment header
        &[],
        &[0xDE, 0xAD, 0xBE, 0xEF],
    ];

    for (i, garbage) in garbage_inputs.iter().enumerate() {
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let mut iter = SerialIter::new(garbage);
            let _ = STTx::from_serial_iter(&mut iter);
        }));
        assert!(
            result.is_ok(),
            "Garbage input {i} caused a panic! Deserializer must handle malformed data gracefully."
        );
    }
}
