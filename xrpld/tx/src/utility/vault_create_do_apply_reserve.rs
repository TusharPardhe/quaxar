//! Front the reference implementation owner/vault/reserve shell.
//!
//! This ports the exact current behavior around:
//!
//! - `tefINTERNAL` when the owner account is missing,
//! - constructing the vault before the directory-link attempt,
//! - returning the first `dirLink(...)` failure unchanged,
//! - adjusting owner count before the reserve guard,
//! - and mapping the reserve shortfall to `tecINSUFFICIENT_RESERVE`.

use protocol::{Ter, is_tes_success};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VaultCreateDoApplyReserveSetup<Owner, Vault> {
    pub owner: Owner,
    pub vault: Vault,
}

pub fn load_vault_create_do_apply_reserve_setup<
    Owner,
    Vault,
    ReadOwner,
    MakeVault,
    DirLink,
    AdjustOwnerCount,
    HasReserve,
>(
    read_owner: ReadOwner,
    make_vault: MakeVault,
    dir_link: DirLink,
    adjust_owner_count: AdjustOwnerCount,
    has_reserve: HasReserve,
) -> Result<VaultCreateDoApplyReserveSetup<Owner, Vault>, Ter>
where
    ReadOwner: FnOnce() -> Option<Owner>,
    MakeVault: FnOnce() -> Vault,
    DirLink: FnOnce(&Vault) -> Ter,
    AdjustOwnerCount: FnOnce(&mut Owner),
    HasReserve: FnOnce(&Owner) -> bool,
{
    let mut owner = read_owner().ok_or(Ter::TEF_INTERNAL)?;
    let vault = make_vault();

    let ter = dir_link(&vault);
    if !is_tes_success(ter) {
        return Err(ter);
    }

    adjust_owner_count(&mut owner);
    if !has_reserve(&owner) {
        return Err(Ter::TEC_INSUFFICIENT_RESERVE);
    }

    Ok(VaultCreateDoApplyReserveSetup { owner, vault })
}

#[cfg(test)]
mod tests {
    use std::{cell::Cell, rc::Rc};

    use protocol::{Ter, trans_token};

    use super::{VaultCreateDoApplyReserveSetup, load_vault_create_do_apply_reserve_setup};

    #[derive(Debug, Clone, PartialEq, Eq)]
    struct TestOwner {
        owner_count: u32,
    }

    #[test]
    fn vault_create_do_apply_reserve_returns_tefinternal_when_owner_is_missing() {
        let vault_created = Cell::new(false);

        let result = load_vault_create_do_apply_reserve_setup(
            || None::<TestOwner>,
            || {
                vault_created.set(true);
                "vault"
            },
            |_| Ter::TES_SUCCESS,
            |_| unreachable!("missing owner should skip owner-count adjustment"),
            |_| true,
        );

        assert_eq!(result, Err(Ter::TEF_INTERNAL));
        assert_eq!(trans_token(result.unwrap_err()), "tefINTERNAL");
        assert!(!vault_created.get());
    }

    #[test]
    fn vault_create_do_apply_reserve_returns_dir_link_failure_unchanged() {
        let adjusted = Cell::new(false);

        let result = load_vault_create_do_apply_reserve_setup(
            || Some(TestOwner { owner_count: 3 }),
            || "vault",
            |_| Ter::TEC_DIR_FULL,
            |_| adjusted.set(true),
            |_| true,
        );

        assert_eq!(result, Err(Ter::TEC_DIR_FULL));
        assert_eq!(trans_token(result.unwrap_err()), "tecDIR_FULL");
        assert!(!adjusted.get());
    }

    #[test]
    fn vault_create_do_apply_reserve_adjusts_owner_count_before_reserve_check() {
        let seen = Rc::new(std::cell::RefCell::new(Vec::new()));

        let result = load_vault_create_do_apply_reserve_setup(
            || Some(TestOwner { owner_count: 3 }),
            || "vault",
            {
                let seen = Rc::clone(&seen);
                move |_| {
                    seen.borrow_mut().push("dir");
                    Ter::TES_SUCCESS
                }
            },
            {
                let seen = Rc::clone(&seen);
                move |owner: &mut TestOwner| {
                    seen.borrow_mut().push("adjust");
                    owner.owner_count += 2;
                }
            },
            {
                let seen = Rc::clone(&seen);
                move |owner| {
                    seen.borrow_mut().push("reserve");
                    assert_eq!(owner.owner_count, 5);
                    true
                }
            },
        );

        assert_eq!(
            result,
            Ok(VaultCreateDoApplyReserveSetup {
                owner: TestOwner { owner_count: 5 },
                vault: "vault",
            })
        );
        assert_eq!(seen.borrow().as_slice(), ["dir", "adjust", "reserve"]);
    }

    #[test]
    fn vault_create_do_apply_reserve_maps_shortfall_to_tecinsufficient_reserve() {
        let result = load_vault_create_do_apply_reserve_setup(
            || Some(TestOwner { owner_count: 7 }),
            || "vault",
            |_| Ter::TES_SUCCESS,
            |owner| owner.owner_count += 2,
            |_| false,
        );

        assert_eq!(result, Err(Ter::TEC_INSUFFICIENT_RESERVE));
        assert_eq!(trans_token(result.unwrap_err()), "tecINSUFFICIENT_RESERVE");
    }

    #[test]
    fn vault_create_do_apply_reserve_returns_loaded_setup_on_success() {
        let result = load_vault_create_do_apply_reserve_setup(
            || Some(TestOwner { owner_count: 10 }),
            || "vault-keylet",
            |vault| {
                assert_eq!(*vault, "vault-keylet");
                Ter::TES_SUCCESS
            },
            |owner| owner.owner_count += 2,
            |owner| owner.owner_count == 12,
        );

        assert_eq!(
            result,
            Ok(VaultCreateDoApplyReserveSetup {
                owner: TestOwner { owner_count: 12 },
                vault: "vault-keylet",
            })
        );
    }
}
