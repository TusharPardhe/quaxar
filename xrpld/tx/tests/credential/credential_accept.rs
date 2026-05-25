//! Integration tests that pin the narrowed Rust `CredentialAccept.cpp` shell to
//! the current C++ behavior.

use protocol::{Ter, tfUniversalMask, trans_token};
use tx::{
    CREDENTIAL_ACCEPTED_FLAG, CREDENTIAL_MAX_TYPE_LENGTH, CredentialAcceptApplySink,
    CredentialAcceptPreclaimFacts, CredentialAcceptPreflightFacts,
    get_credential_accept_flags_mask, run_credential_accept_do_apply,
    run_credential_accept_preclaim, run_credential_accept_preflight,
};

#[derive(Debug, Clone, PartialEq, Eq)]
struct TestAcceptSink {
    subject_exists: bool,
    issuer_exists: bool,
    subject_owner_count: u32,
    subject_has_reserve: bool,
    credential_exists: bool,
    credential_expired: bool,
    delete_result: Ter,
    events: Vec<String>,
}

impl TestAcceptSink {
    fn new() -> Self {
        Self {
            subject_exists: true,
            issuer_exists: true,
            subject_owner_count: 0,
            subject_has_reserve: true,
            credential_exists: true,
            credential_expired: false,
            delete_result: Ter::TES_SUCCESS,
            events: Vec::new(),
        }
    }
}

impl CredentialAcceptApplySink for TestAcceptSink {
    fn subject_exists(&mut self) -> bool {
        self.events.push("subject_exists".to_string());
        self.subject_exists
    }

    fn issuer_exists(&mut self) -> bool {
        self.events.push("issuer_exists".to_string());
        self.issuer_exists
    }

    fn subject_owner_count(&mut self) -> u32 {
        self.events.push("subject_owner_count".to_string());
        self.subject_owner_count
    }

    fn subject_has_reserve(&mut self, owner_count_after: u32) -> bool {
        self.events
            .push(format!("subject_has_reserve:{owner_count_after}"));
        self.subject_has_reserve
    }

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

    fn set_credential_flags(&mut self, flags: u32) {
        self.events
            .push(format!("set_credential_flags:{flags:#010x}"));
    }

    fn update_credential(&mut self) {
        self.events.push("update_credential".to_string());
    }

    fn adjust_issuer_owner_count(&mut self, delta: i32) {
        self.events
            .push(format!("adjust_issuer_owner_count:{delta}"));
    }

    fn adjust_subject_owner_count(&mut self, delta: i32) {
        self.events
            .push(format!("adjust_subject_owner_count:{delta}"));
    }
}

#[test]
fn credential_accept_flags_mask_matches_fix_invalid_tx_flags_gate() {
    assert_eq!(get_credential_accept_flags_mask(false), 0);
    assert_eq!(get_credential_accept_flags_mask(true), tfUniversalMask);
}

#[test]
fn credential_accept_preflight_validates_issuer_and_type() {
    assert_eq!(
        run_credential_accept_preflight(CredentialAcceptPreflightFacts {
            issuer_present: false,
            credential_type_len: 3,
        }),
        Ter::TEM_INVALID_ACCOUNT_ID
    );
    assert_eq!(
        run_credential_accept_preflight(CredentialAcceptPreflightFacts {
            issuer_present: true,
            credential_type_len: 0,
        }),
        Ter::TEM_MALFORMED
    );
    assert_eq!(
        run_credential_accept_preflight(CredentialAcceptPreflightFacts {
            issuer_present: true,
            credential_type_len: CREDENTIAL_MAX_TYPE_LENGTH + 1,
        }),
        Ter::TEM_MALFORMED
    );
    assert_eq!(
        run_credential_accept_preflight(CredentialAcceptPreflightFacts {
            issuer_present: true,
            credential_type_len: CREDENTIAL_MAX_TYPE_LENGTH,
        }),
        Ter::TES_SUCCESS
    );
}

#[test]
fn credential_accept_preclaim_maps_missing_issuer_entry_and_duplicate() {
    assert_eq!(
        run_credential_accept_preclaim(CredentialAcceptPreclaimFacts {
            issuer_exists: false,
            credential_exists: false,
            credential_accepted: false,
        }),
        Ter::TEC_NO_ISSUER
    );
    assert_eq!(
        run_credential_accept_preclaim(CredentialAcceptPreclaimFacts {
            issuer_exists: false,
            credential_exists: true,
            credential_accepted: false,
        }),
        Ter::TEC_NO_ISSUER
    );
    assert_eq!(
        run_credential_accept_preclaim(CredentialAcceptPreclaimFacts {
            issuer_exists: true,
            credential_exists: false,
            credential_accepted: false,
        }),
        Ter::TEC_NO_ENTRY
    );
    assert_eq!(
        run_credential_accept_preclaim(CredentialAcceptPreclaimFacts {
            issuer_exists: true,
            credential_exists: true,
            credential_accepted: true,
        }),
        Ter::TEC_DUPLICATE
    );
    assert_eq!(
        run_credential_accept_preclaim(CredentialAcceptPreclaimFacts {
            issuer_exists: true,
            credential_exists: true,
            credential_accepted: false,
        }),
        Ter::TES_SUCCESS
    );
}

#[test]
fn credential_accept_do_apply_preserves_current_cpp_success_order() {
    let mut sink = TestAcceptSink::new();

    let result = run_credential_accept_do_apply(&mut sink);

    assert_eq!(result, Ter::TES_SUCCESS);
    assert_eq!(
        sink.events,
        [
            "subject_exists",
            "issuer_exists",
            "subject_owner_count",
            "subject_has_reserve:1",
            "credential_exists",
            "credential_expired",
            &format!("set_credential_flags:{CREDENTIAL_ACCEPTED_FLAG:#010x}"),
            "update_credential",
            "adjust_issuer_owner_count:-1",
            "adjust_subject_owner_count:1",
        ]
    );
}

#[test]
fn credential_accept_do_apply_maps_missing_accounts_reserve_and_internal() {
    let mut missing_subject = TestAcceptSink::new();
    missing_subject.subject_exists = false;
    let result = run_credential_accept_do_apply(&mut missing_subject);
    assert_eq!(result, Ter::TEF_INTERNAL);
    assert_eq!(missing_subject.events, ["subject_exists", "issuer_exists"]);

    let mut no_reserve = TestAcceptSink::new();
    no_reserve.subject_has_reserve = false;
    let result = run_credential_accept_do_apply(&mut no_reserve);
    assert_eq!(result, Ter::TEC_INSUFFICIENT_RESERVE);
    assert_eq!(
        no_reserve.events,
        [
            "subject_exists",
            "issuer_exists",
            "subject_owner_count",
            "subject_has_reserve:1",
        ]
    );

    let mut missing_credential = TestAcceptSink::new();
    missing_credential.credential_exists = false;
    let result = run_credential_accept_do_apply(&mut missing_credential);
    assert_eq!(result, Ter::TEF_INTERNAL);
    assert_eq!(
        missing_credential.events,
        [
            "subject_exists",
            "issuer_exists",
            "subject_owner_count",
            "subject_has_reserve:1",
            "credential_exists",
        ]
    );
}

#[test]
fn credential_accept_do_apply_maps_expired_delete_path() {
    let mut sink = TestAcceptSink::new();
    sink.credential_expired = true;

    let result = run_credential_accept_do_apply(&mut sink);

    assert_eq!(result, Ter::TEC_EXPIRED);
    assert_eq!(trans_token(result), "tecEXPIRED");
    assert_eq!(
        sink.events,
        [
            "subject_exists",
            "issuer_exists",
            "subject_owner_count",
            "subject_has_reserve:1",
            "credential_exists",
            "credential_expired",
            "delete_credential",
        ]
    );

    let mut delete_failure = TestAcceptSink::new();
    delete_failure.credential_expired = true;
    delete_failure.delete_result = Ter::TEF_BAD_LEDGER;
    let result = run_credential_accept_do_apply(&mut delete_failure);
    assert_eq!(result, Ter::TEF_BAD_LEDGER);
    assert_eq!(trans_token(result), "tefBAD_LEDGER");
}
