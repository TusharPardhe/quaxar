//! Tests for the deposit authorized RPC handler.

use std::{collections::BTreeMap, time::Duration};

use basics::base_uint::{Uint160, Uint256};
use protocol::{
    AccountID, JsonValue, LedgerEntryType, STLedgerEntry, account_keylet, credential_keylet,
    deposit_preauth_credentials_keylet, deposit_preauth_keylet, get_field_by_symbol, lsfAccepted,
    lsfDepositAuth, to_base58,
};
use rpc::{
    DepositAuthorizedRequest, DepositAuthorizedSource, LedgerLookupLedger, LedgerLookupSource,
    RpcRole, do_deposit_authorized,
};
use sha2::Digest;

#[derive(Debug, Default)]
struct FakeSource {
    ledger: Option<LedgerLookupLedger>,
    account_roots: BTreeMap<AccountID, STLedgerEntry>,
    entries: BTreeMap<Uint256, STLedgerEntry>,
    parent_close_time: u32,
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

impl DepositAuthorizedSource for FakeSource {
    fn read_account_root(
        &self,
        _ledger: &LedgerLookupLedger,
        account_id: AccountID,
    ) -> Option<STLedgerEntry> {
        self.account_roots.get(&account_id).cloned()
    }

    fn read_ledger_entry(
        &self,
        _ledger: &LedgerLookupLedger,
        entry_index: Uint256,
    ) -> Option<STLedgerEntry> {
        self.entries.get(&entry_index).cloned()
    }

    fn parent_close_time(&self, _ledger: &LedgerLookupLedger) -> u32 {
        self.parent_close_time
    }
}

fn sample_account(fill: u8) -> AccountID {
    AccountID::from_array([fill; 20])
}

fn sample_hash(fill: u8) -> Uint256 {
    Uint256::from_array([fill; 32])
}

fn object(params: impl IntoIterator<Item = (&'static str, JsonValue)>) -> JsonValue {
    JsonValue::Object(
        params
            .into_iter()
            .map(|(key, value)| (key.to_owned(), value))
            .collect(),
    )
}

fn closed_ledger() -> LedgerLookupLedger {
    LedgerLookupLedger {
        hash: sample_hash(0xAA),
        seq: 100,
        open: false,
    }
}

fn make_account_root(account: AccountID, deposit_auth: bool) -> STLedgerEntry {
    let account_key = Uint160::from_slice(account.data()).expect("account width");
    let mut sle = STLedgerEntry::from_type_and_key(
        LedgerEntryType::AccountRoot,
        account_keylet(account_key).key,
    );
    sle.set_account_id(get_field_by_symbol("sfAccount"), account);
    if deposit_auth {
        sle.set_field_u32(get_field_by_symbol("sfFlags"), lsfDepositAuth);
    }
    sle
}

fn make_direct_preauth(dst: AccountID, src: AccountID) -> STLedgerEntry {
    let dst_key = Uint160::from_slice(dst.data()).expect("account width");
    let src_key = Uint160::from_slice(src.data()).expect("account width");
    STLedgerEntry::new(deposit_preauth_keylet(dst_key, src_key))
}

fn make_credential(
    subject: AccountID,
    issuer: AccountID,
    credential_type: &[u8],
    accepted: bool,
    expiration: Option<u32>,
) -> STLedgerEntry {
    let subject_key = Uint160::from_slice(subject.data()).expect("account width");
    let issuer_key = Uint160::from_slice(issuer.data()).expect("account width");
    let mut sle = STLedgerEntry::new(credential_keylet(subject_key, issuer_key, credential_type));
    sle.set_account_id(get_field_by_symbol("sfSubject"), subject);
    sle.set_account_id(get_field_by_symbol("sfIssuer"), issuer);
    sle.set_field_vl(get_field_by_symbol("sfCredentialType"), credential_type);
    if accepted {
        sle.set_field_u32(get_field_by_symbol("sfFlags"), lsfAccepted);
    }
    if let Some(expiration) = expiration {
        sle.set_field_u32(get_field_by_symbol("sfExpiration"), expiration);
    }
    sle
}

fn make_credentials_preauth(dst: AccountID, hashes: &[Uint256]) -> STLedgerEntry {
    let dst_key = Uint160::from_slice(dst.data()).expect("account width");
    STLedgerEntry::new(deposit_preauth_credentials_keylet(dst_key, hashes))
}

#[test]
fn deposit_authorized_reports_missing_and_malformed() {
    let source = FakeSource {
        ledger: Some(closed_ledger()),
        ..Default::default()
    };
    let request = DepositAuthorizedRequest {
        params: &JsonValue::Object(Default::default()),
        api_version: 1,
        role: RpcRole::Admin,
    };

    let missing = do_deposit_authorized(&request, &source);
    let JsonValue::Object(missing) = missing else {
        panic!("missing response must be an object");
    };
    assert_eq!(
        missing.get("error_message"),
        Some(&JsonValue::String(
            "Missing field 'source_account'.".to_owned()
        ))
    );

    let invalid = do_deposit_authorized(
        &DepositAuthorizedRequest {
            params: &object([
                ("source_account", JsonValue::Unsigned(1)),
                (
                    "destination_account",
                    JsonValue::String(to_base58(sample_account(0x22))),
                ),
            ]),
            api_version: 1,
            role: RpcRole::Admin,
        },
        &source,
    );
    let JsonValue::Object(invalid) = invalid else {
        panic!("invalid response must be an object");
    };
    assert_eq!(
        invalid.get("error"),
        Some(&JsonValue::String("invalidParams".to_owned()))
    );

    let malformed = do_deposit_authorized(
        &DepositAuthorizedRequest {
            params: &object([
                ("source_account", JsonValue::String("foo".to_owned())),
                (
                    "destination_account",
                    JsonValue::String(to_base58(sample_account(0x22))),
                ),
            ]),
            api_version: 1,
            role: RpcRole::Admin,
        },
        &source,
    );
    let JsonValue::Object(malformed) = malformed else {
        panic!("malformed response must be an object");
    };
    assert_eq!(
        malformed.get("error"),
        Some(&JsonValue::String("actMalformed".to_owned()))
    );
}

#[test]
fn deposit_authorized_reports_lookup_errors() {
    let src = sample_account(0x11);
    let dst = sample_account(0x22);
    let source = FakeSource {
        ledger: Some(closed_ledger()),
        ..Default::default()
    };

    let src_missing = do_deposit_authorized(
        &DepositAuthorizedRequest {
            params: &object([
                ("source_account", JsonValue::String(to_base58(src))),
                ("destination_account", JsonValue::String(to_base58(dst))),
            ]),
            api_version: 1,
            role: RpcRole::Admin,
        },
        &source,
    );
    let JsonValue::Object(src_missing) = src_missing else {
        panic!("lookup response must be an object");
    };
    assert_eq!(
        src_missing.get("error"),
        Some(&JsonValue::String("srcActNotFound".to_owned()))
    );

    let mut source = source;
    source
        .account_roots
        .insert(src, make_account_root(src, false));
    let dst_missing = do_deposit_authorized(
        &DepositAuthorizedRequest {
            params: &object([
                ("source_account", JsonValue::String(to_base58(src))),
                ("destination_account", JsonValue::String(to_base58(dst))),
            ]),
            api_version: 1,
            role: RpcRole::Admin,
        },
        &source,
    );
    let JsonValue::Object(dst_missing) = dst_missing else {
        panic!("lookup response must be an object");
    };
    assert_eq!(
        dst_missing.get("error"),
        Some(&JsonValue::String("dstActNotFound".to_owned()))
    );
}

#[test]
fn deposit_authorized_accepts_direct_and_credential_preauth() {
    let src = sample_account(0x11);
    let dst = sample_account(0x22);
    let issuer_a = sample_account(0x31);
    let issuer_b = sample_account(0x32);

    let direct_preauth = make_direct_preauth(dst, src);
    let cred_a = make_credential(src, issuer_a, b"alpha", true, Some(200));
    let cred_b = make_credential(src, issuer_b, b"beta", true, Some(200));
    let cred_a_key = cred_a.key().to_owned();
    let cred_b_key = cred_b.key().to_owned();
    let hashes = vec![
        {
            let issuer = Uint160::from_slice(issuer_a.data()).expect("issuer width");
            let credential_type = b"alpha";
            let mut hasher = sha2::Sha512::new();
            hasher.update(issuer.data());
            hasher.update(credential_type);
            let digest = hasher.finalize();
            Uint256::from_slice(&digest[..32]).expect("hash width")
        },
        {
            let issuer = Uint160::from_slice(issuer_b.data()).expect("issuer width");
            let credential_type = b"beta";
            let mut hasher = sha2::Sha512::new();
            hasher.update(issuer.data());
            hasher.update(credential_type);
            let digest = hasher.finalize();
            Uint256::from_slice(&digest[..32]).expect("hash width")
        },
    ];
    let preauth_credentials = make_credentials_preauth(dst, &hashes);

    let mut direct_source = FakeSource {
        ledger: Some(closed_ledger()),
        parent_close_time: 100,
        ..Default::default()
    };
    direct_source
        .account_roots
        .insert(src, make_account_root(src, false));
    direct_source
        .account_roots
        .insert(dst, make_account_root(dst, true));
    direct_source
        .entries
        .insert(direct_preauth.key().to_owned(), direct_preauth);

    let direct_result = do_deposit_authorized(
        &DepositAuthorizedRequest {
            params: &object([
                ("source_account", JsonValue::String(to_base58(src))),
                ("destination_account", JsonValue::String(to_base58(dst))),
            ]),
            api_version: 1,
            role: RpcRole::Admin,
        },
        &direct_source,
    );
    let JsonValue::Object(direct_result) = direct_result else {
        panic!("direct result must be an object");
    };
    assert_eq!(
        direct_result.get("deposit_authorized"),
        Some(&JsonValue::Bool(true))
    );
    assert_eq!(
        direct_result.get("source_account"),
        Some(&JsonValue::String(to_base58(src)))
    );
    assert_eq!(
        direct_result.get("destination_account"),
        Some(&JsonValue::String(to_base58(dst)))
    );

    let mut credential_source = FakeSource {
        ledger: Some(closed_ledger()),
        parent_close_time: 100,
        ..Default::default()
    };
    credential_source
        .account_roots
        .insert(src, make_account_root(src, false));
    credential_source
        .account_roots
        .insert(dst, make_account_root(dst, true));
    credential_source.entries.insert(cred_a_key, cred_a.clone());
    credential_source.entries.insert(cred_b_key, cred_b.clone());
    credential_source
        .entries
        .insert(preauth_credentials.key().to_owned(), preauth_credentials);

    let credential_result = do_deposit_authorized(
        &DepositAuthorizedRequest {
            params: &object([
                ("source_account", JsonValue::String(to_base58(src))),
                ("destination_account", JsonValue::String(to_base58(dst))),
                (
                    "credentials",
                    JsonValue::Array(vec![
                        JsonValue::String(cred_b_key.to_string()),
                        JsonValue::String(cred_a_key.to_string()),
                    ]),
                ),
            ]),
            api_version: 1,
            role: RpcRole::Admin,
        },
        &credential_source,
    );
    let JsonValue::Object(credential_result) = credential_result else {
        panic!("credential result must be an object");
    };
    assert_eq!(
        credential_result.get("deposit_authorized"),
        Some(&JsonValue::Bool(true))
    );
    assert_eq!(
        credential_result.get("credentials"),
        Some(&JsonValue::Array(vec![
            JsonValue::String(cred_b_key.to_string()),
            JsonValue::String(cred_a_key.to_string()),
        ]))
    );
}

#[test]
fn deposit_authorized_rejects_bad_credentials() {
    let src = sample_account(0x11);
    let dst = sample_account(0x22);
    let issuer = sample_account(0x31);
    let valid_cred = make_credential(src, issuer, b"alpha", true, Some(200));
    let expired_cred = make_credential(src, issuer, b"beta", true, Some(50));

    let mut source = FakeSource {
        ledger: Some(closed_ledger()),
        parent_close_time: 100,
        ..Default::default()
    };
    source
        .account_roots
        .insert(src, make_account_root(src, false));
    source
        .account_roots
        .insert(dst, make_account_root(dst, true));
    let valid_cred_key = valid_cred.key().to_owned();
    let expired_cred_key = expired_cred.key().to_owned();
    source.entries.insert(valid_cred_key, valid_cred);
    source
        .entries
        .insert(expired_cred_key, expired_cred.clone());

    let duplicate = do_deposit_authorized(
        &DepositAuthorizedRequest {
            params: &object([
                ("source_account", JsonValue::String(to_base58(src))),
                ("destination_account", JsonValue::String(to_base58(dst))),
                (
                    "credentials",
                    JsonValue::Array(vec![
                        JsonValue::String(valid_cred_key.to_string()),
                        JsonValue::String(valid_cred_key.to_string()),
                    ]),
                ),
            ]),
            api_version: 1,
            role: RpcRole::Admin,
        },
        &source,
    );
    let JsonValue::Object(duplicate) = duplicate else {
        panic!("duplicate response must be an object");
    };
    assert_eq!(
        duplicate.get("error"),
        Some(&JsonValue::String("badCredentials".to_owned()))
    );

    let expired = do_deposit_authorized(
        &DepositAuthorizedRequest {
            params: &object([
                ("source_account", JsonValue::String(to_base58(src))),
                ("destination_account", JsonValue::String(to_base58(dst))),
                (
                    "credentials",
                    JsonValue::Array(vec![JsonValue::String(expired_cred_key.to_string())]),
                ),
            ]),
            api_version: 1,
            role: RpcRole::Admin,
        },
        &source,
    );
    let JsonValue::Object(expired) = expired else {
        panic!("expired response must be an object");
    };
    assert_eq!(
        expired.get("error"),
        Some(&JsonValue::String("badCredentials".to_owned()))
    );
}
