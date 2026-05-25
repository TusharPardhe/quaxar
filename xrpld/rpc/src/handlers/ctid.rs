//! Concise Transaction ID helpers ported from `xrpld/rpc/CTID.h`.

pub const CTID_PREFIX: u64 = 0xC000_0000_0000_0000;
pub const CTID_PREFIX_MASK: u64 = 0xF000_0000_0000_0000;
pub const CTID_LEDGER_PREFIX: u64 = 0xC000_0000;
pub const MAX_LEDGER_SEQ: u32 = 0x0FFF_FFFF;
pub const MAX_TXN_INDEX: u32 = 0xFFFF;
pub const MAX_NETWORK_ID: u32 = 0xFFFF;

pub fn encode_ctid(ledger_seq: u32, txn_index: u32, network_id: u32) -> Option<String> {
    if ledger_seq > MAX_LEDGER_SEQ || txn_index > MAX_TXN_INDEX || network_id > MAX_NETWORK_ID {
        return None;
    }

    let ctid_value = ((CTID_LEDGER_PREFIX + u64::from(ledger_seq)) << 32)
        | ((u64::from(txn_index) << 16) | u64::from(network_id));
    Some(format!("{ctid_value:016X}"))
}

pub trait CtidInput {
    fn into_ctid_value(self) -> Option<u64>;
}

impl CtidInput for u32 {
    fn into_ctid_value(self) -> Option<u64> {
        Some(u64::from(self))
    }
}

impl CtidInput for usize {
    fn into_ctid_value(self) -> Option<u64> {
        Some(self as u64)
    }
}

macro_rules! impl_ctid_input_integral {
    ($($ty:ty),* $(,)?) => {
        $(
            impl CtidInput for $ty {
                fn into_ctid_value(self) -> Option<u64> {
                    Some(self as u64)
                }
            }
        )*
    };
}

impl_ctid_input_integral!(u8, u16, u64, u128, i8, i16, i32, i64, i128, isize);

impl CtidInput for &str {
    fn into_ctid_value(self) -> Option<u64> {
        if self.len() != 16 || !self.chars().all(|c| c.is_ascii_hexdigit()) {
            return None;
        }

        u64::from_str_radix(self, 16).ok()
    }
}

impl CtidInput for String {
    fn into_ctid_value(self) -> Option<u64> {
        self.as_str().into_ctid_value()
    }
}

impl CtidInput for &String {
    fn into_ctid_value(self) -> Option<u64> {
        self.as_str().into_ctid_value()
    }
}

pub fn decode_ctid<T: CtidInput>(ctid: T) -> Option<(u32, u16, u16)> {
    let ctid_value = ctid.into_ctid_value()?;

    if (ctid_value & CTID_PREFIX_MASK) != CTID_PREFIX {
        return None;
    }

    let ledger_seq = ((ctid_value >> 32) & u64::from(MAX_LEDGER_SEQ)) as u32;
    let txn_index = ((ctid_value >> 16) & u64::from(MAX_TXN_INDEX)) as u16;
    let network_id = (ctid_value & u64::from(MAX_NETWORK_ID)) as u16;
    Some((ledger_seq, txn_index, network_id))
}

#[cfg(test)]
mod tests {
    use super::{decode_ctid, encode_ctid};

    #[test]
    fn ctid_round_trips() {
        let ctid = encode_ctid(0x1234567, 0xABCD, 0xEEFF).expect("ctid should encode");
        assert_eq!(ctid, "C1234567ABCDEEFF");
        assert_eq!(
            decode_ctid(ctid.as_str()),
            Some((0x1234567, 0xABCD, 0xEEFF))
        );
        assert_eq!(
            decode_ctid(u64::from_str_radix(&ctid, 16).expect("hex")),
            Some((0x1234567, 0xABCD, 0xEEFF))
        );
    }

    #[test]
    fn ctid_rejects_bad_inputs() {
        assert_eq!(encode_ctid(0x1000_0000, 0, 0), None);
        assert_eq!(decode_ctid("123"), None);
        assert_eq!(decode_ctid("FFFFFFFFFFFFFFFF"), None);
        assert_eq!(decode_ctid("C1234567ABCDEEFG"), None);
    }
}
