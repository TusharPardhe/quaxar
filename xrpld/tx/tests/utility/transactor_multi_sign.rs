//! Integration tests that pin the narrowed Rust
//! `Transactor.cpp::checkMultiSign(...)` helper to the current C++ behavior.

use protocol::{Ter, trans_token};
use tx::{
    ApplyFlags, TransactorMultiSignAccountSigner, TransactorMultiSignSignerList,
    TransactorMultiSignTxSigner, TransactorSingleSignAccountState, run_transactor_check_multi_sign,
};

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

    fn signer_entries(self) -> Result<Self::Entries, Ter> {
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

#[test]
fn tx_transactor_multi_sign_rejects_missing_signer_list() {
    let result = run_transactor_check_multi_sign(
        ApplyFlags::NONE,
        None::<StubSignerList>,
        [StubTxSigner {
            account_id: "alice",
            signing_pub_key_is_empty: false,
        }],
        |_| None::<StubAccountState>,
        |_| true,
        |_| "alice",
    );

    assert_eq!(result, Ter::TEF_NOT_MULTI_SIGNING);
    assert_eq!(trans_token(result), "tefNOT_MULTI_SIGNING");
}

#[test]
fn tx_transactor_multi_sign_rejects_unlisted_or_unknown_signers() {
    let unknown_pubkey = run_transactor_check_multi_sign(
        ApplyFlags::NONE,
        Some(StubSignerList {
            signer_quorum: 1,
            signer_entries: Ok(vec![StubAccountSigner {
                account_id: "alice",
                weight: 1,
            }]),
        }),
        [StubTxSigner {
            account_id: "alice",
            signing_pub_key_is_empty: false,
        }],
        |_| None::<StubAccountState>,
        |_| false,
        |_| "alice",
    );

    let unlisted = run_transactor_check_multi_sign(
        ApplyFlags::NONE,
        Some(StubSignerList {
            signer_quorum: 1,
            signer_entries: Ok(vec![StubAccountSigner {
                account_id: "bob",
                weight: 1,
            }]),
        }),
        [StubTxSigner {
            account_id: "alice",
            signing_pub_key_is_empty: false,
        }],
        |_| None::<StubAccountState>,
        |_| true,
        |_| "alice",
    );

    assert_eq!(unknown_pubkey, Ter::TEF_BAD_SIGNATURE);
    assert_eq!(unlisted, Ter::TEF_BAD_SIGNATURE);
    assert_eq!(trans_token(unknown_pubkey), "tefBAD_SIGNATURE");
}

#[test]
fn tx_transactor_multi_sign_accepts_phantom_and_regular_key_cases() {
    let phantom = run_transactor_check_multi_sign(
        ApplyFlags::DRY_RUN,
        Some(StubSignerList {
            signer_quorum: 1,
            signer_entries: Ok(vec![StubAccountSigner {
                account_id: "phantom",
                weight: 1,
            }]),
        }),
        [StubTxSigner {
            account_id: "phantom",
            signing_pub_key_is_empty: true,
        }],
        |_| None::<StubAccountState>,
        |_| true,
        |_| "phantom",
    );

    let regular = run_transactor_check_multi_sign(
        ApplyFlags::NONE,
        Some(StubSignerList {
            signer_quorum: 2,
            signer_entries: Ok(vec![StubAccountSigner {
                account_id: "alice",
                weight: 2,
            }]),
        }),
        [StubTxSigner {
            account_id: "alice",
            signing_pub_key_is_empty: false,
        }],
        |_| {
            Some(StubAccountState {
                regular_key: Some("carol"),
                is_master_disabled: false,
            })
        },
        |_| true,
        |_| "carol",
    );

    assert_eq!(phantom, Ter::TES_SUCCESS);
    assert_eq!(regular, Ter::TES_SUCCESS);
}

#[test]
fn tx_transactor_multi_sign_returns_master_disabled_and_bad_quorum() {
    let disabled_master = run_transactor_check_multi_sign(
        ApplyFlags::NONE,
        Some(StubSignerList {
            signer_quorum: 1,
            signer_entries: Ok(vec![StubAccountSigner {
                account_id: "alice",
                weight: 1,
            }]),
        }),
        [StubTxSigner {
            account_id: "alice",
            signing_pub_key_is_empty: false,
        }],
        |_| {
            Some(StubAccountState {
                regular_key: Some("regular"),
                is_master_disabled: true,
            })
        },
        |_| true,
        |_| "alice",
    );

    let bad_quorum = run_transactor_check_multi_sign(
        ApplyFlags::NONE,
        Some(StubSignerList {
            signer_quorum: 3,
            signer_entries: Ok(vec![StubAccountSigner {
                account_id: "alice",
                weight: 2,
            }]),
        }),
        [StubTxSigner {
            account_id: "alice",
            signing_pub_key_is_empty: false,
        }],
        |_| {
            Some(StubAccountState {
                regular_key: Some("carol"),
                is_master_disabled: false,
            })
        },
        |_| true,
        |_| "carol",
    );

    assert_eq!(disabled_master, Ter::TEF_MASTER_DISABLED);
    assert_eq!(bad_quorum, Ter::TEF_BAD_QUORUM);
    assert_eq!(trans_token(disabled_master), "tefMASTER_DISABLED");
    assert_eq!(trans_token(bad_quorum), "tefBAD_QUORUM");
}
