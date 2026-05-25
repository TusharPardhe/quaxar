//! Integration tests that pin the higher narrowed Rust
//! `VaultCreate.cpp::doApply()` composition shell to the current C++ behavior.

use std::{cell::Cell, rc::Rc};

use protocol::{Ter, trans_token};
use tx::{
    MPT_CAN_ESCROW_FLAG, MPT_CAN_TRADE_FLAG, MPT_CAN_TRANSFER_FLAG, MPT_REQUIRE_AUTH_FLAG,
    VAULT_PRIVATE_FLAG, VaultCreateDoApplyAuthorizeRequest, VaultCreateDoApplyFacts,
    VaultCreateDoApplyVaultFieldSink, VaultCreateShareIssuanceRequest, run_vault_create_do_apply,
};

#[derive(Clone)]
struct RecordingVault {
    steps: Rc<std::cell::RefCell<Vec<String>>>,
}

impl RecordingVault {
    fn new(steps: Rc<std::cell::RefCell<Vec<String>>>) -> Self {
        Self { steps }
    }

    fn push(&self, step: &str) {
        self.steps.borrow_mut().push(step.to_string());
    }
}

impl VaultCreateDoApplyVaultFieldSink for RecordingVault {
    type Asset = &'static str;
    type AccountId = &'static str;
    type Amount = i64;
    type AssetsMaximum = &'static str;
    type ShareId = &'static str;
    type Data = &'static str;

    fn set_asset(&mut self, _value: Self::Asset) {
        self.push("asset");
    }

    fn set_flags(&mut self, _value: u32) {
        self.push("flags");
    }

    fn set_sequence(&mut self, _value: u32) {
        self.push("sequence");
    }

    fn set_owner(&mut self, _value: Self::AccountId) {
        self.push("owner");
    }

    fn set_account(&mut self, _value: Self::AccountId) {
        self.push("account");
    }

    fn set_assets_total(&mut self, _value: Self::Amount) {
        self.push("assets_total");
    }

    fn set_assets_available(&mut self, _value: Self::Amount) {
        self.push("assets_available");
    }

    fn set_loss_unrealized(&mut self, _value: Self::Amount) {
        self.push("loss_unrealized");
    }

    fn set_assets_maximum(&mut self, _value: Self::AssetsMaximum) {
        self.push("assets_maximum");
    }

    fn set_share_mpt_id(&mut self, _value: Self::ShareId) {
        self.push("share_mpt_id");
    }

    fn set_data(&mut self, _value: Self::Data) {
        self.push("data");
    }

    fn set_withdrawal_policy(&mut self, _value: u8) {
        self.push("withdrawal_policy");
    }

    fn set_scale(&mut self, _value: u8) {
        self.push("scale");
    }

    fn insert_vault(&mut self) {
        self.push("insert_vault");
    }
}

fn sample_facts() -> VaultCreateDoApplyFacts<
    &'static str,
    &'static str,
    i64,
    &'static str,
    &'static str,
    &'static str,
    &'static str,
> {
    VaultCreateDoApplyFacts {
        asset: "USD",
        sequence: 9,
        tx_flags: VAULT_PRIVATE_FLAG,
        owner_account: "owner",
        zero_amount: 0,
        assets_maximum: Some("1000"),
        metadata: Some("meta"),
        domain_id: Some("domain"),
        data: Some("data"),
        withdrawal_policy: Some(7),
        scale_field: Some(6),
        asset_is_mpt: false,
        asset_is_native: false,
    }
}

#[test]
fn vault_create_do_apply_runs_current_cpp_stage_order_for_private_vaults() {
    let steps = Rc::new(std::cell::RefCell::new(Vec::new()));

    let result = run_vault_create_do_apply(
        sample_facts(),
        || Some("owner-sle"),
        || RecordingVault::new(Rc::clone(&steps)),
        {
            let steps = Rc::clone(&steps);
            move |_| {
                steps.borrow_mut().push("dir".to_string());
                Ter::TES_SUCCESS
            }
        },
        {
            let steps = Rc::clone(&steps);
            move |_| {
                steps.borrow_mut().push("adjust".to_string());
            }
        },
        {
            let steps = Rc::clone(&steps);
            move |_| {
                steps.borrow_mut().push("reserve".to_string());
                true
            }
        },
        {
            let steps = Rc::clone(&steps);
            move || {
                steps.borrow_mut().push("pseudo".to_string());
                Ok("pseudo")
            }
        },
        {
            let steps = Rc::clone(&steps);
            move |pseudo| {
                steps.borrow_mut().push(format!("holding:{pseudo}"));
                Ter::TES_SUCCESS
            }
        },
        {
            let steps = Rc::clone(&steps);
            move |request: VaultCreateShareIssuanceRequest<'_, _, _, _>| {
                steps.borrow_mut().push("share".to_string());
                assert_eq!(request.account, &"pseudo");
                assert_eq!(
                    request.flags,
                    MPT_CAN_ESCROW_FLAG
                        | MPT_CAN_TRADE_FLAG
                        | MPT_CAN_TRANSFER_FLAG
                        | MPT_REQUIRE_AUTH_FLAG
                );
                assert_eq!(request.asset_scale, 6);
                Ok("share-id")
            }
        },
        {
            let steps = Rc::clone(&steps);
            move |request: VaultCreateDoApplyAuthorizeRequest<'_, _, _>| {
                steps.borrow_mut().push(format!(
                    "authorize:{}:{}",
                    request.account,
                    request.holder.copied().unwrap_or("none")
                ));
                Ter::TES_SUCCESS
            }
        },
        move |vault, asset| {
            vault.steps.borrow_mut().push(format!("associate:{asset}"));
        },
    );

    assert_eq!(result, Ter::TES_SUCCESS);
    assert_eq!(
        steps.borrow().as_slice(),
        [
            "dir",
            "adjust",
            "reserve",
            "pseudo",
            "holding:pseudo",
            "share",
            "asset",
            "flags",
            "sequence",
            "owner",
            "account",
            "assets_total",
            "assets_available",
            "loss_unrealized",
            "assets_maximum",
            "share_mpt_id",
            "data",
            "withdrawal_policy",
            "scale",
            "insert_vault",
            "authorize:owner:none",
            "authorize:pseudo:owner",
            "associate:USD",
        ]
    );
}

#[test]
fn vault_create_do_apply_returns_reserve_failure_before_setup() {
    let pseudo_called = Cell::new(false);

    let result = run_vault_create_do_apply(
        sample_facts(),
        || Some("owner-sle"),
        || RecordingVault::new(Rc::new(std::cell::RefCell::new(Vec::new()))),
        |_| Ter::TES_SUCCESS,
        |_| {},
        |_| false,
        || {
            pseudo_called.set(true);
            Ok("pseudo")
        },
        |_| Ter::TES_SUCCESS,
        |_| Ok("share-id"),
        |_| Ter::TES_SUCCESS,
        |_, _| {},
    );

    assert_eq!(result, Ter::TEC_INSUFFICIENT_RESERVE);
    assert_eq!(trans_token(result), "tecINSUFFICIENT_RESERVE");
    assert!(!pseudo_called.get());
}

#[test]
fn vault_create_do_apply_returns_setup_failure_before_share() {
    let share_called = Cell::new(false);

    let result = run_vault_create_do_apply(
        sample_facts(),
        || Some("owner-sle"),
        || RecordingVault::new(Rc::new(std::cell::RefCell::new(Vec::new()))),
        |_| Ter::TES_SUCCESS,
        |_| {},
        |_| true,
        || Err(Ter::TER_ADDRESS_COLLISION),
        |_| Ter::TES_SUCCESS,
        |_| {
            share_called.set(true);
            Ok("share-id")
        },
        |_| Ter::TES_SUCCESS,
        |_, _| {},
    );

    assert_eq!(result, Ter::TER_ADDRESS_COLLISION);
    assert_eq!(trans_token(result), "terADDRESS_COLLISION");
    assert!(!share_called.get());
}

#[test]
fn vault_create_do_apply_returns_share_failure_before_field_and_auth_work() {
    let authorize_called = Cell::new(false);

    let result = run_vault_create_do_apply(
        sample_facts(),
        || Some("owner-sle"),
        || RecordingVault::new(Rc::new(std::cell::RefCell::new(Vec::new()))),
        |_| Ter::TES_SUCCESS,
        |_| {},
        |_| true,
        || Ok("pseudo"),
        |_| Ter::TES_SUCCESS,
        |_| Err(Ter::TEC_INSUFFICIENT_RESERVE),
        |_| {
            authorize_called.set(true);
            Ter::TES_SUCCESS
        },
        |_, _| {},
    );

    assert_eq!(result, Ter::TEC_INSUFFICIENT_RESERVE);
    assert_eq!(trans_token(result), "tecINSUFFICIENT_RESERVE");
    assert!(!authorize_called.get());
}

#[test]
fn vault_create_do_apply_returns_authorization_failure_unchanged() {
    let associate_called = Cell::new(false);

    let result = run_vault_create_do_apply(
        sample_facts(),
        || Some("owner-sle"),
        || RecordingVault::new(Rc::new(std::cell::RefCell::new(Vec::new()))),
        |_| Ter::TES_SUCCESS,
        |_| {},
        |_| true,
        || Ok("pseudo"),
        |_| Ter::TES_SUCCESS,
        |_| Ok("share-id"),
        |_| Ter::TEC_NO_AUTH,
        |_, _| {
            associate_called.set(true);
        },
    );

    assert_eq!(result, Ter::TEC_NO_AUTH);
    assert_eq!(trans_token(result), "tecNO_AUTH");
    assert!(!associate_called.get());
}
