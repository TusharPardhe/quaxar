//! Current the reference implementation transaction shell.
//!
//! This ports the deterministic outer behavior around:
//!
//! - rejecting `sfRegularKey == sfAccount` in `preflight(...)`,
//! - loading the active account with the current `tefINTERNAL` fallback,
//! - arming `lsfPasswordSpent` only when the minimum-fee check
//!   fails,
//! - rejecting removal of the last signing path with
//!   `tecNO_ALTERNATIVE_KEY`,
//! - updating the regular key when present and clearing it when absent, and
//! - only persisting the mutated account on the success path.

use protocol::{NotTec, Ter};

pub trait SetRegularKeyTx {
    type AccountId: Clone + Eq;

    fn account_id(&self) -> Self::AccountId;
    fn regular_key(&self) -> Option<Self::AccountId>;
}

pub trait SetRegularKeyDoApplyAccount {
    type AccountId: Clone;

    fn master_disabled(&self) -> bool;
    fn set_password_spent(&mut self);
    fn set_regular_key(&mut self, account_id: Self::AccountId);
    fn clear_regular_key(&mut self);
}

pub trait SetRegularKeyDoApplySink {
    type AccountId: Clone;
    type Account: SetRegularKeyDoApplyAccount<AccountId = Self::AccountId>;

    fn read_account(&mut self) -> Option<Self::Account>;
    fn signer_list_present(&mut self) -> bool;
    fn update_account(&mut self, account: Self::Account);
}

pub fn run_set_regular_key_preflight<Tx>(tx: &Tx) -> NotTec
where
    Tx: SetRegularKeyTx,
{
    if tx.regular_key().as_ref() == Some(&tx.account_id()) {
        return Ter::TEM_BAD_REGKEY;
    }

    Ter::TES_SUCCESS
}

pub fn run_set_regular_key_do_apply<Tx, Sink>(tx: &Tx, spend_password: bool, sink: &mut Sink) -> Ter
where
    Tx: SetRegularKeyTx<AccountId = Sink::AccountId>,
    Sink: SetRegularKeyDoApplySink,
{
    let mut account = match sink.read_account() {
        Some(account) => account,
        None => return Ter::TEF_INTERNAL,
    };

    if spend_password {
        account.set_password_spent();
    }

    if let Some(regular_key) = tx.regular_key() {
        account.set_regular_key(regular_key);
    } else {
        if account.master_disabled() && !sink.signer_list_present() {
            return Ter::TEC_NO_ALTERNATIVE_KEY;
        }

        account.clear_regular_key();
    }

    sink.update_account(account);
    Ter::TES_SUCCESS
}

#[cfg(test)]
mod tests {
    use std::{cell::RefCell, rc::Rc};

    use protocol::trans_token;

    use super::{
        SetRegularKeyDoApplyAccount, SetRegularKeyDoApplySink, SetRegularKeyTx,
        run_set_regular_key_do_apply, run_set_regular_key_preflight,
    };

    #[derive(Clone)]
    struct TestTx {
        account_id: &'static str,
        regular_key: Option<&'static str>,
    }

    impl SetRegularKeyTx for TestTx {
        type AccountId = &'static str;

        fn account_id(&self) -> Self::AccountId {
            self.account_id
        }

        fn regular_key(&self) -> Option<Self::AccountId> {
            self.regular_key
        }
    }

    #[derive(Clone)]
    struct TestAccount {
        master_disabled: bool,
        steps: Rc<RefCell<Vec<String>>>,
    }

    impl SetRegularKeyDoApplyAccount for TestAccount {
        type AccountId = &'static str;

        fn master_disabled(&self) -> bool {
            self.master_disabled
        }

        fn set_password_spent(&mut self) {
            self.steps
                .borrow_mut()
                .push("set_password_spent".to_string());
        }

        fn set_regular_key(&mut self, account_id: Self::AccountId) {
            self.steps
                .borrow_mut()
                .push(format!("set_regular_key={account_id}"));
        }

        fn clear_regular_key(&mut self) {
            self.steps
                .borrow_mut()
                .push("clear_regular_key".to_string());
        }
    }

    struct TestSink {
        signer_list_present: bool,
        account: Option<TestAccount>,
        steps: Rc<RefCell<Vec<String>>>,
    }

    impl SetRegularKeyDoApplySink for TestSink {
        type AccountId = &'static str;
        type Account = TestAccount;

        fn read_account(&mut self) -> Option<Self::Account> {
            self.steps.borrow_mut().push("read_account".to_string());
            self.account.take()
        }

        fn signer_list_present(&mut self) -> bool {
            self.steps
                .borrow_mut()
                .push("check_signer_list".to_string());
            self.signer_list_present
        }

        fn update_account(&mut self, _account: Self::Account) {
            self.steps.borrow_mut().push("update_account".to_string());
        }
    }

    #[test]
    fn set_regular_key_preflight_rejects_self_regular_key() {
        let result = run_set_regular_key_preflight(&TestTx {
            account_id: "alice",
            regular_key: Some("alice"),
        });

        assert_eq!(result, protocol::Ter::TEM_BAD_REGKEY);
        assert_eq!(trans_token(result), "temBAD_REGKEY");
    }

    #[test]
    fn set_regular_key_preflight_accepts_missing_or_distinct_regular_key() {
        let missing = run_set_regular_key_preflight(&TestTx {
            account_id: "alice",
            regular_key: None,
        });
        let distinct = run_set_regular_key_preflight(&TestTx {
            account_id: "alice",
            regular_key: Some("bob"),
        });

        assert_eq!(missing, protocol::Ter::TES_SUCCESS);
        assert_eq!(distinct, protocol::Ter::TES_SUCCESS);
    }

    #[test]
    fn set_regular_key_do_apply_returns_internal_when_account_is_missing() {
        let steps = Rc::new(RefCell::new(Vec::new()));
        let mut sink = TestSink {
            signer_list_present: false,
            account: None,
            steps: steps.clone(),
        };

        let result = run_set_regular_key_do_apply(
            &TestTx {
                account_id: "alice",
                regular_key: Some("bob"),
            },
            false,
            &mut sink,
        );

        assert_eq!(result, protocol::Ter::TEF_INTERNAL);
        assert_eq!(steps.borrow().as_slice(), ["read_account"]);
    }

    #[test]
    fn set_regular_key_do_apply_arms_password_spent_before_setting_key() {
        let steps = Rc::new(RefCell::new(Vec::new()));
        let mut sink = TestSink {
            signer_list_present: false,
            account: Some(TestAccount {
                master_disabled: false,
                steps: steps.clone(),
            }),
            steps: steps.clone(),
        };

        let result = run_set_regular_key_do_apply(
            &TestTx {
                account_id: "alice",
                regular_key: Some("bob"),
            },
            true,
            &mut sink,
        );

        assert_eq!(result, protocol::Ter::TES_SUCCESS);
        assert_eq!(
            steps.borrow().as_slice(),
            [
                "read_account",
                "set_password_spent",
                "set_regular_key=bob",
                "update_account",
            ]
        );
    }

    #[test]
    fn set_regular_key_do_apply_rejects_removing_last_signing_path() {
        let steps = Rc::new(RefCell::new(Vec::new()));
        let mut sink = TestSink {
            signer_list_present: false,
            account: Some(TestAccount {
                master_disabled: true,
                steps: steps.clone(),
            }),
            steps: steps.clone(),
        };

        let result = run_set_regular_key_do_apply(
            &TestTx {
                account_id: "alice",
                regular_key: None,
            },
            true,
            &mut sink,
        );

        assert_eq!(result, protocol::Ter::TEC_NO_ALTERNATIVE_KEY);
        assert_eq!(trans_token(result), "tecNO_ALTERNATIVE_KEY");
        assert_eq!(
            steps.borrow().as_slice(),
            ["read_account", "set_password_spent", "check_signer_list"]
        );
    }

    #[test]
    fn set_regular_key_do_apply_clears_regular_key_when_an_alternative_exists() {
        let steps = Rc::new(RefCell::new(Vec::new()));
        let mut sink = TestSink {
            signer_list_present: true,
            account: Some(TestAccount {
                master_disabled: true,
                steps: steps.clone(),
            }),
            steps: steps.clone(),
        };

        let result = run_set_regular_key_do_apply(
            &TestTx {
                account_id: "alice",
                regular_key: None,
            },
            false,
            &mut sink,
        );

        assert_eq!(result, protocol::Ter::TES_SUCCESS);
        assert_eq!(
            steps.borrow().as_slice(),
            [
                "read_account",
                "check_signer_list",
                "clear_regular_key",
                "update_account",
            ]
        );
    }
}
