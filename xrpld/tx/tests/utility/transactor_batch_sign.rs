//! Integration tests that pin the narrowed Rust
//! `Transactor.cpp::checkBatchSign(...)` shell to the current C++ behavior.

use protocol::{Ter, trans_token};
use tx::{
    ApplyFlags, TransactorBatchMultiSigner, TransactorBatchSignTx, TransactorBatchSigner,
    TransactorMultiSignAccountSigner, TransactorMultiSignSignerList, TransactorMultiSignTxSigner,
    TransactorSingleSignAccountState, run_transactor_check_batch_sign,
    run_transactor_preclaim_check_batch_sign,
};

#[derive(Clone)]
struct StubSigner {
    account_id: &'static str,
    signing_pub_key_is_empty: bool,
    tx_signers: Vec<StubTxSigner>,
}

struct StubTx {
    batch_signers: Vec<StubSigner>,
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

impl TransactorBatchSigner for StubSigner {
    type AccountId = &'static str;

    fn account_id(&self) -> Self::AccountId {
        self.account_id
    }

    fn signing_pub_key_is_empty(&self) -> bool {
        self.signing_pub_key_is_empty
    }
}

impl TransactorBatchMultiSigner<&'static str> for StubSigner {
    type TxSigner = StubTxSigner;
    type TxSigners = Vec<StubTxSigner>;

    fn tx_signers(&self) -> Self::TxSigners {
        self.tx_signers.clone()
    }
}

impl TransactorBatchSignTx for StubTx {
    type AccountId = &'static str;
    type Signer = StubSigner;
    type Signers = Vec<StubSigner>;

    fn batch_signers(&self) -> Self::Signers {
        self.batch_signers.clone()
    }
}

#[test]
fn tx_transactor_batch_sign_uses_shared_multi_sign_for_empty_signing_pub_key() {
    let result = run_transactor_preclaim_check_batch_sign(
        ApplyFlags::DRY_RUN,
        &StubTx {
            batch_signers: vec![StubSigner {
                account_id: "alice",
                signing_pub_key_is_empty: true,
                tx_signers: vec![StubTxSigner {
                    account_id: "alice",
                    signing_pub_key_is_empty: true,
                }],
            }],
        },
        |_| None::<StubAccountState>,
        |_| {
            Some(StubSignerList {
                signer_quorum: 1,
                signer_entries: Ok(vec![StubAccountSigner {
                    account_id: "alice",
                    weight: 1,
                }]),
            })
        },
        |_| panic!("single-sign pubkey typing should not run for multisign"),
        |_| panic!("single-sign signer derivation should not run for multisign"),
        |_| true,
        |_| "alice",
    );

    assert_eq!(result, Ter::TES_SUCCESS);
    assert_eq!(trans_token(result), "tesSUCCESS");
}

#[test]
fn tx_transactor_batch_sign_returns_shared_multi_sign_failure() {
    let result = run_transactor_check_batch_sign(
        ApplyFlags::DRY_RUN,
        [StubSigner {
            account_id: "alice",
            signing_pub_key_is_empty: true,
            tx_signers: vec![StubTxSigner {
                account_id: "alice",
                signing_pub_key_is_empty: true,
            }],
        }],
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
        |_| panic!("single-sign pubkey typing should not run for multisign"),
        |_| panic!("single-sign signer derivation should not run for multisign"),
        |_| true,
        |_| "alice",
    );

    assert_eq!(result, Ter::TEF_BAD_QUORUM);
    assert_eq!(trans_token(result), "tefBAD_QUORUM");
}

#[test]
fn tx_transactor_batch_sign_rejects_unknown_public_key_type() {
    let result = run_transactor_check_batch_sign(
        ApplyFlags::NONE,
        [StubSigner {
            account_id: "bob",
            signing_pub_key_is_empty: false,
            tx_signers: vec![],
        }],
        |_| -> Option<()> { panic!("account lookup should not run after an invalid pubkey type") },
        |_| None::<StubSignerList>,
        |_| false,
        |_| "bob",
        |_| panic!("multisign pubkey typing should not run for single-sign input"),
        |_| panic!("multisign signer derivation should not run for single-sign input"),
    );

    assert_eq!(result, Ter::TEF_BAD_AUTH);
    assert_eq!(trans_token(result), "tefBAD_AUTH");
}

#[test]
fn tx_transactor_batch_sign_rejects_uncreated_account_with_wrong_master_key() {
    let result = run_transactor_check_batch_sign(
        ApplyFlags::NONE,
        [StubSigner {
            account_id: "phantom",
            signing_pub_key_is_empty: false,
            tx_signers: vec![],
        }],
        |_| None::<()>,
        |_| None::<StubSignerList>,
        |_| true,
        |_| "carol",
        |_| panic!("multisign pubkey typing should not run for single-sign input"),
        |_| panic!("multisign signer derivation should not run for single-sign input"),
    );

    assert_eq!(result, Ter::TEF_BAD_AUTH);
    assert_eq!(trans_token(result), "tefBAD_AUTH");
}

#[test]
fn tx_transactor_batch_sign_accepts_uncreated_account_with_matching_master_key() {
    let result = run_transactor_preclaim_check_batch_sign(
        ApplyFlags::NONE,
        &StubTx {
            batch_signers: vec![
                StubSigner {
                    account_id: "phantom",
                    signing_pub_key_is_empty: false,
                    tx_signers: vec![],
                },
                StubSigner {
                    account_id: "later",
                    signing_pub_key_is_empty: true,
                    tx_signers: vec![StubTxSigner {
                        account_id: "later",
                        signing_pub_key_is_empty: true,
                    }],
                },
            ],
        },
        |account_id| {
            if *account_id == "phantom" {
                None::<StubAccountState>
            } else {
                Some(StubAccountState {
                    regular_key: None,
                    is_master_disabled: false,
                })
            }
        },
        |_| -> Option<StubSignerList> {
            panic!("C++ returns early success after the phantom master-key case")
        },
        |_| true,
        |_| "phantom",
        |_| panic!("later multisign pubkey typing should be skipped"),
        |_| panic!("later multisign signer derivation should be skipped"),
    );

    assert_eq!(result, Ter::TES_SUCCESS);
    assert_eq!(trans_token(result), "tesSUCCESS");
}

#[test]
fn tx_transactor_batch_sign_uses_shared_single_sign_rules() {
    let result = run_transactor_check_batch_sign(
        ApplyFlags::NONE,
        [StubSigner {
            account_id: "bob",
            signing_pub_key_is_empty: false,
            tx_signers: vec![],
        }],
        |_| {
            Some(StubAccountState {
                regular_key: Some("carol"),
                is_master_disabled: false,
            })
        },
        |_| None::<StubSignerList>,
        |_| true,
        |_| "carol",
        |_| panic!("multisign pubkey typing should not run for single-sign input"),
        |_| panic!("multisign signer derivation should not run for single-sign input"),
    );

    assert_eq!(result, Ter::TES_SUCCESS);
    assert_eq!(trans_token(result), "tesSUCCESS");
}

#[test]
fn tx_transactor_batch_sign_returns_master_disabled_from_shared_helper() {
    let result = run_transactor_check_batch_sign(
        ApplyFlags::NONE,
        [StubSigner {
            account_id: "bob",
            signing_pub_key_is_empty: false,
            tx_signers: vec![],
        }],
        |_| {
            Some(StubAccountState {
                regular_key: Some("regular"),
                is_master_disabled: true,
            })
        },
        |_| None::<StubSignerList>,
        |_| true,
        |_| "bob",
        |_| panic!("multisign pubkey typing should not run for single-sign input"),
        |_| panic!("multisign signer derivation should not run for single-sign input"),
    );

    assert_eq!(result, Ter::TEF_MASTER_DISABLED);
    assert_eq!(trans_token(result), "tefMASTER_DISABLED");
}
