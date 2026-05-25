//! Account info rendering tests.

use super::*;

#[test]
fn account_info_renders_account_data_flags_and_v1_signer_lists() {
    let ledger = closed_ledger();
    let account = sample_account(0x22);
    let signer = sample_account(0x33);

    let mut source = FakeSource {
        ledger: Some(ledger),
        clawback_enabled: true,
        token_escrow_enabled: true,
        ..Default::default()
    };
    source.account_roots.insert(
        account,
        make_account_root(
            account,
            lsfDefaultRipple
                | lsfDisallowIncomingCheck
                | lsfDisallowIncomingTrustline
                | lsfAllowTrustLineClawback
                | lsfAllowTrustLineLocking,
            Some([
                0xDE, 0xAD, 0xBE, 0xEF, 0xCA, 0xFE, 0xBA, 0xBE, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99,
                0xAA, 0xBB,
            ]),
        ),
    );
    source
        .signer_lists
        .insert(account, make_signer_list(account, signer));

    let result = do_account_info(
        &AccountInfoRequest {
            params: &object([
                ("account", JsonValue::String(to_base58(account))),
                ("signer_lists", JsonValue::Bool(true)),
            ]),
            api_version: 1,
            role: Role::Admin,
        },
        &source,
    );
    let JsonValue::Object(result) = result else {
        panic!("result must be an object");
    };

    let JsonValue::Object(account_data) = result
        .get("account_data")
        .expect("account_data should exist")
    else {
        panic!("account_data must be an object");
    };
    assert_eq!(
        account_data.get("urlgravatar"),
        Some(&JsonValue::String(
            "http://www.gravatar.com/avatar/deadbeefcafebabe445566778899aabb".to_owned()
        ))
    );
    assert!(matches!(
        account_data.get("signer_lists"),
        Some(JsonValue::Array(values)) if values.len() == 1
    ));

    let JsonValue::Object(account_flags) = result
        .get("account_flags")
        .expect("account_flags should exist")
    else {
        panic!("account_flags must be an object");
    };
    assert_eq!(
        account_flags.get("defaultRipple"),
        Some(&JsonValue::Bool(true))
    );
    assert_eq!(
        account_flags.get("disallowIncomingCheck"),
        Some(&JsonValue::Bool(true))
    );
    assert_eq!(
        account_flags.get("allowTrustLineClawback"),
        Some(&JsonValue::Bool(true))
    );
    assert_eq!(
        account_flags.get("allowTrustLineLocking"),
        Some(&JsonValue::Bool(true))
    );
}

#[test]
fn account_info_moves_signer_lists_to_top_level_for_v2() {
    let ledger = closed_ledger();
    let account = sample_account(0x44);
    let signer = sample_account(0x45);

    let mut source = FakeSource {
        ledger: Some(ledger),
        ..Default::default()
    };
    source
        .account_roots
        .insert(account, make_account_root(account, 0, None));
    source
        .signer_lists
        .insert(account, make_signer_list(account, signer));

    let result = do_account_info(
        &AccountInfoRequest {
            params: &object([
                ("account", JsonValue::String(to_base58(account))),
                ("signer_lists", JsonValue::Bool(true)),
            ]),
            api_version: 2,
            role: Role::Admin,
        },
        &source,
    );
    let JsonValue::Object(result) = result else {
        panic!("result must be an object");
    };

    assert!(matches!(
        result.get("signer_lists"),
        Some(JsonValue::Array(values)) if values.len() == 1
    ));
    let JsonValue::Object(account_data) = result
        .get("account_data")
        .expect("account_data should exist")
    else {
        panic!("account_data must be an object");
    };
    assert!(!account_data.contains_key("signer_lists"));
}

#[test]
fn account_info_rejects_non_bool_signer_lists_in_v2() {
    let ledger = closed_ledger();
    let account = sample_account(0x55);

    let mut source = FakeSource {
        ledger: Some(ledger),
        ..Default::default()
    };
    source
        .account_roots
        .insert(account, make_account_root(account, 0, None));

    let result = do_account_info(
        &AccountInfoRequest {
            params: &object([
                ("account", JsonValue::String(to_base58(account))),
                ("signer_lists", JsonValue::String("yes".to_owned())),
            ]),
            api_version: 2,
            role: Role::Admin,
        },
        &source,
    );
    let JsonValue::Object(result) = result else {
        panic!("result must be an object");
    };
    assert_eq!(
        result.get("error"),
        Some(&JsonValue::String("invalidParams".to_owned()))
    );
}

#[test]
fn account_info_returns_open_ledger_queue_data_shell() {
    let ledger = open_ledger();
    let account = sample_account(0x65);

    let mut source = FakeSource {
        ledger: Some(ledger),
        ..Default::default()
    };
    source
        .account_roots
        .insert(account, make_account_root(account, 0, None));

    let result = do_account_info(
        &AccountInfoRequest {
            params: &object([
                ("account", JsonValue::String(to_base58(account))),
                ("queue", JsonValue::Bool(true)),
            ]),
            api_version: 2,
            role: Role::Admin,
        },
        &source,
    );
    let JsonValue::Object(result) = result else {
        panic!("result must be an object");
    };
    assert_eq!(result.get("error"), None);
    assert_eq!(
        result.get("queue_data"),
        Some(&JsonValue::Object(std::collections::BTreeMap::from([(
            "txn_count".to_owned(),
            JsonValue::Unsigned(0),
        )])))
    );
}

#[test]
fn account_info_summarizes_open_ledger_queue_stats() {
    let ledger = open_ledger();
    let account = sample_account(0x66);

    let mut source = FakeSource {
        ledger: Some(ledger),
        ..Default::default()
    };
    source
        .account_roots
        .insert(account, make_account_root(account, 0, None));
    source.queue_txs.insert(
        account,
        vec![
            AccountQueueTransaction {
                seq_proxy: SeqProxy::sequence(7),
                fee_level: 256,
                last_valid: Some(105),
                fee_drops: 12,
                max_spend_drops: 112,
                auth_change: false,
            },
            AccountQueueTransaction {
                seq_proxy: SeqProxy::sequence(8),
                fee_level: 512,
                last_valid: None,
                fee_drops: 10,
                max_spend_drops: 40,
                auth_change: true,
            },
            AccountQueueTransaction {
                seq_proxy: SeqProxy::ticket(44),
                fee_level: 1024,
                last_valid: Some(120),
                fee_drops: 15,
                max_spend_drops: 15,
                auth_change: false,
            },
        ],
    );

    let result = do_account_info(
        &AccountInfoRequest {
            params: &object([
                ("account", JsonValue::String(to_base58(account))),
                ("queue", JsonValue::Bool(true)),
            ]),
            api_version: 2,
            role: Role::Admin,
        },
        &source,
    );
    let JsonValue::Object(result) = result else {
        panic!("result must be an object");
    };
    assert_eq!(result.get("error"), None);
    assert_eq!(
        result.get("queue_data"),
        Some(&JsonValue::Object(std::collections::BTreeMap::from([
            ("txn_count".to_owned(), JsonValue::Unsigned(3)),
            (
                "transactions".to_owned(),
                JsonValue::Array(vec![
                    JsonValue::Object(std::collections::BTreeMap::from([
                        ("seq".to_owned(), JsonValue::Unsigned(7)),
                        ("fee_level".to_owned(), JsonValue::String("256".to_owned())),
                        ("LastLedgerSequence".to_owned(), JsonValue::Unsigned(105)),
                        ("fee".to_owned(), JsonValue::String("12".to_owned())),
                        (
                            "max_spend_drops".to_owned(),
                            JsonValue::String("112".to_owned()),
                        ),
                        ("auth_change".to_owned(), JsonValue::Bool(false)),
                    ])),
                    JsonValue::Object(std::collections::BTreeMap::from([
                        ("seq".to_owned(), JsonValue::Unsigned(8)),
                        ("fee_level".to_owned(), JsonValue::String("512".to_owned())),
                        ("fee".to_owned(), JsonValue::String("10".to_owned())),
                        (
                            "max_spend_drops".to_owned(),
                            JsonValue::String("40".to_owned()),
                        ),
                        ("auth_change".to_owned(), JsonValue::Bool(true)),
                    ])),
                    JsonValue::Object(std::collections::BTreeMap::from([
                        ("ticket".to_owned(), JsonValue::Unsigned(44)),
                        ("fee_level".to_owned(), JsonValue::String("1024".to_owned())),
                        ("LastLedgerSequence".to_owned(), JsonValue::Unsigned(120)),
                        ("fee".to_owned(), JsonValue::String("15".to_owned())),
                        (
                            "max_spend_drops".to_owned(),
                            JsonValue::String("15".to_owned()),
                        ),
                        ("auth_change".to_owned(), JsonValue::Bool(false)),
                    ])),
                ]),
            ),
            ("sequence_count".to_owned(), JsonValue::Unsigned(2)),
            ("ticket_count".to_owned(), JsonValue::Unsigned(1)),
            ("lowest_sequence".to_owned(), JsonValue::Unsigned(7)),
            ("highest_sequence".to_owned(), JsonValue::Unsigned(8)),
            ("lowest_ticket".to_owned(), JsonValue::Unsigned(44)),
            ("highest_ticket".to_owned(), JsonValue::Unsigned(44)),
            ("auth_change_queued".to_owned(), JsonValue::Bool(true)),
            (
                "max_spend_drops_total".to_owned(),
                JsonValue::String("167".to_owned()),
            ),
        ])))
    );
}
