//! Current the reference implementation surface.
//!
//! This ports the deterministic outer behavior around:
//!
//! - rejecting objects that present both single- and multi-sign fields, and
//! - mapping unknown keys, failed verification, or callback panics to the
//!   current `"Invalid signature."` result.

use std::panic::{AssertUnwindSafe, catch_unwind};

pub const CANNOT_BOTH_SINGLE_AND_MULTI_SIGN_ERROR: &str = "Cannot both single- and multi-sign.";
pub const INVALID_SIGNATURE_ERROR: &str = "Invalid signature.";

pub trait StTxSingleSignObject {
    fn signers_present(&self) -> bool;
}

pub fn check_sttx_single_sign<SignatureObject, VerifySignature>(
    signature_object: &SignatureObject,
    verify_signature: VerifySignature,
) -> Result<(), String>
where
    SignatureObject: StTxSingleSignObject,
    VerifySignature: FnOnce(&SignatureObject) -> bool,
{
    if signature_object.signers_present() {
        return Err(CANNOT_BOTH_SINGLE_AND_MULTI_SIGN_ERROR.to_owned());
    }

    let valid_sig =
        catch_unwind(AssertUnwindSafe(|| verify_signature(signature_object))).unwrap_or(false);

    if !valid_sig {
        return Err(INVALID_SIGNATURE_ERROR.to_owned());
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        CANNOT_BOTH_SINGLE_AND_MULTI_SIGN_ERROR, INVALID_SIGNATURE_ERROR, StTxSingleSignObject,
        check_sttx_single_sign,
    };

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
    fn sttx_single_sign_rejects_single_and_multi_sign_mix() {
        let result = check_sttx_single_sign(
            &TestSignatureObject {
                signers_present: true,
            },
            |_| true,
        );

        assert_eq!(
            result,
            Err(CANNOT_BOTH_SINGLE_AND_MULTI_SIGN_ERROR.to_owned())
        );
    }

    #[test]
    fn sttx_single_sign_returns_invalid_signature_on_failed_verify() {
        let result = check_sttx_single_sign(
            &TestSignatureObject {
                signers_present: false,
            },
            |_| false,
        );

        assert_eq!(result, Err(INVALID_SIGNATURE_ERROR.to_owned()));
    }

    #[test]
    fn sttx_single_sign_returns_invalid_signature_on_verify_panic() {
        let result = check_sttx_single_sign(
            &TestSignatureObject {
                signers_present: false,
            },
            |_| panic!("simulated verify failure"),
        );

        assert_eq!(result, Err(INVALID_SIGNATURE_ERROR.to_owned()));
    }
}
