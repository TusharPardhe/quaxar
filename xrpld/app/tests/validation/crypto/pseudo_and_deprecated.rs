//! Tests for deprecated and pseudo-transaction types.
//!
//! Pseudo-transactions (EnableAmendment, SetFee, UNLModify) are injected by
//! consensus and exist in ledger history. Deprecated types (NickNameSet, Contract)
//! existed in early XRPL history (2012-2013).
//!
//! The node must handle all of these during historical ledger sync.

use basics::base_uint::Uint256;
use protocol::{AccountID, STTx, SerialIter, TxType, get_field_by_symbol};

/// Test: Pseudo-transaction type codes are defined and accessible.
#[test]
fn pseudo_tx_type_codes_exist() {
    assert_eq!(TxType::AMENDMENT.to_u16(), 100);
    assert_eq!(TxType::from_u16(101).to_u16(), 101);
    assert_eq!(TxType::UNL_MODIFY.to_u16(), 102);
}

/// Test: Deprecated transaction type codes are defined.
#[test]
fn deprecated_tx_type_codes_exist() {
    assert_eq!(TxType::NICKNAME_SET.to_u16(), 6);
    assert_eq!(TxType::CONTRACT.to_u16(), 9);
}

/// Test: Construct and serialize an EnableAmendment pseudo-tx.
#[test]
fn enable_amendment_pseudo_tx_roundtrip() {
    let genesis = AccountID::default(); // All zeros = genesis account
    let amendment_hash = Uint256::from_u64(0xDEADBEEF_CAFEBABE);

    let tx = STTx::new(TxType::AMENDMENT, |tx| {
        tx.set_account_id(get_field_by_symbol("sfAccount"), genesis);
        tx.set_field_h256(get_field_by_symbol("sfAmendment"), amendment_hash);
        tx.set_field_u32(get_field_by_symbol("sfLedgerSequence"), 90_000_000);
    });

    assert_eq!(tx.get_txn_type(), TxType::AMENDMENT);

    // Serialize and re-deserialize
    let bytes = tx.get_serializer().data().to_vec();
    let mut iter = SerialIter::new(&bytes);
    let reparsed = STTx::from_serial_iter(&mut iter);

    assert_eq!(reparsed.get_txn_type(), TxType::AMENDMENT);
    assert_eq!(
        reparsed.get_field_h256(get_field_by_symbol("sfAmendment")),
        amendment_hash
    );
}

/// Test: Construct and serialize a SetFee pseudo-tx.
#[test]
fn set_fee_pseudo_tx_roundtrip() {
    let genesis = AccountID::default();

    let tx = STTx::new(TxType::from_u16(101), |tx| {
        tx.set_account_id(get_field_by_symbol("sfAccount"), genesis);
        tx.set_field_u64(get_field_by_symbol("sfBaseFeeDrops"), 10);
        tx.set_field_u32(get_field_by_symbol("sfReserveBaseDrops"), 10_000_000);
        tx.set_field_u32(get_field_by_symbol("sfReserveIncrementDrops"), 2_000_000);
        tx.set_field_u32(get_field_by_symbol("sfLedgerSequence"), 85_000_000);
    });

    assert_eq!(tx.get_txn_type(), TxType::from_u16(101));

    let bytes = tx.get_serializer().data().to_vec();
    let mut iter = SerialIter::new(&bytes);
    let reparsed = STTx::from_serial_iter(&mut iter);

    assert_eq!(reparsed.get_txn_type(), TxType::from_u16(101));
}

/// Test: Construct and serialize a UNLModify pseudo-tx.
#[test]
fn unl_modify_pseudo_tx_roundtrip() {
    let genesis = AccountID::default();

    let tx = STTx::new(TxType::UNL_MODIFY, |tx| {
        tx.set_account_id(get_field_by_symbol("sfAccount"), genesis);
        tx.set_field_u32(get_field_by_symbol("sfLedgerSequence"), 100_000_000);
        tx.set_field_u8(get_field_by_symbol("sfUNLModifyDisabling"), 1);
    });

    assert_eq!(tx.get_txn_type(), TxType::UNL_MODIFY);

    let bytes = tx.get_serializer().data().to_vec();
    let mut iter = SerialIter::new(&bytes);
    let reparsed = STTx::from_serial_iter(&mut iter);

    assert_eq!(reparsed.get_txn_type(), TxType::UNL_MODIFY);
}

/// Test: Deprecated NickNameSet type code exists but has no format template.
/// This is expected — the type existed in 2012-2013 but was never fully implemented.
#[test]
fn nickname_set_type_code_exists() {
    assert_eq!(TxType::NICKNAME_SET.to_u16(), 6);
    assert_eq!(TxType::CONTRACT.to_u16(), 9);
    // These types cannot be constructed via STTx::new because they have no
    // format template, but they can be parsed from raw bytes if encountered
    // in historical ledger data.
}

/// Test: All pseudo/deprecated type codes roundtrip through from_u16.
#[test]
fn all_special_type_codes_roundtrip_through_from_u16() {
    for (code, expected) in [
        (6, TxType::NICKNAME_SET),
        (9, TxType::CONTRACT),
        (100, TxType::AMENDMENT),
        (101, TxType::from_u16(101)),
        (102, TxType::UNL_MODIFY),
    ] {
        let parsed = TxType::from_u16(code);
        assert_eq!(parsed, expected, "Type code {code} mismatch");
        assert_eq!(parsed.to_u16(), code);
    }
}
