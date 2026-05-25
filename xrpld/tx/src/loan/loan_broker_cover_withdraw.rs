//! Deterministic
//! the reference implementation metadata, `preflight(...)`, `preclaim(...)`,
//! and `doApply()` shell.
//!
//! This ports the current top-level control flow around:
//!
//! - the zero broker-id malformed guard,
//! - positive-amount and legal-net malformed guards,
//! - the optional zero-destination malformed guard,
//! - pseudo-account destination rejection,
//! - missing-broker and wrong-owner rejection,
//! - the impossible missing-vault fallback to `tefBAD_LEDGER`,
//! - the vault-asset match check,
//! - transferability, optional third-party `canWithdraw(...)`, and auth checks,
//! - conditional freeze checks outside the issuer-short-circuit path,
//! - the two cover-availability plus pseudo-balance insufficiency checks,
//! - and the `doApply()` load, decrement, update, asset-association, and
//!   delegated withdraw ordering.

use protocol::{NotTec, Ter, is_tes_success};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct LoanBrokerCoverWithdrawPreflightFacts {
    pub loan_broker_id_is_zero: bool,
    pub amount_is_positive: bool,
    pub amount_is_legal_net: bool,
    pub destination_is_present: bool,
    pub destination_is_zero: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct LoanBrokerCoverWithdrawPreclaimFacts {
    pub destination_is_pseudo_account: bool,
    pub broker_exists: bool,
    pub submitter_is_broker_owner: bool,
    pub vault_exists: bool,
    pub amount_matches_vault_asset: bool,
    pub destination_is_submitter: bool,
    pub destination_is_vault_asset_issuer: bool,
    pub cover_available_at_least_amount: bool,
    pub cover_after_withdraw_at_least_minimum: bool,
    pub pseudo_balance_at_least_amount: bool,
    pub can_transfer_result: Ter,
    pub can_withdraw_result: Ter,
    pub require_auth_result: Ter,
    pub source_frozen_result: Ter,
    pub destination_deep_frozen_result: Ter,
}

pub trait LoanBrokerCoverWithdrawDoApplyBroker {
    type AccountId;
    type Amount;
    type Asset;
    type VaultId;

    fn vault_id(&self) -> &Self::VaultId;
    fn pseudo_account_id(&self) -> &Self::AccountId;
    fn subtract_cover_available(&mut self, amount: &Self::Amount);
}

pub trait LoanBrokerCoverWithdrawDoApplyVault {
    type Asset;

    fn asset(&self) -> &Self::Asset;
}

pub trait LoanBrokerCoverWithdrawDoApplySink {
    type Broker: LoanBrokerCoverWithdrawDoApplyBroker<
            AccountId = Self::AccountId,
            Amount = Self::Amount,
            Asset = Self::Asset,
            VaultId = Self::VaultId,
        >;
    type Vault: LoanBrokerCoverWithdrawDoApplyVault<Asset = Self::Asset>;
    type AccountId;
    type Amount;
    type Asset;
    type VaultId;

    fn read_broker(&mut self) -> Option<Self::Broker>;
    fn read_vault(&mut self, vault_id: &Self::VaultId) -> Option<Self::Vault>;
    fn update_broker(&mut self, broker: &Self::Broker);
    fn associate_asset(&mut self, broker: &Self::Broker, asset: &Self::Asset);
    fn do_withdraw(
        &mut self,
        destination: &Self::AccountId,
        pseudo_account_id: &Self::AccountId,
        pre_fee_balance: &Self::Amount,
        amount: &Self::Amount,
    ) -> Ter;
}

pub fn run_loan_broker_cover_withdraw_preflight(
    facts: LoanBrokerCoverWithdrawPreflightFacts,
) -> NotTec {
    if facts.loan_broker_id_is_zero {
        return Ter::TEM_INVALID;
    }

    if !facts.amount_is_positive {
        return Ter::TEM_BAD_AMOUNT;
    }

    if !facts.amount_is_legal_net {
        return Ter::TEM_BAD_AMOUNT;
    }

    if facts.destination_is_present && facts.destination_is_zero {
        return Ter::TEM_MALFORMED;
    }

    Ter::TES_SUCCESS
}

pub fn run_loan_broker_cover_withdraw_preclaim(facts: LoanBrokerCoverWithdrawPreclaimFacts) -> Ter {
    if facts.destination_is_pseudo_account {
        return Ter::TEC_PSEUDO_ACCOUNT;
    }

    if !facts.broker_exists {
        return Ter::TEC_NO_ENTRY;
    }

    if !facts.submitter_is_broker_owner {
        return Ter::TEC_NO_PERMISSION;
    }

    if !facts.vault_exists {
        return Ter::TEF_BAD_LEDGER;
    }

    if !facts.amount_matches_vault_asset {
        return Ter::TEC_WRONG_ASSET;
    }

    if !is_tes_success(facts.can_transfer_result) {
        return facts.can_transfer_result;
    }

    if !facts.destination_is_submitter && !is_tes_success(facts.can_withdraw_result) {
        return facts.can_withdraw_result;
    }

    if !is_tes_success(facts.require_auth_result) {
        return facts.require_auth_result;
    }

    if !facts.destination_is_vault_asset_issuer {
        if !is_tes_success(facts.source_frozen_result) {
            return facts.source_frozen_result;
        }

        if !is_tes_success(facts.destination_deep_frozen_result) {
            return facts.destination_deep_frozen_result;
        }
    }

    if !facts.cover_available_at_least_amount {
        return Ter::TEC_INSUFFICIENT_FUNDS;
    }

    if !facts.cover_after_withdraw_at_least_minimum {
        return Ter::TEC_INSUFFICIENT_FUNDS;
    }

    if !facts.pseudo_balance_at_least_amount {
        return Ter::TEC_INSUFFICIENT_FUNDS;
    }

    Ter::TES_SUCCESS
}

pub fn run_loan_broker_cover_withdraw_do_apply<Sink>(
    sink: &mut Sink,
    destination: &Sink::AccountId,
    pre_fee_balance: &Sink::Amount,
    amount: &Sink::Amount,
) -> Ter
where
    Sink: LoanBrokerCoverWithdrawDoApplySink,
    Sink::AccountId: Clone,
    Sink::Asset: Clone,
    Sink::VaultId: Clone,
{
    let mut broker = match sink.read_broker() {
        Some(broker) => broker,
        None => return Ter::TEC_INTERNAL,
    };

    let vault_id = broker.vault_id().clone();
    let vault = match sink.read_vault(&vault_id) {
        Some(vault) => vault,
        None => return Ter::TEC_INTERNAL,
    };

    let vault_asset = vault.asset().clone();
    let pseudo_account_id = broker.pseudo_account_id().clone();

    broker.subtract_cover_available(amount);
    sink.update_broker(&broker);
    sink.associate_asset(&broker, &vault_asset);

    sink.do_withdraw(destination, &pseudo_account_id, pre_fee_balance, amount)
}

#[cfg(test)]
mod tests {
    use std::{cell::RefCell, rc::Rc};

    use protocol::{Ter, trans_token};

    use super::{
        LoanBrokerCoverWithdrawDoApplyBroker, LoanBrokerCoverWithdrawDoApplySink,
        LoanBrokerCoverWithdrawDoApplyVault, LoanBrokerCoverWithdrawPreclaimFacts,
        LoanBrokerCoverWithdrawPreflightFacts, run_loan_broker_cover_withdraw_do_apply,
        run_loan_broker_cover_withdraw_preclaim, run_loan_broker_cover_withdraw_preflight,
    };

    fn base() -> LoanBrokerCoverWithdrawPreclaimFacts {
        LoanBrokerCoverWithdrawPreclaimFacts {
            destination_is_pseudo_account: false,
            broker_exists: true,
            submitter_is_broker_owner: true,
            vault_exists: true,
            amount_matches_vault_asset: true,
            destination_is_submitter: true,
            destination_is_vault_asset_issuer: false,
            cover_available_at_least_amount: true,
            cover_after_withdraw_at_least_minimum: true,
            pseudo_balance_at_least_amount: true,
            can_transfer_result: Ter::TES_SUCCESS,
            can_withdraw_result: Ter::TES_SUCCESS,
            require_auth_result: Ter::TES_SUCCESS,
            source_frozen_result: Ter::TES_SUCCESS,
            destination_deep_frozen_result: Ter::TES_SUCCESS,
        }
    }

    #[test]
    fn loan_broker_cover_withdraw_preflight_rejects_zero_broker_id() {
        let result =
            run_loan_broker_cover_withdraw_preflight(LoanBrokerCoverWithdrawPreflightFacts {
                loan_broker_id_is_zero: true,
                amount_is_positive: true,
                amount_is_legal_net: true,
                destination_is_present: false,
                destination_is_zero: false,
            });

        assert_eq!(result, Ter::TEM_INVALID);
    }

    #[test]
    fn loan_broker_cover_withdraw_preflight_rejects_bad_amounts() {
        let non_positive =
            run_loan_broker_cover_withdraw_preflight(LoanBrokerCoverWithdrawPreflightFacts {
                loan_broker_id_is_zero: false,
                amount_is_positive: false,
                amount_is_legal_net: true,
                destination_is_present: false,
                destination_is_zero: false,
            });
        let illegal =
            run_loan_broker_cover_withdraw_preflight(LoanBrokerCoverWithdrawPreflightFacts {
                loan_broker_id_is_zero: false,
                amount_is_positive: true,
                amount_is_legal_net: false,
                destination_is_present: false,
                destination_is_zero: false,
            });

        assert_eq!(non_positive, Ter::TEM_BAD_AMOUNT);
        assert_eq!(illegal, Ter::TEM_BAD_AMOUNT);
    }

    #[test]
    fn loan_broker_cover_withdraw_preflight_rejects_zero_destination() {
        let result =
            run_loan_broker_cover_withdraw_preflight(LoanBrokerCoverWithdrawPreflightFacts {
                loan_broker_id_is_zero: false,
                amount_is_positive: true,
                amount_is_legal_net: true,
                destination_is_present: true,
                destination_is_zero: true,
            });

        assert_eq!(result, Ter::TEM_MALFORMED);
        assert_eq!(trans_token(result), "temMALFORMED");
    }

    #[test]
    fn loan_broker_cover_withdraw_preclaim_rejects_pseudo_destination() {
        assert_eq!(
            run_loan_broker_cover_withdraw_preclaim(LoanBrokerCoverWithdrawPreclaimFacts {
                destination_is_pseudo_account: true,
                ..base()
            }),
            Ter::TEC_PSEUDO_ACCOUNT
        );
    }

    #[test]
    fn loan_broker_cover_withdraw_preclaim_rejects_missing_broker_and_wrong_owner() {
        let missing =
            run_loan_broker_cover_withdraw_preclaim(LoanBrokerCoverWithdrawPreclaimFacts {
                broker_exists: false,
                ..base()
            });
        let wrong_owner =
            run_loan_broker_cover_withdraw_preclaim(LoanBrokerCoverWithdrawPreclaimFacts {
                submitter_is_broker_owner: false,
                ..base()
            });

        assert_eq!(missing, Ter::TEC_NO_ENTRY);
        assert_eq!(wrong_owner, Ter::TEC_NO_PERMISSION);
    }

    #[test]
    fn loan_broker_cover_withdraw_preclaim_maps_missing_vault_and_wrong_asset() {
        let missing_vault =
            run_loan_broker_cover_withdraw_preclaim(LoanBrokerCoverWithdrawPreclaimFacts {
                vault_exists: false,
                ..base()
            });
        let wrong_asset =
            run_loan_broker_cover_withdraw_preclaim(LoanBrokerCoverWithdrawPreclaimFacts {
                amount_matches_vault_asset: false,
                ..base()
            });

        assert_eq!(missing_vault, Ter::TEF_BAD_LEDGER);
        assert_eq!(wrong_asset, Ter::TEC_WRONG_ASSET);
    }

    #[test]
    fn loan_broker_cover_withdraw_preclaim_returns_transfer_and_withdraw_failures_in_order() {
        let transfer =
            run_loan_broker_cover_withdraw_preclaim(LoanBrokerCoverWithdrawPreclaimFacts {
                can_transfer_result: Ter::TER_NO_RIPPLE,
                ..base()
            });
        let withdraw =
            run_loan_broker_cover_withdraw_preclaim(LoanBrokerCoverWithdrawPreclaimFacts {
                destination_is_submitter: false,
                can_withdraw_result: Ter::TEC_NO_PERMISSION,
                ..base()
            });

        assert_eq!(transfer, Ter::TER_NO_RIPPLE);
        assert_eq!(withdraw, Ter::TEC_NO_PERMISSION);
    }

    #[test]
    fn loan_broker_cover_withdraw_preclaim_skips_third_party_withdraw_check_for_self() {
        let result =
            run_loan_broker_cover_withdraw_preclaim(LoanBrokerCoverWithdrawPreclaimFacts {
                destination_is_submitter: true,
                can_withdraw_result: Ter::TEC_NO_PERMISSION,
                ..base()
            });

        assert_eq!(result, Ter::TES_SUCCESS);
    }

    #[test]
    fn loan_broker_cover_withdraw_preclaim_returns_auth_and_freeze_failures() {
        let auth = run_loan_broker_cover_withdraw_preclaim(LoanBrokerCoverWithdrawPreclaimFacts {
            require_auth_result: Ter::TER_NO_AUTH,
            ..base()
        });
        let frozen =
            run_loan_broker_cover_withdraw_preclaim(LoanBrokerCoverWithdrawPreclaimFacts {
                source_frozen_result: Ter::TEC_FROZEN,
                ..base()
            });
        let deep_frozen =
            run_loan_broker_cover_withdraw_preclaim(LoanBrokerCoverWithdrawPreclaimFacts {
                destination_deep_frozen_result: Ter::TEC_FROZEN,
                ..base()
            });

        assert_eq!(auth, Ter::TER_NO_AUTH);
        assert_eq!(frozen, Ter::TEC_FROZEN);
        assert_eq!(deep_frozen, Ter::TEC_FROZEN);
    }

    #[test]
    fn loan_broker_cover_withdraw_preclaim_skips_freeze_checks_when_sending_to_issuer() {
        let result =
            run_loan_broker_cover_withdraw_preclaim(LoanBrokerCoverWithdrawPreclaimFacts {
                destination_is_vault_asset_issuer: true,
                source_frozen_result: Ter::TEC_FROZEN,
                destination_deep_frozen_result: Ter::TEC_FROZEN,
                ..base()
            });

        assert_eq!(result, Ter::TES_SUCCESS);
    }

    #[test]
    fn loan_broker_cover_withdraw_preclaim_enforces_cover_and_balance_floors() {
        let cover = run_loan_broker_cover_withdraw_preclaim(LoanBrokerCoverWithdrawPreclaimFacts {
            cover_available_at_least_amount: false,
            ..base()
        });
        let minimum =
            run_loan_broker_cover_withdraw_preclaim(LoanBrokerCoverWithdrawPreclaimFacts {
                cover_after_withdraw_at_least_minimum: false,
                ..base()
            });
        let balance =
            run_loan_broker_cover_withdraw_preclaim(LoanBrokerCoverWithdrawPreclaimFacts {
                pseudo_balance_at_least_amount: false,
                ..base()
            });

        assert_eq!(cover, Ter::TEC_INSUFFICIENT_FUNDS);
        assert_eq!(minimum, Ter::TEC_INSUFFICIENT_FUNDS);
        assert_eq!(balance, Ter::TEC_INSUFFICIENT_FUNDS);
    }

    #[derive(Clone, Debug, PartialEq, Eq)]
    struct TestBroker {
        vault_id: &'static str,
        pseudo_account_id: &'static str,
        cover_available: i64,
        steps: Rc<RefCell<Vec<String>>>,
    }

    impl LoanBrokerCoverWithdrawDoApplyBroker for TestBroker {
        type AccountId = &'static str;
        type Amount = i64;
        type Asset = &'static str;
        type VaultId = &'static str;

        fn vault_id(&self) -> &Self::VaultId {
            &self.vault_id
        }

        fn pseudo_account_id(&self) -> &Self::AccountId {
            &self.pseudo_account_id
        }

        fn subtract_cover_available(&mut self, amount: &Self::Amount) {
            self.cover_available -= *amount;
            self.steps.borrow_mut().push(format!("cover-={amount}"));
        }
    }

    #[derive(Clone, Debug, PartialEq, Eq)]
    struct TestVault {
        asset: &'static str,
    }

    impl LoanBrokerCoverWithdrawDoApplyVault for TestVault {
        type Asset = &'static str;

        fn asset(&self) -> &Self::Asset {
            &self.asset
        }
    }

    struct TestSink {
        steps: Rc<RefCell<Vec<String>>>,
        broker: Option<TestBroker>,
        vault: Option<TestVault>,
        withdraw_result: Ter,
        observed_destination: Option<&'static str>,
        observed_pseudo_account: Option<&'static str>,
        observed_pre_fee_balance: Option<i64>,
        observed_amount: Option<i64>,
    }

    impl LoanBrokerCoverWithdrawDoApplySink for TestSink {
        type Broker = TestBroker;
        type Vault = TestVault;
        type AccountId = &'static str;
        type Amount = i64;
        type Asset = &'static str;
        type VaultId = &'static str;

        fn read_broker(&mut self) -> Option<Self::Broker> {
            self.steps.borrow_mut().push("read_broker".to_string());
            self.broker.take()
        }

        fn read_vault(&mut self, vault_id: &Self::VaultId) -> Option<Self::Vault> {
            self.steps
                .borrow_mut()
                .push(format!("read_vault={vault_id}"));
            self.vault.take()
        }

        fn update_broker(&mut self, _broker: &Self::Broker) {
            self.steps.borrow_mut().push("update_broker".to_string());
        }

        fn associate_asset(&mut self, _broker: &Self::Broker, asset: &Self::Asset) {
            self.steps
                .borrow_mut()
                .push(format!("associate_asset={asset}"));
        }

        fn do_withdraw(
            &mut self,
            destination: &Self::AccountId,
            pseudo_account_id: &Self::AccountId,
            pre_fee_balance: &Self::Amount,
            amount: &Self::Amount,
        ) -> Ter {
            self.steps.borrow_mut().push("do_withdraw".to_string());
            self.observed_destination = Some(*destination);
            self.observed_pseudo_account = Some(*pseudo_account_id);
            self.observed_pre_fee_balance = Some(*pre_fee_balance);
            self.observed_amount = Some(*amount);
            self.withdraw_result
        }
    }

    fn build_sink(steps: Rc<RefCell<Vec<String>>>) -> TestSink {
        TestSink {
            steps: Rc::clone(&steps),
            broker: Some(TestBroker {
                vault_id: "vault-1",
                pseudo_account_id: "pseudo-1",
                cover_available: 90,
                steps,
            }),
            vault: Some(TestVault { asset: "USD" }),
            withdraw_result: Ter::TES_SUCCESS,
            observed_destination: None,
            observed_pseudo_account: None,
            observed_pre_fee_balance: None,
            observed_amount: None,
        }
    }

    #[test]
    fn loan_broker_cover_withdraw_do_apply_maps_missing_broker_to_internal() {
        let steps = Rc::new(RefCell::new(Vec::new()));
        let mut sink = TestSink {
            steps: Rc::clone(&steps),
            broker: None,
            vault: Some(TestVault { asset: "USD" }),
            withdraw_result: Ter::TES_SUCCESS,
            observed_destination: None,
            observed_pseudo_account: None,
            observed_pre_fee_balance: None,
            observed_amount: None,
        };

        let result = run_loan_broker_cover_withdraw_do_apply(&mut sink, &"dst", &500_i64, &15_i64);

        assert_eq!(result, Ter::TEC_INTERNAL);
        assert_eq!(steps.borrow().as_slice(), ["read_broker"]);
    }

    #[test]
    fn loan_broker_cover_withdraw_do_apply_maps_missing_vault_to_internal() {
        let steps = Rc::new(RefCell::new(Vec::new()));
        let mut sink = TestSink {
            steps: Rc::clone(&steps),
            broker: Some(TestBroker {
                vault_id: "vault-1",
                pseudo_account_id: "pseudo-1",
                cover_available: 90,
                steps: Rc::clone(&steps),
            }),
            vault: None,
            withdraw_result: Ter::TES_SUCCESS,
            observed_destination: None,
            observed_pseudo_account: None,
            observed_pre_fee_balance: None,
            observed_amount: None,
        };

        let result = run_loan_broker_cover_withdraw_do_apply(&mut sink, &"dst", &500_i64, &15_i64);

        assert_eq!(result, Ter::TEC_INTERNAL);
        assert_eq!(
            steps.borrow().as_slice(),
            ["read_broker", "read_vault=vault-1"]
        );
    }

    #[test]
    fn loan_broker_cover_withdraw_do_apply_returns_do_withdraw_failure_after_updates() {
        let steps = Rc::new(RefCell::new(Vec::new()));
        let mut sink = build_sink(Rc::clone(&steps));
        sink.withdraw_result = Ter::TER_NO_RIPPLE;

        let result = run_loan_broker_cover_withdraw_do_apply(&mut sink, &"dst", &500_i64, &15_i64);

        assert_eq!(result, Ter::TER_NO_RIPPLE);
        assert_eq!(
            steps.borrow().as_slice(),
            [
                "read_broker",
                "read_vault=vault-1",
                "cover-=15",
                "update_broker",
                "associate_asset=USD",
                "do_withdraw",
            ]
        );
        assert_eq!(sink.observed_destination, Some("dst"));
        assert_eq!(sink.observed_pseudo_account, Some("pseudo-1"));
        assert_eq!(sink.observed_pre_fee_balance, Some(500));
        assert_eq!(sink.observed_amount, Some(15));
    }

    #[test]
    fn loan_broker_cover_withdraw_do_apply_runs_current_on_success() {
        let steps = Rc::new(RefCell::new(Vec::new()));
        let mut sink = build_sink(Rc::clone(&steps));

        let result = run_loan_broker_cover_withdraw_do_apply(&mut sink, &"dst", &500_i64, &15_i64);

        assert_eq!(result, Ter::TES_SUCCESS);
        assert_eq!(
            steps.borrow().as_slice(),
            [
                "read_broker",
                "read_vault=vault-1",
                "cover-=15",
                "update_broker",
                "associate_asset=USD",
                "do_withdraw",
            ]
        );
        assert_eq!(sink.observed_destination, Some("dst"));
        assert_eq!(sink.observed_pseudo_account, Some("pseudo-1"));
        assert_eq!(sink.observed_pre_fee_balance, Some(500));
        assert_eq!(sink.observed_amount, Some(15));
    }
}
