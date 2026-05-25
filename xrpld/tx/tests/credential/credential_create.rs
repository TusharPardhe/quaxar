//! Integration tests that pin the narrowed Rust `CredentialCreate.cpp` shell to
//! the current C++ behavior.

use protocol::{Ter, tfUniversalMask, trans_token};
use tx::{
    CREDENTIAL_ACCEPTED_FLAG, CREDENTIAL_MAX_TYPE_LENGTH, CREDENTIAL_MAX_URI_LENGTH,
    CredentialCreateApplyFacts, CredentialCreateApplySink, CredentialCreatePreclaimFacts,
    CredentialCreatePreflightFacts, get_credential_create_flags_mask,
    run_credential_create_do_apply, run_credential_create_preclaim,
    run_credential_create_preflight,
};

#[derive(Debug, Clone, PartialEq, Eq)]
struct TestCreateSink {
    begin_credential: bool,
    issuer_exists: bool,
    issuer_owner_count: u32,
    issuer_has_reserve: bool,
    issuer_dir_page: Option<u64>,
    subject_dir_page: Option<u64>,
    events: Vec<String>,
}

impl TestCreateSink {
    fn new() -> Self {
        Self {
            begin_credential: true,
            issuer_exists: true,
            issuer_owner_count: 3,
            issuer_has_reserve: true,
            issuer_dir_page: Some(11),
            subject_dir_page: Some(22),
            events: Vec::new(),
        }
    }
}

impl CredentialCreateApplySink for TestCreateSink {
    type AccountId = &'static str;
    type CredentialType = &'static [u8];
    type Uri = &'static [u8];
    type OwnerNode = u64;

    fn begin_credential(&mut self) -> bool {
        self.events.push("begin_credential".to_string());
        self.begin_credential
    }

    fn set_expiration(&mut self, expiration: u32) {
        self.events.push(format!("set_expiration:{expiration}"));
    }

    fn issuer_exists(&mut self) -> bool {
        self.events.push("issuer_exists".to_string());
        self.issuer_exists
    }

    fn issuer_owner_count(&mut self) -> u32 {
        self.events.push("issuer_owner_count".to_string());
        self.issuer_owner_count
    }

    fn issuer_has_reserve(&mut self, owner_count_after: u32) -> bool {
        self.events
            .push(format!("issuer_has_reserve:{owner_count_after}"));
        self.issuer_has_reserve
    }

    fn set_subject(&mut self, subject: Self::AccountId) {
        self.events.push(format!("set_subject:{subject}"));
    }

    fn set_issuer(&mut self, issuer: Self::AccountId) {
        self.events.push(format!("set_issuer:{issuer}"));
    }

    fn set_credential_type(&mut self, credential_type: Self::CredentialType) {
        self.events.push(format!(
            "set_credential_type:{}",
            String::from_utf8_lossy(credential_type)
        ));
    }

    fn set_uri(&mut self, uri: Self::Uri) {
        self.events
            .push(format!("set_uri:{}", String::from_utf8_lossy(uri)));
    }

    fn insert_issuer_directory(&mut self) -> Option<Self::OwnerNode> {
        self.events.push("insert_issuer_directory".to_string());
        self.issuer_dir_page
    }

    fn set_issuer_node(&mut self, page: Self::OwnerNode) {
        self.events.push(format!("set_issuer_node:{page}"));
    }

    fn adjust_issuer_owner_count(&mut self, delta: i32) {
        self.events
            .push(format!("adjust_issuer_owner_count:{delta}"));
    }

    fn set_flags(&mut self, flags: u32) {
        self.events.push(format!("set_flags:{flags:#010x}"));
    }

    fn insert_subject_directory(&mut self) -> Option<Self::OwnerNode> {
        self.events.push("insert_subject_directory".to_string());
        self.subject_dir_page
    }

    fn set_subject_node(&mut self, page: Self::OwnerNode) {
        self.events.push(format!("set_subject_node:{page}"));
    }

    fn insert_credential(&mut self) {
        self.events.push("insert_credential".to_string());
    }
}

#[test]
fn credential_create_flags_mask_matches_fix_invalid_tx_flags_gate() {
    assert_eq!(get_credential_create_flags_mask(false), 0);
    assert_eq!(get_credential_create_flags_mask(true), tfUniversalMask);
}

#[test]
fn credential_create_preflight_validates_subject_uri_and_type() {
    assert_eq!(
        run_credential_create_preflight(CredentialCreatePreflightFacts {
            subject_present: false,
            uri_len: None,
            credential_type_len: 3,
        }),
        Ter::TEM_MALFORMED
    );
    assert_eq!(
        run_credential_create_preflight(CredentialCreatePreflightFacts {
            subject_present: true,
            uri_len: Some(0),
            credential_type_len: 3,
        }),
        Ter::TEM_MALFORMED
    );
    assert_eq!(
        run_credential_create_preflight(CredentialCreatePreflightFacts {
            subject_present: true,
            uri_len: Some(CREDENTIAL_MAX_URI_LENGTH + 1),
            credential_type_len: 3,
        }),
        Ter::TEM_MALFORMED
    );
    assert_eq!(
        run_credential_create_preflight(CredentialCreatePreflightFacts {
            subject_present: true,
            uri_len: Some(CREDENTIAL_MAX_URI_LENGTH),
            credential_type_len: CREDENTIAL_MAX_TYPE_LENGTH + 1,
        }),
        Ter::TEM_MALFORMED
    );
    assert_eq!(
        run_credential_create_preflight(CredentialCreatePreflightFacts {
            subject_present: true,
            uri_len: Some(12),
            credential_type_len: CREDENTIAL_MAX_TYPE_LENGTH,
        }),
        Ter::TES_SUCCESS
    );
}

#[test]
fn credential_create_preclaim_maps_subject_and_duplicate_checks() {
    assert_eq!(
        run_credential_create_preclaim(CredentialCreatePreclaimFacts {
            subject_exists: false,
            credential_exists: false,
        }),
        Ter::TEC_NO_TARGET
    );
    assert_eq!(
        run_credential_create_preclaim(CredentialCreatePreclaimFacts {
            subject_exists: true,
            credential_exists: true,
        }),
        Ter::TEC_DUPLICATE
    );
    assert_eq!(
        run_credential_create_preclaim(CredentialCreatePreclaimFacts {
            subject_exists: true,
            credential_exists: false,
        }),
        Ter::TES_SUCCESS
    );
}

#[test]
fn credential_create_do_apply_preserves_current_cpp_success_order() {
    let mut sink = TestCreateSink::new();

    let result = run_credential_create_do_apply(
        CredentialCreateApplyFacts {
            subject: "subject",
            issuer: "issuer",
            credential_type: b"abc".as_slice(),
            uri: Some(b"https://example".as_slice()),
            expiration: Some(55),
            close_time: 54,
        },
        &mut sink,
    );

    assert_eq!(result, Ter::TES_SUCCESS);
    assert_eq!(
        sink.events,
        [
            "begin_credential",
            "set_expiration:55",
            "issuer_exists",
            "issuer_owner_count",
            "issuer_has_reserve:4",
            "set_subject:subject",
            "set_issuer:issuer",
            "set_credential_type:abc",
            "set_uri:https://example",
            "insert_issuer_directory",
            "set_issuer_node:11",
            "adjust_issuer_owner_count:1",
            "insert_subject_directory",
            "set_subject_node:22",
            "insert_credential",
        ]
    );
}

#[test]
fn credential_create_do_apply_self_accepts_without_subject_directory() {
    let mut sink = TestCreateSink::new();

    let result = run_credential_create_do_apply(
        CredentialCreateApplyFacts {
            subject: "issuer",
            issuer: "issuer",
            credential_type: b"self".as_slice(),
            uri: None,
            expiration: None,
            close_time: 0,
        },
        &mut sink,
    );

    assert_eq!(result, Ter::TES_SUCCESS);
    assert_eq!(
        sink.events,
        [
            "begin_credential",
            "issuer_exists",
            "issuer_owner_count",
            "issuer_has_reserve:4",
            "set_subject:issuer",
            "set_issuer:issuer",
            "set_credential_type:self",
            "insert_issuer_directory",
            "set_issuer_node:11",
            "adjust_issuer_owner_count:1",
            &format!("set_flags:{CREDENTIAL_ACCEPTED_FLAG:#010x}"),
            "insert_credential",
        ]
    );
}

#[test]
fn credential_create_do_apply_maps_failure_points() {
    let mut missing = TestCreateSink::new();
    missing.begin_credential = false;
    let result = run_credential_create_do_apply(
        CredentialCreateApplyFacts {
            subject: "subject",
            issuer: "issuer",
            credential_type: b"type".as_slice(),
            uri: None,
            expiration: None,
            close_time: 0,
        },
        &mut missing,
    );
    assert_eq!(result, Ter::TEF_INTERNAL);
    assert_eq!(missing.events, ["begin_credential"]);

    let mut expired = TestCreateSink::new();
    let result = run_credential_create_do_apply(
        CredentialCreateApplyFacts {
            subject: "subject",
            issuer: "issuer",
            credential_type: b"type".as_slice(),
            uri: None,
            expiration: Some(9),
            close_time: 10,
        },
        &mut expired,
    );
    assert_eq!(result, Ter::TEC_EXPIRED);
    assert_eq!(expired.events, ["begin_credential"]);

    let mut no_reserve = TestCreateSink::new();
    no_reserve.issuer_has_reserve = false;
    let result = run_credential_create_do_apply(
        CredentialCreateApplyFacts {
            subject: "subject",
            issuer: "issuer",
            credential_type: b"type".as_slice(),
            uri: None,
            expiration: None,
            close_time: 0,
        },
        &mut no_reserve,
    );
    assert_eq!(result, Ter::TEC_INSUFFICIENT_RESERVE);
    assert_eq!(
        no_reserve.events,
        [
            "begin_credential",
            "issuer_exists",
            "issuer_owner_count",
            "issuer_has_reserve:4",
        ]
    );

    let mut subject_dir_full = TestCreateSink::new();
    subject_dir_full.subject_dir_page = None;
    let result = run_credential_create_do_apply(
        CredentialCreateApplyFacts {
            subject: "subject",
            issuer: "issuer",
            credential_type: b"type".as_slice(),
            uri: None,
            expiration: None,
            close_time: 0,
        },
        &mut subject_dir_full,
    );
    assert_eq!(result, Ter::TEC_DIR_FULL);
    assert_eq!(trans_token(result), "tecDIR_FULL");
    assert_eq!(
        subject_dir_full.events,
        [
            "begin_credential",
            "issuer_exists",
            "issuer_owner_count",
            "issuer_has_reserve:4",
            "set_subject:subject",
            "set_issuer:issuer",
            "set_credential_type:type",
            "insert_issuer_directory",
            "set_issuer_node:11",
            "adjust_issuer_owner_count:1",
            "insert_subject_directory",
        ]
    );
}
