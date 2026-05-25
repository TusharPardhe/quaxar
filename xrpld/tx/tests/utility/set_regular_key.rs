//! Integration tests that pin the narrowed Rust `SetRegularKey.cpp`
//! transaction shell to the current C++ behavior.

use std::{cell::RefCell, rc::Rc};

use protocol::{Ter, trans_token};
use tx::{
    SetRegularKeyDoApplyAccount, SetRegularKeyDoApplySink, SetRegularKeyTx,
    run_set_regular_key_do_apply, run_set_regular_key_preflight,
};

#[derive(Clone)]
struct StubTx {
    account_id: &'static str,
    regular_key: Option<&'static str>,
}

impl SetRegularKeyTx for StubTx {
    type AccountId = &'static str;

    fn account_id(&self) -> Self::AccountId {
        self.account_id
    }

    fn regular_key(&self) -> Option<Self::AccountId> {
        self.regular_key
    }
}

#[derive(Clone)]
struct StubAccount {
    master_disabled: bool,
    steps: Rc<RefCell<Vec<String>>>,
}

impl SetRegularKeyDoApplyAccount for StubAccount {
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

struct StubSink {
    signer_list_present: bool,
    account: Option<StubAccount>,
    steps: Rc<RefCell<Vec<String>>>,
}

impl SetRegularKeyDoApplySink for StubSink {
    type AccountId = &'static str;
    type Account = StubAccount;

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
fn tx_set_regular_key_preflight_rejects_self_regular_key() {
    let result = run_set_regular_key_preflight(&StubTx {
        account_id: "alice",
        regular_key: Some("alice"),
    });

    assert_eq!(result, Ter::TEM_BAD_REGKEY);
    assert_eq!(trans_token(result), "temBAD_REGKEY");
}

#[test]
fn tx_set_regular_key_preflight_accepts_distinct_regular_key() {
    let result = run_set_regular_key_preflight(&StubTx {
        account_id: "alice",
        regular_key: Some("bob"),
    });

    assert_eq!(result, Ter::TES_SUCCESS);
    assert_eq!(trans_token(result), "tesSUCCESS");
}

#[test]
fn tx_set_regular_key_do_apply_returns_internal_when_account_lookup_misses() {
    let steps = Rc::new(RefCell::new(Vec::new()));
    let mut sink = StubSink {
        signer_list_present: false,
        account: None,
        steps: steps.clone(),
    };

    let result = run_set_regular_key_do_apply(
        &StubTx {
            account_id: "alice",
            regular_key: Some("bob"),
        },
        false,
        &mut sink,
    );

    assert_eq!(result, Ter::TEF_INTERNAL);
    assert_eq!(steps.borrow().as_slice(), ["read_account"]);
}

#[test]
fn tx_set_regular_key_do_apply_sets_password_spent_and_regular_key() {
    let steps = Rc::new(RefCell::new(Vec::new()));
    let mut sink = StubSink {
        signer_list_present: false,
        account: Some(StubAccount {
            master_disabled: false,
            steps: steps.clone(),
        }),
        steps: steps.clone(),
    };

    let result = run_set_regular_key_do_apply(
        &StubTx {
            account_id: "alice",
            regular_key: Some("bob"),
        },
        true,
        &mut sink,
    );

    assert_eq!(result, Ter::TES_SUCCESS);
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
fn tx_set_regular_key_do_apply_rejects_removing_last_signing_path() {
    let steps = Rc::new(RefCell::new(Vec::new()));
    let mut sink = StubSink {
        signer_list_present: false,
        account: Some(StubAccount {
            master_disabled: true,
            steps: steps.clone(),
        }),
        steps: steps.clone(),
    };

    let result = run_set_regular_key_do_apply(
        &StubTx {
            account_id: "alice",
            regular_key: None,
        },
        true,
        &mut sink,
    );

    assert_eq!(result, Ter::TEC_NO_ALTERNATIVE_KEY);
    assert_eq!(trans_token(result), "tecNO_ALTERNATIVE_KEY");
    assert_eq!(
        steps.borrow().as_slice(),
        ["read_account", "set_password_spent", "check_signer_list"]
    );
}

#[test]
fn tx_set_regular_key_do_apply_clears_regular_key_when_signer_list_exists() {
    let steps = Rc::new(RefCell::new(Vec::new()));
    let mut sink = StubSink {
        signer_list_present: true,
        account: Some(StubAccount {
            master_disabled: true,
            steps: steps.clone(),
        }),
        steps: steps.clone(),
    };

    let result = run_set_regular_key_do_apply(
        &StubTx {
            account_id: "alice",
            regular_key: None,
        },
        false,
        &mut sink,
    );

    assert_eq!(result, Ter::TES_SUCCESS);
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
