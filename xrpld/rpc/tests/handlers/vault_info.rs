//! Tests for the vault info RPC handler.

use std::{collections::BTreeMap, time::Duration};

use basics::base_uint::{Uint160, Uint192, Uint256};
use protocol::{
    AccountID, Asset, Currency, Issue, JsonValue, LedgerEntryType, STIssue, STLedgerEntry, StBase,
    get_field_by_symbol, mpt_issuance_keylet_from_mptid, to_base58, vault_keylet,
};
use rpc::{
    LedgerLookupLedger, LedgerLookupSource, RpcRole, VaultInfoRequest, VaultInfoSource,
    do_vault_info,
};

#[derive(Debug, Default)]
struct FakeSource {
    ledger: Option<LedgerLookupLedger>,
    entries: BTreeMap<Uint256, STLedgerEntry>,
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

impl VaultInfoSource for FakeSource {
    fn read_ledger_entry(
        &self,
        _ledger: &LedgerLookupLedger,
        entry_index: Uint256,
    ) -> Option<STLedgerEntry> {
        self.entries.get(&entry_index).cloned()
    }
}

fn object(entries: impl IntoIterator<Item = (&'static str, JsonValue)>) -> JsonValue {
    JsonValue::Object(
        entries
            .into_iter()
            .map(|(key, value)| (key.to_owned(), value))
            .collect::<BTreeMap<_, _>>(),
    )
}

fn request(params: JsonValue) -> VaultInfoRequest<'static> {
    let params = Box::leak(Box::new(params));
    VaultInfoRequest {
        params,
        api_version: 2,
        role: RpcRole::User,
    }
}

fn json_object(value: &JsonValue) -> &BTreeMap<String, JsonValue> {
    let JsonValue::Object(object) = value else {
        panic!("expected object");
    };
    object
}

fn error_string(value: &JsonValue) -> Option<&str> {
    json_object(value)
        .get("error")
        .and_then(|value| match value {
            JsonValue::String(text) => Some(text.as_str()),
            _ => None,
        })
}

fn sample_account(fill: u8) -> AccountID {
    AccountID::from_array([fill; 20])
}

fn sample_uint192(fill: u8) -> Uint192 {
    Uint192::from_array([fill; 24])
}

fn sample_uint256(fill: u8) -> Uint256 {
    Uint256::from_array([fill; 32])
}

fn closed_ledger() -> LedgerLookupLedger {
    LedgerLookupLedger {
        hash: sample_uint256(0xAB),
        seq: 91,
        open: false,
    }
}

fn vault_key_hex(key: Uint256) -> String {
    key.to_string()
}

fn make_vault_entry(
    owner: AccountID,
    sequence: u32,
    share_id: Uint192,
    asset_currency: Currency,
) -> STLedgerEntry {
    let key = vault_keylet(
        Uint160::from_slice(owner.data()).expect("account width"),
        sequence,
    )
    .key;
    let mut vault = STLedgerEntry::from_type_and_key(LedgerEntryType::Vault, key);
    vault.set_field_u32(get_field_by_symbol("sfFlags"), 0);
    vault.set_field_h256(get_field_by_symbol("sfPreviousTxnID"), sample_uint256(0x11));
    vault.set_field_u32(get_field_by_symbol("sfPreviousTxnLgrSeq"), 17);
    vault.set_field_u32(get_field_by_symbol("sfSequence"), sequence);
    vault.set_field_u64(get_field_by_symbol("sfOwnerNode"), 0);
    vault.set_account_id(get_field_by_symbol("sfOwner"), owner);
    vault.set_account_id(get_field_by_symbol("sfAccount"), owner);
    vault.set_field_issue(
        get_field_by_symbol("sfAsset"),
        STIssue::new_with_asset(
            get_field_by_symbol("sfAsset"),
            Asset::Issue(Issue::new(asset_currency, owner)),
        ),
    );
    vault.set_field_h192(get_field_by_symbol("sfShareMPTID"), share_id);
    vault.set_field_u8(get_field_by_symbol("sfWithdrawalPolicy"), 0);
    vault
}

fn make_issuance_entry(issuer: AccountID, sequence: u32, share_id: Uint192) -> STLedgerEntry {
    let key = mpt_issuance_keylet_from_mptid(share_id).key;
    let mut issuance = STLedgerEntry::from_type_and_key(LedgerEntryType::MPTokenIssuance, key);
    issuance.set_field_u32(get_field_by_symbol("sfFlags"), 0);
    issuance.set_account_id(get_field_by_symbol("sfIssuer"), issuer);
    issuance.set_field_u32(get_field_by_symbol("sfSequence"), sequence);
    issuance.set_field_u64(get_field_by_symbol("sfOwnerNode"), 0);
    issuance.set_field_u64(get_field_by_symbol("sfOutstandingAmount"), 0);
    issuance.set_field_h256(get_field_by_symbol("sfPreviousTxnID"), sample_uint256(0x22));
    issuance.set_field_u32(get_field_by_symbol("sfPreviousTxnLgrSeq"), 17);
    issuance
}

fn response_fields(value: &JsonValue) -> &BTreeMap<String, JsonValue> {
    json_object(value)
}

#[test]
fn vault_info_rejects_cpp_style_malformed_combinations() {
    let source = FakeSource {
        ledger: Some(closed_ledger()),
        ..Default::default()
    };

    let cases = [
        object([("ledger_index", JsonValue::String("validated".to_owned()))]),
        object([
            ("ledger_index", JsonValue::String("validated".to_owned())),
            ("vault_id", JsonValue::String("foobar".to_owned())),
        ]),
        object([
            ("ledger_index", JsonValue::String("validated".to_owned())),
            ("vault_id", JsonValue::String("0".repeat(64))),
        ]),
        object([
            ("ledger_index", JsonValue::String("validated".to_owned())),
            ("vault_id", JsonValue::Unsigned(0)),
        ]),
        object([
            ("ledger_index", JsonValue::String("validated".to_owned())),
            ("owner", JsonValue::String("foobar".to_owned())),
            ("seq", JsonValue::Unsigned(7)),
        ]),
        object([
            ("ledger_index", JsonValue::String("validated".to_owned())),
            ("owner", JsonValue::String(to_base58(sample_account(0x11)))),
        ]),
        object([
            ("ledger_index", JsonValue::String("validated".to_owned())),
            ("seq", JsonValue::Unsigned(7)),
        ]),
        object([
            ("ledger_index", JsonValue::String("validated".to_owned())),
            ("owner", JsonValue::String(to_base58(sample_account(0x11)))),
            ("seq", JsonValue::String("nope".to_owned())),
        ]),
        object([
            ("ledger_index", JsonValue::String("validated".to_owned())),
            ("owner", JsonValue::String(to_base58(sample_account(0x11)))),
            ("seq", JsonValue::Bool(true)),
        ]),
        object([
            ("ledger_index", JsonValue::String("validated".to_owned())),
            ("owner", JsonValue::String(to_base58(sample_account(0x11)))),
            ("seq", JsonValue::Signed(-1)),
        ]),
        object([
            ("ledger_index", JsonValue::String("validated".to_owned())),
            ("owner", JsonValue::String(to_base58(sample_account(0x11)))),
            ("seq", JsonValue::Unsigned(0)),
        ]),
        object([
            ("ledger_index", JsonValue::String("validated".to_owned())),
            ("vault_id", JsonValue::String("0".repeat(64))),
            ("owner", JsonValue::String(to_base58(sample_account(0x11)))),
        ]),
    ];

    for params in cases {
        let result = do_vault_info(&request(params), &source);
        assert_eq!(error_string(&result), Some("malformedRequest"));
    }
}

#[test]
fn vault_info_returns_entry_not_found() {
    let source = FakeSource {
        ledger: Some(closed_ledger()),
        ..Default::default()
    };
    let vault_key = sample_uint256(0x44);

    let result = do_vault_info(
        &request(object([
            ("ledger_index", JsonValue::String("validated".to_owned())),
            ("vault_id", JsonValue::String(vault_key_hex(vault_key))),
        ])),
        &source,
    );

    assert_eq!(error_string(&result), Some("entryNotFound"));
}

#[test]
fn vault_info_shapes_direct_and_owner_lookup() {
    let owner = sample_account(0x11);
    let share_id = sample_uint192(0x33);
    let asset_currency = Currency::from_array([0x77; 20]);
    let vault = make_vault_entry(owner, 7, share_id, asset_currency);
    let issuance = make_issuance_entry(owner, 9, share_id);
    let vault_key = *vault.key();
    let issuance_key = *issuance.key();
    let mut source = FakeSource {
        ledger: Some(closed_ledger()),
        ..Default::default()
    };
    source.entries.insert(vault_key, vault.clone());
    source.entries.insert(issuance_key, issuance.clone());

    let direct = do_vault_info(
        &request(object([
            ("ledger_index", JsonValue::String("validated".to_owned())),
            ("vault_id", JsonValue::String(vault_key_hex(vault_key))),
        ])),
        &source,
    );
    assert!(error_string(&direct).is_none());
    let direct_fields = response_fields(&direct);
    let JsonValue::Object(vault_json) = direct_fields.get("vault").expect("vault object") else {
        panic!("vault must be an object");
    };
    assert_eq!(
        vault_json.get("index"),
        Some(&JsonValue::String(vault_key.to_string()))
    );
    assert_eq!(
        vault_json.get("shares"),
        Some(&issuance.json(protocol::JsonOptions::NONE))
    );

    let owner_lookup = do_vault_info(
        &request(object([
            ("ledger_index", JsonValue::String("validated".to_owned())),
            ("owner", JsonValue::String(to_base58(owner))),
            ("seq", JsonValue::Unsigned(7)),
        ])),
        &source,
    );
    assert!(error_string(&owner_lookup).is_none());
    let owner_fields = response_fields(&owner_lookup);
    let JsonValue::Object(vault_json) = owner_fields.get("vault").expect("vault object") else {
        panic!("vault must be an object");
    };
    assert_eq!(
        vault_json.get("shares"),
        Some(&issuance.json(protocol::JsonOptions::NONE))
    );
}
