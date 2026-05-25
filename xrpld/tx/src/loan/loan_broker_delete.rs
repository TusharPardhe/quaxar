//! Deterministic
//! the reference implementation metadata, `preflight(...)`, and `preclaim(...)`
//! shells.
//!
//! This ports the current top-level branch ordering around:
//!
//! - the shared lending dependency gate,
//! - zero broker-id rejection in `preflight(...)`,
//! - missing-broker and wrong-owner rejection in `preclaim(...)`,
//! - owner-count and rounded-debt obligation checks,
//! - the impossible missing-vault fallback to `tefBAD_LEDGER`,
//! - and the conditional broker-owner deep-freeze check when cover exists.
//!
//! The current `doApply()` seam ports the exact
//! broker/vault load, directory-removal, payout, cleanup, and owner-count
//! ordering while leaving the concrete ledger objects and account mutation
//! helpers explicit.

use protocol::{NotTec, Ter, is_tes_success};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct LoanBrokerDeletePreclaimFacts {
    pub broker_exists: bool,
    pub submitter_is_broker_owner: bool,
    pub owner_count_is_zero: bool,
    pub vault_exists: bool,
    pub rounded_debt_total_is_zero: bool,
    pub cover_available_is_positive: bool,
    pub deep_frozen_result: Ter,
}

pub fn run_loan_broker_delete_preflight(loan_broker_id_is_zero: bool) -> NotTec {
    if loan_broker_id_is_zero {
        return Ter::TEM_INVALID;
    }

    Ter::TES_SUCCESS
}

pub fn run_loan_broker_delete_preclaim(facts: LoanBrokerDeletePreclaimFacts) -> Ter {
    if !facts.broker_exists {
        return Ter::TEC_NO_ENTRY;
    }

    if !facts.submitter_is_broker_owner {
        return Ter::TEC_NO_PERMISSION;
    }

    if !facts.owner_count_is_zero {
        return Ter::TEC_HAS_OBLIGATIONS;
    }

    if !facts.vault_exists {
        return Ter::TEF_BAD_LEDGER;
    }

    if !facts.rounded_debt_total_is_zero {
        return Ter::TEC_HAS_OBLIGATIONS;
    }

    if facts.cover_available_is_positive && !is_tes_success(facts.deep_frozen_result) {
        return facts.deep_frozen_result;
    }

    Ter::TES_SUCCESS
}

pub trait LoanBrokerDeleteDoApplyBroker {
    type AccountId;
    type VaultId;
    type DirNode;
    type BrokerKey;
    type Amount;

    fn pseudo_account_id(&self) -> &Self::AccountId;
    fn vault_id(&self) -> &Self::VaultId;
    fn owner_node(&self) -> &Self::DirNode;
    fn vault_node(&self) -> &Self::DirNode;
    fn key(&self) -> &Self::BrokerKey;
    fn cover_available(&self) -> &Self::Amount;
}

pub trait LoanBrokerDeleteDoApplyVault {
    type AccountId;
    type Asset;

    fn pseudo_id(&self) -> &Self::AccountId;
    fn asset(&self) -> &Self::Asset;
}

pub trait LoanBrokerDeleteDoApplyPseudoAccount {
    type Amount;
    type OwnerCount;

    fn balance(&self) -> &Self::Amount;
    fn owner_count(&self) -> &Self::OwnerCount;
}

#[allow(clippy::too_many_arguments)]
pub fn run_loan_broker_delete_do_apply<
    Broker,
    Vault,
    PseudoAccount,
    OwnerAccount,
    BrokerId,
    AccountId,
    VaultId,
    DirNode,
    BrokerKey,
    Amount,
    Asset,
    OwnerCount,
    ReadBroker,
    ReadVault,
    RemoveOwnerDirEntry,
    RemoveVaultDirEntry,
    AccountSend,
    RemoveEmptyHolding,
    ReadPseudoAccount,
    ReadOwnerAccount,
    ReadPseudoDirectory,
    ErasePseudoAccount,
    EraseBroker,
    AdjustOwnerCount,
    AssociateAsset,
>(
    broker_id: &BrokerId,
    account: &AccountId,
    read_broker: ReadBroker,
    read_vault: ReadVault,
    remove_owner_dir_entry: RemoveOwnerDirEntry,
    remove_vault_dir_entry: RemoveVaultDirEntry,
    account_send: AccountSend,
    remove_empty_holding: RemoveEmptyHolding,
    read_pseudo_account: ReadPseudoAccount,
    read_owner_account: ReadOwnerAccount,
    read_pseudo_directory: ReadPseudoDirectory,
    erase_pseudo_account: ErasePseudoAccount,
    erase_broker: EraseBroker,
    adjust_owner_count: AdjustOwnerCount,
    associate_asset: AssociateAsset,
) -> Ter
where
    Broker: LoanBrokerDeleteDoApplyBroker<
            AccountId = AccountId,
            VaultId = VaultId,
            DirNode = DirNode,
            BrokerKey = BrokerKey,
            Amount = Amount,
        > + Clone,
    Vault: LoanBrokerDeleteDoApplyVault<AccountId = AccountId, Asset = Asset>,
    PseudoAccount: LoanBrokerDeleteDoApplyPseudoAccount<Amount = Amount, OwnerCount = OwnerCount>,
    ReadBroker: FnOnce(&BrokerId) -> Option<Broker>,
    ReadVault: FnOnce(&VaultId) -> Option<Vault>,
    RemoveOwnerDirEntry: FnOnce(&AccountId, &DirNode, &BrokerKey) -> bool,
    RemoveVaultDirEntry: FnOnce(&AccountId, &DirNode, &BrokerKey) -> bool,
    AccountSend: FnOnce(&AccountId, &AccountId, &Amount) -> Ter,
    RemoveEmptyHolding: FnOnce(&AccountId, &Asset) -> Ter,
    ReadPseudoAccount: FnOnce(&AccountId) -> Option<PseudoAccount>,
    ReadOwnerAccount: FnOnce(&AccountId) -> Option<OwnerAccount>,
    ReadPseudoDirectory: FnOnce(&AccountId) -> bool,
    ErasePseudoAccount: FnOnce(PseudoAccount),
    EraseBroker: FnOnce(Broker),
    AdjustOwnerCount: FnOnce(&mut OwnerAccount, i32),
    AssociateAsset: FnOnce(&Broker, &Asset),
    Amount: Default + PartialEq,
    OwnerCount: Default + PartialEq,
{
    let broker = match read_broker(broker_id) {
        Some(broker) => broker,
        None => return Ter::TEF_BAD_LEDGER,
    };

    let vault = match read_vault(broker.vault_id()) {
        Some(vault) => vault,
        None => return Ter::TEF_BAD_LEDGER,
    };

    let broker_pseudo_id = broker.pseudo_account_id();
    let vault_pseudo_id = vault.pseudo_id();
    let vault_asset = vault.asset();

    if !remove_owner_dir_entry(account, broker.owner_node(), broker.key()) {
        return Ter::TEF_BAD_LEDGER;
    }
    if !remove_vault_dir_entry(vault_pseudo_id, broker.vault_node(), broker.key()) {
        return Ter::TEF_BAD_LEDGER;
    }

    let cover_available = broker.cover_available();
    let payout = account_send(broker_pseudo_id, account, cover_available);
    if !is_tes_success(payout) {
        return payout;
    }

    let empty_holding = remove_empty_holding(broker_pseudo_id, vault_asset);
    if !is_tes_success(empty_holding) {
        return empty_holding;
    }

    let pseudo_account = match read_pseudo_account(broker_pseudo_id) {
        Some(pseudo_account) => pseudo_account,
        None => return Ter::TEF_BAD_LEDGER,
    };

    if *pseudo_account.balance() != Amount::default() {
        return Ter::TEC_HAS_OBLIGATIONS;
    }
    if *pseudo_account.owner_count() != OwnerCount::default() {
        return Ter::TEC_HAS_OBLIGATIONS;
    }
    if read_pseudo_directory(broker_pseudo_id) {
        return Ter::TEC_HAS_OBLIGATIONS;
    }

    let broker_for_assoc = broker.clone();

    erase_pseudo_account(pseudo_account);
    erase_broker(broker);

    let mut owner = match read_owner_account(account) {
        Some(owner) => owner,
        None => return Ter::TEF_BAD_LEDGER,
    };
    adjust_owner_count(&mut owner, -2);

    associate_asset(&broker_for_assoc, vault_asset);

    Ter::TES_SUCCESS
}

#[cfg(test)]
mod tests {
    use std::rc::Rc;

    use protocol::{Ter, trans_token};

    use super::{
        LoanBrokerDeleteDoApplyBroker, LoanBrokerDeleteDoApplyPseudoAccount,
        LoanBrokerDeleteDoApplyVault, LoanBrokerDeletePreclaimFacts,
        run_loan_broker_delete_do_apply, run_loan_broker_delete_preclaim,
        run_loan_broker_delete_preflight,
    };

    fn base() -> LoanBrokerDeletePreclaimFacts {
        LoanBrokerDeletePreclaimFacts {
            broker_exists: true,
            submitter_is_broker_owner: true,
            owner_count_is_zero: true,
            vault_exists: true,
            rounded_debt_total_is_zero: true,
            cover_available_is_positive: false,
            deep_frozen_result: Ter::TES_SUCCESS,
        }
    }

    #[test]
    fn loan_broker_delete_preflight_rejects_zero_broker_id() {
        assert_eq!(run_loan_broker_delete_preflight(true), Ter::TEM_INVALID);
    }

    #[test]
    fn loan_broker_delete_preclaim_rejects_missing_broker() {
        let result = run_loan_broker_delete_preclaim(LoanBrokerDeletePreclaimFacts::default());

        assert_eq!(result, Ter::TEC_NO_ENTRY);
        assert_eq!(trans_token(result), "tecNO_ENTRY");
    }

    #[test]
    fn loan_broker_delete_preclaim_rejects_wrong_owner() {
        let result = run_loan_broker_delete_preclaim(LoanBrokerDeletePreclaimFacts {
            broker_exists: true,
            ..LoanBrokerDeletePreclaimFacts::default()
        });

        assert_eq!(result, Ter::TEC_NO_PERMISSION);
    }

    #[test]
    fn loan_broker_delete_preclaim_rejects_existing_obligations() {
        let result = run_loan_broker_delete_preclaim(LoanBrokerDeletePreclaimFacts {
            owner_count_is_zero: false,
            ..base()
        });

        assert_eq!(result, Ter::TEC_HAS_OBLIGATIONS);
    }

    #[test]
    fn loan_broker_delete_preclaim_maps_missing_vault_to_bad_ledger() {
        let result = run_loan_broker_delete_preclaim(LoanBrokerDeletePreclaimFacts {
            vault_exists: false,
            ..base()
        });

        assert_eq!(result, Ter::TEF_BAD_LEDGER);
    }

    #[test]
    fn loan_broker_delete_preclaim_rejects_nonzero_rounded_debt() {
        let result = run_loan_broker_delete_preclaim(LoanBrokerDeletePreclaimFacts {
            rounded_debt_total_is_zero: false,
            ..base()
        });

        assert_eq!(result, Ter::TEC_HAS_OBLIGATIONS);
    }

    #[test]
    fn loan_broker_delete_preclaim_returns_deep_freeze_failure_when_cover_exists() {
        let result = run_loan_broker_delete_preclaim(LoanBrokerDeletePreclaimFacts {
            cover_available_is_positive: true,
            deep_frozen_result: Ter::TEC_FROZEN,
            ..base()
        });

        assert_eq!(result, Ter::TEC_FROZEN);
    }

    #[test]
    fn loan_broker_delete_preclaim_accepts_empty_owner_broker() {
        assert_eq!(run_loan_broker_delete_preclaim(base()), Ter::TES_SUCCESS);
    }

    #[derive(Clone)]
    struct TestBroker {
        pseudo_account_id: &'static str,
        vault_id: &'static str,
        owner_node: u64,
        vault_node: u64,
        key: &'static str,
        cover_available: i64,
    }

    impl LoanBrokerDeleteDoApplyBroker for TestBroker {
        type AccountId = &'static str;
        type VaultId = &'static str;
        type DirNode = u64;
        type BrokerKey = &'static str;
        type Amount = i64;

        fn pseudo_account_id(&self) -> &Self::AccountId {
            &self.pseudo_account_id
        }

        fn vault_id(&self) -> &Self::VaultId {
            &self.vault_id
        }

        fn owner_node(&self) -> &Self::DirNode {
            &self.owner_node
        }

        fn vault_node(&self) -> &Self::DirNode {
            &self.vault_node
        }

        fn key(&self) -> &Self::BrokerKey {
            &self.key
        }

        fn cover_available(&self) -> &Self::Amount {
            &self.cover_available
        }
    }

    #[derive(Clone)]
    struct TestVault {
        pseudo_id: &'static str,
        asset: &'static str,
    }

    impl LoanBrokerDeleteDoApplyVault for TestVault {
        type AccountId = &'static str;
        type Asset = &'static str;

        fn pseudo_id(&self) -> &Self::AccountId {
            &self.pseudo_id
        }

        fn asset(&self) -> &Self::Asset {
            &self.asset
        }
    }

    #[derive(Clone)]
    struct TestPseudoAccount {
        balance: i64,
        owner_count: u32,
    }

    impl LoanBrokerDeleteDoApplyPseudoAccount for TestPseudoAccount {
        type Amount = i64;
        type OwnerCount = u32;

        fn balance(&self) -> &Self::Amount {
            &self.balance
        }

        fn owner_count(&self) -> &Self::OwnerCount {
            &self.owner_count
        }
    }

    fn broker() -> TestBroker {
        TestBroker {
            pseudo_account_id: "broker-pseudo",
            vault_id: "vault-1",
            owner_node: 7,
            vault_node: 9,
            key: "broker-key",
            cover_available: 42,
        }
    }

    fn vault() -> TestVault {
        TestVault {
            pseudo_id: "vault-pseudo",
            asset: "USD",
        }
    }

    #[test]
    fn loan_broker_delete_do_apply_runs_current() {
        let steps = Rc::new(std::cell::RefCell::new(Vec::new()));

        let result = run_loan_broker_delete_do_apply(
            &"broker-1",
            &"account-1",
            |_| Some(broker()),
            |_| Some(vault()),
            {
                let steps = Rc::clone(&steps);
                move |account, node, key| {
                    steps
                        .borrow_mut()
                        .push(format!("remove_owner_dir:{account}:{node}:{key}"));
                    true
                }
            },
            {
                let steps = Rc::clone(&steps);
                move |account, node, key| {
                    steps
                        .borrow_mut()
                        .push(format!("remove_vault_dir:{account}:{node}:{key}"));
                    true
                }
            },
            {
                let steps = Rc::clone(&steps);
                move |pseudo, account, amount| {
                    steps
                        .borrow_mut()
                        .push(format!("account_send:{pseudo}:{account}:{amount}"));
                    Ter::TES_SUCCESS
                }
            },
            {
                let steps = Rc::clone(&steps);
                move |pseudo, asset| {
                    steps
                        .borrow_mut()
                        .push(format!("remove_empty_holding:{pseudo}:{asset}"));
                    Ter::TES_SUCCESS
                }
            },
            {
                let steps = Rc::clone(&steps);
                move |pseudo| {
                    steps.borrow_mut().push(format!("read_pseudo:{pseudo}"));
                    Some(TestPseudoAccount {
                        balance: 0,
                        owner_count: 0,
                    })
                }
            },
            {
                let steps = Rc::clone(&steps);
                move |account| {
                    steps.borrow_mut().push(format!("read_owner:{account}"));
                    Some("owner-account")
                }
            },
            {
                let steps = Rc::clone(&steps);
                move |pseudo| {
                    steps.borrow_mut().push(format!("read_directory:{pseudo}"));
                    false
                }
            },
            {
                let steps = Rc::clone(&steps);
                move |_| steps.borrow_mut().push("erase_pseudo".to_string())
            },
            {
                let steps = Rc::clone(&steps);
                move |_| steps.borrow_mut().push("erase_broker".to_string())
            },
            {
                let steps = Rc::clone(&steps);
                move |owner, delta| {
                    steps
                        .borrow_mut()
                        .push(format!("adjust_owner:{owner}:{delta}"))
                }
            },
            {
                let steps = Rc::clone(&steps);
                move |_, asset| steps.borrow_mut().push(format!("associate_asset:{asset}"))
            },
        );

        assert_eq!(result, Ter::TES_SUCCESS);
        assert_eq!(
            steps.borrow().as_slice(),
            [
                "remove_owner_dir:account-1:7:broker-key",
                "remove_vault_dir:vault-pseudo:9:broker-key",
                "account_send:broker-pseudo:account-1:42",
                "remove_empty_holding:broker-pseudo:USD",
                "read_pseudo:broker-pseudo",
                "read_directory:broker-pseudo",
                "erase_pseudo",
                "erase_broker",
                "read_owner:account-1",
                "adjust_owner:owner-account:-2",
                "associate_asset:USD",
            ]
        );
    }

    #[test]
    fn loan_broker_delete_do_apply_returns_first_failure() {
        let missing_broker = run_loan_broker_delete_do_apply(
            &"broker-1",
            &"account-1",
            |_| None::<TestBroker>,
            |_| None::<TestVault>,
            |_, _, _| true,
            |_, _, _| true,
            |_, _, _| Ter::TES_SUCCESS,
            |_, _| Ter::TES_SUCCESS,
            |_| None,
            |_| Some("owner"),
            |_| false,
            |_: TestPseudoAccount| {},
            |_: TestBroker| {},
            |_, _| {},
            |_, _| {},
        );
        assert_eq!(missing_broker, Ter::TEF_BAD_LEDGER);
    }
}
