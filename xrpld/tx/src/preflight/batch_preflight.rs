//! Current deterministic validation halves of
//! `xrpl/tx/transactors/system/the reference source`.
//!
//! This ports the deterministic validation rules around:
//!
//! - outer batch mode flag selection,
//! - raw inner-transaction count bounds,
//! - duplicate inner transaction rejection,
//! - disabled or nested inner transaction type rejection,
//! - required `tfInnerBatchTxn` handling,
//! - signature-field rejection for inner and optional counterparty signatures,
//! - zero native-fee enforcement,
//! - exact-one-of `Sequence` / `TicketSequence`,
//! - duplicate sequence/ticket rejection in the ordered batch modes,
//! - required signer-set construction for `preflightSigValidated(...)`,
//! - batch-signer uniqueness and outer-account rejection,
//! - required-signer coverage checks before `checkBatchSign(...)`,
//! - final invalid-signature mapping when the batch-signature
//!   check fails.

use std::collections::{HashMap, HashSet};
use std::hash::Hash;

use protocol::{
    BatchTransactionFlags, INNER_BATCH_TRANSACTION_FLAG, NotTec, Ter, TxType, is_tes_success,
};

pub const MAX_BATCH_TX_COUNT: usize = 8;

pub const DISABLED_INNER_BATCH_TX_TYPES: [TxType; 15] = [
    TxType::VAULT_CREATE,
    TxType::VAULT_SET,
    TxType::VAULT_DELETE,
    TxType::VAULT_DEPOSIT,
    TxType::VAULT_WITHDRAW,
    TxType::VAULT_CLAWBACK,
    TxType::LOAN_BROKER_SET,
    TxType::LOAN_BROKER_DELETE,
    TxType::LOAN_BROKER_COVER_DEPOSIT,
    TxType::LOAN_BROKER_COVER_WITHDRAW,
    TxType::LOAN_BROKER_COVER_CLAWBACK,
    TxType::LOAN_SET,
    TxType::LOAN_DELETE,
    TxType::LOAN_MANAGE,
    TxType::LOAN_PAY,
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct BatchSignatureFacts {
    pub has_txn_signature: bool,
    pub has_signers: bool,
    pub signing_pub_key_is_empty: bool,
}

pub trait BatchInnerTransaction {
    type TxId: Clone + Eq + Hash;
    type Account: Clone + Eq + Hash;

    fn transaction_id(&self) -> Self::TxId;
    fn txn_type(&self) -> TxType;
    fn flags(&self) -> u32;
    fn signature_facts(&self) -> BatchSignatureFacts;
    fn counterparty_signature_facts(&self) -> Option<BatchSignatureFacts>;
    fn fee_is_native_zero(&self) -> bool;
    fn account(&self) -> Self::Account;
    fn counterparty(&self) -> Option<Self::Account>;
    fn sequence(&self) -> u32;
    fn ticket_sequence(&self) -> Option<u32>;
}

pub trait BatchSignerEntry {
    type Account: Clone + Eq + Hash;

    fn account(&self) -> Self::Account;
}

pub fn validate_batch_preflight_structure<InnerTx, InnerPreflight>(
    batch_flags: u32,
    inner_transactions: impl IntoIterator<Item = InnerTx>,
    mut preflight_inner_transaction: InnerPreflight,
) -> NotTec
where
    InnerTx: BatchInnerTransaction,
    InnerPreflight: FnMut(&InnerTx) -> NotTec,
{
    let batch_mode = BatchTransactionFlags::from_bits(batch_flags);
    if batch_mode.bits().count_ones() != 1 {
        return Ter::TEM_INVALID_FLAG;
    }

    let inner_transactions: Vec<_> = inner_transactions.into_iter().collect();
    if inner_transactions.len() <= 1 {
        return Ter::TEM_ARRAY_EMPTY;
    }
    if inner_transactions.len() > MAX_BATCH_TX_COUNT {
        return Ter::TEM_ARRAY_TOO_LARGE;
    }

    let enforce_unique_seq_and_ticket = batch_mode.contains(BatchTransactionFlags::ALL_OR_NOTHING)
        || batch_mode.contains(BatchTransactionFlags::UNTIL_FAILURE);

    let mut unique_hashes = HashSet::new();
    let mut account_seq_ticket: HashMap<InnerTx::Account, HashSet<u32>> = HashMap::new();

    for inner_transaction in &inner_transactions {
        let tx_id = inner_transaction.transaction_id();
        if !unique_hashes.insert(tx_id) {
            return Ter::TEM_REDUNDANT;
        }

        let txn_type = inner_transaction.txn_type();
        if txn_type == TxType::BATCH {
            return Ter::TEM_INVALID;
        }
        if DISABLED_INNER_BATCH_TX_TYPES.contains(&txn_type) {
            return Ter::TEM_INVALID_INNER_BATCH;
        }

        if (inner_transaction.flags() & INNER_BATCH_TRANSACTION_FLAG) == 0 {
            return Ter::TEM_INVALID_FLAG;
        }

        if let Some(error) = validate_signature_facts(inner_transaction.signature_facts()) {
            return error;
        }
        if let Some(counterparty_signature) = inner_transaction.counterparty_signature_facts()
            && let Some(error) = validate_signature_facts(counterparty_signature)
        {
            return error;
        }

        if !inner_transaction.fee_is_native_zero() {
            return Ter::TEM_BAD_FEE;
        }

        if !is_tes_success(preflight_inner_transaction(inner_transaction)) {
            return Ter::TEM_INVALID_INNER_BATCH;
        }

        let sequence = inner_transaction.sequence();
        let ticket_sequence = inner_transaction.ticket_sequence();
        let has_sequence = sequence != 0;
        let has_ticket_sequence = ticket_sequence.is_some();

        if has_sequence == has_ticket_sequence {
            return Ter::TEM_SEQ_AND_TICKET;
        }

        if enforce_unique_seq_and_ticket {
            let account_entries = account_seq_ticket
                .entry(inner_transaction.account())
                .or_default();

            if has_sequence && !account_entries.insert(sequence) {
                return Ter::TEM_REDUNDANT;
            }
            if let Some(ticket_sequence) = ticket_sequence
                && !account_entries.insert(ticket_sequence)
            {
                return Ter::TEM_REDUNDANT;
            }
        }
    }

    Ter::TES_SUCCESS
}

pub fn validate_batch_preflight_sig_validated<InnerTx, Signer, CheckBatchSign>(
    outer_account: InnerTx::Account,
    inner_transactions: impl IntoIterator<Item = InnerTx>,
    batch_signers: Option<impl IntoIterator<Item = Signer>>,
    check_batch_sign: CheckBatchSign,
) -> NotTec
where
    InnerTx: BatchInnerTransaction,
    Signer: BatchSignerEntry<Account = InnerTx::Account>,
    CheckBatchSign: FnOnce() -> bool,
{
    let mut required_signers = HashSet::new();

    for inner_transaction in inner_transactions {
        let inner_account = inner_transaction.account();
        if inner_account != outer_account {
            required_signers.insert(inner_account);
        }

        if let Some(counterparty) = inner_transaction.counterparty()
            && counterparty != outer_account
        {
            required_signers.insert(counterparty);
        }
    }

    if let Some(batch_signers) = batch_signers {
        let batch_signers: Vec<_> = batch_signers.into_iter().collect();
        if batch_signers.len() > MAX_BATCH_TX_COUNT {
            return Ter::TEM_ARRAY_TOO_LARGE;
        }

        let mut unique_batch_signers = HashSet::new();
        for signer in batch_signers {
            let signer_account = signer.account();
            if signer_account == outer_account {
                return Ter::TEM_BAD_SIGNER;
            }
            if !unique_batch_signers.insert(signer_account.clone()) {
                return Ter::TEM_REDUNDANT;
            }
            if !required_signers.remove(&signer_account) {
                return Ter::TEM_BAD_SIGNER;
            }
        }

        if !check_batch_sign() {
            return Ter::TEM_BAD_SIGNATURE;
        }
    }

    if !required_signers.is_empty() {
        return Ter::TEM_BAD_SIGNER;
    }

    Ter::TES_SUCCESS
}

fn validate_signature_facts(signature_facts: BatchSignatureFacts) -> Option<NotTec> {
    if signature_facts.has_txn_signature {
        return Some(Ter::TEM_BAD_SIGNATURE);
    }
    if signature_facts.has_signers {
        return Some(Ter::TEM_BAD_SIGNER);
    }
    if !signature_facts.signing_pub_key_is_empty {
        return Some(Ter::TEM_BAD_REGKEY);
    }

    None
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use protocol::{BatchTransactionFlags, INNER_BATCH_TRANSACTION_FLAG};

    use super::{
        BatchInnerTransaction, BatchSignatureFacts, BatchSignerEntry,
        DISABLED_INNER_BATCH_TX_TYPES, MAX_BATCH_TX_COUNT, validate_batch_preflight_sig_validated,
        validate_batch_preflight_structure,
    };

    #[derive(Clone)]
    struct StubInnerTx {
        tx_id: &'static str,
        txn_type: protocol::TxType,
        flags: u32,
        signature_facts: BatchSignatureFacts,
        counterparty_signature_facts: Option<BatchSignatureFacts>,
        fee_is_native_zero: bool,
        account: &'static str,
        counterparty: Option<&'static str>,
        sequence: u32,
        ticket_sequence: Option<u32>,
    }

    impl StubInnerTx {
        fn new(tx_id: &'static str, account: &'static str) -> Self {
            Self {
                tx_id,
                txn_type: protocol::TxType::PAYMENT,
                flags: INNER_BATCH_TRANSACTION_FLAG,
                signature_facts: BatchSignatureFacts {
                    signing_pub_key_is_empty: true,
                    ..BatchSignatureFacts::default()
                },
                counterparty_signature_facts: None,
                fee_is_native_zero: true,
                account,
                counterparty: None,
                sequence: 1,
                ticket_sequence: None,
            }
        }
    }

    impl BatchInnerTransaction for StubInnerTx {
        type TxId = &'static str;
        type Account = &'static str;

        fn transaction_id(&self) -> Self::TxId {
            self.tx_id
        }

        fn txn_type(&self) -> protocol::TxType {
            self.txn_type
        }

        fn flags(&self) -> u32 {
            self.flags
        }

        fn signature_facts(&self) -> BatchSignatureFacts {
            self.signature_facts
        }

        fn counterparty_signature_facts(&self) -> Option<BatchSignatureFacts> {
            self.counterparty_signature_facts
        }

        fn fee_is_native_zero(&self) -> bool {
            self.fee_is_native_zero
        }

        fn account(&self) -> Self::Account {
            self.account
        }

        fn counterparty(&self) -> Option<Self::Account> {
            self.counterparty
        }

        fn sequence(&self) -> u32 {
            self.sequence
        }

        fn ticket_sequence(&self) -> Option<u32> {
            self.ticket_sequence
        }
    }

    #[derive(Clone)]
    struct StubBatchSigner {
        account: &'static str,
    }

    impl BatchSignerEntry for StubBatchSigner {
        type Account = &'static str;

        fn account(&self) -> Self::Account {
            self.account
        }
    }

    #[test]
    fn batch_preflight_constants_match_cpp_batch_h() {
        assert_eq!(MAX_BATCH_TX_COUNT, 8);
        assert_eq!(DISABLED_INNER_BATCH_TX_TYPES.len(), 15);
        assert!(DISABLED_INNER_BATCH_TX_TYPES.contains(&protocol::TxType::VAULT_CREATE));
        assert!(DISABLED_INNER_BATCH_TX_TYPES.contains(&protocol::TxType::LOAN_PAY));
    }

    #[test]
    fn batch_preflight_rejects_counterparty_signature_fields() {
        let mut first = StubInnerTx::new("tx-1", "alice");
        first.counterparty_signature_facts = Some(BatchSignatureFacts {
            has_signers: true,
            signing_pub_key_is_empty: true,
            ..BatchSignatureFacts::default()
        });

        let result = validate_batch_preflight_structure(
            BatchTransactionFlags::ALL_OR_NOTHING.bits(),
            [first, StubInnerTx::new("tx-2", "bob")],
            |_| protocol::Ter::TES_SUCCESS,
        );

        assert_eq!(result, protocol::Ter::TEM_BAD_SIGNER);
    }

    #[test]
    fn batch_preflight_rejects_missing_sequence_and_ticket() {
        let mut first = StubInnerTx::new("tx-1", "alice");
        first.sequence = 0;

        let result = validate_batch_preflight_structure(
            BatchTransactionFlags::ALL_OR_NOTHING.bits(),
            [first, StubInnerTx::new("tx-2", "bob")],
            |_| protocol::Ter::TES_SUCCESS,
        );

        assert_eq!(result, protocol::Ter::TEM_SEQ_AND_TICKET);
    }

    #[test]
    fn batch_preflight_allows_duplicate_sequence_in_independent_mode() {
        let first = StubInnerTx::new("tx-1", "alice");
        let mut second = StubInnerTx::new("tx-2", "alice");
        second.sequence = 1;

        let result = validate_batch_preflight_structure(
            BatchTransactionFlags::INDEPENDENT.bits(),
            [first, second],
            |_| protocol::Ter::TES_SUCCESS,
        );

        assert_eq!(result, protocol::Ter::TES_SUCCESS);
    }

    #[test]
    fn batch_preflight_sig_validated_rejects_outer_account_as_batch_signer() {
        let result = validate_batch_preflight_sig_validated(
            "outer",
            [
                StubInnerTx::new("tx-1", "alice"),
                StubInnerTx::new("tx-2", "outer"),
            ],
            Some([StubBatchSigner { account: "outer" }]),
            || true,
        );

        assert_eq!(result, protocol::Ter::TEM_BAD_SIGNER);
    }

    #[test]
    fn batch_preflight_sig_validated_rejects_unneeded_signer() {
        let result = validate_batch_preflight_sig_validated(
            "outer",
            [
                StubInnerTx::new("tx-1", "alice"),
                StubInnerTx::new("tx-2", "outer"),
            ],
            Some([StubBatchSigner { account: "bob" }]),
            || true,
        );

        assert_eq!(result, protocol::Ter::TEM_BAD_SIGNER);
    }

    #[test]
    fn batch_preflight_sig_validated_maps_failed_batch_sign_check() {
        let result = validate_batch_preflight_sig_validated(
            "outer",
            [
                StubInnerTx::new("tx-1", "alice"),
                StubInnerTx::new("tx-2", "outer"),
            ],
            Some([StubBatchSigner { account: "alice" }]),
            || false,
        );

        assert_eq!(result, protocol::Ter::TEM_BAD_SIGNATURE);
    }

    #[test]
    fn batch_preflight_sig_validated_requires_missing_signer_even_without_batch_signers() {
        let called = Cell::new(false);
        let result = validate_batch_preflight_sig_validated(
            "outer",
            [
                StubInnerTx::new("tx-1", "alice"),
                StubInnerTx::new("tx-2", "outer"),
            ],
            None::<Vec<StubBatchSigner>>,
            || {
                called.set(true);
                true
            },
        );

        assert_eq!(result, protocol::Ter::TEM_BAD_SIGNER);
        assert!(!called.get());
    }

    #[test]
    fn batch_preflight_sig_validated_accepts_required_signers_and_counterparty() {
        let mut first = StubInnerTx::new("tx-1", "alice");
        first.counterparty = Some("carol");

        let result = validate_batch_preflight_sig_validated(
            "outer",
            [first, StubInnerTx::new("tx-2", "outer")],
            Some([
                StubBatchSigner { account: "alice" },
                StubBatchSigner { account: "carol" },
            ]),
            || true,
        );

        assert_eq!(result, protocol::Ter::TES_SUCCESS);
    }
}
