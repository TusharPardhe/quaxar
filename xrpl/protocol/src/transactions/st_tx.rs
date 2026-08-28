//! `xrpl/protocol/STTx.*` core owner port.

use std::{
    collections::BTreeSet,
    fmt,
    ops::{Deref, DerefMut},
    panic::{AssertUnwindSafe, catch_unwind},
};

use basics::string_utilities::sql_blob_literal;
use basics::{base_uint::Uint256, str_hex::str_hex, string_utilities::str_unhex};

use crate::batch_sign::{
    BatchSigner, INTERNAL_BATCH_SIGNATURE_CHECK_FAILURE, NOT_A_BATCH_TRANSACTION_ERROR,
    check_batch_sign,
};
use crate::signature_check::{
    SignatureCheckObject, check_signature, check_signature_with_counterparty,
};
use crate::sttx_multi_sign::{StTxMultiSignObject, StTxMultiSigner};
use crate::sttx_single_sign::StTxSingleSignObject;
use crate::{
    AccountID, HashPrefix, JsonOptions, JsonValue, PublicKey, Rules, SOETxMPTIssue, STAccount,
    STAmount, STObject, SecretKey, SeqProxy, SerialIter, SerializedTypeId, Serializer, SignError,
    StBase, StBaseCore, TxFormats, TxType, ValidationError, downcast_stbase_ref,
    get_field_by_symbol, to_base58, verify, xrp_account,
};

const TX_MIN_SIZE_BYTES: i32 = 32;
const TX_MAX_SIZE_BYTES: i32 = 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TxnSql {
    New,
    Conflict,
    Held,
    Validated,
    Included,
    Unknown,
}

impl TxnSql {
    pub const fn as_char(self) -> char {
        match self {
            Self::New => 'N',
            Self::Conflict => 'C',
            Self::Held => 'H',
            Self::Validated => 'V',
            Self::Included => 'I',
            Self::Unknown => 'U',
        }
    }
}

impl fmt::Display for TxnSql {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.as_char().to_string())
    }
}

#[derive(Debug)]
pub struct STTx {
    object: STObject,
    transaction_id: Uint256,
    tx_type: TxType,
}

impl Clone for STTx {
    fn clone(&self) -> Self {
        Self {
            object: self.object.clone(),
            transaction_id: self.transaction_id,
            tx_type: self.tx_type,
        }
    }
}

impl PartialEq for STTx {
    fn eq(&self, other: &Self) -> bool {
        self.object == other.object
            && self.transaction_id == other.transaction_id
            && self.tx_type == other.tx_type
    }
}

impl Eq for STTx {}

impl StTxSingleSignObject for STObject {
    fn signers_present(&self) -> bool {
        self.is_field_present(get_field_by_symbol("sfSigners"))
    }
}

impl StTxMultiSigner<AccountID> for STObject {
    fn account_id(&self) -> AccountID {
        self.get_account_id(get_field_by_symbol("sfAccount"))
    }
}

impl StTxMultiSignObject<AccountID, STObject> for STObject {
    type Signers = Vec<STObject>;

    fn signers_present(&self) -> bool {
        self.is_field_present(get_field_by_symbol("sfSigners"))
    }

    fn txn_signature_present(&self) -> bool {
        self.is_field_present(get_field_by_symbol("sfTxnSignature"))
    }

    fn signers(&self) -> Self::Signers {
        self.get_field_array(get_field_by_symbol("sfSigners"))
            .iter()
            .cloned()
            .collect()
    }
}

impl SignatureCheckObject<AccountID, STObject> for STObject {
    fn signing_pub_key_is_empty(&self) -> bool {
        self.get_field_vl(get_field_by_symbol("sfSigningPubKey"))
            .is_empty()
    }
}

impl BatchSigner<AccountID, STObject> for STObject {
    fn signing_pub_key_is_empty(&self) -> bool {
        self.get_field_vl(get_field_by_symbol("sfSigningPubKey"))
            .is_empty()
    }
}

impl STTx {
    pub const MIN_MULTI_SIGNERS: usize = 1;
    pub const MAX_MULTI_SIGNERS: usize = 32;

    pub fn new(type_: TxType, assembler: impl FnOnce(&mut STObject)) -> Self {
        let format = tx_format(type_);
        let mut object = STObject::new(get_field_by_symbol("sfTransaction"));
        object.set(format.so_template());
        object.set_field_u16(
            get_field_by_symbol("sfTransactionType"),
            format.format_type().into(),
        );

        assembler(&mut object);

        let actual_type =
            TxType::from_u16(object.get_field_u16(get_field_by_symbol("sfTransactionType")));
        assert_eq!(
            actual_type, type_,
            "Transaction type was mutated during assembly"
        );

        Self::finish_from_object(object, false)
    }

    pub fn from_stobject(object: STObject) -> Self {
        Self::finish_from_object(object, true)
    }

    pub fn from_serial_iter(sit: &mut SerialIter<'_>) -> Self {
        let length = sit.get_bytes_left();
        if !(TX_MIN_SIZE_BYTES..=TX_MAX_SIZE_BYTES).contains(&length) {
            return Self {
                tx_type: TxType::PAYMENT,
                object: STObject::new(get_field_by_symbol("sfTransaction")),
                transaction_id: Default::default(),
            };
        }

        let mut object = STObject::new(get_field_by_symbol("sfTransaction"));
        let has_object_terminator = object.set_from_serial_iter(sit, 0);
        assert!(
            !has_object_terminator,
            "Transaction contains an object terminator"
        );

        Self::finish_from_object(object, true)
    }

    pub fn get_signature(sig_object: &STObject) -> Vec<u8> {
        let signature_field = get_field_by_symbol("sfTxnSignature");
        if !sig_object.is_field_present(signature_field) {
            return Vec::new();
        }

        catch_unwind(AssertUnwindSafe(|| {
            sig_object.get_field_vl(signature_field)
        }))
        .unwrap_or_default()
    }

    pub fn get_signing_hash(&self) -> Uint256 {
        self.object.get_signing_hash(HashPrefix::TxSign)
    }

    pub fn get_txn_type(&self) -> TxType {
        self.tx_type
    }

    pub fn get_signing_pub_key(&self) -> Vec<u8> {
        self.get_field_vl(get_field_by_symbol("sfSigningPubKey"))
    }

    pub fn get_seq_proxy(&self) -> SeqProxy {
        let sequence = self.get_field_u32(get_field_by_symbol("sfSequence"));
        if sequence != 0 {
            return SeqProxy::sequence(sequence);
        }

        let ticket_sequence = get_field_by_symbol("sfTicketSequence");
        if !self.is_field_present(ticket_sequence) {
            return SeqProxy::sequence(sequence);
        }

        SeqProxy::ticket(self.get_field_u32(ticket_sequence))
    }

    pub fn get_seq_value(&self) -> u32 {
        self.get_seq_proxy().value()
    }

    pub fn get_initiator(&self) -> AccountID {
        let delegate = get_field_by_symbol("sfDelegate");
        if self.is_field_present(delegate) {
            return self.get_account_id(delegate);
        }
        self.get_account_id(get_field_by_symbol("sfAccount"))
    }

    /// Account charged for the transaction fee, matching rippled
    /// `STTx::getFeePayerID()`.
    pub fn get_fee_payer_id(&self) -> AccountID {
        let sponsor = get_field_by_symbol("sfSponsor");
        let sponsor_flags = get_field_by_symbol("sfSponsorFlags");
        const SPF_SPONSOR_FEE: u32 = 1;
        if self.is_field_present(sponsor)
            && self.is_field_present(sponsor_flags)
            && (self.get_field_u32(sponsor_flags) & SPF_SPONSOR_FEE) != 0
        {
            return self.get_account_id(sponsor);
        }
        self.get_initiator()
    }

    #[deprecated(note = "renamed to get_initiator per rippled PR #7603")]
    pub fn get_fee_payer(&self) -> AccountID {
        self.get_initiator()
    }

    pub fn get_mentioned_accounts(&self) -> BTreeSet<AccountID> {
        let mut accounts = BTreeSet::new();

        for field in self.object.iter() {
            if let Some(account) = field.as_any().downcast_ref::<STAccount>() {
                if !account.is_default() {
                    accounts.insert(*account.value());
                }
                continue;
            }

            if let Some(amount) = field.as_any().downcast_ref::<STAmount>() {
                let issuer = amount.asset().issuer();
                if issuer != xrp_account() {
                    accounts.insert(issuer);
                }
            }
        }

        accounts
    }

    pub fn get_transaction_id(&self) -> Uint256 {
        self.transaction_id
    }

    pub fn clone_as_object(&self) -> STObject {
        self.object.clone()
    }

    pub fn sign(
        &mut self,
        public_key: &PublicKey,
        secret_key: &SecretKey,
        signature_target: Option<&'static crate::SField>,
    ) -> Result<(), SignError> {
        let signing_data = self.signing_data();
        let signature = crate::sign(public_key, secret_key, signing_data.data())?;

        if let Some(signature_target) = signature_target {
            self.peek_field_object(signature_target)
                .set_field_vl(get_field_by_symbol("sfTxnSignature"), &signature);
        } else {
            self.set_field_vl(get_field_by_symbol("sfTxnSignature"), &signature);
        }

        self.transaction_id = self.object.get_hash(HashPrefix::TransactionId);
        Ok(())
    }

    pub fn check_sign(&self, rules: &Rules) -> Result<(), String> {
        self.check_sign_for_object(rules, &self.object)?;

        let counterparty_field = get_field_by_symbol("sfCounterpartySignature");
        if self.is_field_present(counterparty_field) {
            let counterparty_signature = self.get_field_object(counterparty_field);
            check_signature_with_counterparty(
                &self.object,
                Some(&counterparty_signature),
                |_| Ok(()),
                |counterparty_signature| self.check_sign_for_object(rules, counterparty_signature),
            )?;
        }

        let sponsor_signature_field = get_field_by_symbol("sfSponsorSignature");
        if self.is_field_present(sponsor_signature_field) {
            let sponsor_signature = self.get_field_object(sponsor_signature_field);
            self.check_sign_for_object(rules, &sponsor_signature)
                .map_err(|error| format!("Sponsor: {error}"))?;
        }

        if self.is_field_present(get_field_by_symbol("sfBatchSigners")) {
            self.check_batch_sign(rules)?;
        }

        Ok(())
    }

    pub fn check_batch_sign(&self, _rules: &Rules) -> Result<(), String> {
        catch_unwind(AssertUnwindSafe(|| {
            if self.tx_type != TxType::BATCH {
                return Err(NOT_A_BATCH_TRANSACTION_ERROR.to_owned());
            }

            let batch_signers_field = get_field_by_symbol("sfBatchSigners");
            if !self.is_field_present(batch_signers_field) {
                return Err("Missing BatchSigners field.".to_owned());
            }
            let batch_signers = self.get_field_array(batch_signers_field);
            if batch_signers.len() > MAX_BATCH_SIGNER_COUNT {
                return Err("BatchSigners array exceeds max entries.".to_owned());
            }

            let raw_transactions_field = get_field_by_symbol("sfRawTransactions");
            if !self.is_field_present(raw_transactions_field) {
                return Err("Missing inner transactions.".to_owned());
            }
            let raw_transactions = self.get_field_array(raw_transactions_field);
            if raw_transactions.is_empty() {
                return Err("Missing inner transactions.".to_owned());
            }
            if raw_transactions.len() > MAX_BATCH_TX_COUNT {
                return Err("Raw Transactions array exceeds max entries.".to_owned());
            }

            // Rippled constructs each raw entry as an STTx after applying its
            // transaction template, before deriving the inner ID. Do that
            // before any signature work so malformed/nested input cannot be
            // hashed as an arbitrary STObject representation.
            let tx_type_field = get_field_by_symbol("sfTransactionType");
            let transaction_ids = raw_transactions
                .iter()
                .map(|raw_transaction| {
                    let tx_type = TxType::from_u16(raw_transaction.get_field_u16(tx_type_field));
                    if tx_type == TxType::BATCH {
                        return Err("Batch inner transaction cannot be a Batch.".to_owned());
                    }
                    let Some(format) = TxFormats::get_instance().find_by_type(tx_type) else {
                        return Err("Invalid batch inner transaction type.".to_owned());
                    };
                    let mut canonical = raw_transaction.clone();
                    canonical.apply_template(format.so_template());
                    Ok(STTx::from_stobject(canonical).get_transaction_id())
                })
                .collect::<Result<Vec<_>, _>>()?;
            let batch_message = serialize_batch_message(
                self.get_account_id(get_field_by_symbol("sfAccount")),
                self.get_seq_value(),
                self.get_flags(),
                &transaction_ids,
            );

            check_batch_sign(
                self.tx_type,
                batch_signers.iter().cloned().collect::<Vec<_>>(),
                |batch_signer| {
                    let mut message = batch_message.clone();
                    crate::finish_multi_signing_data(batch_signer.account_id(), &mut message);
                    verify_signature_bytes(batch_signer, message.data())
                },
                |batch_signer, signer: &STObject| {
                    let mut message = batch_message.clone();
                    crate::finish_multi_signing_data(batch_signer.account_id(), &mut message);
                    crate::finish_multi_signing_data(signer.account_id(), &mut message);
                    if verify_signature_bytes(signer, message.data()) {
                        Ok(())
                    } else {
                        Err(String::new())
                    }
                },
                |account_id| to_base58(*account_id),
            )
        }))
        .unwrap_or_else(|_| Err(INTERNAL_BATCH_SIGNATURE_CHECK_FAILURE.to_owned()))
    }

    pub fn get_batch_transaction_ids(&self) -> Vec<Uint256> {
        catch_unwind(AssertUnwindSafe(|| self.canonical_batch_transaction_ids()))
            .ok()
            .and_then(Result::ok)
            .unwrap_or_default()
    }

    fn canonical_batch_transaction_ids(&self) -> Result<Vec<Uint256>, String> {
        if self.tx_type != TxType::BATCH {
            return Err(NOT_A_BATCH_TRANSACTION_ERROR.to_owned());
        }

        let raw_transactions_field = get_field_by_symbol("sfRawTransactions");
        if !self.is_field_present(raw_transactions_field) {
            return Err("Missing inner transactions.".to_owned());
        }
        let raw_transactions = self.get_field_array(raw_transactions_field);
        if raw_transactions.is_empty() {
            return Err("Missing inner transactions.".to_owned());
        }
        if raw_transactions.len() > MAX_BATCH_TX_COUNT {
            return Err("Raw Transactions array exceeds max entries.".to_owned());
        }

        let tx_type_field = get_field_by_symbol("sfTransactionType");
        raw_transactions
            .iter()
            .map(|raw_transaction| {
                let tx_type = TxType::from_u16(raw_transaction.get_field_u16(tx_type_field));
                if tx_type == TxType::BATCH {
                    return Err("Batch inner transaction cannot be a Batch.".to_owned());
                }
                let Some(format) = TxFormats::get_instance().find_by_type(tx_type) else {
                    return Err("Invalid batch inner transaction type.".to_owned());
                };
                let mut canonical = raw_transaction.clone();
                canonical.apply_template(format.so_template());
                Ok(STTx::from_stobject(canonical).get_transaction_id())
            })
            .collect()
    }

    pub fn get_meta_sql_insert_replace_header() -> &'static str {
        "INSERT OR REPLACE INTO Transactions \
(TransID, TransType, FromAcct, FromSeq, LedgerSeq, Status, RawTxn, TxnMeta) VALUES "
    }

    pub fn get_meta_sql(&self, in_ledger: u32, escaped_meta_data: &str) -> String {
        let mut raw_txn = Serializer::default();
        self.add(&mut raw_txn);
        self.get_meta_sql_with_raw_txn(raw_txn, in_ledger, TxnSql::Validated, escaped_meta_data)
    }

    pub fn get_meta_sql_with_raw_txn(
        &self,
        raw_txn: Serializer,
        in_ledger: u32,
        status: TxnSql,
        escaped_meta_data: &str,
    ) -> String {
        let raw_txn = sql_blob_literal(raw_txn.peek_data());
        let format = tx_format(self.tx_type);

        format!(
            "('{}', '{}', '{}', '{}', '{}', '{}', {}, {})",
            self.get_transaction_id(),
            format.name(),
            to_base58(self.get_account_id(get_field_by_symbol("sfAccount"))),
            self.get_field_u32(get_field_by_symbol("sfSequence")),
            in_ledger,
            status.as_char(),
            raw_txn,
            escaped_meta_data
        )
    }

    pub fn get_json_binary(&self, options: JsonOptions, binary: bool) -> JsonValue {
        let v1 = (options & JsonOptions::DISABLE_API_PRIOR_V2) == JsonOptions::NONE;

        if binary {
            let data = str_hex(self.object.get_serializer().data());
            if v1 {
                return JsonValue::Object(
                    [
                        (
                            "hash".to_string(),
                            JsonValue::String(self.transaction_id.to_string()),
                        ),
                        ("tx".to_string(), JsonValue::String(data)),
                    ]
                    .into_iter()
                    .collect(),
                );
            }
            return JsonValue::String(data);
        }

        self.json(options)
    }

    fn finish_from_object(mut object: STObject, reapply_template: bool) -> Self {
        let tx_type =
            TxType::from_u16(object.get_field_u16(get_field_by_symbol("sfTransactionType")));
        if reapply_template && let Some(format) = TxFormats::get_instance().find_by_type(tx_type) {
            object.apply_template(format.so_template());
        }

        let transaction_id = object.get_hash(HashPrefix::TransactionId);
        Self {
            object,
            transaction_id,
            tx_type,
        }
    }

    fn signing_data(&self) -> Serializer {
        let mut serializer = Serializer::default();
        serializer.add32_prefix(HashPrefix::TxSign);
        self.object.add_without_signing_fields(&mut serializer);
        serializer
    }

    fn check_sign_for_object(&self, _rules: &Rules, sig_object: &STObject) -> Result<(), String> {
        let signing_data = self.signing_data();
        let multi_signing_data_start = crate::start_multi_signing_data(&self.object);
        let txn_account_id = std::ptr::eq(
            sig_object as *const STObject,
            &self.object as *const STObject,
        )
        .then(|| self.get_initiator());

        check_signature(
            sig_object,
            txn_account_id.as_ref(),
            |sig_object| verify_signature_bytes(sig_object, signing_data.data()),
            |signer| {
                let mut message = multi_signing_data_start.clone();
                crate::finish_multi_signing_data(signer.account_id(), &mut message);
                if verify_signature_bytes(signer, message.data()) {
                    Ok(())
                } else {
                    Err(String::new())
                }
            },
            |account_id| to_base58(*account_id),
        )
    }
}

impl Deref for STTx {
    type Target = STObject;

    fn deref(&self) -> &Self::Target {
        &self.object
    }
}

impl DerefMut for STTx {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.object
    }
}

impl StBase for STTx {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }

    fn core(&self) -> &StBaseCore {
        self.object.core()
    }

    fn core_mut(&mut self) -> &mut StBaseCore {
        self.object.core_mut()
    }

    fn stype(&self) -> SerializedTypeId {
        SerializedTypeId::Transaction
    }

    fn full_text(&self) -> String {
        format!(
            "\"{}\" = {{{}}}",
            self.transaction_id,
            self.object.full_text()
        )
    }

    fn text(&self) -> String {
        self.object.text()
    }

    fn json(&self, _options: JsonOptions) -> JsonValue {
        let mut object = match self.object.json(JsonOptions::NONE) {
            JsonValue::Object(object) => object,
            _ => unreachable!("STObject::json must produce an object"),
        };

        if (_options & JsonOptions::DISABLE_API_PRIOR_V2) == JsonOptions::NONE {
            object.insert(
                "hash".to_string(),
                JsonValue::String(self.transaction_id.to_string()),
            );
        }

        JsonValue::Object(object)
    }

    fn add(&self, serializer: &mut crate::Serializer) {
        self.object.add(serializer);
    }

    fn is_equivalent(&self, other: &dyn StBase) -> bool {
        let other = downcast_stbase_ref::<Self>(other);
        self.tx_type == other.tx_type
            && self.transaction_id == other.transaction_id
            && self.object.is_equivalent(&other.object)
    }

    fn is_default(&self) -> bool {
        self.object.is_default()
    }

    fn is_valid(&self) -> bool {
        self.object.is_valid()
    }

    fn check(&self) -> Result<(), ValidationError> {
        self.object.check()
    }
}

fn tx_format(type_: TxType) -> &'static crate::KnownFormatItem<TxType, crate::TxFormatMetadata> {
    TxFormats::get_instance()
        .find_by_type(type_)
        .unwrap_or_else(|| panic!("Invalid transaction type {}", type_.to_u16()))
}

fn verify_signature_bytes(signature_object: &STObject, message: &[u8]) -> bool {
    let signing_pub_key = signature_object.get_field_vl(get_field_by_symbol("sfSigningPubKey"));
    let Ok(public_key) = PublicKey::from_slice(&signing_pub_key) else {
        return false;
    };

    verify(
        &public_key,
        message,
        &signature_object.get_field_vl(get_field_by_symbol("sfTxnSignature")),
    )
}

fn serialize_batch_message(
    outer_account: AccountID,
    outer_seq_value: u32,
    flags: u32,
    txids: &[Uint256],
) -> Serializer {
    let mut message = Serializer::default();
    message.add32_prefix(HashPrefix::Batch);
    message.add_bit_string(outer_account);
    message.add32(outer_seq_value);
    message.add32(flags);
    message.add32(txids.len() as u32);
    for txid in txids {
        message.add_bit_string(*txid);
    }
    message
}

pub const MAX_BATCH_TX_COUNT: usize = 8;
pub const MAX_BATCH_SIGNER_COUNT: usize = MAX_BATCH_TX_COUNT * 3;

pub fn build_multi_signing_data(object: &STObject, signing_id: AccountID) -> Serializer {
    crate::build_multi_signing_data(object, signing_id)
}

pub fn start_multi_signing_data(object: &STObject) -> Serializer {
    crate::start_multi_signing_data(object)
}

pub fn finish_multi_signing_data(signing_id: AccountID, serializer: &mut Serializer) {
    crate::finish_multi_signing_data(signing_id, serializer);
}

pub fn is_pseudo_tx(tx: &STObject) -> bool {
    if !tx.is_field_present(get_field_by_symbol("sfTransactionType")) {
        return false;
    }

    matches!(
        TxType::from_u16(tx.get_field_u16(get_field_by_symbol("sfTransactionType"))),
        TxType::AMENDMENT | TxType::FEE | TxType::UNL_MODIFY
    )
}

pub fn sterilize(stx: &STTx) -> std::sync::Arc<STTx> {
    let mut serializer = Serializer::default();
    stx.add(&mut serializer);
    let mut sit = SerialIter::new(serializer.data());
    std::sync::Arc::new(STTx::from_serial_iter(&mut sit))
}

pub fn passes_local_checks(st: &STObject) -> Result<(), String> {
    is_memo_okay(st)?;

    if !is_account_field_okay(st) {
        return Err("An account field is invalid.".to_owned());
    }

    if is_pseudo_tx(st) {
        return Err("Cannot submit pseudo transactions.".to_owned());
    }

    if invalid_mpt_amount_in_tx(st) {
        return Err("Amount can not be MPT.".to_owned());
    }

    is_raw_transaction_okay(st)
}

fn is_memo_okay(st: &STObject) -> Result<(), String> {
    let memos_field = get_field_by_symbol("sfMemos");
    if !st.is_field_present(memos_field) {
        return Ok(());
    }

    let memos = st.get_field_array(memos_field);
    let mut serializer = Serializer::new(2048);
    memos.add(&mut serializer);
    if serializer.get_data_length() > 1024 {
        return Err("The memo exceeds the maximum allowed size.".to_owned());
    }

    let memo_field = get_field_by_symbol("sfMemo");
    let memo_type = get_field_by_symbol("sfMemoType");
    let memo_data = get_field_by_symbol("sfMemoData");
    let memo_format = get_field_by_symbol("sfMemoFormat");
    for memo in memos.iter() {
        if memo.fname() != memo_field {
            return Err("A memo array may contain only Memo objects.".to_owned());
        }

        for memo_element in memo.iter() {
            let name = memo_element.fname();
            if name != memo_type && name != memo_data && name != memo_format {
                return Err(
                    "A memo may contain only MemoType, MemoData or MemoFormat fields.".to_owned(),
                );
            }

            let Some(data) = str_unhex(&memo_element.text()) else {
                return Err(
                    "The MemoType, MemoData and MemoFormat fields may only contain hex-encoded data."
                        .to_owned(),
                );
            };

            if name == memo_data {
                continue;
            }

            if data.iter().any(|byte| !is_rfc3986_allowed(*byte)) {
                return Err(
                    "The MemoType and MemoFormat fields may only contain characters that are allowed in URLs under RFC 3986."
                        .to_owned(),
                );
            }
        }
    }

    Ok(())
}

fn is_rfc3986_allowed(byte: u8) -> bool {
    matches!(
        byte,
        b'0'..=b'9'
            | b'A'..=b'Z'
            | b'a'..=b'z'
            | b'-'
            | b'.'
            | b'_'
            | b'~'
            | b':'
            | b'/'
            | b'?'
            | b'#'
            | b'['
            | b']'
            | b'@'
            | b'!'
            | b'$'
            | b'&'
            | b'\''
            | b'('
            | b')'
            | b'*'
            | b'+'
            | b','
            | b';'
            | b'='
            | b'%'
    )
}

fn is_account_field_okay(st: &STObject) -> bool {
    !st.iter().any(|field| {
        field
            .as_any()
            .downcast_ref::<STAccount>()
            .is_some_and(STAccount::is_default)
    })
}

fn invalid_mpt_amount_in_tx(tx: &STObject) -> bool {
    let tx_type_field = get_field_by_symbol("sfTransactionType");
    if !tx.is_field_present(tx_type_field) {
        return false;
    }

    let tx_type = TxType::from_u16(tx.get_field_u16(tx_type_field));
    let Some(item) = TxFormats::get_instance().find_by_type(tx_type) else {
        return false;
    };

    for element in item.so_template().iter() {
        if !tx.is_field_present(element.sfield()) || element.support_mpt() == SOETxMPTIssue::None {
            continue;
        }

        match element.sfield().field_type() {
            SerializedTypeId::Amount => {
                if tx.get_field_amount(element.sfield()).holds_mpt_issue()
                    && element.support_mpt() != SOETxMPTIssue::Supported
                {
                    return true;
                }
            }
            SerializedTypeId::Issue => {
                if matches!(
                    tx.get_field_issue(element.sfield()).asset(),
                    crate::Asset::MPTIssue(_)
                ) && element.support_mpt() != SOETxMPTIssue::Supported
                {
                    return true;
                }
            }
            _ => {}
        }
    }

    false
}

fn is_raw_transaction_okay(st: &STObject) -> Result<(), String> {
    let raw_transactions_field = get_field_by_symbol("sfRawTransactions");
    if !st.is_field_present(raw_transactions_field) {
        return Ok(());
    }

    let tx_type_field = get_field_by_symbol("sfTransactionType");
    if TxType::from_u16(st.get_field_u16(tx_type_field)) != TxType::BATCH {
        return Err("Only Batch transactions may contain raw transactions.".to_owned());
    }

    let batch_signers_field = get_field_by_symbol("sfBatchSigners");
    if st.is_field_present(batch_signers_field)
        && st.get_field_array(batch_signers_field).len() > MAX_BATCH_SIGNER_COUNT
    {
        return Err("Batch Signers array exceeds max entries.".to_owned());
    }

    let raw_transactions = st.get_field_array(raw_transactions_field);
    if raw_transactions.len() > MAX_BATCH_TX_COUNT {
        return Err("Raw Transactions array exceeds max entries.".to_owned());
    }

    for raw in raw_transactions.iter() {
        let tx_type = TxType::from_u16(raw.get_field_u16(tx_type_field));
        if tx_type == TxType::BATCH {
            return Err("Raw Transactions may not contain batch transactions.".to_owned());
        }

        let Some(format) = TxFormats::get_instance().find_by_type(tx_type) else {
            return Err("Invalid raw transaction type.".to_owned());
        };
        let mut candidate = raw.clone();
        candidate.apply_template(format.so_template());
        let inner = STTx::from_stobject(candidate);
        passes_local_checks(&inner)?;
    }

    Ok(())
}
