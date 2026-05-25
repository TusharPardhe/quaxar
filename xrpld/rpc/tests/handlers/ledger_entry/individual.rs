//! Individual ledger entry type tests.

use super::*;

#[test]
fn ledger_entry_reports_cpp_style_errors_like_current_handler() {
    let owner = parse_base58_account_id("rHb9CJAWyB4rj91VRWn96DkukG4bwdtyTh")
        .expect("known account should parse");
    let source = source_with(vec![STLedgerEntry::from_type_and_key(
        LedgerEntryType::Offer,
        account_keylet(account160(owner)).key,
    )]);

    let duplicate = do_ledger_entry(
        &request(
            object([
                ("ledger_index", JsonValue::Unsigned(9)),
                ("account", JsonValue::String(to_base58(owner))),
                ("index", JsonValue::String("00".repeat(32))),
            ]),
            2,
        ),
        &source,
    );
    assert_error(&duplicate, "invalidParams");

    let legacy_missing = do_ledger_entry(
        &request(object([("ledger_index", JsonValue::Unsigned(9))]), 1),
        &source,
    );
    let JsonValue::Object(legacy_object) = legacy_missing else {
        panic!("expected object");
    };
    assert_eq!(
        legacy_object.get("error"),
        Some(&JsonValue::String("unknownOption".to_owned()))
    );
    assert!(legacy_object.contains_key("ledger_hash"));

    let modern_missing = do_ledger_entry(
        &request(object([("ledger_index", JsonValue::Unsigned(9))]), 2),
        &source,
    );
    assert_error(&modern_missing, "invalidParams");

    let not_found = do_ledger_entry(
        &request(
            object([
                ("ledger_index", JsonValue::Unsigned(9)),
                (
                    "offer",
                    JsonValue::Object(BTreeMap::from([
                        ("account".to_owned(), JsonValue::String(to_base58(owner))),
                        ("seq".to_owned(), JsonValue::Unsigned(99)),
                    ])),
                ),
            ]),
            2,
        ),
        &source,
    );
    assert_error(&not_found, "entryNotFound");

    let unexpected_type = do_ledger_entry(
        &request(
            object([
                ("ledger_index", JsonValue::Unsigned(9)),
                ("account", JsonValue::String(to_base58(owner))),
            ]),
            2,
        ),
        &source,
    );
    assert_error(&unexpected_type, "unexpectedLedgerType");
}

#[test]
fn ledger_entry_account_root_by_account_string() {
    let owner = account(0x30);
    let mut root = STLedgerEntry::from_type_and_key(
        LedgerEntryType::AccountRoot,
        account_keylet(account160(owner)).key,
    );
    root.set_account_id(get_field_by_symbol("sfAccount"), owner);
    root.set_field_u32(get_field_by_symbol("sfSequence"), 42);

    let source = source_with(vec![root]);
    let result = do_ledger_entry(
        &request(
            object([
                ("ledger_index", JsonValue::String("validated".to_owned())),
                ("account_root", JsonValue::String(to_base58(owner))),
            ]),
            2,
        ),
        &source,
    );
    assert_result_index(&result, account_keylet(account160(owner)).key);

    let obj = json_object(&result);
    assert!(obj.contains_key("node") || obj.contains_key("node_binary"));
    assert_eq!(
        obj.get("ledger_hash"),
        Some(&JsonValue::String(closed_ledger().hash.to_string()))
    );
    assert_eq!(
        obj.get("ledger_index"),
        Some(&JsonValue::Unsigned(u64::from(closed_ledger().seq)))
    );
    assert_eq!(obj.get("validated"), Some(&JsonValue::Bool(true)));
}

#[test]
fn ledger_entry_by_index_hex() {
    let owner = account(0x31);
    let key = account_keylet(account160(owner)).key;
    let mut root = STLedgerEntry::from_type_and_key(LedgerEntryType::AccountRoot, key);
    root.set_account_id(get_field_by_symbol("sfAccount"), owner);

    let source = source_with(vec![root]);
    let result = do_ledger_entry(
        &request(
            object([
                ("ledger_index", JsonValue::String("validated".to_owned())),
                ("index", JsonValue::String(key.to_string())),
            ]),
            2,
        ),
        &source,
    );
    assert_result_index(&result, key);
}

#[test]
fn ledger_entry_binary_mode() {
    let owner = account(0x32);
    let key = account_keylet(account160(owner)).key;
    let mut root = STLedgerEntry::from_type_and_key(LedgerEntryType::AccountRoot, key);
    root.set_account_id(get_field_by_symbol("sfAccount"), owner);

    let source = source_with(vec![root.clone()]);
    let result = do_ledger_entry(
        &request(
            object([
                ("ledger_index", JsonValue::String("validated".to_owned())),
                ("index", JsonValue::String(key.to_string())),
                ("binary", JsonValue::Bool(true)),
            ]),
            2,
        ),
        &source,
    );
    let obj = json_object(&result);
    assert!(obj.contains_key("node_binary"));
    assert!(!obj.contains_key("node"));
    let JsonValue::String(hex) = obj.get("node_binary").unwrap() else {
        panic!("node_binary must be a string");
    };
    assert_eq!(hex, &serialize_hex(&root));
}

#[test]
fn ledger_entry_not_found_error() {
    let source = source_with(vec![]);
    let fake_key = Uint256::from_array([0xDD; 32]);
    let result = do_ledger_entry(
        &request(
            object([
                ("ledger_index", JsonValue::String("validated".to_owned())),
                ("index", JsonValue::String(fake_key.to_string())),
            ]),
            2,
        ),
        &source,
    );
    assert_error(&result, "entryNotFound");
}

#[test]
fn ledger_entry_missing_selector_error() {
    let source = source_with(vec![]);
    let result = do_ledger_entry(
        &request(
            object([("ledger_index", JsonValue::String("validated".to_owned()))]),
            2,
        ),
        &source,
    );
    assert_error(&result, "invalidParams");
}

#[test]
fn ledger_entry_invalid_index_hex() {
    let source = source_with(vec![]);
    let result = do_ledger_entry(
        &request(
            object([
                ("ledger_index", JsonValue::String("validated".to_owned())),
                ("index", JsonValue::String("not_valid_hex".to_owned())),
            ]),
            2,
        ),
        &source,
    );
    assert_error(&result, "malformedRequest");
}

#[test]
fn ledger_entry_directory_by_owner() {
    let owner = account(0x33);
    let dir_key = protocol::owner_dir_keylet(account160(owner));
    let mut dir = STLedgerEntry::from_type_and_key(LedgerEntryType::DirectoryNode, dir_key.key);
    dir.set_account_id(get_field_by_symbol("sfOwner"), owner);

    let source = source_with(vec![dir]);
    let result = do_ledger_entry(
        &request(
            object([
                ("ledger_index", JsonValue::String("validated".to_owned())),
                (
                    "directory",
                    object([("owner", JsonValue::String(to_base58(owner)))]),
                ),
            ]),
            2,
        ),
        &source,
    );
    assert_result_index(&result, dir_key.key);
}

#[test]
fn ledger_entry_offer_by_account_and_seq() {
    let owner = account(0x34);
    let offer_key = protocol::offer_keylet(account160(owner), 7);
    let mut offer = STLedgerEntry::from_type_and_key(LedgerEntryType::Offer, offer_key.key);
    offer.set_account_id(get_field_by_symbol("sfAccount"), owner);
    offer.set_field_u32(get_field_by_symbol("sfSequence"), 7);

    let source = source_with(vec![offer]);
    let result = do_ledger_entry(
        &request(
            object([
                ("ledger_index", JsonValue::String("validated".to_owned())),
                (
                    "offer",
                    object([
                        ("account", JsonValue::String(to_base58(owner))),
                        ("seq", JsonValue::Unsigned(7)),
                    ]),
                ),
            ]),
            2,
        ),
        &source,
    );
    assert_result_index(&result, offer_key.key);
}

#[test]
fn ledger_entry_ripple_state_by_accounts_and_currency() {
    let alice = account(0x35);
    let bob = account(0x36);
    let usd = currency(0x01);
    let (low, high) = if alice < bob {
        (alice, bob)
    } else {
        (bob, alice)
    };
    let line_key = protocol::line(low, high, usd);
    let mut line = STLedgerEntry::from_type_and_key(LedgerEntryType::RippleState, line_key.key);
    line.set_field_u32(get_field_by_symbol("sfFlags"), 0);

    let source = source_with(vec![line]);
    let result = do_ledger_entry(
        &request(
            object([
                ("ledger_index", JsonValue::String("validated".to_owned())),
                (
                    "ripple_state",
                    object([
                        (
                            "accounts",
                            JsonValue::Array(vec![
                                JsonValue::String(to_base58(alice)),
                                JsonValue::String(to_base58(bob)),
                            ]),
                        ),
                        (
                            "currency",
                            JsonValue::String(protocol::currency_to_string(usd)),
                        ),
                    ]),
                ),
            ]),
            2,
        ),
        &source,
    );
    assert_result_index(&result, line_key.key);
}

#[test]
fn ledger_entry_escrow_by_owner_and_seq() {
    let owner = account(0x40);
    let escrow_key = protocol::escrow_keylet(account160(owner), 3);
    let mut escrow = STLedgerEntry::from_type_and_key(LedgerEntryType::Escrow, escrow_key.key);
    escrow.set_account_id(get_field_by_symbol("sfAccount"), owner);
    escrow.set_field_u32(get_field_by_symbol("sfSequence"), 3);

    let source = source_with(vec![escrow]);
    let result = do_ledger_entry(
        &request(
            object([
                ("ledger_index", JsonValue::String("validated".to_owned())),
                (
                    "escrow",
                    object([
                        ("owner", JsonValue::String(to_base58(owner))),
                        ("seq", JsonValue::Unsigned(3)),
                    ]),
                ),
            ]),
            2,
        ),
        &source,
    );
    assert_result_index(&result, escrow_key.key);
}

#[test]
fn ledger_entry_check_by_index() {
    let owner = account(0x41);
    let check_key = protocol::check_keylet(account160(owner), 5);
    let mut check = STLedgerEntry::from_type_and_key(LedgerEntryType::Check, check_key.key);
    check.set_account_id(get_field_by_symbol("sfAccount"), owner);

    let source = source_with(vec![check]);
    let result = do_ledger_entry(
        &request(
            object([
                ("ledger_index", JsonValue::String("validated".to_owned())),
                ("index", JsonValue::String(check_key.key.to_string())),
            ]),
            2,
        ),
        &source,
    );
    assert_result_index(&result, check_key.key);
}

#[test]
fn ledger_entry_ticket_by_account_and_seq() {
    let owner = account(0x42);
    let ticket_key = protocol::ticket_keylet(account160(owner), 10);
    let mut ticket = STLedgerEntry::from_type_and_key(LedgerEntryType::Ticket, ticket_key.key);
    ticket.set_account_id(get_field_by_symbol("sfAccount"), owner);
    ticket.set_field_u32(get_field_by_symbol("sfTicketSequence"), 10);

    let source = source_with(vec![ticket]);
    let result = do_ledger_entry(
        &request(
            object([
                ("ledger_index", JsonValue::String("validated".to_owned())),
                (
                    "ticket",
                    object([
                        ("account", JsonValue::String(to_base58(owner))),
                        ("ticket_seq", JsonValue::Unsigned(10)),
                    ]),
                ),
            ]),
            2,
        ),
        &source,
    );
    assert_result_index(&result, ticket_key.key);
}

#[test]
fn ledger_entry_deposit_preauth_by_owner_and_authorized() {
    let owner = account(0x43);
    let authorized = account(0x44);
    let preauth_key = protocol::deposit_preauth_keylet(account160(owner), account160(authorized));
    let mut preauth =
        STLedgerEntry::from_type_and_key(LedgerEntryType::DepositPreauth, preauth_key.key);
    preauth.set_account_id(get_field_by_symbol("sfAccount"), owner);

    let source = source_with(vec![preauth]);
    let result = do_ledger_entry(
        &request(
            object([
                ("ledger_index", JsonValue::String("validated".to_owned())),
                (
                    "deposit_preauth",
                    object([
                        ("owner", JsonValue::String(to_base58(owner))),
                        ("authorized", JsonValue::String(to_base58(authorized))),
                    ]),
                ),
            ]),
            2,
        ),
        &source,
    );
    assert_result_index(&result, preauth_key.key);
}

#[test]
fn ledger_entry_amendments() {
    let amendments_key = protocol::amendments_keylet();
    let amendments =
        STLedgerEntry::from_type_and_key(LedgerEntryType::Amendments, amendments_key.key);

    let source = source_with(vec![amendments]);
    let result = do_ledger_entry(
        &request(
            object([
                ("ledger_index", JsonValue::String("validated".to_owned())),
                ("index", JsonValue::String(amendments_key.key.to_string())),
            ]),
            2,
        ),
        &source,
    );
    assert_result_index(&result, amendments_key.key);
}

#[test]
fn ledger_entry_fee_settings() {
    let fee_key = protocol::fee_settings_keylet();
    let fee = STLedgerEntry::from_type_and_key(LedgerEntryType::FeeSettings, fee_key.key);

    let source = source_with(vec![fee]);
    let result = do_ledger_entry(
        &request(
            object([
                ("ledger_index", JsonValue::String("validated".to_owned())),
                ("index", JsonValue::String(fee_key.key.to_string())),
            ]),
            2,
        ),
        &source,
    );
    assert_result_index(&result, fee_key.key);
}

#[test]
fn ledger_entry_negative_unl() {
    let nunl_key = protocol::negative_unl_keylet();
    let nunl = STLedgerEntry::from_type_and_key(LedgerEntryType::NegativeUnl, nunl_key.key);

    let source = source_with(vec![nunl]);
    let result = do_ledger_entry(
        &request(
            object([
                ("ledger_index", JsonValue::String("validated".to_owned())),
                ("index", JsonValue::String(nunl_key.key.to_string())),
            ]),
            2,
        ),
        &source,
    );
    assert_result_index(&result, nunl_key.key);
}

#[test]
fn ledger_entry_signer_list() {
    let owner = account(0x45);
    let signers_key = protocol::signers_keylet(account160(owner));
    let mut signers =
        STLedgerEntry::from_type_and_key(LedgerEntryType::SignerList, signers_key.key);
    signers.set_field_u32(get_field_by_symbol("sfSignerQuorum"), 3);

    let source = source_with(vec![signers]);
    let result = do_ledger_entry(
        &request(
            object([
                ("ledger_index", JsonValue::String("validated".to_owned())),
                ("index", JsonValue::String(signers_key.key.to_string())),
            ]),
            2,
        ),
        &source,
    );
    assert_result_index(&result, signers_key.key);
}

#[test]
fn ledger_entry_pay_channel() {
    let source_acct = account(0x46);
    let dest = account(0x47);
    let paychan_key = protocol::pay_channel_keylet(account160(source_acct), account160(dest), 1);
    let mut paychan =
        STLedgerEntry::from_type_and_key(LedgerEntryType::PayChannel, paychan_key.key);
    paychan.set_account_id(get_field_by_symbol("sfAccount"), source_acct);
    paychan.set_account_id(get_field_by_symbol("sfDestination"), dest);

    let source = source_with(vec![paychan]);
    let result = do_ledger_entry(
        &request(
            object([
                ("ledger_index", JsonValue::String("validated".to_owned())),
                ("index", JsonValue::String(paychan_key.key.to_string())),
            ]),
            2,
        ),
        &source,
    );
    assert_result_index(&result, paychan_key.key);
}

#[test]
fn ledger_entry_malformed_account_root() {
    let source = source_with(vec![]);
    let result = do_ledger_entry(
        &request(
            object([
                ("ledger_index", JsonValue::String("validated".to_owned())),
                ("account_root", JsonValue::String("notAnAccount".to_owned())),
            ]),
            2,
        ),
        &source,
    );
    assert_error(&result, "malformedAddress");
}

#[test]
fn ledger_entry_malformed_offer_missing_seq() {
    let source = source_with(vec![]);
    let result = do_ledger_entry(
        &request(
            object([
                ("ledger_index", JsonValue::String("validated".to_owned())),
                (
                    "offer",
                    object([("account", JsonValue::String(to_base58(account(0x50))))]),
                ),
            ]),
            2,
        ),
        &source,
    );
    assert_error(&result, "malformedRequest");
}

#[test]
fn ledger_entry_malformed_ripple_state_missing_currency() {
    let source = source_with(vec![]);
    let result = do_ledger_entry(
        &request(
            object([
                ("ledger_index", JsonValue::String("validated".to_owned())),
                (
                    "ripple_state",
                    object([(
                        "accounts",
                        JsonValue::Array(vec![
                            JsonValue::String(to_base58(account(0x51))),
                            JsonValue::String(to_base58(account(0x52))),
                        ]),
                    )]),
                ),
            ]),
            2,
        ),
        &source,
    );
    assert_error(&result, "malformedRequest");
}

#[test]
fn ledger_entry_malformed_escrow_missing_seq() {
    let source = source_with(vec![]);
    let result = do_ledger_entry(
        &request(
            object([
                ("ledger_index", JsonValue::String("validated".to_owned())),
                (
                    "escrow",
                    object([("owner", JsonValue::String(to_base58(account(0x53))))]),
                ),
            ]),
            2,
        ),
        &source,
    );
    assert_error(&result, "malformedSeq");
}
