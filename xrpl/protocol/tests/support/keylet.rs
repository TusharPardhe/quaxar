//! Integration tests that pin narrowed public `xrpl/protocol` key helpers to
//! the current C++ `Indexes.cpp` behavior.

use basics::base_uint::{Uint160, Uint256};
use protocol::SeqProxy;
use protocol::keylet::{
    Keylet, LedgerEntryTypeInfo, account, amendments, book, check, credential, delegate,
    depositPreauth, did, escrow, fees, getBookBase, getQuality, getQualityNext, getTicketIndex,
    ledger_entry_type_catalog, ledger_entry_type_code, ledger_entry_type_from_code,
    ledger_entry_type_from_name, loanbroker, mptIssuance, mptoken, negativeUNL, next, offer,
    oracle, ownerDir, page, payChan, permissionedDomain, quality, signers, skip, ticket, unchecked,
    vault,
};
use protocol::{
    AccountID, Asset, Book, Currency, DIRECT_ACCOUNT_KEYLETS, Issue, account_keylet,
    account_root_key, amm, amm_keylet, book_keylet, bridge_keylet_from_door_issue,
    check_keylet_from_key, credential_keylet_from_key, deposit_preauth_keylet_from_key, did_keylet,
    escrow_keylet_from_key, get_book_base, get_quality_next, ledger_hashes_keylet, line,
    line_from_issue, loan_broker_keylet_from_key, loan_key, loan_keylet_from_key,
    mpt_issuance_keylet_from_id, mpt_issuance_keylet_from_mptid, mptoken_keylet_from_id,
    mptoken_keylet_from_mptid, next_keylet, nft_buy_offers_keylet, nft_offer_keylet,
    nft_offer_keylet_for_owner, nft_offer_keylet_from_key, nft_page_max_keylet,
    nft_page_min_keylet, nft_sell_offers_keylet, offer_keylet_from_key, owner_dir_keylet,
    page_keylet, pay_channel_keylet_from_key, permissioned_domain_keylet_from_id, quality_from_key,
    quality_keylet, signers_keylet, signers_keylet_for_page, ticket_index,
    ticket_index_from_seq_proxy, ticket_keylet, ticket_keylet_from_key,
    ticket_keylet_from_seq_proxy, vault_keylet_from_key, xchain_owned_claim_id_keylet_from_bridge,
    xchain_owned_create_account_claim_id_keylet_from_bridge,
};

#[test]
fn protocol_loan_key_matches_current_cpp_vector() {
    let loan_broker_id =
        Uint256::from_hex("0123456789ABCDEFFEDCBA98765432100123456789ABCDEFFEDCBA9876543210")
            .expect("expected loan broker id should parse");

    assert_eq!(
        loan_key(loan_broker_id, 7),
        Uint256::from_hex("B9CF90CA6D45957E6BB9A59666C328113077AA775B5B6516C8AFDDC507647E90")
            .expect("expected loan key should parse")
    );
}

#[test]
fn protocol_loan_key_changes_when_sequence_changes() {
    let loan_broker_id =
        Uint256::from_hex("0123456789ABCDEFFEDCBA98765432100123456789ABCDEFFEDCBA9876543210")
            .expect("expected loan broker id should parse");

    assert_ne!(loan_key(loan_broker_id, 7), loan_key(loan_broker_id, 8));
}

#[test]
fn protocol_loan_key_changes_when_broker_id_changes() {
    let first =
        Uint256::from_hex("0123456789ABCDEFFEDCBA98765432100123456789ABCDEFFEDCBA9876543210")
            .expect("first loan broker id should parse");
    let second =
        Uint256::from_hex("FFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFF00000000000000000000000000000000")
            .expect("second loan broker id should parse");

    assert_ne!(loan_key(first, 7), loan_key(second, 7));
}

#[test]
fn protocol_keylet_catalog_exports_current_index_helpers() {
    let account = Uint160::from_hex("B5F762798A53D543A014CAF8B297CFF8F2F937E8")
        .expect("expected account should parse");
    let key = Uint256::from_u64(42);

    assert_eq!(account_keylet(account).key, account_root_key(account));
    assert_eq!(
        ledger_hashes_keylet().entry_type,
        protocol::LedgerEntryType::LedgerHashes
    );
    assert_eq!(
        nft_offer_keylet(key).entry_type,
        protocol::LedgerEntryType::NFTokenOffer
    );
    assert_eq!(nft_offer_keylet_from_key(key), nft_offer_keylet(key));
    assert_eq!(
        did_keylet(account).entry_type,
        protocol::LedgerEntryType::DID
    );
    assert_eq!(
        permissioned_domain_keylet_from_id(key).entry_type,
        protocol::LedgerEntryType::PermissionedDomain
    );
    assert_eq!(
        mpt_issuance_keylet_from_id(key).entry_type,
        protocol::LedgerEntryType::MPTokenIssuance
    );
    assert_eq!(
        mptoken_keylet_from_id(key).entry_type,
        protocol::LedgerEntryType::MPToken
    );
    assert_eq!(
        vault_keylet_from_key(key).entry_type,
        protocol::LedgerEntryType::Vault
    );
    assert_eq!(
        offer_keylet_from_key(key).entry_type,
        protocol::LedgerEntryType::Offer
    );
    assert_eq!(
        ticket_keylet_from_key(key).entry_type,
        protocol::LedgerEntryType::Ticket
    );
    assert_eq!(
        check_keylet_from_key(key).entry_type,
        protocol::LedgerEntryType::Check
    );
    assert_eq!(
        deposit_preauth_keylet_from_key(key).entry_type,
        protocol::LedgerEntryType::DepositPreauth
    );
    assert_eq!(
        escrow_keylet_from_key(key).entry_type,
        protocol::LedgerEntryType::Escrow
    );
    assert_eq!(
        pay_channel_keylet_from_key(key).entry_type,
        protocol::LedgerEntryType::PayChannel
    );
    assert_eq!(
        loan_broker_keylet_from_key(key).entry_type,
        protocol::LedgerEntryType::LoanBroker
    );
    assert_eq!(
        loan_keylet_from_key(key).entry_type,
        protocol::LedgerEntryType::Loan
    );
    assert_eq!(
        credential_keylet_from_key(key).entry_type,
        protocol::LedgerEntryType::Credential
    );
    assert_eq!(
        nft_page_min_keylet(account).entry_type,
        protocol::LedgerEntryType::NFTokenPage
    );
    assert_eq!(
        nft_page_max_keylet(account).entry_type,
        protocol::LedgerEntryType::NFTokenPage
    );
}

#[test]
fn protocol_cpp_name_aliases_match_current_keylet_helpers() {
    let account_id = AccountID::from_hex("AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA")
        .expect("account should parse");
    let other = AccountID::from_hex("BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB")
        .expect("other account should parse");
    let account_raw = Uint160::from_hex("AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA")
        .expect("raw account should parse");
    let other_raw = Uint160::from_hex("BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB")
        .expect("raw other account should parse");
    let book_input = Book::new(
        Issue::new(
            Currency::from_hex("0102030405060708090A0B0C0D0E0F1011121314")
                .expect("expected input currency should parse"),
            account_id,
        ),
        Issue::new(
            Currency::from_hex("1111111111111111111111111111111111111111")
                .expect("expected output currency should parse"),
            other,
        ),
        None,
    );
    let quality_base = owner_dir_keylet(account_raw);
    let quality_key = quality_keylet(quality_base, 7);
    let issuance_id = Uint256::from_u64(9);

    assert_eq!(account(account_raw), account_keylet(account_raw));
    assert_eq!(amendments(), protocol::amendments_keylet());
    assert_eq!(book(book_input), book_keylet(book_input));
    assert_eq!(
        check(account_raw, 9),
        protocol::check_keylet(account_raw, 9)
    );
    assert_eq!(
        credential(account_raw, other_raw, b"cred"),
        protocol::credential_keylet(account_raw, other_raw, b"cred")
    );
    assert_eq!(
        delegate(account_raw, other_raw),
        protocol::delegate_keylet(account_raw, other_raw)
    );
    assert_eq!(
        depositPreauth(account_raw, other_raw),
        protocol::deposit_preauth_keylet(account_raw, other_raw)
    );
    assert_eq!(did(account_raw), did_keylet(account_raw));
    assert_eq!(
        escrow(account_raw, 4),
        protocol::escrow_keylet(account_raw, 4)
    );
    assert_eq!(fees(), protocol::fee_settings_keylet());
    assert_eq!(getBookBase(book_input), get_book_base(book_input));
    assert_eq!(
        getQuality(quality_key.key),
        quality_from_key(quality_key.key)
    );
    assert_eq!(
        getQualityNext(quality_key.key),
        get_quality_next(quality_key.key)
    );
    assert_eq!(getTicketIndex(account_raw, 7), ticket_index(account_raw, 7));
    assert_eq!(
        loanbroker(account_raw, 4),
        protocol::loan_broker_keylet(account_raw, 4)
    );
    assert_eq!(
        mptIssuance(5, account_raw),
        protocol::mpt_issuance_keylet(5, account_raw)
    );
    assert_eq!(
        mptoken(issuance_id, account_raw),
        protocol::mptoken_keylet(issuance_id, account_raw)
    );
    let issuance_mptid =
        protocol::MPTID::from_hex("000102030405060708090A0B0C0D0E0F1011121314151617")
            .expect("issuance id should parse");
    let issuance_keylet = mpt_issuance_keylet_from_mptid(issuance_mptid);
    assert_eq!(
        issuance_keylet.entry_type,
        protocol::LedgerEntryType::MPTokenIssuance
    );
    assert_eq!(
        mptoken_keylet_from_mptid(issuance_mptid, account_raw),
        protocol::mptoken_keylet(issuance_keylet.key, account_raw)
    );
    assert_eq!(negativeUNL(), protocol::negative_unl_keylet());
    assert_eq!(next(quality_key), next_keylet(quality_key));
    assert_eq!(
        offer(account_raw, 3),
        protocol::offer_keylet(account_raw, 3)
    );
    assert_eq!(
        oracle(account_raw, 8),
        protocol::oracle_keylet(account_raw, 8)
    );
    assert_eq!(ownerDir(account_raw), owner_dir_keylet(account_raw));
    assert_eq!(page(quality_base, 1), page_keylet(quality_base, 1));
    assert_eq!(
        payChan(account_raw, other_raw, 6),
        protocol::pay_channel_keylet(account_raw, other_raw, 6)
    );
    assert_eq!(
        permissionedDomain(account_raw, 11),
        protocol::permissioned_domain_keylet(account_raw, 11)
    );
    assert_eq!(quality(quality_base, 7), quality_keylet(quality_base, 7));
    assert_eq!(signers(account_raw), signers_keylet(account_raw));
    assert_eq!(skip(), protocol::skip_keylet());
    assert_eq!(
        ticket(account_raw, 12),
        protocol::ticket_keylet(account_raw, 12)
    );
    assert_eq!(
        unchecked(Uint256::from_u64(5)),
        protocol::unchecked_keylet(Uint256::from_u64(5))
    );
    assert_eq!(
        vault(account_raw, 13),
        protocol::vault_keylet(account_raw, 13)
    );
}

#[test]
fn protocol_trustline_keylet_matches_current_cpp_vectors_and_issue_overload() {
    let left = AccountID::from_hex("1111111111111111111111111111111111111111")
        .expect("expected left account should parse");
    let right = AccountID::from_hex("2222222222222222222222222222222222222222")
        .expect("expected right account should parse");
    let currency = Currency::from_hex("0123456789ABCDEFFEDCBA987654321001234567")
        .expect("expected trustline currency should parse");
    let issue = Issue::new(currency, right);
    let expected =
        Uint256::from_hex("DFE13F9596C42788C6B6B0AB5631FDF65D807AF038C4BD61C892695273EFB115")
            .expect("expected trustline key should parse");

    assert_eq!(line(left, right, currency), line(right, left, currency));
    assert_eq!(
        line(left, right, currency).entry_type,
        protocol::LedgerEntryType::RippleState
    );
    assert_eq!(line(left, right, currency).key, expected);
    assert_eq!(line_from_issue(left, issue), line(left, right, currency));
}

#[test]
fn protocol_amm_keylet_matches_current_cpp_vectors_and_canonicalization() {
    let first_account = AccountID::from_hex("AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA")
        .expect("expected first account should parse");
    let second_account = AccountID::from_hex("BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB")
        .expect("expected second account should parse");
    let first_currency = Currency::from_hex("0102030405060708090A0B0C0D0E0F1011121314")
        .expect("expected first currency should parse");
    let second_currency = Currency::from_hex("1111111111111111111111111111111111111111")
        .expect("expected second currency should parse");
    let first_issue = Issue::new(first_currency, first_account);
    let second_issue = Issue::new(second_currency, second_account);
    let first_asset = Asset::from(first_issue);
    let second_asset = Asset::from(second_issue);
    let expected =
        Uint256::from_hex("40D0490ADC5253643F097C4EDAC65DA5065AB53B3AB624141D2849BB85C08584")
            .expect("expected AMM key should parse");

    assert_eq!(
        amm(first_asset, second_asset),
        amm(second_asset, first_asset)
    );
    assert_eq!(
        amm(first_asset, second_asset).entry_type,
        protocol::LedgerEntryType::AMM
    );
    assert_eq!(amm(first_asset, second_asset).key, expected);
    assert_eq!(amm_keylet(expected), amm(first_asset, second_asset));
}

#[test]
fn protocol_bridge_and_xchain_keylets_match_current_cpp_vectors() {
    let locking_door = Uint160::from_hex("AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA")
        .expect("locking door should parse");
    let issuing_door = Uint160::from_hex("BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB")
        .expect("issuing door should parse");
    let locking_issue = Issue::new(
        Currency::from_hex("5553440000000000000000000000000000000000")
            .expect("locking currency should parse"),
        AccountID::from_hex("0000000000000000000000000000000000000000")
            .expect("zero issuer should parse"),
    );
    let issuing_issue = Issue::new(
        Currency::from_hex("434E590000000000000000000000000000000000")
            .expect("issuing currency should parse"),
        AccountID::from_hex("CCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCC")
            .expect("issuing issuer should parse"),
    );

    assert_eq!(
        bridge_keylet_from_door_issue(locking_door, locking_issue).key,
        Uint256::from_hex("C2772BC97F6567C08C8F15E72A34D7801DFFCAE55245FD8337C0F606EEC5E2E1")
            .expect("bridge key should parse")
    );
    assert_eq!(
        xchain_owned_claim_id_keylet_from_bridge(
            locking_door,
            locking_issue,
            issuing_door,
            issuing_issue,
            7,
        )
        .key,
        Uint256::from_hex("8B7ED2DA2F19997E31D74134896AF8765CBE4884C3AEAEDC6B6E16F89833E837")
            .expect("xchain claim id key should parse")
    );
    assert_eq!(
        xchain_owned_create_account_claim_id_keylet_from_bridge(
            locking_door,
            locking_issue,
            issuing_door,
            issuing_issue,
            7,
        )
        .key,
        Uint256::from_hex("BBB2B5028ED800A79E48DFC174C29E740AA6A71ED1F6A3C0FAC99B4A390E513D")
            .expect("xchain create account claim id key should parse")
    );
}

#[test]
fn protocol_book_base_and_keylet_match_current_cpp_vectors() {
    let book = Book::new(
        Issue::new(
            Currency::from_hex("0102030405060708090A0B0C0D0E0F1011121314")
                .expect("expected input currency should parse"),
            AccountID::from_hex("AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA")
                .expect("expected input account should parse"),
        ),
        Issue::new(
            Currency::from_hex("1111111111111111111111111111111111111111")
                .expect("expected output currency should parse"),
            AccountID::from_hex("BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB")
                .expect("expected output account should parse"),
        ),
        None,
    );

    let expected =
        Uint256::from_hex("E00419C2BE1EF9FE80939748893A123B383F5904C0401A3E0000000000000000")
            .expect("expected book base should parse");

    assert_eq!(get_book_base(book), expected);
    assert_eq!(
        book_keylet(book).entry_type,
        protocol::LedgerEntryType::DirectoryNode
    );
    assert_eq!(book_keylet(book).key, expected);
}

#[test]
fn protocol_book_base_includes_domain() {
    let book = Book::new(
        Issue::new(
            Currency::from_hex("0102030405060708090A0B0C0D0E0F1011121314")
                .expect("expected input currency should parse"),
            AccountID::from_hex("AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA")
                .expect("expected input account should parse"),
        ),
        Issue::new(
            Currency::from_hex("1111111111111111111111111111111111111111")
                .expect("expected output currency should parse"),
            AccountID::from_hex("BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB")
                .expect("expected output account should parse"),
        ),
        Some(
            Uint256::from_hex("0123456789ABCDEFFEDCBA98765432100123456789ABCDEFFEDCBA9876543210")
                .expect("expected domain should parse"),
        ),
    );

    assert_eq!(
        get_book_base(book),
        Uint256::from_hex("1E9AC7B1CFFAEF3A8E81ED8133431F717C7DE3CCC7F3B3510000000000000000")
            .expect("expected domain book base should parse")
    );
}

#[test]
fn protocol_direct_account_keylet_catalog_shape() {
    let account = Uint160::from_hex("AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA")
        .expect("expected account should parse");

    assert_eq!(DIRECT_ACCOUNT_KEYLETS.len(), 6);
    assert_eq!(
        (DIRECT_ACCOUNT_KEYLETS[0].function)(account),
        account_keylet(account)
    );
    assert_eq!(DIRECT_ACCOUNT_KEYLETS[0].expected_le_name, "AccountRoot");
    assert!(!DIRECT_ACCOUNT_KEYLETS[0].include_in_tests);
    assert_eq!(DIRECT_ACCOUNT_KEYLETS[1].expected_le_name, "DirectoryNode");
    assert_eq!(DIRECT_ACCOUNT_KEYLETS[2].expected_le_name, "SignerList");
    assert_eq!(DIRECT_ACCOUNT_KEYLETS[3].expected_le_name, "NFTokenPage");
    assert_eq!(DIRECT_ACCOUNT_KEYLETS[4].expected_le_name, "NFTokenPage");
    assert_eq!(DIRECT_ACCOUNT_KEYLETS[5].expected_le_name, "DID");
    assert!(
        DIRECT_ACCOUNT_KEYLETS[1..]
            .iter()
            .all(|desc| desc.include_in_tests)
    );
}

#[test]
fn protocol_keylet_check_ledger_entry_any_child_and_exact_rules() {
    let key = Uint256::from_u64(7);
    let other_key = Uint256::from_u64(8);

    assert!(
        Keylet::new(protocol::LedgerEntryType::Any, key)
            .check_ledger_entry(protocol::LedgerEntryType::Offer, other_key)
    );

    assert!(
        Keylet::new(protocol::LedgerEntryType::Child, key)
            .check_ledger_entry(protocol::LedgerEntryType::Offer, other_key)
    );
    assert!(
        !Keylet::new(protocol::LedgerEntryType::Child, key)
            .check_ledger_entry(protocol::LedgerEntryType::DirectoryNode, other_key)
    );

    assert!(
        Keylet::new(protocol::LedgerEntryType::Offer, key)
            .check_ledger_entry(protocol::LedgerEntryType::Offer, key)
    );
    assert!(
        !Keylet::new(protocol::LedgerEntryType::Offer, key)
            .check_ledger_entry(protocol::LedgerEntryType::Offer, other_key)
    );
    assert!(
        !Keylet::new(protocol::LedgerEntryType::Offer, key)
            .check_ledger_entry(protocol::LedgerEntryType::Check, key)
    );
}

#[test]
fn protocol_directory_quality_keylet_matches_current_cpp_vector() {
    let owner = Uint160::from_hex("B5F762798A53D543A014CAF8B297CFF8F2F937E8")
        .expect("expected owner should parse");

    assert_eq!(
        quality_keylet(owner_dir_keylet(owner), 0x1122_3344_5566_7788).key,
        Uint256::from_hex("D8120FC732737A2CF2E9968FDF3797A43B457F2A81AA06D21122334455667788")
            .expect("expected quality key should parse")
    );
}

#[test]
fn protocol_directory_next_keylet_matches_current_cpp_vector() {
    let quality = Keylet::new(
        protocol::LedgerEntryType::DirectoryNode,
        Uint256::from_hex("D8120FC732737A2CF2E9968FDF3797A43B457F2A81AA06D21122334455667788")
            .expect("expected quality key should parse"),
    );

    assert_eq!(
        next_keylet(quality).key,
        Uint256::from_hex("D8120FC732737A2CF2E9968FDF3797A43B457F2A81AA06D31122334455667788")
            .expect("expected next key should parse")
    );
}

#[test]
fn protocol_directory_page_keylet_matches_current_cpp_vectors() {
    let owner = Uint160::from_hex("B5F762798A53D543A014CAF8B297CFF8F2F937E8")
        .expect("expected owner should parse");
    let root = owner_dir_keylet(owner);

    assert_eq!(page_keylet(root, 0), root);
    assert_eq!(
        page_keylet(root, 1).key,
        Uint256::from_hex("B001E91B2C4405A56F0BD0F6770A0B3230832C472667DFE9754933CA7F49A4F7")
            .expect("expected page key should parse")
    );
}

#[test]
fn protocol_directory_quality_reader_matches_current_cpp_vector() {
    let quality =
        Uint256::from_hex("D8120FC732737A2CF2E9968FDF3797A43B457F2A81AA06D21122334455667788")
            .expect("expected quality key should parse");

    assert_eq!(quality_from_key(quality), 0x1122_3344_5566_7788);
}

#[test]
fn protocol_nft_offer_directories_match_current_cpp_vectors() {
    let token_id =
        Uint256::from_hex("0123456789ABCDEFFEDCBA98765432100123456789ABCDEFFEDCBA9876543210")
            .expect("expected token id should parse");

    assert_eq!(
        nft_buy_offers_keylet(token_id).key,
        Uint256::from_hex("4BA5C0274A9FA4223ECAE038EF2307EA8F58CD6AF7CBE5E22BAF0FE7275E3B23")
            .expect("expected buy-offer directory key should parse")
    );
    assert_eq!(
        nft_sell_offers_keylet(token_id).key,
        Uint256::from_hex("DC6B4198C90EBED069DF7D181F3D8937EFCBD25DA8A37AA372BF980902A0BBB6")
            .expect("expected sell-offer directory key should parse")
    );
}

#[test]
fn protocol_nft_offer_owner_sequence_keylet_matches_current_cpp_vector() {
    let owner = Uint160::from_hex("B5F762798A53D543A014CAF8B297CFF8F2F937E8")
        .expect("expected owner should parse");

    assert_eq!(
        nft_offer_keylet_for_owner(owner, 7).key,
        Uint256::from_hex("1BEA469F51623A9142E139B46807344E5C5B638ED5F36FD8E47E67CEE8910896")
            .expect("expected nft offer key should parse")
    );
}

#[test]
fn protocol_ticket_keylet_from_seq_proxy_matches_current_cpp_vector() {
    let account = Uint160::from_hex("B5F762798A53D543A014CAF8B297CFF8F2F937E8")
        .expect("expected account should parse");

    assert_eq!(
        ticket_keylet_from_seq_proxy(account, SeqProxy::ticket(7)).key,
        Uint256::from_hex("38EF979A371455DF7B79A56CFB7F6840741BD83A26E07708C8964D9606909CA4")
            .expect("expected ticket key should parse")
    );
    assert_eq!(
        ticket_keylet_from_seq_proxy(account, SeqProxy::ticket(7)),
        ticket_keylet(account, 7)
    );
    assert_eq!(ticket_index(account, 7), ticket_keylet(account, 7).key);
    assert_eq!(
        ticket_index_from_seq_proxy(account, SeqProxy::ticket(7)),
        ticket_keylet(account, 7).key
    );
}

#[test]
fn protocol_signers_keylet_page_overload_matches_current_cpp_vectors() {
    let account = Uint160::from_hex("B5F762798A53D543A014CAF8B297CFF8F2F937E8")
        .expect("expected account should parse");

    assert_eq!(signers_keylet(account), signers_keylet_for_page(account, 0));
    assert_eq!(
        signers_keylet_for_page(account, 0).key,
        Uint256::from_hex("778365D5180F5DF3016817D1F318527AD7410D83F8636CF48C43E8AF72AB49BF")
            .expect("expected signer-list page 0 key should parse")
    );
    assert_eq!(
        signers_keylet_for_page(account, 1).key,
        Uint256::from_hex("3D8E92C7E441BCA1275FEC17ED9985C487F5954BBEBE6F88973AEC8CB73A19C7")
            .expect("expected signer-list page 1 key should parse")
    );
    assert_eq!(
        signers_keylet_for_page(account, 7).key,
        Uint256::from_hex("25E242F535D2208B22AD269EF38408B70D778605105C74A7F9146155B410997B")
            .expect("expected signer-list page 7 key should parse")
    );
}

#[test]
fn protocol_ledger_entry_type_catalog_matches_current_cpp_codes_and_names() {
    let catalog = ledger_entry_type_catalog();
    let codes = catalog.iter().map(|entry| entry.code).collect::<Vec<_>>();
    let mut sorted_codes = codes.clone();
    sorted_codes.sort_unstable();

    assert_eq!(codes, sorted_codes, "catalog should stay ordered by code");
    assert_eq!(
        catalog.first(),
        Some(&LedgerEntryTypeInfo {
            entry_type: protocol::LedgerEntryType::Any,
            code: 0x0000,
            name: "Any",
        })
    );
    assert_eq!(
        catalog.last(),
        Some(&LedgerEntryTypeInfo {
            entry_type: protocol::LedgerEntryType::Child,
            code: 0x1CD2,
            name: "Child",
        })
    );

    for expected in [
        LedgerEntryTypeInfo {
            entry_type: protocol::LedgerEntryType::Contract,
            code: 0x0063,
            name: "Contract",
        },
        LedgerEntryTypeInfo {
            entry_type: protocol::LedgerEntryType::GeneratorMap,
            code: 0x0067,
            name: "GeneratorMap",
        },
        LedgerEntryTypeInfo {
            entry_type: protocol::LedgerEntryType::Nickname,
            code: 0x006E,
            name: "Nickname",
        },
        LedgerEntryTypeInfo {
            entry_type: protocol::LedgerEntryType::RippleState,
            code: 0x0072,
            name: "RippleState",
        },
        LedgerEntryTypeInfo {
            entry_type: protocol::LedgerEntryType::FeeSettings,
            code: 0x0073,
            name: "FeeSettings",
        },
    ] {
        assert!(
            catalog.contains(&expected),
            "catalog should contain {expected:?}"
        );
        assert_eq!(ledger_entry_type_code(expected.entry_type), expected.code);
        assert_eq!(expected.entry_type.code(), expected.code);
        assert_eq!(expected.entry_type.as_str(), expected.name);
        assert_eq!(
            ledger_entry_type_from_code(expected.code),
            Some(expected.entry_type)
        );
        assert_eq!(
            ledger_entry_type_from_name(expected.name),
            Some(expected.entry_type)
        );
    }

    assert_eq!(
        ledger_entry_type_from_name("XChainOwnedClaimID"),
        Some(protocol::LedgerEntryType::XChainOwnedClaimId)
    );
    assert_eq!(
        ledger_entry_type_from_name("XChainOwnedCreateAccountClaimID"),
        Some(protocol::LedgerEntryType::XChainOwnedCreateAccountClaimId)
    );
    assert_eq!(ledger_entry_type_from_code(0x0062), None);
}
