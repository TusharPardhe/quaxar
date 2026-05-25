//! Current the reference implementation helpers.
//!
//! This ports the deterministic outer behavior around:
//!
//! - selecting multi-sign verification when `SigningPubKey` is empty,
//! - selecting single-sign verification otherwise,
//! - returning the chosen helper result unchanged,
//! - mapping signer-access or helper panics to the current internal-failure
//!   string, and
//! - the higher wrapper that prefixes counterparty-signature failures with the
//!   current `"Counterparty: "` text.

use std::panic::{AssertUnwindSafe, catch_unwind};

use crate::sttx_multi_sign::{StTxMultiSignObject, StTxMultiSigner, check_sttx_multi_sign};
use crate::sttx_single_sign::{StTxSingleSignObject, check_sttx_single_sign};

pub const INTERNAL_SIGNATURE_CHECK_FAILURE: &str = "Internal signature check failure.";
pub const COUNTERPARTY_SIGNATURE_ERROR_PREFIX: &str = "Counterparty: ";

pub trait SignatureCheckObject<AccountId, Signer>:
    StTxSingleSignObject + StTxMultiSignObject<AccountId, Signer>
where
    Signer: StTxMultiSigner<AccountId>,
{
    fn signing_pub_key_is_empty(&self) -> bool;
}

pub fn check_signature<
    AccountId,
    SignatureObject,
    Signer,
    VerifySingleSign,
    VerifyMultiSigner,
    FormatAccountId,
>(
    signature_object: &SignatureObject,
    txn_account_id: Option<&AccountId>,
    verify_single_sign: VerifySingleSign,
    verify_multi_signer: VerifyMultiSigner,
    format_account_id: FormatAccountId,
) -> Result<(), String>
where
    AccountId: Clone + Eq + Ord,
    SignatureObject: SignatureCheckObject<AccountId, Signer>,
    Signer: StTxMultiSigner<AccountId>,
    VerifySingleSign: FnOnce(&SignatureObject) -> bool,
    VerifyMultiSigner: FnMut(&Signer) -> Result<(), String>,
    FormatAccountId: FnMut(&AccountId) -> String,
{
    catch_unwind(AssertUnwindSafe(|| {
        if signature_object.signing_pub_key_is_empty() {
            check_sttx_multi_sign(
                signature_object,
                txn_account_id,
                verify_multi_signer,
                format_account_id,
            )
        } else {
            check_sttx_single_sign(signature_object, verify_single_sign)
        }
    }))
    .unwrap_or_else(|_| Err(INTERNAL_SIGNATURE_CHECK_FAILURE.to_owned()))
}

pub fn check_signature_with_counterparty<SignatureObject, CheckSignature, CheckCounterparty>(
    signature_object: &SignatureObject,
    counterparty_signature_object: Option<&SignatureObject>,
    check_signature: CheckSignature,
    check_counterparty_signature: CheckCounterparty,
) -> Result<(), String>
where
    CheckSignature: FnOnce(&SignatureObject) -> Result<(), String>,
    CheckCounterparty: FnOnce(&SignatureObject) -> Result<(), String>,
{
    check_signature(signature_object)?;

    if let Some(counterparty_signature_object) = counterparty_signature_object {
        let counterparty_result = catch_unwind(AssertUnwindSafe(|| {
            check_counterparty_signature(counterparty_signature_object)
        }))
        .unwrap_or_else(|_| Err(INTERNAL_SIGNATURE_CHECK_FAILURE.to_owned()));

        counterparty_result.map_err(|err| format!("{COUNTERPARTY_SIGNATURE_ERROR_PREFIX}{err}"))?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use super::{
        COUNTERPARTY_SIGNATURE_ERROR_PREFIX, INTERNAL_SIGNATURE_CHECK_FAILURE,
        SignatureCheckObject, check_signature, check_signature_with_counterparty,
    };
    use crate::sttx_multi_sign::{
        EMPTY_SIGNING_PUB_KEY_ERROR, StTxMultiSignObject, StTxMultiSigner,
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
        signing_pub_key_is_empty: bool,
        signers_present: bool,
        txn_signature_present: bool,
        signers: Vec<TestSigner>,
        panic_on_pub_key_lookup: bool,
        panic_on_signers_present: bool,
    }

    impl crate::sttx_single_sign::StTxSingleSignObject for TestSignatureObject {
        fn signers_present(&self) -> bool {
            assert!(
                !self.panic_on_signers_present,
                "simulated sfSigners access failure"
            );
            self.signers_present
        }
    }

    impl StTxMultiSignObject<&'static str, TestSigner> for TestSignatureObject {
        type Signers = Vec<TestSigner>;

        fn signers_present(&self) -> bool {
            assert!(
                !self.panic_on_signers_present,
                "simulated sfSigners access failure"
            );
            self.signers_present
        }

        fn txn_signature_present(&self) -> bool {
            self.txn_signature_present
        }

        fn signers(&self) -> Self::Signers {
            self.signers.clone()
        }
    }

    impl SignatureCheckObject<&'static str, TestSigner> for TestSignatureObject {
        fn signing_pub_key_is_empty(&self) -> bool {
            assert!(
                !self.panic_on_pub_key_lookup,
                "simulated SigningPubKey access failure"
            );
            self.signing_pub_key_is_empty
        }
    }

    #[test]
    fn check_signature_uses_multi_sign_when_signing_pub_key_is_empty() {
        let multi_called = Cell::new(false);
        let single_called = Cell::new(false);

        let result = check_signature(
            &TestSignatureObject {
                signing_pub_key_is_empty: true,
                signers_present: true,
                txn_signature_present: false,
                signers: vec![TestSigner {
                    account_id: "alice",
                }],
                panic_on_pub_key_lookup: false,
                panic_on_signers_present: false,
            },
            Some(&"carol"),
            |_| {
                single_called.set(true);
                true
            },
            |_| {
                multi_called.set(true);
                Ok(())
            },
            |account_id| (*account_id).to_owned(),
        );

        assert_eq!(result, Ok(()));
        assert!(multi_called.get());
        assert!(!single_called.get());
    }

    #[test]
    fn check_signature_uses_single_sign_when_signing_pub_key_is_present() {
        let multi_called = Cell::new(false);
        let single_called = Cell::new(false);

        let result = check_signature(
            &TestSignatureObject {
                signing_pub_key_is_empty: false,
                signers_present: false,
                txn_signature_present: false,
                signers: vec![],
                panic_on_pub_key_lookup: false,
                panic_on_signers_present: false,
            },
            Some(&"carol"),
            |_| {
                single_called.set(true);
                true
            },
            |_| {
                multi_called.set(true);
                Ok(())
            },
            |account_id| (*account_id).to_owned(),
        );

        assert_eq!(result, Ok(()));
        assert!(!multi_called.get());
        assert!(single_called.get());
    }

    #[test]
    fn check_signature_returns_lower_single_sign_failure_unchanged() {
        let result = check_signature(
            &TestSignatureObject {
                signing_pub_key_is_empty: false,
                signers_present: true,
                txn_signature_present: false,
                signers: vec![],
                panic_on_pub_key_lookup: false,
                panic_on_signers_present: false,
            },
            None,
            |_| true,
            |_| Ok(()),
            |account_id| (*account_id).to_owned(),
        );

        assert_eq!(
            result,
            Err(CANNOT_BOTH_SINGLE_AND_MULTI_SIGN_ERROR.to_owned())
        );
    }

    #[test]
    fn check_signature_returns_lower_multi_sign_failure_unchanged() {
        let result = check_signature(
            &TestSignatureObject {
                signing_pub_key_is_empty: true,
                signers_present: false,
                txn_signature_present: false,
                signers: vec![],
                panic_on_pub_key_lookup: false,
                panic_on_signers_present: false,
            },
            None,
            |_| true,
            |_| Ok(()),
            |account_id| (*account_id).to_owned(),
        );

        assert_eq!(result, Err(EMPTY_SIGNING_PUB_KEY_ERROR.to_owned()));
    }

    #[test]
    fn check_signature_maps_signer_access_panics_to_internal_failure() {
        let result = check_signature(
            &TestSignatureObject {
                signing_pub_key_is_empty: false,
                signers_present: false,
                txn_signature_present: false,
                signers: vec![],
                panic_on_pub_key_lookup: true,
                panic_on_signers_present: false,
            },
            None,
            |_| true,
            |_| Ok(()),
            |account_id| (*account_id).to_owned(),
        );

        assert_eq!(result, Err(INTERNAL_SIGNATURE_CHECK_FAILURE.to_owned()));
    }

    #[test]
    fn check_signature_maps_lower_helper_panics_to_internal_failure() {
        let result = check_signature(
            &TestSignatureObject {
                signing_pub_key_is_empty: true,
                signers_present: true,
                txn_signature_present: false,
                signers: vec![TestSigner {
                    account_id: "alice",
                }],
                panic_on_pub_key_lookup: false,
                panic_on_signers_present: true,
            },
            None,
            |_| true,
            |_| Ok(()),
            |account_id| (*account_id).to_owned(),
        );

        assert_eq!(result, Err(INTERNAL_SIGNATURE_CHECK_FAILURE.to_owned()));
    }

    #[test]
    fn check_signature_with_counterparty_returns_primary_failure_unchanged() {
        let counterparty_checked = Cell::new(false);

        let result = check_signature_with_counterparty(
            &TestSignatureObject {
                signing_pub_key_is_empty: false,
                signers_present: false,
                txn_signature_present: false,
                signers: vec![],
                panic_on_pub_key_lookup: false,
                panic_on_signers_present: false,
            },
            Some(&TestSignatureObject {
                signing_pub_key_is_empty: false,
                signers_present: false,
                txn_signature_present: false,
                signers: vec![],
                panic_on_pub_key_lookup: false,
                panic_on_signers_present: false,
            }),
            |_| Err("Invalid signature.".to_owned()),
            |_| {
                counterparty_checked.set(true);
                Ok(())
            },
        );

        assert_eq!(result, Err("Invalid signature.".to_owned()));
        assert!(!counterparty_checked.get());
    }

    #[test]
    fn check_signature_with_counterparty_prefixes_counterparty_failures() {
        let result = check_signature_with_counterparty(
            &TestSignatureObject {
                signing_pub_key_is_empty: false,
                signers_present: false,
                txn_signature_present: false,
                signers: vec![],
                panic_on_pub_key_lookup: false,
                panic_on_signers_present: false,
            },
            Some(&TestSignatureObject {
                signing_pub_key_is_empty: false,
                signers_present: false,
                txn_signature_present: false,
                signers: vec![],
                panic_on_pub_key_lookup: false,
                panic_on_signers_present: false,
            }),
            |_| Ok(()),
            |_| Err("Invalid signature.".to_owned()),
        );

        assert_eq!(
            result,
            Err(format!(
                "{COUNTERPARTY_SIGNATURE_ERROR_PREFIX}Invalid signature."
            ))
        );
    }

    #[test]
    fn check_signature_with_counterparty_maps_panic_to_prefixed_internal_failure() {
        let result = check_signature_with_counterparty(
            &TestSignatureObject {
                signing_pub_key_is_empty: false,
                signers_present: false,
                txn_signature_present: false,
                signers: vec![],
                panic_on_pub_key_lookup: false,
                panic_on_signers_present: false,
            },
            Some(&TestSignatureObject {
                signing_pub_key_is_empty: false,
                signers_present: false,
                txn_signature_present: false,
                signers: vec![],
                panic_on_pub_key_lookup: false,
                panic_on_signers_present: false,
            }),
            |_| Ok(()),
            |_| panic!("simulated counterparty verification failure"),
        );

        assert_eq!(
            result,
            Err(format!(
                "{COUNTERPARTY_SIGNATURE_ERROR_PREFIX}{INTERNAL_SIGNATURE_CHECK_FAILURE}"
            ))
        );
    }

    #[test]
    fn check_signature_with_counterparty_succeeds_without_counterparty() {
        let result = check_signature_with_counterparty(
            &TestSignatureObject {
                signing_pub_key_is_empty: false,
                signers_present: false,
                txn_signature_present: false,
                signers: vec![],
                panic_on_pub_key_lookup: false,
                panic_on_signers_present: false,
            },
            None,
            |_| Ok(()),
            |_| Err("counterparty callback should not run".to_owned()),
        );

        assert_eq!(result, Ok(()));
    }
}
