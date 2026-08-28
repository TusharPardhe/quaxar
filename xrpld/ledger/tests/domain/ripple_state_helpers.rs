use std::sync::Arc;

use basics::base_uint::Uint160;
use ledger::{Ledger, LedgerHeader, RawView, ReadView, Sandbox, ripple_state_helpers};
use protocol::{
    AccountID, ApplyFlags, IOUAmount, Issue, STAmount, STLedgerEntry, Ter, currency_from_string,
    get_field_by_symbol as sf,
};

fn seed_account(ledger: &mut Ledger, account: AccountID) {
    let keylet = protocol::account_keylet(Uint160::from_void(account.data()));
    let mut entry = STLedgerEntry::new(keylet);
    entry.set_account_id(sf("sfAccount"), account);
    entry.set_field_u32(sf("sfSequence"), 1);
    entry.set_field_amount(
        sf("sfBalance"),
        STAmount::from_xrp_amount(protocol::XRPAmount::from_drops(100_000_000)),
    );
    entry.set_field_u32(sf("sfOwnerCount"), 0);
    entry.set_field_u32(sf("sfFlags"), 0);
    ledger.raw_insert(Arc::new(entry)).expect("seed account");
}

fn assert_root_directory_hints_are_present(view: &impl ReadView, issue: Issue, holder: AccountID) {
    let line = view
        .read(protocol::line(issue.account, holder, issue.currency))
        .expect("read trust line")
        .expect("trust line exists");

    assert!(line.is_field_present(sf("sfLowNode")));
    assert!(line.is_field_present(sf("sfHighNode")));
    assert_eq!(line.get_field_u64(sf("sfLowNode")), 0);
    assert_eq!(line.get_field_u64(sf("sfHighNode")), 0);

    // These zero-valued optional fields are still serialized into the state
    // leaf. Omitting either changes the canonical SHAMap account-state root.
    let bytes = line.get_serializer().data().to_vec();
    assert!(
        bytes
            .windows(9)
            .any(|window| window == [0x37, 0, 0, 0, 0, 0, 0, 0, 0])
    );
    assert!(
        bytes
            .windows(9)
            .any(|window| window == [0x38, 0, 0, 0, 0, 0, 0, 0, 0])
    );
}

fn trust_line_flags(view: &impl ReadView, issue: Issue, holder: AccountID) -> u32 {
    view.read(protocol::line(issue.account, holder, issue.currency))
        .expect("read trust line")
        .expect("trust line exists")
        .get_field_u32(sf("sfFlags"))
}

fn assert_balance_uses_no_account(view: &impl ReadView, issue: Issue, holder: AccountID) {
    let line = view
        .read(protocol::line(issue.account, holder, issue.currency))
        .expect("read trust line")
        .expect("trust line exists");
    assert_eq!(
        line.get_field_amount(sf("sfBalance")).issue().account,
        protocol::no_account()
    );
}

#[test]
fn issue_iou_persists_zero_root_directory_hints() {
    let issuer = AccountID::from_array([0x11; 20]);
    let holder = AccountID::from_array([0x22; 20]);
    let issue = Issue::new(currency_from_string("USD"), issuer);
    let amount = STAmount::from_iou_amount(
        sf("sfAmount"),
        IOUAmount::from_parts(10, 0).expect("valid IOU amount"),
        issue,
    );
    let mut ledger = Ledger::new(LedgerHeader::default(), false);
    seed_account(&mut ledger, issuer);
    seed_account(&mut ledger, holder);
    let mut view = Sandbox::new(Arc::new(ledger), ApplyFlags::NONE);

    assert_eq!(
        ripple_state_helpers::issue_iou(&mut view, &holder, &amount, &issue),
        Ter::TES_SUCCESS
    );
    assert_root_directory_hints_are_present(&view, issue, holder);
    assert_balance_uses_no_account(&view, issue, holder);
    // Issuer is low and holder is high: reserve the holder's high side and,
    // because neither account enables DefaultRipple, set both NoRipple bits.
    assert_eq!(trust_line_flags(&view, issue, holder), 0x0032_0000);
}

#[test]
fn direct_send_persists_zero_root_directory_hints() {
    let issuer = AccountID::from_array([0x31; 20]);
    let holder = AccountID::from_array([0x42; 20]);
    let issue = Issue::new(currency_from_string("EUR"), issuer);
    let amount = STAmount::from_iou_amount(
        sf("sfAmount"),
        IOUAmount::from_parts(25, 0).expect("valid IOU amount"),
        issue,
    );
    let mut ledger = Ledger::new(LedgerHeader::default(), false);
    seed_account(&mut ledger, issuer);
    seed_account(&mut ledger, holder);
    let mut view = Sandbox::new(Arc::new(ledger), ApplyFlags::NONE);

    assert_eq!(
        ripple_state_helpers::direct_send_no_fee_iou_pub(&mut view, &issuer, &holder, &amount,),
        Ter::TES_SUCCESS
    );
    assert_root_directory_hints_are_present(&view, issue, holder);
    assert_eq!(trust_line_flags(&view, issue, holder), 0x0032_0000);
}

#[test]
fn issue_iou_uses_the_low_holder_side_when_the_issuer_sorts_high() {
    let issuer = AccountID::from_array([0x44; 20]);
    let holder = AccountID::from_array([0x12; 20]);
    let issue = Issue::new(currency_from_string("JPY"), issuer);
    let amount = STAmount::from_iou_amount(
        sf("sfAmount"),
        IOUAmount::from_parts(7, 0).expect("valid IOU amount"),
        issue,
    );
    let mut ledger = Ledger::new(LedgerHeader::default(), false);
    seed_account(&mut ledger, issuer);
    seed_account(&mut ledger, holder);
    let mut view = Sandbox::new(Arc::new(ledger), ApplyFlags::NONE);

    assert_eq!(
        ripple_state_helpers::issue_iou(&mut view, &holder, &amount, &issue),
        Ter::TES_SUCCESS
    );
    let line = view
        .read(protocol::line(issue.account, holder, issue.currency))
        .expect("read trust line")
        .expect("trust line exists");
    assert_eq!(line.get_field_u32(sf("sfFlags")), 0x0031_0000);
    assert_eq!(line.get_field_amount(sf("sfBalance")).iou(), amount.iou());
    assert_balance_uses_no_account(&view, issue, holder);
}
