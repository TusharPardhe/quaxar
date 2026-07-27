use protocol::{
    AccountID, BatchTransactionFlags, INNER_BATCH_TRANSACTION_FLAG, Rules, STAmount, STArray,
    STObject, STTx, StBase, Ter, TxType, get_field_by_symbol,
};
use tx::{validate_sttx_batch_preflight, validate_sttx_batch_preflight_with_rules};

fn account(fill: u8) -> AccountID {
    AccountID::from_array([fill; 20])
}

fn inner_payment(account_id: AccountID, sequence: u32) -> STTx {
    STTx::new(TxType::PAYMENT, move |tx| {
        tx.set_account_id(get_field_by_symbol("sfAccount"), account_id);
        tx.set_account_id(get_field_by_symbol("sfDestination"), account(0xF0));
        tx.set_field_amount(
            get_field_by_symbol("sfAmount"),
            STAmount::new_native(1, false),
        );
        tx.set_field_amount(get_field_by_symbol("sfFee"), STAmount::new_native(0, false));
        tx.set_field_u32(get_field_by_symbol("sfSequence"), sequence);
        tx.set_field_u32(get_field_by_symbol("sfFlags"), INNER_BATCH_TRANSACTION_FLAG);
        tx.set_field_vl(get_field_by_symbol("sfSigningPubKey"), &[]);
    })
}

fn raw_transaction(tx: &STTx) -> STObject {
    let mut raw = tx.clone_as_object();
    raw.set_fname(get_field_by_symbol("sfRawTransaction"));
    raw
}

fn batch(outer: AccountID, inners: &[STTx]) -> STTx {
    let mut raw_transactions = STArray::new(get_field_by_symbol("sfRawTransactions"));
    for inner in inners {
        raw_transactions.push_back(raw_transaction(inner));
    }

    STTx::new(TxType::BATCH, move |tx| {
        tx.set_account_id(get_field_by_symbol("sfAccount"), outer);
        tx.set_field_amount(
            get_field_by_symbol("sfFee"),
            STAmount::new_native(10, false),
        );
        tx.set_field_u32(get_field_by_symbol("sfSequence"), 1);
        tx.set_field_u32(
            get_field_by_symbol("sfFlags"),
            BatchTransactionFlags::ALL_OR_NOTHING.bits(),
        );
        tx.set_field_array(get_field_by_symbol("sfRawTransactions"), raw_transactions);
    })
}

fn set_batch_signers(batch: &mut STTx, accounts: &[AccountID]) {
    let mut signers = STArray::new(get_field_by_symbol("sfBatchSigners"));
    for account_id in accounts {
        let mut signer = STObject::make_inner_object(get_field_by_symbol("sfBatchSigner"));
        signer.set_account_id(get_field_by_symbol("sfAccount"), *account_id);
        signers.push_back(signer);
    }
    batch.set_field_array(get_field_by_symbol("sfBatchSigners"), signers);
}

#[test]
fn tx_sttx_batch_preflight_uses_actual_delegate_for_required_signer() {
    let outer = account(0x10);
    let authorizing_account = account(0x20);
    let delegate = account(0x30);
    let mut delegated = inner_payment(authorizing_account, 1);
    delegated.set_account_id(get_field_by_symbol("sfDelegate"), delegate);
    let outer_inner = inner_payment(outer, 2);

    let mut batch = batch(outer, &[delegated, outer_inner]);
    set_batch_signers(&mut batch, &[authorizing_account]);
    assert_eq!(validate_sttx_batch_preflight(&batch), Ter::TEM_BAD_SIGNER);

    set_batch_signers(&mut batch, &[delegate]);
    assert_eq!(validate_sttx_batch_preflight(&batch), Ter::TES_SUCCESS);
}

#[test]
fn tx_sttx_batch_preflight_requires_actual_sponsor_only_with_sponsor_signature() {
    let outer = account(0x10);
    let authorizing_account = account(0x20);
    let sponsor = account(0x30);
    let mut sponsored = inner_payment(authorizing_account, 1);
    sponsored.set_account_id(get_field_by_symbol("sfSponsor"), sponsor);
    let outer_inner = inner_payment(outer, 2);

    let mut without_signature = batch(outer, &[sponsored.clone(), outer_inner.clone()]);
    set_batch_signers(&mut without_signature, &[authorizing_account]);
    assert_eq!(
        validate_sttx_batch_preflight(&without_signature),
        Ter::TES_SUCCESS
    );

    sponsored
        .peek_field_object(get_field_by_symbol("sfSponsorSignature"))
        .set_field_vl(get_field_by_symbol("sfSigningPubKey"), &[]);
    let mut with_signature = batch(outer, &[sponsored, outer_inner]);
    set_batch_signers(&mut with_signature, &[authorizing_account]);
    assert_eq!(
        validate_sttx_batch_preflight(&with_signature),
        Ter::TEM_BAD_SIGNER
    );

    set_batch_signers(&mut with_signature, &[authorizing_account, sponsor]);
    assert_eq!(
        validate_sttx_batch_preflight(&with_signature),
        Ter::TES_SUCCESS
    );
}

#[test]
fn tx_sttx_batch_preflight_applies_rule_aware_inner_delegate_validation() {
    let outer = account(0x10);
    let authorizing_account = account(0x20);
    let delegate = account(0x30);
    let mut delegated = inner_payment(authorizing_account, 1);
    delegated.set_account_id(get_field_by_symbol("sfDelegate"), delegate);
    let outer_inner = inner_payment(outer, 2);

    let mut batch_tx = batch(outer, &[delegated.clone(), outer_inner.clone()]);
    set_batch_signers(&mut batch_tx, &[delegate]);
    assert_eq!(
        validate_sttx_batch_preflight_with_rules(&batch_tx, &Rules::default()),
        Ter::TEM_INVALID_INNER_BATCH
    );

    let delegation_rules = Rules::new([protocol::feature_id("PermissionDelegationV1_1")]);
    assert_eq!(
        validate_sttx_batch_preflight_with_rules(&batch_tx, &delegation_rules),
        Ter::TES_SUCCESS
    );

    delegated.set_account_id(get_field_by_symbol("sfDelegate"), authorizing_account);
    let mut self_delegated = batch(outer, &[delegated, outer_inner]);
    set_batch_signers(&mut self_delegated, &[authorizing_account]);
    assert_eq!(
        validate_sttx_batch_preflight_with_rules(&self_delegated, &delegation_rules),
        Ter::TEM_INVALID_INNER_BATCH
    );
}

#[test]
fn tx_sttx_batch_preflight_rejects_typed_account_delete_self_destination() {
    let outer = account(0x10);
    let malformed_account_delete = STTx::new(TxType::ACCOUNT_DELETE, |tx| {
        tx.set_account_id(get_field_by_symbol("sfAccount"), outer);
        tx.set_account_id(get_field_by_symbol("sfDestination"), outer);
        tx.set_field_amount(get_field_by_symbol("sfFee"), STAmount::new_native(0, false));
        tx.set_field_u32(get_field_by_symbol("sfSequence"), 1);
        tx.set_field_u32(get_field_by_symbol("sfFlags"), INNER_BATCH_TRANSACTION_FLAG);
        tx.set_field_vl(get_field_by_symbol("sfSigningPubKey"), &[]);
    });

    let batch_tx = batch(outer, &[malformed_account_delete, inner_payment(outer, 2)]);
    assert_eq!(
        validate_sttx_batch_preflight_with_rules(&batch_tx, &Rules::default()),
        Ter::TEM_INVALID_INNER_BATCH
    );
}

#[test]
fn tx_sttx_batch_preflight_rejects_typed_ticket_create_malformed_inner() {
    let outer = account(0x10);
    let malformed_ticket_create = STTx::new(TxType::TICKET_CREATE, |tx| {
        tx.set_account_id(get_field_by_symbol("sfAccount"), outer);
        tx.set_field_u32(get_field_by_symbol("sfTicketCount"), 0);
        tx.set_field_amount(get_field_by_symbol("sfFee"), STAmount::new_native(0, false));
        tx.set_field_u32(get_field_by_symbol("sfSequence"), 1);
        tx.set_field_u32(get_field_by_symbol("sfFlags"), INNER_BATCH_TRANSACTION_FLAG);
        tx.set_field_vl(get_field_by_symbol("sfSigningPubKey"), &[]);
    });

    let batch_tx = batch(outer, &[malformed_ticket_create, inner_payment(outer, 2)]);
    assert_eq!(
        validate_sttx_batch_preflight_with_rules(&batch_tx, &Rules::default()),
        Ter::TEM_INVALID_INNER_BATCH
    );
}

#[test]
fn tx_sttx_batch_preflight_rejects_actual_fee_sponsored_inner_transaction() {
    let outer = account(0x10);
    let mut fee_sponsored = inner_payment(account(0x20), 1);
    fee_sponsored.set_account_id(get_field_by_symbol("sfSponsor"), account(0x30));
    fee_sponsored.set_field_u32(get_field_by_symbol("sfSponsorFlags"), 1);
    let batch = batch(outer, &[fee_sponsored, inner_payment(outer, 2)]);

    assert_eq!(validate_sttx_batch_preflight(&batch), Ter::TEM_INVALID_FLAG);
}

#[test]
fn tx_sttx_batch_preflight_rejects_actual_outer_reserve_sponsorship() {
    let outer = account(0x10);
    let mut batch = batch(
        outer,
        &[inner_payment(account(0x20), 1), inner_payment(outer, 2)],
    );
    batch.set_field_u32(get_field_by_symbol("sfSponsorFlags"), 2);

    assert_eq!(validate_sttx_batch_preflight(&batch), Ter::TEM_INVALID_FLAG);
}

#[test]
fn tx_sttx_batch_preflight_rejects_actual_unknown_outer_flag() {
    let outer = account(0x10);
    let mut batch = batch(
        outer,
        &[inner_payment(account(0x20), 1), inner_payment(outer, 2)],
    );
    batch.set_field_u32(
        get_field_by_symbol("sfFlags"),
        BatchTransactionFlags::ALL_OR_NOTHING.bits() | 0x0000_0001,
    );

    assert_eq!(validate_sttx_batch_preflight(&batch), Ter::TEM_INVALID_FLAG);
}

#[test]
fn tx_sttx_batch_preflight_rejects_typed_check_create_self_destination() {
    let outer = account(0x10);
    let malformed_check_create = STTx::new(TxType::CHECK_CREATE, |tx| {
        tx.set_account_id(get_field_by_symbol("sfAccount"), outer);
        tx.set_account_id(get_field_by_symbol("sfDestination"), outer);
        tx.set_field_amount(
            get_field_by_symbol("sfSendMax"),
            STAmount::new_native(1, false),
        );
        tx.set_field_amount(get_field_by_symbol("sfFee"), STAmount::new_native(0, false));
        tx.set_field_u32(get_field_by_symbol("sfSequence"), 1);
        tx.set_field_u32(get_field_by_symbol("sfFlags"), INNER_BATCH_TRANSACTION_FLAG);
        tx.set_field_vl(get_field_by_symbol("sfSigningPubKey"), &[]);
    });

    let batch_tx = batch(outer, &[malformed_check_create, inner_payment(outer, 2)]);
    assert_eq!(
        validate_sttx_batch_preflight_with_rules(&batch_tx, &Rules::default()),
        Ter::TEM_INVALID_INNER_BATCH
    );
}

#[test]
fn tx_sttx_batch_preflight_rejects_typed_paychan_create_invalid_public_key() {
    let outer = account(0x10);
    let malformed_paychan_create = STTx::new(TxType::PAYCHAN_CREATE, |tx| {
        tx.set_account_id(get_field_by_symbol("sfAccount"), outer);
        tx.set_account_id(get_field_by_symbol("sfDestination"), account(0x20));
        tx.set_field_amount(
            get_field_by_symbol("sfAmount"),
            STAmount::new_native(1, false),
        );
        tx.set_field_vl(get_field_by_symbol("sfPublicKey"), &[0]);
        tx.set_field_amount(get_field_by_symbol("sfFee"), STAmount::new_native(0, false));
        tx.set_field_u32(get_field_by_symbol("sfSequence"), 1);
        tx.set_field_u32(get_field_by_symbol("sfFlags"), INNER_BATCH_TRANSACTION_FLAG);
        tx.set_field_vl(get_field_by_symbol("sfSigningPubKey"), &[]);
    });

    let batch_tx = batch(outer, &[malformed_paychan_create, inner_payment(outer, 2)]);
    assert_eq!(
        validate_sttx_batch_preflight_with_rules(&batch_tx, &Rules::default()),
        Ter::TEM_INVALID_INNER_BATCH
    );
}

#[test]
fn tx_sttx_batch_preflight_rejects_typed_escrow_create_without_expiration() {
    let outer = account(0x10);
    let malformed_escrow_create = STTx::new(TxType::ESCROW_CREATE, |tx| {
        tx.set_account_id(get_field_by_symbol("sfAccount"), outer);
        tx.set_account_id(get_field_by_symbol("sfDestination"), account(0x20));
        tx.set_field_amount(
            get_field_by_symbol("sfAmount"),
            STAmount::new_native(1, false),
        );
        tx.set_field_amount(get_field_by_symbol("sfFee"), STAmount::new_native(0, false));
        tx.set_field_u32(get_field_by_symbol("sfSequence"), 1);
        tx.set_field_u32(get_field_by_symbol("sfFlags"), INNER_BATCH_TRANSACTION_FLAG);
        tx.set_field_vl(get_field_by_symbol("sfSigningPubKey"), &[]);
    });

    let batch_tx = batch(outer, &[malformed_escrow_create, inner_payment(outer, 2)]);
    assert_eq!(
        validate_sttx_batch_preflight_with_rules(&batch_tx, &Rules::default()),
        Ter::TEM_INVALID_INNER_BATCH
    );
}
