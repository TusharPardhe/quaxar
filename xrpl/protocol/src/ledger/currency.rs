//! `Currency` / `MPTID` helpers ported from `xrpl/protocol/UintTypes.*`.

use std::sync::OnceLock;

use basics::base_uint::BaseUInt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct CurrencyTag;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct DirectoryTag;

pub type Currency = BaseUInt<20, CurrencyTag>;
pub type Directory = BaseUInt<32, DirectoryTag>;
pub type Domain = BaseUInt<32>;
pub type LedgerHash = basics::base_uint::Uint256;
pub type MPTID = BaseUInt<24>;

const SYSTEM_CURRENCY_CODE: &str = "XRP";
const ISO_CHAR_SET: &str =
    "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789<>(){}[]|?!@#$%^&*";
const ISO_CODE_OFFSET: usize = 12;
const ISO_CODE_LENGTH: usize = 3;

fn iso_bits_mask() -> Currency {
    static ISO_BITS_MASK: OnceLock<Currency> = OnceLock::new();
    *ISO_BITS_MASK.get_or_init(|| {
        Currency::from_hex("FFFFFFFFFFFFFFFFFFFFFFFF000000FFFFFFFFFF")
            .expect("Currency ISO mask must remain valid")
    })
}

pub fn system_currency_code() -> &'static str {
    SYSTEM_CURRENCY_CODE
}

pub fn xrp_currency() -> Currency {
    Currency::zero()
}

pub fn no_currency() -> Currency {
    Currency::from_u64(1)
}

pub fn bad_currency() -> Currency {
    Currency::from_u64(0x5852_5000_0000_0000)
}

pub fn is_xrp_currency(currency: Currency) -> bool {
    currency.is_zero()
}

pub fn currency_to_string(currency: Currency) -> String {
    if currency.is_zero() {
        return system_currency_code().to_string();
    }

    if currency == no_currency() {
        return "1".to_string();
    }

    if (currency & iso_bits_mask()).is_zero() {
        let iso = &currency.data()[ISO_CODE_OFFSET..ISO_CODE_OFFSET + ISO_CODE_LENGTH];
        if iso
            .iter()
            .all(|byte| ISO_CHAR_SET.as_bytes().contains(byte))
        {
            let iso = String::from_utf8_lossy(iso).to_string();
            if iso != system_currency_code() {
                return iso;
            }
        }
    }

    currency.to_string()
}

pub fn to_currency(currency: &mut Currency, code: &str) -> bool {
    if code.is_empty() || code == system_currency_code() {
        *currency = xrp_currency();
        return true;
    }

    if code.len() == ISO_CODE_LENGTH {
        if code
            .bytes()
            .any(|byte| !ISO_CHAR_SET.as_bytes().contains(&byte))
        {
            return false;
        }

        let mut bytes = [0u8; Currency::BYTES];
        bytes[ISO_CODE_OFFSET..ISO_CODE_OFFSET + ISO_CODE_LENGTH].copy_from_slice(code.as_bytes());
        *currency = Currency::from_array(bytes);
        return true;
    }

    currency.parse_hex(code)
}

pub fn currency_from_string(code: &str) -> Currency {
    let mut currency = Currency::zero();
    if !to_currency(&mut currency, code) {
        currency = no_currency();
    }
    currency
}

pub fn make_mpt_id(sequence: u32, account: crate::AccountID) -> MPTID {
    let mut bytes = [0u8; MPTID::BYTES];
    bytes[..4].copy_from_slice(&sequence.to_be_bytes());
    bytes[4..].copy_from_slice(account.data());
    MPTID::from_array(bytes)
}

#[cfg(test)]
mod tests {
    use super::{
        Currency, bad_currency, currency_from_string, currency_to_string, make_mpt_id, no_currency,
        xrp_currency,
    };
    use basics::base_uint::BaseUInt;

    #[test]
    fn currency_string_conversion_special_cases() {
        assert_eq!(currency_to_string(xrp_currency()), "XRP");
        assert_eq!(currency_to_string(no_currency()), "1");
        assert_eq!(
            currency_to_string(bad_currency()),
            "0000000000000000000000005852500000000000"
        );

        let usd = currency_from_string("USD");
        assert_eq!(currency_to_string(usd), "USD");

        let hex =
            Currency::from_hex("0123456789ABCDEFFEDCBA987654321001234567").expect("hex currency");
        assert_eq!(
            currency_to_string(hex),
            "0123456789ABCDEFFEDCBA987654321001234567"
        );
    }

    #[test]
    fn make_mpt_id_keeps_big_endian_sequence_then_account() {
        let account =
            BaseUInt::<20>::from_hex("B5F762798A53D543A014CAF8B297CFF8F2F937E8").expect("account");
        let mpt_id = make_mpt_id(
            0x0102_0304,
            crate::AccountID::from_slice(account.data()).expect("account width"),
        );

        assert_eq!(
            mpt_id.to_string(),
            "01020304B5F762798A53D543A014CAF8B297CFF8F2F937E8"
        );
    }
}
