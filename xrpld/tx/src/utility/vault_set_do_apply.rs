//! Deterministic
//! the reference implementation update-and-associate tail.
//!
//! This ports the exact ordered behavior around:
//!
//! - loading the vault and share issuance with current `tefINTERNAL` fallback,
//! - updating optional data and optional assets-maximum fields on the vault,
//! - enforcing the current non-zero maximum-versus-assets-total limit guard,
//! - applying optional domain updates on the issuance,
//! - updating the issuance before the vault when `sfDomainID` is present,
//! - always updating the vault before `associateAsset(...)`,
//! - and returning `tesSUCCESS` after the final asset association.

use protocol::Ter;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VaultSetDoApplyDomainUpdate<DomainId> {
    Set(DomainId),
    Clear,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VaultSetDoApplyFacts<Amount, Data, DomainId> {
    pub data: Option<Data>,
    pub assets_maximum: Option<Amount>,
    pub zero_amount: Amount,
    pub domain_update: Option<VaultSetDoApplyDomainUpdate<DomainId>>,
}

pub trait VaultSetDoApplyVault {
    type Asset;
    type Amount;
    type Data;
    type IssuanceId;

    fn asset(&self) -> &Self::Asset;
    fn assets_total(&self) -> &Self::Amount;
    fn issuance_id(&self) -> &Self::IssuanceId;
    fn set_data(&mut self, value: Self::Data);
    fn set_assets_maximum(&mut self, value: Self::Amount);
}

pub trait VaultSetDoApplyIssuance {
    type DomainId;

    fn has_domain_id(&self) -> bool;
    fn set_domain_id(&mut self, value: Self::DomainId);
    fn clear_domain_id(&mut self);
}

pub trait VaultSetDoApplySink {
    type Vault: VaultSetDoApplyVault<
            Asset = Self::Asset,
            Amount = Self::Amount,
            Data = Self::Data,
            IssuanceId = Self::IssuanceId,
        >;
    type Issuance: VaultSetDoApplyIssuance<DomainId = Self::DomainId>;
    type Asset;
    type Amount;
    type Data;
    type DomainId;
    type IssuanceId;

    fn read_vault(&mut self) -> Option<Self::Vault>;
    fn read_issuance(&mut self, issuance_id: &Self::IssuanceId) -> Option<Self::Issuance>;
    fn update_issuance(&mut self, issuance: Self::Issuance);
    fn update_vault(&mut self, vault: Self::Vault);
    fn associate_asset(&mut self, asset: &Self::Asset);
}

pub fn run_vault_set_do_apply<Sink>(
    sink: &mut Sink,
    facts: VaultSetDoApplyFacts<Sink::Amount, Sink::Data, Sink::DomainId>,
) -> Ter
where
    Sink: VaultSetDoApplySink,
    Sink::Asset: Clone,
    Sink::Amount: PartialEq + PartialOrd,
    Sink::IssuanceId: Clone,
{
    let mut vault = match sink.read_vault() {
        Some(vault) => vault,
        None => return Ter::TEF_INTERNAL,
    };

    let vault_asset = vault.asset().clone();
    let issuance_id = vault.issuance_id().clone();
    let mut issuance = match sink.read_issuance(&issuance_id) {
        Some(issuance) => issuance,
        None => return Ter::TEF_INTERNAL,
    };

    if let Some(data) = facts.data {
        vault.set_data(data);
    }

    if let Some(value) = facts.assets_maximum {
        if value != facts.zero_amount && value < *vault.assets_total() {
            return Ter::TEC_LIMIT_EXCEEDED;
        }
        vault.set_assets_maximum(value);
    }

    if let Some(domain_update) = facts.domain_update {
        match domain_update {
            VaultSetDoApplyDomainUpdate::Set(domain_id) => issuance.set_domain_id(domain_id),
            VaultSetDoApplyDomainUpdate::Clear => {
                if issuance.has_domain_id() {
                    issuance.clear_domain_id();
                }
            }
        }
        sink.update_issuance(issuance);
    }

    sink.update_vault(vault);
    sink.associate_asset(&vault_asset);
    Ter::TES_SUCCESS
}

#[cfg(test)]
mod tests {
    use std::rc::Rc;

    use protocol::{Ter, trans_token};

    use super::{
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
            issuance: issuance_exists.then(|| TestIssuance {
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
}
