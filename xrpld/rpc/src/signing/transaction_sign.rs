use tx::{CheckValidityResult, Validity};

pub const INVALID_SIGNATURE_MESSAGE: &str = "Invalid signature.";

pub fn run_transaction_sign_validity_gate(
    check_sigs: bool,
    force_sig_good_only: impl FnOnce(),
    check_validity: impl FnOnce() -> CheckValidityResult,
) -> Result<(), &'static str> {
    if !check_sigs {
        force_sig_good_only();
    }

    if check_validity().validity != Validity::Valid {
        return Err(INVALID_SIGNATURE_MESSAGE);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{INVALID_SIGNATURE_MESSAGE, run_transaction_sign_validity_gate};
    use std::cell::RefCell;
    use tx::{CheckValidityResult, Validity};
    use xrpl_core::HashRouterFlags;

    #[test]
    fn transaction_sign_force_sig_good_only_runs_before_validity_check() {
        let calls = RefCell::new(Vec::new());

        let result = run_transaction_sign_validity_gate(
            false,
            || calls.borrow_mut().push("force"),
            || {
                calls.borrow_mut().push("check");
                CheckValidityResult {
                    validity: Validity::Valid,
                    reason: String::new(),
                    flags_to_set: HashRouterFlags::UNDEFINED,
                }
            },
        );

        assert_eq!(result, Ok(()));
        assert_eq!(calls.into_inner(), vec!["force", "check"]);
    }

    #[test]
    fn transaction_sign_invalidity_maps_to_current_rpc_message() {
        let result = run_transaction_sign_validity_gate(
            true,
            || panic!("forceValidity must not run when signatures are enabled"),
            || CheckValidityResult {
                validity: Validity::SigGoodOnly,
                reason: "Local checks failed.".to_string(),
                flags_to_set: HashRouterFlags::UNDEFINED,
            },
        );

        assert_eq!(result, Err(INVALID_SIGNATURE_MESSAGE));
    }
}
