#![allow(unused_imports, unused_variables, unused_mut, dead_code, unused_comparisons)]
//! Transaction builder helpers — mirrors C++ `test::jtx` transaction constructors.

use protocol::{
    AccountID, STAmount, STTx, TxType, XRPAmount, get_field_by_symbol,
};

use super::test_account::TestAccount;

/// Build a Payment transaction.
pub fn payment(from: &mut TestAccount, to: &TestAccount, amount: STAmount) -> STTx {
    let seq = from.next_seq();
    STTx::new(TxType::PAYMENT, |tx| {
        tx.set_account_id(get_field_by_symbol("sfAccount"), from.id);
        tx.set_account_id(get_field_by_symbol("sfDestination"), to.id);
        tx.set_field_amount(get_field_by_symbol("sfAmount"), amount);
        tx.set_field_amount(
            get_field_by_symbol("sfFee"),
            STAmount::from_xrp_amount(XRPAmount::from_drops(10)),
        );
        tx.set_field_u32(get_field_by_symbol("sfSequence"), seq);
    })
}

/// Build a TrustSet transaction.
pub fn trust_set(from: &mut TestAccount, limit: STAmount) -> STTx {
    let seq = from.next_seq();
    STTx::new(TxType::TRUST_SET, |tx| {
        tx.set_account_id(get_field_by_symbol("sfAccount"), from.id);
        tx.set_field_amount(get_field_by_symbol("sfLimitAmount"), limit);
        tx.set_field_amount(
            get_field_by_symbol("sfFee"),
            STAmount::from_xrp_amount(XRPAmount::from_drops(10)),
        );
        tx.set_field_u32(get_field_by_symbol("sfSequence"), seq);
    })
}

/// Build a CheckCreate transaction.
pub fn check_create(from: &mut TestAccount, to: &TestAccount, amount: STAmount) -> STTx {
    let seq = from.next_seq();
    STTx::new(TxType::CHECK_CREATE, |tx| {
        tx.set_account_id(get_field_by_symbol("sfAccount"), from.id);
        tx.set_account_id(get_field_by_symbol("sfDestination"), to.id);
        tx.set_field_amount(get_field_by_symbol("sfSendMax"), amount);
        tx.set_field_amount(
            get_field_by_symbol("sfFee"),
            STAmount::from_xrp_amount(XRPAmount::from_drops(10)),
        );
        tx.set_field_u32(get_field_by_symbol("sfSequence"), seq);
    })
}

/// Build a CheckCash transaction.
pub fn check_cash(from: &mut TestAccount, check_id: basics::base_uint::Uint256, amount: STAmount) -> STTx {
    let seq = from.next_seq();
    STTx::new(TxType::CHECK_CASH, |tx| {
        tx.set_account_id(get_field_by_symbol("sfAccount"), from.id);
        tx.set_field_h256(get_field_by_symbol("sfCheckID"), check_id);
        tx.set_field_amount(get_field_by_symbol("sfAmount"), amount);
        tx.set_field_amount(
            get_field_by_symbol("sfFee"),
            STAmount::from_xrp_amount(XRPAmount::from_drops(10)),
        );
        tx.set_field_u32(get_field_by_symbol("sfSequence"), seq);
    })
}

/// Build a CheckCancel transaction.
pub fn check_cancel(from: &mut TestAccount, check_id: basics::base_uint::Uint256) -> STTx {
    let seq = from.next_seq();
    STTx::new(TxType::CHECK_CANCEL, |tx| {
        tx.set_account_id(get_field_by_symbol("sfAccount"), from.id);
        tx.set_field_h256(get_field_by_symbol("sfCheckID"), check_id);
        tx.set_field_amount(
            get_field_by_symbol("sfFee"),
            STAmount::from_xrp_amount(XRPAmount::from_drops(10)),
        );
        tx.set_field_u32(get_field_by_symbol("sfSequence"), seq);
    })
}

/// Build an OfferCreate transaction.
pub fn offer_create(
    from: &mut TestAccount,
    taker_pays: STAmount,
    taker_gets: STAmount,
) -> STTx {
    let seq = from.next_seq();
    STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(get_field_by_symbol("sfAccount"), from.id);
        tx.set_field_amount(get_field_by_symbol("sfTakerPays"), taker_pays);
        tx.set_field_amount(get_field_by_symbol("sfTakerGets"), taker_gets);
        tx.set_field_amount(
            get_field_by_symbol("sfFee"),
            STAmount::from_xrp_amount(XRPAmount::from_drops(10)),
        );
        tx.set_field_u32(get_field_by_symbol("sfSequence"), seq);
    })
}

/// Build an OfferCancel transaction.
pub fn offer_cancel(from: &mut TestAccount, offer_seq: u32) -> STTx {
    let seq = from.next_seq();
    STTx::new(TxType::OFFER_CANCEL, |tx| {
        tx.set_account_id(get_field_by_symbol("sfAccount"), from.id);
        tx.set_field_u32(get_field_by_symbol("sfOfferSequence"), offer_seq);
        tx.set_field_amount(
            get_field_by_symbol("sfFee"),
            STAmount::from_xrp_amount(XRPAmount::from_drops(10)),
        );
        tx.set_field_u32(get_field_by_symbol("sfSequence"), seq);
    })
}

/// Build a SignerListSet transaction.
pub fn signer_list_set(from: &mut TestAccount, quorum: u32) -> STTx {
    let seq = from.next_seq();
    STTx::new(TxType::SIGNER_LIST_SET, |tx| {
        tx.set_account_id(get_field_by_symbol("sfAccount"), from.id);
        tx.set_field_u32(get_field_by_symbol("sfSignerQuorum"), quorum);
        tx.set_field_amount(
            get_field_by_symbol("sfFee"),
            STAmount::from_xrp_amount(XRPAmount::from_drops(10)),
        );
        tx.set_field_u32(get_field_by_symbol("sfSequence"), seq);
    })
}

/// Build an AccountDelete transaction.
pub fn account_delete(from: &mut TestAccount, to: &TestAccount) -> STTx {
    let seq = from.next_seq();
    STTx::new(TxType::ACCOUNT_DELETE, |tx| {
        tx.set_account_id(get_field_by_symbol("sfAccount"), from.id);
        tx.set_account_id(get_field_by_symbol("sfDestination"), to.id);
        tx.set_field_amount(
            get_field_by_symbol("sfFee"),
            STAmount::from_xrp_amount(XRPAmount::from_drops(2_000_000)),
        );
        tx.set_field_u32(get_field_by_symbol("sfSequence"), seq);
    })
}

/// Helper: XRP STAmount.
pub fn xrp_amount(drops: i64) -> STAmount {
    STAmount::from_xrp_amount(XRPAmount::from_drops(drops))
}
