//! App-owned `LedgerToJson` wrapper above the landed `ledger` crate helpers.

use basics::chrono::NetClockTimePoint;
use basics::tagged_cache::CacheClock;
use ledger::{
    DEFAULT_LEDGER_JSON_API_VERSION, Ledger, LedgerFill as LedgerCoreFill, LedgerFillOptions,
    copy_from as ledger_copy_from, fill_json as fill_ledger_json,
    fill_json_with_family as fill_ledger_json_with_family,
};
use protocol::{AccountID, JsonValue, STTx, TxMeta};
use shamap::family::{FullBelowCache, MissingNodeReporter, SHAMapFamily, SHAMapNodeFetcher};
use shamap::traversal::TraversalError;
use std::collections::BTreeMap;
use std::hash::BuildHasher;
use std::sync::Arc;
use tx::TxDetails;

use crate::LedgerToJsonContext;
use crate::ledger_to_json::ledger_to_json_queue::fill_json_queue;
use crate::ledger_to_json::ledger_to_json_tx::{
    fill_json_transactions, fill_json_transactions_with_family,
};

#[derive(Clone, Copy)]
pub struct LedgerTxEntry<'a> {
    pub txn: &'a STTx,
    pub meta: Option<&'a TxMeta>,
}

pub struct AppLedgerFill<'a> {
    pub ledger: &'a Ledger,
    pub options: LedgerFillOptions,
    pub transactions: &'a [LedgerTxEntry<'a>],
    pub tx_queue: &'a [TxDetails<Arc<STTx>, AccountID>],
    pub context: Option<&'a dyn LedgerToJsonContext>,
    pub close_time: Option<NetClockTimePoint>,
    api_version_override: Option<u32>,
}

impl<'a> AppLedgerFill<'a> {
    pub fn new(ledger: &'a Ledger, options: LedgerFillOptions) -> Self {
        Self {
            ledger,
            options,
            transactions: &[],
            tx_queue: &[],
            context: None,
            close_time: None,
            api_version_override: None,
        }
    }

    pub fn with_transactions(mut self, transactions: &'a [LedgerTxEntry<'a>]) -> Self {
        self.transactions = transactions;
        self
    }

    pub fn with_tx_queue(mut self, tx_queue: &'a [TxDetails<Arc<STTx>, AccountID>]) -> Self {
        self.tx_queue = tx_queue;
        self
    }

    pub fn with_context(mut self, context: &'a dyn LedgerToJsonContext) -> Self {
        self.close_time = context.get_close_time_by_seq(self.ledger.header().seq);
        self.context = Some(context);
        self
    }

    pub fn with_close_time(mut self, close_time: Option<NetClockTimePoint>) -> Self {
        self.close_time = close_time;
        self
    }

    pub fn with_api_version(mut self, api_version: u32) -> Self {
        self.api_version_override = Some(api_version);
        self
    }

    pub fn api_version(&self) -> u32 {
        self.api_version_override
            .or_else(|| self.context.map(LedgerToJsonContext::api_version))
            .unwrap_or(DEFAULT_LEDGER_JSON_API_VERSION)
    }

    pub fn is_full(&self) -> bool {
        self.options.contains(LedgerFillOptions::FULL)
    }

    pub fn is_expanded(&self) -> bool {
        self.is_full() || self.options.contains(LedgerFillOptions::EXPAND)
    }

    pub fn is_binary(&self) -> bool {
        self.options.contains(LedgerFillOptions::BINARY)
    }
}

pub fn fill_json(json: &mut JsonValue, fill: &AppLedgerFill<'_>) -> Result<(), TraversalError> {
    let core_fill = LedgerCoreFill::new(fill.ledger, fill.options)
        .with_closed(fill.ledger.is_immutable())
        .with_api_version(fill.api_version())
        .with_close_time(fill.close_time);
    fill_ledger_json(json, &core_fill)?;

    if fill.is_full() || fill.options.contains(LedgerFillOptions::DUMP_TXRP) {
        fill_json_transactions(json, fill);
    }

    Ok(())
}

pub fn get_json_with_family<CLOCK, S, C, F, MR, NS>(
    fill: &AppLedgerFill<'_>,
    family: &SHAMapFamily<CLOCK, S, C, F, MR, NS>,
) -> Result<JsonValue, TraversalError>
where
    CLOCK: CacheClock,
    S: BuildHasher + Clone,
    C: FullBelowCache,
    F: SHAMapNodeFetcher,
    MR: MissingNodeReporter,
{
    let core_fill = LedgerCoreFill::new(fill.ledger, fill.options)
        .with_closed(fill.ledger.is_immutable())
        .with_api_version(fill.api_version())
        .with_close_time(fill.close_time);
    let mut json = JsonValue::Null;
    fill_ledger_json_with_family(&mut json, &core_fill, family)?;

    if fill.is_full() || fill.options.contains(LedgerFillOptions::DUMP_TXRP) {
        fill_json_transactions_with_family(&mut json, fill, family)?;
    }

    Ok(json)
}

pub fn add_json(json: &mut JsonValue, fill: &AppLedgerFill<'_>) -> Result<(), TraversalError> {
    let JsonValue::Object(root) = json else {
        if matches!(json, JsonValue::Null) {
            *json = JsonValue::Object(BTreeMap::new());
        } else {
            panic!("ledger json root must be an object or null");
        }
        return add_json(json, fill);
    };

    let mut ledger = JsonValue::Object(BTreeMap::new());
    fill_json(&mut ledger, fill)?;
    root.insert("ledger".to_owned(), ledger);

    if fill.options.contains(LedgerFillOptions::DUMP_QUEUE) && !fill.tx_queue.is_empty() {
        fill_json_queue(json, fill, fill.tx_queue);
    }

    Ok(())
}

pub fn get_json(fill: &AppLedgerFill<'_>) -> Result<JsonValue, TraversalError> {
    let mut json = JsonValue::Null;
    fill_json(&mut json, fill)?;
    Ok(json)
}

pub fn copy_from(to: &mut JsonValue, from: &JsonValue) {
    ledger_copy_from(to, from);
}
