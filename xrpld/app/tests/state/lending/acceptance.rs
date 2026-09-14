use app::state::{
    application_root::apply_submit_transactor_shell,
    lending::{apply_loan_manage, apply_loan_pay},
};
use basics::base_uint::Uint256;
use ledger::{ApplyViewImpl, Ledger, ReadView};
use protocol::{AccountID, Asset, STAmount, STLedgerEntry, STTx, Ter, TxType, XRPAmount};

use super::fixtures::*;

#[test]
fn issue_55_loan_pay_xrp_fee_recipient_reserve_conservation_tracks_cleanup_3_4() {
    // The owner is the fee recipient (cover is exactly at the zero threshold).
    // Prior to fixCleanup3_4_0 the conservation sample used xrpLiquid, while
    // the corrected branch samples raw AccountRoot balances.
    for cleanup in [false, true] {
        for (label, recipient_balance, expected) in [
            (
                "below reserve",
                199,
                if cleanup {
                    Ter::TES_SUCCESS
                } else {
                    Ter::TEF_INTERNAL
                },
            ),
            ("exactly at reserve", 200, Ter::TES_SUCCESS),
            ("above reserve", 201, Ter::TES_SUCCESS),
        ] {
            let parties = parties(if cleanup { 0x20 } else { 0x10 });
            let mut view = view(ledger(
                parties,
                xrp_asset(),
                0,
                cleanup,
                DUE,
                GRACE,
                false,
                recipient_balance,
                PAYMENT,
                [],
            ));
            let borrower_before = xrp_balance(&view, parties.borrower);
            let recipient_before = xrp_balance(&view, parties.broker_owner);

            assert_eq!(
                apply_loan_pay(&mut view, &loan_pay_tx(parties)),
                expected,
                "cleanup={cleanup} fee recipient {label}"
            );
            if expected == Ter::TES_SUCCESS {
                assert_eq!(
                    xrp_balance(&view, parties.borrower),
                    borrower_before - PAYMENT - SERVICE_FEE
                );
                assert_eq!(
                    xrp_balance(&view, parties.broker_owner),
                    recipient_before + SERVICE_FEE
                );
            }
        }
    }
}

#[test]
fn issue_55_loan_manage_mpt_default_allows_only_cleanup_pseudo_legs_and_keeps_auth() {
    let locked = protocol::lsfMPTLocked | protocol::lsfMPTCanTransfer;
    let auth_required = protocol::lsfMPTCanTransfer | protocol::lsfMPTRequireAuth;

    for cleanup in [false, true] {
        for (label, issuance_flags, broker_flags, vault_flags, expected) in [
            (
                "issuance locked",
                locked,
                0,
                0,
                if cleanup {
                    Ter::TES_SUCCESS
                } else {
                    Ter::TEC_LOCKED
                },
            ),
            (
                "broker holding locked",
                protocol::lsfMPTCanTransfer,
                protocol::lsfMPTLocked,
                0,
                if cleanup {
                    Ter::TES_SUCCESS
                } else {
                    Ter::TEC_LOCKED
                },
            ),
            (
                "vault holding locked",
                protocol::lsfMPTCanTransfer,
                0,
                protocol::lsfMPTLocked,
                if cleanup {
                    Ter::TES_SUCCESS
                } else {
                    Ter::TEC_LOCKED
                },
            ),
        ] {
            let parties = parties(if cleanup { 0x50 } else { 0x40 });
            let Asset::MPTIssue(issue) = mpt_asset(parties.issuer, 1) else {
                unreachable!()
            };
            let extras = [
                mpt_issuance(issue, PAYMENT as u64, issuance_flags),
                mptoken(issue, parties.broker_pseudo, PAYMENT as u64, broker_flags),
                mptoken(issue, parties.vault_pseudo, 0, vault_flags),
                // An unrelated locked holder is not a transfer leg and cannot
                // receive the default exemption accidentally.
                mptoken(issue, account(0xE1), 0, protocol::lsfMPTLocked),
            ];
            let mut view = view(ledger(
                parties,
                Asset::MPTIssue(issue),
                DUE + GRACE + 1,
                cleanup,
                DUE,
                GRACE,
                false,
                10_000,
                PAYMENT,
                extras,
            ));
            assert_eq!(
                apply_loan_manage(&mut view, &loan_manage_tx(parties, protocol::tfLoanDefault)),
                expected,
                "cleanup={cleanup} {label}"
            );
        }

        // Authorization is deliberately not part of the freeze exemption.
        let parties = parties(if cleanup { 0x70 } else { 0x60 });
        let Asset::MPTIssue(issue) = mpt_asset(parties.issuer, 1) else {
            unreachable!()
        };
        let extras = [
            mpt_issuance(issue, PAYMENT as u64, auth_required),
            mptoken(
                issue,
                parties.broker_pseudo,
                PAYMENT as u64,
                protocol::lsfMPTAuthorized,
            ),
            mptoken(issue, parties.vault_pseudo, 0, 0),
        ];
        let mut view = view(ledger(
            parties,
            Asset::MPTIssue(issue),
            DUE + GRACE + 1,
            cleanup,
            DUE,
            GRACE,
            false,
            10_000,
            PAYMENT,
            extras,
        ));
        assert_eq!(
            apply_submit_transactor_shell(
                &mut view,
                &loan_manage_tx(parties, protocol::tfLoanDefault),
                TxType::LOAN_MANAGE,
            ),
            Ter::TEC_INVARIANT_FAILED,
            "cleanup={cleanup} must retain MPT destination authorization"
        );
    }
}

#[test]
fn issue_55_loan_manage_default_frozen_iou_is_limited_to_broker_vault_legs() {
    for cleanup in [false, true] {
        let parties = parties(if cleanup { 0x38 } else { 0x28 });
        let issue = protocol::Issue::new(protocol::currency_from_string("USD"), parties.issuer);
        let frozen = protocol::lsfHighFreeze | protocol::lsfHighDeepFreeze;
        let mut view = view(ledger(
            parties,
            Asset::Issue(issue),
            DUE + GRACE + 1,
            cleanup,
            DUE,
            GRACE,
            false,
            10_000,
            PAYMENT,
            [
                iou_line(parties.broker_pseudo, issue, PAYMENT, frozen),
                iou_line(parties.vault_pseudo, issue, 0, frozen),
            ],
        ));
        assert_eq!(
            apply_loan_manage(&mut view, &loan_manage_tx(parties, protocol::tfLoanDefault)),
            Ter::TES_SUCCESS,
            "cleanup={cleanup}: the direct LoanManage broker/vault frozen IOU legs settle"
        );
    }
}

#[test]
fn issue_55_loan_manage_and_pay_due_grace_boundaries_track_both_eras() {
    for cleanup in [false, true] {
        // A regular payment is on time through the due instant only after the
        // fix; a late payment is intentionally not used so the result pins the
        // expiration gate itself.
        for (label, now, expected) in [
            ("before due", DUE - 1, Ter::TES_SUCCESS),
            (
                "at due",
                DUE,
                if cleanup {
                    Ter::TES_SUCCESS
                } else {
                    Ter::TEC_EXPIRED
                },
            ),
            ("after due", DUE + 1, Ter::TEC_EXPIRED),
        ] {
            let parties = parties(if cleanup { 0x90 } else { 0x80 });
            let mut view = view(ledger(
                parties,
                xrp_asset(),
                now,
                cleanup,
                DUE,
                GRACE,
                false,
                10_000,
                0,
                [],
            ));
            assert_eq!(
                apply_loan_pay(&mut view, &loan_pay_tx(parties)),
                expected,
                "cleanup={cleanup} pay {label}"
            );
        }

        for (label, now, expected) in [
            (
                "before due",
                DUE - 1,
                if cleanup {
                    Ter::TEC_TOO_SOON
                } else {
                    Ter::TES_SUCCESS
                },
            ),
            (
                "at due",
                DUE,
                if cleanup {
                    Ter::TEC_TOO_SOON
                } else {
                    Ter::TES_SUCCESS
                },
            ),
            ("after due", DUE + 1, Ter::TES_SUCCESS),
        ] {
            let parties = parties(if cleanup { 0xB0 } else { 0xA0 });
            let mut view = view(ledger(
                parties,
                xrp_asset(),
                now,
                cleanup,
                DUE,
                GRACE,
                false,
                10_000,
                0,
                [],
            ));
            assert_eq!(
                apply_loan_manage(&mut view, &loan_manage_tx(parties, protocol::tfLoanImpair)),
                expected,
                "cleanup={cleanup} impair {label}"
            );
            if expected == Ter::TES_SUCCESS && cleanup {
                let loan = view
                    .read(protocol::loan_keylet_from_key(parties.loan_id))
                    .expect("loan read")
                    .expect("loan");
                assert_eq!(
                    loan.get_field_u32(sf("sfNextPaymentDueDate")),
                    DUE,
                    "post-fix impairment must not rewrite next due"
                );
            }
        }

        for (label, now, expected) in [
            ("before grace", DUE + GRACE - 1, Ter::TEC_TOO_SOON),
            (
                "at grace",
                DUE + GRACE,
                if cleanup {
                    Ter::TEC_TOO_SOON
                } else {
                    Ter::TES_SUCCESS
                },
            ),
            ("after grace", DUE + GRACE + 1, Ter::TES_SUCCESS),
        ] {
            let parties = parties(if cleanup { 0xD0 } else { 0xC0 });
            let mut view = view(ledger(
                parties,
                xrp_asset(),
                now,
                cleanup,
                DUE,
                GRACE,
                false,
                10_000,
                0,
                [],
            ));
            assert_eq!(
                apply_loan_manage(&mut view, &loan_manage_tx(parties, protocol::tfLoanDefault)),
                expected,
                "cleanup={cleanup} default {label}"
            );
        }

        for (label, now) in [
            ("before due", DUE - 1),
            ("at due", DUE),
            ("after due", DUE + 1),
        ] {
            let parties = parties(if cleanup { 0xF0 } else { 0xE0 });
            let mut view = view(ledger(
                parties,
                xrp_asset(),
                now,
                cleanup,
                DUE,
                GRACE,
                true,
                10_000,
                0,
                [],
            ));
            assert_eq!(
                apply_loan_manage(
                    &mut view,
                    &loan_manage_tx(parties, protocol::tfLoanUnimpair)
                ),
                Ter::TES_SUCCESS,
                "cleanup={cleanup} unimpair {label}"
            );
            let loan = view
                .read(protocol::loan_keylet_from_key(parties.loan_id))
                .expect("loan read")
                .expect("loan");
            if cleanup {
                assert_eq!(
                    loan.get_field_u32(sf("sfNextPaymentDueDate")),
                    DUE,
                    "post-fix unimpair must not rewrite next due ({label})"
                );
            }
        }
    }
}

// #54: LoanBrokerCoverWithdraw must consume the same CredentialIDs result
// ordering as its immutable preclaim before the live cover mutation runs.
fn issue_54_cover_withdraw_tx(
    parties: Parties,
    destination: AccountID,
    credential_ids: Vec<Uint256>,
) -> STTx {
    STTx::new(TxType::LOAN_BROKER_COVER_WITHDRAW, move |tx| {
        tx.set_account_id(sf("sfAccount"), parties.broker_owner);
        tx.set_account_id(sf("sfDestination"), destination);
        tx.set_field_h256(sf("sfLoanBrokerID"), parties.broker_id);
        tx.set_field_amount(
            sf("sfAmount"),
            STAmount::from_xrp_amount(XRPAmount::from_drops(10)),
        );
        tx.set_field_v256(
            sf("sfCredentialIDs"),
            protocol::STVector256::from_values(sf("sfCredentialIDs"), credential_ids),
        );
        tx.set_field_amount(
            sf("sfFee"),
            STAmount::from_xrp_amount(XRPAmount::from_drops(10)),
        );
        tx.set_field_u32(sf("sfSequence"), 1);
    })
}

fn issue_54_credential(
    subject: AccountID,
    issuer: AccountID,
    credential_type: &[u8],
    expiration: Option<u32>,
) -> STLedgerEntry {
    let mut entry = STLedgerEntry::from_type_and_key(
        protocol::LedgerEntryType::Credential,
        protocol::credential_keylet(raw(subject), raw(issuer), credential_type).key,
    );
    entry.set_account_id(sf("sfSubject"), subject);
    entry.set_account_id(sf("sfIssuer"), issuer);
    entry.set_field_vl(sf("sfCredentialType"), credential_type);
    entry.set_field_u64(sf("sfIssuerNode"), 0);
    entry.set_field_u64(sf("sfSubjectNode"), 0);
    entry.set_field_u32(sf("sfFlags"), protocol::lsfAccepted);
    if let Some(expiration) = expiration {
        entry.set_field_u32(sf("sfExpiration"), expiration);
    }
    entry
}

fn issue_54_credential_preauth(
    destination: AccountID,
    issuer: AccountID,
    credential_type: &[u8],
) -> STLedgerEntry {
    let hash = protocol::sha512_half_slices(&[issuer.data(), credential_type]);
    let mut entry = STLedgerEntry::new(protocol::deposit_preauth_credentials_keylet(
        raw(destination),
        &[hash],
    ));
    entry.set_account_id(sf("sfAccount"), destination);
    let mut authorized = protocol::STObject::make_inner_object(sf("sfCredential"));
    authorized.set_account_id(sf("sfIssuer"), issuer);
    authorized.set_field_vl(sf("sfCredentialType"), credential_type);
    let mut credentials = protocol::STArray::new(sf("sfAuthorizeCredentials"));
    credentials.push_back(authorized);
    entry.set_field_array(sf("sfAuthorizeCredentials"), credentials);
    entry.set_field_u64(sf("sfOwnerNode"), 0);
    entry
}

fn issue_54_cover_fixture(
    credential_expiration: Option<Option<u32>>,
    authorize: bool,
) -> (ApplyViewImpl<Ledger>, Parties, AccountID, Uint256) {
    let parties = parties(0x54);
    let destination = account(0x64);
    let issuer = account(0x74);
    let credential_type = b"issue-54-cover";
    let credential_id =
        protocol::credential_keylet(raw(parties.broker_owner), raw(issuer), credential_type).key;
    let mut extras = vec![account_root(
        destination,
        10_000,
        0,
        protocol::lsfDepositAuth,
    )];
    if let Some(expiration) = credential_expiration {
        extras.push(issue_54_credential(
            parties.broker_owner,
            issuer,
            credential_type,
            expiration,
        ));
    }
    if authorize {
        extras.push(issue_54_credential_preauth(
            destination,
            issuer,
            credential_type,
        ));
    }
    let view = view(ledger(
        parties,
        xrp_asset(),
        0,
        true,
        DUE,
        GRACE,
        false,
        10_000,
        PAYMENT + 10,
        extras,
    ));
    (view, parties, destination, credential_id)
}

fn issue_54_cover_preclaim(view: &impl ReadView, tx: &STTx) -> Ter {
    let shape = ledger::credential_helpers::check_fields(tx, &view.rules());
    if shape != Ter::TES_SUCCESS {
        return shape;
    }
    tx::run_loan_read_view_preclaim(view, tx, TxType::LOAN_BROKER_COVER_WITHDRAW)
        .expect("LoanBrokerCoverWithdraw must have a typed preclaim")
}

fn issue_54_cover_snapshot(
    view: &impl ReadView,
    parties: Parties,
    destination: AccountID,
) -> (Vec<u8>, i64, i64) {
    let broker = view
        .read(protocol::loan_broker_keylet_from_key(parties.broker_id))
        .expect("broker read")
        .expect("broker exists")
        .get_serializer()
        .data()
        .to_vec();
    (
        broker,
        xrp_balance(view, parties.broker_pseudo),
        xrp_balance(view, destination),
    )
}

#[test]
fn issue_54_loan_broker_cover_withdraw_credential_ids_matrix() {
    for (label, expiration, authorize, ids, expected) in [
        ("valid", Some(None), true, 0_u8, Ter::TES_SUCCESS),
        (
            "invalid or missing",
            None,
            true,
            1,
            Ter::TEC_BAD_CREDENTIALS,
        ),
        // Credentials used by DepositPreauth remain valid after their optional
        // expiration; expiry is a permissioned-domain membership concern.
        ("expired", Some(Some(0)), true, 0, Ter::TES_SUCCESS),
        ("duplicate", Some(None), true, 2, Ter::TEM_MALFORMED),
        (
            "valid but unauthorized",
            Some(None),
            false,
            0,
            Ter::TEC_NO_PERMISSION,
        ),
    ] {
        let (mut view, parties, destination, credential_id) =
            issue_54_cover_fixture(expiration, authorize);
        let supplied = match ids {
            0 => vec![credential_id],
            1 => vec![Uint256::from_array([0xE1; 32])],
            2 => vec![credential_id, credential_id],
            _ => unreachable!(),
        };
        let tx = issue_54_cover_withdraw_tx(parties, destination, supplied);
        let before = issue_54_cover_snapshot(&view, parties, destination);

        assert_eq!(issue_54_cover_preclaim(&view, &tx), expected, "{label}");
        if expected == Ter::TES_SUCCESS {
            assert_eq!(
                app::state::lending::apply_loan_broker_cover_withdraw(&mut view, &tx, 10_000),
                Ter::TES_SUCCESS,
                "{label} live apply"
            );
            let broker = view
                .read(protocol::loan_broker_keylet_from_key(parties.broker_id))
                .expect("broker read")
                .expect("broker exists");
            assert_eq!(
                broker.get_field_number(sf("sfCoverAvailable")).value(),
                number(xrp_asset(), PAYMENT).value()
            );
            assert_eq!(xrp_balance(&view, parties.broker_pseudo), before.1 - 10);
            assert_eq!(xrp_balance(&view, destination), before.2 + 10);
        } else {
            assert_eq!(
                issue_54_cover_snapshot(&view, parties, destination),
                before,
                "{label} must fail before broker cover or payment mutation"
            );
        }
    }
}
