use overlay::{
    Compressed, CompressionAlgorithm, Message, ProtocolMessage, ProtocolPayload, TmManifests,
    decode_protocol_message, parse_message_header,
};

#[test]
fn compressed_tm_manifests_frame_header_shape() {
    let manifests = TmManifests {
        list: (0..40)
            .map(|index| overlay::message::wire::TmManifest {
                stobject: vec![index as u8; 100],
            })
            .collect(),
        ..Default::default()
    };
    let message = Message::new(
        ProtocolMessage::new(ProtocolPayload::Manifests(manifests)),
        None,
    );

    let compressed = message.get_buffer(Compressed::On);
    let header = parse_message_header(compressed)
        .expect("header parse")
        .expect("header present");

    assert_eq!(header.header_size, 10);
    assert_eq!(header.algorithm, CompressionAlgorithm::Lz4);
    assert_eq!(
        header.message_type,
        overlay::ProtocolMessageType::MtManifests as u16
    );
    assert!(header.payload_wire_size < header.uncompressed_size);
    assert_eq!(header.total_wire_size as usize, compressed.len());
    assert_eq!(compressed[0] & 0xF0, CompressionAlgorithm::Lz4 as u8);

    let decoded = decode_protocol_message(compressed, true).expect("decode compressed message");
    assert_eq!(decoded.consumed, compressed.len());
    assert!(matches!(
        decoded.message,
        Some(ProtocolMessage {
            payload: ProtocolPayload::Manifests(_),
            ..
        })
    ));
}
