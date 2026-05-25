//! Integration tests that pin the narrowed Rust `Transactor.cpp::checkSign`
//! shells to the current C++ behavior.

use protocol::{Ter, trans_token};
use tx::{
    ApplyFlags, TransactorMultiSignAccountSigner, TransactorMultiSignSignerList,
    TransactorMultiSignTxSigner, TransactorSignMultiSignObject, TransactorSignObject,
    TransactorSignTx, TransactorSingleSignAccountState, run_transactor_check_sign,
    run_transactor_preclaim_check_sign,
};

#[derive(Clone)]
struct StubSignObject {
    signing_pub_key_is_empty: bool,
    has_signers: bool,
    has_txn_signature: bool,
    tx_signers: Vec<StubTxSigner>,
}

impl TransactorSignObject for StubSignObject {
    fn signing_pub_key_is_empty(&self) -> bool {
        self.signing_pub_key_is_empty
    }

    fn has_signers(&self) -> bool {
        self.has_signers
    }

    fn has_txn_signature(&self) -> bool {
        self.has_txn_signature
    }
}

impl TransactorSignMultiSignObject<&'static str> for StubSignObject {
    type TxSigner = StubTxSigner;
    type TxSigners = Vec<StubTxSigner>;

    fn tx_signers(&self) -> Self::TxSigners {
        self.tx_signers.clone()
    }
}

#[derive(Clone, Copy)]
struct StubTxSigner {
    account_id: &'static str,
    signing_pub_key_is_empty: bool,
}

impl TransactorMultiSignTxSigner<&'static str> for StubTxSigner {
    fn account_id(&self) -> &'static str {
        self.account_id
    }

    fn signing_pub_key_is_empty(&self) -> bool {
        self.signing_pub_key_is_empty
    }
}

#[derive(Clone, Copy)]
struct StubAccountSigner {
    account_id: &'static str,
    weight: u32,
}

impl TransactorMultiSignAccountSigner<&'static str> for StubAccountSigner {
    fn account_id(&self) -> &&'static str {
        &self.account_id
    }

    fn weight(&self) -> u32 {
        self.weight
    }
}

struct StubSignerList {
    signer_quorum: u32,
    signer_entries: Result<Vec<StubAccountSigner>, Ter>,
}

impl TransactorMultiSignSignerList<StubAccountSigner> for StubSignerList {
    type Entries = Vec<StubAccountSigner>;

    fn signer_list_id_present(&self) -> bool {
        true
    }

    fn signer_list_id(&self) -> u32 {
        0
    }

    fn signer_quorum(&self) -> u32 {
        self.signer_quorum
    }

    fn signer_entries(self) -> Result<Self::Entries, protocol::NotTec> {
        self.signer_entries
    }
}

struct StubTx {
    has_delegate: bool,
    delegate: &'static str,
    account: &'static str,
}

struct StubAccountState {
    regular_key: Option<&'static str>,
    is_master_disabled: bool,
}

impl TransactorSingleSignAccountState<&'static str> for StubAccountState {
    fn regular_key(&self) -> Option<&&'static str> {
        self.regular_key.as_ref()
    }

    fn is_master_disabled(&self) -> bool {
        self.is_master_disabled
    }
}

impl TransactorSignTx for StubTx {
    type AccountId = &'static str;

    fn has_delegate(&self) -> bool {
        self.has_delegate
    }

    fn delegate_account_id(&self) -> Self::AccountId {
        self.delegate
    }

    fn account_id(&self) -> Self::AccountId {
        self.account
    }
}

#[test]
fn tx_transactor_sign_batch_inner_rejects_unexpected_signature_fields() {
    let result = run_transactor_check_sign(
        ApplyFlags::NONE,
        true,
        true,
        false,
        &"alice",
        &StubSignObject {
            signing_pub_key_is_empty: false,
            has_signers: false,
            has_txn_signature: true,
            tx_signers: vec![],
        },
        |_| Some(()),
        |_| None::<StubSignerList>,
        |_| false,
        |_| true,
        |_| "alice",
        |_| panic!("batch-inner defensive reject should skip multisign pubkey typing"),
        |_| panic!("batch-inner defensive reject should skip multisign signer derivation"),
    );

    assert_eq!(result, Ter::TEM_INVALID_FLAG);
    assert_eq!(trans_token(result), "temINVALID_FLAG");
}

#[test]
fn tx_transactor_sign_batch_inner_skips_signature_checks_when_fields_are_absent() {
    let result = run_transactor_check_sign(
        ApplyFlags::NONE,
        true,
        true,
        false,
        &"alice",
        &StubSignObject {
            signing_pub_key_is_empty: true,
            has_signers: false,
            has_txn_signature: false,
            tx_signers: vec![],
        },
        |_| Some(()),
        |_| None::<StubSignerList>,
        |_| false,
        |_| panic!("batch-inner bypass should skip pubkey typing"),
        |_| panic!("batch-inner bypass should skip signer derivation"),
        |_| panic!("batch-inner bypass should skip multisign pubkey typing"),
        |_| panic!("batch-inner bypass should skip multisign signer derivation"),
    );

    assert_eq!(result, Ter::TES_SUCCESS);
    assert_eq!(trans_token(result), "tesSUCCESS");
}

#[test]
fn tx_transactor_sign_dry_run_skips_when_no_signing_material_is_present() {
    let result = run_transactor_check_sign(
        ApplyFlags::DRY_RUN,
        false,
        false,
        false,
        &"alice",
        &StubSignObject {
            signing_pub_key_is_empty: true,
            has_signers: false,
            has_txn_signature: false,
            tx_signers: vec![],
        },
        |_| Some(()),
        |_| None::<StubSignerList>,
        |_| false,
        |_| panic!("dry-run bypass should skip pubkey typing"),
        |_| panic!("dry-run bypass should skip signer derivation"),
        |_| panic!("dry-run bypass should skip multisign pubkey typing"),
        |_| panic!("dry-run bypass should skip multisign signer derivation"),
    );

    assert_eq!(result, Ter::TES_SUCCESS);
    assert_eq!(trans_token(result), "tesSUCCESS");
}

#[test]
fn tx_transactor_sign_routes_to_shared_multi_sign_when_signers_are_present() {
    let result = run_transactor_check_sign(
        ApplyFlags::DRY_RUN,
        false,
        false,
        false,
        &"alice",
        &StubSignObject {
            signing_pub_key_is_empty: true,
            has_signers: true,
            has_txn_signature: false,
            tx_signers: vec![StubTxSigner {
                account_id: "alice",
                signing_pub_key_is_empty: true,
            }],
        },
        |_| None::<StubAccountState>,
        |_| {
            Some(StubSignerList {
                signer_quorum: 2,
                signer_entries: Ok(vec![StubAccountSigner {
                    account_id: "alice",
                    weight: 1,
                }]),
            })
        },
        |_| false,
        |_| panic!("multisign path should skip single-sign pubkey typing"),
        |_| panic!("multisign path should skip signer derivation"),
        |_| true,
        |_| "alice",
    );

    assert_eq!(result, Ter::TEF_BAD_QUORUM);
    assert_eq!(trans_token(result), "tefBAD_QUORUM");
}

#[test]
fn tx_transactor_sign_rejects_unknown_public_key_type() {
    let result = run_transactor_check_sign(
        ApplyFlags::NONE,
        false,
        false,
        false,
        &"alice",
        &StubSignObject {
            signing_pub_key_is_empty: false,
            has_signers: false,
            has_txn_signature: true,
            tx_signers: vec![],
        },
        |_| {
            Some(StubAccountState {
                regular_key: None,
                is_master_disabled: false,
            })
        },
        |_| None::<StubSignerList>,
        |_| false,
        |_| false,
        |_| "alice",
        |_| panic!("single-sign input should skip multisign pubkey typing"),
        |_| panic!("single-sign input should skip multisign signer derivation"),
    );

    assert_eq!(result, Ter::TEF_BAD_AUTH);
    assert_eq!(trans_token(result), "tefBAD_AUTH");
}

#[test]
fn tx_transactor_sign_returns_no_account_for_missing_single_sign_account() {
    let result = run_transactor_check_sign(
        ApplyFlags::NONE,
        false,
        false,
        false,
        &"alice",
        &StubSignObject {
            signing_pub_key_is_empty: false,
            has_signers: false,
            has_txn_signature: true,
            tx_signers: vec![],
        },
        |_| None::<()>,
        |_| None::<StubSignerList>,
        |_| false,
        |_| true,
        |_| "alice",
        |_| panic!("single-sign input should skip multisign pubkey typing"),
        |_| panic!("single-sign input should skip multisign signer derivation"),
    );

    assert_eq!(result, Ter::TER_NO_ACCOUNT);
    assert_eq!(trans_token(result), "terNO_ACCOUNT");
}

#[test]
fn tx_transactor_sign_preclaim_wrapper_prefers_delegate() {
    let result = run_transactor_preclaim_check_sign(
        ApplyFlags::NONE,
        false,
        false,
        false,
        &StubTx {
            has_delegate: true,
            delegate: "delegate",
            account: "source",
        },
        &StubSignObject {
            signing_pub_key_is_empty: false,
            has_signers: false,
            has_txn_signature: true,
            tx_signers: vec![],
        },
        |account_id| {
            assert_eq!(*account_id, "delegate");
            None::<()>
        },
        |_| None::<StubSignerList>,
        |_| false,
        |_| true,
        |_| "delegate",
        |_| true,
        |_| "delegate",
    );

    assert_eq!(result, Ter::TER_NO_ACCOUNT);
    assert_eq!(trans_token(result), "terNO_ACCOUNT");
}

#[test]
fn tx_transactor_sign_preclaim_wrapper_falls_back_to_account() {
    let result = run_transactor_preclaim_check_sign(
        ApplyFlags::NONE,
        false,
        false,
        false,
        &StubTx {
            has_delegate: false,
            delegate: "delegate",
            account: "source",
        },
        &StubSignObject {
            signing_pub_key_is_empty: false,
            has_signers: false,
            has_txn_signature: true,
            tx_signers: vec![],
        },
        |account_id| {
            assert_eq!(*account_id, "source");
            None::<()>
        },
        |_| None::<StubSignerList>,
        |_| false,
        |_| true,
        |_| "source",
        |_| true,
        |_| "source",
    );

    assert_eq!(result, Ter::TER_NO_ACCOUNT);
    assert_eq!(trans_token(result), "terNO_ACCOUNT");
}

#[test]
fn tx_transactor_sign_uses_shared_single_sign_rules() {
    let result = run_transactor_check_sign(
        ApplyFlags::NONE,
        false,
        false,
        false,
        &"alice",
        &StubSignObject {
            signing_pub_key_is_empty: false,
            has_signers: false,
            has_txn_signature: true,
            tx_signers: vec![],
        },
        |_| {
            Some(StubAccountState {
                regular_key: Some("carol"),
                is_master_disabled: false,
            })
        },
        |_| None::<StubSignerList>,
        |_| false,
        |_| true,
        |_| "carol",
        |_| panic!("single-sign input should skip multisign pubkey typing"),
        |_| panic!("single-sign input should skip multisign signer derivation"),
    );

    assert_eq!(result, Ter::TES_SUCCESS);
    assert_eq!(trans_token(result), "tesSUCCESS");
}

#[test]
fn tx_transactor_sign_returns_master_disabled_from_shared_helper() {
    let result = run_transactor_check_sign(
        ApplyFlags::NONE,
        false,
        false,
        false,
        &"alice",
        &StubSignObject {
            signing_pub_key_is_empty: false,
            has_signers: false,
            has_txn_signature: true,
            tx_signers: vec![],
        },
        |_| {
            Some(StubAccountState {
                regular_key: Some("regular"),
                is_master_disabled: true,
            })
        },
        |_| None::<StubSignerList>,
        |_| false,
        |_| true,
        |_| "alice",
        |_| panic!("single-sign input should skip multisign pubkey typing"),
        |_| panic!("single-sign input should skip multisign signer derivation"),
    );

    assert_eq!(result, Ter::TEF_MASTER_DISABLED);
    assert_eq!(trans_token(result), "tefMASTER_DISABLED");
}
