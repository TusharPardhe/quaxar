//! Singleton ledger-entry adapter for the current `Ledger::setup()` path.
//!
//! This intentionally covers only the singleton state entries and fields that
//! the narrowed Rust ledger setup seam consumes today:
//! - `keylet::amendments()`
//! - `keylet::fees()`
//! - amendment vectors
//! - legacy fee scalars
//! - XRP-amount fee fields
//!
//! The raw byte decoders stay here for malformed-wire coverage, while the
//! ledger owner can now also consume the same singleton objects through the
//! landed `STLedgerEntry` surface.

use crate::{
    Fees,
    setup::{AmendmentsEntry, FeeSettingsFields},
};
use basics::base_uint::Uint256;
use protocol::{
    LedgerEntryDecodeError, LedgerEntryType, STLedgerEntry, SerialIter, amendments_keylet,
    decode_constructor_amendments_entry as protocol_decode_constructor_amendments_entry,
    decode_fee_settings_entry as protocol_decode_fee_settings_entry,
    encode_amendments_entry as protocol_encode_amendments_entry,
    encode_fee_settings_entry as protocol_encode_fee_settings_entry, fee_settings_keylet,
    get_field_by_symbol,
};
use shamap::traversal::TraversalError;
use std::panic::{AssertUnwindSafe, catch_unwind};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LedgerSetupError {
    Traversal(TraversalError),
    MissingLedgerEntryType,
    UnexpectedLedgerEntryType { expected: u16, actual: u16 },
    MissingField(&'static str),
    TruncatedField,
    InvalidVariableLengthPrefix(u8),
    InvalidVector256Length(usize),
    UnsupportedFieldType(u8),
    DuplicateField(&'static str),
    UnexpectedObjectEnd,
    UnexpectedArrayEnd,
    MissingObjectEnd,
    MissingArrayEnd,
}

impl From<TraversalError> for LedgerSetupError {
    fn from(error: TraversalError) -> Self {
        Self::Traversal(error)
    }
}

impl From<LedgerEntryDecodeError> for LedgerSetupError {
    fn from(error: LedgerEntryDecodeError) -> Self {
        match error {
            LedgerEntryDecodeError::MissingLedgerEntryType => Self::MissingLedgerEntryType,
            LedgerEntryDecodeError::UnexpectedLedgerEntryType { expected, actual } => {
                Self::UnexpectedLedgerEntryType { expected, actual }
            }
            LedgerEntryDecodeError::MissingField(name) => Self::MissingField(name),
            LedgerEntryDecodeError::TruncatedField => Self::TruncatedField,
            LedgerEntryDecodeError::InvalidVariableLengthPrefix(prefix) => {
                Self::InvalidVariableLengthPrefix(prefix)
            }
            LedgerEntryDecodeError::InvalidVector256Length(length) => {
                Self::InvalidVector256Length(length)
            }
            LedgerEntryDecodeError::UnsupportedFieldType(field_type) => {
                Self::UnsupportedFieldType(field_type)
            }
            LedgerEntryDecodeError::DuplicateField(name) => Self::DuplicateField(name),
            LedgerEntryDecodeError::UnexpectedObjectEnd => Self::UnexpectedObjectEnd,
            LedgerEntryDecodeError::UnexpectedArrayEnd => Self::UnexpectedArrayEnd,
            LedgerEntryDecodeError::MissingObjectEnd => Self::MissingObjectEnd,
            LedgerEntryDecodeError::MissingArrayEnd => Self::MissingArrayEnd,
        }
    }
}

pub fn parse_amendments_sle(payload: &[u8]) -> Option<STLedgerEntry> {
    parse_singleton_sle(payload, amendments_keylet(), LedgerEntryType::Amendments)
}

pub fn parse_fee_settings_sle(payload: &[u8]) -> Option<STLedgerEntry> {
    parse_singleton_sle(payload, fee_settings_keylet(), LedgerEntryType::FeeSettings)
}

pub fn decode_amendments_entry_from_sle(
    payload: &[u8],
    digest: Uint256,
) -> Option<AmendmentsEntry> {
    let entry = parse_amendments_sle(payload)?;
    let amendments = if entry.is_field_present(get_field_by_symbol("sfAmendments")) {
        entry
            .get_field_v256(get_field_by_symbol("sfAmendments"))
            .value()
            .to_vec()
    } else {
        Vec::new()
    };

    Some(AmendmentsEntry { digest, amendments })
}

pub fn decode_fee_settings_fields_from_sle(payload: &[u8]) -> Option<FeeSettingsFields> {
    let entry = parse_fee_settings_sle(payload)?;
    let mut decoded = FeeSettingsFields::default();

    if entry.is_field_present(get_field_by_symbol("sfBaseFee")) {
        decoded.base_fee = Some(entry.get_field_u64(get_field_by_symbol("sfBaseFee")));
    }
    if entry.is_field_present(get_field_by_symbol("sfReferenceFeeUnits")) {
        decoded.reference_fee_units =
            Some(entry.get_field_u32(get_field_by_symbol("sfReferenceFeeUnits")));
    }
    if entry.is_field_present(get_field_by_symbol("sfReserveBase")) {
        decoded.reserve_base = Some(entry.get_field_u32(get_field_by_symbol("sfReserveBase")));
    }
    if entry.is_field_present(get_field_by_symbol("sfReserveIncrement")) {
        decoded.reserve_increment =
            Some(entry.get_field_u32(get_field_by_symbol("sfReserveIncrement")));
    }
    if entry.is_field_present(get_field_by_symbol("sfBaseFeeDrops")) {
        decoded.base_fee_drops = Some(decoded_amount_field_from_stamount(
            entry.get_field_amount(get_field_by_symbol("sfBaseFeeDrops")),
        ));
    }
    if entry.is_field_present(get_field_by_symbol("sfReserveBaseDrops")) {
        decoded.reserve_base_drops = Some(decoded_amount_field_from_stamount(
            entry.get_field_amount(get_field_by_symbol("sfReserveBaseDrops")),
        ));
    }
    if entry.is_field_present(get_field_by_symbol("sfReserveIncrementDrops")) {
        decoded.reserve_increment_drops = Some(decoded_amount_field_from_stamount(
            entry.get_field_amount(get_field_by_symbol("sfReserveIncrementDrops")),
        ));
    }
    if entry.is_field_present(get_field_by_symbol("sfPreviousTxnID")) {
        decoded.previous_txn_id =
            Some(entry.get_field_h256(get_field_by_symbol("sfPreviousTxnID")));
    }
    if entry.is_field_present(get_field_by_symbol("sfPreviousTxnLgrSeq")) {
        decoded.previous_txn_lgr_seq =
            Some(entry.get_field_u32(get_field_by_symbol("sfPreviousTxnLgrSeq")));
    }

    Some(decoded)
}

pub fn decode_amendments_entry(
    payload: &[u8],
    digest: Uint256,
) -> Result<AmendmentsEntry, LedgerSetupError> {
    let decoded = protocol_decode_constructor_amendments_entry(payload)?;
    Ok(AmendmentsEntry {
        digest,
        amendments: decoded.amendments,
    })
}

pub fn decode_fee_settings_fields(payload: &[u8]) -> Result<FeeSettingsFields, LedgerSetupError> {
    Ok(protocol_decode_fee_settings_entry(payload)?)
}

pub fn encode_amendments_entry(amendments: &[Uint256]) -> Vec<u8> {
    protocol_encode_amendments_entry(amendments)
}

pub fn encode_fee_settings_entry(fees: Fees, xrp_fees_enabled: bool) -> Vec<u8> {
    protocol_encode_fee_settings_entry(fees.base, fees.reserve, fees.increment, xrp_fees_enabled)
}

fn parse_singleton_sle(
    payload: &[u8],
    keylet: protocol::Keylet,
    expected_type: LedgerEntryType,
) -> Option<STLedgerEntry> {
    let mut iter = SerialIter::new(payload);
    let entry = catch_unwind(AssertUnwindSafe(|| {
        STLedgerEntry::from_serial_iter(&mut iter, keylet.key)
    }))
    .ok()?;
    if !iter.empty() || entry.get_type() != expected_type {
        return None;
    }
    Some(entry)
}

fn decoded_amount_field_from_stamount(amount: protocol::STAmount) -> protocol::DecodedAmountField {
    protocol::DecodedAmountField {
        drops: amount.mantissa(),
        native: amount.native(),
        negative: amount.negative(),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        decode_amendments_entry, decode_amendments_entry_from_sle, decode_fee_settings_fields,
        decode_fee_settings_fields_from_sle, encode_amendments_entry, encode_fee_settings_entry,
    };
    use crate::{AmountField, Fees};
    use basics::base_uint::Uint256;
    use protocol::{
        ConstructorAmendmentsEntry, STAmount, STArray, STLedgerEntry, STObject, STVector256,
        amendments_keylet, fee_settings_keylet, get_field_by_symbol,
    };

    const STI_UINT16: u8 = 1;
    const STI_UINT32: u8 = 2;
    const STI_UINT64: u8 = 3;
    const STI_UINT256: u8 = 5;
    const STI_AMOUNT: u8 = 6;
    const STI_OBJECT: u8 = 14;
    const STI_ARRAY: u8 = 15;
    const STI_VECTOR256: u8 = 19;

    const LT_AMENDMENTS: u16 = 0x0066;
    const LT_FEE_SETTINGS: u16 = 0x0073;
    const OBJECT_END: u8 = 0xE1;
    const ARRAY_END: u8 = 0xF1;
    const STAMOUNT_POSITIVE: u64 = 0x4000_0000_0000_0000;

    fn field_id(field_type: u8, field_name: u8) -> Vec<u8> {
        if field_type < 16 && field_name < 16 {
            vec![(field_type << 4) | field_name]
        } else if field_type < 16 {
            vec![field_type << 4, field_name]
        } else if field_name < 16 {
            vec![field_name, field_type]
        } else {
            vec![0, field_type, field_name]
        }
    }

    fn encode_u16(field_name: u8, value: u16) -> Vec<u8> {
        let mut bytes = field_id(STI_UINT16, field_name);
        bytes.extend_from_slice(&value.to_be_bytes());
        bytes
    }

    fn encode_u32(field_name: u8, value: u32) -> Vec<u8> {
        let mut bytes = field_id(STI_UINT32, field_name);
        bytes.extend_from_slice(&value.to_be_bytes());
        bytes
    }

    fn encode_u64(field_name: u8, value: u64) -> Vec<u8> {
        let mut bytes = field_id(STI_UINT64, field_name);
        bytes.extend_from_slice(&value.to_be_bytes());
        bytes
    }

    fn encode_native_amount(field_name: u8, value: u64) -> Vec<u8> {
        let mut bytes = field_id(STI_AMOUNT, field_name);
        bytes.extend_from_slice(&(value | STAMOUNT_POSITIVE).to_be_bytes());
        bytes
    }

    fn encode_vector256(field_name: u8, values: &[Uint256]) -> Vec<u8> {
        let mut bytes = field_id(STI_VECTOR256, field_name);
        let payload_len = values.len() * Uint256::BYTES;
        bytes
            .push(u8::try_from(payload_len).expect("small vector256 payload must fit in one byte"));
        for value in values {
            bytes.extend_from_slice(value.data());
        }
        bytes
    }

    fn encode_uint256(field_name: u8, value: Uint256) -> Vec<u8> {
        let mut bytes = field_id(STI_UINT256, field_name);
        bytes.extend_from_slice(value.data());
        bytes
    }

    fn encode_majorities_array() -> Vec<u8> {
        let amendment = Uint256::from_array([0xAB; 32]);
        let mut bytes = field_id(STI_ARRAY, 16);
        bytes.extend_from_slice(&field_id(STI_OBJECT, 18));
        bytes.extend_from_slice(&encode_u32(7, 22));
        bytes.extend_from_slice(&encode_uint256(19, amendment));
        bytes.push(OBJECT_END);
        bytes.push(ARRAY_END);
        bytes
    }

    fn typed_amendments_payload(amendments: &[Uint256], majorities: &[(Uint256, u32)]) -> Vec<u8> {
        let mut entry = STLedgerEntry::new(amendments_keylet());
        entry.set_field_h256(
            get_field_by_symbol("sfPreviousTxnID"),
            Uint256::from_array([0xC1; 32]),
        );
        entry.set_field_u32(get_field_by_symbol("sfPreviousTxnLgrSeq"), 901);
        entry.set_field_v256(
            get_field_by_symbol("sfAmendments"),
            STVector256::from_values(get_field_by_symbol("sfAmendments"), amendments.to_vec()),
        );

        if !majorities.is_empty() {
            let mut array = STArray::new(get_field_by_symbol("sfMajorities"));
            for (amendment, close_time) in majorities {
                let mut object = STObject::new(get_field_by_symbol("sfMajority"));
                object.set_field_u32(get_field_by_symbol("sfCloseTime"), *close_time);
                object.set_field_h256(get_field_by_symbol("sfAmendment"), *amendment);
                array.push_back(object);
            }
            entry.set_field_array(get_field_by_symbol("sfMajorities"), array);
        }

        entry.get_serializer().data().to_vec()
    }

    fn typed_fee_settings_payload(
        legacy_fees: Option<Fees>,
        xrp_fees: Option<Fees>,
        previous: Option<(Uint256, u32)>,
    ) -> Vec<u8> {
        let mut entry = STLedgerEntry::new(fee_settings_keylet());

        if let Some(fees) = legacy_fees {
            entry.set_field_u32(
                get_field_by_symbol("sfReserveIncrement"),
                u32::try_from(fees.increment).expect("legacy fee increment must fit in u32"),
            );
            entry.set_field_u32(
                get_field_by_symbol("sfReserveBase"),
                u32::try_from(fees.reserve).expect("legacy reserve base must fit in u32"),
            );
            entry.set_field_u32(
                get_field_by_symbol("sfReferenceFeeUnits"),
                protocol::REFERENCE_FEE_UNITS_DEPRECATED,
            );
            entry.set_field_u64(get_field_by_symbol("sfBaseFee"), fees.base);
        }

        if let Some(fees) = xrp_fees {
            entry.set_field_amount(
                get_field_by_symbol("sfReserveIncrementDrops"),
                STAmount::new_native(fees.increment, false),
            );
            entry.set_field_amount(
                get_field_by_symbol("sfReserveBaseDrops"),
                STAmount::new_native(fees.reserve, false),
            );
            entry.set_field_amount(
                get_field_by_symbol("sfBaseFeeDrops"),
                STAmount::new_native(fees.base, false),
            );
        }

        if let Some((previous_txn_id, previous_txn_lgr_seq)) = previous {
            entry.set_field_h256(get_field_by_symbol("sfPreviousTxnID"), previous_txn_id);
            entry.set_field_u32(
                get_field_by_symbol("sfPreviousTxnLgrSeq"),
                previous_txn_lgr_seq,
            );
        }

        entry.get_serializer().data().to_vec()
    }

    #[test]
    fn decoder_reads_target_fields_and_skips_unneeded_ones() {
        let amendment = Uint256::from_array([0x11; 32]);
        let mut amendments_bytes = Vec::new();
        amendments_bytes.extend_from_slice(&encode_u16(1, LT_AMENDMENTS));
        amendments_bytes.extend_from_slice(&encode_vector256(3, &[amendment]));
        amendments_bytes.extend_from_slice(&encode_majorities_array());
        amendments_bytes.push(OBJECT_END);

        let amendments =
            decode_amendments_entry(&amendments_bytes, Uint256::from_array([0x22; 32]))
                .expect("amendments entry should decode");
        assert_eq!(amendments.amendments, vec![amendment]);

        let mut fee_bytes = Vec::new();
        fee_bytes.extend_from_slice(&encode_u16(1, LT_FEE_SETTINGS));
        fee_bytes.extend_from_slice(&encode_u64(5, 10));
        fee_bytes.extend_from_slice(&encode_u32(30, 256));
        fee_bytes.extend_from_slice(&encode_u32(31, 20));
        fee_bytes.extend_from_slice(&encode_native_amount(24, 30));
        fee_bytes.push(OBJECT_END);

        let fees = decode_fee_settings_fields(&fee_bytes).expect("fees entry should decode");
        assert_eq!(fees.base_fee, Some(10));
        assert_eq!(fees.reserve_base, Some(20));
        assert_eq!(
            fees.reserve_increment_drops,
            Some(AmountField {
                drops: 30,
                native: true,
                negative: false,
            })
        );
    }

    #[test]
    fn amendments_encoder_round_trips_through_decoder() {
        let amendments = vec![
            Uint256::from_array([0x31; 32]),
            Uint256::from_array([0x32; 32]),
        ];
        let payload = encode_amendments_entry(&amendments);
        let digest = Uint256::from_array([0x33; 32]);

        let decoded = decode_amendments_entry(&payload, digest)
            .expect("encoded amendments entry should decode");

        assert_eq!(decoded.digest, digest);
        assert_eq!(decoded.amendments, amendments);
    }

    #[test]
    fn amendments_decoder_matches_protocol_typed_constructor_shape() {
        let amendments = vec![
            Uint256::from_array([0x41; 32]),
            Uint256::from_array([0x42; 32]),
        ];
        let payload = encode_amendments_entry(&amendments);
        let digest = Uint256::from_array([0x43; 32]);

        let decoded = decode_amendments_entry(&payload, digest)
            .expect("typed amendments entry should decode through ledger adapter");

        assert_eq!(
            decoded.amendments,
            ConstructorAmendmentsEntry { amendments }.amendments
        );
    }

    #[test]
    fn typed_amendments_decoder_reads_sle_payload() {
        let digest = Uint256::from_array([0x51; 32]);
        let amendments = vec![
            Uint256::from_array([0x52; 32]),
            Uint256::from_array([0x53; 32]),
        ];
        let payload =
            typed_amendments_payload(&amendments, &[(Uint256::from_array([0x54; 32]), 777)]);

        let decoded = decode_amendments_entry_from_sle(&payload, digest)
            .expect("typed amendments payload should decode through sle adapter");

        assert_eq!(decoded.digest, digest);
        assert_eq!(decoded.amendments, amendments);
    }

    #[test]
    fn fee_settings_encoder_round_trips_legacy_fields() {
        let payload = encode_fee_settings_entry(
            Fees {
                base: 10,
                reserve: 20,
                increment: 30,
            },
            false,
        );

        let decoded = decode_fee_settings_fields(&payload)
            .expect("encoded legacy fee settings entry should decode");

        assert_eq!(decoded.base_fee, Some(10));
        assert_eq!(
            decoded.reference_fee_units,
            Some(protocol::REFERENCE_FEE_UNITS_DEPRECATED),
        );
        assert_eq!(decoded.reserve_base, Some(20));
        assert_eq!(decoded.reserve_increment, Some(30));
        assert_eq!(decoded.base_fee_drops, None);
        assert_eq!(decoded.reserve_base_drops, None);
        assert_eq!(decoded.reserve_increment_drops, None);
    }

    #[test]
    fn fee_settings_encoder_round_trips_xrp_fee_fields() {
        let payload = encode_fee_settings_entry(
            Fees {
                base: 11,
                reserve: 22,
                increment: 33,
            },
            true,
        );

        let decoded =
            decode_fee_settings_fields(&payload).expect("encoded xrp fee settings should decode");

        assert_eq!(decoded.base_fee, None);
        assert_eq!(decoded.reference_fee_units, None);
        assert_eq!(decoded.reserve_base, None);
        assert_eq!(decoded.reserve_increment, None);
        assert_eq!(
            decoded.base_fee_drops,
            Some(AmountField {
                drops: 11,
                native: true,
                negative: false,
            })
        );
        assert_eq!(
            decoded.reserve_base_drops,
            Some(AmountField {
                drops: 22,
                native: true,
                negative: false,
            })
        );
        assert_eq!(
            decoded.reserve_increment_drops,
            Some(AmountField {
                drops: 33,
                native: true,
                negative: false,
            })
        );
    }

    #[test]
    fn typed_fee_settings_decoder_reads_sle_payload_and_ignores_thread_fields() {
        let payload = typed_fee_settings_payload(
            None,
            Some(Fees {
                base: 41,
                reserve: 52,
                increment: 63,
            }),
            Some((Uint256::from_array([0x61; 32]), 880)),
        );

        let decoded = decode_fee_settings_fields_from_sle(&payload)
            .expect("typed fee settings payload should decode through sle adapter");

        assert_eq!(decoded.base_fee, None);
        assert_eq!(decoded.reference_fee_units, None);
        assert_eq!(decoded.reserve_base, None);
        assert_eq!(decoded.reserve_increment, None);
        assert_eq!(
            decoded.base_fee_drops,
            Some(AmountField {
                drops: 41,
                native: true,
                negative: false,
            })
        );
        assert_eq!(
            decoded.reserve_base_drops,
            Some(AmountField {
                drops: 52,
                native: true,
                negative: false,
            })
        );
        assert_eq!(
            decoded.reserve_increment_drops,
            Some(AmountField {
                drops: 63,
                native: true,
                negative: false,
            })
        );
        assert_eq!(
            decoded.previous_txn_id,
            Some(Uint256::from_array([0x61; 32]))
        );
        assert_eq!(decoded.previous_txn_lgr_seq, Some(880));
    }

    #[test]
    fn typed_fee_settings_decoder_accepts_noncanonical_field_insertion_order() {
        let payload = typed_fee_settings_payload(
            Some(Fees {
                base: 71,
                reserve: 82,
                increment: 93,
            }),
            None,
            None,
        );

        let decoded = decode_fee_settings_fields_from_sle(&payload)
            .expect("serializer should canonicalize field order for typed fee settings");

        assert_eq!(decoded.base_fee, Some(71));
        assert_eq!(
            decoded.reference_fee_units,
            Some(protocol::REFERENCE_FEE_UNITS_DEPRECATED),
        );
        assert_eq!(decoded.reserve_base, Some(82));
        assert_eq!(decoded.reserve_increment, Some(93));
    }
}
