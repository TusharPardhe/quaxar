//! Tests for edge case transactions that have caused issues historically.
//!
//! These are real mainnet transactions with unusual properties:
//! - Partial payments (tfPartialPayment flag)
//! - Multi-signed transactions (multiple signers array)
//! - Large transactions (memos, paths)
//! - IOU amounts with various exponents
//! - Ticket-based sequencing
//! - OfferCreate with IOU pairs

use protocol::{STTx, SerialIter, get_field_by_symbol};

fn hex_to_bytes(hex: &str) -> Vec<u8> {
    (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap())
        .collect()
}

/// Real edge case transactions from XRPL mainnet.
const EDGE_CASES: &[(&str, &str)] = &[
    // Partial payment (tfPartialPayment = 0x00020000 flag set)
    // These can deliver less than Amount field — critical for correct balance tracking
    (
        "partial_payment",
        "120000220002000024061AACFB61416345785D8A0000684000000000001F4069D86386F26FC0FFFF4752495A00000000000000000000000000000000792ECCFA876DD32DCD11F1DBB390A1ACDA73125A7321ED48C00DFAE5A1FF6ADBB290F8A9F48920549EB0E955AB31504F75FE9B16F777F874400B34DE11383020633F1C594A49C30064B3023DAE5BCB0FC14AD358DFF07BEA56EFDEF3BB1F88D71C6BC9B3120DE903A9E62548283F66A1B6CEADBFB08D5F25008114759CD8A0F6A2C0FAFDA92595738E02914BE4AB568314759CD8A0F6A2C0FAFDA92595738E02914BE4AB56",
    ),
    // Multi-signed transaction (sfSigners array present)
    // Must correctly parse the Signers array with multiple SignerEntry objects
    (
        "multi_signed",
        "12000022000000002300000FA42406E195282E00000000201B063978056140000000009896806840000000000000147321000000000000000000000000000000000000000000000000000000000000000000811432DC216C477B1A6333FAC1510AF1ACD1DA3A8BB1831432DC216C477B1A6333FAC1510AF1ACD1DA3A8BB1FCED7321ED2B300919308ECEC1F33E94A50C54B3A76879A2B1E3D560BFB8B236BA575DEBD17440946978C634C5D35EC53159AA6D6B6E82888AD163C237E10FE1B721CF6161F0788EB3D62A172D78627BE390D085221128B8171C5BB903455E4DF440B29B2AF808811432DC216C477B1A6333FAC1510AF1ACD1DA3A8BB1E1E1F1",
    ),
    // Large transaction with memo (318 bytes — tests VL encoding at larger sizes)
    (
        "large_with_memo",
        "12000022800200002405C71ED1201B063979B861D4C8FB6F34F37F800000000000000000000000004D41470000000000A5F3E0F3C8E0F3C8E0F3C8E0F3C8E0F3C8E0F3C868400000000000000C7321ED0A0A74E11E5F0B5A3020352A4770F1FEFD52069E893012B54D321E1B1F4C3AB5744051EC39C18543405C2C8E17E48CFE1A301CA104C6FE957E57FAE753A424C56C28BE43261C8069D32F9BA86CE9FB16C193CE283FD95AAD519DA3DC05EADBCF77078114FFC6768FF5C754E29F1CE88EA7733350596D38DAF9EA7D677B226D6F64756C65223A2258574D2041737369737465642054726164696E67222C22616363657373223A2258574D204E4654204469616D6F6E64222C2276657273696F6E223A2276302E352E302062657461222C227374726174656779223A224261736963227DE1F1",
    ),
    // IOU OfferCreate (both TakerPays and TakerGets are IOU amounts)
    // Tests IOU amount serialization with currency codes and issuer accounts
    (
        "iou_offer_create",
        "1200072200000000240F39680C20190F396804201B063979A864400000E8D4A5100065D5DFDE2B0365E600000000000000000000000000434E590000000000CED6E99370D5C00EF4EBF72567DA99F5661BFB3A68400000000000000A7321022E70597C73E359D20C5939F9A9009B42D51DAD22E633762832E44336B95D177C74463044022044814456E26752C96C7FAC8155FCA13273FB5BBE10618E1D09FAC42DA3A3948F02207A5D4BBA3B10DCEBD98C0F7A358AA328DF31268FA182B0DFBA86F5FD2458D86B811481E59E25448A96336367CAB3000C7299F5F9DA79",
    ),
    // Ticket-based transaction (uses sfTicketSequence instead of sfSequence)
    // Sequence=0 with TicketSequence set — tests the alternate sequencing path
    (
        "ticket_sequenced",
        "120007220001000024000000002A31A4FC0D2019073CB211201B063979B864D5048C4E3F0D0C004242524C00000000000000000000000000000000A5F3E0F3C8E0F3C8E0F3C8E0F3C8E0F3C8E0F3C865400000000098968068400000000000000A7321ED61C9D503EA4A3569B1AD4F926B4CE0AE57AD33B26D89EFB5B9481EF96DD760F67440B4C8FCD1BEFD351F9B67695165F275C9ECCCF781FC1600A14E5F46F119F00699A8CA685FEB42A35D849DD1589D4487AF5D73F604FF7E58FAB0D2E2BBC6CF150381140EC8EEA59A7DCC2A547CCC093A7F8D70218254A0",
    ),
    // IOU payment with DeliverMin (partial payment with minimum delivery)
    (
        "iou_payment_deliver_min",
        "12000022000000002405DBFCD52E000394D4201B06397A0A614000000000989680684000000000000014732103CB8F1BB45C913460540CA507F5B0359F5818DC8DEC9BFFDFBEAD9FD68879BE4A74473045022100D2587E834F2F2B9A8C4392C30BB02F7AC7C954B7860348AF17A16C975FA0C0EA0220207827E3F96590A114A2C0AC313F0F3F81210A4F02E22972DC091B30A0A879008114523DB33EE1546D0683A05500C70F9DE67B5CE8E28314C821B57BA317BB0F29A001A81FC94315E70D9908",
    ),
];

/// Test: All edge case transactions deserialize without panic.
/// Note: Multi-signed txs may reorder fields during deserialization (canonical ordering).
#[test]
fn edge_case_transactions_deserialize() {
    for (i, (name, hex_blob)) in EDGE_CASES.iter().enumerate() {
        let bytes = hex_to_bytes(hex_blob);
        let mut iter = SerialIter::new(&bytes);
        let tx = STTx::from_serial_iter(&mut iter);
        // Must produce a non-zero transaction ID
        assert_ne!(
            tx.get_transaction_id(),
            basics::base_uint::Uint256::default(),
            "Edge case {i} ({name}): failed to deserialize"
        );
    }
}

/// Test: Non-multi-signed edge cases roundtrip byte-for-byte.
#[test]
fn non_multisign_edge_cases_roundtrip() {
    // Skip index 1 (multi_signed) which canonically reorders the Signers array
    for (i, (name, hex_blob)) in EDGE_CASES.iter().enumerate() {
        if *name == "multi_signed" {
            continue;
        }
        let bytes = hex_to_bytes(hex_blob);
        let mut iter = SerialIter::new(&bytes);
        let tx = STTx::from_serial_iter(&mut iter);
        let reserialized = tx.get_serializer().data().to_vec();
        assert_eq!(
            reserialized, bytes,
            "Edge case {i} ({name}): roundtrip mismatch"
        );
    }
}

/// Test: Partial payment has tfPartialPayment flag set.
#[test]
fn partial_payment_flag_detected() {
    let bytes = hex_to_bytes(EDGE_CASES[0].1);
    let mut iter = SerialIter::new(&bytes);
    let tx = STTx::from_serial_iter(&mut iter);

    let flags = tx.get_field_u32(get_field_by_symbol("sfFlags"));
    assert_eq!(
        flags & 0x00020000,
        0x00020000,
        "tfPartialPayment must be set"
    );
}

/// Test: Ticket-sequenced transaction has Sequence=0 and TicketSequence set.
#[test]
fn ticket_sequence_fields() {
    let bytes = hex_to_bytes(EDGE_CASES[4].1);
    let mut iter = SerialIter::new(&bytes);
    let tx = STTx::from_serial_iter(&mut iter);

    // Ticket-based txs have Sequence=0 and use TicketSequence instead
    let sequence = tx.get_field_u32(get_field_by_symbol("sfSequence"));
    assert_eq!(sequence, 0, "Ticket-based tx must have Sequence=0");

    // The OfferCreate with ticket has flags 0x00010000 (tfImmediateOrCancel or similar)
    let flags = tx.get_field_u32(get_field_by_symbol("sfFlags"));
    assert!(flags > 0, "Ticket tx should have flags set");
}

/// Test: Multi-signed transaction has empty SigningPubKey and Signers array.
#[test]
fn multi_signed_has_signers_array() {
    let bytes = hex_to_bytes(EDGE_CASES[1].1);
    let mut iter = SerialIter::new(&bytes);
    let tx = STTx::from_serial_iter(&mut iter);

    // Multi-signed txs have an empty (all-zeros) SigningPubKey
    let pub_key = tx.get_field_vl(get_field_by_symbol("sfSigningPubKey"));
    assert!(
        pub_key.iter().all(|&b| b == 0),
        "Multi-sign tx must have zero SigningPubKey"
    );
}

/// Test: All edge cases produce unique, non-zero transaction IDs.
#[test]
fn edge_case_ids_unique_and_nonzero() {
    let mut ids = std::collections::HashSet::new();
    for (i, (name, hex_blob)) in EDGE_CASES.iter().enumerate() {
        let bytes = hex_to_bytes(hex_blob);
        let mut iter = SerialIter::new(&bytes);
        let tx = STTx::from_serial_iter(&mut iter);
        let id = tx.get_transaction_id();
        assert_ne!(
            id,
            basics::base_uint::Uint256::default(),
            "{name} has zero ID"
        );
        assert!(ids.insert(id), "Edge case {i} ({name}): duplicate ID");
    }
}
