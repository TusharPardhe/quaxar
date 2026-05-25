//! Front the reference implementation shell.
//!
//! This ports the exact current behavior around:
//!
//! - `tefINTERNAL` when the vault lookup is unexpectedly missing,
//! - returning the first `removeEmptyHolding(...)` failure unchanged, and
//! - `tefBAD_LEDGER` when the pseudo-account lookup is unexpectedly missing.

use protocol::{Ter, is_tes_success};

pub trait VaultDeleteDoApplyFrontVault {
    type AccountId;
    type Asset;

    fn pseudo_id(&self) -> &Self::AccountId;
    fn asset(&self) -> &Self::Asset;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VaultDeleteDoApplyFront<Vault, PseudoAccount, AccountId, Asset> {
    pub vault: Vault,
    pub pseudo_id: AccountId,
    pub asset: Asset,
    pub pseudo_account: PseudoAccount,
}

pub fn load_vault_delete_do_apply_front<
    Vault,
    PseudoAccount,
    ReadVault,
    RemoveHolding,
    ReadPseudo,
>(
    read_vault: ReadVault,
    remove_holding: RemoveHolding,
    read_pseudo: ReadPseudo,
) -> Result<VaultDeleteDoApplyFront<Vault, PseudoAccount, Vault::AccountId, Vault::Asset>, Ter>
where
    Vault: VaultDeleteDoApplyFrontVault,
    Vault::AccountId: Clone,
    Vault::Asset: Clone,
    ReadVault: FnOnce() -> Option<Vault>,
    RemoveHolding: FnOnce(&Vault::AccountId, &Vault::Asset) -> Ter,
    ReadPseudo: FnOnce(&Vault::AccountId) -> Option<PseudoAccount>,
{
    let vault = read_vault().ok_or(Ter::TEF_INTERNAL)?;
    let asset = vault.asset().clone();
    let pseudo_id = vault.pseudo_id().clone();

    let ter = remove_holding(&pseudo_id, &asset);
    if !is_tes_success(ter) {
        return Err(ter);
    }

    let pseudo_account = read_pseudo(&pseudo_id).ok_or(Ter::TEF_BAD_LEDGER)?;

    Ok(VaultDeleteDoApplyFront {
        vault,
        pseudo_id,
        asset,
        pseudo_account,
    })
}

#[cfg(test)]
mod tests {
    use std::{cell::Cell, rc::Rc};

    use protocol::{Ter, trans_token};

    use super::{
        VaultDeleteDoApplyFront, VaultDeleteDoApplyFrontVault, load_vault_delete_do_apply_front,
    };

    #[derive(Debug, Clone, PartialEq, Eq)]
    struct TestVault {
        pseudo_id: &'static str,
        asset: &'static str,
    }

    impl VaultDeleteDoApplyFrontVault for TestVault {
        type AccountId = &'static str;
        type Asset = &'static str;

        fn pseudo_id(&self) -> &Self::AccountId {
            &self.pseudo_id
        }

        fn asset(&self) -> &Self::Asset {
            &self.asset
        }
    }

    fn test_vault() -> TestVault {
        TestVault {
            pseudo_id: "vault-pseudo",
            asset: "USD",
        }
    }

    #[test]
    fn vault_delete_do_apply_front_returns_tefinternal_when_vault_is_missing() {
        let removed = Cell::new(false);

        let result = load_vault_delete_do_apply_front(
            || None::<TestVault>,
            |_, _| {
                removed.set(true);
                Ter::TES_SUCCESS
            },
            |_| Some("pseudo"),
        );

        assert_eq!(result, Err(Ter::TEF_INTERNAL));
        assert_eq!(trans_token(result.unwrap_err()), "tefINTERNAL");
        assert!(!removed.get());
    }

    #[test]
    fn vault_delete_do_apply_front_returns_remove_empty_holding_failure_unchanged() {
        let pseudo_read = Cell::new(false);

        let result = load_vault_delete_do_apply_front(
            || Some(test_vault()),
            |pseudo_id, asset| {
                assert_eq!(*pseudo_id, "vault-pseudo");
                assert_eq!(*asset, "USD");
                Ter::TEC_INTERNAL
            },
            |_| {
                pseudo_read.set(true);
                Some("pseudo")
            },
        );

        assert_eq!(result, Err(Ter::TEC_INTERNAL));
        assert_eq!(trans_token(result.unwrap_err()), "tecINTERNAL");
        assert!(!pseudo_read.get());
    }

    #[test]
    fn vault_delete_do_apply_front_returns_tefbad_ledger_when_pseudo_account_is_missing() {
        let result = load_vault_delete_do_apply_front(
            || Some(test_vault()),
            |_, _| Ter::TES_SUCCESS,
            |_| None::<&'static str>,
        );

        assert_eq!(result, Err(Ter::TEF_BAD_LEDGER));
        assert_eq!(trans_token(result.unwrap_err()), "tefBAD_LEDGER");
    }

    #[test]
    fn vault_delete_do_apply_front_loads_vault_and_pseudo_after_holding_removal() {
        let events = Rc::new(std::cell::RefCell::new(Vec::new()));

        let result = load_vault_delete_do_apply_front(
            || Some(test_vault()),
            {
                let events = Rc::clone(&events);
                move |pseudo_id, asset| {
                    events.borrow_mut().push("remove");
                    assert_eq!(*pseudo_id, "vault-pseudo");
                    assert_eq!(*asset, "USD");
                    Ter::TES_SUCCESS
                }
            },
            {
                let events = Rc::clone(&events);
                move |pseudo_id| {
                    events.borrow_mut().push("pseudo");
                    assert_eq!(*pseudo_id, "vault-pseudo");
                    Some("pseudo-account")
                }
            },
        );

        assert_eq!(
            result,
            Ok(VaultDeleteDoApplyFront {
                vault: test_vault(),
                pseudo_id: "vault-pseudo",
                asset: "USD",
                pseudo_account: "pseudo-account",
            })
        );
        assert_eq!(events.borrow().as_slice(), ["remove", "pseudo"]);
    }
}
