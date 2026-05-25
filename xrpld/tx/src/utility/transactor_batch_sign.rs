//! Current Rust helper mirroring the reference implementation.
//!
//! This module preserves the deterministic outer behavior around:
//!
//! - iterating `sfBatchSigners` in order,
//! - routing empty `SigningPubKey` entries through the shared multi-sign
//!   helper,
//! - rejecting unknown single-sign public-key types with `tefBAD_AUTH`,
//! - allowing an uncreated batch signer only when the signer matches the
//!   account's master key, and
//! - preserving the current early `tesSUCCESS` return for that uncreated
//!   master-key case, plus the higher `PreclaimContext` wrapper that forwards
//!   `ctx.flags` and `ctx.tx.getFieldArray(sfBatchSigners)` into this helper.

use protocol::{NotTec, Ter, is_tes_success};

use crate::ApplyFlags;
use crate::transactor_multi_sign::{
    TransactorMultiSignAccountSigner, TransactorMultiSignSignerList, TransactorMultiSignTxSigner,
    run_transactor_check_multi_sign,
};
use crate::transactor_single_sign::{
    TransactorSingleSignAccountState, run_transactor_check_single_sign,
};

pub trait TransactorBatchSigner {
    type AccountId: Clone + Eq;

    fn account_id(&self) -> Self::AccountId;
    fn signing_pub_key_is_empty(&self) -> bool;
}

pub trait TransactorBatchMultiSigner<AccountId>:
    TransactorBatchSigner<AccountId = AccountId>
{
    type TxSigner: TransactorMultiSignTxSigner<AccountId>;
    type TxSigners: IntoIterator<Item = Self::TxSigner>;

    fn tx_signers(&self) -> Self::TxSigners;
}

pub trait TransactorBatchSignTx {
    type AccountId: Clone + Eq + Ord;
    type Signer: TransactorBatchSigner<AccountId = Self::AccountId>
        + TransactorBatchMultiSigner<Self::AccountId>;
    type Signers: IntoIterator<Item = Self::Signer>;

    fn batch_signers(&self) -> Self::Signers;
}

pub fn run_transactor_check_batch_sign<
    Signer,
    Signers,
    AccountId,
    AccountState,
    SignerList,
    AccountSigner,
    ReadAccount,
    ReadSignerList,
    PublicKeyTypeKnown,
    SignerAccountFromPublicKey,
    TxSignerPublicKeyTypeKnown,
    TxSignerAccountFromPublicKey,
>(
    flags: ApplyFlags,
    signers: Signers,
    mut read_account: ReadAccount,
    mut read_signer_list: ReadSignerList,
    mut public_key_type_known: PublicKeyTypeKnown,
    mut signer_account_from_public_key: SignerAccountFromPublicKey,
    mut tx_signer_public_key_type_known: TxSignerPublicKeyTypeKnown,
    mut tx_signer_account_from_public_key: TxSignerAccountFromPublicKey,
) -> NotTec
where
    Signers: IntoIterator<Item = Signer>,
    Signer: TransactorBatchSigner<AccountId = AccountId> + TransactorBatchMultiSigner<AccountId>,
    AccountId: Clone + Eq + Ord,
    AccountState: TransactorSingleSignAccountState<AccountId>,
    SignerList: TransactorMultiSignSignerList<AccountSigner>,
    AccountSigner: TransactorMultiSignAccountSigner<AccountId>,
    ReadAccount: FnMut(&AccountId) -> Option<AccountState>,
    ReadSignerList: FnMut(&AccountId) -> Option<SignerList>,
    PublicKeyTypeKnown: FnMut(&Signer) -> bool,
    SignerAccountFromPublicKey: FnMut(&Signer) -> AccountId,
    TxSignerPublicKeyTypeKnown: FnMut(&Signer::TxSigner) -> bool,
    TxSignerAccountFromPublicKey: FnMut(&Signer::TxSigner) -> AccountId,
{
    for signer in signers {
        let id_account = signer.account_id();

        if signer.signing_pub_key_is_empty() {
            let ret = run_transactor_check_multi_sign(
                flags,
                read_signer_list(&id_account),
                signer.tx_signers(),
                |account_id| read_account(account_id),
                |tx_signer| tx_signer_public_key_type_known(tx_signer),
                |tx_signer| tx_signer_account_from_public_key(tx_signer),
            );
            if !is_tes_success(ret) {
                return ret;
            }
            continue;
        }

        if !public_key_type_known(&signer) {
            return Ter::TEF_BAD_AUTH;
        }

        let id_signer = signer_account_from_public_key(&signer);
        let Some(account_state) = read_account(&id_account) else {
            if id_account != id_signer {
                return Ter::TEF_BAD_AUTH;
            }

            return Ter::TES_SUCCESS;
        };

        let ret = run_transactor_check_single_sign(&id_signer, &id_account, &account_state);
        if !is_tes_success(ret) {
            return ret;
        }
    }

    Ter::TES_SUCCESS
}

#[allow(clippy::too_many_arguments)]
pub fn run_transactor_preclaim_check_batch_sign<
    Tx,
    AccountState,
    SignerList,
    AccountSigner,
    ReadAccount,
    ReadSignerList,
    PublicKeyTypeKnown,
    SignerAccountFromPublicKey,
    TxSignerPublicKeyTypeKnown,
    TxSignerAccountFromPublicKey,
>(
    flags: ApplyFlags,
    tx: &Tx,
    read_account: ReadAccount,
    read_signer_list: ReadSignerList,
    public_key_type_known: PublicKeyTypeKnown,
    signer_account_from_public_key: SignerAccountFromPublicKey,
    tx_signer_public_key_type_known: TxSignerPublicKeyTypeKnown,
    tx_signer_account_from_public_key: TxSignerAccountFromPublicKey,
) -> NotTec
where
    Tx: TransactorBatchSignTx,
    AccountState: TransactorSingleSignAccountState<Tx::AccountId>,
    SignerList: TransactorMultiSignSignerList<AccountSigner>,
    AccountSigner: TransactorMultiSignAccountSigner<Tx::AccountId>,
    ReadAccount: FnMut(&Tx::AccountId) -> Option<AccountState>,
    ReadSignerList: FnMut(&Tx::AccountId) -> Option<SignerList>,
    PublicKeyTypeKnown: FnMut(&Tx::Signer) -> bool,
    SignerAccountFromPublicKey: FnMut(&Tx::Signer) -> Tx::AccountId,
    TxSignerPublicKeyTypeKnown:
        FnMut(&<Tx::Signer as TransactorBatchMultiSigner<Tx::AccountId>>::TxSigner) -> bool,
    TxSignerAccountFromPublicKey: FnMut(
        &<Tx::Signer as TransactorBatchMultiSigner<Tx::AccountId>>::TxSigner,
    ) -> Tx::AccountId,
{
    run_transactor_check_batch_sign(
        flags,
        tx.batch_signers(),
        read_account,
        read_signer_list,
        public_key_type_known,
        signer_account_from_public_key,
        tx_signer_public_key_type_known,
        tx_signer_account_from_public_key,
    )
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use protocol::trans_token;

    use super::{
        TransactorBatchMultiSigner, TransactorBatchSignTx, TransactorBatchSigner,
        run_transactor_check_batch_sign, run_transactor_preclaim_check_batch_sign,
    };
    use crate::ApplyFlags;
    use crate::transactor_multi_sign::{
        TransactorMultiSignAccountSigner, TransactorMultiSignSignerList,
        TransactorMultiSignTxSigner,
    };
    use crate::transactor_single_sign::TransactorSingleSignAccountState;

    #[derive(Clone)]
    struct TestSigner {
        account_id: &'static str,
        signing_pub_key_is_empty: bool,
        tx_signers: Vec<TestTxSigner>,
    }

    struct TestTx {
        batch_signers: Vec<TestSigner>,
    }

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
        signer_quorum: u32,
        signer_entries: Result<Vec<TestAccountSigner>, protocol::Ter>,
    }

    impl TransactorMultiSignSignerList<TestAccountSigner> for TestSignerList {
        type Entries = Vec<TestAccountSigner>;

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

    impl TransactorBatchSigner for TestSigner {
        type AccountId = &'static str;

        fn account_id(&self) -> Self::AccountId {
            self.account_id
        }

        fn signing_pub_key_is_empty(&self) -> bool {
            self.signing_pub_key_is_empty
        }
    }

    impl TransactorBatchMultiSigner<&'static str> for TestSigner {
        type TxSigner = TestTxSigner;
        type TxSigners = Vec<TestTxSigner>;

        fn tx_signers(&self) -> Self::TxSigners {
            self.tx_signers.clone()
        }
    }

    impl TransactorBatchSignTx for TestTx {
        type AccountId = &'static str;
        type Signer = TestSigner;
        type Signers = Vec<TestSigner>;

        fn batch_signers(&self) -> Self::Signers {
            self.batch_signers.clone()
        }
    }

    #[test]
    fn transactor_batch_sign_uses_shared_multi_sign_when_signing_pub_key_is_empty() {
        let read_signer_list_called = Cell::new(false);
        let single_called = Cell::new(false);

        let result = run_transactor_check_batch_sign(
            ApplyFlags::DRY_RUN,
            [TestSigner {
                account_id: "alice",
                signing_pub_key_is_empty: true,
                tx_signers: vec![TestTxSigner {
                    account_id: "alice",
                    signing_pub_key_is_empty: true,
                }],
            }],
            |_| None::<TestAccountState>,
            |_| {
                read_signer_list_called.set(true);
                Some(TestSignerList {
                    signer_quorum: 1,
                    signer_entries: Ok(vec![TestAccountSigner {
                        account_id: "alice",
                        weight: 1,
                    }]),
                })
            },
            |_| {
                single_called.set(true);
                true
            },
            |_| "alice",
            |_| true,
            |_| "alice",
        );

        assert_eq!(result, protocol::Ter::TES_SUCCESS);
        assert!(read_signer_list_called.get());
        assert!(!single_called.get());
    }

    #[test]
    fn transactor_batch_sign_returns_first_shared_multi_sign_failure_unchanged() {
        let later_single_sign_called = Cell::new(false);

        let result = run_transactor_check_batch_sign(
            ApplyFlags::DRY_RUN,
            [
                TestSigner {
                    account_id: "alice",
                    signing_pub_key_is_empty: true,
                    tx_signers: vec![TestTxSigner {
                        account_id: "alice",
                        signing_pub_key_is_empty: true,
                    }],
                },
                TestSigner {
                    account_id: "bob",
                    signing_pub_key_is_empty: false,
                    tx_signers: vec![],
                },
            ],
            |_| None::<TestAccountState>,
            |_| {
                Some(TestSignerList {
                    signer_quorum: 2,
                    signer_entries: Ok(vec![TestAccountSigner {
                        account_id: "alice",
                        weight: 1,
                    }]),
                })
            },
            |_| {
                later_single_sign_called.set(true);
                true
            },
            |_| "bob",
            |_| true,
            |_| "alice",
        );

        assert_eq!(result, protocol::Ter::TEF_BAD_QUORUM);
        assert_eq!(trans_token(result), "tefBAD_QUORUM");
        assert!(!later_single_sign_called.get());
    }

    #[test]
    fn transactor_batch_sign_rejects_unknown_public_key_type() {
        let account_lookup_called = Cell::new(false);

        let result = run_transactor_check_batch_sign(
            ApplyFlags::NONE,
            [TestSigner {
                account_id: "alice",
                signing_pub_key_is_empty: false,
                tx_signers: vec![],
            }],
            |_| {
                account_lookup_called.set(true);
                Some(())
            },
            |_| None::<TestSignerList>,
            |_| false,
            |_| "alice",
            |_| true,
            |_| "alice",
        );

        assert_eq!(result, protocol::Ter::TEF_BAD_AUTH);
        assert_eq!(trans_token(result), "tefBAD_AUTH");
        assert!(!account_lookup_called.get());
    }

    #[test]
    fn transactor_batch_sign_rejects_uncreated_account_with_different_signer() {
        let result = run_transactor_check_batch_sign(
            ApplyFlags::NONE,
            [TestSigner {
                account_id: "phantom",
                signing_pub_key_is_empty: false,
                tx_signers: vec![],
            }],
            |_| None::<()>,
            |_| None::<TestSignerList>,
            |_| true,
            |_| "carol",
            |_| true,
            |_| "alice",
        );

        assert_eq!(result, protocol::Ter::TEF_BAD_AUTH);
        assert_eq!(trans_token(result), "tefBAD_AUTH");
    }

    #[test]
    fn transactor_batch_sign_accepts_uncreated_account_with_master_key_and_stops() {
        let later_signer_checked = Cell::new(false);

        let result = run_transactor_check_batch_sign(
            ApplyFlags::NONE,
            [
                TestSigner {
                    account_id: "phantom",
                    signing_pub_key_is_empty: false,
                    tx_signers: vec![],
                },
                TestSigner {
                    account_id: "bob",
                    signing_pub_key_is_empty: true,
                    tx_signers: vec![TestTxSigner {
                        account_id: "bob",
                        signing_pub_key_is_empty: true,
                    }],
                },
            ],
            |account_id| {
                if *account_id == "phantom" {
                    None::<()>
                } else {
                    Some(())
                }
            },
            |_| {
                later_signer_checked.set(true);
                None::<TestSignerList>
            },
            |_| true,
            |_| "phantom",
            |_| true,
            |_| "alice",
        );

        assert_eq!(result, protocol::Ter::TES_SUCCESS);
        assert_eq!(trans_token(result), "tesSUCCESS");
        assert!(!later_signer_checked.get());
    }

    #[test]
    fn transactor_batch_sign_uses_shared_single_sign_rules() {
        let result = run_transactor_check_batch_sign(
            ApplyFlags::NONE,
            [TestSigner {
                account_id: "bob",
                signing_pub_key_is_empty: false,
                tx_signers: vec![],
            }],
            |_| {
                Some(TestAccountState {
                    regular_key: Some("carol"),
                    is_master_disabled: false,
                })
            },
            |_| None::<TestSignerList>,
            |_| true,
            |_| "carol",
            |_| true,
            |_| "alice",
        );

        assert_eq!(result, protocol::Ter::TES_SUCCESS);
    }

    #[test]
    fn transactor_batch_sign_surfaces_disabled_master_key_from_shared_helper() {
        let result = run_transactor_check_batch_sign(
            ApplyFlags::NONE,
            [TestSigner {
                account_id: "bob",
                signing_pub_key_is_empty: false,
                tx_signers: vec![],
            }],
            |_| {
                Some(TestAccountState {
                    regular_key: Some("regular"),
                    is_master_disabled: true,
                })
            },
            |_| None::<TestSignerList>,
            |_| true,
            |_| "bob",
            |_| true,
            |_| "alice",
        );

        assert_eq!(result, protocol::Ter::TEF_MASTER_DISABLED);
        assert_eq!(trans_token(result), "tefMASTER_DISABLED");
    }

    #[test]
    fn transactor_preclaim_batch_sign_reads_signers_from_tx() {
        let result = run_transactor_preclaim_check_batch_sign(
            ApplyFlags::DRY_RUN,
            &TestTx {
                batch_signers: vec![TestSigner {
                    account_id: "alice",
                    signing_pub_key_is_empty: true,
                    tx_signers: vec![TestTxSigner {
                        account_id: "alice",
                        signing_pub_key_is_empty: true,
                    }],
                }],
            },
            |_| None::<TestAccountState>,
            |_| {
                Some(TestSignerList {
                    signer_quorum: 1,
                    signer_entries: Ok(vec![TestAccountSigner {
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

        assert_eq!(result, protocol::Ter::TES_SUCCESS);
        assert_eq!(trans_token(result), "tesSUCCESS");
    }

    #[test]
    fn transactor_preclaim_batch_sign_keeps_early_master_key_success() {
        let later_signer_checked = Cell::new(false);

        let result = run_transactor_preclaim_check_batch_sign(
            ApplyFlags::NONE,
            &TestTx {
                batch_signers: vec![
                    TestSigner {
                        account_id: "phantom",
                        signing_pub_key_is_empty: false,
                        tx_signers: vec![],
                    },
                    TestSigner {
                        account_id: "later",
                        signing_pub_key_is_empty: true,
                        tx_signers: vec![TestTxSigner {
                            account_id: "later",
                            signing_pub_key_is_empty: true,
                        }],
                    },
                ],
            },
            |account_id| {
                if *account_id == "phantom" {
                    None::<TestAccountState>
                } else {
                    Some(TestAccountState {
                        regular_key: None,
                        is_master_disabled: false,
                    })
                }
            },
            |_| {
                later_signer_checked.set(true);
                None::<TestSignerList>
            },
            |_| true,
            |_| "phantom",
            |_| true,
            |_| "later",
        );

        assert_eq!(result, protocol::Ter::TES_SUCCESS);
        assert_eq!(trans_token(result), "tesSUCCESS");
        assert!(!later_signer_checked.get());
    }
}
