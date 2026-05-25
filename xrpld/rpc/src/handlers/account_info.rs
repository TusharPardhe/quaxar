//! Narrow `account_info` RPC handler slice.

use std::{collections::BTreeMap, sync::Arc};

use app::ApplicationRoot;
use basics::base_uint::Uint160;
use basics::str_hex::str_hex;
use ledger::Ledger;
use protocol::{
    AccountID, AccountRoot, JsonOptions, JsonValue, LedgerEntryType, LedgerFormats, SField,
    STLedgerEntry, SignerList, StBase, account_keylet, feature_clawback, feature_token_escrow,
    lsfAllowTrustLineClawback, lsfAllowTrustLineLocking, lsfDefaultRipple, lsfDepositAuth,
    lsfDisableMaster, lsfDisallowIncomingCheck, lsfDisallowIncomingNFTokenOffer,
    lsfDisallowIncomingPayChan, lsfDisallowIncomingTrustline, lsfDisallowXRP, lsfGlobalFreeze,
    lsfNoFreeze, lsfPasswordSpent, lsfRequireAuth, lsfRequireDestTag, parse_base58_account_id,
    signers_keylet, to_base58,
};
use tx::TxDetails;

use crate::commands::rpc_helpers::inject_error;
use crate::handlers::ledger_lookup::{
    LedgerLookupContext, LedgerLookupLedger, LedgerLookupSource, RpcRole, RpcStatus,
    lookup_ledger_with_result,
};
use crate::status::RpcErrorCode;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AccountInfoRequest<'a> {
    pub params: &'a JsonValue,
    pub api_version: u32,
    pub role: RpcRole,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AccountQueueTransaction {
    pub seq_proxy: protocol::SeqProxy,
    pub fee_level: u64,
    pub last_valid: Option<u32>,
    pub fee_drops: u64,
    pub max_spend_drops: u64,
    pub auth_change: bool,
}

impl<Tx, Account> From<TxDetails<Tx, Account>> for AccountQueueTransaction {
    fn from(value: TxDetails<Tx, Account>) -> Self {
        let fee_drops = value.consequences.fee();
        let max_spend_drops = value
            .consequences
            .potential_spend()
            .saturating_add(fee_drops);

        Self {
            seq_proxy: value.seq_proxy,
            fee_level: value.fee_level,
            last_valid: value.last_valid,
            fee_drops,
            max_spend_drops,
            auth_change: value.consequences.is_blocker(),
        }
    }
}

pub trait AccountInfoSource: LedgerLookupSource {
    fn read_account_root(
        &self,
        ledger: &LedgerLookupLedger,
        account_id: AccountID,
    ) -> Option<STLedgerEntry>;

    fn read_signer_list(
        &self,
        ledger: &LedgerLookupLedger,
        account_id: AccountID,
    ) -> Option<STLedgerEntry>;

    fn feature_clawback_enabled(&self, _ledger: &LedgerLookupLedger) -> bool {
        false
    }

    fn feature_token_escrow_enabled(&self, _ledger: &LedgerLookupLedger) -> bool {
        false
    }

    fn account_queue_txs(
        &self,
        _ledger: &LedgerLookupLedger,
        _account_id: AccountID,
    ) -> Vec<AccountQueueTransaction> {
        Vec::new()
    }
}

#[derive(Debug, Clone)]
pub struct ApplicationAccountInfoSource<'a> {
    app: &'a ApplicationRoot,
    current_ledger: Option<Arc<Ledger>>,
}

impl<'a> ApplicationAccountInfoSource<'a> {
    pub fn new(app: &'a ApplicationRoot) -> Self {
        Self {
            app,
            current_ledger: None,
        }
    }

    pub fn with_current_ledger(app: &'a ApplicationRoot, current_ledger: Arc<Ledger>) -> Self {
        Self {
            app,
            current_ledger: Some(current_ledger),
        }
    }

    fn current_lookup_ledger(&self) -> Option<LedgerLookupLedger> {
        self.current_ledger
            .as_deref()
            .map(|ledger| ledger_lookup_ledger(ledger, true))
    }

    fn lookup_resolved_ledger(&self, ledger: &LedgerLookupLedger) -> Option<Arc<Ledger>> {
        if ledger.open {
            return self
                .current_ledger
                .as_ref()
                .filter(|candidate| ledger_lookup_ledger(candidate.as_ref(), true) == *ledger)
                .cloned();
        }

        [
            self.app.validated_ledger(),
            self.app.published_ledger(),
            self.app.closed_ledger(),
        ]
        .into_iter()
        .flatten()
        .find(|candidate| ledger_lookup_ledger(candidate.as_ref(), false) == *ledger)
    }

    fn read_entry(
        &self,
        ledger: &LedgerLookupLedger,
        keylet: protocol::Keylet,
    ) -> Option<STLedgerEntry> {
        self.lookup_resolved_ledger(ledger)
            .and_then(|resolved| resolved.read(keylet).ok().flatten())
    }
}

impl LedgerLookupSource for ApplicationAccountInfoSource<'_> {
    fn get_ledger_by_hash(&self, hash: basics::base_uint::Uint256) -> Option<LedgerLookupLedger> {
        self.current_lookup_ledger()
            .filter(|ledger| ledger.hash == hash)
            .or_else(|| {
                self.app
                    .validated_ledger()
                    .map(|ledger| ledger_lookup_ledger(ledger.as_ref(), false))
                    .filter(|ledger| ledger.hash == hash)
            })
            .or_else(|| {
                self.app
                    .published_ledger()
                    .map(|ledger| ledger_lookup_ledger(ledger.as_ref(), false))
                    .filter(|ledger| ledger.hash == hash)
            })
            .or_else(|| {
                self.app
                    .closed_ledger()
                    .map(|ledger| ledger_lookup_ledger(ledger.as_ref(), false))
                    .filter(|ledger| ledger.hash == hash)
            })
    }

    fn get_ledger_by_seq(&self, seq: u32) -> Option<LedgerLookupLedger> {
        self.current_lookup_ledger()
            .filter(|ledger| ledger.seq == seq)
            .or_else(|| {
                self.app
                    .validated_ledger()
                    .map(|ledger| ledger_lookup_ledger(ledger.as_ref(), false))
                    .filter(|ledger| ledger.seq == seq)
            })
            .or_else(|| {
                self.app
                    .published_ledger()
                    .map(|ledger| ledger_lookup_ledger(ledger.as_ref(), false))
                    .filter(|ledger| ledger.seq == seq)
            })
            .or_else(|| {
                self.app
                    .closed_ledger()
                    .map(|ledger| ledger_lookup_ledger(ledger.as_ref(), false))
                    .filter(|ledger| ledger.seq == seq)
            })
    }

    fn get_current_ledger(&self) -> Option<LedgerLookupLedger> {
        self.current_lookup_ledger()
    }

    fn get_closed_ledger(&self) -> Option<LedgerLookupLedger> {
        self.app
            .closed_ledger()
            .map(|ledger| ledger_lookup_ledger(ledger.as_ref(), false))
    }

    fn get_validated_ledger(&self) -> Option<LedgerLookupLedger> {
        self.app
            .validated_ledger()
            .map(|ledger| ledger_lookup_ledger(ledger.as_ref(), false))
    }

    fn get_valid_ledger_index(&self) -> u32 {
        self.app
            .validated_ledger_seq()
            .or_else(|| self.app.published_ledger_seq())
            .or_else(|| self.app.closed_ledger_seq())
            .or_else(|| self.current_lookup_ledger().map(|ledger| ledger.seq))
            .unwrap_or_default()
    }

    fn get_validated_ledger_age(&self) -> std::time::Duration {
        self.app.validated_ledger_age()
    }

    fn is_validated(&self, ledger: &LedgerLookupLedger) -> bool {
        !ledger.open
            && self
                .app
                .validated_ledger()
                .is_some_and(|validated| ledger_lookup_ledger(validated.as_ref(), false) == *ledger)
    }
}

impl AccountInfoSource for ApplicationAccountInfoSource<'_> {
    fn read_account_root(
        &self,
        ledger: &LedgerLookupLedger,
        account_id: AccountID,
    ) -> Option<STLedgerEntry> {
        self.read_entry(
            ledger,
            account_keylet(Uint160::from_slice(account_id.data()).expect("account width")),
        )
    }

    fn read_signer_list(
        &self,
        ledger: &LedgerLookupLedger,
        account_id: AccountID,
    ) -> Option<STLedgerEntry> {
        self.read_entry(
            ledger,
            signers_keylet(Uint160::from_slice(account_id.data()).expect("account width")),
        )
    }

    fn feature_clawback_enabled(&self, ledger: &LedgerLookupLedger) -> bool {
        self.lookup_resolved_ledger(ledger)
            .is_some_and(|resolved| resolved.rules().enabled(&feature_clawback()))
    }

    fn feature_token_escrow_enabled(&self, ledger: &LedgerLookupLedger) -> bool {
        self.lookup_resolved_ledger(ledger)
            .is_some_and(|resolved| resolved.rules().enabled(&feature_token_escrow()))
    }

    fn account_queue_txs(
        &self,
        ledger: &LedgerLookupLedger,
        account_id: AccountID,
    ) -> Vec<AccountQueueTransaction> {
        if self.current_lookup_ledger() != Some(*ledger) {
            return Vec::new();
        }

        self.app
            .tx_q_account_txs(account_id)
            .into_iter()
            .map(Into::into)
            .collect()
    }
}

fn ensure_object(value: &mut JsonValue) -> &mut BTreeMap<String, JsonValue> {
    if !matches!(value, JsonValue::Object(_)) {
        *value = JsonValue::Object(BTreeMap::new());
    }

    let JsonValue::Object(object) = value else {
        unreachable!("json value should be an object");
    };
    object
}

fn parse_ident(params: &JsonValue) -> Result<String, RpcStatus> {
    let JsonValue::Object(object) = params else {
        return Err(RpcStatus::missing_field_error("account"));
    };

    if let Some(account) = object.get("account") {
        let JsonValue::String(account) = account else {
            return Err(RpcStatus::invalid_field_error("account"));
        };
        return Ok(account.clone());
    }

    if let Some(ident) = object.get("ident") {
        let JsonValue::String(ident) = ident else {
            return Err(RpcStatus::invalid_field_error("ident"));
        };
        return Ok(ident.clone());
    }

    Err(RpcStatus::missing_field_error("account"))
}

fn queue_requested(params: &JsonValue) -> bool {
    matches!(
        params,
        JsonValue::Object(object) if matches!(object.get("queue"), Some(JsonValue::Bool(true)))
    )
}

fn signer_lists_requested(params: &JsonValue, api_version: u32) -> Result<bool, RpcStatus> {
    let JsonValue::Object(object) = params else {
        return Ok(false);
    };

    let Some(value) = object.get("signer_lists") else {
        return Ok(false);
    };

    match value {
        JsonValue::Bool(flag) => Ok(*flag),
        _ if api_version > 1 => Err(RpcStatus::new(RpcErrorCode::InvalidParams)),
        _ => Ok(false),
    }
}

fn insert_gravatar_if_present(
    account_data: &mut BTreeMap<String, JsonValue>,
    account_root: &AccountRoot,
) {
    let email_hash_field = protocol::get_field_by_symbol("sfEmailHash");
    if !account_root
        .as_st_ledger_entry()
        .is_field_present(email_hash_field)
    {
        return;
    }

    let mut md5 = str_hex(
        account_root
            .as_st_ledger_entry()
            .get_field_h128(email_hash_field)
            .data(),
    );
    md5.make_ascii_lowercase();
    account_data.insert(
        "urlgravatar".to_owned(),
        JsonValue::String(format!("http://www.gravatar.com/avatar/{md5}")),
    );
}

fn inject_account_data(account_root: &AccountRoot) -> JsonValue {
    let mut account_data = match account_root.as_st_ledger_entry().json(JsonOptions::NONE) {
        JsonValue::Object(object) => object,
        _ => BTreeMap::new(),
    };

    if account_root.get_type() == LedgerEntryType::AccountRoot {
        insert_gravatar_if_present(&mut account_data, account_root);
    } else {
        account_data.insert("Invalid".to_owned(), JsonValue::Bool(true));
    }

    JsonValue::Object(account_data)
}

fn insert_flag(
    account_flags: &mut BTreeMap<String, JsonValue>,
    name: &str,
    sle: &STLedgerEntry,
    flag: u32,
) {
    account_flags.insert(name.to_owned(), JsonValue::Bool(sle.is_flag(flag)));
}

fn build_account_flags<S: AccountInfoSource>(
    source: &S,
    ledger: &LedgerLookupLedger,
    account_root: &AccountRoot,
) -> JsonValue {
    let mut account_flags = BTreeMap::new();
    let sle = account_root.as_st_ledger_entry();

    for (name, flag) in [
        ("defaultRipple", lsfDefaultRipple),
        ("depositAuth", lsfDepositAuth),
        ("disableMasterKey", lsfDisableMaster),
        ("disallowIncomingXRP", lsfDisallowXRP),
        ("globalFreeze", lsfGlobalFreeze),
        ("noFreeze", lsfNoFreeze),
        ("passwordSpent", lsfPasswordSpent),
        ("requireAuthorization", lsfRequireAuth),
        ("requireDestinationTag", lsfRequireDestTag),
        (
            "disallowIncomingNFTokenOffer",
            lsfDisallowIncomingNFTokenOffer,
        ),
        ("disallowIncomingCheck", lsfDisallowIncomingCheck),
        ("disallowIncomingPayChan", lsfDisallowIncomingPayChan),
        ("disallowIncomingTrustline", lsfDisallowIncomingTrustline),
    ] {
        insert_flag(&mut account_flags, name, sle, flag);
    }

    if source.feature_clawback_enabled(ledger) {
        insert_flag(
            &mut account_flags,
            "allowTrustLineClawback",
            sle,
            lsfAllowTrustLineClawback,
        );
    }

    if source.feature_token_escrow_enabled(ledger) {
        insert_flag(
            &mut account_flags,
            "allowTrustLineLocking",
            sle,
            lsfAllowTrustLineLocking,
        );
    }

    JsonValue::Object(account_flags)
}

fn ledger_lookup_ledger(ledger: &Ledger, open: bool) -> LedgerLookupLedger {
    LedgerLookupLedger {
        hash: *ledger.header().hash.as_uint256(),
        seq: ledger.header().seq,
        open,
    }
}

fn pseudo_account_type(account_root: &AccountRoot) -> Option<String> {
    let format = LedgerFormats::get_instance()
        .find_by_type(LedgerEntryType::AccountRoot)
        .expect("account root format must exist");

    for field in format.so_template().iter().map(|element| element.sfield()) {
        if !field.should_meta(SField::S_MD_PSEUDO_ACCOUNT)
            || !account_root.as_st_ledger_entry().is_field_present(field)
        {
            continue;
        }

        let mut name = field.name().to_owned();
        if name.ends_with("ID") {
            name.truncate(name.len().saturating_sub(2));
        }
        return Some(name);
    }

    None
}

fn build_queue_data(queue_txs: Vec<AccountQueueTransaction>) -> JsonValue {
    let mut queue_data = BTreeMap::new();
    queue_data.insert(
        "txn_count".to_owned(),
        JsonValue::Unsigned(u64::try_from(queue_txs.len()).unwrap_or(u64::MAX)),
    );

    if queue_txs.is_empty() {
        return JsonValue::Object(queue_data);
    }

    let mut queue_entries = Vec::with_capacity(queue_txs.len());
    let mut seq_count = 0_u64;
    let mut ticket_count = 0_u64;
    let mut lowest_seq: Option<u32> = None;
    let mut highest_seq: Option<u32> = None;
    let mut lowest_ticket: Option<u32> = None;
    let mut highest_ticket: Option<u32> = None;
    let mut any_auth_changed = false;
    let mut max_spend_drops_total = 0_u64;

    for tx in queue_txs {
        let mut tx_json = BTreeMap::new();
        if tx.seq_proxy.is_seq() {
            let seq = tx.seq_proxy.value();
            seq_count = seq_count.saturating_add(1);
            lowest_seq = Some(lowest_seq.map_or(seq, |current| current.min(seq)));
            highest_seq = Some(highest_seq.map_or(seq, |current| current.max(seq)));
            tx_json.insert("seq".to_owned(), JsonValue::Unsigned(u64::from(seq)));
        } else {
            let ticket = tx.seq_proxy.value();
            ticket_count = ticket_count.saturating_add(1);
            lowest_ticket = Some(lowest_ticket.map_or(ticket, |current| current.min(ticket)));
            highest_ticket = Some(highest_ticket.map_or(ticket, |current| current.max(ticket)));
            tx_json.insert("ticket".to_owned(), JsonValue::Unsigned(u64::from(ticket)));
        }

        tx_json.insert(
            "fee_level".to_owned(),
            JsonValue::String(tx.fee_level.to_string()),
        );
        if let Some(last_valid) = tx.last_valid {
            tx_json.insert(
                "LastLedgerSequence".to_owned(),
                JsonValue::Unsigned(u64::from(last_valid)),
            );
        }
        tx_json.insert(
            "fee".to_owned(),
            JsonValue::String(tx.fee_drops.to_string()),
        );
        tx_json.insert(
            "max_spend_drops".to_owned(),
            JsonValue::String(tx.max_spend_drops.to_string()),
        );
        tx_json.insert("auth_change".to_owned(), JsonValue::Bool(tx.auth_change));

        any_auth_changed |= tx.auth_change;
        max_spend_drops_total = max_spend_drops_total.saturating_add(tx.max_spend_drops);
        queue_entries.push(JsonValue::Object(tx_json));
    }

    queue_data.insert("transactions".to_owned(), JsonValue::Array(queue_entries));
    if seq_count != 0 {
        queue_data.insert("sequence_count".to_owned(), JsonValue::Unsigned(seq_count));
    }
    if ticket_count != 0 {
        queue_data.insert("ticket_count".to_owned(), JsonValue::Unsigned(ticket_count));
    }
    if let Some(lowest_seq) = lowest_seq {
        queue_data.insert(
            "lowest_sequence".to_owned(),
            JsonValue::Unsigned(u64::from(lowest_seq)),
        );
    }
    if let Some(highest_seq) = highest_seq {
        queue_data.insert(
            "highest_sequence".to_owned(),
            JsonValue::Unsigned(u64::from(highest_seq)),
        );
    }
    if let Some(lowest_ticket) = lowest_ticket {
        queue_data.insert(
            "lowest_ticket".to_owned(),
            JsonValue::Unsigned(u64::from(lowest_ticket)),
        );
    }
    if let Some(highest_ticket) = highest_ticket {
        queue_data.insert(
            "highest_ticket".to_owned(),
            JsonValue::Unsigned(u64::from(highest_ticket)),
        );
    }
    queue_data.insert(
        "auth_change_queued".to_owned(),
        JsonValue::Bool(any_auth_changed),
    );
    queue_data.insert(
        "max_spend_drops_total".to_owned(),
        JsonValue::String(max_spend_drops_total.to_string()),
    );

    JsonValue::Object(queue_data)
}

pub fn do_account_info<S: AccountInfoSource>(
    request: &AccountInfoRequest<'_>,
    source: &S,
) -> JsonValue {
    let ident = match parse_ident(request.params) {
        Ok(ident) => {
            tracing::trace!(target: "rpc", account = %ident, "account_info query");
            ident
        }
        Err(status) => {
            let mut error = JsonValue::Object(BTreeMap::new());
            status.inject(&mut error);
            return error;
        }
    };

    let context = LedgerLookupContext {
        params: request.params,
        source,
        api_version: request.api_version,
        role: request.role,
    };

    let (ledger, mut result) = match lookup_ledger_with_result(&context) {
        Ok(result) => result,
        Err(status) => {
            let mut error = JsonValue::Object(BTreeMap::new());
            status.inject(&mut error);
            return error;
        }
    };

    let Some(account_id) = parse_base58_account_id(&ident) else {
        inject_error(RpcErrorCode::ActMalformed, &mut result);
        return result;
    };

    let queue = queue_requested(request.params);
    if queue && !ledger.open {
        inject_error(RpcErrorCode::InvalidParams, &mut result);
        return result;
    }

    let signer_lists = match signer_lists_requested(request.params, request.api_version) {
        Ok(signer_lists) => signer_lists,
        Err(status) => {
            status.inject(&mut result);
            return result;
        }
    };

    let Some(account_root) = source.read_account_root(&ledger, account_id) else {
        ensure_object(&mut result).insert(
            "account".to_owned(),
            JsonValue::String(to_base58(account_id)),
        );
        inject_error(RpcErrorCode::ActNotFound, &mut result);
        return result;
    };
    let account_root = AccountRoot::new(Arc::new(account_root))
        .expect("account root entry should match the AccountRoot wrapper");

    let object = ensure_object(&mut result);
    object.insert(
        "account_data".to_owned(),
        inject_account_data(&account_root),
    );
    object.insert(
        "account_flags".to_owned(),
        build_account_flags(source, &ledger, &account_root),
    );
    if let Some(pseudo_type) = pseudo_account_type(&account_root) {
        object.insert(
            "pseudo_account".to_owned(),
            JsonValue::Object(BTreeMap::from([(
                "type".to_owned(),
                JsonValue::String(pseudo_type),
            )])),
        );
    }

    if signer_lists {
        let signer_lists_value = match source.read_signer_list(&ledger, account_id) {
            Some(signer_list) => {
                let signer_list = SignerList::new(Arc::new(signer_list))
                    .expect("signer list entry should match the SignerList wrapper");
                JsonValue::Array(vec![
                    signer_list.as_st_ledger_entry().json(JsonOptions::NONE),
                ])
            }
            None => JsonValue::Array(Vec::new()),
        };

        if request.api_version == 1 {
            let JsonValue::Object(account_data) = object
                .entry("account_data".to_owned())
                .or_insert_with(|| JsonValue::Object(BTreeMap::new()))
            else {
                unreachable!("account_data should remain an object");
            };
            account_data.insert("signer_lists".to_owned(), signer_lists_value);
        } else {
            object.insert("signer_lists".to_owned(), signer_lists_value);
        }
    }

    if queue {
        object.insert(
            "queue_data".to_owned(),
            build_queue_data(source.account_queue_txs(&ledger, account_id)),
        );
    }

    result
}
