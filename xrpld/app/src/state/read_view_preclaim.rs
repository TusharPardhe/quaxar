//! Read-only transaction-family preclaims used by the live application dispatcher.
//!
//! The generic transactor gates (account, sequence, signature, and fee) remain
//! outside this module.  This module only translates an immutable `ReadView`
//! and a typed transaction into the existing transaction-family fact helpers.
//! It intentionally performs no apply, sandbox, or dry-run work.

use std::collections::HashSet;

use basics::base_uint::{Uint160, Uint256};
use ledger::ReadView;
use protocol::{
    AccountID, ApplyFlags, Asset, STAmount, STTx, Ter, TxType, feature_batch_v1_1,
    get_field_by_symbol, is_tes_success, lsfAllowTrustLineLocking, lsfDepositAuth,
    lsfDisallowIncomingCheck, lsfDisallowIncomingPayChan, lsfGlobalFreeze, lsfHighAuth,
    lsfHighDeepFreeze, lsfHighFreeze, lsfLowAuth, lsfLowDeepFreeze, lsfLowFreeze, lsfRequireAuth,
    lsfRequireDestTag,
};
use tx::{
    AccountDeleteDirectoryEntryDisposition, AccountDeletePreclaimNftAndSequenceFacts,
    AccountDeletePreclaimScanState, AccountSetPreclaimFacts, CheckCancelPreclaimFacts,
    CheckCreatePreclaimFacts, PaymentChannelCreatePreclaimFacts, PaymentPreclaimFacts,
    run_account_delete_preclaim_directory_scan, run_account_delete_preclaim_nft_and_sequence,
    run_account_set_preclaim, run_check_cancel_preclaim, run_check_create_preclaim,
    run_payment_channel_claim_preclaim, run_payment_channel_create_preclaim,
    run_payment_preclaim_with_facts,
};

fn sf(name: &str) -> &'static protocol::SField {
    get_field_by_symbol(name)
}

fn account_keylet(account: AccountID) -> protocol::Keylet {
    protocol::account_keylet(Uint160::from_void(account.data()))
}

fn view_error() -> Ter {
    Ter::TEF_BAD_LEDGER
}

fn read_account<V: ReadView>(
    view: &V,
    account: AccountID,
) -> Result<Option<std::sync::Arc<protocol::STLedgerEntry>>, Ter> {
    view.read(account_keylet(account)).map_err(|_| view_error())
}

fn account_is_pseudo(sle: &protocol::STLedgerEntry) -> bool {
    ledger::is_pseudo_account(sle)
}

fn has_expired<V: ReadView>(view: &V, expiration: Option<u32>) -> bool {
    expiration.is_some_and(|expiration| view.header().parent_close_time >= expiration)
}

fn account_flag(sle: &protocol::STLedgerEntry, flag: u32) -> bool {
    sle.get_field_u32(sf("sfFlags")) & flag != 0
}

fn directory_entries<V: ReadView>(view: &V, account: AccountID) -> Result<Vec<Uint256>, Ter> {
    let root = protocol::owner_dir_keylet(Uint160::from_void(account.data()));
    let Some(_) = view.read(root).map_err(|_| view_error())? else {
        return Ok(Vec::new());
    };

    let mut result = Vec::new();
    let mut seen = HashSet::new();
    let mut page = 0_u64;
    loop {
        if !seen.insert(page) {
            return Err(view_error());
        }
        let keylet = protocol::page_keylet(root, page);
        let Some(node) = view.read(keylet).map_err(|_| view_error())? else {
            return Err(view_error());
        };
        result.extend(node.get_field_v256(sf("sfIndexes")).value().iter().copied());
        let next = node.get_field_u64(sf("sfIndexNext"));
        if next == 0 {
            return Ok(result);
        }
        page = next;
    }
}

fn iou_frozen<V: ReadView>(
    view: &V,
    account: AccountID,
    issue: protocol::Issue,
) -> Result<bool, Ter> {
    if issue.native() || account == issue.account {
        return Ok(false);
    }
    let Some(issuer) = read_account(view, issue.account)? else {
        return Ok(false);
    };
    if account_flag(&issuer, lsfGlobalFreeze) {
        return Ok(true);
    }
    let Some(line) = view
        .read(protocol::line(account, issue.account, issue.currency))
        .map_err(|_| view_error())?
    else {
        return Ok(false);
    };
    Ok(account_flag(
        &line,
        if issue.account > account {
            lsfHighFreeze
        } else {
            lsfLowFreeze
        },
    ))
}

fn iou_trustline_frozen<V: ReadView>(
    view: &V,
    account: AccountID,
    issue: protocol::Issue,
) -> Result<bool, Ter> {
    if account == issue.account {
        return Ok(false);
    }
    let Some(line) = view
        .read(protocol::line(account, issue.account, issue.currency))
        .map_err(|_| view_error())?
    else {
        return Ok(false);
    };
    Ok(account_flag(
        &line,
        if issue.account > account {
            lsfHighFreeze
        } else {
            lsfLowFreeze
        },
    ))
}

fn iou_deep_frozen<V: ReadView>(
    view: &V,
    account: AccountID,
    issue: protocol::Issue,
) -> Result<bool, Ter> {
    if issue.native() || account == issue.account {
        return Ok(false);
    }
    let Some(line) = view
        .read(protocol::line(account, issue.account, issue.currency))
        .map_err(|_| view_error())?
    else {
        return Ok(false);
    };
    Ok(account_flag(
        &line,
        if issue.account > account {
            lsfHighDeepFreeze
        } else {
            lsfLowDeepFreeze
        },
    ))
}

fn iou_auth<V: ReadView>(view: &V, account: AccountID, issue: protocol::Issue) -> Result<Ter, Ter> {
    if issue.native() || account == issue.account {
        return Ok(Ter::TES_SUCCESS);
    }
    let Some(issuer) = read_account(view, issue.account)? else {
        return Ok(Ter::TEC_NO_ISSUER);
    };
    if !account_flag(&issuer, lsfRequireAuth) {
        return Ok(Ter::TES_SUCCESS);
    }
    let Some(line) = view
        .read(protocol::line(account, issue.account, issue.currency))
        .map_err(|_| view_error())?
    else {
        return Ok(Ter::TEC_NO_AUTH);
    };
    Ok(
        if account_flag(
            &line,
            if account > issue.account {
                lsfLowAuth
            } else {
                lsfHighAuth
            },
        ) {
            Ter::TES_SUCCESS
        } else {
            Ter::TEC_NO_AUTH
        },
    )
}

fn account_holds_at_least<V: ReadView>(
    view: &V,
    account: AccountID,
    amount: &STAmount,
    include_check_reserve: bool,
) -> Result<bool, Ter> {
    match amount.asset() {
        Asset::Issue(issue) if issue.native() => {
            let Some(root) = read_account(view, account)? else {
                return Ok(false);
            };
            let balance = view
                .balance_hook_iou(
                    account,
                    protocol::xrp_account(),
                    root.get_field_amount(sf("sfBalance")),
                )
                .xrp()
                .drops();
            let owner_count = view
                .owner_count_hook(account, ledger::OwnerCounts::from_sle(&root))
                .count();
            let reserve = if ledger::is_pseudo_account(&root) {
                0
            } else {
                ledger::effective_account_reserve_with_owner_count(
                    view.fees(),
                    &root,
                    owner_count,
                    0,
                    0,
                ) as i64
            };
            let available = balance.saturating_sub(reserve)
                + if include_check_reserve {
                    view.fees().increment as i64
                } else {
                    0
                };
            Ok(amount.xrp().drops() <= available)
        }
        Asset::Issue(issue) => {
            if account == issue.account {
                return Ok(true);
            }
            let Some(line) = view
                .read(protocol::line(account, issue.account, issue.currency))
                .map_err(|_| view_error())?
            else {
                return Ok(false);
            };
            let mut balance = line.get_field_amount(sf("sfBalance"));
            if account > issue.account {
                balance.negate();
            }
            balance.set_issuer(issue.account);
            Ok(amount <= &balance)
        }
        Asset::MPTIssue(issue) => {
            if account == issue.issuer() {
                let Some(issuance) = view
                    .read(protocol::mpt_issuance_keylet_from_mptid(issue.mpt_id()))
                    .map_err(|_| view_error())?
                else {
                    return Ok(false);
                };
                let max = if issuance.is_field_present(sf("sfMaximumAmount")) {
                    issuance.get_field_u64(sf("sfMaximumAmount"))
                } else {
                    i64::MAX as u64
                };
                return Ok(amount.mpt().value() >= 0
                    && (amount.mpt().value() as u64)
                        <= max.saturating_sub(issuance.get_field_u64(sf("sfOutstandingAmount"))));
            }
            let Some(token) = view
                .read(protocol::mptoken_keylet_from_mptid(
                    issue.mpt_id(),
                    Uint160::from_void(account.data()),
                ))
                .map_err(|_| view_error())?
            else {
                return Ok(false);
            };
            Ok(amount.mpt().value() >= 0
                && (amount.mpt().value() as u64) <= token.get_field_u64(sf("sfMPTAmount")))
        }
    }
}

fn read_check<V: ReadView>(
    view: &V,
    id: Uint256,
) -> Result<Option<std::sync::Arc<protocol::STLedgerEntry>>, Ter> {
    view.read(protocol::check_keylet_from_key(id))
        .map_err(|_| view_error())
}

fn read_escrow<V: ReadView>(
    view: &V,
    owner: AccountID,
    sequence: u32,
) -> Result<Option<std::sync::Arc<protocol::STLedgerEntry>>, Ter> {
    view.read(protocol::escrow_keylet(
        Uint160::from_void(owner.data()),
        sequence,
    ))
    .map_err(|_| view_error())
}

/// Runs only the requested account, payment, check, and escrow preclaims.
/// `None` deliberately means an out-of-scope transaction type and lets the
/// existing generic dispatcher continue unchanged.
pub fn run_read_view_preclaim<V: ReadView>(
    view: &V,
    tx: &STTx,
    txn_type: TxType,
    apply_flags: ApplyFlags,
) -> Option<Ter> {
    let result = match txn_type {
        TxType::ACCOUNT_SET => preclaim_account_set(view, tx, apply_flags),
        TxType::ACCOUNT_DELETE => preclaim_account_delete(view, tx),
        TxType::DELEGATE_SET => preclaim_delegate_set(view, tx),
        TxType::REGULAR_KEY_SET => preclaim_set_regular_key(),
        TxType::SIGNER_LIST_SET => preclaim_signer_list_set(),
        TxType::DEPOSIT_PREAUTH => preclaim_deposit_preauth(view, tx),
        TxType::PAYMENT => preclaim_payment(view, tx, apply_flags),
        TxType::PAYCHAN_CREATE => preclaim_payment_channel_create(view, tx),
        TxType::PAYCHAN_FUND => preclaim_payment_channel_fund(),
        TxType::PAYCHAN_CLAIM => preclaim_payment_channel_claim(view, tx),
        TxType::CHECK_CREATE => preclaim_check_create(view, tx),
        TxType::CHECK_CASH => preclaim_check_cash(view, tx),
        TxType::CHECK_CANCEL => preclaim_check_cancel(view, tx),
        TxType::ESCROW_CREATE => preclaim_escrow_create(view, tx),
        TxType::ESCROW_FINISH => preclaim_escrow_finish(view, tx),
        TxType::ESCROW_CANCEL => preclaim_escrow_cancel(view, tx),
        _ => return None,
    };
    Some(result.unwrap_or_else(|ter| ter))
}

fn preclaim_account_set<V: ReadView>(
    view: &V,
    tx: &STTx,
    apply_flags: ApplyFlags,
) -> Result<Ter, Ter> {
    let account = tx.get_account_id(sf("sfAccount"));
    let account_root = read_account(view, account)?;
    let Some(account_root_ref) = account_root.as_ref() else {
        return Ok(Ter::TER_NO_ACCOUNT);
    };
    let account_flags = account_root_ref.get_field_u32(sf("sfFlags"));
    let set_flag = tx.get_field_u32(sf("sfSetFlag"));
    let setting_require_auth = (tx.get_flags() & tx::ACCOUNT_SET_REQUIRE_AUTH_FLAG != 0)
        || set_flag == tx::ASF_REQUIRE_AUTH;
    let needs_owner_directory = (setting_require_auth && account_flags & tx::LSF_REQUIRE_AUTH == 0)
        || (set_flag == tx::ASF_ALLOW_TRUST_LINE_CLAWBACK
            && account_flags & tx::LSF_NO_FREEZE == 0);
    let owner_dir_empty = if needs_owner_directory {
        directory_entries(view, account)?.is_empty()
    } else {
        true
    };
    Ok(run_account_set_preclaim(AccountSetPreclaimFacts {
        tx_flags: tx.get_flags(),
        set_flag,
        apply_flags,
        account_exists: true,
        account_flags,
        owner_dir_empty,
        feature_clawback_enabled: view.rules().enabled(&protocol::feature_clawback()),
    }))
}

fn preclaim_account_delete<V: ReadView>(view: &V, tx: &STTx) -> Result<Ter, Ter> {
    let account = tx.get_account_id(sf("sfAccount"));
    let destination = tx.get_account_id(sf("sfDestination"));
    let Some(destination_sle) = read_account(view, destination)? else {
        return Ok(Ter::TEC_NO_DST);
    };
    if account_flag(&destination_sle, lsfRequireDestTag)
        && !tx.is_field_present(sf("sfDestinationTag"))
    {
        return Ok(Ter::TEC_DST_TAG_NEEDED);
    }

    let credentials_present = tx.is_field_present(sf("sfCredentialIDs"));
    let credentials =
        ledger::credential_helpers::valid(view, tx, &account).map_err(|_| view_error())?;
    if credentials != Ter::TES_SUCCESS {
        return Ok(credentials);
    }
    if !credentials_present && account_flag(&destination_sle, lsfDepositAuth) {
        let preauthorized = view
            .exists(protocol::deposit_preauth_keylet(
                Uint160::from_void(destination.data()),
                Uint160::from_void(account.data()),
            ))
            .map_err(|_| view_error())?;
        if !preauthorized {
            return Ok(Ter::TEC_NO_PERMISSION);
        }
    }

    let Some(source) = read_account(view, account)? else {
        return Ok(Ter::TER_NO_ACCOUNT);
    };
    let minted_nftokens = source.get_field_u32(sf("sfMintedNFTokens"));
    let burned_nftokens = source.get_field_u32(sf("sfBurnedNFTokens"));
    if minted_nftokens != burned_nftokens {
        return Ok(Ter::TEC_HAS_OBLIGATIONS);
    }
    let nft_min = protocol::nft_page_min_keylet(Uint160::from_void(account.data()));
    let nft_max = protocol::nft_page_max_keylet(Uint160::from_void(account.data()));
    let owned_nft_page_present = view
        .succ(nft_min.key, Some(nft_max.key.next()))
        .map_err(|_| view_error())?
        .is_some();
    match run_account_delete_preclaim_nft_and_sequence(AccountDeletePreclaimNftAndSequenceFacts {
        minted_nftokens,
        burned_nftokens,
        owned_nft_page_present,
        sponsor_mismatch: source.is_field_present(sf("sfSponsor"))
            && source.get_account_id(sf("sfSponsor")) != destination,
        sponsoring_dependents: source.is_field_present(sf("sfSponsoringOwnerCount"))
            || source.is_field_present(sf("sfSponsoringAccountCount")),
        account_sequence: source.get_field_u32(sf("sfSequence")),
        ledger_sequence: view.seq(),
        first_nftoken_sequence: source
            .is_field_present(sf("sfFirstNFTokenSequence"))
            .then(|| source.get_field_u32(sf("sfFirstNFTokenSequence"))),
        // rippled does not inspect the owner directory until every NFT,
        // sponsorship, and sequence-age check above has passed.  Force the
        // helper through those earlier checks first, then evaluate the
        // directory lazily below so a malformed page cannot override their
        // canonical TER precedence.
        owner_dir_empty: false,
    }) {
        AccountDeletePreclaimScanState::Return(ter) => Ok(ter),
        AccountDeletePreclaimScanState::ContinueToDirectoryScan => {
            let directory_keys = directory_entries(view, account)?;
            if directory_keys.is_empty() {
                return Ok(Ter::TES_SUCCESS);
            }
            let mut entries = Vec::new();
            for key in directory_keys {
                let disposition = match view
                    .read(protocol::child_keylet(key))
                    .map_err(|_| view_error())?
                {
                    None => AccountDeleteDirectoryEntryDisposition::MissingObject,
                    Some(sle)
                        if matches!(
                            sle.get_type(),
                            protocol::LedgerEntryType::Offer
                                | protocol::LedgerEntryType::SignerList
                                | protocol::LedgerEntryType::Ticket
                                | protocol::LedgerEntryType::DepositPreauth
                                | protocol::LedgerEntryType::NFTokenOffer
                                | protocol::LedgerEntryType::DID
                                | protocol::LedgerEntryType::Oracle
                                | protocol::LedgerEntryType::Credential
                                | protocol::LedgerEntryType::Delegate
                        ) =>
                    {
                        AccountDeleteDirectoryEntryDisposition::Deletable
                    }
                    Some(_) => AccountDeleteDirectoryEntryDisposition::Undeletable,
                };
                entries.push(disposition);
            }
            Ok(run_account_delete_preclaim_directory_scan(
                !entries.is_empty(),
                &entries,
            ))
        }
    }
}

fn preclaim_delegate_set<V: ReadView>(view: &V, tx: &STTx) -> Result<Ter, Ter> {
    let account = tx.get_account_id(sf("sfAccount"));
    let authorize = tx.get_account_id(sf("sfAuthorize"));
    if !view
        .exists(account_keylet(account))
        .map_err(|_| view_error())?
    {
        return Ok(Ter::TER_NO_ACCOUNT);
    }
    let Some(authorize_sle) = read_account(view, authorize)? else {
        return Ok(Ter::TEC_NO_TARGET);
    };
    if ledger::is_pseudo_account(&authorize_sle) {
        return Ok(Ter::TEC_PSEUDO_ACCOUNT);
    }
    if tx.get_field_array(sf("sfPermissions")).is_empty()
        && !view
            .exists(protocol::delegate_keylet(
                Uint160::from_void(account.data()),
                Uint160::from_void(authorize.data()),
            ))
            .map_err(|_| view_error())?
    {
        return Ok(Ter::TEC_NO_ENTRY);
    }
    Ok(Ter::TES_SUCCESS)
}

// rippled SetRegularKey and SignerListSet inherit Transactor::preclaim without
// a family override. These explicit, family-local no-op adapters prevent their
// treatment as an out-of-scope dispatcher default and perform no view reads.
fn preclaim_set_regular_key() -> Result<Ter, Ter> {
    Ok(Ter::TES_SUCCESS)
}

fn preclaim_signer_list_set() -> Result<Ter, Ter> {
    Ok(Ter::TES_SUCCESS)
}

fn preclaim_deposit_preauth<V: ReadView>(view: &V, tx: &STTx) -> Result<Ter, Ter> {
    let account = tx.get_account_id(sf("sfAccount"));
    let account_key = Uint160::from_void(account.data());
    if tx.is_field_present(sf("sfAuthorize")) {
        let target = tx.get_account_id(sf("sfAuthorize"));
        let Some(target_sle) = read_account(view, target)? else {
            return Ok(Ter::TEC_NO_TARGET);
        };
        if view
            .rules()
            .enabled(&protocol::feature_id("fixCleanup3_3_0"))
            && account_is_pseudo(&target_sle)
        {
            return Ok(Ter::TEC_PSEUDO_ACCOUNT);
        }
        return Ok(
            if view
                .exists(protocol::deposit_preauth_keylet(
                    account_key,
                    Uint160::from_void(target.data()),
                ))
                .map_err(|_| view_error())?
            {
                Ter::TEC_DUPLICATE
            } else {
                Ter::TES_SUCCESS
            },
        );
    }
    if tx.is_field_present(sf("sfUnauthorize")) {
        let target = tx.get_account_id(sf("sfUnauthorize"));
        return Ok(
            if view
                .exists(protocol::deposit_preauth_keylet(
                    account_key,
                    Uint160::from_void(target.data()),
                ))
                .map_err(|_| view_error())?
            {
                Ter::TES_SUCCESS
            } else {
                Ter::TEC_NO_ENTRY
            },
        );
    }

    let (credential_field, authorizing) = if tx.is_field_present(sf("sfAuthorizeCredentials")) {
        (sf("sfAuthorizeCredentials"), true)
    } else if tx.is_field_present(sf("sfUnauthorizeCredentials")) {
        (sf("sfUnauthorizeCredentials"), false)
    } else {
        return Ok(Ter::TES_SUCCESS);
    };
    let mut sorted = std::collections::BTreeSet::new();
    for credential in tx.get_field_array(credential_field).iter() {
        let issuer = credential.get_account_id(sf("sfIssuer"));
        let credential_type = credential.get_field_vl(sf("sfCredentialType"));
        if authorizing
            && !view
                .exists(account_keylet(issuer))
                .map_err(|_| view_error())?
        {
            return Ok(Ter::TEC_NO_ISSUER);
        }
        if !sorted.insert((issuer, credential_type)) {
            return Ok(Ter::TEF_INTERNAL);
        }
    }
    let credential_hashes = sorted
        .iter()
        .map(|(issuer, credential_type)| {
            protocol::sha512_half_slices(&[issuer.data(), credential_type])
        })
        .collect::<Vec<_>>();
    let exists = view
        .exists(protocol::deposit_preauth_credentials_keylet(
            account_key,
            &credential_hashes,
        ))
        .map_err(|_| view_error())?;
    Ok(match (authorizing, exists) {
        (true, true) => Ter::TEC_DUPLICATE,
        (false, false) => Ter::TEC_NO_ENTRY,
        _ => Ter::TES_SUCCESS,
    })
}

fn preclaim_payment<V: ReadView>(view: &V, tx: &STTx, apply_flags: ApplyFlags) -> Result<Ter, Ter> {
    let account = tx.get_account_id(sf("sfAccount"));
    let destination = tx.get_account_id(sf("sfDestination"));
    let amount = tx.get_field_amount(sf("sfAmount"));
    let destination_sle = read_account(view, destination)?;
    let sponsor_created_account = tx.get_flags() & protocol::tfSponsorCreatedAccount != 0;
    let paths = tx
        .is_field_present(sf("sfPaths"))
        .then(|| tx.get_field_path_set(sf("sfPaths")));
    let domain_id = tx
        .is_field_present(sf("sfDomainID"))
        .then(|| tx.get_field_h256(sf("sfDomainID")));
    // Pinned Payment::preclaim performs all destination/tag/path decisions
    // before reading credentials or permissioned-domain objects.  Keep those
    // later reads lazy so an unrelated SHAMap fault cannot override the
    // canonical earlier TER.
    let early = run_payment_preclaim_with_facts(PaymentPreclaimFacts {
        tx_flags: tx.get_flags(),
        has_paths: paths.is_some(),
        send_max_present: tx.is_field_present(sf("sfSendMax")),
        dst_amount_native: amount.native(),
        destination_exists: destination_sle.is_some(),
        sponsor_created_account,
        view_open: view.open(),
        destination_requires_tag: destination_sle
            .as_ref()
            .is_some_and(|sle| account_flag(sle, lsfRequireDestTag)),
        destination_tag_present: tx.is_field_present(sf("sfDestinationTag")),
        destination_can_create_with_amount: amount.native()
            && (sponsor_created_account || amount.xrp().drops() >= view.fees().reserve as i64),
        path_count: paths.as_ref().map_or(0, |paths| paths.size()),
        path_has_too_long_segment: paths
            .as_ref()
            .is_some_and(|paths| paths.iter().any(|path| path.size() > tx::MAX_PATH_LENGTH)),
        credentials_valid_result: Ter::TES_SUCCESS,
        domain_id_present: false,
        source_in_domain: true,
        destination_in_domain: true,
        is_batch_inner: tx.get_flags() & protocol::INNER_BATCH_TRANSACTION_FLAG != 0
            || (apply_flags.bits() & ApplyFlags::BATCH.bits()) != 0,
        batch_v1_1_enabled: view.rules().enabled(&feature_batch_v1_1()),
    });
    if !is_tes_success(early) {
        return Ok(early);
    }

    let credentials =
        ledger::credential_helpers::valid(view, tx, &account).map_err(|_| view_error())?;
    if !is_tes_success(credentials) {
        return Ok(credentials);
    }
    if let Some(domain) = domain_id {
        if !ledger::permissioned_dex_helpers::account_in_domain(view, &account, &domain)
            .map_err(|_| view_error())?
        {
            return Ok(Ter::TEC_NO_PERMISSION);
        }
        if !ledger::permissioned_dex_helpers::account_in_domain(view, &destination, &domain)
            .map_err(|_| view_error())?
        {
            return Ok(Ter::TEC_NO_PERMISSION);
        }
    }
    Ok(Ter::TES_SUCCESS)
}

fn preclaim_payment_channel_create<V: ReadView>(view: &V, tx: &STTx) -> Result<Ter, Ter> {
    let account = tx.get_account_id(sf("sfAccount"));
    let destination = tx.get_account_id(sf("sfDestination"));
    let amount = tx.get_field_amount(sf("sfAmount"));
    let source = read_account(view, account)?;
    let Some(source) = source.as_ref() else {
        return Ok(Ter::TER_NO_ACCOUNT);
    };
    let (covers_reserve, covers_amount) = if view.rules().enabled(&protocol::feature_id("Sponsor"))
    {
        // rippled defers both reserve checks to doApply under Sponsor, where
        // the transaction reserve bearer and the source's post-lock reserve
        // can be evaluated independently.
        (true, true)
    } else {
        let balance = source.get_field_amount(sf("sfBalance")).xrp().drops();
        let reserve = ledger::effective_account_reserve(view.fees(), source, 1, 0) as i64;
        (
            balance >= reserve,
            balance >= reserve.saturating_add(amount.xrp().drops()),
        )
    };
    if !covers_reserve {
        return Ok(Ter::TEC_INSUFFICIENT_RESERVE);
    }
    if !covers_amount {
        return Ok(Ter::TEC_UNFUNDED);
    }
    let destination_sle = read_account(view, destination)?;
    Ok(run_payment_channel_create_preclaim(
        PaymentChannelCreatePreclaimFacts {
            source_account_exists: true,
            source_balance_covers_reserve: true,
            source_balance_covers_reserve_plus_amount: true,
            destination_exists: destination_sle.is_some(),
            destination_disallow_incoming_pay_chan: destination_sle
                .as_ref()
                .is_some_and(|sle| account_flag(sle, lsfDisallowIncomingPayChan)),
            destination_requires_dest_tag: destination_sle
                .as_ref()
                .is_some_and(|sle| account_flag(sle, lsfRequireDestTag)),
            destination_has_dest_tag: tx.is_field_present(sf("sfDestinationTag")),
            destination_is_pseudo_account: destination_sle
                .as_ref()
                .is_some_and(|sle| account_is_pseudo(sle)),
        },
    ))
}

// rippled PaymentChannelFund inherits Transactor::preclaim without a family
// override. Keep that exact no-op explicit rather than relying on a fallback.
fn preclaim_payment_channel_fund() -> Result<Ter, Ter> {
    Ok(Ter::TES_SUCCESS)
}

fn preclaim_payment_channel_claim<V: ReadView>(view: &V, tx: &STTx) -> Result<Ter, Ter> {
    let credentials_enabled = view.rules().enabled(&protocol::feature_id("Credentials"));
    if !credentials_enabled {
        return Ok(run_payment_channel_claim_preclaim(
            false,
            || Ter::TES_SUCCESS,
            || unreachable!("rippled skips credential validation before Credentials"),
        ));
    }

    let account = tx.get_account_id(sf("sfAccount"));
    let credentials =
        ledger::credential_helpers::valid(view, tx, &account).map_err(|_| view_error())?;
    Ok(run_payment_channel_claim_preclaim(
        true,
        || unreachable!("rippled bypasses Transactor::preclaim with Credentials"),
        || credentials,
    ))
}

fn preclaim_check_create<V: ReadView>(view: &V, tx: &STTx) -> Result<Ter, Ter> {
    let source = tx.get_account_id(sf("sfAccount"));
    let destination = tx.get_account_id(sf("sfDestination"));
    let amount = tx.get_field_amount(sf("sfSendMax"));
    let destination_sle = read_account(view, destination)?;
    let Some(destination_sle) = destination_sle.as_ref() else {
        return Ok(Ter::TEC_NO_DST);
    };
    if account_flag(destination_sle, lsfDisallowIncomingCheck) || account_is_pseudo(destination_sle)
    {
        return Ok(Ter::TEC_NO_PERMISSION);
    }
    if account_flag(destination_sle, lsfRequireDestTag)
        && !tx.is_field_present(sf("sfDestinationTag"))
    {
        return Ok(Ter::TEC_DST_TAG_NEEDED);
    }
    let expired = has_expired(
        view,
        tx.is_field_present(sf("sfExpiration"))
            .then(|| tx.get_field_u32(sf("sfExpiration"))),
    );
    let result = match amount.asset() {
        Asset::Issue(issue) if issue.native() => {
            run_check_create_preclaim(CheckCreatePreclaimFacts {
                destination_exists: true,
                destination_disallow_incoming_check: false,
                destination_is_pseudo_account: false,
                destination_require_dest_tag: false,
                tx_has_destination_tag: true,
                send_max_is_native: true,
                send_max_issuer_is_source: true,
                send_max_issuer_is_destination: true,
                send_max_issuer_globally_frozen: false,
                source_to_issuer_trustline_frozen: false,
                issuer_to_destination_trustline_frozen: false,
                tx_expired: expired,
            })
        }
        Asset::Issue(issue) => {
            let issuer = read_account(view, issue.account)?;
            let global = issuer
                .as_ref()
                .is_some_and(|sle| account_flag(sle, lsfGlobalFreeze));
            if global {
                return Ok(Ter::TEC_FROZEN);
            }
            let source_frozen = iou_trustline_frozen(view, source, issue)?;
            if source_frozen {
                return Ok(Ter::TEC_FROZEN);
            }
            let destination_frozen = iou_trustline_frozen(view, destination, issue)?;
            run_check_create_preclaim(CheckCreatePreclaimFacts {
                destination_exists: true,
                destination_disallow_incoming_check: false,
                destination_is_pseudo_account: false,
                destination_require_dest_tag: false,
                tx_has_destination_tag: true,
                send_max_is_native: false,
                send_max_issuer_is_source: issue.account == source,
                send_max_issuer_is_destination: issue.account == destination,
                send_max_issuer_globally_frozen: global,
                source_to_issuer_trustline_frozen: source_frozen,
                issuer_to_destination_trustline_frozen: destination_frozen,
                tx_expired: expired,
            })
        }
        Asset::MPTIssue(issue) => {
            let base = run_check_create_preclaim(CheckCreatePreclaimFacts {
                destination_exists: true,
                destination_disallow_incoming_check: false,
                destination_is_pseudo_account: false,
                destination_require_dest_tag: false,
                tx_has_destination_tag: true,
                send_max_is_native: true,
                send_max_issuer_is_source: false,
                send_max_issuer_is_destination: false,
                send_max_issuer_globally_frozen: false,
                source_to_issuer_trustline_frozen: false,
                issuer_to_destination_trustline_frozen: false,
                tx_expired: expired,
            });
            if base != Ter::TES_SUCCESS {
                base
            } else if (source != issue.issuer()
                && ledger::mptoken_helpers::is_frozen_mpt(view, &source, &issue)
                    .map_err(|_| view_error())?)
                || (destination != issue.issuer()
                    && ledger::mptoken_helpers::is_frozen_mpt(view, &destination, &issue)
                        .map_err(|_| view_error())?)
            {
                Ter::TEC_LOCKED
            } else {
                ledger::mptoken_helpers::can_transfer_mpt(view, &issue, &source, &destination)
                    .map_err(|_| view_error())?
            }
        }
    };
    Ok(result)
}

fn preclaim_check_cash<V: ReadView>(view: &V, tx: &STTx) -> Result<Ter, Ter> {
    let Some(check) = read_check(view, tx.get_field_h256(sf("sfCheckID")))? else {
        return Ok(Ter::TEC_NO_ENTRY);
    };
    let source = check.get_account_id(sf("sfAccount"));
    let destination = check.get_account_id(sf("sfDestination"));
    let requested = if tx.is_field_present(sf("sfAmount")) {
        tx.get_field_amount(sf("sfAmount"))
    } else if tx.is_field_present(sf("sfDeliverMin")) {
        tx.get_field_amount(sf("sfDeliverMin"))
    } else {
        return Ok(Ter::TEM_MALFORMED);
    };
    let send_max = check.get_field_amount(sf("sfSendMax"));
    if tx.get_account_id(sf("sfAccount")) != destination {
        return Ok(Ter::TEC_NO_PERMISSION);
    }
    if source == destination {
        return Ok(Ter::TEC_INTERNAL);
    }
    let source_sle = read_account(view, source)?;
    let destination_sle = read_account(view, destination)?;
    let (Some(_), Some(destination_sle)) = (source_sle, destination_sle) else {
        return Ok(Ter::TEC_NO_ENTRY);
    };
    if account_flag(&destination_sle, lsfRequireDestTag)
        && !check.is_field_present(sf("sfDestinationTag"))
    {
        return Ok(Ter::TEC_DST_TAG_NEEDED);
    }
    if has_expired(
        view,
        check
            .is_field_present(sf("sfExpiration"))
            .then(|| check.get_field_u32(sf("sfExpiration"))),
    ) {
        return Ok(Ter::TEC_EXPIRED);
    }
    if view
        .rules()
        .enabled(&protocol::feature_id("fixCleanup3_2_0"))
        && !send_max.is_legal_mpt()
    {
        return Ok(Ter::TEF_BAD_LEDGER);
    }
    if requested.asset() != send_max.asset()
        || requested.asset().issuer() != send_max.asset().issuer()
    {
        return Ok(Ter::TEM_MALFORMED);
    }
    if requested > send_max {
        return Ok(Ter::TEC_PATH_PARTIAL);
    }
    // CheckCash releases the source's owner-reserve increment only when the
    // Check itself is not sponsored. A sponsored Check is charged to its
    // sponsor, so removing it does not increase the source's XRP liquidity.
    // This is the `!sleCheck->isFieldPresent(sfSponsor)` branch in rippled's
    // CheckCash::preclaim.
    let releases_source_reserve = requested.native() && !check.is_field_present(sf("sfSponsor"));
    if !account_holds_at_least(view, source, &requested, releases_source_reserve)? {
        return Ok(Ter::TEC_PATH_PARTIAL);
    }
    if requested.native() || requested.asset().issuer() == destination {
        return Ok(Ter::TES_SUCCESS);
    }
    match requested.asset() {
        Asset::Issue(issue) => {
            let Some(issuer) = read_account(view, issue.account)? else {
                return Ok(Ter::TEC_NO_ISSUER);
            };
            if account_flag(&issuer, lsfRequireAuth)
                && iou_auth(view, destination, issue)? != Ter::TES_SUCCESS
            {
                return Ok(Ter::TEC_NO_AUTH);
            }
            if iou_frozen(view, destination, issue)? {
                return Ok(Ter::TEC_FROZEN);
            }
        }
        Asset::MPTIssue(issue) => {
            let Some(_) = read_account(view, issue.issuer())? else {
                return Ok(Ter::TEC_NO_ISSUER);
            };
            let auth = ledger::mptoken_helpers::require_auth_mpt(view, &issue, &destination)
                .map_err(|_| view_error())?;
            if auth != Ter::TES_SUCCESS {
                return Ok(auth);
            }
            if ledger::mptoken_helpers::is_frozen_mpt(view, &destination, &issue)
                .map_err(|_| view_error())?
            {
                return Ok(Ter::TEC_LOCKED);
            }
            let transfer =
                ledger::mptoken_helpers::can_transfer_mpt(view, &issue, &source, &destination)
                    .map_err(|_| view_error())?;
            if transfer != Ter::TES_SUCCESS {
                return Ok(transfer);
            }
        }
    }
    Ok(Ter::TES_SUCCESS)
}

fn preclaim_check_cancel<V: ReadView>(view: &V, tx: &STTx) -> Result<Ter, Ter> {
    let Some(check) = read_check(view, tx.get_field_h256(sf("sfCheckID")))? else {
        return Ok(Ter::TEC_NO_ENTRY);
    };
    let account = tx.get_account_id(sf("sfAccount"));
    Ok(run_check_cancel_preclaim(CheckCancelPreclaimFacts {
        check_exists: true,
        check_expired: has_expired(
            view,
            check
                .is_field_present(sf("sfExpiration"))
                .then(|| check.get_field_u32(sf("sfExpiration"))),
        ),
        tx_account_is_check_source: account == check.get_account_id(sf("sfAccount")),
        tx_account_is_check_destination: account == check.get_account_id(sf("sfDestination")),
    }))
}

fn preclaim_escrow_create<V: ReadView>(view: &V, tx: &STTx) -> Result<Ter, Ter> {
    let account = tx.get_account_id(sf("sfAccount"));
    let destination = tx.get_account_id(sf("sfDestination"));
    let amount = tx.get_field_amount(sf("sfAmount"));
    let Some(destination_sle) = read_account(view, destination)? else {
        return Ok(Ter::TEC_NO_DST);
    };
    if account_is_pseudo(&destination_sle) {
        return Ok(Ter::TEC_NO_PERMISSION);
    }
    if amount.native() {
        return Ok(Ter::TES_SUCCESS);
    }
    if !view.rules().enabled(&protocol::feature_token_escrow()) {
        return Ok(Ter::TEM_DISABLED);
    }

    match amount.asset() {
        Asset::Issue(issue) if issue.native() => Ok(Ter::TES_SUCCESS),
        Asset::Issue(issue) => {
            if issue.account == account {
                return Ok(Ter::TEC_NO_PERMISSION);
            }
            let Some(issuer) = read_account(view, issue.account)? else {
                return Ok(Ter::TEC_NO_ISSUER);
            };
            if !account_flag(&issuer, lsfAllowTrustLineLocking) {
                return Ok(Ter::TEC_NO_PERMISSION);
            }
            let Some(line) = view
                .read(protocol::line(account, issue.account, issue.currency))
                .map_err(|_| view_error())?
            else {
                return Ok(Ter::TEC_NO_LINE);
            };
            let mut spendable = line.get_field_amount(sf("sfBalance"));
            if account > issue.account {
                spendable.negate();
            }
            spendable.set_issuer(issue.account);
            if spendable.signum() < 0 {
                return Ok(Ter::TEC_NO_PERMISSION);
            }
            let auth = iou_auth(view, account, issue)?;
            if auth != Ter::TES_SUCCESS {
                return Ok(auth);
            }
            let auth = iou_auth(view, destination, issue)?;
            if auth != Ter::TES_SUCCESS {
                return Ok(auth);
            }
            if iou_frozen(view, account, issue)? || iou_frozen(view, destination, issue)? {
                return Ok(Ter::TEC_FROZEN);
            }
            if spendable.signum() <= 0 || amount > spendable {
                return Ok(Ter::TEC_INSUFFICIENT_FUNDS);
            }
            Ok(if spendable.iou().checked_add(amount.iou()).is_ok() {
                Ter::TES_SUCCESS
            } else {
                Ter::TEC_PRECISION_LOSS
            })
        }
        Asset::MPTIssue(issue) => {
            if issue.issuer() == account {
                return Ok(Ter::TEC_NO_PERMISSION);
            }
            let Some(issuance) = view
                .read(protocol::mpt_issuance_keylet_from_mptid(issue.mpt_id()))
                .map_err(|_| view_error())?
            else {
                return Ok(Ter::TEC_OBJECT_NOT_FOUND);
            };
            if !issuance.is_flag(protocol::lsfMPTCanEscrow) {
                return Ok(Ter::TEC_NO_PERMISSION);
            }
            if issuance.get_account_id(sf("sfIssuer")) != issue.issuer() {
                return Ok(Ter::TEC_NO_PERMISSION);
            }
            let Some(token) = view
                .read(protocol::mptoken_keylet_from_mptid(
                    issue.mpt_id(),
                    Uint160::from_void(account.data()),
                ))
                .map_err(|_| view_error())?
            else {
                return Ok(Ter::TEC_OBJECT_NOT_FOUND);
            };
            let auth = ledger::mptoken_helpers::require_auth_mpt(view, &issue, &account)
                .map_err(|_| view_error())?;
            if auth != Ter::TES_SUCCESS {
                return Ok(auth);
            }
            let auth = ledger::mptoken_helpers::require_auth_mpt(view, &issue, &destination)
                .map_err(|_| view_error())?;
            if auth != Ter::TES_SUCCESS {
                return Ok(auth);
            }
            if ledger::mptoken_helpers::is_frozen_mpt(view, &account, &issue)
                .map_err(|_| view_error())?
                || ledger::mptoken_helpers::is_frozen_mpt(view, &destination, &issue)
                    .map_err(|_| view_error())?
            {
                return Ok(Ter::TEC_LOCKED);
            }
            let transfer =
                ledger::mptoken_helpers::can_transfer_mpt(view, &issue, &account, &destination)
                    .map_err(|_| view_error())?;
            if transfer != Ter::TES_SUCCESS {
                return Ok(transfer);
            }
            let available = token.get_field_u64(sf("sfMPTAmount"));
            Ok(
                if amount.mpt().value() <= 0 || (amount.mpt().value() as u64) > available {
                    Ter::TEC_INSUFFICIENT_FUNDS
                } else {
                    Ter::TES_SUCCESS
                },
            )
        }
    }
}

fn preclaim_escrow_finish<V: ReadView>(view: &V, tx: &STTx) -> Result<Ter, Ter> {
    if view.rules().enabled(&protocol::feature_id("Credentials")) {
        let credentials =
            ledger::credential_helpers::valid(view, tx, &tx.get_account_id(sf("sfAccount")))
                .map_err(|_| view_error())?;
        if credentials != Ter::TES_SUCCESS {
            return Ok(credentials);
        }
    }
    if !view.rules().enabled(&protocol::feature_token_escrow()) {
        return Ok(Ter::TES_SUCCESS);
    }
    let Some(escrow) = read_escrow(
        view,
        tx.get_account_id(sf("sfOwner")),
        tx.get_field_u32(sf("sfOfferSequence")),
    )?
    else {
        return Ok(Ter::TEC_NO_TARGET);
    };
    match escrow.get_field_amount(sf("sfAmount")).asset() {
        Asset::Issue(issue) if issue.native() => Ok(Ter::TES_SUCCESS),
        Asset::Issue(issue) => {
            let destination = escrow.get_account_id(sf("sfDestination"));
            let auth = iou_auth(view, destination, issue)?;
            if auth != Ter::TES_SUCCESS {
                return Ok(auth);
            }
            Ok(if iou_deep_frozen(view, destination, issue)? {
                Ter::TEC_FROZEN
            } else {
                Ter::TES_SUCCESS
            })
        }
        Asset::MPTIssue(issue) => {
            let destination = escrow.get_account_id(sf("sfDestination"));
            // The issuer has no MPToken holding to authorize or freeze.
            // rippled returns success before those checks when escrowed MPTs
            // are redeemed to their issuer.
            if destination == issue.issuer() {
                return Ok(Ter::TES_SUCCESS);
            }
            if view
                .read(protocol::mpt_issuance_keylet_from_mptid(issue.mpt_id()))
                .map_err(|_| view_error())?
                .is_none()
            {
                return Ok(Ter::TEC_OBJECT_NOT_FOUND);
            }
            let auth = ledger::mptoken_helpers::require_auth_mpt(view, &issue, &destination)
                .map_err(|_| view_error())?;
            if auth != Ter::TES_SUCCESS {
                return Ok(auth);
            }
            Ok(
                if ledger::mptoken_helpers::is_frozen_mpt(view, &destination, &issue)
                    .map_err(|_| view_error())?
                {
                    Ter::TEC_LOCKED
                } else {
                    Ter::TES_SUCCESS
                },
            )
        }
    }
}

fn preclaim_escrow_cancel<V: ReadView>(view: &V, tx: &STTx) -> Result<Ter, Ter> {
    if !view.rules().enabled(&protocol::feature_token_escrow()) {
        return Ok(Ter::TES_SUCCESS);
    }
    let Some(escrow) = read_escrow(
        view,
        tx.get_account_id(sf("sfOwner")),
        tx.get_field_u32(sf("sfOfferSequence")),
    )?
    else {
        return Ok(Ter::TEC_NO_TARGET);
    };
    let owner = escrow.get_account_id(sf("sfAccount"));
    match escrow.get_field_amount(sf("sfAmount")).asset() {
        Asset::Issue(issue) if issue.native() => Ok(Ter::TES_SUCCESS),
        Asset::Issue(issue) => {
            if issue.account == owner {
                return Ok(Ter::TEC_INTERNAL);
            }
            iou_auth(view, owner, issue)
        }
        Asset::MPTIssue(issue) => {
            if issue.issuer() == owner {
                return Ok(Ter::TEC_INTERNAL);
            }
            if view
                .read(protocol::mpt_issuance_keylet_from_mptid(issue.mpt_id()))
                .map_err(|_| view_error())?
                .is_none()
            {
                return Ok(Ter::TEC_OBJECT_NOT_FOUND);
            }
            ledger::mptoken_helpers::require_auth_mpt(view, &issue, &owner)
                .map_err(|_| view_error())
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{collections::BTreeMap, sync::Arc};

    use basics::base_uint::Uint256;
    use ledger::{Fees, LedgerHeader, ReadView, ReadViewTx, ViewError};
    use protocol::{
        AccountID, ApplyFlags, Currency, IOUAmount, Issue, Keylet, MPTAmount, MPTIssue, Rules,
        STAmount, STLedgerEntry, STTx, Ter, TxType, XRPAmount,
    };

    use super::run_read_view_preclaim;

    #[derive(Debug, Default)]
    struct MockView {
        entries: BTreeMap<Uint256, Arc<STLedgerEntry>>,
        header: LedgerHeader,
        rules: Rules,
        fail_read_key: Option<Uint256>,
        fail_exists_key: Option<Uint256>,
        fail_succ: bool,
    }

    impl ReadView for MockView {
        fn open(&self) -> bool {
            false
        }
        fn header(&self) -> LedgerHeader {
            self.header.clone()
        }
        fn fees(&self) -> Fees {
            Fees {
                base: 10,
                reserve: 200,
                increment: 50,
            }
        }
        fn rules(&self) -> Rules {
            self.rules.clone()
        }
        fn exists(&self, keylet: Keylet) -> Result<bool, ViewError> {
            if self.fail_exists_key == Some(keylet.key) {
                return Err(ViewError::Conversion("injected exists failure".into()));
            }
            Ok(self.entries.contains_key(&keylet.key))
        }
        fn succ(
            &self,
            _key: Uint256,
            _last: Option<Uint256>,
        ) -> Result<Option<Uint256>, ViewError> {
            if self.fail_succ {
                return Err(ViewError::Conversion("injected successor failure".into()));
            }
            Ok(None)
        }
        fn read(&self, keylet: Keylet) -> Result<Option<Arc<STLedgerEntry>>, ViewError> {
            if self.fail_read_key == Some(keylet.key) {
                return Err(ViewError::Conversion("injected read failure".into()));
            }
            Ok(self.entries.get(&keylet.key).cloned())
        }
        fn sles(&self) -> Result<Vec<Arc<STLedgerEntry>>, ViewError> {
            Ok(self.entries.values().cloned().collect())
        }
        fn tx_exists(&self, _key: Uint256) -> Result<bool, ViewError> {
            Ok(false)
        }
        fn tx_read(&self, _key: Uint256) -> Result<Option<ReadViewTx>, ViewError> {
            Ok(None)
        }
        fn txs(&self) -> Result<Vec<ReadViewTx>, ViewError> {
            Ok(Vec::new())
        }
    }

    fn account(fill: u8) -> AccountID {
        AccountID::from_array([fill; 20])
    }
    fn sf(name: &str) -> &'static protocol::SField {
        protocol::get_field_by_symbol(name)
    }
    fn payment(account: AccountID, destination: AccountID, amount: STAmount) -> STTx {
        STTx::new(TxType::PAYMENT, move |tx| {
            tx.set_account_id(sf("sfAccount"), account);
            tx.set_account_id(sf("sfDestination"), destination);
            tx.set_field_amount(sf("sfAmount"), amount);
        })
    }

    fn account_delete(account: AccountID, destination: AccountID) -> STTx {
        STTx::new(TxType::ACCOUNT_DELETE, move |tx| {
            tx.set_account_id(sf("sfAccount"), account);
            tx.set_account_id(sf("sfDestination"), destination);
        })
    }

    fn account_set(account: AccountID, set_flag: u32, tx_flags: u32) -> STTx {
        STTx::new(TxType::ACCOUNT_SET, move |tx| {
            tx.set_account_id(sf("sfAccount"), account);
            tx.set_field_u32(sf("sfSetFlag"), set_flag);
            tx.set_field_u32(sf("sfFlags"), tx_flags);
        })
    }

    fn payment_channel_create(account: AccountID, destination: AccountID, amount: i64) -> STTx {
        STTx::new(TxType::PAYCHAN_CREATE, move |tx| {
            tx.set_account_id(sf("sfAccount"), account);
            tx.set_account_id(sf("sfDestination"), destination);
            tx.set_field_amount(
                sf("sfAmount"),
                STAmount::from_xrp_amount(XRPAmount::from_drops(amount)),
            );
        })
    }

    fn deposit_preauth_authorize(account: AccountID, authorize: AccountID) -> STTx {
        STTx::new(TxType::DEPOSIT_PREAUTH, move |tx| {
            tx.set_account_id(sf("sfAccount"), account);
            tx.set_account_id(sf("sfAuthorize"), authorize);
        })
    }

    fn escrow_create_iou(account: AccountID, destination: AccountID, issuer: AccountID) -> STTx {
        STTx::new(TxType::ESCROW_CREATE, move |tx| {
            tx.set_account_id(sf("sfAccount"), account);
            tx.set_account_id(sf("sfDestination"), destination);
            tx.set_field_amount(
                sf("sfAmount"),
                STAmount::from_iou_amount(
                    sf("sfAmount"),
                    IOUAmount::from_parts(1, 0).expect("valid IOU"),
                    Issue::new(Currency::from_array([0x5A; 20]), issuer),
                ),
            );
        })
    }

    fn delegate_set(account: AccountID, authorize: AccountID) -> STTx {
        STTx::new(TxType::DELEGATE_SET, move |tx| {
            tx.set_account_id(sf("sfAccount"), account);
            tx.set_account_id(sf("sfAuthorize"), authorize);
        })
    }

    fn check_cash(account: AccountID, check_id: Uint256) -> STTx {
        STTx::new(TxType::CHECK_CASH, move |tx| {
            tx.set_account_id(sf("sfAccount"), account);
            tx.set_field_h256(sf("sfCheckID"), check_id);
            tx.set_field_amount(
                sf("sfAmount"),
                STAmount::from_xrp_amount(XRPAmount::from_drops(1)),
            );
        })
    }

    fn check_create_iou(account: AccountID, destination: AccountID, issuer: AccountID) -> STTx {
        STTx::new(TxType::CHECK_CREATE, move |tx| {
            tx.set_account_id(sf("sfAccount"), account);
            tx.set_account_id(sf("sfDestination"), destination);
            tx.set_field_amount(
                sf("sfSendMax"),
                STAmount::from_iou_amount(
                    sf("sfSendMax"),
                    IOUAmount::from_parts(1, 0).expect("valid IOU"),
                    Issue::new(Currency::from_array([0x6A; 20]), issuer),
                ),
            );
        })
    }

    fn account_root(account: AccountID, flags: u32) -> (Uint256, Arc<STLedgerEntry>) {
        let keylet =
            protocol::account_keylet(basics::base_uint::Uint160::from_void(account.data()));
        let mut root = STLedgerEntry::new(keylet);
        root.set_account_id(sf("sfAccount"), account);
        root.set_field_u32(sf("sfFlags"), flags);
        root.set_field_u32(sf("sfSequence"), 1);
        (keylet.key, Arc::new(root))
    }

    #[test]
    fn account_delete_missing_destination_precedes_source_storage_failure() {
        let source = account(0x11);
        let destination = account(0x12);
        let source_key =
            protocol::account_keylet(basics::base_uint::Uint160::from_void(source.data()));
        let view = MockView {
            fail_read_key: Some(source_key.key),
            ..MockView::default()
        };

        assert_eq!(
            run_read_view_preclaim(
                &view,
                &account_delete(source, destination),
                TxType::ACCOUNT_DELETE,
                ApplyFlags::NONE,
            ),
            Some(Ter::TEC_NO_DST)
        );
    }

    #[test]
    fn payment_missing_destination_precedes_permissioned_domain_storage_failure() {
        let source = account(0x13);
        let destination = account(0x14);
        let domain = Uint256::from_array([0xD0; 32]);
        let issue = protocol::Issue::new(protocol::currency_from_string("USD"), source);
        let tx = STTx::new(TxType::PAYMENT, |tx| {
            tx.set_account_id(sf("sfAccount"), source);
            tx.set_account_id(sf("sfDestination"), destination);
            tx.set_field_amount(
                sf("sfAmount"),
                STAmount::from_iou_amount(
                    sf("sfAmount"),
                    protocol::IOUAmount::from_parts(1, 0).expect("valid IOU"),
                    issue,
                ),
            );
            tx.set_field_h256(sf("sfDomainID"), domain);
        });
        let view = MockView {
            fail_read_key: Some(domain),
            ..MockView::default()
        };

        assert_eq!(
            run_read_view_preclaim(&view, &tx, TxType::PAYMENT, ApplyFlags::NONE),
            Some(Ter::TEC_NO_DST)
        );
    }

    #[test]
    fn account_delete_destination_tag_precedes_deposit_preauth_storage_failure() {
        let source = account(0x21);
        let destination = account(0x22);
        let (destination_key, destination_root) = account_root(
            destination,
            protocol::lsfRequireDestTag | protocol::lsfDepositAuth,
        );
        let preauth = protocol::deposit_preauth_keylet(
            basics::base_uint::Uint160::from_void(destination.data()),
            basics::base_uint::Uint160::from_void(source.data()),
        );
        let mut view = MockView {
            fail_exists_key: Some(preauth.key),
            ..MockView::default()
        };
        view.entries.insert(destination_key, destination_root);

        assert_eq!(
            run_read_view_preclaim(
                &view,
                &account_delete(source, destination),
                TxType::ACCOUNT_DELETE,
                ApplyFlags::NONE,
            ),
            Some(Ter::TEC_DST_TAG_NEEDED)
        );
    }

    #[test]
    fn account_delete_deposit_preauth_storage_failure_is_hard() {
        let source = account(0x23);
        let destination = account(0x24);
        let (destination_key, destination_root) =
            account_root(destination, protocol::lsfDepositAuth);
        let preauth = protocol::deposit_preauth_keylet(
            basics::base_uint::Uint160::from_void(destination.data()),
            basics::base_uint::Uint160::from_void(source.data()),
        );
        let mut view = MockView {
            fail_exists_key: Some(preauth.key),
            ..MockView::default()
        };
        view.entries.insert(destination_key, destination_root);

        assert_eq!(
            run_read_view_preclaim(
                &view,
                &account_delete(source, destination),
                TxType::ACCOUNT_DELETE,
                ApplyFlags::NONE,
            ),
            Some(Ter::TEF_BAD_LEDGER)
        );
    }

    #[test]
    fn account_delete_minted_obligation_precedes_nft_page_storage_failure() {
        let source = account(0x25);
        let destination = account(0x26);
        let (source_key, source_root) = account_root(source, 0);
        let (destination_key, destination_root) = account_root(destination, 0);
        let mut source_root = (*source_root).clone();
        source_root.set_field_u32(sf("sfMintedNFTokens"), 1);
        source_root.set_field_u32(sf("sfBurnedNFTokens"), 0);
        let mut view = MockView {
            fail_succ: true,
            ..MockView::default()
        };
        view.entries.insert(source_key, Arc::new(source_root));
        view.entries.insert(destination_key, destination_root);

        assert_eq!(
            run_read_view_preclaim(
                &view,
                &account_delete(source, destination),
                TxType::ACCOUNT_DELETE,
                ApplyFlags::NONE,
            ),
            Some(Ter::TEC_HAS_OBLIGATIONS)
        );
    }

    #[test]
    fn unrelated_account_set_does_not_read_owner_directory() {
        let account = account(0x27);
        let (account_key, account_root) = account_root(account, 0);
        let owner_dir =
            protocol::owner_dir_keylet(basics::base_uint::Uint160::from_void(account.data()));
        let mut view = MockView {
            fail_read_key: Some(owner_dir.key),
            ..MockView::default()
        };
        view.entries.insert(account_key, account_root);

        assert_eq!(
            run_read_view_preclaim(
                &view,
                &account_set(account, tx::ASF_REQUIRE_DEST, 0),
                TxType::ACCOUNT_SET,
                ApplyFlags::NONE,
            ),
            Some(Ter::TES_SUCCESS)
        );
    }

    #[test]
    fn account_set_clawback_no_freeze_precedes_owner_directory_failure() {
        let account = account(0x28);
        let (account_key, account_root) = account_root(account, tx::LSF_NO_FREEZE);
        let owner_dir =
            protocol::owner_dir_keylet(basics::base_uint::Uint160::from_void(account.data()));
        let mut view = MockView {
            fail_read_key: Some(owner_dir.key),
            ..MockView::default()
        };
        view.entries.insert(account_key, account_root);

        assert_eq!(
            run_read_view_preclaim(
                &view,
                &account_set(account, tx::ASF_ALLOW_TRUST_LINE_CLAWBACK, 0),
                TxType::ACCOUNT_SET,
                ApplyFlags::NONE,
            ),
            Some(Ter::TEC_NO_PERMISSION)
        );
    }

    #[test]
    fn account_set_require_auth_owner_directory_failure_is_hard() {
        let account = account(0x29);
        let (account_key, account_root) = account_root(account, 0);
        let owner_dir =
            protocol::owner_dir_keylet(basics::base_uint::Uint160::from_void(account.data()));
        let mut view = MockView {
            fail_read_key: Some(owner_dir.key),
            ..MockView::default()
        };
        view.entries.insert(account_key, account_root);

        assert_eq!(
            run_read_view_preclaim(
                &view,
                &account_set(account, tx::ASF_REQUIRE_AUTH, 0),
                TxType::ACCOUNT_SET,
                ApplyFlags::NONE,
            ),
            Some(Ter::TEF_BAD_LEDGER)
        );
    }

    #[test]
    fn paychan_source_reserve_failure_precedes_destination_storage_failure() {
        let source = account(0x2A);
        let destination = account(0x2B);
        let (source_key, mut source_root) = account_root(source, 0);
        Arc::make_mut(&mut source_root).set_field_amount(
            sf("sfBalance"),
            STAmount::from_xrp_amount(XRPAmount::from_drops(1)),
        );
        let destination_key =
            protocol::account_keylet(basics::base_uint::Uint160::from_void(destination.data()));
        let mut view = MockView {
            fail_read_key: Some(destination_key.key),
            ..MockView::default()
        };
        view.entries.insert(source_key, source_root);

        assert_eq!(
            run_read_view_preclaim(
                &view,
                &payment_channel_create(source, destination, 1),
                TxType::PAYCHAN_CREATE,
                ApplyFlags::NONE,
            ),
            Some(Ter::TEC_INSUFFICIENT_RESERVE)
        );
    }

    #[test]
    fn deposit_preauth_missing_target_precedes_duplicate_lookup_failure() {
        let source = account(0x2C);
        let target = account(0x2D);
        let preauth = protocol::deposit_preauth_keylet(
            basics::base_uint::Uint160::from_void(source.data()),
            basics::base_uint::Uint160::from_void(target.data()),
        );
        let view = MockView {
            fail_exists_key: Some(preauth.key),
            ..MockView::default()
        };

        assert_eq!(
            run_read_view_preclaim(
                &view,
                &deposit_preauth_authorize(source, target),
                TxType::DEPOSIT_PREAUTH,
                ApplyFlags::NONE,
            ),
            Some(Ter::TEC_NO_TARGET)
        );
    }

    #[test]
    fn escrow_create_missing_destination_precedes_issuer_storage_failure() {
        let source = account(0x2E);
        let destination = account(0x2F);
        let issuer = account(0x30);
        let issuer_key =
            protocol::account_keylet(basics::base_uint::Uint160::from_void(issuer.data()));
        let view = MockView {
            rules: Rules::new([protocol::feature_token_escrow()]),
            fail_read_key: Some(issuer_key.key),
            ..MockView::default()
        };

        assert_eq!(
            run_read_view_preclaim(
                &view,
                &escrow_create_iou(source, destination, issuer),
                TxType::ESCROW_CREATE,
                ApplyFlags::NONE,
            ),
            Some(Ter::TEC_NO_DST)
        );
    }

    #[test]
    fn delegate_set_missing_source_precedes_authorize_storage_failure() {
        let source = account(0x31);
        let authorize = account(0x32);
        let authorize_key =
            protocol::account_keylet(basics::base_uint::Uint160::from_void(authorize.data()));
        let view = MockView {
            fail_read_key: Some(authorize_key.key),
            ..MockView::default()
        };

        assert_eq!(
            run_read_view_preclaim(
                &view,
                &delegate_set(source, authorize),
                TxType::DELEGATE_SET,
                ApplyFlags::NONE,
            ),
            Some(Ter::TER_NO_ACCOUNT)
        );
    }

    #[test]
    fn check_cash_wrong_actor_precedes_source_storage_failure() {
        let source = account(0x33);
        let destination = account(0x34);
        let wrong_actor = account(0x35);
        let check_id = Uint256::from_array([0x36; 32]);
        let check_keylet = protocol::check_keylet_from_key(check_id);
        let mut check = STLedgerEntry::new(check_keylet);
        check.set_account_id(sf("sfAccount"), source);
        check.set_account_id(sf("sfDestination"), destination);
        check.set_field_amount(
            sf("sfSendMax"),
            STAmount::from_xrp_amount(XRPAmount::from_drops(1)),
        );
        let source_key =
            protocol::account_keylet(basics::base_uint::Uint160::from_void(source.data()));
        let mut view = MockView {
            fail_read_key: Some(source_key.key),
            ..MockView::default()
        };
        view.entries.insert(check_keylet.key, Arc::new(check));

        assert_eq!(
            run_read_view_preclaim(
                &view,
                &check_cash(wrong_actor, check_id),
                TxType::CHECK_CASH,
                ApplyFlags::NONE,
            ),
            Some(Ter::TEC_NO_PERMISSION)
        );
    }

    #[test]
    fn check_create_missing_destination_precedes_issuer_storage_failure() {
        let source = account(0x37);
        let destination = account(0x38);
        let issuer = account(0x39);
        let issuer_key =
            protocol::account_keylet(basics::base_uint::Uint160::from_void(issuer.data()));
        let view = MockView {
            fail_read_key: Some(issuer_key.key),
            ..MockView::default()
        };

        assert_eq!(
            run_read_view_preclaim(
                &view,
                &check_create_iou(source, destination, issuer),
                TxType::CHECK_CREATE,
                ApplyFlags::NONE,
            ),
            Some(Ter::TEC_NO_DST)
        );
    }

    #[test]
    fn check_cash_mpt_requires_issuer_account_root_not_only_issuance() {
        let source = account(0x3A);
        let destination = account(0x3B);
        let issuer = account(0x3C);
        let mpt_id = protocol::make_mpt_id(1, issuer);
        let issue = MPTIssue::new(mpt_id);
        let check_id = Uint256::from_array([0x3D; 32]);
        let check_keylet = protocol::check_keylet_from_key(check_id);
        let mut check = STLedgerEntry::new(check_keylet);
        check.set_account_id(sf("sfAccount"), source);
        check.set_account_id(sf("sfDestination"), destination);
        check.set_field_amount(
            sf("sfSendMax"),
            STAmount::from_mpt_amount(sf("sfSendMax"), MPTAmount::from_value(10), issue),
        );
        let issuance_keylet = protocol::mpt_issuance_keylet_from_mptid(mpt_id);
        let mut issuance = STLedgerEntry::new(issuance_keylet);
        issuance.set_account_id(sf("sfIssuer"), issuer);
        let token_keylet = protocol::mptoken_keylet_from_mptid(
            mpt_id,
            basics::base_uint::Uint160::from_void(source.data()),
        );
        let mut token = STLedgerEntry::new(token_keylet);
        token.set_field_u64(sf("sfMPTAmount"), 10);
        let (source_key, source_root) = account_root(source, 0);
        let (destination_key, destination_root) = account_root(destination, 0);
        let mut view = MockView::default();
        view.entries.insert(check_keylet.key, Arc::new(check));
        view.entries.insert(issuance_keylet.key, Arc::new(issuance));
        view.entries.insert(token_keylet.key, Arc::new(token));
        view.entries.insert(source_key, source_root);
        view.entries.insert(destination_key, destination_root);

        let tx = STTx::new(TxType::CHECK_CASH, |tx| {
            tx.set_account_id(sf("sfAccount"), destination);
            tx.set_field_h256(sf("sfCheckID"), check_id);
            tx.set_field_amount(
                sf("sfAmount"),
                STAmount::from_mpt_amount(sf("sfAmount"), MPTAmount::from_value(1), issue),
            );
        });
        assert_eq!(
            run_read_view_preclaim(&view, &tx, TxType::CHECK_CASH, ApplyFlags::NONE),
            Some(Ter::TEC_NO_ISSUER)
        );
    }

    #[test]
    fn sponsored_xrp_check_preclaim_does_not_release_source_reserve() {
        let source = account(0x31);
        let destination = account(0x32);
        let sponsor = account(0x33);
        let check_id = Uint256::from_array([0x44; 32]);

        let mut view = MockView::default();
        for (id, balance, owner_count) in [(source, 250, 1), (destination, 1_000, 0)] {
            let keylet = protocol::account_keylet(basics::base_uint::Uint160::from_void(id.data()));
            let mut root = STLedgerEntry::new(keylet);
            root.set_account_id(sf("sfAccount"), id);
            root.set_field_amount(
                sf("sfBalance"),
                STAmount::from_xrp_amount(XRPAmount::from_drops(balance)),
            );
            root.set_field_u32(sf("sfOwnerCount"), owner_count);
            view.entries.insert(keylet.key, Arc::new(root));
        }

        let check_keylet = protocol::check_keylet_from_key(check_id);
        let mut check = STLedgerEntry::new(check_keylet);
        check.set_account_id(sf("sfAccount"), source);
        check.set_account_id(sf("sfDestination"), destination);
        check.set_account_id(sf("sfSponsor"), sponsor);
        check.set_field_amount(
            sf("sfSendMax"),
            STAmount::from_xrp_amount(XRPAmount::from_drops(50)),
        );
        view.entries.insert(check_keylet.key, Arc::new(check));

        let cash = STTx::new(TxType::CHECK_CASH, |tx| {
            tx.set_account_id(sf("sfAccount"), destination);
            tx.set_field_h256(sf("sfCheckID"), check_id);
            tx.set_field_amount(
                sf("sfAmount"),
                STAmount::from_xrp_amount(XRPAmount::from_drops(50)),
            );
        });

        assert_eq!(
            run_read_view_preclaim(&view, &cash, TxType::CHECK_CASH, ApplyFlags::NONE),
            Some(Ter::TEC_PATH_PARTIAL)
        );
    }

    #[test]
    fn payment_sponsored_account_preclaim_allows_one_drop_only_for_new_destination() {
        let source = account(0x41);
        let destination = account(0x42);
        let tx = STTx::new(TxType::PAYMENT, |object| {
            object.set_account_id(sf("sfAccount"), source);
            object.set_account_id(sf("sfDestination"), destination);
            object.set_field_amount(
                sf("sfAmount"),
                STAmount::from_xrp_amount(XRPAmount::from_drops(1)),
            );
            object.set_field_u32(sf("sfFlags"), protocol::tfSponsorCreatedAccount);
        });
        let mut view = MockView {
            rules: Rules::new([protocol::feature_id("Sponsor")]),
            ..MockView::default()
        };

        assert_eq!(
            run_read_view_preclaim(&view, &tx, TxType::PAYMENT, ApplyFlags::NONE),
            Some(Ter::TES_SUCCESS)
        );

        let keylet =
            protocol::account_keylet(basics::base_uint::Uint160::from_void(destination.data()));
        let mut destination_root = STLedgerEntry::new(keylet);
        destination_root.set_account_id(sf("sfAccount"), destination);
        destination_root.set_field_amount(
            sf("sfBalance"),
            STAmount::from_xrp_amount(XRPAmount::from_drops(200)),
        );
        view.entries.insert(keylet.key, Arc::new(destination_root));
        assert_eq!(
            run_read_view_preclaim(&view, &tx, TxType::PAYMENT, ApplyFlags::NONE),
            Some(Ter::TEC_NO_SPONSOR_PERMISSION)
        );
    }

    #[test]
    fn typed_dispatcher_routes_requested_families_without_apply_fallbacks() {
        let view = MockView::default();
        let source = account(1);
        let destination = account(2);
        let cases = [
            (
                TxType::ACCOUNT_SET,
                STTx::new(TxType::ACCOUNT_SET, move |tx| {
                    tx.set_account_id(sf("sfAccount"), source)
                }),
                Ter::TER_NO_ACCOUNT,
            ),
            (
                TxType::ACCOUNT_DELETE,
                STTx::new(TxType::ACCOUNT_DELETE, move |tx| {
                    tx.set_account_id(sf("sfAccount"), source);
                    tx.set_account_id(sf("sfDestination"), destination);
                }),
                Ter::TEC_NO_DST,
            ),
            (
                TxType::DELEGATE_SET,
                STTx::new(TxType::DELEGATE_SET, move |tx| {
                    tx.set_account_id(sf("sfAccount"), source);
                    tx.set_account_id(sf("sfAuthorize"), destination);
                }),
                Ter::TER_NO_ACCOUNT,
            ),
            (
                TxType::REGULAR_KEY_SET,
                STTx::new(TxType::REGULAR_KEY_SET, move |tx| {
                    tx.set_account_id(sf("sfAccount"), source);
                }),
                Ter::TES_SUCCESS,
            ),
            (
                TxType::SIGNER_LIST_SET,
                STTx::new(TxType::SIGNER_LIST_SET, move |tx| {
                    tx.set_account_id(sf("sfAccount"), source);
                }),
                Ter::TES_SUCCESS,
            ),
            (
                TxType::DEPOSIT_PREAUTH,
                STTx::new(TxType::DEPOSIT_PREAUTH, move |tx| {
                    tx.set_account_id(sf("sfAccount"), source);
                    tx.set_account_id(sf("sfAuthorize"), destination);
                }),
                Ter::TEC_NO_TARGET,
            ),
            (
                TxType::PAYMENT,
                payment(
                    source,
                    destination,
                    STAmount::from_xrp_amount(XRPAmount::from_drops(1)),
                ),
                Ter::TEC_NO_DST_INSUF_XRP,
            ),
            (
                TxType::PAYCHAN_CREATE,
                STTx::new(TxType::PAYCHAN_CREATE, move |tx| {
                    tx.set_account_id(sf("sfAccount"), source);
                    tx.set_account_id(sf("sfDestination"), destination);
                    tx.set_field_amount(
                        sf("sfAmount"),
                        STAmount::from_xrp_amount(XRPAmount::from_drops(1)),
                    );
                }),
                Ter::TER_NO_ACCOUNT,
            ),
            (
                TxType::PAYCHAN_FUND,
                STTx::new(TxType::PAYCHAN_FUND, move |tx| {
                    tx.set_account_id(sf("sfAccount"), source);
                }),
                Ter::TES_SUCCESS,
            ),
            (
                TxType::PAYCHAN_CLAIM,
                STTx::new(TxType::PAYCHAN_CLAIM, move |tx| {
                    tx.set_account_id(sf("sfAccount"), source);
                }),
                Ter::TES_SUCCESS,
            ),
            (
                TxType::CHECK_CREATE,
                STTx::new(TxType::CHECK_CREATE, move |tx| {
                    tx.set_account_id(sf("sfAccount"), source);
                    tx.set_account_id(sf("sfDestination"), destination);
                    tx.set_field_amount(
                        sf("sfSendMax"),
                        STAmount::from_xrp_amount(XRPAmount::from_drops(1)),
                    );
                }),
                Ter::TEC_NO_DST,
            ),
            (
                TxType::CHECK_CASH,
                STTx::new(TxType::CHECK_CASH, move |tx| {
                    tx.set_account_id(sf("sfAccount"), destination);
                    tx.set_field_h256(sf("sfCheckID"), Uint256::from_array([3; 32]));
                    tx.set_field_amount(
                        sf("sfAmount"),
                        STAmount::from_xrp_amount(XRPAmount::from_drops(1)),
                    );
                }),
                Ter::TEC_NO_ENTRY,
            ),
            (
                TxType::CHECK_CANCEL,
                STTx::new(TxType::CHECK_CANCEL, move |tx| {
                    tx.set_account_id(sf("sfAccount"), destination);
                    tx.set_field_h256(sf("sfCheckID"), Uint256::from_array([4; 32]));
                }),
                Ter::TEC_NO_ENTRY,
            ),
            (
                TxType::ESCROW_CREATE,
                STTx::new(TxType::ESCROW_CREATE, move |tx| {
                    tx.set_account_id(sf("sfAccount"), source);
                    tx.set_account_id(sf("sfDestination"), destination);
                    tx.set_field_amount(
                        sf("sfAmount"),
                        STAmount::from_xrp_amount(XRPAmount::from_drops(1)),
                    );
                }),
                Ter::TEC_NO_DST,
            ),
            (
                TxType::ESCROW_FINISH,
                STTx::new(TxType::ESCROW_FINISH, move |tx| {
                    tx.set_account_id(sf("sfAccount"), source);
                    tx.set_account_id(sf("sfOwner"), source);
                    tx.set_field_u32(sf("sfOfferSequence"), 1);
                }),
                Ter::TES_SUCCESS,
            ),
            (
                TxType::ESCROW_CANCEL,
                STTx::new(TxType::ESCROW_CANCEL, move |tx| {
                    tx.set_account_id(sf("sfAccount"), source);
                    tx.set_account_id(sf("sfOwner"), source);
                    tx.set_field_u32(sf("sfOfferSequence"), 1);
                }),
                Ter::TES_SUCCESS,
            ),
        ];
        for (txn_type, tx, expected) in cases {
            assert_eq!(
                run_read_view_preclaim(&view, &tx, txn_type, ApplyFlags::NONE),
                Some(expected),
                "{txn_type:?}"
            );
        }
        assert_eq!(
            run_read_view_preclaim(
                &view,
                &STTx::new(TxType::OFFER_CREATE, |_| {}),
                TxType::OFFER_CREATE,
                ApplyFlags::NONE
            ),
            None
        );
    }
}
