//! Current the reference implementation single-sign wrapper shells.
//!
//! This ports the deterministic wrapper behavior around:
//!
//! - `STTx::checkSingleSign(...)` forwarding the current signing data into the
//!   shared single-sign helper, and
//! - `STTx::checkBatchSingleSign(...)` forwarding the current batch message
//!   into that same helper.

use crate::sttx_single_sign::{StTxSingleSignObject, check_sttx_single_sign};

pub fn run_sttx_check_single_sign<SignatureObject, SigningData, VerifySignature>(
    signature_object: &SignatureObject,
    signing_data: &SigningData,
    verify_signature: VerifySignature,
) -> Result<(), String>
where
    SignatureObject: StTxSingleSignObject,
    VerifySignature: FnOnce(&SignatureObject, &SigningData) -> bool,
{
    check_sttx_single_sign(signature_object, |signature_object| {
        verify_signature(signature_object, signing_data)
    })
}

pub fn run_sttx_check_batch_single_sign<SignatureObject, BatchMessage, VerifySignature>(
    batch_signer: &SignatureObject,
    batch_message: &BatchMessage,
    verify_signature: VerifySignature,
) -> Result<(), String>
where
    SignatureObject: StTxSingleSignObject,
    VerifySignature: FnOnce(&SignatureObject, &BatchMessage) -> bool,
{
    check_sttx_single_sign(batch_signer, |batch_signer| {
        verify_signature(batch_signer, batch_message)
    })
}

#[cfg(test)]
mod tests {
    use super::{run_sttx_check_batch_single_sign, run_sttx_check_single_sign};
    use crate::sttx_single_sign::{CANNOT_BOTH_SINGLE_AND_MULTI_SIGN_ERROR, StTxSingleSignObject};

    #[derive(Clone, Copy)]
    struct TestSignatureObject {
        signers_present: bool,
    }

    impl StTxSingleSignObject for TestSignatureObject {
        fn signers_present(&self) -> bool {
            self.signers_present
        }
    }

    #[test]
    fn sttx_check_single_sign_passes_signing_data_to_verifier() {
        let mut seen = None;

        let result = run_sttx_check_single_sign(
            &TestSignatureObject {
                signers_present: false,
            },
            &"txn-signing-data",
            |_, signing_data| {
                seen = Some(*signing_data);
                true
            },
        );

        assert_eq!(result, Ok(()));
        assert_eq!(seen, Some("txn-signing-data"));
    }

    #[test]
    fn sttx_check_batch_single_sign_passes_batch_message_to_verifier() {
        let mut seen = None;

        let result = run_sttx_check_batch_single_sign(
            &TestSignatureObject {
                signers_present: false,
            },
            &"batch-message",
            |_, batch_message| {
                seen = Some(*batch_message);
                true
            },
        );

        assert_eq!(result, Ok(()));
        assert_eq!(seen, Some("batch-message"));
    }

    #[test]
    fn sttx_check_single_sign_returns_lower_helper_failure_unchanged() {
        let result = run_sttx_check_single_sign(
            &TestSignatureObject {
                signers_present: true,
            },
            &"txn-signing-data",
            |_, _| true,
        );

        assert_eq!(
            result,
            Err(CANNOT_BOTH_SINGLE_AND_MULTI_SIGN_ERROR.to_owned())
        );
    }
}
