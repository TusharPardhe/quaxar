//! Current the reference implementation multi-sign wrapper shells.
//!
//! This ports the deterministic wrapper behavior around:
//!
//! - `STTx::checkMultiSign(...)` only rejecting self-multisigning for the
//!   primary signature object, and
//! - `STTx::checkBatchMultiSign(...)` always allowing the shared helper to run
//!   with no primary-account self-multisign restriction.

use crate::sttx_multi_sign::{StTxMultiSignObject, StTxMultiSigner, check_sttx_multi_sign};

pub fn run_sttx_check_multi_sign<
    AccountId,
    SignatureObject,
    Signer,
    MessageSeed,
    Message,
    BuildMessage,
    VerifySigner,
    FormatAccountId,
>(
    signature_object: &SignatureObject,
    is_primary_signature_object: bool,
    txn_account_id: &AccountId,
    message_seed: &MessageSeed,
    mut build_message: BuildMessage,
    mut verify_signer: VerifySigner,
    format_account_id: FormatAccountId,
) -> Result<(), String>
where
    AccountId: Clone + Eq + Ord,
    SignatureObject: StTxMultiSignObject<AccountId, Signer>,
    Signer: StTxMultiSigner<AccountId>,
    BuildMessage: FnMut(&MessageSeed, &AccountId) -> Message,
    VerifySigner: FnMut(&Signer, Message) -> Result<(), String>,
    FormatAccountId: FnMut(&AccountId) -> String,
{
    let txn_account_id = is_primary_signature_object.then_some(txn_account_id);

    check_sttx_multi_sign(
        signature_object,
        txn_account_id,
        |signer| {
            let account_id = signer.account_id();
            verify_signer(signer, build_message(message_seed, &account_id))
        },
        format_account_id,
    )
}

pub fn run_sttx_check_batch_multi_sign<
    AccountId,
    SignatureObject,
    Signer,
    MessageSeed,
    Message,
    BuildMessage,
    VerifySigner,
    FormatAccountId,
>(
    batch_signer: &SignatureObject,
    message_seed: &MessageSeed,
    mut build_message: BuildMessage,
    mut verify_signer: VerifySigner,
    format_account_id: FormatAccountId,
) -> Result<(), String>
where
    AccountId: Clone + Eq + Ord,
    SignatureObject: StTxMultiSignObject<AccountId, Signer>,
    Signer: StTxMultiSigner<AccountId>,
    BuildMessage: FnMut(&MessageSeed, &AccountId) -> Message,
    VerifySigner: FnMut(&Signer, Message) -> Result<(), String>,
    FormatAccountId: FnMut(&AccountId) -> String,
{
    check_sttx_multi_sign(
        batch_signer,
        None,
        |signer| {
            let account_id = signer.account_id();
            verify_signer(signer, build_message(message_seed, &account_id))
        },
        format_account_id,
    )
}

#[cfg(test)]
mod tests {
    use super::{run_sttx_check_batch_multi_sign, run_sttx_check_multi_sign};
    use crate::sttx_multi_sign::{
        EMPTY_SIGNING_PUB_KEY_ERROR, INVALID_MULTISIGNER_ERROR, StTxMultiSignObject,
        StTxMultiSigner,
    };

    #[derive(Clone, Copy)]
    struct TestSigner {
        account_id: &'static str,
    }

    impl StTxMultiSigner<&'static str> for TestSigner {
        fn account_id(&self) -> &'static str {
            self.account_id
        }
    }

    #[derive(Clone)]
    struct TestSignatureObject {
        signers_present: bool,
        txn_signature_present: bool,
        signers: Vec<TestSigner>,
    }

    impl StTxMultiSignObject<&'static str, TestSigner> for TestSignatureObject {
        type Signers = Vec<TestSigner>;

        fn signers_present(&self) -> bool {
            self.signers_present
        }

        fn txn_signature_present(&self) -> bool {
            self.txn_signature_present
        }

        fn signers(&self) -> Self::Signers {
            self.signers.clone()
        }
    }

    #[test]
    fn sttx_check_multi_sign_rejects_primary_self_multisigning() {
        let result = run_sttx_check_multi_sign(
            &TestSignatureObject {
                signers_present: true,
                txn_signature_present: false,
                signers: vec![TestSigner {
                    account_id: "alice",
                }],
            },
            true,
            &"alice",
            &"seed",
            |seed, account_id| format!("{seed}-{account_id}"),
            |_, _| Ok(()),
            |account_id| (*account_id).to_owned(),
        );

        assert_eq!(result, Err(INVALID_MULTISIGNER_ERROR.to_owned()));
    }

    #[test]
    fn sttx_check_multi_sign_skips_self_multisigning_rejection_for_non_primary_objects() {
        let mut seen = Vec::new();

        let result = run_sttx_check_multi_sign(
            &TestSignatureObject {
                signers_present: true,
                txn_signature_present: false,
                signers: vec![TestSigner {
                    account_id: "alice",
                }],
            },
            false,
            &"alice",
            &"seed",
            |seed, account_id| format!("{seed}-{account_id}"),
            |_, message| {
                seen.push(message);
                Ok(())
            },
            |account_id| (*account_id).to_owned(),
        );

        assert_eq!(result, Ok(()));
        assert_eq!(seen, vec!["seed-alice".to_owned()]);
    }

    #[test]
    fn sttx_check_batch_multi_sign_passes_built_messages_without_self_multisign_rejection() {
        let mut seen = Vec::new();

        let result = run_sttx_check_batch_multi_sign(
            &TestSignatureObject {
                signers_present: true,
                txn_signature_present: false,
                signers: vec![TestSigner {
                    account_id: "alice",
                }],
            },
            &"batch",
            |seed, account_id| format!("{seed}-{account_id}"),
            |_, message| {
                seen.push(message);
                Ok(())
            },
            |account_id| (*account_id).to_owned(),
        );

        assert_eq!(result, Ok(()));
        assert_eq!(seen, vec!["batch-alice".to_owned()]);
    }

    #[test]
    fn sttx_check_multi_sign_returns_lower_helper_failure_unchanged() {
        let result = run_sttx_check_multi_sign(
            &TestSignatureObject {
                signers_present: false,
                txn_signature_present: false,
                signers: vec![],
            },
            true,
            &"alice",
            &"seed",
            |seed, account_id| format!("{seed}-{account_id}"),
            |_, _| Ok(()),
            |account_id| (*account_id).to_owned(),
        );

        assert_eq!(result, Err(EMPTY_SIGNING_PUB_KEY_ERROR.to_owned()));
    }
}
