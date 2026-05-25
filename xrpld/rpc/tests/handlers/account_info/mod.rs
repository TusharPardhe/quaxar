//! Tests for the account_info RPC handler.

//! Tests for the account info RPC handler.

use std::{collections::HashMap, sync::Arc, time::Duration};

use app::ApplicationRoot;
use basics::base_uint::{Uint160, Uint256};
use basics::chrono::EPOCH_OFFSET_SECONDS;
use ledger::{LEDGER_DEFAULT_TIME_RESOLUTION, Ledger, LedgerHeader};
use protocol::{
    AccountID, JsonValue, LedgerEntryType, Rules, STArray, STLedgerEntry, STObject, SeqProxy,
    account_keylet, feature_clawback, feature_token_escrow, get_field_by_symbol,
    lsfAllowTrustLineClawback, lsfAllowTrustLineLocking, lsfDefaultRipple,
    lsfDisallowIncomingCheck, lsfDisallowIncomingTrustline, signers_keylet, to_base58,
};
use rpc::Role;
use rpc::{
    AccountInfoRequest, AccountInfoSource, AccountQueueTransaction, ApplicationAccountInfoSource,
    do_account_info,
};
use rpc::{LedgerLookupLedger, LedgerLookupSource};
use shamap::item::SHAMapItem;
use shamap::mutation::MutableTree;
use shamap::sync::{SHAMapType, SyncState, SyncTree};
use shamap::tree_node::SHAMapNodeType;

#[derive(Debug, Default)]
struct FakeSource {
    ledger: Option<LedgerLookupLedger>,
    account_roots: HashMap<AccountID, STLedgerEntry>,
    signer_lists: HashMap<AccountID, STLedgerEntry>,
    queue_txs: HashMap<AccountID, Vec<AccountQueueTransaction>>,
    clawback_enabled: bool,
    token_escrow_enabled: bool,
}

impl LedgerLookupSource for FakeSource {
    fn get_ledger_by_hash(&self, hash: Uint256) -> Option<LedgerLookupLedger> {
        self.ledger.filter(|ledger| ledger.hash == hash)
    }

    fn get_ledger_by_seq(&self, seq: u32) -> Option<LedgerLookupLedger> {
        self.ledger.filter(|ledger| ledger.seq == seq)
    }

    fn get_current_ledger(&self) -> Option<LedgerLookupLedger> {
        self.ledger
    }

    fn get_closed_ledger(&self) -> Option<LedgerLookupLedger> {
        self.ledger.filter(|ledger| !ledger.open)
    }

    fn get_validated_ledger(&self) -> Option<LedgerLookupLedger> {
        self.ledger.filter(|ledger| !ledger.open)
    }

    fn get_valid_ledger_index(&self) -> u32 {
        self.ledger.map(|ledger| ledger.seq).unwrap_or_default()
    }

    fn get_validated_ledger_age(&self) -> Duration {
        Duration::from_secs(1)
    }

    fn is_validated(&self, ledger: &LedgerLookupLedger) -> bool {
        !ledger.open && self.ledger == Some(*ledger)
    }
}

impl AccountInfoSource for FakeSource {
    fn read_account_root(
        &self,
        _ledger: &LedgerLookupLedger,
        account_id: AccountID,
    ) -> Option<STLedgerEntry> {
        self.account_roots.get(&account_id).cloned()
    }

    fn read_signer_list(
        &self,
        _ledger: &LedgerLookupLedger,
        account_id: AccountID,
    ) -> Option<STLedgerEntry> {
        self.signer_lists.get(&account_id).cloned()
    }

    fn feature_clawback_enabled(&self, _ledger: &LedgerLookupLedger) -> bool {
        self.clawback_enabled
    }

    fn feature_token_escrow_enabled(&self, _ledger: &LedgerLookupLedger) -> bool {
        self.token_escrow_enabled
    }

    fn account_queue_txs(
        &self,
        _ledger: &LedgerLookupLedger,
        account_id: AccountID,
    ) -> Vec<AccountQueueTransaction> {
        self.queue_txs.get(&account_id).cloned().unwrap_or_default()
    }
}

pub(super) fn sample_account(fill: u8) -> AccountID {
    AccountID::from_array([fill; 20])
}

pub(super) fn sample_hash(fill: u8) -> Uint256 {
    Uint256::from_array([fill; 32])
}

pub(super) fn object(params: impl IntoIterator<Item = (&'static str, JsonValue)>) -> JsonValue {
    JsonValue::Object(
        params
            .into_iter()
            .map(|(key, value)| (key.to_owned(), value))
            .collect(),
    )
}

pub(super) fn make_account_root(
    account: AccountID,
    flags: u32,
    email_hash: Option<[u8; 16]>,
) -> STLedgerEntry {
    let account_key = Uint160::from_slice(account.data()).expect("account width");
    let mut sle = STLedgerEntry::from_type_and_key(
        LedgerEntryType::AccountRoot,
        account_keylet(account_key).key,
    );
    sle.set_account_id(get_field_by_symbol("sfAccount"), account);
    sle.set_field_u32(get_field_by_symbol("sfFlags"), flags);
    sle.set_field_u32(get_field_by_symbol("sfSequence"), 7);
    sle.set_field_u32(get_field_by_symbol("sfOwnerCount"), 2);
    sle.set_field_u32(get_field_by_symbol("sfPreviousTxnLgrSeq"), 1);
    sle.set_field_h256(get_field_by_symbol("sfPreviousTxnID"), sample_hash(0x44));
    sle.set_field_amount(
        get_field_by_symbol("sfBalance"),
        protocol::STAmount::new_native(1_000_000, false),
    );
    if let Some(email_hash) = email_hash {
        sle.set_field_h128(
            get_field_by_symbol("sfEmailHash"),
            basics::base_uint::Uint128::from_array(email_hash),
        );
    }
    sle
}

pub(super) fn make_signer_list(account: AccountID, signer: AccountID) -> STLedgerEntry {
    let account_key = Uint160::from_slice(account.data()).expect("account width");
    let mut sle = STLedgerEntry::new(signers_keylet(account_key));
    sle.set_field_u32(get_field_by_symbol("sfSignerQuorum"), 2);

    let mut entry = STObject::make_inner_object(get_field_by_symbol("sfSignerEntry"));
    entry.set_account_id(get_field_by_symbol("sfAccount"), signer);
    entry.set_field_u16(get_field_by_symbol("sfSignerWeight"), 3);

    let mut array = STArray::new(get_field_by_symbol("sfSignerEntries"));
    array.push_back(entry);
    sle.set_field_array(get_field_by_symbol("sfSignerEntries"), array);
    sle
}

pub(super) fn make_pseudo_account_root(
    account: AccountID,
    pseudo_field_symbol: &'static str,
) -> STLedgerEntry {
    let mut sle = make_account_root(account, 0, None);
    sle.set_field_h256(get_field_by_symbol(pseudo_field_symbol), sample_hash(0x77));
    sle
}

pub(super) fn ledger_with_state_entries(
    seq: u32,
    close_time: u32,
    entries: impl IntoIterator<Item = STLedgerEntry>,
    rules: Rules,
) -> Arc<Ledger> {
    let mut state_tree = MutableTree::new(1);
    for entry in entries {
        state_tree
            .add_item(
                SHAMapNodeType::AccountState,
                SHAMapItem::new(*entry.key(), entry.get_serializer().data().to_vec()),
            )
            .expect("state entry should insert");
    }

    let mut ledger = Ledger::from_maps(
        LedgerHeader {
            seq,
            close_time,
            close_time_resolution: LEDGER_DEFAULT_TIME_RESOLUTION,
            ..LedgerHeader::default()
        },
        SyncTree::from_root_with_type(
            state_tree.root(),
            SHAMapType::State,
            false,
            seq,
            SyncState::Immutable,
        ),
        SyncTree::new_with_type(SHAMapType::Transaction, false, seq),
    );
    ledger.set_rules(rules);
    Arc::new(ledger)
}

pub(super) fn closed_ledger() -> LedgerLookupLedger {
    LedgerLookupLedger {
        hash: sample_hash(0xAB),
        seq: 91,
        open: false,
    }
}

pub(super) fn open_ledger() -> LedgerLookupLedger {
    LedgerLookupLedger {
        hash: sample_hash(0xAC),
        seq: 92,
        open: true,
    }
}

pub(super) fn current_net_close_time() -> u32 {
    let unix_now = time::OffsetDateTime::now_utc().unix_timestamp();
    let net_now = unix_now.saturating_sub(EPOCH_OFFSET_SECONDS);
    u32::try_from(net_now).unwrap_or_default()
}

mod rendering;
mod source;
mod validation;
