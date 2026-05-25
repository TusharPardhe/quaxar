//! Integration tests that pin the narrowed Rust `VaultSet.cpp::doApply()`
//! update-and-associate tail to the current C++ behavior.

use std::rc::Rc;

use protocol::{Ter, trans_token};
use tx::{
    VaultSetDoApplyDomainUpdate, VaultSetDoApplyFacts, VaultSetDoApplyIssuance,
    VaultSetDoApplySink, VaultSetDoApplyVault, run_vault_set_do_apply,
};

#[derive(Clone)]
struct TestVault {
    asset: Rc<str>,
    assets_total: i64,
    issuance_id: &'static str,
    steps: Rc<std::cell::RefCell<Vec<String>>>,
}

impl VaultSetDoApplyVault for TestVault {
    type Asset = Rc<str>;
    type Amount = i64;
    type Data = &'static str;
    type IssuanceId = &'static str;

    fn asset(&self) -> &Self::Asset {
        &self.asset
    }

    fn assets_total(&self) -> &Self::Amount {
        &self.assets_total
    }

    fn issuance_id(&self) -> &Self::IssuanceId {
        &self.issuance_id
    }

    fn set_data(&mut self, value: Self::Data) {
        self.steps.borrow_mut().push(format!("data={value}"));
    }

    fn set_assets_maximum(&mut self, value: Self::Amount) {
        self.steps
            .borrow_mut()
            .push(format!("assets_maximum={value}"));
    }
}

struct TestIssuance {
    has_domain_id: bool,
    steps: Rc<std::cell::RefCell<Vec<String>>>,
}

impl VaultSetDoApplyIssuance for TestIssuance {
    type DomainId = &'static str;

    fn has_domain_id(&self) -> bool {
        self.has_domain_id
    }

    fn set_domain_id(&mut self, value: Self::DomainId) {
        self.steps.borrow_mut().push(format!("set_domain={value}"));
    }

    fn clear_domain_id(&mut self) {
        self.steps.borrow_mut().push("clear_domain".to_string());
    }
}

struct TestSink {
    steps: Rc<std::cell::RefCell<Vec<String>>>,
    vault: Option<TestVault>,
    issuance: Option<TestIssuance>,
}

impl VaultSetDoApplySink for TestSink {
    type Vault = TestVault;
    type Issuance = TestIssuance;
    type Asset = Rc<str>;
    type Amount = i64;
    type Data = &'static str;
    type DomainId = &'static str;
    type IssuanceId = &'static str;

    fn read_vault(&mut self) -> Option<Self::Vault> {
        self.steps.borrow_mut().push("read_vault".to_string());
        self.vault.take()
    }

    fn read_issuance(&mut self, issuance_id: &Self::IssuanceId) -> Option<Self::Issuance> {
        self.steps
            .borrow_mut()
            .push(format!("read_issuance={issuance_id}"));
        self.issuance.take()
    }

    fn update_issuance(&mut self, _issuance: Self::Issuance) {
        self.steps.borrow_mut().push("update_issuance".to_string());
    }

    fn update_vault(&mut self, _vault: Self::Vault) {
        self.steps.borrow_mut().push("update_vault".to_string());
    }

    fn associate_asset(&mut self, asset: &Self::Asset) {
        self.steps.borrow_mut().push(format!("associate={asset}"));
    }
}

fn build_sink(
    steps: Rc<std::cell::RefCell<Vec<String>>>,
    vault_exists: bool,
    issuance_exists: bool,
    issuance_has_domain: bool,
) -> TestSink {
    TestSink {
        steps: Rc::clone(&steps),
        vault: vault_exists.then(|| TestVault {
            asset: Rc::from("USD"),
            assets_total: 100,
            issuance_id: "share-id",
            steps: Rc::clone(&steps),
        }),
        issuance: issuance_exists.then_some(TestIssuance {
            has_domain_id: issuance_has_domain,
            steps,
        }),
    }
}

#[test]
fn vault_set_do_apply_returns_tefinternal_when_vault_is_missing() {
    let steps = Rc::new(std::cell::RefCell::new(Vec::new()));
    let mut sink = build_sink(Rc::clone(&steps), false, true, false);

    let result = run_vault_set_do_apply(
        &mut sink,
        VaultSetDoApplyFacts {
            data: None::<&'static str>,
            assets_maximum: None,
            zero_amount: 0,
            domain_update: None::<VaultSetDoApplyDomainUpdate<&'static str>>,
        },
    );

    assert_eq!(result, Ter::TEF_INTERNAL);
    assert_eq!(trans_token(result), "tefINTERNAL");
    assert_eq!(steps.borrow().as_slice(), ["read_vault"]);
}

#[test]
fn vault_set_do_apply_returns_tefinternal_when_issuance_is_missing() {
    let steps = Rc::new(std::cell::RefCell::new(Vec::new()));
    let mut sink = build_sink(Rc::clone(&steps), true, false, false);

    let result = run_vault_set_do_apply(
        &mut sink,
        VaultSetDoApplyFacts {
            data: None::<&'static str>,
            assets_maximum: None,
            zero_amount: 0,
            domain_update: None::<VaultSetDoApplyDomainUpdate<&'static str>>,
        },
    );

    assert_eq!(result, Ter::TEF_INTERNAL);
    assert_eq!(
        steps.borrow().as_slice(),
        ["read_vault", "read_issuance=share-id"]
    );
}

#[test]
fn vault_set_do_apply_returns_limit_exceeded_before_updates() {
    let steps = Rc::new(std::cell::RefCell::new(Vec::new()));
    let mut sink = build_sink(Rc::clone(&steps), true, true, false);

    let result = run_vault_set_do_apply(
        &mut sink,
        VaultSetDoApplyFacts {
            data: None::<&'static str>,
            assets_maximum: Some(50),
            zero_amount: 0,
            domain_update: None::<VaultSetDoApplyDomainUpdate<&'static str>>,
        },
    );

    assert_eq!(result, Ter::TEC_LIMIT_EXCEEDED);
    assert_eq!(trans_token(result), "tecLIMIT_EXCEEDED");
    assert_eq!(
        steps.borrow().as_slice(),
        ["read_vault", "read_issuance=share-id"]
    );
}

#[test]
fn vault_set_do_apply_updates_domain_then_vault_then_associate() {
    let steps = Rc::new(std::cell::RefCell::new(Vec::new()));
    let mut sink = build_sink(Rc::clone(&steps), true, true, true);

    let result = run_vault_set_do_apply(
        &mut sink,
        VaultSetDoApplyFacts {
            data: Some("0"),
            assets_maximum: Some(150),
            zero_amount: 0,
            domain_update: Some(VaultSetDoApplyDomainUpdate::Set("42")),
        },
    );

    assert_eq!(result, Ter::TES_SUCCESS);
    assert_eq!(
        steps.borrow().as_slice(),
        [
            "read_vault",
            "read_issuance=share-id",
            "data=0",
            "assets_maximum=150",
            "set_domain=42",
            "update_issuance",
            "update_vault",
            "associate=USD",
        ]
    );
}

#[test]
fn vault_set_do_apply_clears_domain_only_when_present_but_still_updates_issuance() {
    let steps = Rc::new(std::cell::RefCell::new(Vec::new()));
    let mut sink = build_sink(Rc::clone(&steps), true, true, false);

    let result = run_vault_set_do_apply(
        &mut sink,
        VaultSetDoApplyFacts {
            data: None::<&'static str>,
            assets_maximum: Some(0),
            zero_amount: 0,
            domain_update: Some(VaultSetDoApplyDomainUpdate::Clear),
        },
    );

    assert_eq!(result, Ter::TES_SUCCESS);
    assert_eq!(
        steps.borrow().as_slice(),
        [
            "read_vault",
            "read_issuance=share-id",
            "assets_maximum=0",
            "update_issuance",
            "update_vault",
            "associate=USD",
        ]
    );
}
