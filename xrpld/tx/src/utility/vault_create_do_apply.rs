//! Higher top-level the reference implementation shell.
//!
//! This ports the exact deterministic composition around:
//!
//! - the front reserve shell,
//! - the pseudo-account and setup shell,
//! - the share-issuance creation shell,
//! - the vault-field population shell,
//! - and the final authorization plus asset-association tail,
//!   returning the first failing `TER` unchanged.

use protocol::Ter;

use crate::{
    VaultCreateDoApplyAuthorizeRequest, VaultCreateDoApplyReserveSetup,
    VaultCreateDoApplySetupFacts, VaultCreateDoApplyVaultFieldSink, VaultCreateDoApplyVaultFields,
    VaultCreateShareIssuanceInputs, VaultCreateShareIssuanceRequest,
    load_vault_create_do_apply_reserve_setup, load_vault_create_do_apply_setup,
    run_vault_create_do_apply_authorization_tail, run_vault_create_do_apply_share_issuance,
    run_vault_create_do_apply_vault_fields,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VaultCreateDoApplyFacts<
    Asset,
    AccountId,
    Amount,
    AssetsMaximum,
    Metadata,
    DomainId,
    Data,
> {
    pub asset: Asset,
    pub sequence: u32,
    pub tx_flags: u32,
    pub owner_account: AccountId,
    pub zero_amount: Amount,
    pub assets_maximum: Option<AssetsMaximum>,
    pub metadata: Option<Metadata>,
    pub domain_id: Option<DomainId>,
    pub data: Option<Data>,
    pub withdrawal_policy: Option<u8>,
    pub scale_field: Option<u8>,
    pub asset_is_mpt: bool,
    pub asset_is_native: bool,
}

#[allow(clippy::too_many_arguments)]
pub fn run_vault_create_do_apply<
    Owner,
    Vault,
    AccountId,
    Asset,
    Amount,
    AssetsMaximum,
    Metadata,
    DomainId,
    Data,
    ShareId,
    ReadOwner,
    MakeVault,
    DirLink,
    AdjustOwnerCount,
    HasReserve,
    CreatePseudo,
    AddEmptyHolding,
    CreateShare,
    Authorize,
    AssociateAsset,
>(
    facts: VaultCreateDoApplyFacts<
        Asset,
        AccountId,
        Amount,
        AssetsMaximum,
        Metadata,
        DomainId,
        Data,
    >,
    read_owner: ReadOwner,
    make_vault: MakeVault,
    dir_link: DirLink,
    adjust_owner_count: AdjustOwnerCount,
    has_reserve: HasReserve,
    create_pseudo: CreatePseudo,
    add_empty_holding: AddEmptyHolding,
    create_share: CreateShare,
    authorize: Authorize,
    associate_asset: AssociateAsset,
) -> Ter
where
    Vault: VaultCreateDoApplyVaultFieldSink<
            Asset = Asset,
            AccountId = AccountId,
            Amount = Amount,
            AssetsMaximum = AssetsMaximum,
            ShareId = ShareId,
            Data = Data,
        >,
    AccountId: Clone,
    Asset: Clone,
    Amount: Clone,
    ShareId: Clone,
    ReadOwner: FnOnce() -> Option<Owner>,
    MakeVault: FnOnce() -> Vault,
    DirLink: FnOnce(&Vault) -> Ter,
    AdjustOwnerCount: FnOnce(&mut Owner),
    HasReserve: FnOnce(&Owner) -> bool,
    CreatePseudo: FnOnce() -> Result<AccountId, Ter>,
    AddEmptyHolding: FnOnce(&AccountId) -> Ter,
    CreateShare: FnOnce(
        VaultCreateShareIssuanceRequest<'_, AccountId, Metadata, DomainId>,
    ) -> Result<ShareId, Ter>,
    Authorize: FnMut(VaultCreateDoApplyAuthorizeRequest<'_, ShareId, AccountId>) -> Ter,
    AssociateAsset: FnOnce(&mut Vault, &Asset),
{
    let VaultCreateDoApplyReserveSetup {
        owner: _owner,
        mut vault,
    } = match load_vault_create_do_apply_reserve_setup(
        read_owner,
        make_vault,
        dir_link,
        adjust_owner_count,
        has_reserve,
    ) {
        Ok(setup) => setup,
        Err(err) => return err,
    };

    let setup = match load_vault_create_do_apply_setup(
        VaultCreateDoApplySetupFacts {
            asset_is_mpt: facts.asset_is_mpt,
            asset_is_native: facts.asset_is_native,
            scale_field: facts.scale_field,
            tx_flags: facts.tx_flags,
        },
        create_pseudo,
        add_empty_holding,
    ) {
        Ok(setup) => setup,
        Err(err) => return err,
    };

    let share_mpt_id = match run_vault_create_do_apply_share_issuance(
        &VaultCreateShareIssuanceInputs {
            pseudo_id: setup.pseudo_id.clone(),
            mpt_flags: setup.mpt_flags,
            scale: setup.scale,
            metadata: facts.metadata,
            domain_id: facts.domain_id,
        },
        create_share,
    ) {
        Ok(share_id) => share_id,
        Err(err) => return err,
    };

    run_vault_create_do_apply_vault_fields(
        &mut vault,
        VaultCreateDoApplyVaultFields {
            asset: facts.asset.clone(),
            tx_flags: facts.tx_flags,
            sequence: facts.sequence,
            owner: facts.owner_account.clone(),
            pseudo_id: setup.pseudo_id.clone(),
            zero_amount: facts.zero_amount,
            assets_maximum: facts.assets_maximum,
            share_mpt_id: share_mpt_id.clone(),
            data: facts.data,
            withdrawal_policy: facts.withdrawal_policy,
            scale: setup.scale,
        },
    );

    run_vault_create_do_apply_authorization_tail(
        facts.tx_flags,
        &share_mpt_id,
        &facts.owner_account,
        &setup.pseudo_id,
        &facts.asset,
        authorize,
        |asset| associate_asset(&mut vault, asset),
    )
}

#[cfg(test)]
mod tests {
    use std::{cell::Cell, rc::Rc};

    use protocol::{Ter, trans_token};

    use super::{VaultCreateDoApplyFacts, run_vault_create_do_apply};
    use crate::{
        MPT_CAN_ESCROW_FLAG, MPT_CAN_TRADE_FLAG, MPT_CAN_TRANSFER_FLAG, MPT_REQUIRE_AUTH_FLAG,
        VAULT_PRIVATE_FLAG, VaultCreateDoApplyAuthorizeRequest, VaultCreateDoApplyVaultFieldSink,
        VaultCreateShareIssuanceRequest,
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
}
