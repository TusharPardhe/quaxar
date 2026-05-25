//! Rust port of the current `xrpld/app/misc/Transaction.*` owner surface.

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;

use basics::{base_uint::Uint256, range_set::ClosedInterval};
use protocol::{
    JsonOptions, JsonValue, STTx, SerialIter, Ter, TxMeta, TxSearched, XRPAmount,
    get_field_by_symbol,
};

#[allow(non_camel_case_types)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransStatus {
    NEW = 0,
    INVALID = 1,
    INCLUDED = 2,
    CONFLICTED = 3,
    COMMITTED = 4,
    HELD = 5,
    REMOVED = 6,
    OBSOLETE = 7,
    INCOMPLETE = 8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SubmitResult {
    pub applied: bool,
    pub broadcast: bool,
    pub queued: bool,
    pub kept: bool,
}

impl SubmitResult {
    pub fn clear(&mut self) {
        self.applied = false;
        self.broadcast = false;
        self.queued = false;
        self.kept = false;
    }

    pub fn any(&self) -> bool {
        self.applied || self.broadcast || self.queued || self.kept
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CurrentLedgerState {
    pub validated_ledger: u32,
    pub min_fee_required: XRPAmount,
    pub account_seq_next: u32,
    pub account_seq_avail: u32,
}

impl CurrentLedgerState {
    pub fn new(
        validated_ledger: u32,
        min_fee_required: XRPAmount,
        account_seq_next: u32,
        account_seq_avail: u32,
    ) -> Self {
        Self {
            validated_ledger,
            min_fee_required,
            account_seq_next,
            account_seq_avail,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransactionLocator {
    Found {
        nodestore_hash: Uint256,
        ledger_seq: u32,
    },
    Searched(ClosedInterval<u32>),
}

impl TransactionLocator {
    pub fn is_found(&self) -> bool {
        matches!(self, Self::Found { .. })
    }

    pub fn nodestore_hash(&self) -> Option<Uint256> {
        match self {
            Self::Found { nodestore_hash, .. } => Some(*nodestore_hash),
            Self::Searched(_) => None,
        }
    }

    pub fn ledger_sequence(&self) -> Option<u32> {
        match self {
            Self::Found { ledger_seq, .. } => Some(*ledger_seq),
            Self::Searched(_) => None,
        }
    }

    pub fn ledger_range_searched(&self) -> Option<ClosedInterval<u32>> {
        match self {
            Self::Found { .. } => None,
            Self::Searched(range) => Some(*range),
        }
    }
}

#[derive(Debug, Clone)]
pub enum TransactionLoadOutcome {
    Found {
        transaction: Arc<Transaction>,
        meta: Option<TxMeta>,
    },
    NotFound(TxSearched),
}

pub trait TransactionLocatorSource {
    fn locate_transaction(&self, id: Uint256) -> TransactionLocator;
}

pub trait TransactionLoadSource {
    type Error;

    fn load_transaction(
        &self,
        id: Uint256,
        range: Option<ClosedInterval<u32>>,
    ) -> Result<TransactionLoadOutcome, Self::Error>;
}

pub trait TransactionCloseTimeSource {
    fn close_time_for_ledger_seq(&self, ledger_seq: u32) -> Option<i64>;
}

#[derive(Debug, Clone)]
pub struct Transaction {
    transaction: Arc<STTx>,
    transaction_id: basics::base_uint::Uint256,
    ledger_index: u32,
    txn_seq: Option<u32>,
    network_id: Option<u32>,
    status: TransStatus,
    result: Ter,
    applying: bool,
    submit_result: SubmitResult,
    current_ledger_state: Option<CurrentLedgerState>,
}

impl Transaction {
    pub fn new(transaction: Arc<STTx>) -> Self {
        let transaction_id = transaction.get_transaction_id();
        Self {
            transaction,
            transaction_id,
            ledger_index: 0,
            txn_seq: None,
            network_id: None,
            status: TransStatus::NEW,
            result: Ter::TEM_UNCERTAIN,
            applying: false,
            submit_result: SubmitResult {
                applied: false,
                broadcast: false,
                queued: false,
                kept: false,
            },
            current_ledger_state: None,
        }
    }

    pub fn transaction_from_sql(
        ledger_seq: Option<u64>,
        status: Option<&str>,
        raw_txn: &[u8],
    ) -> Result<Self, String> {
        let in_ledger = ledger_seq
            .unwrap_or(0)
            .try_into()
            .map_err(|_| "ledger sequence exceeds u32".to_string())?;

        let parsed = catch_unwind(AssertUnwindSafe(|| {
            let mut serial = SerialIter::new(raw_txn);
            STTx::from_serial_iter(&mut serial)
        }))
        .map_err(|payload| {
            unwind_message(payload).unwrap_or_else(|| "failed to parse STTx".into())
        })?;

        let mut transaction = Self::new(Arc::new(parsed));
        transaction.set_status(Self::sql_transaction_status(status));
        transaction.set_ledger(in_ledger);
        Ok(transaction)
    }

    pub fn locate<S: TransactionLocatorSource>(id: Uint256, source: &S) -> TransactionLocator {
        source.locate_transaction(id)
    }

    pub fn load<S: TransactionLoadSource>(
        id: Uint256,
        source: &S,
    ) -> Result<TransactionLoadOutcome, S::Error> {
        source.load_transaction(id, None)
    }

    pub fn load_in_range<S: TransactionLoadSource>(
        id: Uint256,
        source: &S,
        range: ClosedInterval<u32>,
    ) -> Result<TransactionLoadOutcome, S::Error> {
        source.load_transaction(id, Some(range))
    }

    pub fn sql_transaction_status(status: Option<&str>) -> TransStatus {
        let code = status.and_then(|status| status.as_bytes().first()).copied();

        match code {
            Some(b'N') => TransStatus::NEW,
            Some(b'C') => TransStatus::CONFLICTED,
            Some(b'H') => TransStatus::HELD,
            Some(b'V') => TransStatus::COMMITTED,
            Some(b'I') => TransStatus::INCLUDED,
            Some(b'U') | None => TransStatus::INVALID,
            Some(other) => {
                debug_assert!(
                    false,
                    "xrpl::Transaction::sqlTransactionStatus : unknown transaction status ({other})"
                );
                TransStatus::INVALID
            }
        }
    }

    pub fn get_s_transaction(&self) -> &Arc<STTx> {
        &self.transaction
    }

    pub fn get_id(&self) -> basics::base_uint::Uint256 {
        self.transaction_id
    }

    pub fn get_ledger(&self) -> u32 {
        self.ledger_index
    }

    pub fn is_validated(&self) -> bool {
        self.ledger_index != 0
    }

    pub fn get_status(&self) -> TransStatus {
        self.status
    }

    pub fn get_result(&self) -> Ter {
        self.result
    }

    pub fn set_result(&mut self, result: Ter) {
        self.result = result;
    }

    pub fn set_status_with_ledger(
        &mut self,
        status: TransStatus,
        ledger_seq: u32,
        transaction_seq: Option<u32>,
        network_id: Option<u32>,
    ) {
        self.status = status;
        self.ledger_index = ledger_seq;
        if let Some(transaction_seq) = transaction_seq {
            self.txn_seq = Some(transaction_seq);
        }
        if let Some(network_id) = network_id {
            self.network_id = Some(network_id);
        }
    }

    pub fn set_status(&mut self, status: TransStatus) {
        self.status = status;
    }

    pub fn set_ledger(&mut self, ledger: u32) {
        self.ledger_index = ledger;
    }

    pub fn set_applying(&mut self) {
        self.applying = true;
    }

    pub fn get_applying(&self) -> bool {
        self.applying
    }

    pub fn clear_applying(&mut self) {
        self.applying = false;
    }

    pub fn get_submit_result(&self) -> SubmitResult {
        self.submit_result
    }

    pub fn clear_submit_result(&mut self) {
        self.submit_result.clear();
    }

    pub fn set_applied(&mut self) {
        self.submit_result.applied = true;
    }

    pub fn set_queued(&mut self) {
        self.submit_result.queued = true;
    }

    pub fn set_broadcast(&mut self) {
        self.submit_result.broadcast = true;
    }

    pub fn set_kept(&mut self) {
        self.submit_result.kept = true;
    }

    pub fn get_current_ledger_state(&self) -> Option<CurrentLedgerState> {
        self.current_ledger_state
    }

    pub fn set_current_ledger_state(
        &mut self,
        validated_ledger: u32,
        fee: XRPAmount,
        account_seq: u32,
        available_seq: u32,
    ) {
        self.current_ledger_state = Some(CurrentLedgerState::new(
            validated_ledger,
            fee,
            account_seq,
            available_seq,
        ));
    }

    pub fn get_json(&self, options: JsonOptions, binary: bool) -> JsonValue {
        self.get_json_with_close_time(options, binary, None)
    }

    pub fn get_json_with_close_time_source<S: TransactionCloseTimeSource>(
        &self,
        options: JsonOptions,
        binary: bool,
        source: &S,
    ) -> JsonValue {
        let close_time = if self.ledger_index != 0
            && (options & JsonOptions::INCLUDE_DATE) != JsonOptions::NONE
        {
            source.close_time_for_ledger_seq(self.ledger_index)
        } else {
            None
        };

        self.get_json_with_close_time(options, binary, close_time)
    }

    pub fn get_json_with_close_time(
        &self,
        options: JsonOptions,
        binary: bool,
        close_time: Option<i64>,
    ) -> JsonValue {
        let mut ret = self
            .transaction
            .get_json_binary(options & !JsonOptions::INCLUDE_DATE, binary);

        if self.ledger_index == 0 {
            return ret;
        }

        let JsonValue::Object(ref mut object) = ret else {
            return ret;
        };

        if (options & JsonOptions::DISABLE_API_PRIOR_V2) == JsonOptions::NONE {
            object.insert(
                "inLedger".to_string(),
                JsonValue::Unsigned(u64::from(self.ledger_index)),
            );
        }

        object.insert(
            "ledger_index".to_string(),
            JsonValue::Unsigned(u64::from(self.ledger_index)),
        );

        if (options & JsonOptions::INCLUDE_DATE) != JsonOptions::NONE {
            if let Some(close_time) = close_time {
                object.insert("date".to_string(), JsonValue::Signed(close_time));
            }
        }

        let mut network_id = self.network_id;
        let network_id_field = get_field_by_symbol("sfNetworkID");
        if self.transaction.is_field_present(network_id_field) {
            network_id = Some(self.transaction.get_field_u32(network_id_field));
        }

        if let (Some(txn_seq), Some(network_id)) = (self.txn_seq, network_id) {
            if let Some(ctid) = encode_ctid(self.ledger_index, txn_seq, network_id) {
                object.insert("ctid".to_string(), JsonValue::String(ctid));
            }
        }

        ret
    }
}

fn encode_ctid(ledger_seq: u32, txn_index: u32, network_id: u32) -> Option<String> {
    const MAX_LEDGER_SEQ: u32 = 0x0FFF_FFFF;
    const MAX_TXN_INDEX: u32 = 0xFFFF;
    const MAX_NETWORK_ID: u32 = 0xFFFF;

    if ledger_seq > MAX_LEDGER_SEQ || txn_index > MAX_TXN_INDEX || network_id > MAX_NETWORK_ID {
        return None;
    }

    let ctid_value = ((0xC000_0000_u64 + u64::from(ledger_seq)) << 32)
        | ((u64::from(txn_index) << 16) | u64::from(network_id));
    Some(format!("{ctid_value:016X}"))
}

fn unwind_message(payload: Box<dyn std::any::Any + Send>) -> Option<String> {
    match payload.downcast::<String>() {
        Ok(message) => Some(*message),
        Err(payload) => match payload.downcast::<&'static str>() {
            Ok(message) => Some((*message).to_string()),
            Err(_) => None,
        },
    }
}
