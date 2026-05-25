use basics::{base_uint::Uint256, chrono::NetClockTimePoint, slice::Slice};
use protocol::Serializer;

pub fn proposal_unique_id(
    propose_hash: Uint256,
    previous_ledger: Uint256,
    propose_seq: u32,
    close_time: NetClockTimePoint,
    public_key: Slice<'_>,
    signature: Slice<'_>,
) -> Uint256 {
    let mut serializer = Serializer::new(512);
    serializer.add_bit_string(propose_hash);
    serializer.add_bit_string(previous_ledger);
    serializer.add32(propose_seq);
    serializer.add32(close_time.as_seconds());
    serializer.add_vl(public_key.data());
    serializer.add_vl(signature.data());
    serializer.get_sha512_half()
}

pub fn rcl_txset_id(tx_ids: &[Uint256]) -> Uint256 {
    let mut serializer = Serializer::new(64 * tx_ids.len().max(1));
    for tx_id in tx_ids {
        serializer.add_bit_string(*tx_id);
    }
    serializer.get_sha512_half()
}

#[cfg(test)]
mod tests {
    use basics::{base_uint::Uint256, chrono::NetClockTimePoint, slice::Slice};
    use protocol::Serializer;

    use super::{proposal_unique_id, rcl_txset_id};

    #[test]
    fn proposal_unique_id_matches_current_serializer_layout() {
        let propose_hash = Uint256::from_u64(1);
        let previous_ledger = Uint256::from_u64(2);
        let close_time = NetClockTimePoint::new(33);
        let public_key = [0x11u8; 33];
        let signature = [0x22u8; 64];

        let mut serializer = Serializer::new(512);
        serializer.add_bit_string(propose_hash);
        serializer.add_bit_string(previous_ledger);
        serializer.add32(7);
        serializer.add32(close_time.as_seconds());
        serializer.add_vl(public_key);
        serializer.add_vl(signature);

        assert_eq!(
            proposal_unique_id(
                propose_hash,
                previous_ledger,
                7,
                close_time,
                Slice::new(&public_key),
                Slice::new(&signature),
            ),
            serializer.get_sha512_half()
        );
    }

    #[test]
    fn rcl_txset_id_matches_current_serializer_layout() {
        let tx_ids = [Uint256::from_u64(9), Uint256::from_u64(3)];
        let mut serializer = Serializer::new(64 * tx_ids.len());
        for tx_id in tx_ids {
            serializer.add_bit_string(tx_id);
        }

        assert_eq!(rcl_txset_id(&tx_ids), serializer.get_sha512_half());
    }
}
