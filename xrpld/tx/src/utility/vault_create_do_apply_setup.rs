//! Next front the reference implementation pseudo-account/setup shell.
//!
//! This ports the deterministic behavior around:
//!
//! - returning `createPseudoAccount(...)` failure unchanged,
//! - returning the first non-success `addEmptyHolding(...)` result unchanged,
//! - resolving `scale` with the current native-or-MPT zero rule and the current
//!   IOU default,
//! - preserving the original transaction flags,
//! - and deriving `mptFlags` with the current share-transferability and private
//!   vault rules.

use protocol::{Ter, is_tes_success};

use crate::vault_create_metadata::{VAULT_PRIVATE_FLAG, VAULT_SHARE_NON_TRANSFERABLE_FLAG};

pub const VAULT_DEFAULT_IOU_SCALE: u8 = 6;
pub const MPT_REQUIRE_AUTH_FLAG: u32 = 0x0000_0004;
pub const MPT_CAN_ESCROW_FLAG: u32 = 0x0000_0008;
pub const MPT_CAN_TRADE_FLAG: u32 = 0x0000_0010;
pub const MPT_CAN_TRANSFER_FLAG: u32 = 0x0000_0020;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VaultCreateDoApplySetupFacts {
    pub asset_is_mpt: bool,
    pub asset_is_native: bool,
    pub scale_field: Option<u8>,
    pub tx_flags: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VaultCreateDoApplySetup<PseudoId> {
    pub pseudo_id: PseudoId,
    pub scale: u8,
    pub tx_flags: u32,
    pub mpt_flags: u32,
}

pub fn load_vault_create_do_apply_setup<PseudoId, CreatePseudo, AddEmptyHolding>(
    facts: VaultCreateDoApplySetupFacts,
    create_pseudo: CreatePseudo,
    add_empty_holding: AddEmptyHolding,
) -> Result<VaultCreateDoApplySetup<PseudoId>, Ter>
where
    CreatePseudo: FnOnce() -> Result<PseudoId, Ter>,
    AddEmptyHolding: FnOnce(&PseudoId) -> Ter,
{
    let pseudo_id = create_pseudo()?;

    let ter = add_empty_holding(&pseudo_id);
    if !is_tes_success(ter) {
        return Err(ter);
    }

    let scale = if facts.asset_is_mpt || facts.asset_is_native {
        0
    } else {
        facts.scale_field.unwrap_or(VAULT_DEFAULT_IOU_SCALE)
    };

    let mut mpt_flags = 0;
    if (facts.tx_flags & VAULT_SHARE_NON_TRANSFERABLE_FLAG) == 0 {
        mpt_flags |= MPT_CAN_ESCROW_FLAG | MPT_CAN_TRADE_FLAG | MPT_CAN_TRANSFER_FLAG;
    }
    if (facts.tx_flags & VAULT_PRIVATE_FLAG) != 0 {
        mpt_flags |= MPT_REQUIRE_AUTH_FLAG;
    }

    Ok(VaultCreateDoApplySetup {
        pseudo_id,
        scale,
        tx_flags: facts.tx_flags,
        mpt_flags,
    })
}

#[cfg(test)]
mod tests {
    use std::{cell::Cell, rc::Rc};

    use protocol::{Ter, trans_token};

    use super::{
        MPT_CAN_ESCROW_FLAG, MPT_CAN_TRADE_FLAG, MPT_CAN_TRANSFER_FLAG, MPT_REQUIRE_AUTH_FLAG,
        VAULT_DEFAULT_IOU_SCALE, VaultCreateDoApplySetup, VaultCreateDoApplySetupFacts,
        load_vault_create_do_apply_setup,
    };
    use crate::vault_create_metadata::{VAULT_PRIVATE_FLAG, VAULT_SHARE_NON_TRANSFERABLE_FLAG};

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
}
