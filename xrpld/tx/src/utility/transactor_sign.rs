//! Current Rust helpers mirroring the current the reference implementation
//! behavior.
//!
//! This module preserves the deterministic wrapper behavior around:
//!
//! - the lending-gated pseudo-account rejection,
//! - the batch-inner bypass and defensive `temINVALID_FLAG` guard,
//! - the dry-run bypass when neither `SigningPubKey` nor `Signers` is present,
//! - routing to the shared multi-sign helper when `Signers` is present, and
//! - the higher preclaim wrapper that prefers `sfDelegate` over `sfAccount`.

use protocol::{NotTec, Ter, is_tes_success};

use crate::transactor_multi_sign::{
    TransactorMultiSignAccountSigner, TransactorMultiSignSignerList, TransactorMultiSignTxSigner,
    run_transactor_check_multi_sign,
};
use crate::transactor_single_sign::{
    TransactorSingleSignAccountState, run_transactor_check_single_sign,
};
use crate::{ApplyFlags, any_apply_flags};

pub trait TransactorSignObject {
    fn signing_pub_key_is_empty(&self) -> bool;
    fn has_signers(&self) -> bool;
    fn has_txn_signature(&self) -> bool;
}

pub trait TransactorSignMultiSignObject<AccountId>: TransactorSignObject {
    type TxSigner: TransactorMultiSignTxSigner<AccountId>;
    type TxSigners: IntoIterator<Item = Self::TxSigner>;

    fn tx_signers(&self) -> Self::TxSigners;
}

pub trait TransactorSignTx {
    type AccountId: Clone;

    fn has_delegate(&self) -> bool;
    fn delegate_account_id(&self) -> Self::AccountId;
    fn account_id(&self) -> Self::AccountId;
}

pub fn select_transactor_signing_account<Tx>(tx: &Tx) -> Tx::AccountId
where
    Tx: TransactorSignTx,
{
    if tx.has_delegate() {
        tx.delegate_account_id()
    } else {
        tx.account_id()
    }
}

pub fn run_transactor_check_sign<
    SignatureObject,
    AccountId,
    AccountState,
    SignerList,
    AccountSigner,
    ReadAccount,
    ReadSignerList,
    IsPseudoAccount,
    PublicKeyTypeKnown,
    SignerAccountFromPublicKey,
    TxSignerPublicKeyTypeKnown,
    TxSignerAccountFromPublicKey,
>(
    flags: ApplyFlags,
    parent_batch_id_present: bool,
    feature_batch_enabled: bool,
    lending_protocol_enabled: bool,
    id_account: &AccountId,
    signature_object: &SignatureObject,
    mut read_account: ReadAccount,
    mut read_signer_list: ReadSignerList,
    mut is_pseudo_account: IsPseudoAccount,
    mut public_key_type_known: PublicKeyTypeKnown,
    mut signer_account_from_public_key: SignerAccountFromPublicKey,
    mut tx_signer_public_key_type_known: TxSignerPublicKeyTypeKnown,
    mut tx_signer_account_from_public_key: TxSignerAccountFromPublicKey,
) -> NotTec
where
    SignatureObject: TransactorSignObject + TransactorSignMultiSignObject<AccountId>,
    AccountId: Clone + Eq + Ord,
    AccountState: TransactorSingleSignAccountState<AccountId>,
    SignerList: TransactorMultiSignSignerList<AccountSigner>,
    AccountSigner: TransactorMultiSignAccountSigner<AccountId>,
    ReadAccount: FnMut(&AccountId) -> Option<AccountState>,
    ReadSignerList: FnMut(&AccountId) -> Option<SignerList>,
    IsPseudoAccount: FnMut(Option<&AccountState>) -> bool,
    PublicKeyTypeKnown: FnMut(&SignatureObject) -> bool,
    SignerAccountFromPublicKey: FnMut(&SignatureObject) -> AccountId,
    TxSignerPublicKeyTypeKnown: FnMut(&SignatureObject::TxSigner) -> bool,
    TxSignerAccountFromPublicKey: FnMut(&SignatureObject::TxSigner) -> AccountId,
{
    let sle = read_account(id_account);
    if (lending_protocol_enabled || feature_batch_enabled) && is_pseudo_account(sle.as_ref()) {
        return Ter::TEF_BAD_AUTH;
    }

    let signing_pub_key_is_empty = signature_object.signing_pub_key_is_empty();
    if parent_batch_id_present && feature_batch_enabled {
        if signature_object.has_txn_signature()
            || !signing_pub_key_is_empty
            || signature_object.has_signers()
        {
            return Ter::TEM_INVALID_FLAG;
        }
        return Ter::TES_SUCCESS;
    }

    if any_apply_flags(flags & ApplyFlags::DRY_RUN)
        && signing_pub_key_is_empty
        && !signature_object.has_signers()
    {
        return Ter::TES_SUCCESS;
    }

    if signature_object.has_signers() {
        return run_transactor_check_multi_sign(
            flags,
            read_signer_list(id_account),
            signature_object.tx_signers(),
            |account_id| read_account(account_id),
            |tx_signer| tx_signer_public_key_type_known(tx_signer),
            |tx_signer| tx_signer_account_from_public_key(tx_signer),
        );
    }

    if !public_key_type_known(signature_object) {
        return Ter::TEF_BAD_AUTH;
    }

    let id_signer = signer_account_from_public_key(signature_object);
    let Some(sle_account) = read_account(id_account) else {
        return Ter::TER_NO_ACCOUNT;
    };

    let ret = run_transactor_check_single_sign(&id_signer, id_account, &sle_account);
    if !is_tes_success(ret) {
        return ret;
    }

    Ter::TES_SUCCESS
}

#[allow(clippy::too_many_arguments)]
pub fn run_transactor_preclaim_check_sign<
    Tx,
    SignatureObject,
    AccountId,
    AccountState,
    SignerList,
    AccountSigner,
    ReadAccount,
    ReadSignerList,
    IsPseudoAccount,
    PublicKeyTypeKnown,
    SignerAccountFromPublicKey,
    TxSignerPublicKeyTypeKnown,
    TxSignerAccountFromPublicKey,
>(
    flags: ApplyFlags,
    parent_batch_id_present: bool,
    feature_batch_enabled: bool,
    lending_protocol_enabled: bool,
    tx: &Tx,
    signature_object: &SignatureObject,
    read_account: ReadAccount,
    read_signer_list: ReadSignerList,
    is_pseudo_account: IsPseudoAccount,
    public_key_type_known: PublicKeyTypeKnown,
    signer_account_from_public_key: SignerAccountFromPublicKey,
    tx_signer_public_key_type_known: TxSignerPublicKeyTypeKnown,
    tx_signer_account_from_public_key: TxSignerAccountFromPublicKey,
) -> NotTec
where
    Tx: TransactorSignTx<AccountId = AccountId>,
    SignatureObject: TransactorSignObject + TransactorSignMultiSignObject<AccountId>,
    AccountId: Clone + Eq + Ord,
    AccountState: TransactorSingleSignAccountState<AccountId>,
    SignerList: TransactorMultiSignSignerList<AccountSigner>,
    AccountSigner: TransactorMultiSignAccountSigner<AccountId>,
    ReadAccount: FnMut(&AccountId) -> Option<AccountState>,
    ReadSignerList: FnMut(&AccountId) -> Option<SignerList>,
    IsPseudoAccount: FnMut(Option<&AccountState>) -> bool,
    PublicKeyTypeKnown: FnMut(&SignatureObject) -> bool,
    SignerAccountFromPublicKey: FnMut(&SignatureObject) -> AccountId,
    TxSignerPublicKeyTypeKnown: FnMut(&SignatureObject::TxSigner) -> bool,
    TxSignerAccountFromPublicKey: FnMut(&SignatureObject::TxSigner) -> AccountId,
{
    let id_account = select_transactor_signing_account(tx);

    run_transactor_check_sign(
        flags,
        parent_batch_id_present,
        feature_batch_enabled,
        lending_protocol_enabled,
        &id_account,
        signature_object,
        read_account,
        read_signer_list,
        is_pseudo_account,
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
        TransactorSignMultiSignObject, TransactorSignObject, TransactorSignTx,
        run_transactor_check_sign, run_transactor_preclaim_check_sign,
        select_transactor_signing_account,
    };
    use crate::ApplyFlags;
    use crate::transactor_multi_sign::{
        TransactorMultiSignAccountSigner, TransactorMultiSignSignerList,
        TransactorMultiSignTxSigner,
    };
    use crate::transactor_single_sign::TransactorSingleSignAccountState;

    #[derive(Clone)]
    struct TestSignObject {
        signing_pub_key_is_empty: bool,
        has_signers: bool,
        has_txn_signature: bool,
        tx_signers: Vec<TestTxSigner>,
    }

    impl TransactorSignObject for TestSignObject {
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

    impl TransactorSignMultiSignObject<&'static str> for TestSignObject {
        type TxSigner = TestTxSigner;
        type TxSigners = Vec<TestTxSigner>;

        fn tx_signers(&self) -> Self::TxSigners {
            self.tx_signers.clone()
        }
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

    struct TestTx {
        has_delegate: bool,
        delegate_account: &'static str,
        account: &'static str,
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

    impl TransactorSignTx for TestTx {
        type AccountId = &'static str;

        fn has_delegate(&self) -> bool {
            self.has_delegate
        }

        fn delegate_account_id(&self) -> Self::AccountId {
            self.delegate_account
        }

        fn account_id(&self) -> Self::AccountId {
            self.account
        }
    }

    #[test]
    fn transactor_sign_rejects_pseudo_accounts_when_lending_is_enabled() {
        let multi_signer_pubkey_checked = Cell::new(false);

        let result = run_transactor_check_sign(
            ApplyFlags::NONE,
            false,
            false,
            true,
            &"alice",
            &TestSignObject {
                signing_pub_key_is_empty: false,
                has_signers: false,
                has_txn_signature: true,
                tx_signers: vec![],
            },
            |_| {
                Some(TestAccountState {
                    regular_key: None,
                    is_master_disabled: false,
                })
            },
            |_| None::<TestSignerList>,
            |sle| sle.is_some(),
            |_| true,
            |_| "alice",
            |_| {
                multi_signer_pubkey_checked.set(true);
                true
            },
            |_| "alice",
        );

        assert_eq!(result, protocol::Ter::TEF_BAD_AUTH);
        assert_eq!(trans_token(result), "tefBAD_AUTH");
        assert!(!multi_signer_pubkey_checked.get());
    }

    #[test]
    fn transactor_sign_batch_inner_rejects_unexpected_signature_fields() {
        let result = run_transactor_check_sign(
            ApplyFlags::NONE,
            true,
            true,
            false,
            &"alice",
            &TestSignObject {
                signing_pub_key_is_empty: false,
                has_signers: false,
                has_txn_signature: true,
                tx_signers: vec![],
            },
            |_| Some(()),
            |_| None::<TestSignerList>,
            |_| false,
            |_| true,
            |_| "alice",
            |_| panic!("batch-inner defensive reject should skip multisign pubkey typing"),
            |_| panic!("batch-inner defensive reject should skip multisign signer derivation"),
        );

        assert_eq!(result, protocol::Ter::TEM_INVALID_FLAG);
        assert_eq!(trans_token(result), "temINVALID_FLAG");
    }

    #[test]
    fn transactor_sign_batch_inner_skips_signature_verification_when_fields_are_absent() {
        let result = run_transactor_check_sign(
            ApplyFlags::NONE,
            true,
            true,
            false,
            &"alice",
            &TestSignObject {
                signing_pub_key_is_empty: true,
                has_signers: false,
                has_txn_signature: false,
                tx_signers: vec![],
            },
            |_| Some(()),
            |_| None::<TestSignerList>,
            |_| false,
            |_| panic!("batch-inner bypass should skip pubkey typing"),
            |_| panic!("batch-inner bypass should skip signer derivation"),
            |_| panic!("batch-inner bypass should skip multisign pubkey typing"),
            |_| panic!("batch-inner bypass should skip multisign signer derivation"),
        );

        assert_eq!(result, protocol::Ter::TES_SUCCESS);
        assert_eq!(trans_token(result), "tesSUCCESS");
    }

    #[test]
    fn transactor_sign_dry_run_skips_when_no_signing_material_is_present() {
        let result = run_transactor_check_sign(
            ApplyFlags::DRY_RUN,
            false,
            false,
            false,
            &"alice",
            &TestSignObject {
                signing_pub_key_is_empty: true,
                has_signers: false,
                has_txn_signature: false,
                tx_signers: vec![],
            },
            |_| Some(()),
            |_| None::<TestSignerList>,
            |_| false,
            |_| panic!("dry-run bypass should skip pubkey typing"),
            |_| panic!("dry-run bypass should skip signer derivation"),
            |_| panic!("dry-run bypass should skip multisign pubkey typing"),
            |_| panic!("dry-run bypass should skip multisign signer derivation"),
        );

        assert_eq!(result, protocol::Ter::TES_SUCCESS);
    }

    #[test]
    fn transactor_sign_routes_to_shared_multi_sign_helper_when_signers_are_present() {
        let result = run_transactor_check_sign(
            ApplyFlags::DRY_RUN,
            false,
            false,
            false,
            &"alice",
            &TestSignObject {
                signing_pub_key_is_empty: true,
                has_signers: true,
                has_txn_signature: false,
                tx_signers: vec![TestTxSigner {
                    account_id: "alice",
                    signing_pub_key_is_empty: true,
                }],
            },
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
            |_| false,
            |_| panic!("multisign path should not inspect single-sign pubkeys"),
            |_| panic!("multisign path should not derive single-sign accounts"),
            |_| true,
            |_| "alice",
        );

        assert_eq!(result, protocol::Ter::TEF_BAD_QUORUM);
        assert_eq!(trans_token(result), "tefBAD_QUORUM");
    }

    #[test]
    fn transactor_sign_rejects_unknown_public_key_type() {
        let second_lookup = Cell::new(false);

        let result = run_transactor_check_sign(
            ApplyFlags::NONE,
            false,
            false,
            false,
            &"alice",
            &TestSignObject {
                signing_pub_key_is_empty: false,
                has_signers: false,
                has_txn_signature: true,
                tx_signers: vec![],
            },
            |_| {
                second_lookup.set(true);
                Some(TestAccountState {
                    regular_key: None,
                    is_master_disabled: false,
                })
            },
            |_| None::<TestSignerList>,
            |_| false,
            |_| false,
            |_| "alice",
            |_| true,
            |_| "alice",
        );

        assert_eq!(result, protocol::Ter::TEF_BAD_AUTH);
        assert_eq!(trans_token(result), "tefBAD_AUTH");
        assert!(second_lookup.get());
    }

    #[test]
    fn transactor_sign_returns_no_account_when_single_sign_account_is_missing() {
        let read_count = Cell::new(0);

        let result = run_transactor_check_sign(
            ApplyFlags::NONE,
            false,
            false,
            false,
            &"alice",
            &TestSignObject {
                signing_pub_key_is_empty: false,
                has_signers: false,
                has_txn_signature: true,
                tx_signers: vec![],
            },
            |_| {
                read_count.set(read_count.get() + 1);
                None::<()>
            },
            |_| None::<TestSignerList>,
            |_| false,
            |_| true,
            |_| "alice",
            |_| true,
            |_| "alice",
        );

        assert_eq!(result, protocol::Ter::TER_NO_ACCOUNT);
        assert_eq!(trans_token(result), "terNO_ACCOUNT");
        assert_eq!(read_count.get(), 2);
    }

    #[test]
    fn transactor_sign_uses_shared_single_sign_rules() {
        let result = run_transactor_check_sign(
            ApplyFlags::NONE,
            false,
            false,
            false,
            &"alice",
            &TestSignObject {
                signing_pub_key_is_empty: false,
                has_signers: false,
                has_txn_signature: true,
                tx_signers: vec![],
            },
            |_| {
                Some(TestAccountState {
                    regular_key: Some("carol"),
                    is_master_disabled: false,
                })
            },
            |_| None::<TestSignerList>,
            |_| false,
            |_| true,
            |_| "carol",
            |_| true,
            |_| "alice",
        );

        assert_eq!(result, protocol::Ter::TES_SUCCESS);
    }

    #[test]
    fn transactor_sign_surfaces_disabled_master_key_from_shared_helper() {
        let result = run_transactor_check_sign(
            ApplyFlags::NONE,
            false,
            false,
            false,
            &"alice",
            &TestSignObject {
                signing_pub_key_is_empty: false,
                has_signers: false,
                has_txn_signature: true,
                tx_signers: vec![],
            },
            |_| {
                Some(TestAccountState {
                    regular_key: Some("regular"),
                    is_master_disabled: true,
                })
            },
            |_| None::<TestSignerList>,
            |_| false,
            |_| true,
            |_| "alice",
            |_| true,
            |_| "alice",
        );

        assert_eq!(result, protocol::Ter::TEF_MASTER_DISABLED);
        assert_eq!(trans_token(result), "tefMASTER_DISABLED");
    }

    #[test]
    fn transactor_preclaim_signing_account_prefers_delegate() {
        let result = run_transactor_preclaim_check_sign(
            ApplyFlags::NONE,
            false,
            false,
            false,
            &TestTx {
                has_delegate: true,
                delegate_account: "delegate",
                account: "source",
            },
            &TestSignObject {
                signing_pub_key_is_empty: false,
                has_signers: false,
                has_txn_signature: true,
                tx_signers: vec![],
            },
            |account_id| {
                assert_eq!(*account_id, "delegate");
                None::<()>
            },
            |_| None::<TestSignerList>,
            |_| false,
            |_| true,
            |_| "delegate",
            |_| true,
            |_| "delegate",
        );

        assert_eq!(result, protocol::Ter::TER_NO_ACCOUNT);
    }

    #[test]
    fn transactor_preclaim_signing_account_falls_back_to_account() {
        let result = run_transactor_preclaim_check_sign(
            ApplyFlags::NONE,
            false,
            false,
            false,
            &TestTx {
                has_delegate: false,
                delegate_account: "delegate",
                account: "source",
            },
            &TestSignObject {
                signing_pub_key_is_empty: false,
                has_signers: false,
                has_txn_signature: true,
                tx_signers: vec![],
            },
            |account_id| {
                assert_eq!(*account_id, "source");
                None::<()>
            },
            |_| None::<TestSignerList>,
            |_| false,
            |_| true,
            |_| "source",
            |_| true,
            |_| "source",
        );

        assert_eq!(result, protocol::Ter::TER_NO_ACCOUNT);
    }

    #[test]
    fn transactor_signing_account_selector_prefers_delegate() {
        let selected = select_transactor_signing_account(&TestTx {
            has_delegate: true,
            delegate_account: "delegate",
            account: "source",
        });

        assert_eq!(selected, "delegate");
    }

    #[test]
    fn transactor_signing_account_selector_falls_back_to_account() {
        let selected = select_transactor_signing_account(&TestTx {
            has_delegate: false,
            delegate_account: "delegate",
            account: "source",
        });

        assert_eq!(selected, "source");
    }
}
