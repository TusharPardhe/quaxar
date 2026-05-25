//! Integration tests that pin the narrowed Rust
//! `LoanSet.cpp::doApply()` `associateAsset(...)` tail to the current C++
//! behavior.

use std::rc::Rc;

use protocol::Ter;
use tx::{LoanSetDoApplyAssociateAssetsSink, run_loan_set_do_apply_associate_assets};

#[derive(Default)]
struct RecordingSink {
    steps: Vec<String>,
    seen_asset_ptrs: Vec<*const str>,
    vault_calls: u32,
    broker_calls: u32,
    loan_calls: u32,
}

impl LoanSetDoApplyAssociateAssetsSink for RecordingSink {
    type Asset = Rc<str>;

    fn associate_vault_asset(&mut self, asset: &Self::Asset) {
        self.vault_calls += 1;
        self.steps.push(format!("vault={asset}"));
        self.seen_asset_ptrs.push(Rc::as_ptr(asset));
    }

    fn associate_broker_asset(&mut self, asset: &Self::Asset) {
        self.broker_calls += 1;
        self.steps.push(format!("broker={asset}"));
        self.seen_asset_ptrs.push(Rc::as_ptr(asset));
    }

    fn associate_loan_asset(&mut self, asset: &Self::Asset) {
        self.loan_calls += 1;
        self.steps.push(format!("loan={asset}"));
        self.seen_asset_ptrs.push(Rc::as_ptr(asset));
    }
}

#[test]
fn tx_loan_set_do_apply_associate_assets_uses_current() {
    let mut sink = RecordingSink::default();
    let vault_asset: Rc<str> = Rc::from("USD");

    let result = run_loan_set_do_apply_associate_assets(&mut sink, &vault_asset);

    assert_eq!(result, Ter::TES_SUCCESS);
    assert_eq!(sink.steps, vec!["vault=USD", "broker=USD", "loan=USD",]);
}

#[test]
fn tx_loan_set_do_apply_associate_assets_reuses_the_same_asset_for_all_calls() {
    let mut sink = RecordingSink::default();
    let vault_asset: Rc<str> = Rc::from("XRP");

    let result = run_loan_set_do_apply_associate_assets(&mut sink, &vault_asset);

    assert_eq!(result, Ter::TES_SUCCESS);
    assert_eq!(
        sink.seen_asset_ptrs,
        vec![
            Rc::as_ptr(&vault_asset),
            Rc::as_ptr(&vault_asset),
            Rc::as_ptr(&vault_asset),
        ]
    );
}

#[test]
fn tx_loan_set_do_apply_associate_assets_calls_each_target_once() {
    let mut sink = RecordingSink::default();
    let vault_asset: Rc<str> = Rc::from("EUR");

    let result = run_loan_set_do_apply_associate_assets(&mut sink, &vault_asset);

    assert_eq!(result, Ter::TES_SUCCESS);
    assert_eq!(sink.vault_calls, 1);
    assert_eq!(sink.broker_calls, 1);
    assert_eq!(sink.loan_calls, 1);
}
