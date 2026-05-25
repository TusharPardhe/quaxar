//! Current the reference implementation feature-gate, `preflight(...)`,
//! `preclaim(...)`, `doApply()`, and `removeFromLedger(...)` shells.
//!
//! This ports the exact deterministic behavior around:
//!
//! - the credentials amendment gate, and
//! - the ordered `preflight(...)` field-combination, zero-account, and
//!   self-preauthorization checks.
//! - the ordered `preclaim(...)` existence and duplicate checks.
//! - the full `doApply()` branch ordering across account and credential paths.
//! - the shared `removeFromLedger(...)` first-failure unlink/owner/erase flow.

use std::collections::BTreeSet;

use protocol::{NotTec, Ter};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DepositPreauthPreflightFacts<AccountId> {
    pub account: AccountId,
    pub authorize: Option<AccountId>,
    pub unauthorize: Option<AccountId>,
    pub authorize_is_zero: bool,
    pub unauthorize_is_zero: bool,
    pub authorize_credentials_present: bool,
    pub unauthorize_credentials_present: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DepositPreauthCredentialPreclaimFact<AccountId, CredentialType> {
    pub issuer: AccountId,
    pub credential_type: CredentialType,
    pub issuer_exists: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DepositPreauthPreclaimFacts<AccountId, CredentialType> {
    pub authorize: Option<AccountId>,
    pub unauthorize: Option<AccountId>,
    pub authorize_target_exists: bool,
    pub authorize_preauth_exists: bool,
    pub unauthorize_preauth_exists: bool,
    pub authorize_credentials_present: bool,
    pub authorize_credentials: Vec<DepositPreauthCredentialPreclaimFact<AccountId, CredentialType>>,
    pub authorize_credentials_preauth_exists: bool,
    pub unauthorize_credentials_present: bool,
    pub unauthorize_credentials_preauth_exists: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct DepositPreauthDoApplyAccountFacts {
    pub authorize_present: bool,
    pub unauthorize_present: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DepositPreauthDoApplyAccountPath {
    Return(Ter),
    ContinueToCredentialPaths,
}

pub trait DepositPreauthDoApplyAccountSink {
    type OwnerNode: Copy;

    fn authorize_owner_exists(&mut self) -> bool;
    fn authorize_has_reserve(&mut self) -> bool;
    fn create_authorize_preauth(&mut self);
    fn dir_insert_authorize_preauth(&mut self) -> Option<Self::OwnerNode>;
    fn set_authorize_owner_node(&mut self, page: Self::OwnerNode);
    fn adjust_authorize_owner_count(&mut self);
    fn remove_unauthorize_preauth(&mut self) -> Ter;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct DepositPreauthDoApplyCredentialFacts {
    pub authorize_credentials_present: bool,
    pub unauthorize_credentials_present: bool,
}

pub trait DepositPreauthDoApplyCredentialSink {
    type OwnerNode: Copy;

    fn authorize_credentials_owner_exists(&mut self) -> bool;
    fn authorize_credentials_has_reserve(&mut self) -> bool;
    fn sort_authorize_credentials(&mut self);
    fn create_authorize_credentials_preauth(&mut self) -> bool;
    fn dir_insert_authorize_credentials_preauth(&mut self) -> Option<Self::OwnerNode>;
    fn set_authorize_credentials_owner_node(&mut self, page: Self::OwnerNode);
    fn adjust_authorize_credentials_owner_count(&mut self);
    fn remove_unauthorize_credentials_preauth(&mut self) -> Ter;
}

pub fn deposit_preauth_check_extra_features(
    authorize_credentials_present: bool,
    unauthorize_credentials_present: bool,
    feature_credentials_enabled: bool,
) -> bool {
    !(authorize_credentials_present || unauthorize_credentials_present)
        || feature_credentials_enabled
}

pub fn run_deposit_preauth_preflight<AccountId: Eq>(
    facts: DepositPreauthPreflightFacts<AccountId>,
    check_credentials_array: impl FnOnce() -> NotTec,
) -> NotTec {
    let auth_cred_present =
        facts.authorize_credentials_present as i32 + facts.unauthorize_credentials_present as i32;
    let auth_present = facts.authorize.is_some() as i32 + facts.unauthorize.is_some() as i32;

    if auth_present + auth_cred_present != 1 {
        return Ter::TEM_MALFORMED;
    }

    if auth_present != 0 {
        if facts.authorize.is_some() {
            if facts.authorize_is_zero {
                return Ter::TEM_INVALID_ACCOUNT_ID;
            }

            if facts.authorize.as_ref() == Some(&facts.account) {
                return Ter::TEM_CANNOT_PREAUTH_SELF;
            }
        } else if facts.unauthorize_is_zero {
            return Ter::TEM_INVALID_ACCOUNT_ID;
        }
    } else {
        let err = check_credentials_array();
        if err != Ter::TES_SUCCESS {
            return err;
        }
    }

    Ter::TES_SUCCESS
}

pub fn run_deposit_preauth_preclaim<AccountId: Ord, CredentialType: Ord>(
    facts: DepositPreauthPreclaimFacts<AccountId, CredentialType>,
) -> Ter {
    if facts.authorize.is_some() {
        if !facts.authorize_target_exists {
            return Ter::TEC_NO_TARGET;
        }

        if facts.authorize_preauth_exists {
            return Ter::TEC_DUPLICATE;
        }
    } else if facts.unauthorize.is_some() {
        if !facts.unauthorize_preauth_exists {
            return Ter::TEC_NO_ENTRY;
        }
    } else if facts.authorize_credentials_present {
        // a duplicate preauth entry afterwards — no flag branching.
        let mut sorted = BTreeSet::new();

        for credential in facts.authorize_credentials {
            if !credential.issuer_exists {
                return Ter::TEC_NO_ISSUER;
            }

            if !sorted.insert((credential.issuer, credential.credential_type)) {
                return Ter::TEF_INTERNAL;
            }
        }

        if facts.authorize_credentials_preauth_exists {
            return Ter::TEC_DUPLICATE;
        }
    } else if facts.unauthorize_credentials_present && !facts.unauthorize_credentials_preauth_exists
    {
        return Ter::TEC_NO_ENTRY;
    }

    Ter::TES_SUCCESS
}

pub fn run_deposit_preauth_do_apply_account_paths<S: DepositPreauthDoApplyAccountSink>(
    facts: DepositPreauthDoApplyAccountFacts,
    sink: &mut S,
) -> DepositPreauthDoApplyAccountPath {
    if facts.authorize_present {
        if !sink.authorize_owner_exists() {
            return DepositPreauthDoApplyAccountPath::Return(Ter::TEF_INTERNAL);
        }

        if !sink.authorize_has_reserve() {
            return DepositPreauthDoApplyAccountPath::Return(Ter::TEC_INSUFFICIENT_RESERVE);
        }

        sink.create_authorize_preauth();

        let Some(page) = sink.dir_insert_authorize_preauth() else {
            return DepositPreauthDoApplyAccountPath::Return(Ter::TEC_DIR_FULL);
        };

        sink.set_authorize_owner_node(page);
        sink.adjust_authorize_owner_count();

        return DepositPreauthDoApplyAccountPath::Return(Ter::TES_SUCCESS);
    }

    if facts.unauthorize_present {
        return DepositPreauthDoApplyAccountPath::Return(sink.remove_unauthorize_preauth());
    }

    DepositPreauthDoApplyAccountPath::ContinueToCredentialPaths
}

pub fn run_deposit_preauth_do_apply_credential_paths<S: DepositPreauthDoApplyCredentialSink>(
    facts: DepositPreauthDoApplyCredentialFacts,
    sink: &mut S,
) -> Ter {
    if facts.authorize_credentials_present {
        if !sink.authorize_credentials_owner_exists() {
            return Ter::TEF_INTERNAL;
        }

        if !sink.authorize_credentials_has_reserve() {
            return Ter::TEC_INSUFFICIENT_RESERVE;
        }

        sink.sort_authorize_credentials();

        if !sink.create_authorize_credentials_preauth() {
            return Ter::TEF_INTERNAL;
        }

        let Some(page) = sink.dir_insert_authorize_credentials_preauth() else {
            return Ter::TEC_DIR_FULL;
        };

        sink.set_authorize_credentials_owner_node(page);
        sink.adjust_authorize_credentials_owner_count();

        return Ter::TES_SUCCESS;
    }

    if facts.unauthorize_credentials_present {
        return sink.remove_unauthorize_credentials_preauth();
    }

    Ter::TES_SUCCESS
}

pub fn run_deposit_preauth_remove_from_ledger<Entry>(
    preauth: Option<Entry>,
    dir_remove: impl FnOnce(&Entry) -> bool,
    owner_exists: impl FnOnce(&Entry) -> bool,
    adjust_owner_count: impl FnOnce(),
    erase_preauth: impl FnOnce(Entry),
) -> Ter {
    let Some(preauth) = preauth else {
        return Ter::TEC_NO_ENTRY;
    };

    if !dir_remove(&preauth) {
        return Ter::TEF_BAD_LEDGER;
    }

    if !owner_exists(&preauth) {
        return Ter::TEF_INTERNAL;
    }

    adjust_owner_count();
    erase_preauth(preauth);
    Ter::TES_SUCCESS
}

pub fn run_deposit_preauth_do_apply<S>(
    account_facts: DepositPreauthDoApplyAccountFacts,
    credential_facts: DepositPreauthDoApplyCredentialFacts,
    sink: &mut S,
) -> Ter
where
    S: DepositPreauthDoApplyAccountSink + DepositPreauthDoApplyCredentialSink,
{
    match run_deposit_preauth_do_apply_account_paths(account_facts, sink) {
        DepositPreauthDoApplyAccountPath::Return(ter) => ter,
        DepositPreauthDoApplyAccountPath::ContinueToCredentialPaths => {
            run_deposit_preauth_do_apply_credential_paths(credential_facts, sink)
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{cell::Cell, rc::Rc};

    use protocol::trans_token;

    use super::{
        DepositPreauthCredentialPreclaimFact, DepositPreauthDoApplyAccountFacts,
        DepositPreauthDoApplyAccountPath, DepositPreauthDoApplyAccountSink,
        DepositPreauthDoApplyCredentialFacts, DepositPreauthDoApplyCredentialSink,
        DepositPreauthPreclaimFacts, DepositPreauthPreflightFacts,
        deposit_preauth_check_extra_features, run_deposit_preauth_do_apply,
        run_deposit_preauth_do_apply_account_paths, run_deposit_preauth_do_apply_credential_paths,
        run_deposit_preauth_preclaim, run_deposit_preauth_preflight,
        run_deposit_preauth_remove_from_ledger,
    };

    fn empty_preclaim_facts() -> DepositPreauthPreclaimFacts<&'static str, &'static str> {
        DepositPreauthPreclaimFacts {
            authorize: None,
            unauthorize: None,
            authorize_target_exists: false,
            authorize_preauth_exists: false,
            unauthorize_preauth_exists: false,
            authorize_credentials_present: false,
            authorize_credentials: Vec::new(),
            authorize_credentials_preauth_exists: false,
            unauthorize_credentials_present: false,
            unauthorize_credentials_preauth_exists: false,
        }
    }

    #[derive(Debug, Clone)]
    struct TestDoApplySink {
        owner_exists: bool,
        has_reserve: bool,
        dir_page: Option<u64>,
        remove_result: protocol::Ter,
        owner_node: Option<u64>,
        adjusted: bool,
        credential_owner_exists: bool,
        credential_has_reserve: bool,
        credential_create_ok: bool,
        credential_dir_page: Option<u64>,
        credential_remove_result: protocol::Ter,
        credential_owner_node: Option<u64>,
        credential_adjusted: bool,
        events: Rc<std::cell::RefCell<Vec<&'static str>>>,
    }

    impl TestDoApplySink {
        fn new(events: Rc<std::cell::RefCell<Vec<&'static str>>>) -> Self {
            Self {
                owner_exists: true,
                has_reserve: true,
                dir_page: Some(7),
                remove_result: protocol::Ter::TES_SUCCESS,
                owner_node: None,
                adjusted: false,
                credential_owner_exists: true,
                credential_has_reserve: true,
                credential_create_ok: true,
                credential_dir_page: Some(13),
                credential_remove_result: protocol::Ter::TES_SUCCESS,
                credential_owner_node: None,
                credential_adjusted: false,
                events,
            }
        }
    }

    impl DepositPreauthDoApplyAccountSink for TestDoApplySink {
        type OwnerNode = u64;

        fn authorize_owner_exists(&mut self) -> bool {
            self.events.borrow_mut().push("owner");
            self.owner_exists
        }

        fn authorize_has_reserve(&mut self) -> bool {
            self.events.borrow_mut().push("reserve");
            self.has_reserve
        }

        fn create_authorize_preauth(&mut self) {
            self.events.borrow_mut().push("create");
        }

        fn dir_insert_authorize_preauth(&mut self) -> Option<Self::OwnerNode> {
            self.events.borrow_mut().push("dir");
            self.dir_page
        }

        fn set_authorize_owner_node(&mut self, page: Self::OwnerNode) {
            self.events.borrow_mut().push("owner_node");
            self.owner_node = Some(page);
        }

        fn adjust_authorize_owner_count(&mut self) {
            self.events.borrow_mut().push("adjust");
            self.adjusted = true;
        }

        fn remove_unauthorize_preauth(&mut self) -> protocol::Ter {
            self.events.borrow_mut().push("remove");
            self.remove_result
        }
    }

    impl DepositPreauthDoApplyCredentialSink for TestDoApplySink {
        type OwnerNode = u64;

        fn authorize_credentials_owner_exists(&mut self) -> bool {
            self.events.borrow_mut().push("cred_owner");
            self.credential_owner_exists
        }

        fn authorize_credentials_has_reserve(&mut self) -> bool {
            self.events.borrow_mut().push("cred_reserve");
            self.credential_has_reserve
        }

        fn sort_authorize_credentials(&mut self) {
            self.events.borrow_mut().push("cred_sort");
        }

        fn create_authorize_credentials_preauth(&mut self) -> bool {
            self.events.borrow_mut().push("cred_create");
            self.credential_create_ok
        }

        fn dir_insert_authorize_credentials_preauth(&mut self) -> Option<Self::OwnerNode> {
            self.events.borrow_mut().push("cred_dir");
            self.credential_dir_page
        }

        fn set_authorize_credentials_owner_node(&mut self, page: Self::OwnerNode) {
            self.events.borrow_mut().push("cred_owner_node");
            self.credential_owner_node = Some(page);
        }

        fn adjust_authorize_credentials_owner_count(&mut self) {
            self.events.borrow_mut().push("cred_adjust");
            self.credential_adjusted = true;
        }

        fn remove_unauthorize_credentials_preauth(&mut self) -> protocol::Ter {
            self.events.borrow_mut().push("cred_remove");
            self.credential_remove_result
        }
    }

    #[test]
    fn deposit_preauth_check_extra_features_gate() {
        assert!(deposit_preauth_check_extra_features(false, false, false));
        assert!(deposit_preauth_check_extra_features(true, false, true));
        assert!(deposit_preauth_check_extra_features(false, true, true));
        assert!(!deposit_preauth_check_extra_features(true, false, false));
        assert!(!deposit_preauth_check_extra_features(false, true, false));
    }

    #[test]
    fn deposit_preauth_preflight_rejects_invalid_field_combinations() {
        let none = run_deposit_preauth_preflight(
            DepositPreauthPreflightFacts {
                account: "alice",
                authorize: None,
                unauthorize: None,
                authorize_is_zero: false,
                unauthorize_is_zero: false,
                authorize_credentials_present: false,
                unauthorize_credentials_present: false,
            },
            || protocol::Ter::TES_SUCCESS,
        );
        let both_accounts = run_deposit_preauth_preflight(
            DepositPreauthPreflightFacts {
                account: "alice",
                authorize: Some("becky"),
                unauthorize: Some("carol"),
                authorize_is_zero: false,
                unauthorize_is_zero: false,
                authorize_credentials_present: false,
                unauthorize_credentials_present: false,
            },
            || protocol::Ter::TES_SUCCESS,
        );
        let account_and_credentials = run_deposit_preauth_preflight(
            DepositPreauthPreflightFacts {
                account: "alice",
                authorize: Some("becky"),
                unauthorize: None,
                authorize_is_zero: false,
                unauthorize_is_zero: false,
                authorize_credentials_present: true,
                unauthorize_credentials_present: false,
            },
            || protocol::Ter::TES_SUCCESS,
        );

        assert_eq!(none, protocol::Ter::TEM_MALFORMED);
        assert_eq!(both_accounts, protocol::Ter::TEM_MALFORMED);
        assert_eq!(account_and_credentials, protocol::Ter::TEM_MALFORMED);
    }

    #[test]
    fn deposit_preauth_preflight_rejects_zero_authorize_target() {
        let result = run_deposit_preauth_preflight(
            DepositPreauthPreflightFacts {
                account: "alice",
                authorize: Some("zero"),
                unauthorize: None,
                authorize_is_zero: true,
                unauthorize_is_zero: false,
                authorize_credentials_present: false,
                unauthorize_credentials_present: false,
            },
            || protocol::Ter::TES_SUCCESS,
        );

        assert_eq!(result, protocol::Ter::TEM_INVALID_ACCOUNT_ID);
        assert_eq!(trans_token(result), "temINVALID_ACCOUNT_ID");
    }

    #[test]
    fn deposit_preauth_preflight_rejects_zero_unauthorize_target() {
        let result = run_deposit_preauth_preflight(
            DepositPreauthPreflightFacts {
                account: "alice",
                authorize: None,
                unauthorize: Some("zero"),
                authorize_is_zero: false,
                unauthorize_is_zero: true,
                authorize_credentials_present: false,
                unauthorize_credentials_present: false,
            },
            || protocol::Ter::TES_SUCCESS,
        );

        assert_eq!(result, protocol::Ter::TEM_INVALID_ACCOUNT_ID);
    }

    #[test]
    fn deposit_preauth_preflight_rejects_self_authorize() {
        let result = run_deposit_preauth_preflight(
            DepositPreauthPreflightFacts {
                account: "alice",
                authorize: Some("alice"),
                unauthorize: None,
                authorize_is_zero: false,
                unauthorize_is_zero: false,
                authorize_credentials_present: false,
                unauthorize_credentials_present: false,
            },
            || protocol::Ter::TES_SUCCESS,
        );

        assert_eq!(result, protocol::Ter::TEM_CANNOT_PREAUTH_SELF);
        assert_eq!(trans_token(result), "temCANNOT_PREAUTH_SELF");
    }

    #[test]
    fn deposit_preauth_preflight_allows_unauthorize_self() {
        let result = run_deposit_preauth_preflight(
            DepositPreauthPreflightFacts {
                account: "alice",
                authorize: None,
                unauthorize: Some("alice"),
                authorize_is_zero: false,
                unauthorize_is_zero: false,
                authorize_credentials_present: false,
                unauthorize_credentials_present: false,
            },
            || protocol::Ter::TES_SUCCESS,
        );

        assert_eq!(result, protocol::Ter::TES_SUCCESS);
    }

    #[test]
    fn deposit_preauth_preflight_runs_credential_array_check_only_for_array_paths() {
        let called = Cell::new(false);

        let result = run_deposit_preauth_preflight(
            DepositPreauthPreflightFacts {
                account: "alice",
                authorize: None,
                unauthorize: None,
                authorize_is_zero: false,
                unauthorize_is_zero: false,
                authorize_credentials_present: true,
                unauthorize_credentials_present: false,
            },
            || {
                called.set(true);
                protocol::Ter::TEM_ARRAY_TOO_LARGE
            },
        );

        assert!(called.get());
        assert_eq!(result, protocol::Ter::TEM_ARRAY_TOO_LARGE);
    }

    #[test]
    fn deposit_preauth_preflight_skips_credential_array_check_for_account_paths() {
        let called = Cell::new(false);

        let result = run_deposit_preauth_preflight(
            DepositPreauthPreflightFacts {
                account: "alice",
                authorize: Some("becky"),
                unauthorize: None,
                authorize_is_zero: false,
                unauthorize_is_zero: false,
                authorize_credentials_present: false,
                unauthorize_credentials_present: false,
            },
            || {
                called.set(true);
                protocol::Ter::TEM_ARRAY_EMPTY
            },
        );

        assert!(!called.get());
        assert_eq!(result, protocol::Ter::TES_SUCCESS);
    }

    #[test]
    fn deposit_preauth_preclaim_rejects_missing_authorize_target() {
        let result = run_deposit_preauth_preclaim(DepositPreauthPreclaimFacts {
            authorize: Some("becky"),
            authorize_target_exists: false,
            ..empty_preclaim_facts()
        });

        assert_eq!(result, protocol::Ter::TEC_NO_TARGET);
        assert_eq!(trans_token(result), "tecNO_TARGET");
    }

    #[test]
    fn deposit_preauth_preclaim_rejects_duplicate_authorize_entry() {
        let result = run_deposit_preauth_preclaim(DepositPreauthPreclaimFacts {
            authorize: Some("becky"),
            authorize_target_exists: true,
            authorize_preauth_exists: true,
            ..empty_preclaim_facts()
        });

        assert_eq!(result, protocol::Ter::TEC_DUPLICATE);
    }

    // the target check fires first and we get tecNO_TARGET — not tecDUPLICATE.
    #[test]
    fn deposit_preauth_preclaim_returns_no_target_even_when_preauth_exists() {
        let result = run_deposit_preauth_preclaim(DepositPreauthPreclaimFacts {
            authorize: Some("becky"),
            authorize_target_exists: false,
            authorize_preauth_exists: true,
            ..empty_preclaim_facts()
        });

        assert_eq!(result, protocol::Ter::TEC_NO_TARGET);
    }

    #[test]
    fn deposit_preauth_preclaim_rejects_missing_credential_issuer() {
        let result = run_deposit_preauth_preclaim(DepositPreauthPreclaimFacts {
            authorize_credentials_present: true,
            authorize_credentials: vec![DepositPreauthCredentialPreclaimFact {
                issuer: "issuer",
                credential_type: "cred-a",
                issuer_exists: false,
            }],
            ..empty_preclaim_facts()
        });

        assert_eq!(result, protocol::Ter::TEC_NO_ISSUER);
        assert_eq!(trans_token(result), "tecNO_ISSUER");
    }

    #[test]
    fn deposit_preauth_preclaim_rejects_duplicate_credential_pair() {
        let result = run_deposit_preauth_preclaim(DepositPreauthPreclaimFacts {
            authorize_credentials_present: true,
            authorize_credentials: vec![
                DepositPreauthCredentialPreclaimFact {
                    issuer: "issuer",
                    credential_type: "cred-a",
                    issuer_exists: true,
                },
                DepositPreauthCredentialPreclaimFact {
                    issuer: "issuer",
                    credential_type: "cred-a",
                    issuer_exists: true,
                },
            ],
            ..empty_preclaim_facts()
        });

        assert_eq!(result, protocol::Ter::TEF_INTERNAL);
        assert_eq!(trans_token(result), "tefINTERNAL");
    }

    #[test]
    fn deposit_preauth_preclaim_uses_current_cpp_branch_order() {
        // authorize branch takes priority over authorize_credentials branch;
        // within authorize, target check fires before duplicate check.
        let result = run_deposit_preauth_preclaim(DepositPreauthPreclaimFacts {
            authorize: Some("becky"),
            authorize_target_exists: false,
            authorize_credentials_present: true,
            authorize_credentials: vec![DepositPreauthCredentialPreclaimFact {
                issuer: "issuer",
                credential_type: "cred-a",
                issuer_exists: true,
            }],
            authorize_credentials_preauth_exists: true,
            ..empty_preclaim_facts()
        });

        assert_eq!(result, protocol::Ter::TEC_NO_TARGET);
    }

    #[test]
    fn deposit_preauth_do_apply_authorize_returns_tefinternal_for_missing_owner() {
        let events = Rc::new(std::cell::RefCell::new(Vec::new()));
        let mut sink = TestDoApplySink::new(Rc::clone(&events));
        sink.owner_exists = false;

        let result = run_deposit_preauth_do_apply_account_paths(
            DepositPreauthDoApplyAccountFacts {
                authorize_present: true,
                unauthorize_present: false,
            },
            &mut sink,
        );

        assert_eq!(
            result,
            DepositPreauthDoApplyAccountPath::Return(protocol::Ter::TEF_INTERNAL)
        );
        assert_eq!(trans_token(protocol::Ter::TEF_INTERNAL), "tefINTERNAL");
        assert_eq!(events.borrow().as_slice(), ["owner"]);
    }

    #[test]
    fn deposit_preauth_do_apply_authorize_checks_reserve_before_create() {
        let events = Rc::new(std::cell::RefCell::new(Vec::new()));
        let mut sink = TestDoApplySink::new(Rc::clone(&events));
        sink.has_reserve = false;

        let result = run_deposit_preauth_do_apply_account_paths(
            DepositPreauthDoApplyAccountFacts {
                authorize_present: true,
                unauthorize_present: false,
            },
            &mut sink,
        );

        assert_eq!(
            result,
            DepositPreauthDoApplyAccountPath::Return(protocol::Ter::TEC_INSUFFICIENT_RESERVE)
        );
        assert_eq!(events.borrow().as_slice(), ["owner", "reserve"]);
    }

    #[test]
    fn deposit_preauth_do_apply_authorize_maps_missing_dir_page_to_tecdir_full() {
        let events = Rc::new(std::cell::RefCell::new(Vec::new()));
        let mut sink = TestDoApplySink::new(Rc::clone(&events));
        sink.dir_page = None;

        let result = run_deposit_preauth_do_apply_account_paths(
            DepositPreauthDoApplyAccountFacts {
                authorize_present: true,
                unauthorize_present: false,
            },
            &mut sink,
        );

        assert_eq!(
            result,
            DepositPreauthDoApplyAccountPath::Return(protocol::Ter::TEC_DIR_FULL)
        );
        assert_eq!(trans_token(protocol::Ter::TEC_DIR_FULL), "tecDIR_FULL");
        assert_eq!(
            events.borrow().as_slice(),
            ["owner", "reserve", "create", "dir"]
        );
    }

    #[test]
    fn deposit_preauth_do_apply_authorize_preserves_current_on_success() {
        let events = Rc::new(std::cell::RefCell::new(Vec::new()));
        let mut sink = TestDoApplySink::new(Rc::clone(&events));

        let result = run_deposit_preauth_do_apply_account_paths(
            DepositPreauthDoApplyAccountFacts {
                authorize_present: true,
                unauthorize_present: false,
            },
            &mut sink,
        );

        assert_eq!(
            result,
            DepositPreauthDoApplyAccountPath::Return(protocol::Ter::TES_SUCCESS)
        );
        assert_eq!(
            events.borrow().as_slice(),
            ["owner", "reserve", "create", "dir", "owner_node", "adjust"]
        );
        assert_eq!(sink.owner_node, Some(7));
        assert!(sink.adjusted);
    }

    #[test]
    fn deposit_preauth_do_apply_unauthorize_returns_remove_result_unchanged() {
        let events = Rc::new(std::cell::RefCell::new(Vec::new()));
        let mut sink = TestDoApplySink::new(Rc::clone(&events));
        sink.remove_result = protocol::Ter::TEC_NO_ENTRY;

        let result = run_deposit_preauth_do_apply_account_paths(
            DepositPreauthDoApplyAccountFacts {
                authorize_present: false,
                unauthorize_present: true,
            },
            &mut sink,
        );

        assert_eq!(
            result,
            DepositPreauthDoApplyAccountPath::Return(protocol::Ter::TEC_NO_ENTRY)
        );
        assert_eq!(events.borrow().as_slice(), ["remove"]);
    }

    #[test]
    fn deposit_preauth_do_apply_account_paths_continue_to_credentials_when_no_account_path_exists()
    {
        let events = Rc::new(std::cell::RefCell::new(Vec::new()));
        let mut sink = TestDoApplySink::new(Rc::clone(&events));

        let result = run_deposit_preauth_do_apply_account_paths(
            DepositPreauthDoApplyAccountFacts::default(),
            &mut sink,
        );

        assert_eq!(
            result,
            DepositPreauthDoApplyAccountPath::ContinueToCredentialPaths
        );
        assert!(events.borrow().is_empty());
    }

    #[test]
    fn deposit_preauth_do_apply_account_paths_use_authorize_branch_first() {
        let events = Rc::new(std::cell::RefCell::new(Vec::new()));
        let mut sink = TestDoApplySink::new(Rc::clone(&events));
        sink.owner_exists = false;
        sink.remove_result = protocol::Ter::TEC_NO_ENTRY;

        let result = run_deposit_preauth_do_apply_account_paths(
            DepositPreauthDoApplyAccountFacts {
                authorize_present: true,
                unauthorize_present: true,
            },
            &mut sink,
        );

        assert_eq!(
            result,
            DepositPreauthDoApplyAccountPath::Return(protocol::Ter::TEF_INTERNAL)
        );
        assert_eq!(events.borrow().as_slice(), ["owner"]);
    }

    #[test]
    fn deposit_preauth_do_apply_credentials_return_tefinternal_for_missing_owner() {
        let events = Rc::new(std::cell::RefCell::new(Vec::new()));
        let mut sink = TestDoApplySink::new(Rc::clone(&events));
        sink.credential_owner_exists = false;

        let result = run_deposit_preauth_do_apply_credential_paths(
            DepositPreauthDoApplyCredentialFacts {
                authorize_credentials_present: true,
                unauthorize_credentials_present: false,
            },
            &mut sink,
        );

        assert_eq!(result, protocol::Ter::TEF_INTERNAL);
        assert_eq!(events.borrow().as_slice(), ["cred_owner"]);
    }

    #[test]
    fn deposit_preauth_do_apply_credentials_check_reserve_before_sort() {
        let events = Rc::new(std::cell::RefCell::new(Vec::new()));
        let mut sink = TestDoApplySink::new(Rc::clone(&events));
        sink.credential_has_reserve = false;

        let result = run_deposit_preauth_do_apply_credential_paths(
            DepositPreauthDoApplyCredentialFacts {
                authorize_credentials_present: true,
                unauthorize_credentials_present: false,
            },
            &mut sink,
        );

        assert_eq!(result, protocol::Ter::TEC_INSUFFICIENT_RESERVE);
        assert_eq!(events.borrow().as_slice(), ["cred_owner", "cred_reserve"]);
    }

    #[test]
    fn deposit_preauth_do_apply_credentials_map_create_failure_to_tefinternal() {
        let events = Rc::new(std::cell::RefCell::new(Vec::new()));
        let mut sink = TestDoApplySink::new(Rc::clone(&events));
        sink.credential_create_ok = false;

        let result = run_deposit_preauth_do_apply_credential_paths(
            DepositPreauthDoApplyCredentialFacts {
                authorize_credentials_present: true,
                unauthorize_credentials_present: false,
            },
            &mut sink,
        );

        assert_eq!(result, protocol::Ter::TEF_INTERNAL);
        assert_eq!(
            events.borrow().as_slice(),
            ["cred_owner", "cred_reserve", "cred_sort", "cred_create"]
        );
    }

    #[test]
    fn deposit_preauth_do_apply_credentials_map_missing_dir_page_to_tecdir_full() {
        let events = Rc::new(std::cell::RefCell::new(Vec::new()));
        let mut sink = TestDoApplySink::new(Rc::clone(&events));
        sink.credential_dir_page = None;

        let result = run_deposit_preauth_do_apply_credential_paths(
            DepositPreauthDoApplyCredentialFacts {
                authorize_credentials_present: true,
                unauthorize_credentials_present: false,
            },
            &mut sink,
        );

        assert_eq!(result, protocol::Ter::TEC_DIR_FULL);
        assert_eq!(
            events.borrow().as_slice(),
            [
                "cred_owner",
                "cred_reserve",
                "cred_sort",
                "cred_create",
                "cred_dir"
            ]
        );
    }

    #[test]
    fn deposit_preauth_do_apply_credentials_preserve_current_on_success() {
        let events = Rc::new(std::cell::RefCell::new(Vec::new()));
        let mut sink = TestDoApplySink::new(Rc::clone(&events));

        let result = run_deposit_preauth_do_apply_credential_paths(
            DepositPreauthDoApplyCredentialFacts {
                authorize_credentials_present: true,
                unauthorize_credentials_present: false,
            },
            &mut sink,
        );

        assert_eq!(result, protocol::Ter::TES_SUCCESS);
        assert_eq!(
            events.borrow().as_slice(),
            [
                "cred_owner",
                "cred_reserve",
                "cred_sort",
                "cred_create",
                "cred_dir",
                "cred_owner_node",
                "cred_adjust",
            ]
        );
        assert_eq!(sink.credential_owner_node, Some(13));
        assert!(sink.credential_adjusted);
    }

    #[test]
    fn deposit_preauth_do_apply_unauthorize_credentials_return_remove_result_unchanged() {
        let events = Rc::new(std::cell::RefCell::new(Vec::new()));
        let mut sink = TestDoApplySink::new(Rc::clone(&events));
        sink.credential_remove_result = protocol::Ter::TEC_NO_ENTRY;

        let result = run_deposit_preauth_do_apply_credential_paths(
            DepositPreauthDoApplyCredentialFacts {
                authorize_credentials_present: false,
                unauthorize_credentials_present: true,
            },
            &mut sink,
        );

        assert_eq!(result, protocol::Ter::TEC_NO_ENTRY);
        assert_eq!(events.borrow().as_slice(), ["cred_remove"]);
    }

    #[test]
    fn deposit_preauth_remove_from_ledger_returns_tecno_entry_for_missing_preauth() {
        let events = Rc::new(std::cell::RefCell::new(Vec::new()));
        let result = run_deposit_preauth_remove_from_ledger::<u64>(
            None,
            |_| {
                events.borrow_mut().push("dir_remove");
                true
            },
            |_| {
                events.borrow_mut().push("owner");
                true
            },
            || events.borrow_mut().push("adjust"),
            |_| events.borrow_mut().push("erase"),
        );

        assert_eq!(result, protocol::Ter::TEC_NO_ENTRY);
        assert!(events.borrow().is_empty());
    }

    #[test]
    fn deposit_preauth_remove_from_ledger_maps_dir_remove_failure() {
        let events = Rc::new(std::cell::RefCell::new(Vec::new()));
        let result = run_deposit_preauth_remove_from_ledger(
            Some("preauth"),
            |_| {
                events.borrow_mut().push("dir_remove");
                false
            },
            |_| {
                events.borrow_mut().push("owner");
                true
            },
            || events.borrow_mut().push("adjust"),
            |_| events.borrow_mut().push("erase"),
        );

        assert_eq!(result, protocol::Ter::TEF_BAD_LEDGER);
        assert_eq!(events.borrow().as_slice(), ["dir_remove"]);
    }

    #[test]
    fn deposit_preauth_remove_from_ledger_returns_tefinternal_for_missing_owner() {
        let events = Rc::new(std::cell::RefCell::new(Vec::new()));
        let result = run_deposit_preauth_remove_from_ledger(
            Some("preauth"),
            |_| {
                events.borrow_mut().push("dir_remove");
                true
            },
            |_| {
                events.borrow_mut().push("owner");
                false
            },
            || events.borrow_mut().push("adjust"),
            |_| events.borrow_mut().push("erase"),
        );

        assert_eq!(result, protocol::Ter::TEF_INTERNAL);
        assert_eq!(events.borrow().as_slice(), ["dir_remove", "owner"]);
    }

    #[test]
    fn deposit_preauth_remove_from_ledger_preserves_current_on_success() {
        let events = Rc::new(std::cell::RefCell::new(Vec::new()));
        let erased = Cell::new(None);
        let result = run_deposit_preauth_remove_from_ledger(
            Some("preauth"),
            |_| {
                events.borrow_mut().push("dir_remove");
                true
            },
            |_| {
                events.borrow_mut().push("owner");
                true
            },
            || events.borrow_mut().push("adjust"),
            |preauth| {
                events.borrow_mut().push("erase");
                erased.set(Some(preauth));
            },
        );

        assert_eq!(result, protocol::Ter::TES_SUCCESS);
        assert_eq!(
            events.borrow().as_slice(),
            ["dir_remove", "owner", "adjust", "erase"]
        );
        assert_eq!(erased.get(), Some("preauth"));
    }

    #[test]
    fn deposit_preauth_do_apply_runs_credential_paths_after_account_shell() {
        let events = Rc::new(std::cell::RefCell::new(Vec::new()));
        let mut sink = TestDoApplySink::new(Rc::clone(&events));

        let result = run_deposit_preauth_do_apply(
            DepositPreauthDoApplyAccountFacts::default(),
            DepositPreauthDoApplyCredentialFacts {
                authorize_credentials_present: true,
                unauthorize_credentials_present: false,
            },
            &mut sink,
        );

        assert_eq!(result, protocol::Ter::TES_SUCCESS);
        assert_eq!(
            events.borrow().as_slice(),
            [
                "cred_owner",
                "cred_reserve",
                "cred_sort",
                "cred_create",
                "cred_dir",
                "cred_owner_node",
                "cred_adjust",
            ]
        );
    }

    #[test]
    fn deposit_preauth_do_apply_defaults_to_tessuccess_when_no_path_exists() {
        let events = Rc::new(std::cell::RefCell::new(Vec::new()));
        let mut sink = TestDoApplySink::new(Rc::clone(&events));

        let result = run_deposit_preauth_do_apply(
            DepositPreauthDoApplyAccountFacts::default(),
            DepositPreauthDoApplyCredentialFacts::default(),
            &mut sink,
        );

        assert_eq!(result, protocol::Ter::TES_SUCCESS);
        assert!(events.borrow().is_empty());
    }
}
