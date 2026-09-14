use std::sync::Arc;

use basics::{
    base_uint::{Uint160, Uint192, Uint256},
    number::NumberParts as RuntimeNumber,
};
use ledger::{ApplyViewImpl, Fees, Ledger, LedgerHeader, RawView, ReadView};
use protocol::{
    AccountID, ApplyFlags, Asset, LedgerEntryType, MPTIssue, STAmount, STIssue, STLedgerEntry,
    STNumber, STTx, TxType, XRPAmount, account_keylet, get_field_by_symbol, xrp_issue,
};

pub(super) const DUE: u32 = 100;
pub(super) const GRACE: u32 = 20;
pub(super) const PAYMENT: i64 = 100;
pub(super) const SERVICE_FEE: i64 = 10;

#[derive(Clone, Copy)]
pub(super) struct Parties {
    pub(super) borrower: AccountID,
    pub(super) broker_owner: AccountID,
    pub(super) broker_pseudo: AccountID,
    pub(super) vault_pseudo: AccountID,
    pub(super) issuer: AccountID,
    pub(super) loan_id: Uint256,
    pub(super) broker_id: Uint256,
    pub(super) vault_id: Uint256,
}

pub(super) fn parties(seed: u8) -> Parties {
    Parties {
        borrower: account(seed),
        broker_owner: account(seed.wrapping_add(1)),
        broker_pseudo: account(seed.wrapping_add(2)),
        vault_pseudo: account(seed.wrapping_add(3)),
        issuer: account(seed.wrapping_add(4)),
        loan_id: Uint256::from_array([seed.wrapping_add(5); 32]),
        broker_id: Uint256::from_array([seed.wrapping_add(6); 32]),
        vault_id: Uint256::from_array([seed.wrapping_add(7); 32]),
    }
}

pub(super) fn account(value: u8) -> AccountID {
    AccountID::from_array([value; 20])
}
pub(super) fn raw(account: AccountID) -> Uint160 {
    Uint160::from_void(account.data())
}
pub(super) fn sf(name: &str) -> &'static protocol::SField {
    get_field_by_symbol(name)
}

pub(super) fn number(asset: Asset, value: i64) -> STNumber {
    let mut value = STNumber::from(RuntimeNumber::from_i64(value));
    value.associate_asset(asset);
    value
}

pub(super) fn account_root(
    account: AccountID,
    balance: i64,
    owner_count: u32,
    flags: u32,
) -> STLedgerEntry {
    let keylet = account_keylet(raw(account));
    let mut entry = STLedgerEntry::from_type_and_key(LedgerEntryType::AccountRoot, keylet.key);
    entry.set_account_id(sf("sfAccount"), account);
    entry.set_field_u32(sf("sfSequence"), 1);
    entry.set_field_amount(
        sf("sfBalance"),
        STAmount::from_xrp_amount(XRPAmount::from_drops(balance)),
    );
    entry.set_field_u32(sf("sfOwnerCount"), owner_count);
    entry.set_field_u32(sf("sfFlags"), flags);
    entry
}

pub(super) fn lending_pseudo_root(
    account: AccountID,
    balance: i64,
    broker_id: Option<Uint256>,
    vault_id: Option<Uint256>,
) -> STLedgerEntry {
    let mut entry = account_root(account, balance, 0, 0);
    if let Some(broker_id) = broker_id {
        entry.set_field_h256(sf("sfLoanBrokerID"), broker_id);
    }
    if let Some(vault_id) = vault_id {
        entry.set_field_h256(sf("sfVaultID"), vault_id);
    }
    entry
}

fn loan(parties: Parties, asset: Asset, due: u32, grace: u32, impaired: bool) -> STLedgerEntry {
    let mut entry = STLedgerEntry::from_type_and_key(
        LedgerEntryType::Loan,
        protocol::loan_keylet_from_key(parties.loan_id).key,
    );
    entry.set_field_h256(sf("sfLoanBrokerID"), parties.broker_id);
    entry.set_account_id(sf("sfBorrower"), parties.borrower);
    entry.set_field_i32(sf("sfLoanScale"), 0);
    entry.set_field_number(sf("sfTotalValueOutstanding"), number(asset, PAYMENT));
    entry.set_field_number(sf("sfPrincipalOutstanding"), number(asset, PAYMENT));
    entry.set_field_number(sf("sfManagementFeeOutstanding"), number(asset, 0));
    entry.set_field_number(sf("sfPeriodicPayment"), number(asset, PAYMENT));
    entry.set_field_number(sf("sfLoanServiceFee"), number(asset, SERVICE_FEE));
    entry.set_field_u32(sf("sfPaymentRemaining"), 1);
    entry.set_field_u32(sf("sfNextPaymentDueDate"), due);
    entry.set_field_u32(sf("sfPreviousPaymentDueDate"), due.saturating_sub(30));
    entry.set_field_u32(sf("sfStartDate"), due.saturating_sub(30));
    entry.set_field_u32(sf("sfPaymentInterval"), 30);
    entry.set_field_u32(sf("sfGracePeriod"), grace);
    if impaired {
        entry.set_field_u32(sf("sfFlags"), protocol::lsfLoanImpaired);
    }
    entry
}

fn broker(parties: Parties, asset: Asset, cover: i64) -> STLedgerEntry {
    let mut entry = STLedgerEntry::from_type_and_key(
        LedgerEntryType::LoanBroker,
        protocol::loan_broker_keylet_from_key(parties.broker_id).key,
    );
    entry.set_field_h256(sf("sfVaultID"), parties.vault_id);
    entry.set_account_id(sf("sfOwner"), parties.broker_owner);
    entry.set_account_id(sf("sfAccount"), parties.broker_pseudo);
    entry.set_field_number(sf("sfDebtTotal"), number(asset, PAYMENT));
    entry.set_field_number(sf("sfCoverAvailable"), number(asset, cover));
    entry.set_field_u32(sf("sfCoverRateMinimum"), 100_000);
    entry.set_field_u32(sf("sfCoverRateLiquidation"), 100_000);
    entry
}

fn vault(parties: Parties, asset: Asset, loss_unrealized: i64) -> STLedgerEntry {
    let mut entry = STLedgerEntry::from_type_and_key(
        LedgerEntryType::Vault,
        protocol::vault_keylet_from_key(parties.vault_id).key,
    );
    entry.set_account_id(sf("sfOwner"), parties.broker_owner);
    entry.set_account_id(sf("sfAccount"), parties.vault_pseudo);
    entry.set_field_issue(sf("sfAsset"), STIssue::new_with_asset(sf("sfAsset"), asset));
    entry.set_field_number(sf("sfAssetsTotal"), number(asset, PAYMENT));
    entry.set_field_number(sf("sfAssetsAvailable"), number(asset, 0));
    entry.set_field_number(sf("sfLossUnrealized"), number(asset, loss_unrealized));
    entry
}

pub(super) fn mpt_issuance(issue: MPTIssue, outstanding: u64, flags: u32) -> STLedgerEntry {
    let mut entry = STLedgerEntry::from_type_and_key(
        LedgerEntryType::MPTokenIssuance,
        protocol::mpt_issuance_keylet_from_mptid(issue.mpt_id()).key,
    );
    entry.set_account_id(sf("sfIssuer"), issue.issuer());
    entry.set_field_u32(sf("sfSequence"), 1);
    entry.set_field_u64(sf("sfOutstandingAmount"), outstanding);
    entry.set_field_u64(sf("sfMaximumAmount"), 10_000);
    entry.set_field_u32(sf("sfFlags"), flags);
    entry
}

pub(super) fn iou_line(
    holder: AccountID,
    issue: protocol::Issue,
    balance: i64,
    flags: u32,
) -> STLedgerEntry {
    let keylet = protocol::line(holder, issue.account, issue.currency);
    let mut entry = STLedgerEntry::from_type_and_key(LedgerEntryType::RippleState, keylet.key);
    entry.set_field_amount(
        sf("sfBalance"),
        STAmount::from_iou_amount(
            sf("sfBalance"),
            protocol::IOUAmount::from_parts(balance, 0).expect("IOU balance"),
            issue,
        ),
    );
    entry.set_field_amount(
        sf("sfLowLimit"),
        STAmount::from_iou_amount(
            sf("sfLowLimit"),
            protocol::IOUAmount::from_parts(1_000, 0).expect("IOU limit"),
            protocol::Issue::new(issue.currency, holder),
        ),
    );
    entry.set_field_amount(
        sf("sfHighLimit"),
        STAmount::from_iou_amount(
            sf("sfHighLimit"),
            protocol::IOUAmount::from_parts(1_000, 0).expect("IOU limit"),
            protocol::Issue::new(issue.currency, issue.account),
        ),
    );
    entry.set_field_u32(sf("sfFlags"), flags);
    entry
}

pub(super) fn mptoken(
    issue: MPTIssue,
    holder: AccountID,
    amount: u64,
    flags: u32,
) -> STLedgerEntry {
    let mut entry = STLedgerEntry::from_type_and_key(
        LedgerEntryType::MPToken,
        protocol::mptoken_keylet_from_mptid(issue.mpt_id(), raw(holder)).key,
    );
    entry.set_account_id(sf("sfAccount"), holder);
    entry.set_field_h192(sf("sfMPTokenIssuanceID"), issue.mpt_id());
    entry.set_field_u64(sf("sfMPTAmount"), amount);
    entry.set_field_u32(sf("sfFlags"), flags);
    entry.set_field_u64(sf("sfOwnerNode"), 0);
    entry
}

pub(super) fn ledger(
    parties: Parties,
    asset: Asset,
    now: u32,
    cleanup: bool,
    due: u32,
    grace: u32,
    impaired: bool,
    broker_owner_balance: i64,
    cover: i64,
    extras: impl IntoIterator<Item = STLedgerEntry>,
) -> Ledger {
    let mut ledger = Ledger::new(
        LedgerHeader {
            seq: 1,
            parent_close_time: now,
            ..LedgerHeader::default()
        },
        false,
    );
    for entry in [
        account_root(parties.borrower, 10_000, 0, 0),
        account_root(parties.broker_owner, broker_owner_balance, 0, 0),
        lending_pseudo_root(parties.broker_pseudo, 10_000, Some(parties.broker_id), None),
        lending_pseudo_root(parties.vault_pseudo, 10_000, None, Some(parties.vault_id)),
        account_root(parties.issuer, 10_000, 0, 0),
        loan(parties, asset, due, grace, impaired),
        broker(parties, asset, cover),
        vault(parties, asset, if impaired { PAYMENT } else { 0 }),
    ] {
        ledger
            .raw_insert(Arc::new(entry))
            .expect("insert lending fixture");
    }
    for entry in extras {
        ledger
            .raw_insert(Arc::new(entry))
            .expect("insert lending fixture extra");
    }
    let mut features = vec![
        protocol::feature_id("LendingProtocol"),
        protocol::feature_id("SingleAssetVault"),
        protocol::feature_id("MPTokensV1"),
        protocol::feature_id("MPTokensV2"),
        protocol::feature_id("DeepFreeze"),
    ];
    if cleanup {
        features.push(protocol::feature_id("fixCleanup3_4_0"));
    }
    ledger.set_rules(protocol::Rules::new(features));
    ledger.set_fees(Fees {
        base: 10,
        reserve: 200,
        increment: 50,
    });
    ledger
}

pub(super) fn view(ledger: Ledger) -> ApplyViewImpl<Ledger> {
    ApplyViewImpl::new(Arc::new(ledger), ApplyFlags::NONE)
}

pub(super) fn loan_pay_tx(parties: Parties) -> STTx {
    STTx::new(TxType::LOAN_PAY, move |tx| {
        tx.set_account_id(sf("sfAccount"), parties.borrower);
        tx.set_field_h256(sf("sfLoanID"), parties.loan_id);
        tx.set_field_amount(
            sf("sfAmount"),
            STAmount::from_xrp_amount(XRPAmount::from_drops(PAYMENT + SERVICE_FEE)),
        );
        tx.set_field_amount(
            sf("sfFee"),
            STAmount::from_xrp_amount(XRPAmount::from_drops(10)),
        );
        tx.set_field_u32(sf("sfSequence"), 1);
    })
}

pub(super) fn loan_manage_tx(parties: Parties, flags: u32) -> STTx {
    STTx::new(TxType::LOAN_MANAGE, move |tx| {
        tx.set_account_id(sf("sfAccount"), parties.broker_owner);
        tx.set_field_h256(sf("sfLoanID"), parties.loan_id);
        tx.set_field_u32(sf("sfFlags"), flags);
        tx.set_field_amount(
            sf("sfFee"),
            STAmount::from_xrp_amount(XRPAmount::from_drops(10)),
        );
        tx.set_field_u32(sf("sfSequence"), 1);
    })
}

pub(super) fn mpt_asset(issuer: AccountID, sequence: u32) -> Asset {
    Asset::MPTIssue(MPTIssue::new(Uint192::from(protocol::make_mpt_id(
        sequence, issuer,
    ))))
}
pub(super) fn xrp_asset() -> Asset {
    Asset::Issue(xrp_issue())
}

pub(super) fn xrp_balance(view: &impl ReadView, account: AccountID) -> i64 {
    view.read(account_keylet(raw(account)))
        .expect("read account")
        .expect("account exists")
        .get_field_amount(sf("sfBalance"))
        .xrp()
        .drops()
}
