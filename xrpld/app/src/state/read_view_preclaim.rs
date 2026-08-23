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
    get_field_by_symbol, lsfAllowTrustLineLocking, lsfDisallowIncomingCheck,
    lsfDisallowIncomingPayChan, lsfGlobalFreeze, lsfHighAuth, lsfHighDeepFreeze, lsfHighFreeze,
    lsfLowAuth, lsfLowDeepFreeze, lsfLowFreeze, lsfRequireAuth, lsfRequireDestTag,
};
use tx::{
    AccountDeleteDirectoryEntryDisposition, AccountDeletePreclaimFrontFacts,
    AccountDeletePreclaimNftAndSequenceFacts, AccountDeletePreclaimScanState,
    AccountSetPreclaimFacts, CheckCancelPreclaimFacts, CheckCashPreclaimFacts,
    CheckCreatePreclaimFacts, DelegateSetPreclaimFacts, DepositPreauthCredentialPreclaimFact,
    DepositPreauthPreclaimFacts, EscrowCancelIssuePreclaimFacts, EscrowCancelMptPreclaimFacts,
    EscrowCancelPreclaimFacts, EscrowCreateAmountKind, EscrowCreateIssuePreclaimFacts,
    EscrowCreateMptPreclaimFacts, EscrowCreatePreclaimFacts, PaymentChannelCreatePreclaimFacts,
    PaymentPreclaimFacts, run_account_delete_preclaim_directory_scan,
    run_account_delete_preclaim_front, run_account_delete_preclaim_nft_and_sequence,
    run_account_set_preclaim, run_check_cancel_preclaim, run_check_cash_preclaim,
    run_check_create_preclaim, run_delegate_set_preclaim, run_deposit_preauth_preclaim,
    run_escrow_cancel_issue_preclaim, run_escrow_cancel_mpt_preclaim, run_escrow_cancel_preclaim,
    run_escrow_create_issue_preclaim, run_escrow_create_mpt_preclaim, run_escrow_create_preclaim,
    run_payment_channel_claim_preclaim, run_payment_channel_create_preclaim,
    run_payment_preclaim_with_facts,
};

const LSF_PSEUDO_ACCOUNT_FIELDS: [&str; 3] = ["sfAMMID", "sfVaultID", "sfLoanBrokerID"];

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
    LSF_PSEUDO_ACCOUNT_FIELDS
        .iter()
        .any(|field| sle.is_field_present(sf(field)))
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
            let balance = root.get_field_amount(sf("sfBalance")).xrp().drops();
            let reserve = view
                .fees()
                .account_reserve(root.get_field_u32(sf("sfOwnerCount")) as usize)
                as i64;
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
    let owner_dir_empty = directory_entries(view, account)?.is_empty();
    Ok(run_account_set_preclaim(AccountSetPreclaimFacts {
        tx_flags: tx.get_flags(),
        set_flag: tx.get_field_u32(sf("sfSetFlag")),
        apply_flags,
        account_exists: account_root.is_some(),
        account_flags: account_root
            .as_ref()
            .map_or(0, |sle| sle.get_field_u32(sf("sfFlags"))),
        owner_dir_empty,
        feature_clawback_enabled: view.rules().enabled(&protocol::feature_clawback()),
    }))
}

fn preclaim_account_delete<V: ReadView>(view: &V, tx: &STTx) -> Result<Ter, Ter> {
    let account = tx.get_account_id(sf("sfAccount"));
    let destination = tx.get_account_id(sf("sfDestination"));
    let destination_sle = read_account(view, destination)?;
    let source_sle = read_account(view, account)?;
    let credentials_present = tx.is_field_present(sf("sfCredentialIDs"));
    let credentials =
        ledger::credential_helpers::valid(view, tx, &account).map_err(|_| view_error())?;
    let preauth = view
        .exists(protocol::deposit_preauth_keylet(
            Uint160::from_void(destination.data()),
            Uint160::from_void(account.data()),
        ))
        .map_err(|_| view_error())?;
    let front = run_account_delete_preclaim_front(
        AccountDeletePreclaimFrontFacts {
            destination_exists: destination_sle.is_some(),
            destination_flags: destination_sle
                .as_ref()
                .map_or(0, |sle| sle.get_field_u32(sf("sfFlags"))),
            destination_tag_present: tx.is_field_present(sf("sfDestinationTag")),
            credential_ids_present: credentials_present,
            source_account_exists: source_sle.is_some(),
        },
        || credentials,
        || preauth,
    );
    if front != Ter::TES_SUCCESS {
        return Ok(front);
    }
    let source = source_sle.expect("front verified source account");
    let nft_min = protocol::nft_page_min_keylet(Uint160::from_void(account.data()));
    let nft_max = protocol::nft_page_max_keylet(Uint160::from_void(account.data()));
    let owned_nft_page_present = view
        .succ(nft_min.key, Some(nft_max.key.next()))
        .map_err(|_| view_error())?
        .is_some();
    match run_account_delete_preclaim_nft_and_sequence(AccountDeletePreclaimNftAndSequenceFacts {
        minted_nftokens: source.get_field_u32(sf("sfMintedNFTokens")),
        burned_nftokens: source.get_field_u32(sf("sfBurnedNFTokens")),
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
    Ok(run_delegate_set_preclaim(DelegateSetPreclaimFacts {
        account_exists: read_account(view, account)?.is_some(),
        authorize_exists: read_account(view, authorize)?.is_some(),
        permissions_empty: tx.get_field_array(sf("sfPermissions")).is_empty(),
        delegate_exists: view
            .exists(protocol::delegate_keylet(
                Uint160::from_void(account.data()),
                Uint160::from_void(authorize.data()),
            ))
            .map_err(|_| view_error())?,
    }))
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
    let authorize = tx
        .is_field_present(sf("sfAuthorize"))
        .then(|| tx.get_account_id(sf("sfAuthorize")));
    let unauthorize = tx
        .is_field_present(sf("sfUnauthorize"))
        .then(|| tx.get_account_id(sf("sfUnauthorize")));
    let authorize_credentials_present = tx.is_field_present(sf("sfAuthorizeCredentials"));
    let unauthorize_credentials_present = tx.is_field_present(sf("sfUnauthorizeCredentials"));
    let account_key = Uint160::from_void(account.data());

    let authorize_preauth_exists = if let Some(target) = authorize {
        view.exists(protocol::deposit_preauth_keylet(
            account_key,
            Uint160::from_void(target.data()),
        ))
        .map_err(|_| view_error())?
    } else {
        false
    };
    let unauthorize_preauth_exists = if let Some(target) = unauthorize {
        view.exists(protocol::deposit_preauth_keylet(
            account_key,
            Uint160::from_void(target.data()),
        ))
        .map_err(|_| view_error())?
    } else {
        false
    };

    let credential_field = if authorize_credentials_present {
        sf("sfAuthorizeCredentials")
    } else {
        sf("sfUnauthorizeCredentials")
    };
    let raw_credentials = (authorize_credentials_present || unauthorize_credentials_present)
        .then(|| tx.get_field_array(credential_field));
    let credentials: Vec<DepositPreauthCredentialPreclaimFact<AccountID, Vec<u8>>> =
        if let Some(items) = raw_credentials.as_ref() {
            items
                .iter()
                .map(|item| {
                    let issuer = item.get_account_id(sf("sfIssuer"));
                    Ok(DepositPreauthCredentialPreclaimFact {
                        issuer,
                        credential_type: item.get_field_vl(sf("sfCredentialType")),
                        issuer_exists: view
                            .exists(account_keylet(issuer))
                            .map_err(|_| view_error())?,
                    })
                })
                .collect::<Result<_, Ter>>()?
        } else {
            Vec::new()
        };
    let mut credential_hashes = credentials
        .iter()
        .map(|credential| {
            protocol::sha512_half_slices(&[credential.issuer.data(), &credential.credential_type])
        })
        .collect::<Vec<_>>();
    credential_hashes.sort();
    let credential_preauth_exists = if raw_credentials.is_some() {
        view.exists(protocol::deposit_preauth_credentials_keylet(
            account_key,
            &credential_hashes,
        ))
        .map_err(|_| view_error())?
    } else {
        false
    };

    Ok(run_deposit_preauth_preclaim(DepositPreauthPreclaimFacts {
        authorize,
        unauthorize,
        authorize_target_exists: authorize
            .map(|target| {
                view.exists(account_keylet(target))
                    .map_err(|_| view_error())
            })
            .transpose()?
            .unwrap_or(false),
        authorize_preauth_exists,
        unauthorize_preauth_exists,
        authorize_credentials_present,
        authorize_credentials: credentials,
        authorize_credentials_preauth_exists: credential_preauth_exists,
        unauthorize_credentials_present,
        unauthorize_credentials_preauth_exists: credential_preauth_exists,
    }))
}

fn preclaim_payment<V: ReadView>(view: &V, tx: &STTx, apply_flags: ApplyFlags) -> Result<Ter, Ter> {
    let account = tx.get_account_id(sf("sfAccount"));
    let destination = tx.get_account_id(sf("sfDestination"));
    let amount = tx.get_field_amount(sf("sfAmount"));
    let destination_sle = read_account(view, destination)?;
    let paths = tx
        .is_field_present(sf("sfPaths"))
        .then(|| tx.get_field_path_set(sf("sfPaths")));
    let credentials =
        ledger::credential_helpers::valid(view, tx, &account).map_err(|_| view_error())?;
    let domain_id = tx
        .is_field_present(sf("sfDomainID"))
        .then(|| tx.get_field_h256(sf("sfDomainID")));
    let (source_in_domain, destination_in_domain) = if let Some(domain) = domain_id {
        (
            ledger::permissioned_dex_helpers::account_in_domain(view, &account, &domain)
                .map_err(|_| view_error())?,
            ledger::permissioned_dex_helpers::account_in_domain(view, &destination, &domain)
                .map_err(|_| view_error())?,
        )
    } else {
        (true, true)
    };
    Ok(run_payment_preclaim_with_facts(PaymentPreclaimFacts {
        tx_flags: tx.get_flags(),
        has_paths: paths.is_some(),
        send_max_present: tx.is_field_present(sf("sfSendMax")),
        dst_amount_native: amount.native(),
        destination_exists: destination_sle.is_some(),
        view_open: view.open(),
        destination_requires_tag: destination_sle
            .as_ref()
            .is_some_and(|sle| account_flag(sle, lsfRequireDestTag)),
        destination_tag_present: tx.is_field_present(sf("sfDestinationTag")),
        destination_can_create_with_amount: amount.native()
            && amount.xrp().drops() >= view.fees().reserve as i64,
        path_count: paths.as_ref().map_or(0, |paths| paths.size()),
        path_has_too_long_segment: paths
            .as_ref()
            .is_some_and(|paths| paths.iter().any(|path| path.size() > tx::MAX_PATH_LENGTH)),
        credentials_valid_result: credentials,
        domain_id_present: domain_id.is_some(),
        source_in_domain,
        destination_in_domain,
        is_batch_inner: tx.get_flags() & protocol::INNER_BATCH_TRANSACTION_FLAG != 0
            || (apply_flags.bits() & ApplyFlags::BATCH.bits()) != 0,
        batch_v1_1_enabled: view.rules().enabled(&feature_batch_v1_1()),
    }))
}

fn preclaim_payment_channel_create<V: ReadView>(view: &V, tx: &STTx) -> Result<Ter, Ter> {
    let account = tx.get_account_id(sf("sfAccount"));
    let destination = tx.get_account_id(sf("sfDestination"));
    let amount = tx.get_field_amount(sf("sfAmount"));
    let source = read_account(view, account)?;
    let destination_sle = read_account(view, destination)?;
    let (covers_reserve, covers_amount) =
        if source.is_some() && view.rules().enabled(&protocol::feature_id("Sponsor")) {
            // rippled defers both reserve checks to doApply under Sponsor, where
            // the transaction reserve bearer and the source's post-lock reserve
            // can be evaluated independently.
            (true, true)
        } else {
            source
                .as_ref()
                .map(|source| {
                    let balance = source.get_field_amount(sf("sfBalance")).xrp().drops();
                    let reserve = view
                        .fees()
                        .account_reserve(source.get_field_u32(sf("sfOwnerCount")) as usize + 1)
                        as i64;
                    (
                        balance >= reserve,
                        balance >= reserve.saturating_add(amount.xrp().drops()),
                    )
                })
                .unwrap_or((false, false))
        };
    Ok(run_payment_channel_create_preclaim(
        PaymentChannelCreatePreclaimFacts {
            source_account_exists: source.is_some(),
            source_balance_covers_reserve: covers_reserve,
            source_balance_covers_reserve_plus_amount: covers_amount,
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
    let expired = has_expired(
        view,
        tx.is_field_present(sf("sfExpiration"))
            .then(|| tx.get_field_u32(sf("sfExpiration"))),
    );
    let result = match amount.asset() {
        Asset::Issue(issue) if issue.native() => {
            run_check_create_preclaim(CheckCreatePreclaimFacts {
                destination_exists: destination_sle.is_some(),
                destination_disallow_incoming_check: destination_sle
                    .as_ref()
                    .is_some_and(|sle| account_flag(sle, lsfDisallowIncomingCheck)),
                destination_is_pseudo_account: destination_sle
                    .as_ref()
                    .is_some_and(|sle| account_is_pseudo(sle)),
                destination_require_dest_tag: destination_sle
                    .as_ref()
                    .is_some_and(|sle| account_flag(sle, lsfRequireDestTag)),
                tx_has_destination_tag: tx.is_field_present(sf("sfDestinationTag")),
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
            let source_frozen = iou_frozen(view, source, issue)?;
            let destination_frozen = iou_frozen(view, destination, issue)?;
            run_check_create_preclaim(CheckCreatePreclaimFacts {
                destination_exists: destination_sle.is_some(),
                destination_disallow_incoming_check: destination_sle
                    .as_ref()
                    .is_some_and(|sle| account_flag(sle, lsfDisallowIncomingCheck)),
                destination_is_pseudo_account: destination_sle
                    .as_ref()
                    .is_some_and(|sle| account_is_pseudo(sle)),
                destination_require_dest_tag: destination_sle
                    .as_ref()
                    .is_some_and(|sle| account_flag(sle, lsfRequireDestTag)),
                tx_has_destination_tag: tx.is_field_present(sf("sfDestinationTag")),
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
                destination_exists: destination_sle.is_some(),
                destination_disallow_incoming_check: destination_sle
                    .as_ref()
                    .is_some_and(|sle| account_flag(sle, lsfDisallowIncomingCheck)),
                destination_is_pseudo_account: destination_sle
                    .as_ref()
                    .is_some_and(|sle| account_is_pseudo(sle)),
                destination_require_dest_tag: destination_sle
                    .as_ref()
                    .is_some_and(|sle| account_flag(sle, lsfRequireDestTag)),
                tx_has_destination_tag: tx.is_field_present(sf("sfDestinationTag")),
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
    let source_sle = read_account(view, source)?;
    let destination_sle = read_account(view, destination)?;
    let amount_exceeds_available =
        !account_holds_at_least(view, source, &requested, requested.native())?;
    let mut base = run_check_cash_preclaim(CheckCashPreclaimFacts {
        check_exists: true,
        tx_account_is_check_destination: tx.get_account_id(sf("sfAccount")) == destination,
        check_source_is_destination: source == destination,
        source_account_exists: source_sle.is_some(),
        destination_account_exists: destination_sle.is_some(),
        destination_require_dest_tag: destination_sle
            .as_ref()
            .is_some_and(|sle| account_flag(sle, lsfRequireDestTag)),
        check_has_destination_tag: check.is_field_present(sf("sfDestinationTag")),
        check_expired: has_expired(
            view,
            check
                .is_field_present(sf("sfExpiration"))
                .then(|| check.get_field_u32(sf("sfExpiration"))),
        ),
        requested_currency_matches_send_max: requested.asset() == send_max.asset(),
        requested_issuer_matches_send_max: requested.issue().issuer() == send_max.issue().issuer(),
        requested_value_exceeds_send_max: requested > send_max,
        requested_value_exceeds_available_funds: amount_exceeds_available,
        requested_value_native: requested.native(),
        requested_value_issuer_is_destination: requested.issue().issuer() == destination,
        issuer_exists: true,
        issuer_requires_auth: false,
        destination_trustline_exists: true,
        destination_trustline_authorized: true,
        destination_trustline_frozen: false,
    });
    if base != Ter::TES_SUCCESS || requested.native() || requested.issue().issuer() == destination {
        return Ok(base);
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
            let Some(_) = view
                .read(protocol::mpt_issuance_keylet_from_mptid(issue.mpt_id()))
                .map_err(|_| view_error())?
            else {
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
            base = ledger::mptoken_helpers::can_transfer_mpt(view, &issue, &source, &destination)
                .map_err(|_| view_error())?;
        }
    }
    Ok(base)
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
    let destination_sle = read_account(view, destination)?;
    let (kind, asset_preclaim_result) = match amount.asset() {
        Asset::Issue(issue) if issue.native() => (EscrowCreateAmountKind::Xrp, Ter::TES_SUCCESS),
        Asset::Issue(issue) => {
            let issuer = read_account(view, issue.account)?;
            let line = view
                .read(protocol::line(account, issue.account, issue.currency))
                .map_err(|_| view_error())?;
            let mut spendable = line
                .as_ref()
                .map(|line| line.get_field_amount(sf("sfBalance")))
                .unwrap_or_else(|| amount.zeroed());
            if account > issue.account {
                spendable.negate();
            }
            spendable.set_issuer(issue.account);
            let asset = run_escrow_create_issue_preclaim(EscrowCreateIssuePreclaimFacts {
                issuer_equals_account: issue.account == account,
                issuer_exists: issuer.is_some(),
                issuer_allows_trustline_locking: issuer
                    .as_ref()
                    .is_some_and(|sle| account_flag(sle, lsfAllowTrustLineLocking)),
                trustline_exists: line.is_some(),
                trustline_balance_sign_valid: spendable.signum() >= 0,
                sender_auth_result: iou_auth(view, account, issue)?,
                destination_auth_result: iou_auth(view, destination, issue)?,
                sender_frozen: iou_frozen(view, account, issue)?,
                destination_frozen: iou_frozen(view, destination, issue)?,
                spendable_amount_positive: spendable.signum() > 0,
                spendable_amount_covers_amount: amount <= spendable,
                can_add_amount: spendable.iou().checked_add(amount.iou()).is_ok(),
            });
            (EscrowCreateAmountKind::Issue, asset)
        }
        Asset::MPTIssue(issue) => {
            let issuance = view
                .read(protocol::mpt_issuance_keylet_from_mptid(issue.mpt_id()))
                .map_err(|_| view_error())?;
            let token = view
                .read(protocol::mptoken_keylet_from_mptid(
                    issue.mpt_id(),
                    Uint160::from_void(account.data()),
                ))
                .map_err(|_| view_error())?;
            let available = token
                .as_ref()
                .map_or(0, |token| token.get_field_u64(sf("sfMPTAmount")));
            let asset = run_escrow_create_mpt_preclaim(EscrowCreateMptPreclaimFacts {
                issuer_equals_account: issue.issuer() == account,
                issuance_exists: issuance.is_some(),
                issuance_can_escrow: issuance
                    .as_ref()
                    .is_some_and(|sle| sle.is_flag(protocol::lsfMPTCanEscrow)),
                issuance_issuer_matches: issuance
                    .as_ref()
                    .is_some_and(|sle| sle.get_account_id(sf("sfIssuer")) == issue.issuer()),
                sender_token_exists: token.is_some(),
                sender_auth_result: ledger::mptoken_helpers::require_auth_mpt(
                    view, &issue, &account,
                )
                .map_err(|_| view_error())?,
                destination_auth_result: ledger::mptoken_helpers::require_auth_mpt(
                    view,
                    &issue,
                    &destination,
                )
                .map_err(|_| view_error())?,
                sender_locked: ledger::mptoken_helpers::is_frozen_mpt(view, &account, &issue)
                    .map_err(|_| view_error())?,
                destination_locked: ledger::mptoken_helpers::is_frozen_mpt(
                    view,
                    &destination,
                    &issue,
                )
                .map_err(|_| view_error())?,
                can_transfer_result: ledger::mptoken_helpers::can_transfer_mpt(
                    view,
                    &issue,
                    &account,
                    &destination,
                )
                .map_err(|_| view_error())?,
                spendable_amount_positive: available > 0,
                spendable_amount_covers_amount: amount.mpt().value() >= 0
                    && (amount.mpt().value() as u64) <= available,
            });
            (EscrowCreateAmountKind::Mpt, asset)
        }
    };
    Ok(run_escrow_create_preclaim(EscrowCreatePreclaimFacts {
        destination_exists: destination_sle.is_some(),
        destination_is_pseudo_account: destination_sle
            .as_ref()
            .is_some_and(|sle| account_is_pseudo(sle)),
        amount_kind: kind,
        token_escrow_enabled: view.rules().enabled(&protocol::feature_token_escrow()),
        asset_preclaim_result,
    }))
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
    let asset_preclaim_result = match escrow.get_field_amount(sf("sfAmount")).asset() {
        Asset::Issue(issue) if issue.native() => Ter::TES_SUCCESS,
        Asset::Issue(issue) => run_escrow_cancel_issue_preclaim(EscrowCancelIssuePreclaimFacts {
            issuer_equals_account: issue.account == owner,
            require_auth_result: iou_auth(view, owner, issue)?,
        }),
        Asset::MPTIssue(issue) => run_escrow_cancel_mpt_preclaim(EscrowCancelMptPreclaimFacts {
            issuer_equals_account: issue.issuer() == owner,
            issuance_exists: view
                .read(protocol::mpt_issuance_keylet_from_mptid(issue.mpt_id()))
                .map_err(|_| view_error())?
                .is_some(),
            require_auth_result: ledger::mptoken_helpers::require_auth_mpt(view, &issue, &owner)
                .map_err(|_| view_error())?,
        }),
    };
    Ok(run_escrow_cancel_preclaim(EscrowCancelPreclaimFacts {
        token_escrow_enabled: true,
        escrow_exists: true,
        amount_is_xrp: escrow.get_field_amount(sf("sfAmount")).native(),
        asset_preclaim_result,
    }))
}

#[cfg(test)]
mod tests {
    use std::{collections::BTreeMap, sync::Arc};

    use basics::base_uint::Uint256;
    use ledger::{Fees, LedgerHeader, ReadView, ReadViewTx, ViewError};
    use protocol::{
        AccountID, ApplyFlags, Keylet, Rules, STAmount, STLedgerEntry, STTx, Ter, TxType, XRPAmount,
    };

    use super::run_read_view_preclaim;

    #[derive(Debug, Default)]
    struct MockView {
        entries: BTreeMap<Uint256, Arc<STLedgerEntry>>,
        header: LedgerHeader,
        rules: Rules,
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
            Ok(self.entries.contains_key(&keylet.key))
        }
        fn succ(
            &self,
            _key: Uint256,
            _last: Option<Uint256>,
        ) -> Result<Option<Uint256>, ViewError> {
            Ok(None)
        }
        fn read(&self, keylet: Keylet) -> Result<Option<Arc<STLedgerEntry>>, ViewError> {
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
