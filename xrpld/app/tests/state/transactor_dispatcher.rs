use std::sync::Arc;

use app::state::application_root::apply_submit_transactor_shell;
use app::state::transactor_dispatcher::handle_real_dispatch;
use basics::base_uint::{Uint160, Uint192, Uint256};
use basics::number::NumberParts as RuntimeNumber;
use ledger::{ApplyView, ApplyViewImpl, Ledger, LedgerHeader, ReadView, pseudo_account_address};
use protocol::{
    AccountID, ApplyFlags, Asset, Currency, IOUAmount, Issue, LedgerEntryType, STAmount, STArray,
    STIssue, STLedgerEntry, STNumber, STObject, STTx, StBase, Ter, TxType, XRPAmount,
    account_keylet, amm_lpt_currency, currency_from_string, get_field_by_symbol, line,
    lsfDefaultRipple, lsfDisableMaster, lsfLoanImpaired, owner_dir_keylet,
    permissioned_domain_keylet, sf_generic, signers_keylet, tfLoanDefault, tfLoanImpair,
    tfLoanUnimpair, xrp_issue,
};
use shamap::item::SHAMapItem;
use shamap::mutation::MutableTree;
use shamap::sync::{SHAMapType, SyncState, SyncTree};
use shamap::tree_node::SHAMapNodeType;
use tx::{LSF_ONE_OWNER_COUNT, MPT_CAN_ESCROW_FLAG, MPT_CAN_TRADE_FLAG, MPT_CAN_TRANSFER_FLAG};

fn sample_account(fill: u8) -> AccountID {
    AccountID::from_array([fill; 20])
}

fn sample_uint256(fill: u8) -> Uint256 {
    Uint256::from_array([fill; 32])
}

fn raw_account_id(account: AccountID) -> Uint160 {
    Uint160::from_slice(account.data()).expect("account width")
}

fn sf(name: &str) -> &'static protocol::SField {
    get_field_by_symbol(name)
}

fn account_root(account: AccountID, owner_count: u32, flags: u32) -> STLedgerEntry {
    account_root_with_balance(account, owner_count, flags, 1_000_000)
}

fn account_root_with_balance(
    account: AccountID,
    owner_count: u32,
    flags: u32,
    balance_drops: i64,
) -> STLedgerEntry {
    let mut entry = STLedgerEntry::from_type_and_key(
        LedgerEntryType::AccountRoot,
        account_keylet(raw_account_id(account)).key,
    );
    entry.set_account_id(get_field_by_symbol("sfAccount"), account);
    entry.set_field_u32(get_field_by_symbol("sfSequence"), 1);
    entry.set_field_amount(
        get_field_by_symbol("sfBalance"),
        STAmount::from_xrp_amount(XRPAmount::from_drops(balance_drops)),
    );
    entry.set_field_u32(get_field_by_symbol("sfOwnerCount"), owner_count);
    entry.set_field_h256(get_field_by_symbol("sfPreviousTxnID"), sample_uint256(0xA1));
    entry.set_field_u32(get_field_by_symbol("sfPreviousTxnLgrSeq"), 1);
    if flags != 0 {
        entry.set_field_u32(get_field_by_symbol("sfFlags"), flags);
    }
    entry
}

fn owner_dir_root(page_owner: AccountID, child: Uint256) -> STLedgerEntry {
    let root = owner_dir_keylet(raw_account_id(page_owner));
    let mut entry = STLedgerEntry::new(root);
    entry.set_field_h256(get_field_by_symbol("sfRootIndex"), root.key);
    entry.set_field_v256(
        get_field_by_symbol("sfIndexes"),
        protocol::STVector256::from_values(get_field_by_symbol("sfIndexes"), vec![child]),
    );
    entry
}

fn signer_list_entry(account: AccountID, owner_node: u64, flags: u32) -> STLedgerEntry {
    let keylet = signers_keylet(raw_account_id(account));
    let mut entry = STLedgerEntry::from_type_and_key(LedgerEntryType::SignerList, keylet.key);
    entry.set_field_u32(get_field_by_symbol("sfSignerQuorum"), 2);
    entry.set_field_u32(get_field_by_symbol("sfSignerListID"), 0);
    entry.set_field_u64(get_field_by_symbol("sfOwnerNode"), owner_node);
    if flags != 0 {
        entry.set_field_u32(get_field_by_symbol("sfFlags"), flags);
    }

    let mut signer_entries = STArray::new(get_field_by_symbol("sfSignerEntries"));
    let mut signer_entry = STObject::make_inner_object(get_field_by_symbol("sfSignerEntry"));
    signer_entry.set_account_id(get_field_by_symbol("sfAccount"), sample_account(0x66));
    signer_entry.set_field_u16(get_field_by_symbol("sfSignerWeight"), 1);
    signer_entries.push_back(signer_entry);
    entry.set_field_array(get_field_by_symbol("sfSignerEntries"), signer_entries);
    entry
}

fn empty_ledger(entries: Vec<STLedgerEntry>) -> Ledger {
    ledger_with_header(
        LedgerHeader {
            seq: 1,
            ..LedgerHeader::default()
        },
        entries,
    )
}

fn ledger_with_header(header: LedgerHeader, entries: Vec<STLedgerEntry>) -> Ledger {
    let mut tree = MutableTree::new(1);
    for entry in entries {
        tree.add_item(
            SHAMapNodeType::AccountState,
            SHAMapItem::new(*entry.key(), entry.get_serializer().data().to_vec()),
        )
        .expect("state insert should succeed");
    }

    Ledger::from_maps(
        header,
        SyncTree::from_root_with_type(
            tree.root(),
            SHAMapType::State,
            false,
            1,
            SyncState::Immutable,
        ),
        SyncTree::new_with_type(SHAMapType::Transaction, false, 1),
    )
}

fn asset_number(asset: Asset, value: i64) -> STNumber {
    let mut number = STNumber::from(RuntimeNumber::from_i64(value));
    number.associate_asset(asset);
    number
}

fn trust_line_entry(
    low: AccountID,
    high: AccountID,
    currency: Currency,
    balance: i64,
) -> STLedgerEntry {
    let keylet = line(low, high, currency);
    let mut sle = STLedgerEntry::from_type_and_key(LedgerEntryType::RippleState, keylet.key);
    sle.set_field_amount(
        sf("sfBalance"),
        STAmount::from_iou_amount(
            sf_generic(),
            IOUAmount::from_parts(balance, 0).expect("trustline balance"),
            Issue::new(currency, low),
        ),
    );
    sle.set_field_amount(
        sf("sfLowLimit"),
        STAmount::from_iou_amount(
            sf_generic(),
            IOUAmount::from_parts(1_000_000, 0).expect("low limit"),
            Issue::new(currency, low),
        ),
    );
    sle.set_field_amount(
        sf("sfHighLimit"),
        STAmount::from_iou_amount(
            sf_generic(),
            IOUAmount::from_parts(1_000_000, 0).expect("high limit"),
            Issue::new(currency, high),
        ),
    );
    sle.set_field_u32(sf("sfFlags"), 0);
    sle
}

fn amm_vote_entry(account: AccountID, trading_fee: u16, vote_weight: u32) -> STObject {
    let mut vote = STObject::make_inner_object(sf("sfVoteEntry"));
    vote.set_account_id(sf("sfAccount"), account);
    vote.set_field_u16(sf("sfTradingFee"), trading_fee);
    vote.set_field_u32(sf("sfVoteWeight"), vote_weight);
    vote
}

fn amm_slot(account: AccountID, discounted_fee: u16) -> STObject {
    let mut slot = STObject::make_inner_object(sf("sfAuctionSlot"));
    slot.set_account_id(sf("sfAccount"), account);
    slot.set_field_u32(sf("sfExpiration"), 1_000);
    slot.set_field_u16(sf("sfDiscountedFee"), discounted_fee);
    slot
}

fn amm_entry(
    amm_account: AccountID,
    issue1: Issue,
    issue2: Issue,
    lp_balance: i64,
    vote_entries: Vec<STObject>,
    discounted_fee: u16,
) -> STLedgerEntry {
    let keylet = protocol::keylet::amm(Asset::Issue(issue1), Asset::Issue(issue2));
    let mut entry = STLedgerEntry::from_type_and_key(LedgerEntryType::AMM, keylet.key);
    entry.set_account_id(sf("sfAccount"), amm_account);
    entry.set_field_u16(sf("sfTradingFee"), 17);
    entry.set_field_amount(
        sf("sfLPTokenBalance"),
        STAmount::from_iou_amount(
            sf_generic(),
            IOUAmount::from_parts(lp_balance, 0).expect("lp balance"),
            Issue::new(
                amm_lpt_currency(issue1.currency, issue2.currency),
                amm_account,
            ),
        ),
    );
    entry.set_field_issue(
        sf("sfAsset"),
        STIssue::new_with_asset(sf("sfAsset"), Asset::Issue(issue1)),
    );
    entry.set_field_issue(
        sf("sfAsset2"),
        STIssue::new_with_asset(sf("sfAsset2"), Asset::Issue(issue2)),
    );
    let mut votes = STArray::new(sf("sfVoteSlots"));
    for vote in vote_entries {
        votes.push_back(vote);
    }
    entry.set_field_array(sf("sfVoteSlots"), votes);
    entry.set_field_object(sf("sfAuctionSlot"), amm_slot(amm_account, discounted_fee));
    entry
}

fn signer_list_set_tx(account: AccountID, quorum: u32, signers: &[(AccountID, u16)]) -> STTx {
    let mut signer_entries = STArray::new(get_field_by_symbol("sfSignerEntries"));
    for (signer_account, weight) in signers {
        let mut signer_entry = STObject::make_inner_object(get_field_by_symbol("sfSignerEntry"));
        signer_entry.set_account_id(get_field_by_symbol("sfAccount"), *signer_account);
        signer_entry.set_field_u16(get_field_by_symbol("sfSignerWeight"), *weight);
        signer_entries.push_back(signer_entry);
    }

    STTx::new(TxType::SIGNER_LIST_SET, move |object| {
        object.set_account_id(get_field_by_symbol("sfAccount"), account);
        object.set_field_u32(get_field_by_symbol("sfSignerQuorum"), quorum);
        if !signers.is_empty() {
            object.set_field_array(get_field_by_symbol("sfSignerEntries"), signer_entries);
        }
        object.set_field_amount(
            get_field_by_symbol("sfFee"),
            STAmount::from_xrp_amount(XRPAmount::from_drops(10)),
        );
    })
}

fn delegate_permissions(permissions: &[u32]) -> STArray {
    let mut permission_entries = STArray::new(get_field_by_symbol("sfPermissions"));
    for permission in permissions {
        let mut entry = STObject::make_inner_object(get_field_by_symbol("sfPermission"));
        entry.set_field_u32(get_field_by_symbol("sfPermissionValue"), *permission);
        permission_entries.push_back(entry);
    }
    permission_entries
}

fn delegate_set_tx(account: AccountID, authorize: AccountID, permissions: &[u32]) -> STTx {
    let permission_entries = delegate_permissions(permissions);
    STTx::new(TxType::DELEGATE_SET, move |object| {
        object.set_account_id(get_field_by_symbol("sfAccount"), account);
        object.set_account_id(get_field_by_symbol("sfAuthorize"), authorize);
        object.set_field_array(
            get_field_by_symbol("sfPermissions"),
            permission_entries.clone(),
        );
        object.set_field_amount(
            get_field_by_symbol("sfFee"),
            STAmount::from_xrp_amount(XRPAmount::from_drops(10)),
        );
        object.set_field_u32(get_field_by_symbol("sfSequence"), 1);
    })
}

fn delegate_entry(
    account: AccountID,
    authorize: AccountID,
    permissions: &[u32],
    owner_node: u64,
    destination_node: u64,
) -> STLedgerEntry {
    let keylet = protocol::delegate_keylet(raw_account_id(account), raw_account_id(authorize));
    let mut entry = STLedgerEntry::from_type_and_key(LedgerEntryType::Delegate, keylet.key);
    entry.set_account_id(get_field_by_symbol("sfAccount"), account);
    entry.set_account_id(get_field_by_symbol("sfAuthorize"), authorize);
    entry.set_field_array(
        get_field_by_symbol("sfPermissions"),
        delegate_permissions(permissions),
    );
    entry.set_field_u64(get_field_by_symbol("sfOwnerNode"), owner_node);
    entry.set_field_u64(get_field_by_symbol("sfDestinationNode"), destination_node);
    entry
}

fn payment_tx(sequence: u32) -> STTx {
    STTx::new(TxType::PAYMENT, |tx| {
        tx.set_account_id(get_field_by_symbol("sfAccount"), sample_account(0x71));
        tx.set_account_id(get_field_by_symbol("sfDestination"), sample_account(0x72));
        tx.set_field_amount(
            get_field_by_symbol("sfAmount"),
            STAmount::from_xrp_amount(XRPAmount::from_drops(1_000)),
        );
        tx.set_field_amount(
            get_field_by_symbol("sfFee"),
            STAmount::from_xrp_amount(XRPAmount::from_drops(10)),
        );
        tx.set_field_u32(get_field_by_symbol("sfSequence"), sequence);
    })
}

fn inner_batch_payment_tx(
    account: AccountID,
    destination: AccountID,
    sequence: u32,
    amount_drops: i64,
) -> STTx {
    STTx::new(TxType::PAYMENT, move |tx| {
        tx.set_account_id(get_field_by_symbol("sfAccount"), account);
        tx.set_account_id(get_field_by_symbol("sfDestination"), destination);
        tx.set_field_amount(
            get_field_by_symbol("sfAmount"),
            STAmount::from_xrp_amount(XRPAmount::from_drops(amount_drops)),
        );
        tx.set_field_amount(
            get_field_by_symbol("sfFee"),
            STAmount::from_xrp_amount(XRPAmount::from_drops(0)),
        );
        tx.set_field_u32(get_field_by_symbol("sfSequence"), sequence);
        tx.set_field_u32(
            get_field_by_symbol("sfFlags"),
            protocol::INNER_BATCH_TRANSACTION_FLAG,
        );
        tx.set_field_vl(get_field_by_symbol("sfSigningPubKey"), &[]);
    })
}

fn batch_raw_transaction(tx: &STTx) -> STObject {
    let mut raw = tx.clone_as_object();
    raw.set_fname(get_field_by_symbol("sfRawTransaction"));
    raw
}

fn batch_tx_with_inner(account: AccountID, flags: u32, inner: &[STTx]) -> STTx {
    let mut raw_transactions = STArray::new(get_field_by_symbol("sfRawTransactions"));
    for tx in inner {
        raw_transactions.push_back(batch_raw_transaction(tx));
    }

    STTx::new(TxType::BATCH, move |tx| {
        tx.set_account_id(get_field_by_symbol("sfAccount"), account);
        tx.set_field_amount(
            get_field_by_symbol("sfFee"),
            STAmount::from_xrp_amount(XRPAmount::from_drops(10)),
        );
        tx.set_field_u32(get_field_by_symbol("sfSequence"), 1);
        tx.set_field_u32(get_field_by_symbol("sfFlags"), flags);
        tx.set_field_array(
            get_field_by_symbol("sfRawTransactions"),
            raw_transactions.clone(),
        );
    })
}

fn batch_tx(account: AccountID) -> STTx {
    let first = payment_tx(3);
    let second = payment_tx(4);
    batch_tx_with_inner(account, protocol::tfAllOrNothing, &[first, second])
}

fn permissioned_domain_credentials(entries: &[(AccountID, &[u8])]) -> STArray {
    let mut credentials = STArray::new(get_field_by_symbol("sfAcceptedCredentials"));
    for (issuer, credential_type) in entries {
        let mut credential = STObject::make_inner_object(get_field_by_symbol("sfCredential"));
        credential.set_account_id(get_field_by_symbol("sfIssuer"), *issuer);
        credential.set_field_vl(get_field_by_symbol("sfCredentialType"), credential_type);
        credentials.push_back(credential);
    }
    credentials
}

fn permissioned_domain_entry(
    owner: AccountID,
    seq: u32,
    owner_node: u64,
    credentials: &[(AccountID, &[u8])],
) -> STLedgerEntry {
    let mut entry = STLedgerEntry::new(permissioned_domain_keylet(raw_account_id(owner), seq));
    entry.set_account_id(get_field_by_symbol("sfOwner"), owner);
    entry.set_field_u32(get_field_by_symbol("sfSequence"), seq);
    entry.set_field_array(
        get_field_by_symbol("sfAcceptedCredentials"),
        permissioned_domain_credentials(credentials),
    );
    entry.set_field_u64(get_field_by_symbol("sfOwnerNode"), owner_node);
    entry
}

fn permissioned_domain_set_tx(
    account: AccountID,
    sequence: u32,
    domain_id: Option<Uint256>,
    credentials: &[(AccountID, &[u8])],
) -> STTx {
    let accepted_credentials = permissioned_domain_credentials(credentials);
    STTx::new(TxType::PERMISSIONED_DOMAIN_SET, move |object| {
        object.set_account_id(get_field_by_symbol("sfAccount"), account);
        object.set_field_u32(get_field_by_symbol("sfSequence"), sequence);
        object.set_field_array(
            get_field_by_symbol("sfAcceptedCredentials"),
            accepted_credentials,
        );
        object.set_field_amount(
            get_field_by_symbol("sfFee"),
            STAmount::from_xrp_amount(XRPAmount::from_drops(10)),
        );
        if let Some(domain_id) = domain_id {
            object.set_field_h256(get_field_by_symbol("sfDomainID"), domain_id);
        }
    })
}

fn permissioned_domain_delete_tx(account: AccountID, domain_id: Uint256) -> STTx {
    STTx::new(TxType::PERMISSIONED_DOMAIN_DELETE, move |object| {
        object.set_account_id(get_field_by_symbol("sfAccount"), account);
        object.set_field_h256(get_field_by_symbol("sfDomainID"), domain_id);
        object.set_field_amount(
            get_field_by_symbol("sfFee"),
            STAmount::from_xrp_amount(XRPAmount::from_drops(10)),
        );
    })
}

fn nft_page_token(token_id: Uint256) -> STObject {
    let mut token = STObject::new(get_field_by_symbol("sfNFToken"));
    token.set_field_h256(get_field_by_symbol("sfNFTokenID"), token_id);
    token
}

fn nft_page_entry(
    keylet: protocol::Keylet,
    token_id: Uint256,
    previous: Option<Uint256>,
    next: Option<Uint256>,
) -> STLedgerEntry {
    let mut entry = STLedgerEntry::new(keylet);
    let mut tokens = STArray::new(get_field_by_symbol("sfNFTokens"));
    tokens.push_back(nft_page_token(token_id));
    entry.set_field_array(get_field_by_symbol("sfNFTokens"), tokens);
    if let Some(previous) = previous {
        entry.set_field_h256(get_field_by_symbol("sfPreviousPageMin"), previous);
    }
    if let Some(next) = next {
        entry.set_field_h256(get_field_by_symbol("sfNextPageMin"), next);
    }
    entry
}

fn ledger_state_fix_tx(account: AccountID, owner: Option<AccountID>, fix_type: u16) -> STTx {
    STTx::new(TxType::LEDGER_STATE_FIX, move |object| {
        object.set_account_id(get_field_by_symbol("sfAccount"), account);
        object.set_field_u16(get_field_by_symbol("sfLedgerFixType"), fix_type);
        object.set_field_amount(
            get_field_by_symbol("sfFee"),
            STAmount::from_xrp_amount(XRPAmount::from_drops(10)),
        );
        object.set_field_u32(get_field_by_symbol("sfSequence"), 1);
        if let Some(owner) = owner {
            object.set_account_id(get_field_by_symbol("sfOwner"), owner);
        }
    })
}

fn vault_entry(owner: AccountID, pseudo: AccountID, seq: u32, asset: Asset) -> STLedgerEntry {
    let keylet = protocol::vault_keylet(raw_account_id(owner), seq);
    let mut entry = STLedgerEntry::from_type_and_key(LedgerEntryType::Vault, keylet.key);
    entry.set_field_u32(get_field_by_symbol("sfFlags"), 0);
    entry.set_field_h256(get_field_by_symbol("sfPreviousTxnID"), sample_uint256(0xB1));
    entry.set_field_u32(get_field_by_symbol("sfPreviousTxnLgrSeq"), 1);
    entry.set_field_u32(get_field_by_symbol("sfSequence"), seq);
    entry.set_field_u64(get_field_by_symbol("sfOwnerNode"), 0);
    entry.set_account_id(get_field_by_symbol("sfOwner"), owner);
    entry.set_account_id(get_field_by_symbol("sfAccount"), pseudo);
    entry.set_field_issue(
        get_field_by_symbol("sfAsset"),
        STIssue::new_with_asset(get_field_by_symbol("sfAsset"), asset),
    );
    entry.set_field_number(get_field_by_symbol("sfAssetsTotal"), asset_number(asset, 0));
    entry.set_field_number(
        get_field_by_symbol("sfAssetsAvailable"),
        asset_number(asset, 0),
    );
    entry.set_field_number(
        get_field_by_symbol("sfLossUnrealized"),
        asset_number(asset, 0),
    );
    entry
}

fn share_id_for(account: AccountID, sequence: u32) -> Uint192 {
    let mut bytes = [0u8; 24];
    bytes[..4].copy_from_slice(&sequence.to_be_bytes());
    bytes[4..].copy_from_slice(account.data());
    Uint192::from_slice(&bytes).expect("share id width")
}

fn vault_entry_with_share(
    owner: AccountID,
    pseudo: AccountID,
    seq: u32,
    asset: Asset,
    share_id: Uint192,
) -> STLedgerEntry {
    let mut entry = vault_entry(owner, pseudo, seq, asset);
    entry.set_field_h192(get_field_by_symbol("sfShareMPTID"), share_id);
    entry
}

fn mpt_issuance_entry(
    issuer: AccountID,
    sequence: u32,
    outstanding_amount: u64,
    flags: u32,
) -> STLedgerEntry {
    let keylet = protocol::mpt_issuance_keylet(sequence, raw_account_id(issuer));
    let mut entry = STLedgerEntry::new(keylet);
    entry.set_account_id(get_field_by_symbol("sfIssuer"), issuer);
    entry.set_field_u32(get_field_by_symbol("sfSequence"), sequence);
    entry.set_field_u64(
        get_field_by_symbol("sfOutstandingAmount"),
        outstanding_amount,
    );
    entry.set_field_u32(get_field_by_symbol("sfFlags"), flags);
    entry.set_field_u64(get_field_by_symbol("sfOwnerNode"), 0);
    entry
}

fn mptoken_entry(account: AccountID, share_id: Uint192, amount: u64) -> STLedgerEntry {
    let keylet = protocol::mptoken_keylet_from_mptid(share_id, raw_account_id(account));
    let mut entry = STLedgerEntry::new(keylet);
    entry.set_account_id(get_field_by_symbol("sfAccount"), account);
    entry.set_field_h192(get_field_by_symbol("sfMPTokenIssuanceID"), share_id);
    entry.set_field_u64(get_field_by_symbol("sfMPTAmount"), amount);
    entry.set_field_u32(get_field_by_symbol("sfFlags"), 0);
    entry.set_field_u64(get_field_by_symbol("sfOwnerNode"), 0);
    entry
}

fn vault_create_tx(account: AccountID, asset: Asset, sequence: u32) -> STTx {
    STTx::new(TxType::VAULT_CREATE, move |object| {
        object.set_account_id(get_field_by_symbol("sfAccount"), account);
        object.set_field_amount(
            get_field_by_symbol("sfAsset"),
            asset.amount(RuntimeNumber::zero()).expect("asset zero"),
        );
        object.set_field_amount(
            get_field_by_symbol("sfFee"),
            STAmount::from_xrp_amount(XRPAmount::from_drops(10)),
        );
        object.set_field_u32(get_field_by_symbol("sfSequence"), sequence);
    })
}

fn vault_set_tx(account: AccountID, vault_id: Uint256, assets_maximum_drops: i64) -> STTx {
    STTx::new(TxType::VAULT_SET, move |object| {
        object.set_account_id(get_field_by_symbol("sfAccount"), account);
        object.set_field_h256(get_field_by_symbol("sfVaultID"), vault_id);
        object.set_field_amount(
            get_field_by_symbol("sfAssetsMaximum"),
            STAmount::from_xrp_amount(XRPAmount::from_drops(assets_maximum_drops)),
        );
        object.set_field_amount(
            get_field_by_symbol("sfFee"),
            STAmount::from_xrp_amount(XRPAmount::from_drops(10)),
        );
    })
}

fn vault_delete_tx(account: AccountID, vault_id: Uint256) -> STTx {
    STTx::new(TxType::VAULT_DELETE, move |object| {
        object.set_account_id(get_field_by_symbol("sfAccount"), account);
        object.set_field_h256(get_field_by_symbol("sfVaultID"), vault_id);
        object.set_field_amount(
            get_field_by_symbol("sfFee"),
            STAmount::from_xrp_amount(XRPAmount::from_drops(10)),
        );
    })
}

fn vault_deposit_tx(account: AccountID, vault_id: Uint256, amount_drops: i64) -> STTx {
    STTx::new(TxType::VAULT_DEPOSIT, move |object| {
        object.set_account_id(get_field_by_symbol("sfAccount"), account);
        object.set_field_h256(get_field_by_symbol("sfVaultID"), vault_id);
        object.set_field_amount(
            get_field_by_symbol("sfAmount"),
            STAmount::from_xrp_amount(XRPAmount::from_drops(amount_drops)),
        );
        object.set_field_amount(
            get_field_by_symbol("sfFee"),
            STAmount::from_xrp_amount(XRPAmount::from_drops(10)),
        );
    })
}

fn vault_withdraw_share_tx(
    account: AccountID,
    vault_id: Uint256,
    share_id: Uint192,
    amount: i64,
) -> STTx {
    STTx::new(TxType::VAULT_WITHDRAW, move |object| {
        object.set_account_id(get_field_by_symbol("sfAccount"), account);
        object.set_field_h256(get_field_by_symbol("sfVaultID"), vault_id);
        object.set_field_amount(
            get_field_by_symbol("sfAmount"),
            STAmount::from_mpt_amount(
                get_field_by_symbol("sfAmount"),
                protocol::MPTAmount::from_value(amount),
                protocol::MPTIssue::new(share_id),
            ),
        );
        object.set_field_amount(
            get_field_by_symbol("sfFee"),
            STAmount::from_xrp_amount(XRPAmount::from_drops(10)),
        );
    })
}

fn vault_clawback_share_tx(
    account: AccountID,
    holder: AccountID,
    vault_id: Uint256,
    share_id: Uint192,
) -> STTx {
    STTx::new(TxType::VAULT_CLAWBACK, move |object| {
        object.set_account_id(get_field_by_symbol("sfAccount"), account);
        object.set_account_id(get_field_by_symbol("sfHolder"), holder);
        object.set_field_h256(get_field_by_symbol("sfVaultID"), vault_id);
        object.set_field_amount(
            get_field_by_symbol("sfAmount"),
            STAmount::from_mpt_amount(
                get_field_by_symbol("sfAmount"),
                protocol::MPTAmount::new(),
                protocol::MPTIssue::new(share_id),
            ),
        );
        object.set_field_amount(
            get_field_by_symbol("sfFee"),
            STAmount::from_xrp_amount(XRPAmount::from_drops(10)),
        );
    })
}

fn loan_broker_set_tx(account: AccountID, vault_id: Uint256, sequence: u32) -> STTx {
    STTx::new(TxType::LOAN_BROKER_SET, move |object| {
        object.set_account_id(get_field_by_symbol("sfAccount"), account);
        object.set_field_h256(get_field_by_symbol("sfVaultID"), vault_id);
        object.set_field_u32(get_field_by_symbol("sfSequence"), sequence);
        object.set_field_amount(
            get_field_by_symbol("sfFee"),
            STAmount::from_xrp_amount(XRPAmount::from_drops(10)),
        );
    })
}

fn loan_broker_delete_tx(account: AccountID, broker_id: Uint256) -> STTx {
    STTx::new(TxType::LOAN_BROKER_DELETE, move |object| {
        object.set_account_id(get_field_by_symbol("sfAccount"), account);
        object.set_field_h256(get_field_by_symbol("sfLoanBrokerID"), broker_id);
        object.set_field_amount(
            get_field_by_symbol("sfFee"),
            STAmount::from_xrp_amount(XRPAmount::from_drops(10)),
        );
    })
}

fn loan_delete_tx(account: AccountID, loan_id: Uint256) -> STTx {
    STTx::new(TxType::LOAN_DELETE, move |object| {
        object.set_account_id(get_field_by_symbol("sfAccount"), account);
        object.set_field_h256(get_field_by_symbol("sfLoanID"), loan_id);
        object.set_field_amount(
            get_field_by_symbol("sfFee"),
            STAmount::from_xrp_amount(XRPAmount::from_drops(10)),
        );
    })
}

fn loan_manage_tx(account: AccountID, loan_id: Uint256, flags: u32) -> STTx {
    STTx::new(TxType::LOAN_MANAGE, move |object| {
        object.set_account_id(get_field_by_symbol("sfAccount"), account);
        object.set_field_h256(get_field_by_symbol("sfLoanID"), loan_id);
        object.set_field_u32(get_field_by_symbol("sfFlags"), flags);
        object.set_field_amount(
            get_field_by_symbol("sfFee"),
            STAmount::from_xrp_amount(XRPAmount::from_drops(10)),
        );
    })
}

fn loan_set_tx(
    account: AccountID,
    broker_id: Uint256,
    principal_requested_drops: i64,
    sequence: u32,
) -> STTx {
    STTx::new(TxType::LOAN_SET, move |object| {
        object.set_account_id(get_field_by_symbol("sfAccount"), account);
        object.set_field_h256(get_field_by_symbol("sfLoanBrokerID"), broker_id);
        object.set_field_amount(
            get_field_by_symbol("sfPrincipalRequested"),
            STAmount::from_xrp_amount(XRPAmount::from_drops(principal_requested_drops)),
        );
        object.set_field_u32(get_field_by_symbol("sfPaymentInterval"), 60);
        object.set_field_u32(get_field_by_symbol("sfPaymentTotal"), 1);
        object.set_field_u32(get_field_by_symbol("sfInterestRate"), 0);
        object.set_field_amount(
            get_field_by_symbol("sfFee"),
            STAmount::from_xrp_amount(XRPAmount::from_drops(10)),
        );
        object.set_field_u32(get_field_by_symbol("sfSequence"), sequence);
    })
}

fn loan_set_tx_with_origination_fee(
    account: AccountID,
    broker_id: Uint256,
    principal_requested_drops: i64,
    origination_fee_drops: i64,
    sequence: u32,
) -> STTx {
    STTx::new(TxType::LOAN_SET, move |object| {
        object.set_account_id(get_field_by_symbol("sfAccount"), account);
        object.set_field_h256(get_field_by_symbol("sfLoanBrokerID"), broker_id);
        object.set_field_amount(
            get_field_by_symbol("sfPrincipalRequested"),
            STAmount::from_xrp_amount(XRPAmount::from_drops(principal_requested_drops)),
        );
        object.set_field_amount(
            get_field_by_symbol("sfLoanOriginationFee"),
            STAmount::from_xrp_amount(XRPAmount::from_drops(origination_fee_drops)),
        );
        object.set_field_u32(get_field_by_symbol("sfPaymentInterval"), 60);
        object.set_field_u32(get_field_by_symbol("sfPaymentTotal"), 1);
        object.set_field_u32(get_field_by_symbol("sfInterestRate"), 0);
        object.set_field_amount(
            get_field_by_symbol("sfFee"),
            STAmount::from_xrp_amount(XRPAmount::from_drops(10)),
        );
        object.set_field_u32(get_field_by_symbol("sfSequence"), sequence);
    })
}

fn loan_entry(
    loan_id: Uint256,
    borrower: AccountID,
    broker_id: Uint256,
    broker_node: u64,
    owner_node: u64,
) -> STLedgerEntry {
    let mut entry = STLedgerEntry::from_type_and_key(
        LedgerEntryType::Loan,
        protocol::loan_keylet_from_key(loan_id).key,
    );
    entry.set_field_h256(get_field_by_symbol("sfPreviousTxnID"), sample_uint256(0xC1));
    entry.set_field_u32(get_field_by_symbol("sfPreviousTxnLgrSeq"), 1);
    entry.set_field_u64(get_field_by_symbol("sfLoanBrokerNode"), broker_node);
    entry.set_field_u64(get_field_by_symbol("sfOwnerNode"), owner_node);
    entry.set_field_h256(get_field_by_symbol("sfLoanBrokerID"), broker_id);
    entry.set_field_u32(get_field_by_symbol("sfLoanSequence"), 1);
    entry.set_account_id(get_field_by_symbol("sfBorrower"), borrower);
    entry.set_field_u32(get_field_by_symbol("sfPaymentRemaining"), 0);
    entry
}

fn loan_broker_entry(
    broker_id: Uint256,
    owner: AccountID,
    pseudo: AccountID,
    vault_id: Uint256,
    asset: Asset,
    debt_total: i64,
    cover_available: i64,
    cover_rate_minimum: u32,
    cover_rate_liquidation: u32,
) -> STLedgerEntry {
    let mut entry = STLedgerEntry::from_type_and_key(
        LedgerEntryType::LoanBroker,
        protocol::loan_broker_keylet_from_key(broker_id).key,
    );
    entry.set_field_h256(get_field_by_symbol("sfPreviousTxnID"), sample_uint256(0xD1));
    entry.set_field_u32(get_field_by_symbol("sfPreviousTxnLgrSeq"), 1);
    entry.set_field_h256(get_field_by_symbol("sfVaultID"), vault_id);
    entry.set_account_id(get_field_by_symbol("sfOwner"), owner);
    entry.set_account_id(get_field_by_symbol("sfAccount"), pseudo);
    entry.set_field_u32(get_field_by_symbol("sfLoanSequence"), 1);
    entry.set_field_u32(get_field_by_symbol("sfOwnerCount"), 0);
    entry.set_field_number(
        get_field_by_symbol("sfDebtTotal"),
        asset_number(asset, debt_total),
    );
    entry.set_field_number(
        get_field_by_symbol("sfCoverAvailable"),
        asset_number(asset, cover_available),
    );
    entry.set_field_u32(
        get_field_by_symbol("sfCoverRateMinimum"),
        cover_rate_minimum,
    );
    entry.set_field_u32(
        get_field_by_symbol("sfCoverRateLiquidation"),
        cover_rate_liquidation,
    );
    entry
}

fn managed_vault_entry(
    owner: AccountID,
    pseudo: AccountID,
    seq: u32,
    asset: Asset,
    assets_total: i64,
    assets_available: i64,
    loss_unrealized: i64,
) -> STLedgerEntry {
    let mut entry = vault_entry(owner, pseudo, seq, asset);
    entry.set_field_number(
        get_field_by_symbol("sfAssetsTotal"),
        asset_number(asset, assets_total),
    );
    entry.set_field_number(
        get_field_by_symbol("sfAssetsAvailable"),
        asset_number(asset, assets_available),
    );
    entry.set_field_number(
        get_field_by_symbol("sfLossUnrealized"),
        asset_number(asset, loss_unrealized),
    );
    entry
}

fn managed_loan_entry(
    loan_id: Uint256,
    borrower: AccountID,
    broker_id: Uint256,
    total_value_outstanding: i64,
    principal_outstanding: i64,
    management_fee_outstanding: i64,
    payment_remaining: u32,
    next_payment_due_date: u32,
) -> STLedgerEntry {
    let asset = Asset::Issue(xrp_issue());
    let mut entry = loan_entry(loan_id, borrower, broker_id, 0, 0);
    entry.set_field_i32(get_field_by_symbol("sfLoanScale"), 0);
    entry.set_field_number(
        get_field_by_symbol("sfTotalValueOutstanding"),
        asset_number(asset, total_value_outstanding),
    );
    entry.set_field_number(
        get_field_by_symbol("sfPrincipalOutstanding"),
        asset_number(asset, principal_outstanding),
    );
    entry.set_field_number(
        get_field_by_symbol("sfManagementFeeOutstanding"),
        asset_number(asset, management_fee_outstanding),
    );
    entry.set_field_u32(get_field_by_symbol("sfPaymentRemaining"), payment_remaining);
    entry.set_field_u32(
        get_field_by_symbol("sfNextPaymentDueDate"),
        next_payment_due_date,
    );
    entry.set_field_u32(get_field_by_symbol("sfPreviousPaymentDueDate"), 120);
    entry.set_field_u32(get_field_by_symbol("sfStartDate"), 100);
    entry.set_field_u32(get_field_by_symbol("sfPaymentInterval"), 30);
    entry.set_field_u32(get_field_by_symbol("sfGracePeriod"), 0);
    entry.set_field_number(
        get_field_by_symbol("sfPeriodicPayment"),
        asset_number(asset, 0),
    );
    entry
}

#[test]
fn signer_list_set_create_populates_signer_sle_and_owner_dir() {
    let account = sample_account(0x11);
    let signer_a = sample_account(0x44);
    let signer_b = sample_account(0x33);
    let ledger = empty_ledger(vec![account_root(account, 0, 0)]);
    let mut view = ApplyViewImpl::new(Arc::new(ledger), ApplyFlags::NONE);
    let sttx = signer_list_set_tx(account, 3, &[(signer_a, 1), (signer_b, 2)]);

    let result = handle_real_dispatch(&mut view, &sttx, TxType::SIGNER_LIST_SET, None);

    assert_eq!(result, protocol::Ter::TES_SUCCESS);

    let signer_list = view
        .read(signers_keylet(raw_account_id(account)))
        .expect("signer list read should succeed")
        .expect("signer list should exist");
    assert_eq!(
        signer_list.get_field_u32(get_field_by_symbol("sfSignerQuorum")),
        3
    );
    assert_eq!(
        signer_list.get_field_u32(get_field_by_symbol("sfSignerListID")),
        0
    );
    assert_eq!(
        signer_list.get_field_u32(get_field_by_symbol("sfFlags")),
        LSF_ONE_OWNER_COUNT
    );
    assert_eq!(
        signer_list.get_field_u64(get_field_by_symbol("sfOwnerNode")),
        0
    );

    let signer_entries = signer_list.get_field_array(get_field_by_symbol("sfSignerEntries"));
    assert_eq!(signer_entries.len(), 2);
    assert_eq!(
        signer_entries
            .get(0)
            .expect("first signer")
            .get_account_id(get_field_by_symbol("sfAccount")),
        signer_b
    );
    assert_eq!(
        signer_entries
            .get(1)
            .expect("second signer")
            .get_account_id(get_field_by_symbol("sfAccount")),
        signer_a
    );

    let account_sle = view
        .read(account_keylet(raw_account_id(account)))
        .expect("account read should succeed")
        .expect("account should exist");
    assert_eq!(
        account_sle.get_field_u32(get_field_by_symbol("sfOwnerCount")),
        1
    );

    let owner_dir = view
        .read(owner_dir_keylet(raw_account_id(account)))
        .expect("owner dir read should succeed")
        .expect("owner dir should exist");
    assert_eq!(
        owner_dir
            .get_field_v256(get_field_by_symbol("sfIndexes"))
            .value(),
        &[signer_list.key().to_owned()]
    );
}

#[test]
fn signer_list_set_destroy_removes_existing_signer_list() {
    let account = sample_account(0x21);
    let signer_list = signer_list_entry(account, 0, LSF_ONE_OWNER_COUNT);
    let ledger = empty_ledger(vec![
        account_root(account, 1, 0),
        owner_dir_root(account, signer_list.key().to_owned()),
        signer_list,
    ]);
    let mut view = ApplyViewImpl::new(Arc::new(ledger), ApplyFlags::NONE);
    let sttx = signer_list_set_tx(account, 0, &[]);

    let result = handle_real_dispatch(&mut view, &sttx, TxType::SIGNER_LIST_SET, None);

    assert_eq!(result, protocol::Ter::TES_SUCCESS);
    assert!(
        view.read(signers_keylet(raw_account_id(account)))
            .expect("signer list read should succeed")
            .is_none()
    );
    let account_sle = view
        .read(account_keylet(raw_account_id(account)))
        .expect("account read should succeed")
        .expect("account should exist");
    assert_eq!(
        account_sle.get_field_u32(get_field_by_symbol("sfOwnerCount")),
        0
    );
}

#[test]
fn signer_list_set_destroy_requires_alternative_key_when_master_disabled() {
    let account = sample_account(0x31);
    let signer_list = signer_list_entry(account, 0, LSF_ONE_OWNER_COUNT);
    let ledger = empty_ledger(vec![
        account_root(account, 1, lsfDisableMaster),
        owner_dir_root(account, signer_list.key().to_owned()),
        signer_list,
    ]);
    let mut view = ApplyViewImpl::new(Arc::new(ledger), ApplyFlags::NONE);
    let sttx = signer_list_set_tx(account, 0, &[]);

    let result = handle_real_dispatch(&mut view, &sttx, TxType::SIGNER_LIST_SET, None);

    assert_eq!(result, protocol::Ter::TEC_NO_ALTERNATIVE_KEY);
    assert!(
        view.read(signers_keylet(raw_account_id(account)))
            .expect("signer list read should succeed")
            .is_some()
    );
}

#[test]
fn permissioned_domain_set_dispatch_creates_sorted_domain_entry() {
    let account = sample_account(0x41);
    let issuer_a = sample_account(0x10);
    let issuer_b = sample_account(0x20);
    let ledger = empty_ledger(vec![account_root(account, 0, 0)]);
    let mut view = ApplyViewImpl::new(Arc::new(ledger), ApplyFlags::NONE);
    let sttx = permissioned_domain_set_tx(
        account,
        9,
        None,
        &[
            (issuer_b, b"beta"),
            (issuer_a, b"zeta"),
            (issuer_a, b"alpha"),
        ],
    );

    let result = handle_real_dispatch(&mut view, &sttx, TxType::PERMISSIONED_DOMAIN_SET, None);

    assert_eq!(result, protocol::Ter::TES_SUCCESS);

    let domain = view
        .read(permissioned_domain_keylet(raw_account_id(account), 9))
        .expect("domain read should succeed")
        .expect("domain should exist");
    assert_eq!(
        domain.get_account_id(get_field_by_symbol("sfOwner")),
        account
    );
    assert_eq!(domain.get_field_u32(get_field_by_symbol("sfSequence")), 9);
    assert_eq!(domain.get_field_u64(get_field_by_symbol("sfOwnerNode")), 0);

    let credentials = domain.get_field_array(get_field_by_symbol("sfAcceptedCredentials"));
    assert_eq!(credentials.len(), 3);
    assert_eq!(
        credentials
            .get(0)
            .expect("first credential")
            .get_account_id(get_field_by_symbol("sfIssuer")),
        issuer_a
    );
    assert_eq!(
        credentials
            .get(0)
            .expect("first credential")
            .get_field_vl(get_field_by_symbol("sfCredentialType")),
        b"alpha"
    );
    assert_eq!(
        credentials
            .get(1)
            .expect("second credential")
            .get_account_id(get_field_by_symbol("sfIssuer")),
        issuer_a
    );
    assert_eq!(
        credentials
            .get(1)
            .expect("second credential")
            .get_field_vl(get_field_by_symbol("sfCredentialType")),
        b"zeta"
    );
    assert_eq!(
        credentials
            .get(2)
            .expect("third credential")
            .get_account_id(get_field_by_symbol("sfIssuer")),
        issuer_b
    );

    let owner = view
        .read(account_keylet(raw_account_id(account)))
        .expect("owner read should succeed")
        .expect("owner should exist");
    assert_eq!(owner.get_field_u32(get_field_by_symbol("sfOwnerCount")), 1);

    let owner_dir = view
        .read(owner_dir_keylet(raw_account_id(account)))
        .expect("owner dir read should succeed")
        .expect("owner dir should exist");
    assert_eq!(
        owner_dir
            .get_field_v256(get_field_by_symbol("sfIndexes"))
            .value(),
        &[domain.key().to_owned()]
    );
}

#[test]
fn permissioned_domain_set_dispatch_updates_existing_domain() {
    let account = sample_account(0x51);
    let issuer_a = sample_account(0x12);
    let issuer_b = sample_account(0x34);
    let existing = permissioned_domain_entry(account, 7, 0, &[(issuer_b, b"legacy")]);
    let ledger = empty_ledger(vec![
        account_root(account, 1, 0),
        owner_dir_root(account, existing.key().to_owned()),
        existing.clone(),
    ]);
    let mut view = ApplyViewImpl::new(Arc::new(ledger), ApplyFlags::NONE);
    let sttx = permissioned_domain_set_tx(
        account,
        11,
        Some(existing.key().to_owned()),
        &[(issuer_b, b"beta"), (issuer_a, b"alpha")],
    );

    let result = handle_real_dispatch(&mut view, &sttx, TxType::PERMISSIONED_DOMAIN_SET, None);

    assert_eq!(result, protocol::Ter::TES_SUCCESS);

    let updated = view
        .read(protocol::permissioned_domain_keylet_from_id(
            existing.key().to_owned(),
        ))
        .expect("updated domain read should succeed")
        .expect("updated domain should exist");
    let credentials = updated.get_field_array(get_field_by_symbol("sfAcceptedCredentials"));
    assert_eq!(credentials.len(), 2);
    assert_eq!(
        credentials
            .get(0)
            .expect("first credential")
            .get_account_id(get_field_by_symbol("sfIssuer")),
        issuer_a
    );
    assert_eq!(
        credentials
            .get(0)
            .expect("first credential")
            .get_field_vl(get_field_by_symbol("sfCredentialType")),
        b"alpha"
    );
    assert_eq!(
        credentials
            .get(1)
            .expect("second credential")
            .get_account_id(get_field_by_symbol("sfIssuer")),
        issuer_b
    );

    let owner = view
        .read(account_keylet(raw_account_id(account)))
        .expect("owner read should succeed")
        .expect("owner should exist");
    assert_eq!(owner.get_field_u32(get_field_by_symbol("sfOwnerCount")), 1);
}

#[test]
fn permissioned_domain_delete_dispatch_removes_loaded_domain() {
    let account = sample_account(0x61);
    let domain = permissioned_domain_entry(account, 13, 0, &[(sample_account(0x71), b"alpha")]);
    let ledger = empty_ledger(vec![
        account_root(account, 1, 0),
        owner_dir_root(account, domain.key().to_owned()),
        domain.clone(),
    ]);
    let mut view = ApplyViewImpl::new(Arc::new(ledger), ApplyFlags::NONE);
    let sttx = permissioned_domain_delete_tx(account, domain.key().to_owned());

    let result = handle_real_dispatch(&mut view, &sttx, TxType::PERMISSIONED_DOMAIN_DELETE, None);

    assert_eq!(result, protocol::Ter::TES_SUCCESS);
    assert!(
        view.read(protocol::permissioned_domain_keylet_from_id(
            domain.key().to_owned()
        ))
        .expect("domain read should succeed")
        .is_none()
    );
    let owner = view
        .read(account_keylet(raw_account_id(account)))
        .expect("owner read should succeed")
        .expect("owner should exist");
    assert_eq!(owner.get_field_u32(get_field_by_symbol("sfOwnerCount")), 0);
    let owner_dir = view
        .read(owner_dir_keylet(raw_account_id(account)))
        .expect("owner dir read should succeed")
        .expect("owner dir should remain");
    assert!(
        owner_dir
            .get_field_v256(get_field_by_symbol("sfIndexes"))
            .value()
            .is_empty()
    );
}

#[test]
fn ledger_state_fix_repairs_broken_nft_page_links_through_dispatch() {
    let owner = sample_account(0x41);
    let bogus = sample_uint256(0x99);
    let base = protocol::nft_page_min_keylet(raw_account_id(owner));
    let first = protocol::nft_page_keylet(base.clone(), sample_uint256(0x11));
    let middle = protocol::nft_page_keylet(base, sample_uint256(0x22));
    let last = protocol::nft_page_max_keylet(raw_account_id(owner));
    let ledger = empty_ledger(vec![
        account_root(owner, 0, 0),
        nft_page_entry(first.clone(), sample_uint256(0x51), Some(bogus), None),
        nft_page_entry(middle.clone(), sample_uint256(0x52), Some(bogus), None),
        nft_page_entry(last.clone(), sample_uint256(0x53), Some(bogus), Some(bogus)),
    ]);
    let mut view = ApplyViewImpl::new(Arc::new(ledger), ApplyFlags::NONE);
    let sttx = ledger_state_fix_tx(owner, Some(owner), 1);

    let result = handle_real_dispatch(&mut view, &sttx, TxType::LEDGER_STATE_FIX, None);

    assert_eq!(result, protocol::Ter::TES_SUCCESS);

    let first_page = view
        .read(first)
        .expect("first page read should succeed")
        .expect("first page should exist");
    assert!(!first_page.is_field_present(get_field_by_symbol("sfPreviousPageMin")));
    assert_eq!(
        first_page.get_field_h256(get_field_by_symbol("sfNextPageMin")),
        middle.key
    );

    let middle_page = view
        .read(middle)
        .expect("middle page read should succeed")
        .expect("middle page should exist");
    assert_eq!(
        middle_page.get_field_h256(get_field_by_symbol("sfPreviousPageMin")),
        first.key
    );
    assert_eq!(
        middle_page.get_field_h256(get_field_by_symbol("sfNextPageMin")),
        last.key
    );

    let last_page = view
        .read(last)
        .expect("last page read should succeed")
        .expect("last page should exist");
    assert_eq!(
        last_page.get_field_h256(get_field_by_symbol("sfPreviousPageMin")),
        middle.key
    );
    assert!(!last_page.is_field_present(get_field_by_symbol("sfNextPageMin")));
}

#[test]
fn ledger_state_fix_returns_failed_processing_when_no_repair_is_needed() {
    let owner = sample_account(0x51);
    let page = protocol::nft_page_max_keylet(raw_account_id(owner));
    let ledger = empty_ledger(vec![
        account_root(owner, 0, 0),
        nft_page_entry(page.clone(), sample_uint256(0x61), None, None),
    ]);
    let mut view = ApplyViewImpl::new(Arc::new(ledger), ApplyFlags::NONE);
    let sttx = ledger_state_fix_tx(owner, Some(owner), 1);

    let result = handle_real_dispatch(&mut view, &sttx, TxType::LEDGER_STATE_FIX, None);

    assert_eq!(result, protocol::Ter::TEC_FAILED_PROCESSING);
    let repaired_page = view
        .read(page)
        .expect("page read should succeed")
        .expect("page should exist");
    assert!(!repaired_page.is_field_present(get_field_by_symbol("sfPreviousPageMin")));
    assert!(!repaired_page.is_field_present(get_field_by_symbol("sfNextPageMin")));
}

#[test]
fn delegate_set_create_inserts_delegate_and_dual_owner_dirs() {
    let account = sample_account(0x31);
    let authorize = sample_account(0x32);
    let mut ledger = empty_ledger(vec![
        account_root(account, 0, 0),
        account_root(authorize, 0, 0),
    ]);
    ledger.set_rules(protocol::Rules::new([protocol::feature_id(
        "PermissionDelegationV1_1",
    )]));
    let mut view = ApplyViewImpl::new(Arc::new(ledger), ApplyFlags::NONE);
    let sttx = delegate_set_tx(account, authorize, &[65_540]);

    let result = handle_real_dispatch(&mut view, &sttx, TxType::DELEGATE_SET, Some(1_000_000));

    assert_eq!(result, protocol::Ter::TES_SUCCESS);

    let delegate = view
        .read(protocol::delegate_keylet(
            raw_account_id(account),
            raw_account_id(authorize),
        ))
        .expect("delegate read should succeed")
        .expect("delegate should exist");
    assert_eq!(
        delegate.get_account_id(get_field_by_symbol("sfAccount")),
        account
    );
    assert_eq!(
        delegate.get_account_id(get_field_by_symbol("sfAuthorize")),
        authorize
    );
    let permissions = delegate.get_field_array(get_field_by_symbol("sfPermissions"));
    assert_eq!(permissions.len(), 1);
    assert_eq!(
        permissions
            .get(0)
            .expect("permission entry")
            .get_field_u32(get_field_by_symbol("sfPermissionValue")),
        65_540
    );

    let owner = view
        .read(account_keylet(raw_account_id(account)))
        .expect("owner read should succeed")
        .expect("owner should exist");
    assert_eq!(owner.get_field_u32(get_field_by_symbol("sfOwnerCount")), 1);

    let owner_dir = view
        .read(owner_dir_keylet(raw_account_id(account)))
        .expect("owner dir read should succeed")
        .expect("owner dir should exist");
    let dest_dir = view
        .read(owner_dir_keylet(raw_account_id(authorize)))
        .expect("authorize dir read should succeed")
        .expect("authorize dir should exist");
    assert_eq!(
        owner_dir
            .get_field_v256(get_field_by_symbol("sfIndexes"))
            .value(),
        &[delegate.key().to_owned()]
    );
    assert_eq!(
        dest_dir
            .get_field_v256(get_field_by_symbol("sfIndexes"))
            .value(),
        &[delegate.key().to_owned()]
    );
}

#[test]
fn delegate_set_delete_removes_existing_delegate() {
    let account = sample_account(0x41);
    let authorize = sample_account(0x42);
    let delegate = delegate_entry(account, authorize, &[65_540], 0, 0);
    let mut ledger = empty_ledger(vec![
        account_root(account, 1, 0),
        account_root(authorize, 0, 0),
        owner_dir_root(account, delegate.key().to_owned()),
        owner_dir_root(authorize, delegate.key().to_owned()),
        delegate,
    ]);
    ledger.set_rules(protocol::Rules::new([protocol::feature_id(
        "PermissionDelegationV1_1",
    )]));
    let mut view = ApplyViewImpl::new(Arc::new(ledger), ApplyFlags::NONE);
    let sttx = delegate_set_tx(account, authorize, &[]);

    let result = handle_real_dispatch(&mut view, &sttx, TxType::DELEGATE_SET, Some(1_000_000));

    assert_eq!(result, protocol::Ter::TES_SUCCESS);
    assert!(
        view.read(protocol::delegate_keylet(
            raw_account_id(account),
            raw_account_id(authorize),
        ))
        .expect("delegate read should succeed")
        .is_none()
    );
    let owner = view
        .read(account_keylet(raw_account_id(account)))
        .expect("owner read should succeed")
        .expect("owner should exist");
    assert_eq!(owner.get_field_u32(get_field_by_symbol("sfOwnerCount")), 0);
}

#[test]
fn batch_dispatch_returns_success_when_batch_feature_is_enabled() {
    let account = sample_account(0x61);
    let mut ledger = empty_ledger(vec![account_root(account, 0, 0)]);
    ledger.set_rules(protocol::Rules::new([protocol::feature_id("Batch")]));
    let mut view = ApplyViewImpl::new(Arc::new(ledger), ApplyFlags::NONE);
    let sttx = batch_tx(account);

    let result = handle_real_dispatch(&mut view, &sttx, TxType::BATCH, None);

    assert_eq!(result, protocol::Ter::TES_SUCCESS);
}

#[test]
fn batch_submit_shell_applies_all_successful_inner_transactions() {
    let account = sample_account(0x61);
    let destination = sample_account(0x62);
    let mut ledger = empty_ledger(vec![
        account_root_with_balance(account, 0, 0, 1_000_000),
        account_root_with_balance(destination, 0, 0, 1_000_000),
    ]);
    ledger.set_rules(protocol::Rules::new([protocol::feature_id("Batch")]));
    let mut view = ApplyViewImpl::new(Arc::new(ledger), ApplyFlags::NONE);
    let first = inner_batch_payment_tx(account, destination, 2, 100);
    let second = inner_batch_payment_tx(account, destination, 3, 200);
    let batch = batch_tx_with_inner(account, protocol::tfIndependent, &[first, second]);

    let result = apply_submit_transactor_shell(&mut view, &batch, TxType::BATCH);

    assert_eq!(result, Ter::TES_SUCCESS);
    let source = view
        .read(account_keylet(raw_account_id(account)))
        .expect("source read should succeed")
        .expect("source should exist");
    let dest = view
        .read(account_keylet(raw_account_id(destination)))
        .expect("destination read should succeed")
        .expect("destination should exist");
    assert_eq!(source.get_field_u32(get_field_by_symbol("sfSequence")), 4);
    assert_eq!(
        source
            .get_field_amount(get_field_by_symbol("sfBalance"))
            .xrp()
            .drops(),
        999_690
    );
    assert_eq!(
        dest.get_field_amount(get_field_by_symbol("sfBalance"))
            .xrp()
            .drops(),
        1_000_300
    );
}

#[test]
fn batch_submit_shell_discards_inner_changes_in_all_or_nothing_mode() {
    let account = sample_account(0x63);
    let destination = sample_account(0x64);
    let mut ledger = empty_ledger(vec![
        account_root_with_balance(account, 0, 0, 1_000_000),
        account_root_with_balance(destination, 0, 0, 1_000_000),
    ]);
    ledger.set_rules(protocol::Rules::new([protocol::feature_id("Batch")]));
    let mut view = ApplyViewImpl::new(Arc::new(ledger), ApplyFlags::NONE);
    let first = inner_batch_payment_tx(account, destination, 2, 100);
    let second = inner_batch_payment_tx(account, destination, 3, 2_000_000);
    let batch = batch_tx_with_inner(account, protocol::tfAllOrNothing, &[first, second]);

    let result = apply_submit_transactor_shell(&mut view, &batch, TxType::BATCH);

    assert_eq!(result, Ter::TES_SUCCESS);
    let source = view
        .read(account_keylet(raw_account_id(account)))
        .expect("source read should succeed")
        .expect("source should exist");
    let dest = view
        .read(account_keylet(raw_account_id(destination)))
        .expect("destination read should succeed")
        .expect("destination should exist");
    assert_eq!(source.get_field_u32(get_field_by_symbol("sfSequence")), 2);
    assert_eq!(
        source
            .get_field_amount(get_field_by_symbol("sfBalance"))
            .xrp()
            .drops(),
        999_990
    );
    assert_eq!(
        dest.get_field_amount(get_field_by_symbol("sfBalance"))
            .xrp()
            .drops(),
        1_000_000
    );
}

#[test]
fn batch_submit_shell_stops_after_first_success_in_only_one_mode() {
    let account = sample_account(0x65);
    let destination = sample_account(0x66);
    let mut ledger = empty_ledger(vec![
        account_root_with_balance(account, 0, 0, 1_000_000),
        account_root_with_balance(destination, 0, 0, 1_000_000),
    ]);
    ledger.set_rules(protocol::Rules::new([protocol::feature_id("Batch")]));
    let mut view = ApplyViewImpl::new(Arc::new(ledger), ApplyFlags::NONE);
    let first = inner_batch_payment_tx(account, destination, 2, 100);
    let second = inner_batch_payment_tx(account, destination, 3, 200);
    let batch = batch_tx_with_inner(account, protocol::tfOnlyOne, &[first, second]);

    let result = apply_submit_transactor_shell(&mut view, &batch, TxType::BATCH);

    assert_eq!(result, Ter::TES_SUCCESS);
    let source = view
        .read(account_keylet(raw_account_id(account)))
        .expect("source read should succeed")
        .expect("source should exist");
    let dest = view
        .read(account_keylet(raw_account_id(destination)))
        .expect("destination read should succeed")
        .expect("destination should exist");
    assert_eq!(source.get_field_u32(get_field_by_symbol("sfSequence")), 3);
    assert_eq!(
        source
            .get_field_amount(get_field_by_symbol("sfBalance"))
            .xrp()
            .drops(),
        999_890
    );
    assert_eq!(
        dest.get_field_amount(get_field_by_symbol("sfBalance"))
            .xrp()
            .drops(),
        1_000_100
    );
}

#[test]
fn batch_submit_shell_applies_successful_prefix_until_failure() {
    let account = sample_account(0x67);
    let destination = sample_account(0x68);
    let mut ledger = empty_ledger(vec![
        account_root_with_balance(account, 0, 0, 1_000_000),
        account_root_with_balance(destination, 0, 0, 1_000_000),
    ]);
    ledger.set_rules(protocol::Rules::new([protocol::feature_id("Batch")]));
    let mut view = ApplyViewImpl::new(Arc::new(ledger), ApplyFlags::NONE);
    let first = inner_batch_payment_tx(account, destination, 2, 100);
    let second = inner_batch_payment_tx(account, destination, 3, 2_000_000);
    let third = inner_batch_payment_tx(account, destination, 4, 300);
    let batch = batch_tx_with_inner(account, protocol::tfUntilFailure, &[first, second, third]);

    let result = apply_submit_transactor_shell(&mut view, &batch, TxType::BATCH);

    assert_eq!(result, Ter::TES_SUCCESS);
    let source = view
        .read(account_keylet(raw_account_id(account)))
        .expect("source read should succeed")
        .expect("source should exist");
    let dest = view
        .read(account_keylet(raw_account_id(destination)))
        .expect("destination read should succeed")
        .expect("destination should exist");
    assert_eq!(source.get_field_u32(get_field_by_symbol("sfSequence")), 4);
    assert_eq!(
        source
            .get_field_amount(get_field_by_symbol("sfBalance"))
            .xrp()
            .drops(),
        999_890
    );
    assert_eq!(
        dest.get_field_amount(get_field_by_symbol("sfBalance"))
            .xrp()
            .drops(),
        1_000_100
    );
}

#[test]
fn loan_broker_set_create_mints_deterministic_pseudo_and_empty_holding() {
    let owner = sample_account(0x71);
    let issuer = sample_account(0x72);
    let vault_pseudo = sample_account(0x73);
    let currency = Currency::from_array([0x55; 20]);
    let vault_asset = Asset::Issue(Issue::new(currency, issuer));
    let vault = vault_entry(owner, vault_pseudo, 7, vault_asset);
    let mut ledger = empty_ledger(vec![
        account_root(owner, 0, 0),
        account_root(issuer, 0, lsfDefaultRipple),
        account_root(vault_pseudo, 0, 0),
        vault,
    ]);
    ledger.set_rules(protocol::Rules::new([
        protocol::feature_id("SingleAssetVault"),
        protocol::feature_id("LendingProtocol"),
    ]));
    let broker_keylet = protocol::loan_broker_keylet(raw_account_id(owner), 1);
    let expected_pseudo = pseudo_account_address(&ledger, broker_keylet.key);
    let mut view = ApplyViewImpl::new(Arc::new(ledger), ApplyFlags::NONE);
    let sttx = loan_broker_set_tx(
        owner,
        protocol::vault_keylet(raw_account_id(owner), 7).key,
        1,
    );

    let result = handle_real_dispatch(&mut view, &sttx, TxType::LOAN_BROKER_SET, Some(1_000_000));

    assert_eq!(result, protocol::Ter::TES_SUCCESS);

    let broker = view
        .read(broker_keylet)
        .expect("broker read should succeed")
        .expect("broker should exist");
    assert_eq!(
        broker.get_account_id(get_field_by_symbol("sfAccount")),
        expected_pseudo
    );

    let pseudo = view
        .read(account_keylet(raw_account_id(expected_pseudo)))
        .expect("pseudo read should succeed")
        .expect("pseudo account should exist");
    assert_eq!(
        pseudo.get_field_h256(get_field_by_symbol("sfLoanBrokerID")),
        broker_keylet.key
    );
    assert_eq!(pseudo.get_field_u32(get_field_by_symbol("sfOwnerCount")), 1);

    let trust_line = view
        .read(line(issuer, expected_pseudo, currency))
        .expect("trust line read should succeed")
        .expect("trust line should exist");
    assert_eq!(trust_line.get_type(), LedgerEntryType::RippleState);
}

#[test]
fn loan_broker_delete_cleans_empty_holding_and_pseudo_account() {
    let owner = sample_account(0x81);
    let issuer = sample_account(0x82);
    let vault_pseudo = sample_account(0x83);
    let currency = Currency::from_array([0x56; 20]);
    let vault_asset = Asset::Issue(Issue::new(currency, issuer));
    let vault_key = protocol::vault_keylet(raw_account_id(owner), 9).key;
    let broker_key = protocol::loan_broker_keylet(raw_account_id(owner), 1).key;
    let mut ledger = empty_ledger(vec![
        account_root(owner, 0, 0),
        account_root(issuer, 0, lsfDefaultRipple),
        account_root(vault_pseudo, 0, 0),
        vault_entry(owner, vault_pseudo, 9, vault_asset),
    ]);
    ledger.set_rules(protocol::Rules::new([
        protocol::feature_id("SingleAssetVault"),
        protocol::feature_id("LendingProtocol"),
    ]));
    let mut view = ApplyViewImpl::new(Arc::new(ledger), ApplyFlags::NONE);

    let create = handle_real_dispatch(
        &mut view,
        &loan_broker_set_tx(owner, vault_key, 1),
        TxType::LOAN_BROKER_SET,
        Some(1_000_000),
    );
    assert_eq!(create, protocol::Ter::TES_SUCCESS);

    let broker = view
        .read(protocol::loan_broker_keylet_from_key(broker_key))
        .expect("broker read should succeed")
        .expect("broker should exist");
    let pseudo = broker.get_account_id(get_field_by_symbol("sfAccount"));
    assert!(
        view.read(line(issuer, pseudo, currency))
            .expect("trust line read should succeed")
            .is_some()
    );

    let delete = handle_real_dispatch(
        &mut view,
        &loan_broker_delete_tx(owner, broker_key),
        TxType::LOAN_BROKER_DELETE,
        None,
    );
    assert_eq!(delete, protocol::Ter::TES_SUCCESS);
    assert!(
        view.read(protocol::loan_broker_keylet_from_key(broker_key))
            .expect("broker read should succeed")
            .is_none()
    );
    assert!(
        view.read(account_keylet(raw_account_id(pseudo)))
            .expect("pseudo read should succeed")
            .is_none()
    );
    assert!(
        view.read(line(issuer, pseudo, currency))
            .expect("trust line read should succeed")
            .is_none()
    );
    let owner_root = view
        .read(account_keylet(raw_account_id(owner)))
        .expect("owner read should succeed")
        .expect("owner should exist");
    assert_eq!(
        owner_root.get_field_u32(get_field_by_symbol("sfOwnerCount")),
        0
    );
}

#[test]
fn loan_delete_removes_loan_and_keeps_broker_pseudo_lifecycle_intact() {
    let owner = sample_account(0x91);
    let borrower = sample_account(0x92);
    let issuer = sample_account(0x93);
    let vault_pseudo = sample_account(0x94);
    let currency = Currency::from_array([0x57; 20]);
    let vault_asset = Asset::Issue(Issue::new(currency, issuer));
    let vault_key = protocol::vault_keylet(raw_account_id(owner), 11).key;
    let broker_key = protocol::loan_broker_keylet(raw_account_id(owner), 1).key;
    let loan_id = sample_uint256(0x95);
    let mut ledger = empty_ledger(vec![
        account_root(owner, 0, 0),
        account_root(borrower, 0, 0),
        account_root(issuer, 0, lsfDefaultRipple),
        account_root(vault_pseudo, 0, 0),
        vault_entry(owner, vault_pseudo, 11, vault_asset),
    ]);
    ledger.set_rules(protocol::Rules::new([
        protocol::feature_id("SingleAssetVault"),
        protocol::feature_id("LendingProtocol"),
    ]));
    let mut view = ApplyViewImpl::new(Arc::new(ledger), ApplyFlags::NONE);

    let create = handle_real_dispatch(
        &mut view,
        &loan_broker_set_tx(owner, vault_key, 1),
        TxType::LOAN_BROKER_SET,
        Some(1_000_000),
    );
    assert_eq!(create, protocol::Ter::TES_SUCCESS);

    let broker = view
        .read(protocol::loan_broker_keylet_from_key(broker_key))
        .expect("broker read should succeed")
        .expect("broker should exist");
    let broker_pseudo = broker.get_account_id(get_field_by_symbol("sfAccount"));

    let broker_page = ledger::dir_insert(
        &mut view,
        &owner_dir_keylet(raw_account_id(broker_pseudo)),
        loan_id,
        &|_| {},
    )
    .expect("broker dir insert should succeed")
    .expect("broker dir page should exist");
    let borrower_page = ledger::dir_insert(
        &mut view,
        &owner_dir_keylet(raw_account_id(borrower)),
        loan_id,
        &|_| {},
    )
    .expect("borrower dir insert should succeed")
    .expect("borrower dir page should exist");

    let borrower_root = view
        .read(account_keylet(raw_account_id(borrower)))
        .expect("borrower read should succeed")
        .expect("borrower should exist");
    let _ = ledger::adjust_owner_count(&mut view, &borrower_root, 1);

    let mut broker_obj = broker.clone_as_object();
    broker_obj.set_field_u32(get_field_by_symbol("sfOwnerCount"), 1);
    let _ = view.update(Arc::new(STLedgerEntry::from_stobject(
        broker_obj,
        *broker.key(),
    )));

    let _ = view.insert(Arc::new(loan_entry(
        loan_id,
        borrower,
        broker_key,
        broker_page,
        borrower_page,
    )));

    let delete = handle_real_dispatch(
        &mut view,
        &loan_delete_tx(owner, loan_id),
        TxType::LOAN_DELETE,
        None,
    );
    assert_eq!(delete, protocol::Ter::TES_SUCCESS);
    assert!(
        view.read(protocol::loan_keylet_from_key(loan_id))
            .expect("loan read should succeed")
            .is_none()
    );

    let borrower_root = view
        .read(account_keylet(raw_account_id(borrower)))
        .expect("borrower read should succeed")
        .expect("borrower should exist");
    assert_eq!(
        borrower_root.get_field_u32(get_field_by_symbol("sfOwnerCount")),
        0
    );

    let broker = view
        .read(protocol::loan_broker_keylet_from_key(broker_key))
        .expect("broker read should succeed")
        .expect("broker should remain");
    assert_eq!(broker.get_field_u32(get_field_by_symbol("sfOwnerCount")), 0);
    assert!(
        view.read(account_keylet(raw_account_id(broker_pseudo)))
            .expect("pseudo read should succeed")
            .is_some()
    );
    assert!(
        view.read(line(issuer, broker_pseudo, currency))
            .expect("trust line read should succeed")
            .is_some()
    );
}

#[test]
fn loan_set_dispatch_creates_loan_and_updates_vault_and_broker() {
    let owner = sample_account(0x96);
    let borrower = sample_account(0x97);
    let vault_pseudo = sample_account(0x98);
    let broker_pseudo = sample_account(0x99);
    let asset = Asset::Issue(xrp_issue());
    let vault_id = protocol::vault_keylet(raw_account_id(owner), 13).key;
    let broker_id = sample_uint256(0x9A);
    let ledger = ledger_with_header(
        LedgerHeader {
            seq: 1,
            parent_close_time: 500,
            ..LedgerHeader::default()
        },
        vec![
            account_root(owner, 0, 0),
            account_root_with_balance(borrower, 0, 0, 1_000_000),
            account_root_with_balance(vault_pseudo, 0, 0, 10_000),
            account_root_with_balance(broker_pseudo, 0, 0, 10_000),
            managed_vault_entry(owner, vault_pseudo, 13, asset, 10_000, 10_000, 0),
            loan_broker_entry(
                broker_id,
                owner,
                broker_pseudo,
                vault_id,
                asset,
                0,
                10_000,
                100_000,
                100_000,
            ),
        ],
    );
    let mut ledger = ledger;
    ledger.set_rules(protocol::Rules::new([protocol::feature_id(
        "LendingProtocol",
    )]));
    let mut view = ApplyViewImpl::new(Arc::new(ledger), ApplyFlags::NONE);

    let result = handle_real_dispatch(
        &mut view,
        &loan_set_tx(borrower, broker_id, 1_000, 1),
        TxType::LOAN_SET,
        Some(1_000_000),
    );
    assert_eq!(result, protocol::Ter::TES_SUCCESS);

    let loan = view
        .read(protocol::loan_keylet(broker_id, 1))
        .expect("loan read should succeed")
        .expect("loan should exist");
    assert_eq!(
        loan.get_account_id(get_field_by_symbol("sfBorrower")),
        borrower
    );
    assert_eq!(
        loan.get_field_number(get_field_by_symbol("sfPrincipalOutstanding"))
            .value(),
        RuntimeNumber::from_i64(1_000)
    );
    assert_eq!(
        loan.get_field_u32(get_field_by_symbol("sfPaymentRemaining")),
        1
    );

    let vault = view
        .read(protocol::vault_keylet_from_key(vault_id))
        .expect("vault read should succeed")
        .expect("vault should remain");
    assert_eq!(
        vault
            .get_field_number(get_field_by_symbol("sfAssetsAvailable"))
            .value(),
        RuntimeNumber::from_i64(9_000)
    );

    let broker = view
        .read(protocol::loan_broker_keylet_from_key(broker_id))
        .expect("broker read should succeed")
        .expect("broker should remain");
    assert_eq!(
        broker.get_field_u32(get_field_by_symbol("sfLoanSequence")),
        2
    );
    assert_eq!(broker.get_field_u32(get_field_by_symbol("sfOwnerCount")), 1);

    let borrower_root = view
        .read(account_keylet(raw_account_id(borrower)))
        .expect("borrower read should succeed")
        .expect("borrower should exist");
    assert_eq!(
        borrower_root.get_field_u32(get_field_by_symbol("sfOwnerCount")),
        1
    );
}

#[test]
fn loan_set_dispatch_links_borrower_and_broker_pseudo_owner_dirs() {
    let owner = sample_account(0x9B);
    let borrower = sample_account(0x9C);
    let vault_pseudo = sample_account(0x9D);
    let broker_pseudo = sample_account(0x9E);
    let asset = Asset::Issue(xrp_issue());
    let vault_id = protocol::vault_keylet(raw_account_id(owner), 14).key;
    let broker_id = sample_uint256(0x9F);
    let ledger = ledger_with_header(
        LedgerHeader {
            seq: 1,
            parent_close_time: 500,
            ..LedgerHeader::default()
        },
        vec![
            account_root(owner, 0, 0),
            account_root_with_balance(borrower, 0, 0, 1_000_000),
            account_root_with_balance(vault_pseudo, 0, 0, 10_000),
            account_root_with_balance(broker_pseudo, 0, 0, 10_000),
            managed_vault_entry(owner, vault_pseudo, 14, asset, 10_000, 10_000, 0),
            loan_broker_entry(
                broker_id,
                owner,
                broker_pseudo,
                vault_id,
                asset,
                0,
                10_000,
                100_000,
                100_000,
            ),
        ],
    );
    let mut ledger = ledger;
    ledger.set_rules(protocol::Rules::new([protocol::feature_id(
        "LendingProtocol",
    )]));
    let mut view = ApplyViewImpl::new(Arc::new(ledger), ApplyFlags::NONE);

    let result = handle_real_dispatch(
        &mut view,
        &loan_set_tx(borrower, broker_id, 1_000, 1),
        TxType::LOAN_SET,
        Some(1_000_000),
    );
    assert_eq!(result, protocol::Ter::TES_SUCCESS);

    let loan_key = protocol::loan_keylet(broker_id, 1).key;
    let loan = view
        .read(protocol::loan_keylet_from_key(loan_key))
        .expect("loan read should succeed")
        .expect("loan should exist");
    assert_eq!(
        loan.get_field_u64(get_field_by_symbol("sfLoanBrokerNode")),
        0
    );
    assert_eq!(loan.get_field_u64(get_field_by_symbol("sfOwnerNode")), 0);

    let broker_dir = view
        .read(owner_dir_keylet(raw_account_id(broker_pseudo)))
        .expect("broker dir read should succeed")
        .expect("broker dir should exist");
    assert!(
        broker_dir
            .get_field_v256(get_field_by_symbol("sfIndexes"))
            .value()
            .contains(&loan_key)
    );

    let borrower_dir = view
        .read(owner_dir_keylet(raw_account_id(borrower)))
        .expect("borrower dir read should succeed")
        .expect("borrower dir should exist");
    assert!(
        borrower_dir
            .get_field_v256(get_field_by_symbol("sfIndexes"))
            .value()
            .contains(&loan_key)
    );
}

#[test]
fn loan_set_dispatch_with_origination_fee_pays_owner_and_updates_debt_total() {
    let owner = sample_account(0xA0);
    let borrower = sample_account(0xA7);
    let vault_pseudo = sample_account(0xA8);
    let broker_pseudo = sample_account(0xA9);
    let asset = Asset::Issue(xrp_issue());
    let vault_id = protocol::vault_keylet(raw_account_id(owner), 15).key;
    let broker_id = sample_uint256(0xAA);
    let ledger = ledger_with_header(
        LedgerHeader {
            seq: 1,
            parent_close_time: 500,
            ..LedgerHeader::default()
        },
        vec![
            account_root_with_balance(owner, 0, 0, 1_000),
            account_root_with_balance(borrower, 0, 0, 1_000_000),
            account_root_with_balance(vault_pseudo, 0, 0, 10_000),
            account_root_with_balance(broker_pseudo, 0, 0, 10_000),
            managed_vault_entry(owner, vault_pseudo, 15, asset, 10_000, 10_000, 0),
            loan_broker_entry(
                broker_id,
                owner,
                broker_pseudo,
                vault_id,
                asset,
                0,
                10_000,
                100_000,
                100_000,
            ),
        ],
    );
    let mut ledger = ledger;
    ledger.set_rules(protocol::Rules::new([protocol::feature_id(
        "LendingProtocol",
    )]));
    let mut view = ApplyViewImpl::new(Arc::new(ledger), ApplyFlags::NONE);

    let result = handle_real_dispatch(
        &mut view,
        &loan_set_tx_with_origination_fee(borrower, broker_id, 1_000, 25, 1),
        TxType::LOAN_SET,
        Some(1_000_000),
    );
    assert_eq!(result, protocol::Ter::TES_SUCCESS);

    let owner_root = view
        .read(account_keylet(raw_account_id(owner)))
        .expect("owner read should succeed")
        .expect("owner should exist");
    assert_eq!(
        owner_root
            .get_field_amount(get_field_by_symbol("sfBalance"))
            .xrp()
            .drops(),
        1_025
    );

    let broker = view
        .read(protocol::loan_broker_keylet_from_key(broker_id))
        .expect("broker read should succeed")
        .expect("broker should remain");
    assert_eq!(
        broker
            .get_field_number(get_field_by_symbol("sfDebtTotal"))
            .value(),
        RuntimeNumber::from_i64(1_000)
    );
    assert_eq!(
        broker.get_field_u32(get_field_by_symbol("sfLoanSequence")),
        2
    );
}

#[test]
fn loan_manage_default_updates_vault_broker_loan_and_moves_cover() {
    let owner = sample_account(0xA1);
    let borrower = sample_account(0xA2);
    let vault_pseudo = sample_account(0xA3);
    let broker_pseudo = sample_account(0xA4);
    let asset = Asset::Issue(xrp_issue());
    let vault_id = protocol::vault_keylet(raw_account_id(owner), 17).key;
    let broker_id = sample_uint256(0xA5);
    let loan_id = sample_uint256(0xA6);
    let ledger = ledger_with_header(
        LedgerHeader {
            seq: 1,
            parent_close_time: 50,
            ..LedgerHeader::default()
        },
        vec![
            account_root(owner, 0, 0),
            account_root(borrower, 0, 0),
            account_root_with_balance(vault_pseudo, 0, 0, 100),
            account_root_with_balance(broker_pseudo, 0, 0, 100),
            managed_vault_entry(owner, vault_pseudo, 17, asset, 500, 100, 0),
            loan_broker_entry(
                broker_id,
                owner,
                broker_pseudo,
                vault_id,
                asset,
                100,
                20,
                100_000,
                100_000,
            ),
            managed_loan_entry(loan_id, borrower, broker_id, 60, 50, 10, 1, 10),
        ],
    );
    let mut ledger = ledger;
    ledger.set_rules(protocol::Rules::new([protocol::feature_id(
        "LendingProtocol",
    )]));
    let mut view = ApplyViewImpl::new(Arc::new(ledger), ApplyFlags::NONE);

    let result = handle_real_dispatch(
        &mut view,
        &loan_manage_tx(owner, loan_id, tfLoanDefault),
        TxType::LOAN_MANAGE,
        None,
    );
    assert_eq!(result, protocol::Ter::TES_SUCCESS);

    let vault = view
        .read(protocol::vault_keylet_from_key(vault_id))
        .expect("vault read should succeed")
        .expect("vault should remain");
    assert_eq!(
        vault
            .get_field_number(get_field_by_symbol("sfAssetsTotal"))
            .value(),
        RuntimeNumber::from_i64(470)
    );
    assert_eq!(
        vault
            .get_field_number(get_field_by_symbol("sfAssetsAvailable"))
            .value(),
        RuntimeNumber::from_i64(120)
    );

    let broker = view
        .read(protocol::loan_broker_keylet_from_key(broker_id))
        .expect("broker read should succeed")
        .expect("broker should remain");
    assert_eq!(
        broker
            .get_field_number(get_field_by_symbol("sfDebtTotal"))
            .value(),
        RuntimeNumber::from_i64(50)
    );
    assert_eq!(
        broker
            .get_field_number(get_field_by_symbol("sfCoverAvailable"))
            .value(),
        RuntimeNumber::zero()
    );

    let loan = view
        .read(protocol::loan_keylet_from_key(loan_id))
        .expect("loan read should succeed")
        .expect("loan should remain");
    assert!(loan.is_flag(protocol::lsfLoanDefault));
    assert_eq!(
        loan.get_field_number(get_field_by_symbol("sfTotalValueOutstanding"))
            .value(),
        RuntimeNumber::zero()
    );
    assert_eq!(
        loan.get_field_u32(get_field_by_symbol("sfPaymentRemaining")),
        0
    );
    assert_eq!(
        loan.get_field_u32(get_field_by_symbol("sfNextPaymentDueDate")),
        0
    );

    let vault_pseudo_root = view
        .read(account_keylet(raw_account_id(vault_pseudo)))
        .expect("vault pseudo read should succeed")
        .expect("vault pseudo should exist");
    let broker_pseudo_root = view
        .read(account_keylet(raw_account_id(broker_pseudo)))
        .expect("broker pseudo read should succeed")
        .expect("broker pseudo should exist");
    assert_eq!(
        vault_pseudo_root
            .get_field_amount(get_field_by_symbol("sfBalance"))
            .xrp()
            .drops(),
        120
    );
    assert_eq!(
        broker_pseudo_root
            .get_field_amount(get_field_by_symbol("sfBalance"))
            .xrp()
            .drops(),
        80
    );
}

#[test]
fn loan_manage_impair_updates_vault_loss_and_due_date() {
    let owner = sample_account(0xB1);
    let borrower = sample_account(0xB2);
    let vault_pseudo = sample_account(0xB3);
    let broker_pseudo = sample_account(0xB4);
    let asset = Asset::Issue(xrp_issue());
    let vault_id = protocol::vault_keylet(raw_account_id(owner), 19).key;
    let broker_id = sample_uint256(0xB5);
    let loan_id = sample_uint256(0xB6);
    let mut ledger = ledger_with_header(
        LedgerHeader {
            seq: 1,
            parent_close_time: 200,
            ..LedgerHeader::default()
        },
        vec![
            account_root(owner, 0, 0),
            account_root(borrower, 0, 0),
            account_root(vault_pseudo, 0, 0),
            account_root(broker_pseudo, 0, 0),
            managed_vault_entry(owner, vault_pseudo, 19, asset, 500, 300, 10),
            loan_broker_entry(
                broker_id,
                owner,
                broker_pseudo,
                vault_id,
                asset,
                100,
                0,
                100_000,
                100_000,
            ),
            managed_loan_entry(loan_id, borrower, broker_id, 125, 100, 25, 1, 250),
        ],
    );
    ledger.set_rules(protocol::Rules::new([protocol::feature_id(
        "LendingProtocol",
    )]));
    let mut view = ApplyViewImpl::new(Arc::new(ledger), ApplyFlags::NONE);

    let result = handle_real_dispatch(
        &mut view,
        &loan_manage_tx(owner, loan_id, tfLoanImpair),
        TxType::LOAN_MANAGE,
        None,
    );
    assert_eq!(result, protocol::Ter::TES_SUCCESS);

    let vault = view
        .read(protocol::vault_keylet_from_key(vault_id))
        .expect("vault read should succeed")
        .expect("vault should remain");
    assert_eq!(
        vault
            .get_field_number(get_field_by_symbol("sfLossUnrealized"))
            .value(),
        RuntimeNumber::from_i64(110)
    );

    let loan = view
        .read(protocol::loan_keylet_from_key(loan_id))
        .expect("loan read should succeed")
        .expect("loan should remain");
    assert!(loan.is_flag(lsfLoanImpaired));
    assert_eq!(
        loan.get_field_u32(get_field_by_symbol("sfNextPaymentDueDate")),
        200
    );
}

#[test]
fn loan_manage_unimpair_reverses_loss_and_restores_due_date() {
    let owner = sample_account(0xC1);
    let borrower = sample_account(0xC2);
    let vault_pseudo = sample_account(0xC3);
    let broker_pseudo = sample_account(0xC4);
    let asset = Asset::Issue(xrp_issue());
    let vault_id = protocol::vault_keylet(raw_account_id(owner), 23).key;
    let broker_id = sample_uint256(0xC5);
    let loan_id = sample_uint256(0xC6);
    let mut loan = managed_loan_entry(loan_id, borrower, broker_id, 125, 100, 25, 1, 250);
    loan.set_flag(lsfLoanImpaired);
    let mut ledger = ledger_with_header(
        LedgerHeader {
            seq: 1,
            parent_close_time: 200,
            ..LedgerHeader::default()
        },
        vec![
            account_root(owner, 0, 0),
            account_root(borrower, 0, 0),
            account_root(vault_pseudo, 0, 0),
            account_root(broker_pseudo, 0, 0),
            managed_vault_entry(owner, vault_pseudo, 23, asset, 500, 300, 150),
            loan_broker_entry(
                broker_id,
                owner,
                broker_pseudo,
                vault_id,
                asset,
                100,
                0,
                100_000,
                100_000,
            ),
            loan,
        ],
    );
    ledger.set_rules(protocol::Rules::new([protocol::feature_id(
        "LendingProtocol",
    )]));
    let mut view = ApplyViewImpl::new(Arc::new(ledger), ApplyFlags::NONE);

    let result = handle_real_dispatch(
        &mut view,
        &loan_manage_tx(owner, loan_id, tfLoanUnimpair),
        TxType::LOAN_MANAGE,
        None,
    );
    assert_eq!(result, protocol::Ter::TES_SUCCESS);

    let vault = view
        .read(protocol::vault_keylet_from_key(vault_id))
        .expect("vault read should succeed")
        .expect("vault should remain");
    assert_eq!(
        vault
            .get_field_number(get_field_by_symbol("sfLossUnrealized"))
            .value(),
        RuntimeNumber::from_i64(50)
    );

    let loan = view
        .read(protocol::loan_keylet_from_key(loan_id))
        .expect("loan read should succeed")
        .expect("loan should remain");
    assert!(!loan.is_flag(lsfLoanImpaired));
    assert_eq!(
        loan.get_field_u32(get_field_by_symbol("sfNextPaymentDueDate")),
        230
    );
}

#[test]
fn vault_create_dispatch_creates_vault_pseudo_and_share_issuance() {
    let owner = sample_account(0xD1);
    let mut ledger = empty_ledger(vec![account_root(owner, 0, 0)]);
    ledger.set_rules(protocol::Rules::new([protocol::feature_id(
        "SingleAssetVault",
    )]));
    let mut view = ApplyViewImpl::new(Arc::new(ledger), ApplyFlags::NONE);

    let result = handle_real_dispatch(
        &mut view,
        &vault_create_tx(owner, Asset::Issue(xrp_issue()), 1),
        TxType::VAULT_CREATE,
        None,
    );
    assert_eq!(result, protocol::Ter::TES_SUCCESS);

    let vault = view
        .read(protocol::vault_keylet(raw_account_id(owner), 1))
        .expect("vault read should succeed")
        .expect("vault should exist");
    let pseudo = vault.get_account_id(get_field_by_symbol("sfAccount"));
    let share_id = vault.get_field_h192(get_field_by_symbol("sfShareMPTID"));

    assert!(
        view.read(account_keylet(raw_account_id(pseudo)))
            .expect("pseudo read should succeed")
            .is_some()
    );
    assert!(
        view.read(protocol::mpt_issuance_keylet_from_mptid(share_id))
            .expect("issuance read should succeed")
            .is_some()
    );
}

#[test]
fn vault_set_dispatch_updates_assets_maximum() {
    let owner = sample_account(0xD2);
    let pseudo = sample_account(0xD3);
    let share_id = share_id_for(pseudo, 1);
    let asset = Asset::Issue(xrp_issue());
    let vault_id = protocol::vault_keylet(raw_account_id(owner), 7).key;
    let mut vault = vault_entry_with_share(owner, pseudo, 7, asset, share_id);
    vault.set_field_number(
        get_field_by_symbol("sfAssetsTotal"),
        asset_number(asset, 50),
    );
    let mut ledger = empty_ledger(vec![
        account_root(owner, 0, 0),
        account_root(pseudo, 1, 0),
        vault,
        mpt_issuance_entry(
            pseudo,
            1,
            0,
            MPT_CAN_ESCROW_FLAG | MPT_CAN_TRADE_FLAG | MPT_CAN_TRANSFER_FLAG,
        ),
    ]);
    ledger.set_rules(protocol::Rules::new([protocol::feature_id(
        "SingleAssetVault",
    )]));
    let mut view = ApplyViewImpl::new(Arc::new(ledger), ApplyFlags::NONE);

    let result = handle_real_dispatch(
        &mut view,
        &vault_set_tx(owner, vault_id, 250),
        TxType::VAULT_SET,
        None,
    );
    assert_eq!(result, protocol::Ter::TES_SUCCESS);

    let vault = view
        .read(protocol::vault_keylet_from_key(vault_id))
        .expect("vault read should succeed")
        .expect("vault should remain");
    assert_eq!(
        vault
            .get_field_number(get_field_by_symbol("sfAssetsMaximum"))
            .value(),
        RuntimeNumber::from_i64(250)
    );
}

#[test]
fn vault_delete_dispatch_removes_vault_and_share_issuance() {
    let owner = sample_account(0xD4);
    let pseudo = sample_account(0xD5);
    let share_id = share_id_for(pseudo, 1);
    let asset = Asset::Issue(xrp_issue());
    let vault_id = protocol::vault_keylet(raw_account_id(owner), 9).key;
    let ledger = {
        let vault = vault_entry_with_share(owner, pseudo, 9, asset, share_id);
        let owner_token = mptoken_entry(owner, share_id, 0);
        let mut pseudo_root = account_root_with_balance(pseudo, 1, 0, 0);
        pseudo_root.set_field_h256(get_field_by_symbol("sfRegularKey"), vault_id);
        let mut owner_root = account_root(owner, 1, 0);
        owner_root.set_field_u32(get_field_by_symbol("sfOwnerCount"), 1);
        let mut ledger = empty_ledger(vec![
            owner_root,
            pseudo_root,
            vault,
            mpt_issuance_entry(
                pseudo,
                1,
                0,
                MPT_CAN_ESCROW_FLAG | MPT_CAN_TRADE_FLAG | MPT_CAN_TRANSFER_FLAG,
            ),
            owner_token,
            owner_dir_root(
                owner,
                protocol::mptoken_keylet_from_mptid(share_id, raw_account_id(owner)).key,
            ),
            owner_dir_root(
                pseudo,
                protocol::mpt_issuance_keylet(1, raw_account_id(pseudo)).key,
            ),
        ]);
        ledger.set_rules(protocol::Rules::new([protocol::feature_id(
            "SingleAssetVault",
        )]));
        ledger
    };
    let mut view = ApplyViewImpl::new(Arc::new(ledger), ApplyFlags::NONE);

    let result = handle_real_dispatch(
        &mut view,
        &vault_delete_tx(owner, vault_id),
        TxType::VAULT_DELETE,
        None,
    );
    assert_eq!(result, protocol::Ter::TES_SUCCESS);
    assert!(
        view.read(protocol::vault_keylet_from_key(vault_id))
            .expect("vault read should succeed")
            .is_none()
    );
    assert!(
        view.read(protocol::mpt_issuance_keylet_from_mptid(share_id))
            .expect("issuance read should succeed")
            .is_none()
    );
}

#[test]
fn vault_deposit_dispatch_updates_vault_and_mints_shares() {
    let owner = sample_account(0xD6);
    let depositor = sample_account(0xD7);
    let pseudo = sample_account(0xD8);
    let share_id = share_id_for(pseudo, 1);
    let asset = Asset::Issue(xrp_issue());
    let vault_id = protocol::vault_keylet(raw_account_id(owner), 11).key;
    let ledger = {
        let vault = vault_entry_with_share(owner, pseudo, 11, asset, share_id);
        let mut ledger = empty_ledger(vec![
            account_root(owner, 0, 0),
            account_root_with_balance(depositor, 0, 0, 1_000),
            account_root(pseudo, 1, 0),
            vault,
            mpt_issuance_entry(
                pseudo,
                1,
                0,
                MPT_CAN_ESCROW_FLAG | MPT_CAN_TRADE_FLAG | MPT_CAN_TRANSFER_FLAG,
            ),
        ]);
        ledger.set_rules(protocol::Rules::new([protocol::feature_id(
            "SingleAssetVault",
        )]));
        ledger
    };
    let mut view = ApplyViewImpl::new(Arc::new(ledger), ApplyFlags::NONE);

    let result = handle_real_dispatch(
        &mut view,
        &vault_deposit_tx(depositor, vault_id, 500),
        TxType::VAULT_DEPOSIT,
        None,
    );
    assert_eq!(result, protocol::Ter::TES_SUCCESS);

    let vault = view
        .read(protocol::vault_keylet_from_key(vault_id))
        .expect("vault read should succeed")
        .expect("vault should remain");
    assert_eq!(
        vault
            .get_field_number(get_field_by_symbol("sfAssetsTotal"))
            .value(),
        RuntimeNumber::from_i64(500)
    );
    assert_eq!(
        vault
            .get_field_number(get_field_by_symbol("sfAssetsAvailable"))
            .value(),
        RuntimeNumber::from_i64(500)
    );
    let token = view
        .read(protocol::mptoken_keylet_from_mptid(
            share_id,
            raw_account_id(depositor),
        ))
        .expect("token read should succeed")
        .expect("depositor token should exist");
    assert_eq!(token.get_field_u64(get_field_by_symbol("sfMPTAmount")), 500);
}

#[test]
fn vault_withdraw_dispatch_redeems_shares_and_pays_account() {
    let owner = sample_account(0xD9);
    let holder = sample_account(0xDA);
    let pseudo = sample_account(0xDB);
    let share_id = share_id_for(pseudo, 1);
    let asset = Asset::Issue(xrp_issue());
    let vault_id = protocol::vault_keylet(raw_account_id(owner), 13).key;
    let mut vault = vault_entry_with_share(owner, pseudo, 13, asset, share_id);
    vault.set_field_number(
        get_field_by_symbol("sfAssetsTotal"),
        asset_number(asset, 500),
    );
    vault.set_field_number(
        get_field_by_symbol("sfAssetsAvailable"),
        asset_number(asset, 500),
    );
    let ledger = {
        let mut ledger = empty_ledger(vec![
            account_root(owner, 0, 0),
            account_root_with_balance(holder, 0, 0, 100),
            account_root_with_balance(pseudo, 1, 0, 500),
            vault,
            mpt_issuance_entry(
                pseudo,
                1,
                500,
                MPT_CAN_ESCROW_FLAG | MPT_CAN_TRADE_FLAG | MPT_CAN_TRANSFER_FLAG,
            ),
            mptoken_entry(holder, share_id, 500),
        ]);
        ledger.set_rules(protocol::Rules::new([protocol::feature_id(
            "SingleAssetVault",
        )]));
        ledger
    };
    let mut view = ApplyViewImpl::new(Arc::new(ledger), ApplyFlags::NONE);

    let result = handle_real_dispatch(
        &mut view,
        &vault_withdraw_share_tx(holder, vault_id, share_id, 200),
        TxType::VAULT_WITHDRAW,
        None,
    );
    assert_eq!(result, protocol::Ter::TES_SUCCESS);

    let holder_root = view
        .read(account_keylet(raw_account_id(holder)))
        .expect("holder read should succeed")
        .expect("holder should remain");
    assert_eq!(
        holder_root
            .get_field_amount(get_field_by_symbol("sfBalance"))
            .xrp()
            .drops(),
        300
    );
    let token = view
        .read(protocol::mptoken_keylet_from_mptid(
            share_id,
            raw_account_id(holder),
        ))
        .expect("token read should succeed")
        .expect("holder token should remain");
    assert_eq!(token.get_field_u64(get_field_by_symbol("sfMPTAmount")), 300);
}

#[test]
fn vault_clawback_dispatch_burns_holder_shares() {
    let owner = sample_account(0xDC);
    let holder = sample_account(0xDD);
    let pseudo = sample_account(0xDE);
    let share_id = share_id_for(pseudo, 1);
    let asset = Asset::Issue(xrp_issue());
    let vault_id = protocol::vault_keylet(raw_account_id(owner), 15).key;
    let mut vault = vault_entry_with_share(owner, pseudo, 15, asset, share_id);
    vault.set_field_number(
        get_field_by_symbol("sfAssetsTotal"),
        asset_number(asset, 500),
    );
    vault.set_field_number(
        get_field_by_symbol("sfAssetsAvailable"),
        asset_number(asset, 500),
    );
    let ledger = {
        let mut ledger = empty_ledger(vec![
            account_root(owner, 0, 0),
            account_root(holder, 1, 0),
            account_root_with_balance(pseudo, 1, 0, 500),
            vault,
            mpt_issuance_entry(
                pseudo,
                1,
                500,
                MPT_CAN_ESCROW_FLAG | MPT_CAN_TRADE_FLAG | MPT_CAN_TRANSFER_FLAG,
            ),
            mptoken_entry(holder, share_id, 500),
        ]);
        ledger.set_rules(protocol::Rules::new([protocol::feature_id(
            "SingleAssetVault",
        )]));
        ledger
    };
    let mut view = ApplyViewImpl::new(Arc::new(ledger), ApplyFlags::NONE);

    let result = handle_real_dispatch(
        &mut view,
        &vault_clawback_share_tx(owner, holder, vault_id, share_id),
        TxType::VAULT_CLAWBACK,
        None,
    );
    assert_eq!(result, protocol::Ter::TES_SUCCESS);

    assert!(
        view.read(protocol::mptoken_keylet_from_mptid(
            share_id,
            raw_account_id(holder)
        ))
        .expect("token read should succeed")
        .is_none()
    );
    let issuance = view
        .read(protocol::mpt_issuance_keylet_from_mptid(share_id))
        .expect("issuance read should succeed")
        .expect("issuance should remain");
    assert_eq!(
        issuance.get_field_u64(get_field_by_symbol("sfOutstandingAmount")),
        0
    );
}

#[test]
fn loan_pay_dispatch_reduces_outstanding_balance() {
    let borrower = sample_account(0xE1);
    let broker_owner = sample_account(0xE2);
    let broker_pseudo = sample_account(0xE3);
    let vault_pseudo = sample_account(0xE4);
    let loan_id = sample_uint256(0xF1);
    let broker_id = sample_uint256(0xF2);
    let vault_id = protocol::vault_keylet(raw_account_id(broker_owner), 1).key;
    let asset = Asset::Issue(xrp_issue());

    let loan = managed_loan_entry(
        loan_id,
        borrower,
        broker_id,
        500_000_000,
        500_000_000,
        0,
        10,
        120,
    );

    let broker = loan_broker_entry(
        broker_id,
        broker_owner,
        broker_pseudo,
        vault_id,
        asset,
        500_000_000,
        0,
        0,
        0,
    );

    let mut vault = vault_entry(broker_owner, vault_pseudo, 1, asset);
    vault.set_field_number(
        get_field_by_symbol("sfAssetsAvailable"),
        asset_number(asset, 0),
    );
    vault.set_field_number(
        get_field_by_symbol("sfAssetsTotal"),
        asset_number(asset, 500_000_000),
    );

    let mut ledger = empty_ledger(vec![
        account_root_with_balance(borrower, 0, 0, 1_000_000_000),
        account_root_with_balance(vault_pseudo, 0, 0, 0),
        account_root_with_balance(broker_pseudo, 0, 0, 0),
        loan,
        broker,
        vault,
    ]);
    ledger.set_rules(protocol::Rules::new([protocol::feature_id(
        "LendingProtocol",
    )]));

    let mut view = ApplyViewImpl::new(Arc::new(ledger), ApplyFlags::NONE);

    let tx = STTx::new(TxType::LOAN_PAY, |tx| {
        tx.set_account_id(get_field_by_symbol("sfAccount"), borrower);
        tx.set_field_h256(get_field_by_symbol("sfLoanID"), loan_id);
        tx.set_field_amount(
            get_field_by_symbol("sfAmount"),
            STAmount::new_native(100_000_000, false),
        );
        tx.set_field_amount(
            get_field_by_symbol("sfFee"),
            STAmount::new_native(10, false),
        );
        tx.set_field_u32(get_field_by_symbol("sfSequence"), 1);
    });

    let result = handle_real_dispatch(&mut view, &tx, TxType::LOAN_PAY, None);
    assert_eq!(result, protocol::Ter::TES_SUCCESS);

    // Verify outstanding reduced on the real loan fields.
    let updated_loan = view
        .read(protocol::loan_keylet_from_key(loan_id))
        .expect("loan read")
        .expect("loan should still exist");
    assert_eq!(
        updated_loan
            .get_field_number(get_field_by_symbol("sfPrincipalOutstanding"))
            .value(),
        RuntimeNumber::from_i64(400_000_000)
    );
    assert_eq!(
        updated_loan
            .get_field_number(get_field_by_symbol("sfTotalValueOutstanding"))
            .value(),
        RuntimeNumber::from_i64(400_000_000)
    );

    // Verify payments remaining decremented
    assert_eq!(
        updated_loan.get_field_u32(get_field_by_symbol("sfPaymentRemaining")),
        9
    );
}

#[test]
fn amm_vote_dispatch_updates_vote_slots_and_weighted_fee() {
    let voter = sample_account(0x11);
    let existing_lp = sample_account(0x12);
    let amm_account = sample_account(0x22);
    let usd_issuer = sample_account(0x31);
    let eur_issuer = sample_account(0x32);
    let usd = Issue::new(currency_from_string("USD"), usd_issuer);
    let eur = Issue::new(currency_from_string("EUR"), eur_issuer);
    let lpt_currency = amm_lpt_currency(usd.currency, eur.currency);

    let amm = amm_entry(
        amm_account,
        usd,
        eur,
        1_000,
        vec![amm_vote_entry(existing_lp, 20, 40_000)],
        2,
    );
    let mut ledger = empty_ledger(vec![
        account_root(amm_account, 0, 0),
        account_root(voter, 0, 0),
        account_root(existing_lp, 0, 0),
        amm,
        trust_line_entry(voter, amm_account, lpt_currency, 600),
        trust_line_entry(existing_lp, amm_account, lpt_currency, 400),
    ]);
    ledger.set_rules(protocol::Rules::new([protocol::feature_id("AMM")]));
    let mut view = ApplyViewImpl::new(Arc::new(ledger), ApplyFlags::NONE);

    let tx = STTx::new(TxType::AMM_VOTE, |tx| {
        tx.set_account_id(sf("sfAccount"), voter);
        tx.set_field_issue(
            sf("sfAsset"),
            STIssue::new_with_asset(sf("sfAsset"), Asset::Issue(usd)),
        );
        tx.set_field_issue(
            sf("sfAsset2"),
            STIssue::new_with_asset(sf("sfAsset2"), Asset::Issue(eur)),
        );
        tx.set_field_u16(sf("sfTradingFee"), 50);
        tx.set_field_amount(
            sf("sfFee"),
            STAmount::from_xrp_amount(XRPAmount::from_drops(10)),
        );
        tx.set_field_u32(sf("sfSequence"), 1);
    });

    let result = handle_real_dispatch(&mut view, &tx, TxType::AMM_VOTE, None);
    assert_eq!(result, Ter::TES_SUCCESS);

    let amm_entry = view
        .read(protocol::keylet::amm(Asset::Issue(usd), Asset::Issue(eur)))
        .expect("amm read")
        .expect("amm should remain");
    assert_eq!(amm_entry.get_field_u16(sf("sfTradingFee")), 38);
    let votes = amm_entry.get_field_array(sf("sfVoteSlots"));
    assert_eq!(votes.len(), 2);
    let new_vote = votes
        .iter()
        .find(|entry| entry.get_account_id(sf("sfAccount")) == voter)
        .expect("new vote should be present");
    assert_eq!(new_vote.get_field_u16(sf("sfTradingFee")), 50);
    assert_eq!(new_vote.get_field_u32(sf("sfVoteWeight")), 60_000);
    let existing_vote = votes
        .iter()
        .find(|entry| entry.get_account_id(sf("sfAccount")) == existing_lp)
        .expect("existing vote should remain");
    assert_eq!(existing_vote.get_field_u16(sf("sfTradingFee")), 20);
    assert_eq!(existing_vote.get_field_u32(sf("sfVoteWeight")), 40_000);
    let auction_slot = amm_entry.get_field_object(sf("sfAuctionSlot"));
    assert_eq!(auction_slot.get_field_u16(sf("sfDiscountedFee")), 3);
}

// ============================================================================
// LOAN_PAY mode tests: Late, Full, Overpayment
// ============================================================================

fn loan_pay_ledger(
    borrower: AccountID,
    broker_owner: AccountID,
    broker_pseudo: AccountID,
    vault_pseudo: AccountID,
    loan_id: Uint256,
    broker_id: Uint256,
    vault_id: Uint256,
    asset: Asset,
    principal: i64,
    periodic_payment: i64,
    debt_total: i64,
    payment_remaining: u32,
) -> Ledger {
    let mut loan = managed_loan_entry(
        loan_id,
        borrower,
        broker_id,
        principal,
        principal,
        0,
        payment_remaining,
        120,
    );
    loan.set_field_number(
        get_field_by_symbol("sfPeriodicPayment"),
        asset_number(asset, periodic_payment),
    );

    let broker = loan_broker_entry(
        broker_id,
        broker_owner,
        broker_pseudo,
        vault_id,
        asset,
        debt_total,
        0,
        0,
        0,
    );

    let mut vault = vault_entry(broker_owner, vault_pseudo, 1, asset);
    vault.set_field_number(
        get_field_by_symbol("sfAssetsAvailable"),
        asset_number(asset, 0),
    );
    vault.set_field_number(
        get_field_by_symbol("sfAssetsTotal"),
        asset_number(asset, principal),
    );

    let mut ledger = empty_ledger(vec![
        account_root_with_balance(borrower, 0, 0, 1_000_000_000),
        account_root_with_balance(vault_pseudo, 0, 0, 0),
        account_root_with_balance(broker_pseudo, 0, 0, 0),
        loan,
        broker,
        vault,
    ]);
    ledger.set_rules(protocol::Rules::new([protocol::feature_id(
        "LendingProtocol",
    )]));
    ledger
}

#[test]
fn loan_pay_late_mode_reduces_principal_and_decrements_payment_remaining() {
    let borrower = sample_account(0xE5);
    let broker_owner = sample_account(0xE6);
    let broker_pseudo = sample_account(0xE7);
    let vault_pseudo = sample_account(0xE8);
    let loan_id = sample_uint256(0xF5);
    let broker_id = sample_uint256(0xF6);
    let vault_id = protocol::vault_keylet(raw_account_id(broker_owner), 1).key;
    let asset = Asset::Issue(xrp_issue());

    let ledger = loan_pay_ledger(
        borrower,
        broker_owner,
        broker_pseudo,
        vault_pseudo,
        loan_id,
        broker_id,
        vault_id,
        asset,
        500_000_000,
        50_000_000,
        500_000_000,
        10,
    );
    let mut view = ApplyViewImpl::new(Arc::new(ledger), ApplyFlags::NONE);

    // LOAN_LATE_PAYMENT_FLAG = 0x0004_0000
    let tx = STTx::new(TxType::LOAN_PAY, |tx| {
        tx.set_account_id(get_field_by_symbol("sfAccount"), borrower);
        tx.set_field_h256(get_field_by_symbol("sfLoanID"), loan_id);
        tx.set_field_amount(
            get_field_by_symbol("sfAmount"),
            STAmount::new_native(50_000_000, false),
        );
        tx.set_field_amount(
            get_field_by_symbol("sfFee"),
            STAmount::new_native(10, false),
        );
        tx.set_field_u32(get_field_by_symbol("sfSequence"), 1);
        tx.set_field_u32(
            get_field_by_symbol("sfFlags"),
            protocol::LOAN_LATE_PAYMENT_FLAG,
        );
    });

    let result = handle_real_dispatch(&mut view, &tx, TxType::LOAN_PAY, None);
    assert_eq!(result, protocol::Ter::TES_SUCCESS);

    let updated_loan = view
        .read(protocol::loan_keylet_from_key(loan_id))
        .expect("loan read")
        .expect("loan should still exist");
    assert_eq!(
        updated_loan
            .get_field_number(get_field_by_symbol("sfPrincipalOutstanding"))
            .value(),
        RuntimeNumber::from_i64(450_000_000)
    );
    assert_eq!(
        updated_loan.get_field_u32(get_field_by_symbol("sfPaymentRemaining")),
        9
    );
}

#[test]
fn loan_pay_full_mode_clears_principal_and_payment_remaining() {
    let borrower = sample_account(0xE9);
    let broker_owner = sample_account(0xEA);
    let broker_pseudo = sample_account(0xEB);
    let vault_pseudo = sample_account(0xEC);
    let loan_id = sample_uint256(0xF7);
    let broker_id = sample_uint256(0xF8);
    let vault_id = protocol::vault_keylet(raw_account_id(broker_owner), 1).key;
    let asset = Asset::Issue(xrp_issue());

    let ledger = loan_pay_ledger(
        borrower,
        broker_owner,
        broker_pseudo,
        vault_pseudo,
        loan_id,
        broker_id,
        vault_id,
        asset,
        500_000_000,
        50_000_000,
        500_000_000,
        10,
    );
    let mut view = ApplyViewImpl::new(Arc::new(ledger), ApplyFlags::NONE);

    // LOAN_FULL_PAYMENT_FLAG = 0x0002_0000
    let tx = STTx::new(TxType::LOAN_PAY, |tx| {
        tx.set_account_id(get_field_by_symbol("sfAccount"), borrower);
        tx.set_field_h256(get_field_by_symbol("sfLoanID"), loan_id);
        tx.set_field_amount(
            get_field_by_symbol("sfAmount"),
            STAmount::new_native(500_000_000, false),
        );
        tx.set_field_amount(
            get_field_by_symbol("sfFee"),
            STAmount::new_native(10, false),
        );
        tx.set_field_u32(get_field_by_symbol("sfSequence"), 1);
        tx.set_field_u32(
            get_field_by_symbol("sfFlags"),
            protocol::LOAN_FULL_PAYMENT_FLAG,
        );
    });

    let result = handle_real_dispatch(&mut view, &tx, TxType::LOAN_PAY, None);
    assert_eq!(result, protocol::Ter::TES_SUCCESS);

    let updated_loan = view
        .read(protocol::loan_keylet_from_key(loan_id))
        .expect("loan read")
        .expect("loan should still exist");
    assert_eq!(
        updated_loan
            .get_field_number(get_field_by_symbol("sfPrincipalOutstanding"))
            .value(),
        RuntimeNumber::zero()
    );
    assert_eq!(
        updated_loan.get_field_u32(get_field_by_symbol("sfPaymentRemaining")),
        0
    );
}

#[test]
fn loan_pay_overpayment_mode_reduces_principal_by_multiple_periods() {
    let borrower = sample_account(0xED);
    let broker_owner = sample_account(0xEE);
    let broker_pseudo = sample_account(0xEF);
    let vault_pseudo = sample_account(0xF0);
    let loan_id = sample_uint256(0xF9);
    let broker_id = sample_uint256(0xFA);
    let vault_id = protocol::vault_keylet(raw_account_id(broker_owner), 1).key;
    let asset = Asset::Issue(xrp_issue());

    let ledger = loan_pay_ledger(
        borrower,
        broker_owner,
        broker_pseudo,
        vault_pseudo,
        loan_id,
        broker_id,
        vault_id,
        asset,
        500_000_000,
        50_000_000,
        500_000_000,
        10,
    );
    let mut view = ApplyViewImpl::new(Arc::new(ledger), ApplyFlags::NONE);

    // LOAN_OVERPAYMENT_FLAG = 0x0001_0000 — pay 3 periods at once
    let tx = STTx::new(TxType::LOAN_PAY, |tx| {
        tx.set_account_id(get_field_by_symbol("sfAccount"), borrower);
        tx.set_field_h256(get_field_by_symbol("sfLoanID"), loan_id);
        tx.set_field_amount(
            get_field_by_symbol("sfAmount"),
            STAmount::new_native(150_000_000, false),
        );
        tx.set_field_amount(
            get_field_by_symbol("sfFee"),
            STAmount::new_native(10, false),
        );
        tx.set_field_u32(get_field_by_symbol("sfSequence"), 1);
        tx.set_field_u32(
            get_field_by_symbol("sfFlags"),
            protocol::LOAN_OVERPAYMENT_FLAG,
        );
    });

    let result = handle_real_dispatch(&mut view, &tx, TxType::LOAN_PAY, None);
    assert_eq!(result, protocol::Ter::TES_SUCCESS);

    let updated_loan = view
        .read(protocol::loan_keylet_from_key(loan_id))
        .expect("loan read")
        .expect("loan should still exist");
    // 3 periods × 50M = 150M paid → 500M - 150M = 350M remaining
    assert_eq!(
        updated_loan
            .get_field_number(get_field_by_symbol("sfPrincipalOutstanding"))
            .value(),
        RuntimeNumber::from_i64(350_000_000)
    );
    assert!(updated_loan.get_field_u32(get_field_by_symbol("sfPaymentRemaining")) < 10);
}
