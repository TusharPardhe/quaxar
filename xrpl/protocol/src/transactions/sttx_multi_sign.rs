//! Current the reference implementation surface.
//!
//! This ports the deterministic outer behavior around:
//!
//! - missing `sfSigners` rejection for empty `SigningPubKey`,
//! - rejecting mixed single- and multi-sign fields,
//! - signer-count bounds,
//! - self-signing rejection when the owning txn account is known,
//! - duplicate and unsorted signer rejection, and
//! - the current per-signer invalid-signature error text.

use std::panic::{AssertUnwindSafe, catch_unwind};

pub const EMPTY_SIGNING_PUB_KEY_ERROR: &str = "Empty SigningPubKey.";
pub const INVALID_SIGNERS_ARRAY_SIZE_ERROR: &str = "Invalid Signers array size.";
pub const INVALID_MULTISIGNER_ERROR: &str = "Invalid multisigner.";
pub const DUPLICATE_SIGNERS_ERROR: &str = "Duplicate Signers not allowed.";
pub const UNSORTED_SIGNERS_ERROR: &str = "Unsorted Signers array.";
pub const MIN_MULTI_SIGNERS: usize = 1;
pub const MAX_MULTI_SIGNERS: usize = 32;

pub trait StTxMultiSigner<AccountId> {
    fn account_id(&self) -> AccountId;
}

pub trait StTxMultiSignObject<AccountId, Signer> {
    type Signers: IntoIterator<Item = Signer>;

    fn signers_present(&self) -> bool;
    fn txn_signature_present(&self) -> bool;
    fn signers(&self) -> Self::Signers;
}

fn panic_message(payload: Box<dyn std::any::Any + Send>) -> String {
    if let Some(message) = payload.downcast_ref::<&'static str>() {
        return (*message).to_owned();
    }
    if let Some(message) = payload.downcast_ref::<String>() {
        return message.clone();
    }
    String::new()
}

fn invalid_signature_error<AccountId, FormatAccountId>(
    account_id: &AccountId,
    error_what: &str,
    mut format_account_id: FormatAccountId,
) -> String
where
    FormatAccountId: FnMut(&AccountId) -> String,
{
    format!(
        "Invalid signature on account {}{}.",
        format_account_id(account_id),
        error_what
    )
}

pub fn check_sttx_multi_sign<AccountId, SignatureObject, Signer, VerifySigner, FormatAccountId>(
    signature_object: &SignatureObject,
    txn_account_id: Option<&AccountId>,
    mut verify_signer: VerifySigner,
    mut format_account_id: FormatAccountId,
) -> Result<(), String>
where
    AccountId: Clone + Eq + Ord,
    SignatureObject: StTxMultiSignObject<AccountId, Signer>,
    Signer: StTxMultiSigner<AccountId>,
    VerifySigner: FnMut(&Signer) -> Result<(), String>,
    FormatAccountId: FnMut(&AccountId) -> String,
{
    if !signature_object.signers_present() {
        return Err(EMPTY_SIGNING_PUB_KEY_ERROR.to_owned());
    }

    if signature_object.txn_signature_present() {
        return Err(super::sttx_single_sign::CANNOT_BOTH_SINGLE_AND_MULTI_SIGN_ERROR.to_owned());
    }

    let signers: Vec<_> = signature_object.signers().into_iter().collect();
    if signers.len() < MIN_MULTI_SIGNERS || signers.len() > MAX_MULTI_SIGNERS {
        return Err(INVALID_SIGNERS_ARRAY_SIZE_ERROR.to_owned());
    }

    let mut last_account_id: Option<AccountId> = None;

    for signer in &signers {
        let account_id = signer.account_id();

        if txn_account_id == Some(&account_id) {
            return Err(INVALID_MULTISIGNER_ERROR.to_owned());
        }

        if last_account_id.as_ref() == Some(&account_id) {
            return Err(DUPLICATE_SIGNERS_ERROR.to_owned());
        }

        if last_account_id
            .as_ref()
            .is_some_and(|last| last > &account_id)
        {
            return Err(UNSORTED_SIGNERS_ERROR.to_owned());
        }

        last_account_id = Some(account_id.clone());

        let verify_result = catch_unwind(AssertUnwindSafe(|| verify_signer(signer)))
            .unwrap_or_else(|payload| Err(panic_message(payload)));

        if let Err(error_what) = verify_result {
            return Err(invalid_signature_error(
                &account_id,
                &error_what,
                &mut format_account_id,
            ));
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        DUPLICATE_SIGNERS_ERROR, EMPTY_SIGNING_PUB_KEY_ERROR, INVALID_MULTISIGNER_ERROR,
        INVALID_SIGNERS_ARRAY_SIZE_ERROR, MAX_MULTI_SIGNERS, StTxMultiSignObject, StTxMultiSigner,
        UNSORTED_SIGNERS_ERROR, check_sttx_multi_sign,
    };
    use crate::sttx_single_sign::CANNOT_BOTH_SINGLE_AND_MULTI_SIGN_ERROR;

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
    fn sttx_multi_sign_requires_signers_for_empty_signing_pub_key() {
        let result = check_sttx_multi_sign(
            &TestSignatureObject {
                signers_present: false,
                txn_signature_present: false,
                signers: vec![],
            },
            None,
            |_| Ok(()),
            |account_id| (*account_id).to_owned(),
        );

        assert_eq!(result, Err(EMPTY_SIGNING_PUB_KEY_ERROR.to_owned()));
    }

    #[test]
    fn sttx_multi_sign_rejects_single_and_multi_sign_mix() {
        let result = check_sttx_multi_sign(
            &TestSignatureObject {
                signers_present: true,
                txn_signature_present: true,
                signers: vec![TestSigner {
                    account_id: "alice",
                }],
            },
            None,
            |_| Ok(()),
            |account_id| (*account_id).to_owned(),
        );

        assert_eq!(
            result,
            Err(CANNOT_BOTH_SINGLE_AND_MULTI_SIGN_ERROR.to_owned())
        );
    }

    #[test]
    fn sttx_multi_sign_rejects_invalid_signers_array_size() {
        let too_many_signers = (0..=MAX_MULTI_SIGNERS)
            .map(|_| TestSigner {
                account_id: "alice",
            })
            .collect::<Vec<_>>();

        let empty_result = check_sttx_multi_sign(
            &TestSignatureObject {
                signers_present: true,
                txn_signature_present: false,
                signers: vec![],
            },
            None,
            |_| Ok(()),
            |account_id| (*account_id).to_owned(),
        );

        let too_many_result = check_sttx_multi_sign(
            &TestSignatureObject {
                signers_present: true,
                txn_signature_present: false,
                signers: too_many_signers,
            },
            None,
            |_| Ok(()),
            |account_id| (*account_id).to_owned(),
        );

        assert_eq!(
            empty_result,
            Err(INVALID_SIGNERS_ARRAY_SIZE_ERROR.to_owned())
        );
        assert_eq!(
            too_many_result,
            Err(INVALID_SIGNERS_ARRAY_SIZE_ERROR.to_owned())
        );
    }

    #[test]
    fn sttx_multi_sign_rejects_invalid_multisigner() {
        let result = check_sttx_multi_sign(
            &TestSignatureObject {
                signers_present: true,
                txn_signature_present: false,
                signers: vec![TestSigner {
                    account_id: "alice",
                }],
            },
            Some(&"alice"),
            |_| Ok(()),
            |account_id| (*account_id).to_owned(),
        );

        assert_eq!(result, Err(INVALID_MULTISIGNER_ERROR.to_owned()));
    }

    #[test]
    fn sttx_multi_sign_rejects_duplicate_signers() {
        let result = check_sttx_multi_sign(
            &TestSignatureObject {
                signers_present: true,
                txn_signature_present: false,
                signers: vec![
                    TestSigner {
                        account_id: "alice",
                    },
                    TestSigner {
                        account_id: "alice",
                    },
                ],
            },
            None,
            |_| Ok(()),
            |account_id| (*account_id).to_owned(),
        );

        assert_eq!(result, Err(DUPLICATE_SIGNERS_ERROR.to_owned()));
    }

    #[test]
    fn sttx_multi_sign_rejects_unsorted_signers() {
        let result = check_sttx_multi_sign(
            &TestSignatureObject {
                signers_present: true,
                txn_signature_present: false,
                signers: vec![
                    TestSigner { account_id: "bob" },
                    TestSigner {
                        account_id: "alice",
                    },
                ],
            },
            None,
            |_| Ok(()),
            |account_id| (*account_id).to_owned(),
        );

        assert_eq!(result, Err(UNSORTED_SIGNERS_ERROR.to_owned()));
    }

    #[test]
    fn sttx_multi_sign_formats_invalid_signature_without_extra_error() {
        let result = check_sttx_multi_sign(
            &TestSignatureObject {
                signers_present: true,
                txn_signature_present: false,
                signers: vec![TestSigner {
                    account_id: "alice",
                }],
            },
            None,
            |_| Err(String::new()),
            |account_id| format!("r{account_id}"),
        );

        assert_eq!(
            result,
            Err("Invalid signature on account ralice.".to_owned())
        );
    }

    #[test]
    fn sttx_multi_sign_appends_verifier_error_text() {
        let result = check_sttx_multi_sign(
            &TestSignatureObject {
                signers_present: true,
                txn_signature_present: false,
                signers: vec![TestSigner {
                    account_id: "alice",
                }],
            },
            None,
            |_| Err(" detail".to_owned()),
            |account_id| format!("r{account_id}"),
        );

        assert_eq!(
            result,
            Err("Invalid signature on account ralice detail.".to_owned())
        );
    }

    #[test]
    fn sttx_multi_sign_accepts_sorted_verified_signers() {
        let result = check_sttx_multi_sign(
            &TestSignatureObject {
                signers_present: true,
                txn_signature_present: false,
                signers: vec![
                    TestSigner {
                        account_id: "alice",
                    },
                    TestSigner { account_id: "bob" },
                ],
            },
            Some(&"carol"),
            |_| Ok(()),
            |account_id| format!("r{account_id}"),
        );

        assert_eq!(result, Ok(()));
    }
}
