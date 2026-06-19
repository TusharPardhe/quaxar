//! Genesis-construction helpers for the current `Ledger` surface.
//!
//! This ports the currently written genesis constructor objects: the fixed
//! master-account `AccountRoot`, optional `Amendments`, and `FeeSettings`
//! singleton, while reusing the protocol-layer account-ID derivation,
//! keylet/index helpers, and serialized constructor-entry helpers.

use basics::base_uint::{Uint160, Uint256};
use protocol::{
    ConstructorAccountRootEntry, ConstructorLedgerEntry, build_genesis_setup_constructor_entries,
    constructor_ledger_entry_key, constructor_ledger_item, genesis_account_id,
};

pub fn genesis_master_account_id() -> Uint160 {
    genesis_account_id()
}

pub fn genesis_master_account_key() -> Uint256 {
    constructor_ledger_entry_key(&ConstructorLedgerEntry::AccountRoot(
        genesis_master_account_root_entry(0),
    ))
}

fn genesis_master_account_root_entry(total_drops: u64) -> ConstructorAccountRootEntry {
    ConstructorAccountRootEntry {
        sequence: 1,
        balance_drops: total_drops,
        account_id: genesis_master_account_id(),
    }
}

pub fn build_genesis_master_account_root_item(total_drops: u64) -> (Uint256, Vec<u8>) {
    constructor_ledger_item(&ConstructorLedgerEntry::AccountRoot(
        genesis_master_account_root_entry(total_drops),
    ))
}

pub fn build_genesis_amendments_item(amendments: &[Uint256]) -> Option<(Uint256, Vec<u8>)> {
    build_genesis_setup_constructor_entries(0, 0, 0, amendments)
        .into_iter()
        .find(|entry| matches!(entry, ConstructorLedgerEntry::Amendments(_)))
        .map(|entry| constructor_ledger_item(&entry))
}

pub fn build_genesis_fees_item(
    base_drops: u64,
    reserve_drops: u64,
    increment_drops: u64,
    amendments: &[Uint256],
) -> (Uint256, Vec<u8>) {
    constructor_ledger_item(
        build_genesis_setup_constructor_entries(
            base_drops,
            reserve_drops,
            increment_drops,
            amendments,
        )
        .iter()
        .find(|entry| matches!(entry, ConstructorLedgerEntry::FeeSettings(_)))
        .expect("genesis setup constructor entries must always contain fees"),
    )
}

pub fn build_genesis_setup_items(
    base_drops: u64,
    reserve_drops: u64,
    increment_drops: u64,
    amendments: &[Uint256],
) -> Vec<(Uint256, Vec<u8>)> {
    let mut items = Vec::with_capacity(1 + usize::from(!amendments.is_empty()));
    if let Some(item) = build_genesis_amendments_item(amendments) {
        items.push(item);
    }
    items.push(build_genesis_fees_item(
        base_drops,
        reserve_drops,
        increment_drops,
        amendments,
    ));
    items
}

#[cfg(test)]
mod tests {
    use super::{
        build_genesis_amendments_item, build_genesis_fees_item,
        build_genesis_master_account_root_item, genesis_master_account_id,
        genesis_master_account_key, genesis_master_account_root_entry,
    };
    use basics::base_uint::{Uint160, Uint256};
    use protocol::{
        ConstructorAccountRootEntry, ConstructorFeeSettingsEntry, ConstructorLedgerEntry,
        amendments_key, constructor_ledger_entry_key, decode_constructor_account_root_entry,
        decode_constructor_amendments_entry, decode_constructor_fee_settings_entry,
        encode_constructor_account_root_entry, feature_xrp_fees, fees_key,
    };

    fn bytes_to_hex(bytes: &[u8]) -> String {
        bytes.iter().map(|byte| format!("{byte:02X}")).collect()
    }

    #[test]
    fn master_account_constants_match_current_cpp_genesis_identity() {
        assert_eq!(
            genesis_master_account_id(),
            Uint160::from_hex("B5F762798A53D543A014CAF8B297CFF8F2F937E8")
                .expect("expected master account id should parse")
        );
        assert_eq!(
            genesis_master_account_key(),
            Uint256::from_hex("2B6AC232AA4C4BE41BF49D2459FA4A0347E1B543A4C92FCEE0821C0201E2E9A8")
                .expect("expected master account key should parse")
        );
        assert_eq!(
            genesis_master_account_key(),
            constructor_ledger_entry_key(&ConstructorLedgerEntry::AccountRoot(
                genesis_master_account_root_entry(0)
            ))
        );
    }

    #[test]
    fn master_account_root_item_matches_current_ctor_field_shape() {
        let (key, payload) = build_genesis_master_account_root_item(100_000_000_000_000_000);

        assert_eq!(
            key,
            Uint256::from_hex("2B6AC232AA4C4BE41BF49D2459FA4A0347E1B543A4C92FCEE0821C0201E2E9A8")
                .expect("expected master account key should parse")
        );
        assert_eq!(
            bytes_to_hex(&payload),
            "1100612200000000240000000125000000002D0000000055000000000000000000000000000000000000000000000000000000000000000062416345785D8A00008114B5F762798A53D543A014CAF8B297CFF8F2F937E8"
        );
    }

    #[test]
    fn master_account_root_entry_matches_current_cpp_ctor_fields() {
        let entry = genesis_master_account_root_entry(100_000_000_000_000_000);

        assert_eq!(
            entry,
            ConstructorAccountRootEntry {
                sequence: 1,
                balance_drops: 100_000_000_000_000_000,
                account_id: Uint160::from_hex("B5F762798A53D543A014CAF8B297CFF8F2F937E8")
                    .expect("expected master account id should parse"),
            }
        );
        assert_eq!(
            decode_constructor_account_root_entry(&encode_constructor_account_root_entry(entry))
                .expect("genesis master account entry should round-trip"),
            entry
        );
    }

    #[test]
    fn genesis_amendments_item_round_trips_through_constructor_decoder() {
        let amendments = [Uint256::from_u64(1), Uint256::from_u64(2)];
        let (key, payload) = build_genesis_amendments_item(&amendments)
            .expect("non-empty amendments should emit item");

        assert_eq!(key, amendments_key());
        assert_eq!(
            decode_constructor_amendments_entry(&payload)
                .expect("genesis amendments item should round-trip")
                .amendments,
            amendments
        );
    }

    #[test]
    fn genesis_amendments_item_skips_empty_list() {
        assert_eq!(build_genesis_amendments_item(&[]), None);
    }

    #[test]
    fn genesis_fees_item_round_trips_legacy_shape_without_xrp_fees() {
        let (key, payload) = build_genesis_fees_item(10, 20, 30, &[]);

        assert_eq!(key, fees_key());
        assert_eq!(
            decode_constructor_fee_settings_entry(&payload)
                .expect("legacy genesis fees item should round-trip"),
            ConstructorFeeSettingsEntry::Legacy {
                base_fee: 10,
                reference_fee_units: protocol::REFERENCE_FEE_UNITS_DEPRECATED,
                reserve_base: Some(20),
                reserve_increment: Some(30),
            }
        );
    }

    #[test]
    fn genesis_fees_item_round_trips_xrp_shape_with_amendment() {
        let (key, payload) = build_genesis_fees_item(11, 22, 33, &[feature_xrp_fees()]);

        assert_eq!(key, fees_key());
        assert_eq!(
            decode_constructor_fee_settings_entry(&payload)
                .expect("xrp genesis fees item should round-trip"),
            ConstructorFeeSettingsEntry::XrpDrops {
                base_fee_drops: 11,
                reserve_base_drops: 22,
                reserve_increment_drops: 33,
            }
        );
    }
}
