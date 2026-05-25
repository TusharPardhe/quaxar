//! Current the reference implementation surface.
//!
//! This ports the deterministic outer behavior around:
//!
//! - rejecting non-`ttBATCH` transactions,
//! - iterating the current `sfBatchSigners` array in order,
//! - selecting batch multi-sign verification when `SigningPubKey` is empty,
//! - selecting batch single-sign verification otherwise,
//! - returning the first helper failure unchanged, and
//! - mapping signer-access or helper panics to the current internal-failure
//!   string.

use std::panic::{AssertUnwindSafe, catch_unwind};

use crate::TxType;
use crate::sttx_multi_sign::{StTxMultiSignObject, StTxMultiSigner, check_sttx_multi_sign};
use crate::sttx_single_sign::{StTxSingleSignObject, check_sttx_single_sign};

pub const NOT_A_BATCH_TRANSACTION_ERROR: &str = "Not a batch transaction.";
pub const INTERNAL_BATCH_SIGNATURE_CHECK_FAILURE: &str = "Internal batch signature check failure.";

pub trait BatchSigner<AccountId, Signer>:
    StTxSingleSignObject + StTxMultiSignObject<AccountId, Signer>
where
    Signer: StTxMultiSigner<AccountId>,
{
    fn signing_pub_key_is_empty(&self) -> bool;
}

pub fn check_batch_sign<
    AccountId,
    BatchSignatureObject,
    Signers,
    Signer,
    VerifySingleSign,
    VerifyMultiSigner,
    FormatAccountId,
>(
    txn_type: TxType,
    signers: Signers,
    mut verify_single_sign: VerifySingleSign,
    mut verify_multi_signer: VerifyMultiSigner,
    mut format_account_id: FormatAccountId,
) -> Result<(), String>
where
    AccountId: Clone + Eq + Ord,
    Signers: IntoIterator<Item = BatchSignatureObject>,
    BatchSignatureObject: BatchSigner<AccountId, Signer>,
    Signer: StTxMultiSigner<AccountId>,
    VerifySingleSign: FnMut(&BatchSignatureObject) -> bool,
    VerifyMultiSigner: FnMut(&Signer) -> Result<(), String>,
    FormatAccountId: FnMut(&AccountId) -> String,
{
    catch_unwind(AssertUnwindSafe(|| {
        if txn_type != TxType::BATCH {
            return Err(NOT_A_BATCH_TRANSACTION_ERROR.to_owned());
        }

        for signer in signers {
            let result = if signer.signing_pub_key_is_empty() {
                check_sttx_multi_sign(
                    &signer,
                    None,
                    &mut verify_multi_signer,
                    &mut format_account_id,
                )
            } else {
                check_sttx_single_sign(&signer, &mut verify_single_sign)
            };

            result?;
        }

        Ok(())
    }))
    .unwrap_or_else(|_| Err(INTERNAL_BATCH_SIGNATURE_CHECK_FAILURE.to_owned()))
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use super::{
        BatchSigner, INTERNAL_BATCH_SIGNATURE_CHECK_FAILURE, NOT_A_BATCH_TRANSACTION_ERROR,
        check_batch_sign,
    };
    use crate::TxType;
    use crate::sttx_multi_sign::{
        EMPTY_SIGNING_PUB_KEY_ERROR, StTxMultiSignObject, StTxMultiSigner,
    };
    use crate::sttx_single_sign::{CANNOT_BOTH_SINGLE_AND_MULTI_SIGN_ERROR, StTxSingleSignObject};

    #[derive(Clone, Copy)]
    struct TestMultiSigner {
        account_id: &'static str,
    }

    impl StTxMultiSigner<&'static str> for TestMultiSigner {
        fn account_id(&self) -> &'static str {
            self.account_id
        }
    }

    #[derive(Clone)]
    struct TestSigner {
        signing_pub_key_is_empty: bool,
        signers_present: bool,
        txn_signature_present: bool,
        signers: Vec<TestMultiSigner>,
        panic_on_pub_key_lookup: bool,
        panic_on_signers_present: bool,
    }

    impl StTxSingleSignObject for TestSigner {
        fn signers_present(&self) -> bool {
            assert!(
                !self.panic_on_signers_present,
                "simulated sfSigners access failure"
            );
            self.signers_present
        }
    }

    impl StTxMultiSignObject<&'static str, TestMultiSigner> for TestSigner {
        type Signers = Vec<TestMultiSigner>;

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

    impl BatchSigner<&'static str, TestMultiSigner> for TestSigner {
        fn signing_pub_key_is_empty(&self) -> bool {
            assert!(
                !self.panic_on_pub_key_lookup,
                "simulated signer SigningPubKey access failure"
            );
            self.signing_pub_key_is_empty
        }
    }

    #[test]
    fn check_batch_sign_rejects_non_batch_transactions() {
        let result = check_batch_sign(
            TxType::PAYMENT,
            [TestSigner {
                signing_pub_key_is_empty: false,
                signers_present: false,
                txn_signature_present: false,
                signers: vec![],
                panic_on_pub_key_lookup: false,
                panic_on_signers_present: false,
            }],
            |_| true,
            |_| Ok(()),
            |account_id| (*account_id).to_owned(),
        );

        assert_eq!(result, Err(NOT_A_BATCH_TRANSACTION_ERROR.to_owned()));
    }

    #[test]
    fn check_batch_sign_uses_batch_multi_sign_when_signing_pub_key_is_empty() {
        let multi_called = Cell::new(false);
        let single_called = Cell::new(false);

        let result = check_batch_sign(
            TxType::BATCH,
            [TestSigner {
                signing_pub_key_is_empty: true,
                signers_present: true,
                txn_signature_present: false,
                signers: vec![TestMultiSigner {
                    account_id: "alice",
                }],
                panic_on_pub_key_lookup: false,
                panic_on_signers_present: false,
            }],
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
    fn check_batch_sign_uses_batch_single_sign_when_signing_pub_key_is_present() {
        let multi_called = Cell::new(false);
        let single_called = Cell::new(false);

        let result = check_batch_sign(
            TxType::BATCH,
            [TestSigner {
                signing_pub_key_is_empty: false,
                signers_present: false,
                txn_signature_present: false,
                signers: vec![],
                panic_on_pub_key_lookup: false,
                panic_on_signers_present: false,
            }],
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
    fn check_batch_sign_returns_lower_single_sign_failure_unchanged() {
        let result = check_batch_sign(
            TxType::BATCH,
            [TestSigner {
                signing_pub_key_is_empty: false,
                signers_present: true,
                txn_signature_present: false,
                signers: vec![],
                panic_on_pub_key_lookup: false,
                panic_on_signers_present: false,
            }],
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
    fn check_batch_sign_returns_lower_multi_sign_failure_unchanged() {
        let result = check_batch_sign(
            TxType::BATCH,
            [TestSigner {
                signing_pub_key_is_empty: true,
                signers_present: false,
                txn_signature_present: false,
                signers: vec![],
                panic_on_pub_key_lookup: false,
                panic_on_signers_present: false,
            }],
            |_| true,
            |_| Ok(()),
            |account_id| (*account_id).to_owned(),
        );

        assert_eq!(result, Err(EMPTY_SIGNING_PUB_KEY_ERROR.to_owned()));
    }

    #[test]
    fn check_batch_sign_returns_first_callback_failure_unchanged() {
        let later_callback_ran = Cell::new(false);

        let result = check_batch_sign(
            TxType::BATCH,
            [
                TestSigner {
                    signing_pub_key_is_empty: false,
                    signers_present: true,
                    txn_signature_present: false,
                    signers: vec![],
                    panic_on_pub_key_lookup: false,
                    panic_on_signers_present: false,
                },
                TestSigner {
                    signing_pub_key_is_empty: true,
                    signers_present: true,
                    txn_signature_present: false,
                    signers: vec![TestMultiSigner { account_id: "bob" }],
                    panic_on_pub_key_lookup: false,
                    panic_on_signers_present: false,
                },
            ],
            |_| true,
            |_| {
                later_callback_ran.set(true);
                Ok(())
            },
            |account_id| (*account_id).to_owned(),
        );

        assert_eq!(
            result,
            Err(CANNOT_BOTH_SINGLE_AND_MULTI_SIGN_ERROR.to_owned())
        );
        assert!(!later_callback_ran.get());
    }

    #[test]
    fn check_batch_sign_maps_signer_access_panics_to_internal_failure() {
        let result = check_batch_sign(
            TxType::BATCH,
            [TestSigner {
                signing_pub_key_is_empty: false,
                signers_present: false,
                txn_signature_present: false,
                signers: vec![],
                panic_on_pub_key_lookup: true,
                panic_on_signers_present: false,
            }],
            |_| true,
            |_| Ok(()),
            |account_id| (*account_id).to_owned(),
        );

        assert_eq!(
            result,
            Err(INTERNAL_BATCH_SIGNATURE_CHECK_FAILURE.to_owned())
        );
    }

    #[test]
    fn check_batch_sign_maps_lower_helper_panics_to_internal_failure() {
        let result = check_batch_sign(
            TxType::BATCH,
            [TestSigner {
                signing_pub_key_is_empty: true,
                signers_present: true,
                txn_signature_present: false,
                signers: vec![TestMultiSigner {
                    account_id: "alice",
                }],
                panic_on_pub_key_lookup: false,
                panic_on_signers_present: true,
            }],
            |_| true,
            |_| Ok(()),
            |account_id| (*account_id).to_owned(),
        );

        assert_eq!(
            result,
            Err(INTERNAL_BATCH_SIGNATURE_CHECK_FAILURE.to_owned())
        );
    }
}
