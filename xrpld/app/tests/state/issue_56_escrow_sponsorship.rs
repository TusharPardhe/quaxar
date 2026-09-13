//! Live acceptance coverage for issue #56 escrow reserve recycling.
//!
//! These fixtures intentionally use the real submit shell rather than the
//! narrowed escrow helpers: the regression is about mutation ordering and the
//! shell's transactional rollback boundary.

use std::sync::Arc;

use basics::base_uint::{Uint160, Uint192, Uint256};
use ledger::{ApplyViewImpl, Fees, Ledger, LedgerHeader, ReadView, Sandbox};
use protocol::{
    AccountID, ApplyFlags, Currency, IOUAmount, Issue, LedgerEntryType, MPTAmount, MPTIssue, Rules,
    STAmount, STLedgerEntry, STTx, Ter, TxType, XRPAmount, account_keylet, currency_from_string,
    get_field_by_symbol, owner_dir_keylet,
};
use shamap::item::SHAMapItem;
use shamap::mutation::MutableTree;
use shamap::sync::{SHAMapType, SyncState, SyncTree};
use shamap::tree_node::SHAMapNodeType;

use app::state::{
    application_root::apply_submit_transactor_shell, transactor_dispatcher::handle_real_dispatch,
};

fn sf(name: &str) -> &'static protocol::SField {
    get_field_by_symbol(name)
}

fn account(seed: u8) -> AccountID {
    AccountID::from_array([seed; 20])
}

fn raw(account: AccountID) -> Uint160 {
    Uint160::from_slice(account.data()).expect("account id width")
}

fn xrp(drops: i64) -> STAmount {
    STAmount::from_xrp_amount(XRPAmount::from_drops(drops))
}

fn account_root(account: AccountID, balance: i64, owner_count: u32) -> STLedgerEntry {
    let mut entry = STLedgerEntry::from_type_and_key(
        LedgerEntryType::AccountRoot,
        account_keylet(raw(account)).key,
    );
    entry.set_account_id(sf("sfAccount"), account);
    entry.set_field_u32(sf("sfSequence"), 1);
    entry.set_field_amount(sf("sfBalance"), xrp(balance));
    entry.set_field_u32(sf("sfOwnerCount"), owner_count);
    entry.set_field_u32(sf("sfFlags"), 0);
    entry
}

fn owner_dir(account: AccountID, children: impl IntoIterator<Item = Uint256>) -> STLedgerEntry {
    let keylet = owner_dir_keylet(raw(account));
    let mut entry = STLedgerEntry::new(keylet);
    entry.set_field_h256(sf("sfRootIndex"), keylet.key);
    entry.set_field_v256(
        sf("sfIndexes"),
        protocol::STVector256::from_values(sf("sfIndexes"), children.into_iter().collect()),
    );
    entry
}

fn ledger(entries: Vec<STLedgerEntry>, features: impl IntoIterator<Item = &'static str>) -> Ledger {
    let mut state = MutableTree::new(1);
    for entry in entries {
        state
            .add_item(
                SHAMapNodeType::AccountState,
                SHAMapItem::new(*entry.key(), entry.get_serializer().data().to_vec()),
            )
            .expect("fixture state insertion");
    }
    let mut ledger = Ledger::from_maps(
        LedgerHeader {
            seq: 10,
            parent_close_time: 2,
            ..LedgerHeader::default()
        },
        SyncTree::from_root_with_type(
            state.root(),
            SHAMapType::State,
            false,
            1,
            SyncState::Immutable,
        ),
        SyncTree::new_with_type(SHAMapType::Transaction, false, 1),
    );
    ledger.set_fees(Fees {
        base: 10,
        reserve: 200,
        increment: 50,
    });
    ledger.set_rules(Rules::new(features.into_iter().map(protocol::feature_id)));
    ledger
}

fn finish(submitter: AccountID, owner: AccountID) -> STTx {
    STTx::new(TxType::ESCROW_FINISH, move |tx| {
        tx.set_account_id(sf("sfAccount"), submitter);
        tx.set_account_id(sf("sfOwner"), owner);
        tx.set_field_u32(sf("sfOfferSequence"), 1);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    })
}

fn cancel(owner: AccountID) -> STTx {
    STTx::new(TxType::ESCROW_CANCEL, move |tx| {
        tx.set_account_id(sf("sfAccount"), owner);
        tx.set_account_id(sf("sfOwner"), owner);
        tx.set_field_u32(sf("sfOfferSequence"), 1);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    })
}

fn escrow(
    owner: AccountID,
    destination: AccountID,
    amount: STAmount,
    cancel: bool,
) -> STLedgerEntry {
    let keylet = protocol::escrow_keylet(raw(owner), 1);
    let mut entry = STLedgerEntry::from_type_and_key(LedgerEntryType::Escrow, keylet.key);
    entry.set_account_id(sf("sfAccount"), owner);
    entry.set_account_id(sf("sfDestination"), destination);
    entry.set_field_amount(sf("sfAmount"), amount);
    entry.set_field_u64(sf("sfOwnerNode"), 0);
    if cancel {
        entry.set_field_u32(sf("sfCancelAfter"), 1);
    }
    entry
}

fn mpt_issue(issuer: AccountID) -> MPTIssue {
    MPTIssue::new(Uint192::from(protocol::make_mpt_id(1, issuer)))
}

fn mpt_issuance(issue: MPTIssue, locked: u64) -> STLedgerEntry {
    let mut entry = STLedgerEntry::from_type_and_key(
        LedgerEntryType::MPTokenIssuance,
        protocol::mpt_issuance_keylet_from_mptid(issue.mpt_id()).key,
    );
    entry.set_account_id(sf("sfIssuer"), issue.issuer());
    entry.set_field_u32(sf("sfSequence"), 1);
    entry.set_field_u64(sf("sfOutstandingAmount"), 10);
    entry.set_field_u64(sf("sfMaximumAmount"), 1_000);
    entry.set_field_u32(sf("sfFlags"), protocol::lsfMPTCanTransfer);
    entry.set_field_u64(sf("sfLockedAmount"), locked);
    entry
}

fn mpt_holding(holder: AccountID, issue: MPTIssue, amount: u64, locked: u64) -> STLedgerEntry {
    let mut entry = STLedgerEntry::from_type_and_key(
        LedgerEntryType::MPToken,
        protocol::mptoken_keylet_from_mptid(issue.mpt_id(), raw(holder)).key,
    );
    entry.set_account_id(sf("sfAccount"), holder);
    entry.set_field_h192(sf("sfMPTokenIssuanceID"), issue.mpt_id());
    entry.set_field_u64(sf("sfMPTAmount"), amount);
    entry.set_field_u64(sf("sfLockedAmount"), locked);
    entry.set_field_u64(sf("sfOwnerNode"), 0);
    entry
}

fn iou_amount(issuer: AccountID) -> STAmount {
    STAmount::from_iou_amount(
        sf("sfAmount"),
        IOUAmount::from_parts(10, 0).expect("IOU amount"),
        Issue::new(currency_from_string("USD"), issuer),
    )
}

fn mpt_amount(issue: MPTIssue) -> STAmount {
    STAmount::from_mpt_amount(sf("sfAmount"), MPTAmount::from_value(10), issue)
}

fn root(view: &impl ReadView, account: AccountID) -> Arc<STLedgerEntry> {
    view.read(account_keylet(raw(account)))
        .expect("account read")
        .expect("account exists")
}

fn balance(view: &impl ReadView, account: AccountID) -> i64 {
    root(view, account)
        .get_field_amount(sf("sfBalance"))
        .xrp()
        .drops()
}

fn owner_count(view: &impl ReadView, account: AccountID) -> u32 {
    root(view, account).get_field_u32(sf("sfOwnerCount"))
}

fn assert_unchanged_escrow_state(
    view: &impl ReadView,
    owner: AccountID,
    escrow_key: protocol::Keylet,
    balance_before: i64,
    owner_count_before: u32,
) {
    assert_eq!(
        balance(view, owner),
        balance_before,
        "failed delivery must not move XRP"
    );
    assert_eq!(
        owner_count(view, owner),
        owner_count_before,
        "failed delivery must not change owner count"
    );
    assert!(
        view.read(escrow_key).expect("escrow read").is_some(),
        "failed delivery must retain escrow"
    );
}

#[test]
fn issue_56_finish_reserve_boundaries_follow_sponsor_or_cleanup_ordering() {
    // Token escrow creation can delete the owner's last IOU line. Finishing a
    // self-directed escrow then has to recreate that line. Only Sponsor or
    // cleanup may release the escrow owner reserve before this new holding is
    // checked. Balances include the ten-drop submit fee.
    for sponsor in [false, true] {
        for cleanup in [false, true] {
            for (label, owner_balance, expected_boundary) in [
                ("below", 249, Ter::TEC_NO_LINE_INSUF_RESERVE),
                ("exact", 250, Ter::TES_SUCCESS),
                ("above", 251, Ter::TES_SUCCESS),
            ] {
                let owner = account(0x11);
                let issuer = account(0x13);
                let escrow_key = protocol::escrow_keylet(raw(owner), 1);
                let mut issuer_root = account_root(issuer, 1_000, 0);
                issuer_root.set_field_u32(sf("sfFlags"), protocol::lsfDefaultRipple);
                let mut features = vec!["TokenEscrow"];
                if sponsor {
                    features.push("Sponsor");
                }
                if cleanup {
                    features.push("fixCleanup3_4_0");
                }
                let mut view = ApplyViewImpl::new(
                    Arc::new(ledger(
                        vec![
                            account_root(owner, owner_balance, 1),
                            issuer_root,
                            owner_dir(owner, [escrow_key.key]),
                            escrow(owner, owner, iou_amount(issuer), false),
                        ],
                        features,
                    )),
                    ApplyFlags::NONE,
                );
                let expected = if sponsor || cleanup {
                    expected_boundary
                } else {
                    Ter::TEC_NO_LINE_INSUF_RESERVE
                };
                let result = apply_submit_transactor_shell(
                    &mut view,
                    &finish(owner, owner),
                    TxType::ESCROW_FINISH,
                );
                assert_eq!(
                    result, expected,
                    "sponsor={sponsor} cleanup={cleanup} {label} reserve"
                );
                if result == Ter::TES_SUCCESS {
                    assert!(
                        view.read(protocol::line(owner, issuer, currency_from_string("USD")))
                            .expect("IOU line read")
                            .is_some()
                    );
                    assert!(view.read(escrow_key).expect("escrow read").is_none());
                    assert_eq!(
                        owner_count(&view, owner),
                        1,
                        "the released escrow reserve funds the new IOU line"
                    );
                }
            }
        }
    }
}

#[test]
fn issue_56_finish_xrp_iou_and_mpt_delivery_preserve_asset_specific_state() {
    // XRP, IOU, and MPT all take the real finish dispatcher.  Token fixtures
    // deliberately omit the destination holding so IOU/MPT creation is live.
    for (kind, amount) in [("xrp", None), ("iou", Some(0)), ("mpt", Some(1))] {
        let owner = account(if kind == "xrp" { 0x21 } else { 0x31 });
        let destination = account(if kind == "xrp" { 0x22 } else { 0x32 });
        let issuer = account(if kind == "xrp" { 0x23 } else { 0x33 });
        let mpt = mpt_issue(issuer);
        let escrow_amount = match amount {
            None => xrp(10),
            Some(0) => iou_amount(issuer),
            Some(_) => mpt_amount(mpt),
        };
        let escrow_key = protocol::escrow_keylet(raw(owner), 1);
        let mut issuer_root = account_root(issuer, 1_000, 0);
        issuer_root.set_field_u32(sf("sfFlags"), protocol::lsfDefaultRipple);
        let mut entries = vec![
            account_root(owner, 1_000, 1),
            account_root(destination, 1_000, 0),
            issuer_root,
            owner_dir(owner, [escrow_key.key]),
            escrow(owner, destination, escrow_amount, false),
        ];
        let mut features = vec!["TokenEscrow", "fixTokenEscrowV1"];
        if kind == "mpt" {
            features.push("MPTokensV1");
            entries.push(mpt_issuance(mpt, 10));
            entries.push(mpt_holding(owner, mpt, 0, 10));
        }
        let mut view = ApplyViewImpl::new(Arc::new(ledger(entries, features)), ApplyFlags::NONE);
        assert_eq!(
            apply_submit_transactor_shell(
                &mut view,
                &finish(destination, owner),
                TxType::ESCROW_FINISH
            ),
            Ter::TES_SUCCESS,
            "{kind} finish delivery"
        );
        assert!(view.read(escrow_key).expect("escrow read").is_none());
        assert_eq!(
            owner_count(&view, owner),
            0,
            "{kind} finish releases escrow reserve"
        );
        match kind {
            "xrp" => assert_eq!(balance(&view, destination), 1_000),
            "iou" => assert!(
                view.read(protocol::line(
                    destination,
                    issuer,
                    Currency::from(currency_from_string("USD"))
                ))
                .expect("IOU line read")
                .is_some()
            ),
            "mpt" => assert_eq!(
                view.read(protocol::mptoken_keylet_from_mptid(
                    mpt.mpt_id(),
                    raw(destination)
                ))
                .expect("MPT holding read")
                .expect("MPT holding created")
                .get_field_u64(sf("sfMPTAmount")),
                10
            ),
            _ => unreachable!(),
        }
    }
}

#[test]
fn issue_56_cancel_reserve_boundaries_are_cleanup_only_for_sponsored_and_unsponsored() {
    // Cancel returns a token to the owner and may need to recreate the owner's
    // deleted IOU line. Sponsor deliberately does not change Cancel ordering;
    // only cleanup releases the escrow reserve before the return.
    for sponsor in [false, true] {
        for cleanup in [false, true] {
            for (label, owner_balance, cleanup_expected) in [
                ("below", 259, Ter::TEC_NO_LINE_INSUF_RESERVE),
                ("exact", 260, Ter::TES_SUCCESS),
                ("above", 261, Ter::TES_SUCCESS),
            ] {
                let owner = account(0x41);
                let issuer = account(0x43);
                let escrow_key = protocol::escrow_keylet(raw(owner), 1);
                let mut issuer_root = account_root(issuer, 1_000, 0);
                issuer_root.set_field_u32(sf("sfFlags"), protocol::lsfDefaultRipple);
                let mut features = vec!["TokenEscrow"];
                if sponsor {
                    features.push("Sponsor");
                }
                if cleanup {
                    features.push("fixCleanup3_4_0");
                }
                let mut view = ApplyViewImpl::new(
                    Arc::new(ledger(
                        vec![
                            account_root(owner, owner_balance, 1),
                            issuer_root,
                            owner_dir(owner, [escrow_key.key]),
                            escrow(owner, owner, iou_amount(issuer), true),
                        ],
                        features,
                    )),
                    ApplyFlags::NONE,
                );
                let expected = if cleanup {
                    cleanup_expected
                } else {
                    Ter::TEC_NO_LINE_INSUF_RESERVE
                };
                let result =
                    apply_submit_transactor_shell(&mut view, &cancel(owner), TxType::ESCROW_CANCEL);
                assert_eq!(
                    result, expected,
                    "sponsor={sponsor} cleanup={cleanup} {label} reserve"
                );
                if result == Ter::TES_SUCCESS {
                    assert!(
                        view.read(protocol::line(owner, issuer, currency_from_string("USD")))
                            .expect("IOU line read")
                            .is_some()
                    );
                    assert!(view.read(escrow_key).expect("escrow read").is_none());
                    assert_eq!(owner_count(&view, owner), 1);
                }
            }
        }
    }
}

#[test]
fn issue_56_failed_delivery_rolls_back_escrow_owner_count_balances_and_holdings() {
    // A missing destination holding below reserve forces MPT delivery failure
    // *after* finish removes owner-directory entries.  The submit shell must
    // restore the complete prior state for every ordering era.
    for sponsor in [false, true] {
        for cleanup in [false, true] {
            let owner = account(0x51);
            let destination = account(0x52);
            let issuer = account(0x53);
            let issue = mpt_issue(issuer);
            let escrow_key = protocol::escrow_keylet(raw(owner), 1);
            let mut features = vec!["MPTokensV1", "TokenEscrow", "fixTokenEscrowV1"];
            if sponsor {
                features.push("Sponsor");
            }
            if cleanup {
                features.push("fixCleanup3_4_0");
            }
            let ledger = ledger(
                vec![
                    account_root(owner, 1_000, 1),
                    account_root(destination, 249, 0),
                    account_root(issuer, 1_000, 0),
                    owner_dir(owner, [escrow_key.key]),
                    escrow(owner, destination, mpt_amount(issue), false),
                    mpt_issuance(issue, 10),
                    mpt_holding(owner, issue, 0, 10),
                ],
                features,
            );
            let owner_before = balance(&ledger, owner);
            let destination_before = balance(&ledger, destination);
            let owner_count_before = owner_count(&ledger, owner);
            let source_holding_before = ledger
                .read(protocol::mptoken_keylet_from_mptid(
                    issue.mpt_id(),
                    raw(owner),
                ))
                .expect("source holding read")
                .expect("source holding")
                .get_serializer()
                .data()
                .to_vec();
            let mut view = Sandbox::new(Arc::new(ledger), ApplyFlags::NONE);
            assert_eq!(
                apply_submit_transactor_shell(
                    &mut view,
                    &finish(destination, owner),
                    TxType::ESCROW_FINISH
                ),
                Ter::TEC_INSUFFICIENT_RESERVE,
                "sponsor={sponsor} cleanup={cleanup} failure"
            );
            assert_unchanged_escrow_state(
                &view,
                owner,
                escrow_key,
                owner_before,
                owner_count_before,
            );
            // tec results charge the submitting destination its fee, but every
            // escrow-delivery mutation must roll back exactly.
            assert_eq!(balance(&view, destination), destination_before - 10);
            assert!(
                view.read(protocol::mptoken_keylet_from_mptid(
                    issue.mpt_id(),
                    raw(destination)
                ))
                .expect("destination holding read")
                .is_none()
            );
            assert_eq!(
                view.read(protocol::mptoken_keylet_from_mptid(
                    issue.mpt_id(),
                    raw(owner)
                ))
                .expect("source holding read")
                .expect("source holding")
                .get_serializer()
                .data()
                .to_vec(),
                source_holding_before
            );
        }
    }
}

#[test]
fn issue_56_sponsored_and_unsponsored_mpt_finish_create_the_expected_holding() {
    // Exercise the actual sponsorship object and transaction fields as well as
    // the unsponsored control. This is separate from the IOU reserve-ordering
    // matrix because a reserve sponsor intentionally changes who funds a new
    // destination MPToken holding.
    for sponsored in [false, true] {
        let owner = account(0x61);
        let destination = account(0x62);
        let issuer = account(0x63);
        let sponsor = account(0x64);
        let issue = mpt_issue(issuer);
        let escrow_key = protocol::escrow_keylet(raw(owner), 1);
        let mut entries = vec![
            account_root(owner, 1_000, 1),
            account_root(destination, if sponsored { 0 } else { 260 }, 0),
            account_root(issuer, 1_000, 0),
            owner_dir(owner, [escrow_key.key]),
            escrow(owner, destination, mpt_amount(issue), false),
            mpt_issuance(issue, 10),
            mpt_holding(owner, issue, 0, 10),
        ];
        if sponsored {
            let sponsorship_key = protocol::sponsorship_keylet(raw(sponsor), raw(destination));
            entries.push(account_root(sponsor, 1_000, 0));
            entries.push(
                protocol::SponsorshipBuilder::new(
                    sponsor,
                    destination,
                    0,
                    0,
                    Uint256::default(),
                    0,
                )
                .set_remaining_owner_count(2)
                .build(sponsorship_key.key)
                .get_sle()
                .as_ref()
                .clone(),
            );
        }
        let mut view = ApplyViewImpl::new(
            Arc::new(ledger(
                entries,
                ["MPTokensV1", "TokenEscrow", "fixTokenEscrowV1", "Sponsor"],
            )),
            ApplyFlags::NONE,
        );
        let mut tx = finish(destination, owner);
        if sponsored {
            tx.set_account_id(sf("sfSponsor"), sponsor);
            tx.set_field_u32(sf("sfSponsorFlags"), ledger::SPF_SPONSOR_RESERVE);
        }
        let pre_fee_balance = balance(&view, destination);
        assert_eq!(
            handle_real_dispatch(&mut view, &tx, TxType::ESCROW_FINISH, Some(pre_fee_balance)),
            Ter::TES_SUCCESS,
            "sponsored={sponsored}",
        );
        let holding = view
            .read(protocol::mptoken_keylet_from_mptid(
                issue.mpt_id(),
                raw(destination),
            ))
            .expect("destination holding read")
            .expect("destination holding created");
        assert_eq!(holding.get_field_u64(sf("sfMPTAmount")), 10);
        assert_eq!(holding.is_field_present(sf("sfSponsor")), sponsored);
        if sponsored {
            assert_eq!(holding.get_account_id(sf("sfSponsor")), sponsor);
            assert_eq!(
                root(&view, destination).get_field_u32(sf("sfSponsoredOwnerCount")),
                1
            );
            assert_eq!(
                root(&view, sponsor).get_field_u32(sf("sfSponsoringOwnerCount")),
                1
            );
        } else {
            assert_eq!(owner_count(&view, destination), 1);
        }
    }
}

#[test]
fn issue_56_cancel_xrp_iou_and_mpt_return_preserve_asset_specific_state() {
    for kind in ["xrp", "iou", "mpt"] {
        let owner = account(if kind == "xrp" { 0x71 } else { 0x81 });
        let issuer = account(if kind == "xrp" { 0x73 } else { 0x83 });
        let issue = mpt_issue(issuer);
        let escrow_key = protocol::escrow_keylet(raw(owner), 1);
        let amount = match kind {
            "xrp" => xrp(10),
            "iou" => iou_amount(issuer),
            "mpt" => mpt_amount(issue),
            _ => unreachable!(),
        };
        let mut issuer_root = account_root(issuer, 1_000, 0);
        issuer_root.set_field_u32(sf("sfFlags"), protocol::lsfDefaultRipple);
        let mut entries = vec![
            account_root(
                owner,
                if kind == "iou" { 260 } else { 1_000 },
                if kind == "mpt" { 2 } else { 1 },
            ),
            issuer_root,
            owner_dir(owner, [escrow_key.key]),
            escrow(owner, owner, amount, true),
        ];
        let mut features = vec!["TokenEscrow", "fixCleanup3_4_0"];
        if kind == "mpt" {
            features.extend(["MPTokensV1", "fixTokenEscrowV1"]);
            entries.push(mpt_issuance(issue, 10));
            entries.push(mpt_holding(owner, issue, 0, 10));
        }
        let mut view = ApplyViewImpl::new(Arc::new(ledger(entries, features)), ApplyFlags::NONE);
        assert_eq!(
            apply_submit_transactor_shell(&mut view, &cancel(owner), TxType::ESCROW_CANCEL),
            Ter::TES_SUCCESS,
            "{kind} cancel return",
        );
        assert!(view.read(escrow_key).expect("escrow read").is_none());
        assert_eq!(owner_count(&view, owner), if kind == "xrp" { 0 } else { 1 });
        match kind {
            "xrp" => assert_eq!(balance(&view, owner), 1_000),
            "iou" => assert!(
                view.read(protocol::line(owner, issuer, currency_from_string("USD")))
                    .expect("IOU return line read")
                    .is_some()
            ),
            "mpt" => assert_eq!(
                view.read(protocol::mptoken_keylet_from_mptid(
                    issue.mpt_id(),
                    raw(owner)
                ))
                .expect("MPT return holding read")
                .expect("MPT return holding")
                .get_field_u64(sf("sfMPTAmount")),
                10
            ),
            _ => unreachable!(),
        }
    }
}
