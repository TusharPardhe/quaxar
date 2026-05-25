//! Integration tests that pin the narrowed Rust `CredentialDelete.cpp` shell to
//! the current C++ behavior.

use protocol::{Ter, tfUniversalMask, trans_token};
use tx::{
    CREDENTIAL_MAX_TYPE_LENGTH, CredentialDeleteApplySink, CredentialDeleteDoApplyFacts,
    CredentialDeletePreclaimFacts, CredentialDeletePreflightFacts, CredentialOptionalAccountField,
    get_credential_delete_flags_mask, run_credential_delete_do_apply,
    run_credential_delete_preclaim, run_credential_delete_preflight,
};

#[derive(Debug, Clone, PartialEq, Eq)]
struct TestDeleteSink {
    credential_exists: bool,
    credential_expired: bool,
    delete_result: Ter,
    events: Vec<String>,
}

impl TestDeleteSink {
    fn new() -> Self {
        Self {
            credential_exists: true,
            credential_expired: false,
            delete_result: Ter::TES_SUCCESS,
            events: Vec::new(),
        }
    }
}

impl CredentialDeleteApplySink for TestDeleteSink {
    fn credential_exists(&mut self) -> bool {
        self.events.push("credential_exists".to_string());
        self.credential_exists
    }

    fn credential_expired(&mut self) -> bool {
        self.events.push("credential_expired".to_string());
        self.credential_expired
    }

    fn delete_credential(&mut self) -> Ter {
        self.events.push("delete_credential".to_string());
        self.delete_result
    }
}

#[test]
fn credential_delete_flags_mask_matches_fix_invalid_tx_flags_gate() {
    assert_eq!(get_credential_delete_flags_mask(false), 0);
    assert_eq!(get_credential_delete_flags_mask(true), tfUniversalMask);
}

#[test]
fn credential_delete_preflight_validates_optional_accounts_and_type() {
    assert_eq!(
        run_credential_delete_preflight(CredentialDeletePreflightFacts {
            subject: CredentialOptionalAccountField::Missing,
            issuer: CredentialOptionalAccountField::Missing,
            credential_type_len: 3,
        }),
        Ter::TEM_MALFORMED
    );
    assert_eq!(
        run_credential_delete_preflight(CredentialDeletePreflightFacts {
            subject: CredentialOptionalAccountField::Zero,
            issuer: CredentialOptionalAccountField::Present,
            credential_type_len: 3,
        }),
        Ter::TEM_INVALID_ACCOUNT_ID
    );
    assert_eq!(
        run_credential_delete_preflight(CredentialDeletePreflightFacts {
            subject: CredentialOptionalAccountField::Present,
            issuer: CredentialOptionalAccountField::Present,
            credential_type_len: 0,
        }),
        Ter::TEM_MALFORMED
    );
    assert_eq!(
        run_credential_delete_preflight(CredentialDeletePreflightFacts {
            subject: CredentialOptionalAccountField::Present,
            issuer: CredentialOptionalAccountField::Missing,
            credential_type_len: CREDENTIAL_MAX_TYPE_LENGTH + 1,
        }),
        Ter::TEM_MALFORMED
    );
    assert_eq!(
        run_credential_delete_preflight(CredentialDeletePreflightFacts {
            subject: CredentialOptionalAccountField::Present,
            issuer: CredentialOptionalAccountField::Missing,
            credential_type_len: CREDENTIAL_MAX_TYPE_LENGTH,
        }),
        Ter::TES_SUCCESS
    );
}

#[test]
fn credential_delete_preclaim_maps_missing_entry() {
    assert_eq!(
        run_credential_delete_preclaim(CredentialDeletePreclaimFacts {
            credential_exists: false,
        }),
        Ter::TEC_NO_ENTRY
    );
    assert_eq!(
        run_credential_delete_preclaim(CredentialDeletePreclaimFacts {
            credential_exists: true,
        }),
        Ter::TES_SUCCESS
    );
}

#[test]
fn credential_delete_do_apply_preserves_permission_gate() {
    let mut sink = TestDeleteSink::new();

    let result = run_credential_delete_do_apply(
        CredentialDeleteDoApplyFacts {
            actor_is_subject: false,
            actor_is_issuer: false,
        },
        &mut sink,
    );

    assert_eq!(result, Ter::TEC_NO_PERMISSION);
    assert_eq!(trans_token(result), "tecNO_PERMISSION");
    assert_eq!(sink.events, ["credential_exists", "credential_expired"]);
}

#[test]
fn credential_delete_do_apply_allows_expired_third_party_and_owner_paths() {
    let mut expired = TestDeleteSink::new();
    expired.credential_expired = true;

    let result = run_credential_delete_do_apply(
        CredentialDeleteDoApplyFacts {
            actor_is_subject: false,
            actor_is_issuer: false,
        },
        &mut expired,
    );

    assert_eq!(result, Ter::TES_SUCCESS);
    assert_eq!(
        expired.events,
        [
            "credential_exists",
            "credential_expired",
            "delete_credential"
        ]
    );

    let mut owner = TestDeleteSink::new();
    let result = run_credential_delete_do_apply(
        CredentialDeleteDoApplyFacts {
            actor_is_subject: true,
            actor_is_issuer: false,
        },
        &mut owner,
    );
    assert_eq!(result, Ter::TES_SUCCESS);
    assert_eq!(owner.events, ["credential_exists", "delete_credential"]);
}

#[test]
fn credential_delete_do_apply_maps_missing_loaded_credential_and_delete_result() {
    let mut missing = TestDeleteSink::new();
    missing.credential_exists = false;

    let result = run_credential_delete_do_apply(
        CredentialDeleteDoApplyFacts {
            actor_is_subject: true,
            actor_is_issuer: false,
        },
        &mut missing,
    );

    assert_eq!(result, Ter::TEF_INTERNAL);
    assert_eq!(trans_token(result), "tefINTERNAL");
    assert_eq!(missing.events, ["credential_exists"]);

    let mut delete_failure = TestDeleteSink::new();
    delete_failure.delete_result = Ter::TEF_BAD_LEDGER;
    let result = run_credential_delete_do_apply(
        CredentialDeleteDoApplyFacts {
            actor_is_subject: true,
            actor_is_issuer: false,
        },
        &mut delete_failure,
    );
    assert_eq!(result, Ter::TEF_BAD_LEDGER);
    assert_eq!(
        delete_failure.events,
        ["credential_exists", "delete_credential"]
    );
}
