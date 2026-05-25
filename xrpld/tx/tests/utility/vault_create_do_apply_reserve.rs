//! Integration tests that pin the narrowed Rust
//! `VaultCreate.cpp::doApply()` owner/vault/reserve shell to the current C++
//! behavior.

use std::{cell::Cell, rc::Rc};

use protocol::{Ter, trans_token};
use tx::{VaultCreateDoApplyReserveSetup, load_vault_create_do_apply_reserve_setup};

#[derive(Debug, Clone, PartialEq, Eq)]
struct StubOwner {
    owner_count: u32,
}

#[test]
fn vault_create_do_apply_reserve_returns_tefinternal_when_owner_is_missing() {
    let vault_created = Cell::new(false);

    let result = load_vault_create_do_apply_reserve_setup(
        || None::<StubOwner>,
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
        || Some(StubOwner { owner_count: 3 }),
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
        || Some(StubOwner { owner_count: 3 }),
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
            move |owner: &mut StubOwner| {
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
            owner: StubOwner { owner_count: 5 },
            vault: "vault",
        })
    );
    assert_eq!(seen.borrow().as_slice(), ["dir", "adjust", "reserve"]);
}

#[test]
fn vault_create_do_apply_reserve_maps_shortfall_to_tecinsufficient_reserve() {
    let result = load_vault_create_do_apply_reserve_setup(
        || Some(StubOwner { owner_count: 7 }),
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
        || Some(StubOwner { owner_count: 10 }),
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
            owner: StubOwner { owner_count: 12 },
            vault: "vault-keylet",
        })
    );
}
