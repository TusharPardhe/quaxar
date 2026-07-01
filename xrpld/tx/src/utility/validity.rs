//! Current `xrpl/tx/apply.h` validity surface.
//!
//! This ports the deterministic control flow around `checkValidity(...)` and
//! the flag-promotion logic used by `forceValidity(...)`.

use protocol::{Rules, feature_batch, feature_batch_v1_1, fix_batch_inner_sigs};
use xrpl_core::{HashRouterFlags, any, merge_set_flags};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Validity {
    SigBad,
    SigGoodOnly,
    Valid,
}

const SF_SIGGOOD: HashRouterFlags = HashRouterFlags::PRIVATE2;
const SF_SIGBAD: HashRouterFlags = HashRouterFlags::PRIVATE1;
const SF_LOCALBAD: HashRouterFlags = HashRouterFlags::PRIVATE3;
const SF_LOCALGOOD: HashRouterFlags = HashRouterFlags::PRIVATE4;

pub const INVALID_INNER_BATCH_TRANSACTION_REASON: &str =
    "Malformed: Invalid inner batch transaction.";
pub const BAD_SIGNATURE_REASON: &str = "Transaction has bad signature.";
pub const LOCAL_CHECKS_FAILED_REASON: &str = "Local checks failed.";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CheckValidityFacts {
    pub inner_batch_flag_set: bool,
    pub txn_signature_present: bool,
    pub signing_pub_key_empty: bool,
    pub signers_present: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckValidityResult {
    pub validity: Validity,
    pub reason: String,
    pub flags_to_set: HashRouterFlags,
}

impl CheckValidityResult {
    fn new(validity: Validity, reason: impl Into<String>, flags_to_set: HashRouterFlags) -> Self {
        Self {
            validity,
            reason: reason.into(),
            flags_to_set,
        }
    }
}

pub fn run_check_validity(
    current_flags: HashRouterFlags,
    facts: CheckValidityFacts,
    rules: &Rules,
    check_sign: impl FnOnce() -> Result<(), String>,
    passes_local_checks: impl FnOnce() -> Result<(), String>,
) -> CheckValidityResult {
    if facts.inner_batch_flag_set
        && (rules.enabled(&feature_batch()) || rules.enabled(&feature_batch_v1_1()))
    {
        if facts.txn_signature_present || !facts.signing_pub_key_empty || facts.signers_present {
            return CheckValidityResult::new(
                Validity::SigBad,
                INVALID_INNER_BATCH_TRANSACTION_REASON,
                HashRouterFlags::UNDEFINED,
            );
        }

        if !rules.enabled(&fix_batch_inner_sigs()) {
            return match passes_local_checks() {
                Ok(()) => CheckValidityResult::new(Validity::Valid, "", SF_SIGGOOD),
                Err(reason) => CheckValidityResult::new(Validity::SigGoodOnly, reason, SF_LOCALBAD),
            };
        }
    }

    let mut flags_to_set = HashRouterFlags::UNDEFINED;

    if any(current_flags & SF_SIGBAD) {
        return CheckValidityResult::new(Validity::SigBad, BAD_SIGNATURE_REASON, flags_to_set);
    }

    if !any(current_flags & SF_SIGGOOD) {
        match check_sign() {
            Ok(()) => flags_to_set |= SF_SIGGOOD,
            Err(reason) => {
                return CheckValidityResult::new(Validity::SigBad, reason, SF_SIGBAD);
            }
        }
    }

    if any(current_flags & SF_LOCALBAD) {
        return CheckValidityResult::new(
            Validity::SigGoodOnly,
            LOCAL_CHECKS_FAILED_REASON,
            flags_to_set,
        );
    }

    if any(current_flags & SF_LOCALGOOD) {
        return CheckValidityResult::new(Validity::Valid, "", flags_to_set);
    }

    match passes_local_checks() {
        Ok(()) => {
            flags_to_set |= SF_LOCALGOOD;
            CheckValidityResult::new(Validity::Valid, "", flags_to_set)
        }
        Err(reason) => {
            flags_to_set |= SF_LOCALBAD;
            CheckValidityResult::new(Validity::SigGoodOnly, reason, flags_to_set)
        }
    }
}

pub fn run_check_validity_with_flag_cache(
    facts: CheckValidityFacts,
    rules: &Rules,
    get_flags: impl FnOnce() -> HashRouterFlags,
    set_flags: impl FnOnce(HashRouterFlags),
    check_sign: impl FnOnce() -> Result<(), String>,
    passes_local_checks: impl FnOnce() -> Result<(), String>,
) -> CheckValidityResult {
    let result = run_check_validity(get_flags(), facts, rules, check_sign, passes_local_checks);

    if any(result.flags_to_set) {
        set_flags(result.flags_to_set);
    }

    result
}

pub fn forced_validity_flags(validity: Validity) -> HashRouterFlags {
    match validity {
        Validity::SigBad => HashRouterFlags::UNDEFINED,
        Validity::SigGoodOnly => SF_SIGGOOD,
        Validity::Valid => SF_SIGGOOD | SF_LOCALGOOD,
    }
}

pub fn run_force_validity(validity: Validity, set_flags: impl FnOnce(HashRouterFlags)) -> bool {
    let flags = forced_validity_flags(validity);
    if !any(flags) {
        return false;
    }

    set_flags(flags);
    true
}

/// Mirrors the deterministic merge rule of `forceValidity(...)` after the
/// caller has already identified a transaction hash.
///
/// Returns the merged flags plus whether the cache entry changed.
pub fn merge_forced_validity(
    current: HashRouterFlags,
    validity: Validity,
) -> (HashRouterFlags, bool) {
    let flags = forced_validity_flags(validity);
    if !any(flags) {
        return (current, false);
    }

    merge_set_flags(current, flags)
}

#[cfg(test)]
mod tests {
    use super::{
        BAD_SIGNATURE_REASON, CheckValidityFacts, CheckValidityResult,
        INVALID_INNER_BATCH_TRANSACTION_REASON, LOCAL_CHECKS_FAILED_REASON, Validity,
        forced_validity_flags, merge_forced_validity, run_check_validity,
        run_check_validity_with_flag_cache, run_force_validity,
    };
    use protocol::{Rules, feature_batch, fix_batch_inner_sigs};
    use std::cell::Cell;
    use xrpl_core::HashRouterFlags;

    #[test]
    fn forced_validity_flags_match_current_cpp_promotion_rules() {
        assert_eq!(
            forced_validity_flags(Validity::SigBad),
            HashRouterFlags::UNDEFINED
        );
        assert_eq!(
            forced_validity_flags(Validity::SigGoodOnly),
            HashRouterFlags::PRIVATE2
        );
        assert_eq!(
            forced_validity_flags(Validity::Valid),
            HashRouterFlags::PRIVATE2 | HashRouterFlags::PRIVATE4
        );
    }

    #[test]
    fn merge_forced_validity_matches_hash_router_setflags_rule() {
        let (sig_good, changed) =
            merge_forced_validity(HashRouterFlags::UNDEFINED, Validity::SigGoodOnly);
        assert_eq!(sig_good, HashRouterFlags::PRIVATE2);
        assert!(changed);

        let (valid, changed_again) = merge_forced_validity(sig_good, Validity::Valid);
        assert_eq!(valid, HashRouterFlags::PRIVATE2 | HashRouterFlags::PRIVATE4);
        assert!(changed_again);

        let (unchanged, changed_final) = merge_forced_validity(valid, Validity::Valid);
        assert_eq!(unchanged, valid);
        assert!(!changed_final);
    }

    #[test]
    fn merge_forced_validity_ignores_sigbad_force_validity() {
        let current = HashRouterFlags::BAD | HashRouterFlags::PRIVATE2;
        let (merged, changed) = merge_forced_validity(current, Validity::SigBad);

        assert_eq!(merged, current);
        assert!(!changed);
    }

    #[test]
    fn check_validity_with_flag_cache_sets_new_flags_once() {
        let seen = Cell::new(HashRouterFlags::UNDEFINED.bits());

        let result = run_check_validity_with_flag_cache(
            CheckValidityFacts::default(),
            &Rules::new(std::iter::empty()),
            || HashRouterFlags::UNDEFINED,
            |flags| seen.set(flags.bits()),
            || Ok(()),
            || Ok(()),
        );

        assert_eq!(
            result,
            CheckValidityResult::new(
                Validity::Valid,
                "",
                HashRouterFlags::PRIVATE2 | HashRouterFlags::PRIVATE4
            )
        );
        assert_eq!(
            seen.get(),
            (HashRouterFlags::PRIVATE2 | HashRouterFlags::PRIVATE4).bits()
        );
    }

    #[test]
    fn check_validity_with_flag_cache_skips_set_flags_when_nothing_new() {
        let set_called = Cell::new(false);

        let result = run_check_validity_with_flag_cache(
            CheckValidityFacts::default(),
            &Rules::new(std::iter::empty()),
            || HashRouterFlags::PRIVATE1,
            |_flags| set_called.set(true),
            || Ok(()),
            || Ok(()),
        );

        assert_eq!(
            result,
            CheckValidityResult::new(
                Validity::SigBad,
                BAD_SIGNATURE_REASON,
                HashRouterFlags::UNDEFINED
            )
        );
        assert!(!set_called.get());
    }

    #[test]
    fn run_force_validity_sets_siggoodonly_flags() {
        let seen = Cell::new(HashRouterFlags::UNDEFINED.bits());

        let changed = run_force_validity(Validity::SigGoodOnly, |flags| seen.set(flags.bits()));

        assert!(changed);
        assert_eq!(seen.get(), HashRouterFlags::PRIVATE2.bits());
    }

    #[test]
    fn run_force_validity_skips_sigbad() {
        let set_called = Cell::new(false);

        let changed = run_force_validity(Validity::SigBad, |_flags| set_called.set(true));

        assert!(!changed);
        assert!(!set_called.get());
    }

    #[test]
    fn check_validity_rejects_signed_inner_batch_transactions_before_cache_checks() {
        let result = run_check_validity(
            HashRouterFlags::PRIVATE1,
            CheckValidityFacts {
                inner_batch_flag_set: true,
                txn_signature_present: true,
                signing_pub_key_empty: true,
                signers_present: false,
            },
            &Rules::new([feature_batch()]),
            || Ok(()),
            || Ok(()),
        );

        assert_eq!(
            result,
            CheckValidityResult::new(
                Validity::SigBad,
                INVALID_INNER_BATCH_TRANSACTION_REASON,
                HashRouterFlags::UNDEFINED
            )
        );
    }

    #[test]
    fn check_validity_inner_batch_before_fix_skips_signature_check_and_returns_valid() {
        let sign_checked = Cell::new(false);

        let result = run_check_validity(
            HashRouterFlags::UNDEFINED,
            CheckValidityFacts {
                inner_batch_flag_set: true,
                txn_signature_present: false,
                signing_pub_key_empty: true,
                signers_present: false,
            },
            &Rules::new([feature_batch()]),
            || {
                sign_checked.set(true);
                Ok(())
            },
            || Ok(()),
        );

        assert_eq!(
            result,
            CheckValidityResult::new(Validity::Valid, "", HashRouterFlags::PRIVATE2)
        );
        assert!(!sign_checked.get());
    }

    #[test]
    fn check_validity_inner_batch_before_fix_returns_local_failure_without_siggood() {
        let result = run_check_validity(
            HashRouterFlags::UNDEFINED,
            CheckValidityFacts {
                inner_batch_flag_set: true,
                txn_signature_present: false,
                signing_pub_key_empty: true,
                signers_present: false,
            },
            &Rules::new([feature_batch()]),
            || Ok(()),
            || Err("inner local checks failed".to_owned()),
        );

        assert_eq!(
            result,
            CheckValidityResult::new(
                Validity::SigGoodOnly,
                "inner local checks failed",
                HashRouterFlags::PRIVATE3
            )
        );
    }

    #[test]
    fn check_validity_cached_sigbad_returns_current_cpp_reason_without_callbacks() {
        let sign_checked = Cell::new(false);
        let local_checked = Cell::new(false);

        let result = run_check_validity(
            HashRouterFlags::PRIVATE1,
            CheckValidityFacts::default(),
            &Rules::new([feature_batch(), fix_batch_inner_sigs()]),
            || {
                sign_checked.set(true);
                Ok(())
            },
            || {
                local_checked.set(true);
                Ok(())
            },
        );

        assert_eq!(
            result,
            CheckValidityResult::new(
                Validity::SigBad,
                BAD_SIGNATURE_REASON,
                HashRouterFlags::UNDEFINED
            )
        );
        assert!(!sign_checked.get());
        assert!(!local_checked.get());
    }

    #[test]
    fn check_validity_cached_localbad_returns_current_cpp_text_after_siggood_promotion() {
        let result = run_check_validity(
            HashRouterFlags::PRIVATE3,
            CheckValidityFacts::default(),
            &Rules::new(std::iter::empty()),
            || Ok(()),
            || Err("should not run".to_owned()),
        );

        assert_eq!(
            result,
            CheckValidityResult::new(
                Validity::SigGoodOnly,
                LOCAL_CHECKS_FAILED_REASON,
                HashRouterFlags::PRIVATE2
            )
        );
    }

    #[test]
    fn check_validity_sets_sigbad_when_signature_verification_fails() {
        let result = run_check_validity(
            HashRouterFlags::UNDEFINED,
            CheckValidityFacts::default(),
            &Rules::new([feature_batch(), fix_batch_inner_sigs()]),
            || Err("bad signature from callback".to_owned()),
            || Ok(()),
        );

        assert_eq!(
            result,
            CheckValidityResult::new(
                Validity::SigBad,
                "bad signature from callback",
                HashRouterFlags::PRIVATE1
            )
        );
    }

    #[test]
    fn check_validity_sets_siggood_and_localgood_on_fresh_success() {
        let result = run_check_validity(
            HashRouterFlags::UNDEFINED,
            CheckValidityFacts::default(),
            &Rules::new(std::iter::empty()),
            || Ok(()),
            || Ok(()),
        );

        assert_eq!(
            result,
            CheckValidityResult::new(
                Validity::Valid,
                "",
                HashRouterFlags::PRIVATE2 | HashRouterFlags::PRIVATE4
            )
        );
    }
}
