//! Integration tests that pin the narrowed Rust `apply.cpp::checkValidity(...)`
//! shell to the current C++ behavior.

use std::cell::Cell;

use protocol::{Rules, feature_batch, fix_batch_inner_sigs};
use tx::{
    BAD_SIGNATURE_REASON, CheckValidityFacts, CheckValidityResult,
    INVALID_INNER_BATCH_TRANSACTION_REASON, LOCAL_CHECKS_FAILED_REASON, Validity,
    run_check_validity, run_check_validity_with_flag_cache, run_force_validity,
};
use xrpl_core::HashRouterFlags;

#[test]
fn tx_check_validity_rejects_signed_inner_batch_transaction() {
    let result = run_check_validity(
        HashRouterFlags::UNDEFINED,
        CheckValidityFacts {
            inner_batch_flag_set: true,
            txn_signature_present: true,
            signing_pub_key_empty: true,
            signers_present: false,
            alternate_signature_present: false,
        },
        &Rules::new([feature_batch()]),
        || Ok(()),
        || Ok(()),
    );

    assert_eq!(
        result,
        CheckValidityResult {
            validity: Validity::SigBad,
            reason: INVALID_INNER_BATCH_TRANSACTION_REASON.to_owned(),
            flags_to_set: HashRouterFlags::UNDEFINED,
        }
    );
}

#[test]
fn tx_check_validity_inner_batch_before_fix_skips_signature_check() {
    let sign_checked = Cell::new(false);

    let result = run_check_validity(
        HashRouterFlags::UNDEFINED,
        CheckValidityFacts {
            inner_batch_flag_set: true,
            txn_signature_present: false,
            signing_pub_key_empty: true,
            signers_present: false,
            alternate_signature_present: false,
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
        CheckValidityResult {
            validity: Validity::Valid,
            reason: String::new(),
            flags_to_set: HashRouterFlags::PRIVATE2,
        }
    );
    assert!(!sign_checked.get());
}

#[test]
fn tx_check_validity_cached_sigbad_uses_fixed_reason() {
    let result = run_check_validity(
        HashRouterFlags::PRIVATE1,
        CheckValidityFacts::default(),
        &Rules::new(std::iter::empty()),
        || Err("should not run".to_owned()),
        || Err("should not run".to_owned()),
    );

    assert_eq!(
        result,
        CheckValidityResult {
            validity: Validity::SigBad,
            reason: BAD_SIGNATURE_REASON.to_owned(),
            flags_to_set: HashRouterFlags::UNDEFINED,
        }
    );
}

#[test]
fn tx_check_validity_cached_localbad_returns_local_checks_failed() {
    let result = run_check_validity(
        HashRouterFlags::PRIVATE3,
        CheckValidityFacts::default(),
        &Rules::new(std::iter::empty()),
        || Ok(()),
        || Err("should not run".to_owned()),
    );

    assert_eq!(
        result,
        CheckValidityResult {
            validity: Validity::SigGoodOnly,
            reason: LOCAL_CHECKS_FAILED_REASON.to_owned(),
            flags_to_set: HashRouterFlags::PRIVATE2,
        }
    );
}

#[test]
fn tx_check_validity_after_inner_sig_fix_runs_signature_check() {
    let local_checked = Cell::new(false);

    let result = run_check_validity(
        HashRouterFlags::UNDEFINED,
        CheckValidityFacts {
            inner_batch_flag_set: true,
            txn_signature_present: false,
            signing_pub_key_empty: true,
            signers_present: false,
            alternate_signature_present: false,
        },
        &Rules::new([feature_batch(), fix_batch_inner_sigs()]),
        || Err("inner signature invalid".to_owned()),
        || {
            local_checked.set(true);
            Ok(())
        },
    );

    assert_eq!(
        result,
        CheckValidityResult {
            validity: Validity::SigBad,
            reason: "inner signature invalid".to_owned(),
            flags_to_set: HashRouterFlags::PRIVATE1,
        }
    );
    assert!(!local_checked.get());
}

#[test]
fn tx_check_validity_fresh_success_sets_siggood_and_localgood() {
    let result = run_check_validity(
        HashRouterFlags::UNDEFINED,
        CheckValidityFacts::default(),
        &Rules::new(std::iter::empty()),
        || Ok(()),
        || Ok(()),
    );

    assert_eq!(
        result,
        CheckValidityResult {
            validity: Validity::Valid,
            reason: String::new(),
            flags_to_set: HashRouterFlags::PRIVATE2 | HashRouterFlags::PRIVATE4,
        }
    );
}

#[test]
fn tx_check_validity_with_flag_cache_writes_new_flags_once() {
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
        CheckValidityResult {
            validity: Validity::Valid,
            reason: String::new(),
            flags_to_set: HashRouterFlags::PRIVATE2 | HashRouterFlags::PRIVATE4,
        }
    );
    assert_eq!(
        seen.get(),
        (HashRouterFlags::PRIVATE2 | HashRouterFlags::PRIVATE4).bits()
    );
}

#[test]
fn tx_force_validity_sets_only_promoted_flags() {
    let seen = Cell::new(HashRouterFlags::UNDEFINED.bits());

    let changed = run_force_validity(Validity::Valid, |flags| seen.set(flags.bits()));

    assert!(changed);
    assert_eq!(
        seen.get(),
        (HashRouterFlags::PRIVATE2 | HashRouterFlags::PRIVATE4 | HashRouterFlags::PRIVATE8).bits()
    );
}

#[test]
fn tx_force_validity_ignores_sigbad() {
    let set_called = Cell::new(false);

    let changed = run_force_validity(Validity::SigBad, |_flags| set_called.set(true));

    assert!(!changed);
    assert!(!set_called.get());
}

#[test]
fn tx_role_signature_validity_cache_is_partitioned_by_era_in_both_directions() {
    let alternate_role_signature = CheckValidityFacts {
        alternate_signature_present: true,
        ..CheckValidityFacts::default()
    };
    let legacy = Rules::default();
    let fixed = Rules::new([protocol::fix_cleanup_3_4_0()]);

    let legacy_cached = run_check_validity(
        HashRouterFlags::UNDEFINED,
        alternate_role_signature,
        &legacy,
        || Ok(()),
        || Ok(()),
    );
    assert_eq!(
        legacy_cached.flags_to_set,
        HashRouterFlags::PRIVATE8 | HashRouterFlags::PRIVATE4
    );
    let fixed_checks = Cell::new(0);
    let fixed_after_legacy = run_check_validity(
        legacy_cached.flags_to_set,
        alternate_role_signature,
        &fixed,
        || {
            fixed_checks.set(fixed_checks.get() + 1);
            Ok(())
        },
        || panic!("the cached local validity may be reused, not role signature validity"),
    );
    assert_eq!(fixed_checks.get(), 1);
    assert_eq!(fixed_after_legacy.flags_to_set, HashRouterFlags::PRIVATE2);

    let fixed_cached = run_check_validity(
        HashRouterFlags::UNDEFINED,
        alternate_role_signature,
        &fixed,
        || Ok(()),
        || Ok(()),
    );
    assert_eq!(
        fixed_cached.flags_to_set,
        HashRouterFlags::PRIVATE2 | HashRouterFlags::PRIVATE4
    );
    let legacy_checks = Cell::new(0);
    let legacy_after_fixed = run_check_validity(
        fixed_cached.flags_to_set,
        alternate_role_signature,
        &legacy,
        || {
            legacy_checks.set(legacy_checks.get() + 1);
            Ok(())
        },
        || panic!("the cached local validity may be reused, not role signature validity"),
    );
    assert_eq!(legacy_checks.get(), 1);
    assert_eq!(legacy_after_fixed.flags_to_set, HashRouterFlags::PRIVATE8);
}
