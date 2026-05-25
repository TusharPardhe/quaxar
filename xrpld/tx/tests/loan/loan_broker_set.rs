//! Integration tests that pin the narrowed Rust `LoanBrokerSet.cpp` control
//! flow to the current C++ behavior.

use std::{cell::Cell, cell::RefCell, rc::Rc};

use protocol::{Ter, trans_token};
use tx::loan_broker_set::{
    LoanBrokerSetDoApplyBroker, LoanBrokerSetDoApplyFacts, LoanBrokerSetDoApplyPseudoAccount,
    LoanBrokerSetDoApplySink, LoanBrokerSetDoApplyVault, run_loan_broker_set_check_extra_features,
    run_loan_broker_set_do_apply,
};
use tx::{
    LoanBrokerSetPreclaimFacts, LoanBrokerSetPreflightFacts, run_loan_broker_set_preclaim,
    run_loan_broker_set_preflight,
};

fn preflight_base() -> LoanBrokerSetPreflightFacts {
    LoanBrokerSetPreflightFacts {
        data_is_present: false,
        data_is_empty: true,
        data_length_is_valid: true,
        management_fee_rate_is_valid: true,
        cover_rate_minimum_is_valid: true,
        cover_rate_liquidation_is_valid: true,
        debt_maximum_is_valid: true,
        loan_broker_id_is_present: false,
        management_fee_rate_is_present: false,
        cover_rate_minimum_is_present: false,
        cover_rate_liquidation_is_present: false,
        loan_broker_id_is_zero: false,
        vault_id_is_present: false,
        vault_id_is_zero: false,
        cover_rate_minimum_value: None,
        cover_rate_liquidation_value: None,
    }
}

fn preclaim_base() -> LoanBrokerSetPreclaimFacts {
    LoanBrokerSetPreclaimFacts {
        vault_exists: true,
        submitter_is_vault_owner: true,
        broker_id_is_present: false,
        broker_exists: true,
        vault_id_matches_existing_broker: true,
        submitter_is_broker_owner: true,
        debt_maximum_is_zero_or_not_below_current_debt: true,
        debt_maximum_is_present: false,
        debt_maximum_is_representable: true,
        can_add_holding_result: Ter::TES_SUCCESS,
        check_frozen_result: Ter::TES_SUCCESS,
    }
}

#[test]
fn tx_loan_broker_set_check_extra_features_delegates_to_lending_gate() {
    let helper_called = Cell::new(false);

    let disabled = run_loan_broker_set_check_extra_features(false, || {
        helper_called.set(true);
        true
    });
    assert!(!disabled);
    assert!(!helper_called.get());

    assert!(run_loan_broker_set_check_extra_features(true, || true));
    assert!(!run_loan_broker_set_check_extra_features(true, || false));
}

#[test]
fn tx_loan_broker_set_preflight_rejects_invalid_data_length() {
    let result = run_loan_broker_set_preflight(LoanBrokerSetPreflightFacts {
        data_is_present: true,
        data_is_empty: false,
        data_length_is_valid: false,
        ..preflight_base()
    });

    assert_eq!(result, Ter::TEM_INVALID);
    assert_eq!(trans_token(result), "temINVALID");
}

#[test]
fn tx_loan_broker_set_preflight_rejects_fixed_fields_on_existing_broker() {
    let result = run_loan_broker_set_preflight(LoanBrokerSetPreflightFacts {
        loan_broker_id_is_present: true,
        management_fee_rate_is_present: true,
        ..preflight_base()
    });

    assert_eq!(result, Ter::TEM_INVALID);
}

#[test]
fn tx_loan_broker_set_preflight_prioritizes_fixed_fields_over_zero_broker_id() {
    let result = run_loan_broker_set_preflight(LoanBrokerSetPreflightFacts {
        loan_broker_id_is_present: true,
        management_fee_rate_is_present: true,
        loan_broker_id_is_zero: true,
        ..preflight_base()
    });

    assert_eq!(result, Ter::TEM_INVALID);
}

#[test]
fn tx_loan_broker_set_preflight_rejects_zero_broker_id() {
    let result = run_loan_broker_set_preflight(LoanBrokerSetPreflightFacts {
        loan_broker_id_is_present: true,
        loan_broker_id_is_zero: true,
        ..preflight_base()
    });

    assert_eq!(result, Ter::TEM_INVALID);
}

#[test]
fn tx_loan_broker_set_preflight_rejects_zero_vault_id() {
    let result = run_loan_broker_set_preflight(LoanBrokerSetPreflightFacts {
        vault_id_is_present: true,
        vault_id_is_zero: true,
        ..preflight_base()
    });

    assert_eq!(result, Ter::TEM_INVALID);
}

#[test]
fn tx_loan_broker_set_preflight_rejects_mismatched_cover_rates() {
    let result = run_loan_broker_set_preflight(LoanBrokerSetPreflightFacts {
        cover_rate_minimum_value: Some(1),
        cover_rate_liquidation_value: None,
        ..preflight_base()
    });

    assert_eq!(result, Ter::TEM_INVALID);
}

#[test]
fn tx_loan_broker_set_preflight_accepts_valid_payload() {
    assert_eq!(
        run_loan_broker_set_preflight(preflight_base()),
        Ter::TES_SUCCESS
    );
}

#[test]
fn tx_loan_broker_set_preclaim_rejects_missing_vault() {
    let result = run_loan_broker_set_preclaim(LoanBrokerSetPreclaimFacts {
        vault_exists: false,
        ..preclaim_base()
    });

    assert_eq!(result, Ter::TEC_NO_ENTRY);
    assert_eq!(trans_token(result), "tecNO_ENTRY");
}

#[test]
fn tx_loan_broker_set_preclaim_prioritizes_missing_vault_over_other_failures() {
    let result = run_loan_broker_set_preclaim(LoanBrokerSetPreclaimFacts {
        vault_exists: false,
        submitter_is_vault_owner: false,
        broker_id_is_present: true,
        broker_exists: false,
        ..preclaim_base()
    });

    assert_eq!(result, Ter::TEC_NO_ENTRY);
}

#[test]
fn tx_loan_broker_set_preclaim_rejects_non_owner_of_vault() {
    let result = run_loan_broker_set_preclaim(LoanBrokerSetPreclaimFacts {
        submitter_is_vault_owner: false,
        ..preclaim_base()
    });

    assert_eq!(result, Ter::TEC_NO_PERMISSION);
}

#[test]
fn tx_loan_broker_set_preclaim_rejects_missing_broker_on_update() {
    let result = run_loan_broker_set_preclaim(LoanBrokerSetPreclaimFacts {
        broker_id_is_present: true,
        broker_exists: false,
        ..preclaim_base()
    });

    assert_eq!(result, Ter::TEC_NO_ENTRY);
}

#[test]
fn tx_loan_broker_set_preclaim_rejects_vault_mismatch_on_update() {
    let result = run_loan_broker_set_preclaim(LoanBrokerSetPreclaimFacts {
        broker_id_is_present: true,
        vault_id_matches_existing_broker: false,
        ..preclaim_base()
    });

    assert_eq!(result, Ter::TEC_NO_PERMISSION);
}

#[test]
fn tx_loan_broker_set_preclaim_rejects_non_owner_of_broker() {
    let result = run_loan_broker_set_preclaim(LoanBrokerSetPreclaimFacts {
        broker_id_is_present: true,
        submitter_is_broker_owner: false,
        ..preclaim_base()
    });

    assert_eq!(result, Ter::TEC_NO_PERMISSION);
}

#[test]
fn tx_loan_broker_set_preclaim_rejects_debt_maximum_below_current_debt() {
    let result = run_loan_broker_set_preclaim(LoanBrokerSetPreclaimFacts {
        broker_id_is_present: true,
        debt_maximum_is_zero_or_not_below_current_debt: false,
        ..preclaim_base()
    });

    assert_eq!(result, Ter::TEC_LIMIT_EXCEEDED);
}

#[test]
fn tx_loan_broker_set_preclaim_rejects_unrepresentable_debt_maximum() {
    let result = run_loan_broker_set_preclaim(LoanBrokerSetPreclaimFacts {
        debt_maximum_is_present: true,
        debt_maximum_is_representable: false,
        ..preclaim_base()
    });

    assert_eq!(result, Ter::TEC_PRECISION_LOSS);
}

#[test]
fn tx_loan_broker_set_preclaim_rejects_create_path_failures() {
    let add_holding = run_loan_broker_set_preclaim(LoanBrokerSetPreclaimFacts {
        can_add_holding_result: Ter::TEC_NO_PERMISSION,
        ..preclaim_base()
    });
    let frozen = run_loan_broker_set_preclaim(LoanBrokerSetPreclaimFacts {
        check_frozen_result: Ter::TEC_NO_PERMISSION,
        ..preclaim_base()
    });

    assert_eq!(add_holding, Ter::TEC_NO_PERMISSION);
    assert_eq!(frozen, Ter::TEC_NO_PERMISSION);
}

#[test]
fn tx_loan_broker_set_preclaim_accepts_valid_create_and_update_paths() {
    assert_eq!(
        run_loan_broker_set_preclaim(preclaim_base()),
        Ter::TES_SUCCESS
    );

    assert_eq!(
        run_loan_broker_set_preclaim(LoanBrokerSetPreclaimFacts {
            broker_id_is_present: true,
            debt_maximum_is_present: true,
            ..preclaim_base()
        }),
        Ter::TES_SUCCESS
    );
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct TestBroker {
    vault_id: &'static str,
    steps: Rc<RefCell<Vec<String>>>,
}

impl LoanBrokerSetDoApplyBroker for TestBroker {
    type AccountId = &'static str;
    type Amount = i64;
    type Asset = &'static str;
    type VaultId = &'static str;
    type Sequence = u32;
    type Data = &'static str;

    fn vault_id(&self) -> &Self::VaultId {
        self.steps.borrow_mut().push("broker.vault_id".to_string());
        &self.vault_id
    }

    fn set_data(&mut self, value: Self::Data) {
        self.steps.borrow_mut().push(format!("set_data={value}"));
    }

    fn set_debt_maximum(&mut self, value: Self::Amount) {
        self.steps
            .borrow_mut()
            .push(format!("set_debt_maximum={value}"));
    }

    fn set_sequence(&mut self, value: Self::Sequence) {
        self.steps
            .borrow_mut()
            .push(format!("set_sequence={value}"));
    }

    fn set_vault_id(&mut self, value: Self::VaultId) {
        self.steps
            .borrow_mut()
            .push(format!("set_vault_id={value}"));
    }

    fn set_owner(&mut self, value: Self::AccountId) {
        self.steps.borrow_mut().push(format!("set_owner={value}"));
    }

    fn set_account(&mut self, value: Self::AccountId) {
        self.steps.borrow_mut().push(format!("set_account={value}"));
    }

    fn set_loan_sequence(&mut self, value: Self::Sequence) {
        self.steps
            .borrow_mut()
            .push(format!("set_loan_sequence={value}"));
    }

    fn set_management_fee_rate(&mut self, value: u32) {
        self.steps
            .borrow_mut()
            .push(format!("set_management_fee_rate={value}"));
    }

    fn set_cover_rate_minimum(&mut self, value: u32) {
        self.steps
            .borrow_mut()
            .push(format!("set_cover_rate_minimum={value}"));
    }

    fn set_cover_rate_liquidation(&mut self, value: u32) {
        self.steps
            .borrow_mut()
            .push(format!("set_cover_rate_liquidation={value}"));
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct TestVault {
    account_id: &'static str,
    asset: &'static str,
    steps: Rc<RefCell<Vec<String>>>,
}

impl LoanBrokerSetDoApplyVault for TestVault {
    type AccountId = &'static str;
    type Asset = &'static str;

    fn account_id(&self) -> &Self::AccountId {
        self.steps.borrow_mut().push("vault.account_id".to_string());
        &self.account_id
    }

    fn asset(&self) -> &Self::Asset {
        self.steps.borrow_mut().push("vault.asset".to_string());
        &self.asset
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct TestPseudoAccount {
    account_id: &'static str,
}

impl LoanBrokerSetDoApplyPseudoAccount for TestPseudoAccount {
    type AccountId = &'static str;

    fn account_id(&self) -> &Self::AccountId {
        &self.account_id
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct TestOwner;

struct TestSink {
    steps: Rc<RefCell<Vec<String>>>,
    broker: Option<TestBroker>,
    vault: Option<TestVault>,
    owner: Option<TestOwner>,
    reserve: i64,
    dir_link_broker_result: Ter,
    dir_link_vault_result: Ter,
    create_pseudo_result: Result<TestPseudoAccount, Ter>,
    add_empty_holding_result: Ter,
}

impl LoanBrokerSetDoApplySink for TestSink {
    type Broker = TestBroker;
    type Vault = TestVault;
    type Owner = TestOwner;
    type PseudoAccount = TestPseudoAccount;
    type AccountId = &'static str;
    type BrokerId = &'static str;
    type VaultId = &'static str;
    type Amount = i64;
    type Asset = &'static str;
    type Sequence = u32;
    type Data = &'static str;
    type OwnerCount = u32;

    fn read_broker(&mut self, broker_id: &Self::BrokerId) -> Option<Self::Broker> {
        self.steps
            .borrow_mut()
            .push(format!("read_broker={broker_id}"));
        self.broker.take()
    }

    fn read_vault(&mut self, vault_id: &Self::VaultId) -> Option<Self::Vault> {
        self.steps
            .borrow_mut()
            .push(format!("read_vault={vault_id}"));
        self.vault.take()
    }

    fn read_owner(&mut self, account: &Self::AccountId) -> Option<Self::Owner> {
        self.steps
            .borrow_mut()
            .push(format!("read_owner={account}"));
        self.owner.take()
    }

    fn make_broker(
        &mut self,
        account: &Self::AccountId,
        sequence: &Self::Sequence,
    ) -> Self::Broker {
        self.steps
            .borrow_mut()
            .push(format!("make_broker={account}:{sequence}"));
        TestBroker {
            vault_id: "uninitialized",
            steps: Rc::clone(&self.steps),
        }
    }

    fn dir_link_broker(&mut self, _broker: &mut Self::Broker) -> Ter {
        self.steps.borrow_mut().push("dir_link_broker".to_string());
        self.dir_link_broker_result
    }

    fn dir_link_vault(
        &mut self,
        _broker: &mut Self::Broker,
        vault_pseudo_id: &Self::AccountId,
    ) -> Ter {
        self.steps
            .borrow_mut()
            .push(format!("dir_link_vault={vault_pseudo_id}"));
        self.dir_link_vault_result
    }

    fn adjust_owner_count(&mut self, _owner: &mut Self::Owner, delta: u32) -> Self::OwnerCount {
        self.steps
            .borrow_mut()
            .push(format!("adjust_owner_count={delta}"));
        7
    }

    fn account_reserve(&mut self, owner_count: &Self::OwnerCount) -> Self::Amount {
        self.steps
            .borrow_mut()
            .push(format!("account_reserve={owner_count}"));
        self.reserve
    }

    fn create_pseudo_account(
        &mut self,
        _broker: &Self::Broker,
    ) -> Result<Self::PseudoAccount, Ter> {
        self.steps
            .borrow_mut()
            .push("create_pseudo_account".to_string());
        self.create_pseudo_result.clone()
    }

    fn add_empty_holding(
        &mut self,
        pseudo_account_id: &Self::AccountId,
        pre_fee_balance: &Self::Amount,
        asset: &Self::Asset,
    ) -> Ter {
        self.steps.borrow_mut().push(format!(
            "add_empty_holding={pseudo_account_id}:{pre_fee_balance}:{asset}"
        ));
        self.add_empty_holding_result
    }

    fn update_broker(&mut self, _broker: &Self::Broker) {
        self.steps.borrow_mut().push("update_broker".to_string());
    }

    fn insert_broker(&mut self, _broker: &Self::Broker) {
        self.steps.borrow_mut().push("insert_broker".to_string());
    }

    fn associate_asset(&mut self, _broker: &Self::Broker, asset: &Self::Asset) {
        self.steps
            .borrow_mut()
            .push(format!("associate_asset={asset}"));
    }
}

fn do_apply_facts_update()
-> LoanBrokerSetDoApplyFacts<&'static str, &'static str, &'static str, u32, i64, &'static str> {
    LoanBrokerSetDoApplyFacts {
        account: "owner",
        broker_id: Some("broker-1"),
        vault_id: "vault-1",
        sequence: 22,
        pre_fee_balance: 100,
        data: Some("blob"),
        management_fee_rate: None,
        debt_maximum: Some(900),
        cover_rate_minimum: None,
        cover_rate_liquidation: None,
    }
}

fn do_apply_facts_create()
-> LoanBrokerSetDoApplyFacts<&'static str, &'static str, &'static str, u32, i64, &'static str> {
    LoanBrokerSetDoApplyFacts {
        account: "owner",
        broker_id: None,
        vault_id: "vault-1",
        sequence: 22,
        pre_fee_balance: 100,
        data: Some("blob"),
        management_fee_rate: Some(11),
        debt_maximum: Some(900),
        cover_rate_minimum: Some(33),
        cover_rate_liquidation: Some(44),
    }
}

#[test]
fn tx_loan_broker_set_do_apply_update_path_runs_current() {
    let steps = Rc::new(RefCell::new(Vec::new()));
    let mut sink = TestSink {
        steps: Rc::clone(&steps),
        broker: Some(TestBroker {
            vault_id: "vault-1",
            steps: Rc::clone(&steps),
        }),
        vault: Some(TestVault {
            account_id: "vault-pseudo",
            asset: "USD",
            steps: Rc::clone(&steps),
        }),
        owner: Some(TestOwner),
        reserve: 50,
        dir_link_broker_result: Ter::TES_SUCCESS,
        dir_link_vault_result: Ter::TES_SUCCESS,
        create_pseudo_result: Ok(TestPseudoAccount {
            account_id: "broker-pseudo",
        }),
        add_empty_holding_result: Ter::TES_SUCCESS,
    };

    let result = run_loan_broker_set_do_apply(&mut sink, do_apply_facts_update());

    assert_eq!(result, Ter::TES_SUCCESS);
    assert_eq!(
        steps.borrow().as_slice(),
        [
            "read_broker=broker-1",
            "broker.vault_id",
            "read_vault=vault-1",
            "set_data=blob",
            "set_debt_maximum=900",
            "update_broker",
            "vault.asset",
            "associate_asset=USD",
        ]
    );
}

#[test]
fn tx_loan_broker_set_do_apply_update_path_maps_missing_broker_and_vault() {
    let steps = Rc::new(RefCell::new(Vec::new()));
    let mut missing_broker = TestSink {
        steps: Rc::clone(&steps),
        broker: None,
        vault: Some(TestVault {
            account_id: "vault-pseudo",
            asset: "USD",
            steps: Rc::clone(&steps),
        }),
        owner: Some(TestOwner),
        reserve: 50,
        dir_link_broker_result: Ter::TES_SUCCESS,
        dir_link_vault_result: Ter::TES_SUCCESS,
        create_pseudo_result: Ok(TestPseudoAccount {
            account_id: "broker-pseudo",
        }),
        add_empty_holding_result: Ter::TES_SUCCESS,
    };

    assert_eq!(
        run_loan_broker_set_do_apply(&mut missing_broker, do_apply_facts_update()),
        Ter::TEF_BAD_LEDGER
    );
    assert_eq!(steps.borrow().as_slice(), ["read_broker=broker-1"]);

    steps.borrow_mut().clear();
    let mut missing_vault = TestSink {
        steps: Rc::clone(&steps),
        broker: Some(TestBroker {
            vault_id: "vault-1",
            steps: Rc::clone(&steps),
        }),
        vault: None,
        owner: Some(TestOwner),
        reserve: 50,
        dir_link_broker_result: Ter::TES_SUCCESS,
        dir_link_vault_result: Ter::TES_SUCCESS,
        create_pseudo_result: Ok(TestPseudoAccount {
            account_id: "broker-pseudo",
        }),
        add_empty_holding_result: Ter::TES_SUCCESS,
    };

    assert_eq!(
        run_loan_broker_set_do_apply(&mut missing_vault, do_apply_facts_update()),
        Ter::TEC_INTERNAL
    );
    assert_eq!(
        steps.borrow().as_slice(),
        [
            "read_broker=broker-1",
            "broker.vault_id",
            "read_vault=vault-1"
        ]
    );
}

#[test]
fn tx_loan_broker_set_do_apply_create_path_runs_current() {
    let steps = Rc::new(RefCell::new(Vec::new()));
    let mut sink = TestSink {
        steps: Rc::clone(&steps),
        broker: Some(TestBroker {
            vault_id: "vault-1",
            steps: Rc::clone(&steps),
        }),
        vault: Some(TestVault {
            account_id: "vault-pseudo",
            asset: "USD",
            steps: Rc::clone(&steps),
        }),
        owner: Some(TestOwner),
        reserve: 50,
        dir_link_broker_result: Ter::TES_SUCCESS,
        dir_link_vault_result: Ter::TES_SUCCESS,
        create_pseudo_result: Ok(TestPseudoAccount {
            account_id: "broker-pseudo",
        }),
        add_empty_holding_result: Ter::TES_SUCCESS,
    };

    let result = run_loan_broker_set_do_apply(&mut sink, do_apply_facts_create());

    assert_eq!(result, Ter::TES_SUCCESS);
    assert_eq!(
        steps.borrow().as_slice(),
        [
            "read_vault=vault-1",
            "read_owner=owner",
            "make_broker=owner:22",
            "dir_link_broker",
            "vault.account_id",
            "dir_link_vault=vault-pseudo",
            "adjust_owner_count=2",
            "account_reserve=7",
            "create_pseudo_account",
            "vault.asset",
            "add_empty_holding=broker-pseudo:100:USD",
            "set_sequence=22",
            "set_vault_id=vault-1",
            "set_owner=owner",
            "set_account=broker-pseudo",
            "set_loan_sequence=1",
            "set_data=blob",
            "set_management_fee_rate=11",
            "set_debt_maximum=900",
            "set_cover_rate_minimum=33",
            "set_cover_rate_liquidation=44",
            "insert_broker",
            "vault.asset",
            "associate_asset=USD",
        ]
    );
}

#[test]
fn tx_loan_broker_set_do_apply_create_path_returns_early_failures() {
    let steps = Rc::new(RefCell::new(Vec::new()));
    let mut dir_fail = TestSink {
        steps: Rc::clone(&steps),
        broker: Some(TestBroker {
            vault_id: "vault-1",
            steps: Rc::clone(&steps),
        }),
        vault: Some(TestVault {
            account_id: "vault-pseudo",
            asset: "USD",
            steps: Rc::clone(&steps),
        }),
        owner: Some(TestOwner),
        reserve: 50,
        dir_link_broker_result: Ter::TEC_DIR_FULL,
        dir_link_vault_result: Ter::TES_SUCCESS,
        create_pseudo_result: Ok(TestPseudoAccount {
            account_id: "broker-pseudo",
        }),
        add_empty_holding_result: Ter::TES_SUCCESS,
    };
    assert_eq!(
        run_loan_broker_set_do_apply(&mut dir_fail, do_apply_facts_create()),
        Ter::TEC_DIR_FULL
    );

    steps.borrow_mut().clear();
    let mut reserve_fail = TestSink {
        steps: Rc::clone(&steps),
        broker: Some(TestBroker {
            vault_id: "vault-1",
            steps: Rc::clone(&steps),
        }),
        vault: Some(TestVault {
            account_id: "vault-pseudo",
            asset: "USD",
            steps: Rc::clone(&steps),
        }),
        owner: Some(TestOwner),
        reserve: 150,
        dir_link_broker_result: Ter::TES_SUCCESS,
        dir_link_vault_result: Ter::TES_SUCCESS,
        create_pseudo_result: Ok(TestPseudoAccount {
            account_id: "broker-pseudo",
        }),
        add_empty_holding_result: Ter::TES_SUCCESS,
    };
    assert_eq!(
        run_loan_broker_set_do_apply(&mut reserve_fail, do_apply_facts_create()),
        Ter::TEC_INSUFFICIENT_RESERVE
    );

    steps.borrow_mut().clear();
    let mut pseudo_fail = TestSink {
        steps: Rc::clone(&steps),
        broker: Some(TestBroker {
            vault_id: "vault-1",
            steps: Rc::clone(&steps),
        }),
        vault: Some(TestVault {
            account_id: "vault-pseudo",
            asset: "USD",
            steps: Rc::clone(&steps),
        }),
        owner: Some(TestOwner),
        reserve: 50,
        dir_link_broker_result: Ter::TES_SUCCESS,
        dir_link_vault_result: Ter::TES_SUCCESS,
        create_pseudo_result: Err(Ter::TEC_NO_PERMISSION),
        add_empty_holding_result: Ter::TES_SUCCESS,
    };
    assert_eq!(
        run_loan_broker_set_do_apply(&mut pseudo_fail, do_apply_facts_create()),
        Ter::TEC_NO_PERMISSION
    );
}
