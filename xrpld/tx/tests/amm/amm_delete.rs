//! Tests for the narrowed `AMMDelete` helper surface.

use protocol::{
    AccountID, Asset, Currency, IOUAmount, Issue, STAmount, STLedgerEntry, Ter,
    get_field_by_symbol as sf,
};
use tx::{
    AMMDeleteApplyFacts, AMMDeleteApplySink, AMMDeletePreclaimFacts,
    amm_delete_check_extra_features, run_amm_delete_do_apply, run_amm_delete_preclaim_facts,
};

fn account(fill: u8) -> AccountID {
    AccountID::from_array([fill; 20])
}

fn issue(fill: u8, issuer: AccountID) -> Issue {
    Issue::new(Currency::from_array([fill; 20]), issuer)
}

fn amm_entry(amm_account: AccountID, asset1: Asset, asset2: Asset) -> STLedgerEntry {
    let mut sle = STLedgerEntry::new(protocol::keylet::amm(asset1, asset2));
    let lpt = protocol::amm_lpt_issue_from_assets(asset1, asset2, amm_account);
    sle.set_account_id(sf("sfAccount"), amm_account);
    sle.set_field_amount(
        sf("sfLPTokenBalance"),
        STAmount::from_iou_amount(sf("sfLPTokenBalance"), IOUAmount::new(), lpt),
    );
    sle
}

#[derive(Default)]
struct Sink {
    entry: Option<STLedgerEntry>,
    deleted_account: Option<AccountID>,
    deleted_entry: bool,
    delete_account_result: Ter,
}

impl AMMDeleteApplySink for Sink {
    fn get_amm_entry(&mut self, _asset1: &Asset, _asset2: &Asset) -> Option<STLedgerEntry> {
        self.entry.clone()
    }

    fn delete_amm_entry(&mut self, _sle: STLedgerEntry) -> Ter {
        self.deleted_entry = true;
        Ter::TES_SUCCESS
    }

    fn delete_amm_account(&mut self, amm_account: &AccountID) -> Ter {
        self.deleted_account = Some(*amm_account);
        self.delete_account_result
    }
}

#[test]
fn amm_delete_check_extra_features_matches_cpp_mpt_gate() {
    assert!(amm_delete_check_extra_features(true, false, false));
    assert!(amm_delete_check_extra_features(true, true, true));
    assert!(!amm_delete_check_extra_features(true, false, true));
    assert!(!amm_delete_check_extra_features(false, true, false));
}

#[test]
fn amm_delete_preclaim_rejects_missing_or_nonempty_amm() {
    assert_eq!(
        run_amm_delete_preclaim_facts(AMMDeletePreclaimFacts {
            amm_exists: false,
            lp_token_balance_is_zero: true,
        }),
        Ter::TER_NO_AMM
    );
    assert_eq!(
        run_amm_delete_preclaim_facts(AMMDeletePreclaimFacts {
            amm_exists: true,
            lp_token_balance_is_zero: false,
        }),
        Ter::TEC_AMM_NOT_EMPTY
    );
    assert_eq!(
        run_amm_delete_preclaim_facts(AMMDeletePreclaimFacts {
            amm_exists: true,
            lp_token_balance_is_zero: true,
        }),
        Ter::TES_SUCCESS
    );
}

#[test]
fn amm_delete_returns_no_amm_when_entry_missing() {
    let issuer = account(1);
    let asset1 = Asset::from(issue(2, issuer));
    let asset2 = Asset::from(issue(3, issuer));
    let mut sink = Sink {
        delete_account_result: Ter::TES_SUCCESS,
        ..Default::default()
    };

    let result = run_amm_delete_do_apply(
        AMMDeleteApplyFacts {
            account: account(4),
            asset1,
            asset2,
        },
        &mut sink,
    );

    assert_eq!(result, Ter::TER_NO_AMM);
    assert!(sink.deleted_account.is_none());
    assert!(!sink.deleted_entry);
}

#[test]
fn amm_delete_uses_amm_entry_account_field() {
    let issuer = account(1);
    let amm_account = account(9);
    let asset1 = Asset::from(issue(2, issuer));
    let asset2 = Asset::from(issue(3, issuer));
    let mut sink = Sink {
        entry: Some(amm_entry(amm_account, asset1, asset2)),
        delete_account_result: Ter::TES_SUCCESS,
        ..Default::default()
    };

    let result = run_amm_delete_do_apply(
        AMMDeleteApplyFacts {
            account: account(4),
            asset1,
            asset2,
        },
        &mut sink,
    );

    assert_eq!(result, Ter::TES_SUCCESS);
    assert_eq!(sink.deleted_account, Some(amm_account));
    assert!(sink.deleted_entry);
}

#[test]
fn amm_delete_stops_when_account_cleanup_fails() {
    let issuer = account(1);
    let amm_account = account(9);
    let asset1 = Asset::from(issue(2, issuer));
    let asset2 = Asset::from(issue(3, issuer));
    let mut sink = Sink {
        entry: Some(amm_entry(amm_account, asset1, asset2)),
        delete_account_result: Ter::TEC_INTERNAL,
        ..Default::default()
    };

    let result = run_amm_delete_do_apply(
        AMMDeleteApplyFacts {
            account: account(4),
            asset1,
            asset2,
        },
        &mut sink,
    );

    assert_eq!(result, Ter::TEC_INTERNAL);
    assert_eq!(sink.deleted_account, Some(amm_account));
    assert!(!sink.deleted_entry);
}
