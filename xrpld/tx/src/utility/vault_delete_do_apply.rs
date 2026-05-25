//! Higher top-level the reference implementation shell.
//!
//! This ports the exact deterministic composition around:
//!
//! - the front vault-load plus empty-holding-removal shell,
//! - the middle share-issuance destruction shell,
//! - and the final pseudo-account plus vault destruction tail,
//!   returning the first failing `TER` unchanged.

use protocol::{Ter, is_tes_success};

use crate::{
    VaultDeleteDoApplyFrontVault, VaultDeleteDoApplyIssuanceVault,
    VaultDeleteDoApplyTailPseudoAccount, VaultDeleteDoApplyTailVault,
    load_vault_delete_do_apply_front, run_vault_delete_do_apply_issuance,
    run_vault_delete_do_apply_tail,
};

#[allow(clippy::too_many_arguments)]
pub fn run_vault_delete_do_apply<
    Vault,
    FrontPseudoAccount,
    Issuance,
    TailPseudoAccount,
    OwnerAccount,
    AccountId,
    Asset,
    ShareMptId,
    VaultKey,
    ReadVault,
    RemoveVaultAssetHolding,
    ReadFrontPseudoAccount,
    ReadIssuance,
    OwnerHasToken,
    RemoveOwnerShareHolding,
    DirRemoveIssuance,
    AdjustPseudoOwnerCount,
    EraseIssuance,
    PeekPseudoDir,
    ReadTailPseudoAccount,
    PseudoDirExistsAfter,
    ErasePseudoAccount,
    RemoveVaultFromOwnerDir,
    ReadOwnerAccount,
    AdjustOwnerCount,
    EraseVault,
>(
    read_vault: ReadVault,
    remove_vault_asset_holding: RemoveVaultAssetHolding,
    read_front_pseudo_account: ReadFrontPseudoAccount,
    read_issuance: ReadIssuance,
    owner_has_token: OwnerHasToken,
    remove_owner_share_holding: RemoveOwnerShareHolding,
    dir_remove_issuance: DirRemoveIssuance,
    adjust_pseudo_owner_count: AdjustPseudoOwnerCount,
    erase_issuance: EraseIssuance,
    peek_pseudo_dir: PeekPseudoDir,
    read_tail_pseudo_account: ReadTailPseudoAccount,
    pseudo_dir_exists_after: PseudoDirExistsAfter,
    erase_pseudo_account: ErasePseudoAccount,
    remove_vault_from_owner_dir: RemoveVaultFromOwnerDir,
    read_owner_account: ReadOwnerAccount,
    adjust_owner_count: AdjustOwnerCount,
    erase_vault: EraseVault,
) -> Ter
where
    Vault: VaultDeleteDoApplyFrontVault<AccountId = AccountId, Asset = Asset>
        + VaultDeleteDoApplyIssuanceVault<ShareMptId = ShareMptId>
        + VaultDeleteDoApplyTailVault<AccountId = AccountId, VaultKey = VaultKey>,
    AccountId: Clone,
    Asset: Clone,
    ShareMptId: Clone,
    TailPseudoAccount: VaultDeleteDoApplyTailPseudoAccount<VaultKey>,
    ReadVault: FnOnce() -> Option<Vault>,
    RemoveVaultAssetHolding: FnOnce(&AccountId, &Asset) -> Ter,
    ReadFrontPseudoAccount: FnOnce(&AccountId) -> Option<FrontPseudoAccount>,
    ReadIssuance: FnOnce(&ShareMptId) -> Option<Issuance>,
    OwnerHasToken: FnOnce(&ShareMptId) -> bool,
    RemoveOwnerShareHolding: FnOnce(&ShareMptId) -> Ter,
    DirRemoveIssuance: FnOnce(&Issuance) -> bool,
    AdjustPseudoOwnerCount: FnOnce(),
    EraseIssuance: FnOnce(Issuance),
    PeekPseudoDir: FnOnce(&AccountId) -> bool,
    ReadTailPseudoAccount: FnOnce(&AccountId) -> Option<TailPseudoAccount>,
    PseudoDirExistsAfter: FnOnce(&AccountId) -> bool,
    ErasePseudoAccount: FnOnce(TailPseudoAccount),
    RemoveVaultFromOwnerDir: FnOnce(&AccountId, &Vault) -> bool,
    ReadOwnerAccount: FnOnce(&AccountId) -> Option<OwnerAccount>,
    AdjustOwnerCount: FnOnce(OwnerAccount),
    EraseVault: FnOnce(),
{
    let front = match load_vault_delete_do_apply_front(
        read_vault,
        remove_vault_asset_holding,
        read_front_pseudo_account,
    ) {
        Ok(front) => front,
        Err(err) => return err,
    };

    let issuance_result = run_vault_delete_do_apply_issuance(
        &front.vault,
        read_issuance,
        owner_has_token,
        remove_owner_share_holding,
        dir_remove_issuance,
        adjust_pseudo_owner_count,
        erase_issuance,
    );
    if !is_tes_success(issuance_result) {
        return issuance_result;
    }

    run_vault_delete_do_apply_tail(
        &front.vault,
        peek_pseudo_dir,
        read_tail_pseudo_account,
        pseudo_dir_exists_after,
        erase_pseudo_account,
        remove_vault_from_owner_dir,
        read_owner_account,
        adjust_owner_count,
        erase_vault,
    )
}

#[cfg(test)]
mod tests {
    use std::{cell::Cell, rc::Rc};

    use protocol::{Ter, trans_token};

    use super::run_vault_delete_do_apply;
    use crate::{
        VaultDeleteDoApplyFrontVault, VaultDeleteDoApplyIssuanceVault,
        VaultDeleteDoApplyTailPseudoAccount, VaultDeleteDoApplyTailVault,
    };

    #[derive(Debug, Clone, PartialEq, Eq)]
    struct TestVault {
        pseudo_id: &'static str,
        asset: &'static str,
        share_mpt_id: &'static str,
        owner_id: &'static str,
        key: &'static str,
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

    impl VaultDeleteDoApplyIssuanceVault for TestVault {
        type ShareMptId = &'static str;

        fn share_mpt_id(&self) -> &Self::ShareMptId {
            &self.share_mpt_id
        }
    }

    impl VaultDeleteDoApplyTailVault for TestVault {
        type AccountId = &'static str;
        type VaultKey = &'static str;

        fn pseudo_id(&self) -> &Self::AccountId {
            &self.pseudo_id
        }

        fn owner_id(&self) -> &Self::AccountId {
            &self.owner_id
        }

        fn key(&self) -> &Self::VaultKey {
            &self.key
        }
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    struct TestPseudo {
        vault_key: &'static str,
        balance_is_zero: bool,
        owner_count_is_zero: bool,
    }

    impl VaultDeleteDoApplyTailPseudoAccount<&'static str> for TestPseudo {
        fn belongs_to_vault(&self, vault_key: &&'static str) -> bool {
            self.vault_key == *vault_key
        }

        fn balance_is_zero(&self) -> bool {
            self.balance_is_zero
        }

        fn owner_count_is_zero(&self) -> bool {
            self.owner_count_is_zero
        }
    }

    fn test_vault() -> TestVault {
        TestVault {
            pseudo_id: "vault-pseudo",
            asset: "USD",
            share_mpt_id: "share-mpt",
            owner_id: "vault-owner",
            key: "vault-key",
        }
    }

    fn tail_pseudo() -> TestPseudo {
        TestPseudo {
            vault_key: "vault-key",
            balance_is_zero: true,
            owner_count_is_zero: true,
        }
    }

    #[test]
    fn vault_delete_do_apply_runs_current_cpp_stage_order() {
        let steps = Rc::new(std::cell::RefCell::new(Vec::new()));

        let result = run_vault_delete_do_apply(
            || Some(test_vault()),
            {
                let steps = Rc::clone(&steps);
                move |pseudo_id, asset| {
                    steps
                        .borrow_mut()
                        .push(format!("front_remove:{pseudo_id}:{asset}"));
                    Ter::TES_SUCCESS
                }
            },
            {
                let steps = Rc::clone(&steps);
                move |pseudo_id| {
                    steps.borrow_mut().push(format!("front_pseudo:{pseudo_id}"));
                    Some("front-pseudo")
                }
            },
            {
                let steps = Rc::clone(&steps);
                move |share_mpt_id| {
                    steps
                        .borrow_mut()
                        .push(format!("issuance_read:{share_mpt_id}"));
                    Some("issuance")
                }
            },
            {
                let steps = Rc::clone(&steps);
                move |share_mpt_id| {
                    steps
                        .borrow_mut()
                        .push(format!("owner_token:{share_mpt_id}"));
                    true
                }
            },
            {
                let steps = Rc::clone(&steps);
                move |share_mpt_id| {
                    steps
                        .borrow_mut()
                        .push(format!("owner_remove:{share_mpt_id}"));
                    Ter::TES_SUCCESS
                }
            },
            {
                let steps = Rc::clone(&steps);
                move |issuance| {
                    steps
                        .borrow_mut()
                        .push(format!("issuance_dir_remove:{issuance}"));
                    true
                }
            },
            {
                let steps = Rc::clone(&steps);
                move || steps.borrow_mut().push("adjust_pseudo_owner".to_string())
            },
            {
                let steps = Rc::clone(&steps);
                move |issuance| {
                    steps
                        .borrow_mut()
                        .push(format!("erase_issuance:{issuance}"))
                }
            },
            {
                let steps = Rc::clone(&steps);
                move |pseudo_id| {
                    steps
                        .borrow_mut()
                        .push(format!("tail_peek_dir:{pseudo_id}"));
                    false
                }
            },
            {
                let steps = Rc::clone(&steps);
                move |pseudo_id| {
                    steps
                        .borrow_mut()
                        .push(format!("tail_read_pseudo:{pseudo_id}"));
                    Some(tail_pseudo())
                }
            },
            {
                let steps = Rc::clone(&steps);
                move |pseudo_id| {
                    steps
                        .borrow_mut()
                        .push(format!("tail_exists_dir:{pseudo_id}"));
                    false
                }
            },
            {
                let steps = Rc::clone(&steps);
                move |_| steps.borrow_mut().push("erase_pseudo".to_string())
            },
            {
                let steps = Rc::clone(&steps);
                move |owner_id, vault: &TestVault| {
                    steps
                        .borrow_mut()
                        .push(format!("remove_vault_dir:{owner_id}:{}", vault.key()));
                    true
                }
            },
            {
                let steps = Rc::clone(&steps);
                move |owner_id| {
                    steps.borrow_mut().push(format!("read_owner:{owner_id}"));
                    Some("owner")
                }
            },
            {
                let steps = Rc::clone(&steps);
                move |owner| steps.borrow_mut().push(format!("adjust_owner:{owner}"))
            },
            {
                let steps = Rc::clone(&steps);
                move || steps.borrow_mut().push("erase_vault".to_string())
            },
        );

        assert_eq!(result, Ter::TES_SUCCESS);
        assert_eq!(
            steps.borrow().as_slice(),
            [
                "front_remove:vault-pseudo:USD",
                "front_pseudo:vault-pseudo",
                "issuance_read:share-mpt",
                "owner_token:share-mpt",
                "owner_remove:share-mpt",
                "issuance_dir_remove:issuance",
                "adjust_pseudo_owner",
                "erase_issuance:issuance",
                "tail_peek_dir:vault-pseudo",
                "tail_read_pseudo:vault-pseudo",
                "tail_exists_dir:vault-pseudo",
                "erase_pseudo",
                "remove_vault_dir:vault-owner:vault-key",
                "read_owner:vault-owner",
                "adjust_owner:owner",
                "erase_vault",
            ]
        );
    }

    #[test]
    fn vault_delete_do_apply_returns_front_failure_before_later_stages() {
        let issuance_called = Cell::new(false);

        let result = run_vault_delete_do_apply(
            || Some(test_vault()),
            |_, _| Ter::TEC_INTERNAL,
            |_| Some("front-pseudo"),
            |_| {
                issuance_called.set(true);
                Some("issuance")
            },
            |_| false,
            |_| Ter::TES_SUCCESS,
            |_| true,
            || {},
            |_| {},
            |_| false,
            |_| Some(tail_pseudo()),
            |_| false,
            |_| {},
            |_, _| true,
            |_| Some("owner"),
            |_| {},
            || {},
        );

        assert_eq!(result, Ter::TEC_INTERNAL);
        assert_eq!(trans_token(result), "tecINTERNAL");
        assert!(!issuance_called.get());
    }

    #[test]
    fn vault_delete_do_apply_returns_issuance_failure_before_tail() {
        let tail_called = Cell::new(false);

        let result = run_vault_delete_do_apply(
            || Some(test_vault()),
            |_, _| Ter::TES_SUCCESS,
            |_| Some("front-pseudo"),
            |_| Some("issuance"),
            |_| true,
            |_| Ter::TEC_INTERNAL,
            |_| true,
            || {},
            |_| {},
            |_| {
                tail_called.set(true);
                false
            },
            |_| Some(tail_pseudo()),
            |_| false,
            |_| {},
            |_, _| true,
            |_| Some("owner"),
            |_| {},
            || {},
        );

        assert_eq!(result, Ter::TEC_INTERNAL);
        assert_eq!(trans_token(result), "tecINTERNAL");
        assert!(!tail_called.get());
    }
}
