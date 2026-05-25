//! Current Rust helper mirroring `Transactor::checkMultiSign(...)`.
//!
//! This module preserves the deterministic multi-sign verification behavior
//! around:
//!
//! - missing signer-list rejection,
//! - the current `SignerListID` invariant assertions,
//! - sorted linear matching between tx signers and signer-list entries,
//! - dry-run empty-pubkey handling,
//! - signer legitimacy checks for phantom, master-key, and regular-key cases,
//! - weight accumulation, and
//! - final quorum enforcement.

use protocol::{NotTec, Ter, is_tes_success};

use crate::transactor_single_sign::TransactorSingleSignAccountState;
use crate::{ApplyFlags, any_apply_flags};

pub const TRANSACTOR_CHECK_MULTI_SIGN_HAS_SIGNER_LIST_ID_ASSERT: &str =
    "xrpl::Transactor::checkMultiSign : has signer list ID";
pub const TRANSACTOR_CHECK_MULTI_SIGN_SIGNER_LIST_ID_ZERO_ASSERT: &str =
    "xrpl::Transactor::checkMultiSign : signer list ID is 0";
pub const TRANSACTOR_CHECK_MULTI_SIGN_NON_EMPTY_SIGNER_ASSERT: &str =
    "xrpl::Transactor::checkMultiSign : non-empty signer or simulation";

pub trait TransactorMultiSignTxSigner<AccountId> {
    fn account_id(&self) -> AccountId;
    fn signing_pub_key_is_empty(&self) -> bool;
}

pub trait TransactorMultiSignAccountSigner<AccountId> {
    fn account_id(&self) -> &AccountId;
    fn weight(&self) -> u32;
}

pub trait TransactorMultiSignSignerList<AccountSigner> {
    type Entries: IntoIterator<Item = AccountSigner>;

    fn signer_list_id_present(&self) -> bool;
    fn signer_list_id(&self) -> u32;
    fn signer_quorum(&self) -> u32;
    fn signer_entries(self) -> Result<Self::Entries, NotTec>;
}

pub fn run_transactor_check_multi_signer_authorized<AccountId, AccountState>(
    tx_signer_account_id: &AccountId,
    signing_account_from_pub_key: &AccountId,
    account_state: Option<&AccountState>,
) -> NotTec
where
    AccountId: Eq,
    AccountState: TransactorSingleSignAccountState<AccountId>,
{
    if signing_account_from_pub_key == tx_signer_account_id {
        if account_state.is_some_and(TransactorSingleSignAccountState::is_master_disabled) {
            return Ter::TEF_MASTER_DISABLED;
        }

        return Ter::TES_SUCCESS;
    }

    let Some(account_state) = account_state else {
        return Ter::TEF_BAD_SIGNATURE;
    };

    if account_state.regular_key() != Some(signing_account_from_pub_key) {
        return Ter::TEF_BAD_SIGNATURE;
    }

    Ter::TES_SUCCESS
}

pub fn run_transactor_check_multi_sign<
    AccountId,
    SignerList,
    AccountSigner,
    TxSigner,
    TxSigners,
    AccountState,
    ReadAccount,
    PublicKeyTypeKnown,
    SignerAccountFromPublicKey,
>(
    flags: ApplyFlags,
    signer_list: Option<SignerList>,
    tx_signers: TxSigners,
    mut read_account: ReadAccount,
    mut public_key_type_known: PublicKeyTypeKnown,
    mut signer_account_from_public_key: SignerAccountFromPublicKey,
) -> NotTec
where
    AccountId: Clone + Eq + Ord,
    SignerList: TransactorMultiSignSignerList<AccountSigner>,
    AccountSigner: TransactorMultiSignAccountSigner<AccountId>,
    TxSigner: TransactorMultiSignTxSigner<AccountId>,
    TxSigners: IntoIterator<Item = TxSigner>,
    AccountState: TransactorSingleSignAccountState<AccountId>,
    ReadAccount: FnMut(&AccountId) -> Option<AccountState>,
    PublicKeyTypeKnown: FnMut(&TxSigner) -> bool,
    SignerAccountFromPublicKey: FnMut(&TxSigner) -> AccountId,
{
    let Some(signer_list) = signer_list else {
        return Ter::TEF_NOT_MULTI_SIGNING;
    };

    assert!(
        signer_list.signer_list_id_present(),
        "{TRANSACTOR_CHECK_MULTI_SIGN_HAS_SIGNER_LIST_ID_ASSERT}"
    );
    assert!(
        signer_list.signer_list_id() == 0,
        "{TRANSACTOR_CHECK_MULTI_SIGN_SIGNER_LIST_ID_ZERO_ASSERT}"
    );

    let signer_quorum = signer_list.signer_quorum();
    let account_signers = match signer_list.signer_entries() {
        Ok(entries) => entries,
        Err(err) => return err,
    };
    let mut account_signers = account_signers.into_iter();
    let Some(mut current_signer) = account_signers.next() else {
        return Ter::TEF_BAD_SIGNATURE;
    };

    let mut weight_sum = 0_u32;

    for tx_signer in tx_signers {
        let tx_signer_account_id = tx_signer.account_id();

        while current_signer.account_id() < &tx_signer_account_id {
            let Some(next_signer) = account_signers.next() else {
                return Ter::TEF_BAD_SIGNATURE;
            };
            current_signer = next_signer;
        }

        if current_signer.account_id() != &tx_signer_account_id {
            return Ter::TEF_BAD_SIGNATURE;
        }

        let signing_account_from_pub_key = if tx_signer.signing_pub_key_is_empty() {
            assert!(
                any_apply_flags(flags & ApplyFlags::DRY_RUN),
                "{TRANSACTOR_CHECK_MULTI_SIGN_NON_EMPTY_SIGNER_ASSERT}"
            );
            tx_signer_account_id.clone()
        } else {
            if !public_key_type_known(&tx_signer) {
                return Ter::TEF_BAD_SIGNATURE;
            }

            signer_account_from_public_key(&tx_signer)
        };

        let account_state = read_account(&tx_signer_account_id);
        let ret = run_transactor_check_multi_signer_authorized(
            &tx_signer_account_id,
            &signing_account_from_pub_key,
            account_state.as_ref(),
        );
        if !is_tes_success(ret) {
            return ret;
        }

        weight_sum = weight_sum.wrapping_add(current_signer.weight());
    }

    if weight_sum < signer_quorum {
        return Ter::TEF_BAD_QUORUM;
    }

    Ter::TES_SUCCESS
}

#[cfg(test)]
mod tests {
    use protocol::NotTec;
    use protocol::trans_token;

    use super::{
        TRANSACTOR_CHECK_MULTI_SIGN_HAS_SIGNER_LIST_ID_ASSERT,
        TRANSACTOR_CHECK_MULTI_SIGN_NON_EMPTY_SIGNER_ASSERT,
        TRANSACTOR_CHECK_MULTI_SIGN_SIGNER_LIST_ID_ZERO_ASSERT, TransactorMultiSignAccountSigner,
        TransactorMultiSignSignerList, TransactorMultiSignTxSigner,
        run_transactor_check_multi_sign, run_transactor_check_multi_signer_authorized,
    };
    use crate::ApplyFlags;
    use crate::transactor_single_sign::TransactorSingleSignAccountState;

    #[derive(Clone, Copy)]
    struct TestTxSigner {
        account_id: &'static str,
        signing_pub_key_is_empty: bool,
    }

    impl TransactorMultiSignTxSigner<&'static str> for TestTxSigner {
        fn account_id(&self) -> &'static str {
            self.account_id
        }

        fn signing_pub_key_is_empty(&self) -> bool {
            self.signing_pub_key_is_empty
        }
    }

    #[derive(Clone, Copy)]
    struct TestAccountSigner {
        account_id: &'static str,
        weight: u32,
    }

    impl TransactorMultiSignAccountSigner<&'static str> for TestAccountSigner {
        fn account_id(&self) -> &&'static str {
            &self.account_id
        }

        fn weight(&self) -> u32 {
            self.weight
        }
    }

    struct TestSignerList {
        signer_list_id_present: bool,
        signer_list_id: u32,
        signer_quorum: u32,
        signer_entries: Result<Vec<TestAccountSigner>, protocol::Ter>,
    }

    impl TransactorMultiSignSignerList<TestAccountSigner> for TestSignerList {
        type Entries = Vec<TestAccountSigner>;

        fn signer_list_id_present(&self) -> bool {
            self.signer_list_id_present
        }

        fn signer_list_id(&self) -> u32 {
            self.signer_list_id
        }

        fn signer_quorum(&self) -> u32 {
            self.signer_quorum
        }

        fn signer_entries(self) -> Result<Self::Entries, NotTec> {
            self.signer_entries
        }
    }

    struct TestAccountState {
        regular_key: Option<&'static str>,
        is_master_disabled: bool,
    }

    impl TransactorSingleSignAccountState<&'static str> for TestAccountState {
        fn regular_key(&self) -> Option<&&'static str> {
            self.regular_key.as_ref()
        }

        fn is_master_disabled(&self) -> bool {
            self.is_master_disabled
        }
    }

    #[test]
    fn transactor_multi_sign_rejects_missing_signer_list() {
        let result = run_transactor_check_multi_sign(
            ApplyFlags::NONE,
            None::<TestSignerList>,
            [TestTxSigner {
                account_id: "alice",
                signing_pub_key_is_empty: false,
            }],
            |_| {
                Some(TestAccountState {
                    regular_key: None,
                    is_master_disabled: false,
                })
            },
            |_| true,
            |_| "alice",
        );

        assert_eq!(result, protocol::Ter::TEF_NOT_MULTI_SIGNING);
        assert_eq!(trans_token(result), "tefNOT_MULTI_SIGNING");
    }

    #[test]
    fn transactor_multi_sign_propagates_signer_entry_deserialize_failures() {
        let result = run_transactor_check_multi_sign(
            ApplyFlags::NONE,
            Some(TestSignerList {
                signer_list_id_present: true,
                signer_list_id: 0,
                signer_quorum: 1,
                signer_entries: Err(protocol::Ter::TEM_MALFORMED),
            }),
            [TestTxSigner {
                account_id: "alice",
                signing_pub_key_is_empty: false,
            }],
            |_| {
                Some(TestAccountState {
                    regular_key: None,
                    is_master_disabled: false,
                })
            },
            |_| true,
            |_| "alice",
        );

        assert_eq!(result, protocol::Ter::TEM_MALFORMED);
        assert_eq!(trans_token(result), "temMALFORMED");
    }

    #[test]
    fn transactor_multi_sign_rejects_unknown_or_unlisted_signers() {
        let unknown_pubkey = run_transactor_check_multi_sign(
            ApplyFlags::NONE,
            Some(TestSignerList {
                signer_list_id_present: true,
                signer_list_id: 0,
                signer_quorum: 1,
                signer_entries: Ok(vec![TestAccountSigner {
                    account_id: "alice",
                    weight: 1,
                }]),
            }),
            [TestTxSigner {
                account_id: "alice",
                signing_pub_key_is_empty: false,
            }],
            |_| {
                Some(TestAccountState {
                    regular_key: None,
                    is_master_disabled: false,
                })
            },
            |_| false,
            |_| "alice",
        );

        let unlisted_signer = run_transactor_check_multi_sign(
            ApplyFlags::NONE,
            Some(TestSignerList {
                signer_list_id_present: true,
                signer_list_id: 0,
                signer_quorum: 1,
                signer_entries: Ok(vec![TestAccountSigner {
                    account_id: "bob",
                    weight: 1,
                }]),
            }),
            [TestTxSigner {
                account_id: "alice",
                signing_pub_key_is_empty: false,
            }],
            |_| {
                Some(TestAccountState {
                    regular_key: None,
                    is_master_disabled: false,
                })
            },
            |_| true,
            |_| "alice",
        );

        assert_eq!(unknown_pubkey, protocol::Ter::TEF_BAD_SIGNATURE);
        assert_eq!(unlisted_signer, protocol::Ter::TEF_BAD_SIGNATURE);
        assert_eq!(trans_token(unknown_pubkey), "tefBAD_SIGNATURE");
    }

    #[test]
    fn transactor_multi_sign_accepts_phantom_or_regular_signers_and_checks_quorum() {
        let phantom = run_transactor_check_multi_sign(
            ApplyFlags::DRY_RUN,
            Some(TestSignerList {
                signer_list_id_present: true,
                signer_list_id: 0,
                signer_quorum: 1,
                signer_entries: Ok(vec![TestAccountSigner {
                    account_id: "phantom",
                    weight: 1,
                }]),
            }),
            [TestTxSigner {
                account_id: "phantom",
                signing_pub_key_is_empty: true,
            }],
            |_| None::<TestAccountState>,
            |_| true,
            |_| "phantom",
        );

        let regular_key = run_transactor_check_multi_sign(
            ApplyFlags::NONE,
            Some(TestSignerList {
                signer_list_id_present: true,
                signer_list_id: 0,
                signer_quorum: 2,
                signer_entries: Ok(vec![
                    TestAccountSigner {
                        account_id: "alice",
                        weight: 2,
                    },
                    TestAccountSigner {
                        account_id: "bob",
                        weight: 1,
                    },
                ]),
            }),
            [TestTxSigner {
                account_id: "alice",
                signing_pub_key_is_empty: false,
            }],
            |_| {
                Some(TestAccountState {
                    regular_key: Some("carol"),
                    is_master_disabled: false,
                })
            },
            |_| true,
            |_| "carol",
        );

        let bad_quorum = run_transactor_check_multi_sign(
            ApplyFlags::NONE,
            Some(TestSignerList {
                signer_list_id_present: true,
                signer_list_id: 0,
                signer_quorum: 3,
                signer_entries: Ok(vec![TestAccountSigner {
                    account_id: "alice",
                    weight: 2,
                }]),
            }),
            [TestTxSigner {
                account_id: "alice",
                signing_pub_key_is_empty: false,
            }],
            |_| {
                Some(TestAccountState {
                    regular_key: Some("carol"),
                    is_master_disabled: false,
                })
            },
            |_| true,
            |_| "carol",
        );

        assert_eq!(phantom, protocol::Ter::TES_SUCCESS);
        assert_eq!(regular_key, protocol::Ter::TES_SUCCESS);
        assert_eq!(bad_quorum, protocol::Ter::TEF_BAD_QUORUM);
        assert_eq!(trans_token(bad_quorum), "tefBAD_QUORUM");
    }

    #[test]
    fn transactor_multi_sign_signer_authorization_cases() {
        let disabled_master = run_transactor_check_multi_signer_authorized(
            &"alice",
            &"alice",
            Some(&TestAccountState {
                regular_key: Some("regular"),
                is_master_disabled: true,
            }),
        );

        let missing_regular_key = run_transactor_check_multi_signer_authorized(
            &"alice",
            &"carol",
            Some(&TestAccountState {
                regular_key: None,
                is_master_disabled: false,
            }),
        );

        assert_eq!(disabled_master, protocol::Ter::TEF_MASTER_DISABLED);
        assert_eq!(missing_regular_key, protocol::Ter::TEF_BAD_SIGNATURE);
    }

    #[test]
    #[should_panic(expected = "xrpl::Transactor::checkMultiSign : has signer list ID")]
    fn transactor_multi_sign_asserts_signer_list_id_presence() {
        let _ = TRANSACTOR_CHECK_MULTI_SIGN_HAS_SIGNER_LIST_ID_ASSERT;
        let _ = run_transactor_check_multi_sign(
            ApplyFlags::NONE,
            Some(TestSignerList {
                signer_list_id_present: false,
                signer_list_id: 0,
                signer_quorum: 1,
                signer_entries: Ok(vec![TestAccountSigner {
                    account_id: "alice",
                    weight: 1,
                }]),
            }),
            [TestTxSigner {
                account_id: "alice",
                signing_pub_key_is_empty: false,
            }],
            |_| None::<TestAccountState>,
            |_| true,
            |_| "alice",
        );
    }

    #[test]
    #[should_panic(expected = "xrpl::Transactor::checkMultiSign : signer list ID is 0")]
    fn transactor_multi_sign_asserts_zero_signer_list_id() {
        let _ = TRANSACTOR_CHECK_MULTI_SIGN_SIGNER_LIST_ID_ZERO_ASSERT;
        let _ = run_transactor_check_multi_sign(
            ApplyFlags::NONE,
            Some(TestSignerList {
                signer_list_id_present: true,
                signer_list_id: 1,
                signer_quorum: 1,
                signer_entries: Ok(vec![TestAccountSigner {
                    account_id: "alice",
                    weight: 1,
                }]),
            }),
            [TestTxSigner {
                account_id: "alice",
                signing_pub_key_is_empty: false,
            }],
            |_| None::<TestAccountState>,
            |_| true,
            |_| "alice",
        );
    }

    #[test]
    #[should_panic(expected = "xrpl::Transactor::checkMultiSign : non-empty signer or simulation")]
    fn transactor_multi_sign_asserts_non_empty_pubkey_outside_simulation() {
        let _ = TRANSACTOR_CHECK_MULTI_SIGN_NON_EMPTY_SIGNER_ASSERT;
        let _ = run_transactor_check_multi_sign(
            ApplyFlags::NONE,
            Some(TestSignerList {
                signer_list_id_present: true,
                signer_list_id: 0,
                signer_quorum: 1,
                signer_entries: Ok(vec![TestAccountSigner {
                    account_id: "alice",
                    weight: 1,
                }]),
            }),
            [TestTxSigner {
                account_id: "alice",
                signing_pub_key_is_empty: true,
            }],
            |_| None::<TestAccountState>,
            |_| true,
            |_| "alice",
        );
    }
}
