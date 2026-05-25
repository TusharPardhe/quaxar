//! Integration tests that pin the narrowed Rust
//! `VaultCreate.cpp::doApply()` pseudo-account/setup shell to the current C++
//! behavior.

use std::{cell::Cell, rc::Rc};

use protocol::{Ter, trans_token};
use tx::{
    MPT_CAN_ESCROW_FLAG, MPT_CAN_TRADE_FLAG, MPT_CAN_TRANSFER_FLAG, MPT_REQUIRE_AUTH_FLAG,
    VAULT_DEFAULT_IOU_SCALE, VAULT_PRIVATE_FLAG, VAULT_SHARE_NON_TRANSFERABLE_FLAG,
    VaultCreateDoApplySetup, VaultCreateDoApplySetupFacts, load_vault_create_do_apply_setup,
};

#[test]
fn vault_create_do_apply_setup_returns_create_pseudo_failure_unchanged() {
    let holding_called = Cell::new(false);

    let result = load_vault_create_do_apply_setup(
        VaultCreateDoApplySetupFacts {
            asset_is_mpt: false,
            asset_is_native: false,
            scale_field: None,
            tx_flags: 0,
        },
        || Err(Ter::TER_ADDRESS_COLLISION),
        |_: &&str| {
            holding_called.set(true);
            Ter::TES_SUCCESS
        },
    );

    assert_eq!(result, Err(Ter::TER_ADDRESS_COLLISION));
    assert_eq!(trans_token(result.unwrap_err()), "terADDRESS_COLLISION");
    assert!(!holding_called.get());
}

#[test]
fn vault_create_do_apply_setup_returns_add_empty_holding_failure_unchanged() {
    let result = load_vault_create_do_apply_setup(
        VaultCreateDoApplySetupFacts {
            asset_is_mpt: false,
            asset_is_native: false,
            scale_field: Some(9),
            tx_flags: 0,
        },
        || Ok("pseudo"),
        |pseudo_id| {
            assert_eq!(*pseudo_id, "pseudo");
            Ter::TER_NO_RIPPLE
        },
    );

    assert_eq!(result, Err(Ter::TER_NO_RIPPLE));
    assert_eq!(trans_token(result.unwrap_err()), "terNO_RIPPLE");
}

#[test]
fn vault_create_do_apply_setup_uses_zero_scale_for_native_and_mpt() {
    let native = load_vault_create_do_apply_setup(
        VaultCreateDoApplySetupFacts {
            asset_is_mpt: false,
            asset_is_native: true,
            scale_field: Some(18),
            tx_flags: 0,
        },
        || Ok("native"),
        |_| Ter::TES_SUCCESS,
    );
    let mpt = load_vault_create_do_apply_setup(
        VaultCreateDoApplySetupFacts {
            asset_is_mpt: true,
            asset_is_native: false,
            scale_field: Some(18),
            tx_flags: 0,
        },
        || Ok("mpt"),
        |_| Ter::TES_SUCCESS,
    );

    assert_eq!(native.unwrap().scale, 0);
    assert_eq!(mpt.unwrap().scale, 0);
}

#[test]
fn vault_create_do_apply_setup_uses_default_iou_scale() {
    let result = load_vault_create_do_apply_setup(
        VaultCreateDoApplySetupFacts {
            asset_is_mpt: false,
            asset_is_native: false,
            scale_field: None,
            tx_flags: 0,
        },
        || Ok("pseudo"),
        |_| Ter::TES_SUCCESS,
    );

    assert_eq!(result.unwrap().scale, VAULT_DEFAULT_IOU_SCALE);
}

#[test]
fn vault_create_do_apply_setup_derives_transferable_and_private_mpt_flags() {
    let public_transferable = load_vault_create_do_apply_setup(
        VaultCreateDoApplySetupFacts {
            asset_is_mpt: false,
            asset_is_native: false,
            scale_field: Some(7),
            tx_flags: 0,
        },
        || Ok("pseudo"),
        |_| Ter::TES_SUCCESS,
    );
    let private_non_transferable = load_vault_create_do_apply_setup(
        VaultCreateDoApplySetupFacts {
            asset_is_mpt: false,
            asset_is_native: false,
            scale_field: Some(7),
            tx_flags: VAULT_PRIVATE_FLAG | VAULT_SHARE_NON_TRANSFERABLE_FLAG,
        },
        || Ok("pseudo"),
        |_| Ter::TES_SUCCESS,
    );

    assert_eq!(
        public_transferable.unwrap().mpt_flags,
        MPT_CAN_ESCROW_FLAG | MPT_CAN_TRADE_FLAG | MPT_CAN_TRANSFER_FLAG
    );
    assert_eq!(
        private_non_transferable.unwrap().mpt_flags,
        MPT_REQUIRE_AUTH_FLAG
    );
}

#[test]
fn vault_create_do_apply_setup_runs_in_current() {
    let seen = Rc::new(std::cell::RefCell::new(Vec::new()));

    let result = load_vault_create_do_apply_setup(
        VaultCreateDoApplySetupFacts {
            asset_is_mpt: false,
            asset_is_native: false,
            scale_field: Some(11),
            tx_flags: VAULT_PRIVATE_FLAG,
        },
        {
            let seen = Rc::clone(&seen);
            move || {
                seen.borrow_mut().push("pseudo");
                Ok("pseudo-id")
            }
        },
        {
            let seen = Rc::clone(&seen);
            move |pseudo_id| {
                seen.borrow_mut().push("holding");
                assert_eq!(*pseudo_id, "pseudo-id");
                Ter::TES_SUCCESS
            }
        },
    );

    assert_eq!(
        result,
        Ok(VaultCreateDoApplySetup {
            pseudo_id: "pseudo-id",
            scale: 11,
            tx_flags: VAULT_PRIVATE_FLAG,
            mpt_flags: MPT_CAN_ESCROW_FLAG
                | MPT_CAN_TRADE_FLAG
                | MPT_CAN_TRANSFER_FLAG
                | MPT_REQUIRE_AUTH_FLAG,
        })
    );
    assert_eq!(seen.borrow().as_slice(), ["pseudo", "holding"]);
}
