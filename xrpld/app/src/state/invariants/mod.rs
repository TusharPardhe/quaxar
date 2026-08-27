use std::collections::{BTreeMap, BTreeSet};

use basics::base_uint::Uint256;
use ledger::{ApplyView, FlowSandbox, ReadView, flow_sandbox::Action};
use protocol::{AccountID, LedgerEntryType, STAmount, STTx, Ter, XRPAmount, get_field_by_symbol};

mod amm;
mod clawback;
mod common;
mod directory;
mod entry;
mod freeze;
mod lending;
mod mpt;
mod object_deletion;
mod permissioned_dex;
mod permissioned_domain;
mod vault;

use amm::*;
use clawback::*;
use common::{raw_account_id, sf};
use directory::*;
use entry::*;
use freeze::*;
use lending::*;
use mpt::*;
use object_deletion::*;
use permissioned_dex::*;
use permissioned_domain::*;
use vault::*;

type InvariantEntry = (
    ledger::flow_sandbox::Entry,
    Option<std::sync::Arc<protocol::STLedgerEntry>>,
);

#[derive(Clone)]
pub(crate) struct InvariantDeltaPrefix {
    entries: BTreeMap<Uint256, InvariantEntry>,
}

impl InvariantDeltaPrefix {
    pub(crate) fn capture<V: ApplyView + ?Sized>(
        view: &FlowSandbox<V>,
    ) -> Result<Self, ledger::ViewError> {
        let mut entries = BTreeMap::new();
        for (key, entry) in view.items() {
            let before = view.peek_parent(protocol::Keylet::new(entry.sle.get_type(), *key))?;
            entries.insert(*key, (entry.clone(), before));
        }
        Ok(Self { entries })
    }
}

fn merged_invariant_entries<V: ApplyView + ?Sized>(
    sandbox: &FlowSandbox<V>,
    prefix: Option<&InvariantDeltaPrefix>,
) -> Result<BTreeMap<Uint256, InvariantEntry>, ledger::ViewError> {
    let mut merged = prefix
        .map(|prefix| prefix.entries.clone())
        .unwrap_or_default();
    for (key, child) in sandbox.items() {
        if let Some((parent, before)) = merged.get(key).cloned() {
            if parent.action == Action::Insert && child.action == Action::Erase {
                merged.remove(key);
                continue;
            }
            let action = if child.action == Action::Erase {
                Action::Erase
            } else if parent.action == Action::Insert {
                Action::Insert
            } else {
                Action::Modify
            };
            merged.insert(
                *key,
                (
                    ledger::flow_sandbox::Entry {
                        action,
                        sle: std::sync::Arc::clone(&child.sle),
                    },
                    before,
                ),
            );
        } else {
            let before = sandbox.peek_parent(protocol::Keylet::new(child.sle.get_type(), *key))?;
            merged.insert(*key, (child.clone(), before));
        }
    }
    Ok(merged)
}

/// Mirrors `ApplyContext::failInvariantCheck`: a broken invariant while
/// recovering a prior invariant failure is a hard failure and must not enter
/// the ledger as a fee-claim transaction.
fn invariant_failure_result(result: Ter) -> Ter {
    if matches!(
        result,
        Ter::TEC_INVARIANT_FAILED | Ter::TEF_INVARIANT_FAILED
    ) {
        Ter::TEF_INVARIANT_FAILED
    } else {
        Ter::TEC_INVARIANT_FAILED
    }
}

fn map_invariant_result(result: Ter, checked: Result<Ter, ()>) -> Ter {
    match checked {
        Ok(result) => result,
        Err(()) => invariant_failure_result(result),
    }
}

fn mpt_transfer_validation_result(result: Result<bool, ledger::ViewError>) -> Result<bool, Ter> {
    result.map_err(|_| Ter::TEF_BAD_LEDGER)
}

pub fn check_invariants_for_tx<V: ApplyView + ?Sized>(
    sandbox: &FlowSandbox<V>,
    tx: &STTx,
    result: Ter,
    fee: XRPAmount,
) -> Ter {
    check_invariants_for_tx_with_expected_xrp_delta(sandbox, tx, result, fee, None)
}

/// Production invariant entry point. `expected_xrp_delta` describes the
/// mutation scope being checked: handler/cleanup sandboxes exclude the fee and
/// therefore expect zero; the outer transaction sandbox includes the charged
/// fee and expects its negative. The public compatibility wrapper above keeps
/// synthetic invariant tests able to exercise one invariant in isolation.
pub(crate) fn check_invariants_for_tx_with_expected_xrp_delta<V: ApplyView + ?Sized>(
    sandbox: &FlowSandbox<V>,
    tx: &STTx,
    result: Ter,
    fee: XRPAmount,
    expected_xrp_delta: Option<i64>,
) -> Ter {
    check_invariants_for_tx_with_prefix(sandbox, tx, result, fee, expected_xrp_delta, None)
}

pub(crate) fn check_invariants_for_tx_with_prefix<V: ApplyView + ?Sized>(
    sandbox: &FlowSandbox<V>,
    tx: &STTx,
    result: Ter,
    fee: XRPAmount,
    expected_xrp_delta: Option<i64>,
    prefix: Option<&InvariantDeltaPrefix>,
) -> Ter {
    let fee_field = sf("sfFee");
    if tx.is_field_present(fee_field) && fee.drops() > tx.get_field_amount(fee_field).xrp().drops()
    {
        return invariant_failure_result(result);
    }
    let txn_type = tx.get_txn_type();
    let tx_domain = tx
        .is_field_present(sf("sfDomainID"))
        .then(|| tx.get_field_h256(sf("sfDomainID")));
    let tx_account = tx
        .is_field_present(sf("sfAccount"))
        .then(|| tx.get_account_id(sf("sfAccount")));
    let tx_account_paid_fee = tx_account.is_some_and(|account| tx.get_fee_payer_id() == account);
    let tx_destination = tx
        .is_field_present(sf("sfDestination"))
        .then(|| tx.get_account_id(sf("sfDestination")));
    let tx_holder = tx
        .is_field_present(sf("sfHolder"))
        .then(|| tx.get_account_id(sf("sfHolder")));
    let tx_amount = tx
        .is_field_present(sf("sfAmount"))
        .then(|| tx.get_field_amount(sf("sfAmount")));
    let tx_has_holder = tx.is_field_present(sf("sfHolder"));
    let cross_currency_payment = payment_is_cross_currency(tx);
    map_invariant_result(
        result,
        check_invariants_inner(
            sandbox,
            txn_type,
            tx_domain,
            tx_account,
            tx_destination,
            tx_holder,
            tx_amount,
            tx_has_holder,
            cross_currency_payment,
            tx_account_paid_fee,
            result,
            fee,
            expected_xrp_delta,
            prefix,
        ),
    )
}

pub fn check_invariants<V: ApplyView + ?Sized>(
    sandbox: &FlowSandbox<V>,
    txn_type: protocol::TxType,
    result: Ter,
    fee: XRPAmount,
) -> Ter {
    map_invariant_result(
        result,
        check_invariants_inner(
            sandbox, txn_type, None, None, None, None, None, false, false, false, result, fee,
            None, None,
        ),
    )
}

fn payment_is_cross_currency(tx: &STTx) -> bool {
    if tx.get_txn_type() != protocol::TxType::PAYMENT || !tx.is_field_present(sf("sfAmount")) {
        return false;
    }

    let amount = tx.get_field_amount(sf("sfAmount"));
    let send_max = if tx.is_field_present(sf("sfSendMax")) {
        tx.get_field_amount(sf("sfSendMax"))
    } else {
        amount.clone()
    };
    send_max.asset() != amount.asset()
}

fn pay_channel_held_drops(amount: STAmount, balance: STAmount) -> i64 {
    amount.xrp().drops() - balance.xrp().drops()
}

fn check_invariants_inner<V: ApplyView + ?Sized>(
    sandbox: &FlowSandbox<V>,
    txn_type: protocol::TxType,
    tx_domain: Option<Uint256>,
    tx_account: Option<AccountID>,
    tx_destination: Option<AccountID>,
    tx_holder: Option<AccountID>,
    tx_amount: Option<STAmount>,
    tx_has_holder: bool,
    cross_currency_payment: bool,
    tx_account_paid_fee: bool,
    result: Ter,
    fee: XRPAmount,
    expected_xrp_delta: Option<i64>,
    prefix: Option<&InvariantDeltaPrefix>,
) -> Result<Ter, ()> {
    let mut xrp_balance_change: i64 = 0;
    let mut has_xrp_trust_line = false;
    let mut deep_freeze_violation = false;
    let mut mpt_issuance_locked_violation = false;
    let fix_cleanup_3_1_3 = sandbox
        .rules()
        .enabled(&protocol::feature_id("fixCleanup3_1_3"));
    let fix_cleanup_3_2_0 = sandbox
        .rules()
        .enabled(&protocol::feature_id("fixCleanup3_2_0"));
    let amm_invariant_enabled =
        fix_cleanup_3_2_0 || sandbox.rules().enabled(&protocol::fix_ammv1_3());
    let single_asset_vault_enabled = sandbox
        .rules()
        .enabled(&protocol::feature_id("SingleAssetVault"));
    let vault_invariant_enabled = fix_cleanup_3_2_0 || single_asset_vault_enabled;
    let lending_protocol_enabled = sandbox
        .rules()
        .enabled(&protocol::feature_id("LendingProtocol"));
    let mptokens_v2_enabled = sandbox.rules().enabled(&protocol::feature_id("MPTokensV2"));
    let mpt_transfer_invariant_enabled = fix_cleanup_3_2_0 || mptokens_v2_enabled;
    let permissioned_dex_invariant_enabled = fix_cleanup_3_2_0
        || sandbox
            .rules()
            .enabled(&protocol::feature_id("PermissionedDEX"));
    let mut directory_roots = BTreeSet::new();
    let mut mpt_accounting = BTreeMap::new();
    let mut mpt_transfers = BTreeMap::new();
    let mut mpt_issuance_lifecycle = MptIssuanceLifecycle::default();
    let mut confidential_mpt = BTreeMap::new();
    let mut permissioned_domain = PermissionedDomainState::default();
    let mut permissioned_dex = PermissionedDexState::default();
    let mut amm = AmmState::default();
    let mut vault = VaultState::default();
    let mut lending = LendingState::default();
    let mut clawback = ClawbackState::default();
    let mut object_deletion = ObjectDeletionState::default();
    let fix_cleanup_3_3_0 = sandbox.rules().enabled(&protocol::fix_cleanup_3_3_0());
    let fix_cleanup_3_4_0 = sandbox
        .rules()
        .enabled(&protocol::feature_id("fixCleanup3_4_0"));
    let mut before_minted_nfts = 0u64;
    let mut after_minted_nfts = 0u64;
    let mut before_burned_nfts = 0u64;
    let mut after_burned_nfts = 0u64;
    let mut delta_sponsored_owner_count = 0i64;
    let mut delta_sponsoring_owner_count = 0i64;
    let mut delta_sponsored_object_owner_count = 0i64;
    let mut owner_count_below_sponsored = false;
    let mut delta_sponsoring_account_count = 0i64;
    let mut delta_account_sponsor_presence = 0i64;
    let mut invalid_amount = false;
    let mut invalid_pseudo_account = false;
    let mut invalid_nft_page = false;
    let mut deleted_final_nft_page = false;
    let mut deleted_nft_page_link = false;
    let mut accounts_created = 0u32;
    let mut created_account_seq = 0u32;
    let mut created_account_is_pseudo = false;
    let mut created_account_flags = 0u32;
    let mut accounts_deleted = 0u32;
    let mut deleted_account_roots = Vec::new();
    let mut invalid_unmodifiable_field = false;
    let mut freeze = FreezeState::default();

    let merged_entries = match merged_invariant_entries(sandbox, prefix) {
        Ok(entries) => entries,
        Err(_) => return Ok(Ter::TEF_BAD_LEDGER),
    };
    for (_index, (entry, before)) in merged_entries {
        let is_delete = entry.action == Action::Erase;
        let after = if is_delete { None } else { Some(&entry.sle) };

        let before_sle = before.as_deref();
        let after_sle = after.map(|s| &**s);
        // ApplyStateTable::visit always supplies the current SLE as `after`,
        // including for an erase. Deletion is represented exclusively by
        // `is_delete`; invariants which need the post-reset object must not
        // infer deletion from a null after pointer.
        let visited_after_sle = Some(&*entry.sle);
        record_freeze_state(&mut freeze, is_delete, before_sle, &entry.sle);
        record_confidential_mpt(&mut confidential_mpt, is_delete, before_sle, &entry.sle);

        if !is_delete && let Some(before) = before_sle {
            let before_object = before.clone_as_object();
            let after_object = entry.sle.clone_as_object();
            let field_changed = |name: &str| {
                let field = sf(name);
                match (
                    before_object.peek_at_pfield(field),
                    after_object.peek_at_pfield(field),
                ) {
                    (None, None) => false,
                    (Some(left), Some(right)) => !left.is_equivalent(right),
                    _ => true,
                }
            };
            let mut changed = field_changed("sfLedgerEntryType") || field_changed("sfLedgerIndex");
            let fields: &[&str] = match entry.sle.get_type() {
                LedgerEntryType::LoanBroker => &[
                    "sfSequence",
                    "sfOwnerNode",
                    "sfVaultNode",
                    "sfVaultID",
                    "sfAccount",
                    "sfOwner",
                    "sfManagementFeeRate",
                    "sfCoverRateMinimum",
                    "sfCoverRateLiquidation",
                ],
                LedgerEntryType::Loan => &[
                    "sfSequence",
                    "sfOwnerNode",
                    "sfLoanBrokerNode",
                    "sfLoanBrokerID",
                    "sfBorrower",
                    "sfLoanOriginationFee",
                    "sfLoanServiceFee",
                    "sfLatePaymentFee",
                    "sfClosePaymentFee",
                    "sfOverpaymentFee",
                    "sfInterestRate",
                    "sfLateInterestRate",
                    "sfCloseInterestRate",
                    "sfOverpaymentInterestRate",
                    "sfStartDate",
                    "sfPaymentInterval",
                    "sfGracePeriod",
                    "sfLoanScale",
                ],
                LedgerEntryType::Vault
                    if sandbox
                        .rules()
                        .enabled(&protocol::feature_id("LendingProtocolV1_1")) =>
                {
                    &[
                        "sfVaultKind",
                        "sfSubscriptionDate",
                        "sfRedemptionDate",
                        "sfSequence",
                        "sfOwnerNode",
                        "sfOwner",
                        "sfWithdrawalPolicy",
                        "sfScale",
                        "sfLEVersion",
                    ]
                }
                _ => &[],
            };
            changed |= fields.iter().any(|field| field_changed(field));
            invalid_unmodifiable_field |= changed;
        }

        if entry.sle.get_type() == LedgerEntryType::AccountRoot {
            if before_sle.is_none() && !is_delete {
                accounts_created = accounts_created.saturating_add(1);
                created_account_seq = entry.sle.get_field_u32(sf("sfSequence"));
                created_account_is_pseudo = ledger::is_pseudo_account(&entry.sle);
                created_account_flags = entry.sle.get_field_u32(sf("sfFlags"));
            }
            if is_delete && before_sle.is_some() {
                accounts_deleted = accounts_deleted.saturating_add(1);
                deleted_account_roots.push((
                    before_sle.expect("checked deleted account").clone(),
                    entry.sle.as_ref().clone(),
                ));
            }
        }

        let account_counter = |sle: Option<&protocol::STLedgerEntry>, field: &str| -> i64 {
            sle.filter(|entry| entry.get_type() == LedgerEntryType::AccountRoot)
                .map(|entry| i64::from(entry.get_field_u32(sf(field))))
                .unwrap_or(0)
        };
        before_minted_nfts = before_minted_nfts
            .saturating_add(account_counter(before_sle, "sfMintedNFTokens") as u64);
        after_minted_nfts = after_minted_nfts
            .saturating_add(account_counter(visited_after_sle, "sfMintedNFTokens") as u64);
        before_burned_nfts = before_burned_nfts
            .saturating_add(account_counter(before_sle, "sfBurnedNFTokens") as u64);
        after_burned_nfts = after_burned_nfts
            .saturating_add(account_counter(visited_after_sle, "sfBurnedNFTokens") as u64);

        let sponsored_object_count = |sle: Option<&protocol::STLedgerEntry>| -> i64 {
            let Some(sle) = sle else { return 0 };
            if sle.get_type() == LedgerEntryType::AccountRoot {
                return 0;
            }
            if sle.get_type() == LedgerEntryType::RippleState {
                return i64::from(sle.is_field_present(sf("sfHighSponsor")))
                    + i64::from(sle.is_field_present(sf("sfLowSponsor")));
            }
            if !sle.is_field_present(sf("sfSponsor")) {
                return 0;
            }
            match sle.get_type() {
                LedgerEntryType::Oracle => {
                    i64::from(if sle.get_field_array(sf("sfPriceDataSeries")).len() > 5 {
                        2
                    } else {
                        1
                    })
                }
                LedgerEntryType::Vault => 2,
                LedgerEntryType::SignerList => {
                    if sle.is_flag(protocol::lsfOneOwnerCount) {
                        1
                    } else {
                        2 + sle.get_field_array(sf("sfSignerEntries")).len() as i64
                    }
                }
                _ => 1,
            }
        };
        delta_sponsored_owner_count += account_counter(visited_after_sle, "sfSponsoredOwnerCount")
            - account_counter(before_sle, "sfSponsoredOwnerCount");
        delta_sponsoring_owner_count +=
            account_counter(visited_after_sle, "sfSponsoringOwnerCount")
                - account_counter(before_sle, "sfSponsoringOwnerCount");
        delta_sponsored_object_owner_count += (if is_delete {
            0
        } else {
            sponsored_object_count(visited_after_sle)
        }) - sponsored_object_count(before_sle);
        if let Some(account) =
            visited_after_sle.filter(|sle| sle.get_type() == LedgerEntryType::AccountRoot)
            && account.get_field_u32(sf("sfOwnerCount"))
                < account.get_field_u32(sf("sfSponsoredOwnerCount"))
        {
            owner_count_below_sponsored = true;
        }
        delta_sponsoring_account_count +=
            account_counter(visited_after_sle, "sfSponsoringAccountCount")
                - account_counter(before_sle, "sfSponsoringAccountCount");
        delta_account_sponsor_presence += i64::from(visited_after_sle.is_some_and(|sle| {
            sle.get_type() == LedgerEntryType::AccountRoot && sle.is_field_present(sf("sfSponsor"))
        })) - i64::from(before_sle.is_some_and(|sle| {
            sle.get_type() == LedgerEntryType::AccountRoot && sle.is_field_present(sf("sfSponsor"))
        }));

        if !is_delete && let Some(after) = visited_after_sle {
            invalid_amount |= protocol::has_invalid_amount(&after.clone_as_object());
        }
        if !is_delete
            && let Some(account) =
                visited_after_sle.filter(|sle| sle.get_type() == LedgerEntryType::AccountRoot)
        {
            let pseudo_fields = ["sfAMMID", "sfVaultID", "sfLoanBrokerID"];
            let pseudo_field_count = pseudo_fields
                .iter()
                .filter(|field| account.is_field_present(sf(field)))
                .count();
            if pseudo_field_count != 0 || account.get_field_u32(sf("sfSequence")) == 0 {
                let required_flags = protocol::lsfDisableMaster
                    | protocol::lsfDefaultRipple
                    | protocol::lsfDepositAuth;
                invalid_pseudo_account |= pseudo_field_count != 1
                    || before_sle.is_some_and(|before| {
                        before.get_field_u32(sf("sfSequence"))
                            != account.get_field_u32(sf("sfSequence"))
                    })
                    || account.get_field_u32(sf("sfFlags")) & required_flags != required_flags
                    || account.is_field_present(sf("sfRegularKey"))
                    || account.is_field_present(sf("sfSponsoredOwnerCount"))
                    || account.is_field_present(sf("sfSponsoringOwnerCount"))
                    || account.is_field_present(sf("sfSponsoringAccountCount"))
                    || account.is_field_present(sf("sfSponsor"));
            }
        }

        if entry.sle.get_type() == LedgerEntryType::NFTokenPage
            && before_sle.is_none_or(|before| before.get_type() == LedgerEntryType::NFTokenPage)
        {
            let page_mask = protocol::nft_page_mask();
            let account_mask = !page_mask;
            let mut check_page = |page: &protocol::STLedgerEntry| {
                let account_bits = *page.key() & account_mask;
                let high_limit = *page.key() & page_mask;
                let previous = page
                    .is_field_present(sf("sfPreviousPageMin"))
                    .then(|| page.get_field_h256(sf("sfPreviousPageMin")));
                let next = page
                    .is_field_present(sf("sfNextPageMin"))
                    .then(|| page.get_field_h256(sf("sfNextPageMin")));
                if previous.is_some_and(|previous| {
                    account_bits != previous & account_mask || high_limit <= previous & page_mask
                }) || next.is_some_and(|next| {
                    account_bits != next & account_mask || high_limit >= next & page_mask
                }) {
                    invalid_nft_page = true;
                }
                let tokens = page.get_field_array(sf("sfNFTokens"));
                if (!is_delete && tokens.is_empty())
                    || tokens.len() > protocol::DIR_MAX_TOKENS_PER_PAGE
                {
                    invalid_nft_page = true;
                }
                let low_limit = previous.map(|value| value & page_mask).unwrap_or_default();
                let mut prior = low_limit;
                for token in tokens.iter() {
                    let token_id = token.get_field_h256(sf("sfNFTokenID"));
                    if ledger::nftoken_helpers::compare_tokens(&prior, &token_id)
                        != std::cmp::Ordering::Less
                        || (token_id & page_mask) < low_limit
                        || (token_id & page_mask) >= high_limit
                        || (token.is_field_present(sf("sfURI"))
                            && token.get_field_vl(sf("sfURI")).is_empty())
                    {
                        invalid_nft_page = true;
                    }
                    prior = token_id;
                }
            };
            if let Some(before) = before_sle {
                check_page(before);
                if is_delete
                    && (*before.key() & page_mask) == page_mask
                    && before.is_field_present(sf("sfPreviousPageMin"))
                {
                    deleted_final_nft_page = true;
                }
            }
            check_page(&entry.sle);
            if !is_delete
                && let Some(before) = before_sle
                && (*before.key() & page_mask) != page_mask
                && before.is_field_present(sf("sfNextPageMin"))
                && !entry.sle.is_field_present(sf("sfNextPageMin"))
            {
                deleted_nft_page_link = true;
            }
        }

        // 4. LedgerEntryTypesMatch
        if let (Some(b), Some(a)) = (before_sle, after_sle) {
            if b.get_type() != a.get_type() {
                return Err(());
            }
        }

        // 2. AccountRootsNotDeleted
        if is_delete {
            let sle_to_delete = before_sle.unwrap_or(&*entry.sle);
            if sle_to_delete.get_type() == LedgerEntryType::AccountRoot {
                if txn_type != protocol::TxType::ACCOUNT_DELETE
                    && txn_type != protocol::TxType::VAULT_DELETE
                    && txn_type != protocol::TxType::LOAN_BROKER_DELETE
                    && txn_type != protocol::TxType::AMM_DELETE
                    && txn_type != protocol::TxType::AMM_WITHDRAW
                    && txn_type != protocol::TxType::AMM_CLAWBACK
                {
                    return Err(());
                }
            }
        }

        let sle_type = after_sle
            .map(|s| s.get_type())
            .unwrap_or_else(|| before_sle.unwrap_or(&*entry.sle).get_type());

        if amm_invariant_enabled {
            record_amm_state(&mut amm, is_delete, before_sle, after_sle);
        }
        if vault_invariant_enabled {
            record_vault_state(&mut vault, is_delete, before_sle, after_sle);
        }
        if lending_protocol_enabled {
            if record_lending_state(sandbox, &mut lending, after_sle).is_err() {
                return Ok(Ter::TEF_BAD_LEDGER);
            }
        }
        if fix_cleanup_3_1_3 || txn_type == protocol::TxType::PERMISSIONED_DOMAIN_SET {
            record_permissioned_domain_state(
                &mut permissioned_domain,
                is_delete,
                before_sle,
                after_sle,
            );
        }

        if mpt_transfer_invariant_enabled {
            if let Some(b) = before_sle {
                record_mpt_accounting(&mut mpt_accounting, b, true);
                record_mpt_transfer(&mut mpt_transfers, b, true);
            }
            if let Some(a) = after_sle {
                record_mpt_accounting(&mut mpt_accounting, a, false);
                record_mpt_transfer(&mut mpt_transfers, a, false);
                if fix_cleanup_3_2_0 && protocol::has_invalid_amount(&a.clone_as_object()) {
                    return Err(());
                }
            }
        }

        if permissioned_dex_invariant_enabled {
            record_permissioned_dex(&mut permissioned_dex, is_delete, before_sle, after_sle);
        }
        record_clawback_state(&mut clawback, before_sle);

        if fix_cleanup_3_3_0 {
            record_object_deletion_state(&mut object_deletion, is_delete, before_sle);
        }

        if fix_cleanup_3_2_0 || mptokens_v2_enabled {
            let deleted_sle = before_sle.unwrap_or(&entry.sle);
            if record_mpt_issuance_lifecycle(
                sandbox,
                txn_type,
                &mut mpt_issuance_lifecycle,
                is_delete,
                before_sle,
                after_sle,
                deleted_sle,
            )
            .is_err()
            {
                return Ok(Ter::TEF_BAD_LEDGER);
            }
        }

        if fix_cleanup_3_2_0 {
            if !maybe_record_directory_root(&mut directory_roots, is_delete, before_sle, after_sle)
            {
                return Err(());
            }
        }

        match sle_type {
            LedgerEntryType::AccountRoot => {
                // 8. XRPBalanceChecks
                for a in [before_sle, visited_after_sle].into_iter().flatten() {
                    let balance_field = get_field_by_symbol("sfBalance");
                    if a.is_field_present(balance_field) {
                        let bal = a.get_field_amount(balance_field);
                        if !bal.native()
                            || bal.negative()
                            || bal.xrp().drops() > protocol::INITIAL_XRP.drops()
                        {
                            return Err(());
                        }
                    }
                }

                // 1. XRPNotCreated (AccountRoot)
                let bal_before = before_sle
                    .map(|b| {
                        b.get_field_amount(get_field_by_symbol("sfBalance"))
                            .xrp()
                            .drops() as i64
                    })
                    .unwrap_or(0);
                let bal_after = after_sle
                    .map(|a| {
                        a.get_field_amount(get_field_by_symbol("sfBalance"))
                            .xrp()
                            .drops() as i64
                    })
                    .unwrap_or(0);
                xrp_balance_change += bal_after - bal_before;
            }
            LedgerEntryType::Escrow => {
                // 6. NoZeroEscrow
                for a in [before_sle, visited_after_sle].into_iter().flatten() {
                    let amt = a.get_field_amount(get_field_by_symbol("sfAmount"));
                    let invalid = if amt.native() {
                        amt.signum() <= 0 || amt.xrp().drops() >= protocol::INITIAL_XRP.drops()
                    } else {
                        amt.signum() <= 0
                            || match amt.asset() {
                                protocol::Asset::Issue(issue) => {
                                    issue.currency == protocol::bad_currency()
                                }
                                protocol::Asset::MPTIssue(_) => {
                                    amt.mpt().value() > protocol::MAX_MP_TOKEN_AMOUNT
                                }
                            }
                    };
                    if invalid {
                        return Err(());
                    }
                }

                // 1. XRPNotCreated (Escrow). Token escrows are covered by
                // token-specific accounting; only native amounts affect XRP.
                let bal_before = before_sle
                    .map(|b| b.get_field_amount(get_field_by_symbol("sfAmount")))
                    .filter(|amount| amount.native())
                    .map(|amount| amount.xrp().drops())
                    .unwrap_or(0);
                let bal_after = after_sle
                    .map(|a| a.get_field_amount(get_field_by_symbol("sfAmount")))
                    .filter(|amount| amount.native())
                    .map(|amount| amount.xrp().drops())
                    .unwrap_or(0);
                xrp_balance_change += bal_after - bal_before;
            }
            LedgerEntryType::PayChannel => {
                // 1. XRPNotCreated (PayChannel).  A channel's XRP still held
                // in escrow is `sfAmount - sfBalance`, not `sfAmount`.
                // PaymentChannelClaim advances sfBalance while crediting the
                // destination by the same delta; counting sfAmount alone
                // therefore reports that credit as newly-created XRP and
                // incorrectly converts a valid claim to tecINVARIANT_FAILED.
                // This mirrors rippled XRPNotCreated::visitEntry exactly,
                // including ignoring the after-value for a deleted channel
                // (closeChannel refunds the remaining held balance).
                let bal_before = before_sle
                    .map(|b| {
                        pay_channel_held_drops(
                            b.get_field_amount(get_field_by_symbol("sfAmount")),
                            b.get_field_amount(get_field_by_symbol("sfBalance")),
                        )
                    })
                    .unwrap_or(0);
                let bal_after = (!is_delete)
                    .then_some(after_sle)
                    .flatten()
                    .map(|a| {
                        pay_channel_held_drops(
                            a.get_field_amount(get_field_by_symbol("sfAmount")),
                            a.get_field_amount(get_field_by_symbol("sfBalance")),
                        )
                    })
                    .unwrap_or(0);
                xrp_balance_change += bal_after - bal_before;
            }
            LedgerEntryType::Sponsorship => {
                // A prefunded sponsorship escrows XRP in sfFeeAmount. Match
                // rippled XRPNotCreated accounting so creating, consuming, or
                // deleting that object cannot appear to mint or lose XRP.
                let fee_amount = get_field_by_symbol("sfFeeAmount");
                let bal_before = before_sle
                    .filter(|sle| sle.is_field_present(fee_amount))
                    .map(|sle| sle.get_field_amount(fee_amount).xrp().drops())
                    .unwrap_or(0);
                let bal_after = (!is_delete)
                    .then_some(after_sle)
                    .flatten()
                    .filter(|sle| sle.is_field_present(fee_amount))
                    .map(|sle| sle.get_field_amount(fee_amount).xrp().drops())
                    .unwrap_or(0);
                xrp_balance_change += bal_after - bal_before;
            }
            LedgerEntryType::Offer => {
                // 5. NoBadOffers
                for a in [before_sle, visited_after_sle].into_iter().flatten() {
                    let gets = a.get_field_amount(get_field_by_symbol("sfTakerGets"));
                    let pays = a.get_field_amount(get_field_by_symbol("sfTakerPays"));
                    if gets.negative() || pays.negative() || (gets.native() && pays.native()) {
                        return Err(());
                    }
                }
            }
            LedgerEntryType::DirectoryNode => {}
            LedgerEntryType::RippleState => {
                if let Some(a) = after_sle {
                    has_xrp_trust_line = accumulate_invariant_violation(
                        has_xrp_trust_line,
                        is_xrp_trust_line(a),
                        fix_cleanup_3_1_3,
                    );
                    deep_freeze_violation = accumulate_invariant_violation(
                        deep_freeze_violation,
                        has_deep_freeze_without_freeze(a),
                        fix_cleanup_3_1_3,
                    );
                }
            }
            LedgerEntryType::MPTokenIssuance | LedgerEntryType::MPToken => {
                if let Some(a) = after_sle {
                    if a.get_type() == LedgerEntryType::MPTokenIssuance
                        && a.is_field_present(sf("sfLockedAmount"))
                    {
                        mpt_issuance_locked_violation = accumulate_invariant_violation(
                            mpt_issuance_locked_violation,
                            a.get_field_u64(sf("sfOutstandingAmount"))
                                < a.get_field_u64(sf("sfLockedAmount")),
                            fix_cleanup_3_1_3,
                        );
                    }
                    if fix_cleanup_3_2_0 && !validate_mpt_entry(a) {
                        return Err(());
                    }
                }
            }
            LedgerEntryType::Vault => {}
            LedgerEntryType::AMM => {
                if amm_invariant_enabled
                    && amm_invariant_result_applies(result)
                    && let Some(a) = after_sle
                    && !validate_amm_entry(a)
                {
                    return Err(());
                }
            }
            LedgerEntryType::Loan => {
                if lending_protocol_enabled
                    && let Some(a) = after_sle
                    && !validate_loan_entry(before_sle, a)
                {
                    return Err(());
                }
            }
            LedgerEntryType::LoanBroker => {
                if lending_protocol_enabled && let Some(a) = after_sle {
                    let valid = match validate_loan_broker_entry(
                        sandbox,
                        txn_type,
                        fix_cleanup_3_1_3,
                        before_sle,
                        a,
                    ) {
                        Ok(valid) => valid,
                        Err(_) => return Ok(Ter::TEF_BAD_LEDGER),
                    };
                    if !valid {
                        return Err(());
                    }
                }
            }
            _ => {}
        }
    }

    if has_xrp_trust_line || deep_freeze_violation || mpt_issuance_locked_violation {
        return Err(());
    }
    let valid_freeze = match validates_transfers_not_frozen(sandbox, txn_type, &freeze) {
        Ok(valid) => valid,
        Err(_) => return Ok(Ter::TEF_BAD_LEDGER),
    };
    if !valid_freeze {
        return Err(());
    }
    if !validates_confidential_mpt(sandbox, txn_type, result, &confidential_mpt) {
        return Err(());
    }

    let changes_nft_counts = matches!(
        txn_type,
        protocol::TxType::NFTOKEN_MINT | protocol::TxType::NFTOKEN_BURN
    );
    if !changes_nft_counts {
        if before_minted_nfts != after_minted_nfts || before_burned_nfts != after_burned_nfts {
            return Err(());
        }
    } else if txn_type == protocol::TxType::NFTOKEN_MINT {
        if (protocol::is_tes_success(result) && before_minted_nfts >= after_minted_nfts)
            || (!protocol::is_tes_success(result) && before_minted_nfts != after_minted_nfts)
            || before_burned_nfts != after_burned_nfts
        {
            return Err(());
        }
    } else if (protocol::is_tes_success(result) && before_burned_nfts >= after_burned_nfts)
        || (!protocol::is_tes_success(result) && before_burned_nfts != after_burned_nfts)
        || before_minted_nfts != after_minted_nfts
    {
        return Err(());
    }

    if delta_sponsored_owner_count != delta_sponsoring_owner_count
        || owner_count_below_sponsored
        || delta_sponsored_object_owner_count != delta_sponsored_owner_count
        || delta_sponsoring_account_count != delta_account_sponsor_presence
    {
        return Err(());
    }

    if invalid_amount && fix_cleanup_3_2_0 {
        return Err(());
    }
    if invalid_pseudo_account && single_asset_vault_enabled {
        return Err(());
    }
    if invalid_unmodifiable_field && lending_protocol_enabled {
        return Err(());
    }
    if invalid_nft_page
        || (sandbox.rules().enabled(&protocol::fix_nftoken_page_links())
            && (deleted_final_nft_page || deleted_nft_page_link))
    {
        return Err(());
    }

    let must_delete_account = matches!(
        txn_type,
        protocol::TxType::ACCOUNT_DELETE
            | protocol::TxType::AMM_DELETE
            | protocol::TxType::VAULT_DELETE
            | protocol::TxType::LOAN_BROKER_DELETE
    );
    let may_delete_account = matches!(
        txn_type,
        protocol::TxType::AMM_WITHDRAW | protocol::TxType::AMM_CLAWBACK
    );
    if protocol::is_tes_success(result) && must_delete_account {
        if accounts_deleted != 1 {
            return Err(());
        }
    } else if !(protocol::is_tes_success(result) && may_delete_account && accounts_deleted == 1)
        && accounts_deleted != 0
    {
        return Err(());
    }

    if accounts_created > 1 {
        return Err(());
    }
    if accounts_created == 1 {
        let can_create_account = matches!(
            txn_type,
            protocol::TxType::PAYMENT
                | protocol::TxType::XCHAIN_ADD_CLAIM_ATTESTATION
                | protocol::TxType::XCHAIN_ADD_ACCOUNT_CREATE_ATTESTATION
                | protocol::TxType::AMM_CREATE
                | protocol::TxType::VAULT_CREATE
                | protocol::TxType::LOAN_BROKER_SET
        );
        let can_create_pseudo = matches!(
            txn_type,
            protocol::TxType::AMM_CREATE
                | protocol::TxType::VAULT_CREATE
                | protocol::TxType::LOAN_BROKER_SET
        );
        if !protocol::is_tes_success(result) || !can_create_account {
            return Err(());
        }
        let pseudo =
            created_account_is_pseudo && (single_asset_vault_enabled || lending_protocol_enabled);
        if (pseudo && !can_create_pseudo)
            || created_account_seq != if pseudo { 0 } else { sandbox.header().seq }
            || (pseudo
                && created_account_flags
                    != (protocol::lsfDisableMaster
                        | protocol::lsfDefaultRipple
                        | protocol::lsfDepositAuth))
        {
            return Err(());
        }
    }

    let account_delete_clean_enforced = fix_cleanup_3_2_0
        || sandbox.rules().enabled(&protocol::feature_sponsor())
        || single_asset_vault_enabled
        || lending_protocol_enabled;
    if account_delete_clean_enforced {
        for (before, after) in deleted_account_roots {
            if after.get_field_amount(sf("sfBalance")).signum() != 0
                || after.get_field_u32(sf("sfOwnerCount")) != 0
                || after.is_field_present(sf("sfSponsoredOwnerCount"))
                || after.is_field_present(sf("sfSponsoringOwnerCount"))
                || after.is_field_present(sf("sfSponsoringAccountCount"))
                || after.is_field_present(sf("sfSponsor"))
            {
                return Err(());
            }
            let account = before.get_account_id(sf("sfAccount"));
            let raw = raw_account_id(account);
            let direct = [
                protocol::owner_dir_keylet(raw),
                protocol::signers_keylet(raw),
                protocol::nft_page_min_keylet(raw),
                protocol::nft_page_max_keylet(raw),
                protocol::did_keylet(raw),
            ];
            for keylet in direct {
                match sandbox.read(keylet) {
                    Ok(Some(_)) => return Err(()),
                    Ok(None) => {}
                    Err(_) => return Ok(Ter::TEF_BAD_LEDGER),
                }
            }
            let first = protocol::nft_page_min_keylet(raw);
            let last = protocol::nft_page_max_keylet(raw);
            match sandbox.succ(first.key, Some(last.key.next())) {
                Ok(Some(_)) => return Err(()),
                Ok(None) => {}
                Err(_) => return Ok(Ter::TEF_BAD_LEDGER),
            }
            for field in ["sfAMMID", "sfVaultID", "sfLoanBrokerID"] {
                if before.is_field_present(sf(field)) {
                    match sandbox.read(protocol::Keylet::new(
                        LedgerEntryType::Any,
                        before.get_field_h256(sf(field)),
                    )) {
                        Ok(Some(_)) => return Err(()),
                        Ok(None) => {}
                        Err(_) => return Ok(Ter::TEF_BAD_LEDGER),
                    }
                }
            }
        }
    }

    if (fix_cleanup_3_1_3 || txn_type == protocol::TxType::PERMISSIONED_DOMAIN_SET)
        && !validates_permissioned_domain(txn_type, result, fix_cleanup_3_1_3, &permissioned_domain)
    {
        return Err(());
    }

    if permissioned_dex_invariant_enabled {
        let valid_permissioned_dex = match validates_permissioned_dex(
            sandbox,
            txn_type,
            result,
            tx_domain,
            fix_cleanup_3_1_3,
            fix_cleanup_3_2_0,
            fix_cleanup_3_4_0,
            &permissioned_dex,
        ) {
            Ok(valid) => valid,
            Err(_) => return Ok(Ter::TEF_BAD_LEDGER),
        };
        if !valid_permissioned_dex {
            return Err(());
        }
    }

    let valid_clawback = match validates_clawback(
        sandbox,
        txn_type,
        result,
        tx_account,
        tx_holder,
        tx_amount.as_ref(),
        mptokens_v2_enabled,
        &clawback,
    ) {
        Ok(valid) => valid,
        Err(_) => return Ok(Ter::TEF_BAD_LEDGER),
    };
    if !valid_clawback {
        return Err(());
    }

    if fix_cleanup_3_2_0 || mptokens_v2_enabled {
        if !validates_mpt_issuance_lifecycle(&mpt_issuance_lifecycle) {
            return Err(());
        }
        if !validates_mpt_lifecycle_counts(
            txn_type,
            result,
            tx_has_holder,
            single_asset_vault_enabled,
            lending_protocol_enabled,
            mptokens_v2_enabled,
            &mpt_issuance_lifecycle,
        ) {
            return Err(());
        }
    }

    if fix_cleanup_3_2_0 {
        for root_index in directory_roots {
            match sandbox.read(protocol::Keylet::new(
                LedgerEntryType::DirectoryNode,
                root_index,
            )) {
                Ok(Some(_)) => {}
                Ok(None) => return Err(()),
                Err(_) => return Ok(Ter::TEF_BAD_LEDGER),
            }
        }
    }

    if mpt_transfer_invariant_enabled {
        if !validates_mpt_accounting_for_transaction(&mpt_accounting, mptokens_v2_enabled, txn_type)
        {
            return Err(());
        }
        let valid_mpt_transfers = match mpt_transfer_validation_result(validates_mpt_transfers(
            sandbox,
            txn_type,
            cross_currency_payment,
            fix_cleanup_3_2_0,
            mptokens_v2_enabled,
            &mpt_transfers,
        )) {
            Ok(valid) => valid,
            // A storage failure is not a failed protocol invariant. Preserve
            // the hard bad-ledger result instead of fee-claiming it as
            // tecINVARIANT_FAILED.
            Err(ter) => return Ok(ter),
        };
        if !valid_mpt_transfers {
            return Err(());
        }
    }

    if amm_invariant_enabled {
        match validates_amm_state(sandbox, txn_type, result, &amm) {
            Ok(true) => {}
            Ok(false) => return Err(()),
            Err(_) => return Ok(Ter::TEF_BAD_LEDGER),
        }
    }

    if vault_invariant_enabled {
        let vault_reads = match validate_vault_read_channel(sandbox, txn_type, &vault) {
            Ok(reads) => reads,
            Err(_) => return Ok(Ter::TEF_BAD_LEDGER),
        };
        if !validates_vault_state(
            sandbox,
            txn_type,
            tx_account,
            tx_destination,
            tx_holder,
            tx_amount.as_ref(),
            tx_account_paid_fee,
            fee,
            fix_cleanup_3_2_0,
            result,
            &vault,
            &vault_reads,
        ) {
            return Err(());
        }
    }

    if fix_cleanup_3_3_0 {
        match validates_object_deletion(sandbox, &object_deletion) {
            Ok(true) => {}
            Ok(false) => return Err(()),
            Err(_) => return Ok(Ter::TEF_BAD_LEDGER),
        }
    }

    if lending_protocol_enabled {
        for broker_id in lending.broker_refs {
            match sandbox.read(protocol::loan_broker_keylet_from_key(broker_id)) {
                Ok(Some(_)) => {}
                Ok(None) => return Err(()),
                Err(_) => return Ok(Ter::TEF_BAD_LEDGER),
            }
        }
    }

    // 1. XRPNotCreated (finalize). Production callers supply the exact delta
    // appropriate to the sandbox scope. This is the two-sandbox equivalent of
    // rippled's `-drops_ == fee`: handler/cleanup state must conserve XRP,
    // while the outer transaction delta must destroy exactly the charged fee.
    if let Some(expected) = expected_xrp_delta {
        if xrp_balance_change != expected {
            return Err(());
        }
    } else if xrp_balance_change > 0 {
        return Err(());
    }

    // 3. TransactionFeeCheck
    if fee.drops() < 0 || fee.drops() >= protocol::INITIAL_XRP.drops() {
        return Err(());
    }

    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::freeze::freeze_override_allowed;
    use super::permissioned_dex::{
        PermissionedDexState, permissioned_dex_consumed_wrong_domain, record_permissioned_dex,
    };
    use super::vault::{
        VaultAssetDelta, VaultSnapshot, VaultState, add_vault_asset_delta, compute_vault_min_scale,
        rounded_vault_delta, valid_vault_loss_unrealized, vault_transaction_account_asset_delta,
    };
    use super::{
        check_invariants_for_tx_with_expected_xrp_delta, mpt_transfer_validation_result,
        pay_channel_held_drops,
    };
    use basics::{
        base_uint::Uint256,
        number::{NumberParts as RuntimeNumber, get_mantissa_scale},
    };
    use ledger::{ApplyView, FlowSandbox, Ledger, LedgerHeader, Sandbox};
    use protocol::{
        AccountID, ApplyFlags, Asset, Issue, STAmount, STLedgerEntry, Ter, TxType, XRPAmount,
        account_keylet, get_field_by_symbol, pay_channel_keylet_from_key,
    };
    use std::sync::Arc;

    fn account(byte: u8) -> AccountID {
        AccountID::from_array([byte; 20])
    }

    #[test]
    fn mpt_transfer_invariant_storage_failure_is_a_hard_bad_ledger() {
        assert_eq!(mpt_transfer_validation_result(Ok(true)), Ok(true));
        assert_eq!(mpt_transfer_validation_result(Ok(false)), Ok(false));
        assert_eq!(
            mpt_transfer_validation_result(Err(ledger::ViewError::Conversion(
                "injected MPT invariant SHAMap read failure".to_owned(),
            ))),
            Err(Ter::TEF_BAD_LEDGER)
        );
    }

    fn usd_asset() -> Asset {
        Asset::Issue(Issue {
            currency: protocol::currency_from_string("USD"),
            account: account(0xA1),
        })
    }

    fn vault_snapshot_with_scale(scale: Option<i32>) -> VaultSnapshot {
        VaultSnapshot {
            key: Uint256::from_u64(1),
            asset: usd_asset(),
            pseudo_id: account(0xA2),
            share_mpt_id: protocol::MPTIssue::new(protocol::make_mpt_id(1, account(0xA2))).mpt_id(),
            scale,
            assets_total: RuntimeNumber::from_i64(1),
            assets_available: RuntimeNumber::from_i64(1),
            loss_unrealized: RuntimeNumber::zero(),
        }
    }

    #[test]
    fn vault_invariant_compensates_only_the_sources_own_xrp_fee() {
        let depositor = account(0xA3);
        let asset = Asset::Issue(protocol::xrp_issue());
        let mut state = VaultState::default();
        add_vault_asset_delta(
            &mut state,
            depositor,
            asset,
            RuntimeNumber::from_i64(-1_000_010),
            None,
        );

        let delta = vault_transaction_account_asset_delta(
            &state,
            depositor,
            asset,
            true,
            protocol::XRPAmount::from_drops(10),
        )
        .expect("the XRP deposit transfer must retain its nonzero delta");
        assert_eq!(delta.delta, RuntimeNumber::from_i64(-1_000_000));

        let mut sponsored_state = VaultState::default();
        add_vault_asset_delta(
            &mut sponsored_state,
            depositor,
            asset,
            RuntimeNumber::from_i64(-1_000_000),
            None,
        );
        let sponsored = vault_transaction_account_asset_delta(
            &sponsored_state,
            depositor,
            asset,
            false,
            protocol::XRPAmount::from_drops(10),
        )
        .expect("a sponsor's fee must not be added to the source delta");
        assert_eq!(sponsored.delta, RuntimeNumber::from_i64(-1_000_000));
    }

    #[test]
    fn vault_negative_unrealized_loss_is_gated_by_cleanup_3_4_0() {
        let negative = RuntimeNumber::from_i64(-1);
        assert!(valid_vault_loss_unrealized(negative, false));
        assert!(!valid_vault_loss_unrealized(negative, true));
        assert!(valid_vault_loss_unrealized(RuntimeNumber::zero(), true));
    }

    #[test]
    fn permissioned_dex_deleted_domain_is_legacy_only_after_cleanup_3_4_0() {
        let tx_domain = Uint256::from_u64(1);
        let deleted_domain = Uint256::from_u64(2);
        let mut deleted_offer = STLedgerEntry::from_type_and_key(
            protocol::LedgerEntryType::Offer,
            Uint256::from_u64(3),
        );
        deleted_offer.set_field_h256(get_field_by_symbol("sfDomainID"), deleted_domain);
        let mut state = PermissionedDexState::default();
        record_permissioned_dex(&mut state, true, Some(&deleted_offer), None);

        assert!(permissioned_dex_consumed_wrong_domain(
            &state, tx_domain, false
        ));
        assert!(!permissioned_dex_consumed_wrong_domain(
            &state, tx_domain, true
        ));
    }

    #[test]
    fn amm_clawback_freeze_override_is_extended_by_cleanup_3_4_0() {
        assert!(!freeze_override_allowed(true, false, true, false));
        assert!(freeze_override_allowed(true, true, true, false));
        assert!(freeze_override_allowed(true, false, true, true));
        assert!(!freeze_override_allowed(false, true, true, false));
    }

    #[test]
    fn pay_channel_invariant_counts_only_unpaid_xrp() {
        let amount = STAmount::from_xrp_amount(XRPAmount::from_drops(500_000));
        let before_balance = STAmount::from_xrp_amount(XRPAmount::from_drops(0));
        let after_balance = STAmount::from_xrp_amount(XRPAmount::from_drops(25_000));

        let before = pay_channel_held_drops(amount.clone(), before_balance);
        let after = pay_channel_held_drops(amount, after_balance);

        assert_eq!(before, 500_000);
        assert_eq!(after, 475_000);
        assert_eq!(after - before, -25_000);
    }

    fn insert_account<V: ApplyView>(view: &mut V, id: AccountID, balance: i64) {
        let keylet = account_keylet(
            basics::base_uint::Uint160::from_slice(id.data()).expect("account width"),
        );
        let mut sle = STLedgerEntry::new(keylet);
        sle.set_account_id(get_field_by_symbol("sfAccount"), id);
        sle.set_field_amount(
            get_field_by_symbol("sfBalance"),
            STAmount::from_xrp_amount(XRPAmount::from_drops(balance)),
        );
        sle.set_field_u32(get_field_by_symbol("sfSequence"), 1);
        sle.set_field_u32(get_field_by_symbol("sfOwnerCount"), 0);
        view.insert(Arc::new(sle)).expect("insert account");
    }

    fn insert_pay_channel<V: ApplyView>(
        view: &mut V,
        key: Uint256,
        source: AccountID,
        destination: AccountID,
        amount: i64,
        balance: i64,
    ) {
        let mut sle = STLedgerEntry::new(pay_channel_keylet_from_key(key));
        sle.set_account_id(get_field_by_symbol("sfAccount"), source);
        sle.set_account_id(get_field_by_symbol("sfDestination"), destination);
        sle.set_field_amount(
            get_field_by_symbol("sfAmount"),
            STAmount::from_xrp_amount(XRPAmount::from_drops(amount)),
        );
        sle.set_field_amount(
            get_field_by_symbol("sfBalance"),
            STAmount::from_xrp_amount(XRPAmount::from_drops(balance)),
        );
        view.insert(Arc::new(sle)).expect("insert payment channel");
    }

    #[test]
    fn pay_channel_claim_destination_credit_preserves_xrp_invariant() {
        let source = account(0xC1);
        let destination = account(0xC2);
        let channel = Uint256::from_u64(0xCAFE);
        let base = Arc::new(Ledger::new(LedgerHeader::default(), false));
        let mut parent = Sandbox::new(base, ApplyFlags::NONE);
        insert_account(&mut parent, destination, 100_000_000);
        insert_pay_channel(&mut parent, channel, source, destination, 500_000, 0);

        let mut claim = FlowSandbox::new(&mut parent);
        let destination_keylet = account_keylet(
            basics::base_uint::Uint160::from_slice(destination.data()).expect("account width"),
        );
        let destination_sle = claim
            .peek(destination_keylet)
            .expect("read destination")
            .expect("destination exists");
        let mut destination_object = destination_sle.clone_as_object();
        destination_object.set_field_amount(
            get_field_by_symbol("sfBalance"),
            STAmount::from_xrp_amount(XRPAmount::from_drops(100_025_000)),
        );
        claim
            .update(Arc::new(STLedgerEntry::from_stobject(
                destination_object,
                destination_keylet.key,
            )))
            .expect("credit destination");

        let channel_keylet = pay_channel_keylet_from_key(channel);
        let channel_sle = claim
            .peek(channel_keylet)
            .expect("read channel")
            .expect("channel exists");
        let mut channel_object = channel_sle.clone_as_object();
        channel_object.set_field_amount(
            get_field_by_symbol("sfBalance"),
            STAmount::from_xrp_amount(XRPAmount::from_drops(25_000)),
        );
        claim
            .update(Arc::new(STLedgerEntry::from_stobject(
                channel_object,
                channel_keylet.key,
            )))
            .expect("advance channel balance");

        let tx = protocol::STTx::new(TxType::PAYCHAN_CLAIM, |tx| {
            tx.set_field_amount(
                get_field_by_symbol("sfFee"),
                STAmount::from_xrp_amount(XRPAmount::from_drops(0)),
            );
        });
        assert_eq!(
            check_invariants_for_tx_with_expected_xrp_delta(
                &claim,
                &tx,
                Ter::TES_SUCCESS,
                XRPAmount::from_drops(0),
                Some(0),
            ),
            Ter::TES_SUCCESS
        );
    }

    #[test]
    fn pay_channel_close_refund_preserves_xrp_invariant() {
        let source = account(0xD1);
        let destination = account(0xD2);
        let channel = Uint256::from_u64(0xD00D);
        let base = Arc::new(Ledger::new(LedgerHeader::default(), false));
        let mut parent = Sandbox::new(base, ApplyFlags::NONE);
        insert_account(&mut parent, source, 1_000_000);
        insert_pay_channel(&mut parent, channel, source, destination, 500_000, 25_000);

        let mut close = FlowSandbox::new(&mut parent);
        let source_keylet = account_keylet(
            basics::base_uint::Uint160::from_slice(source.data()).expect("account width"),
        );
        let source_sle = close
            .peek(source_keylet)
            .expect("read source")
            .expect("source exists");
        let mut source_object = source_sle.clone_as_object();
        source_object.set_field_amount(
            get_field_by_symbol("sfBalance"),
            STAmount::from_xrp_amount(XRPAmount::from_drops(1_475_000)),
        );
        close
            .update(Arc::new(STLedgerEntry::from_stobject(
                source_object,
                source_keylet.key,
            )))
            .expect("refund source");
        let channel_sle = close
            .peek(pay_channel_keylet_from_key(channel))
            .expect("read channel")
            .expect("channel exists");
        close.erase(channel_sle).expect("erase channel");

        let tx = protocol::STTx::new(TxType::PAYCHAN_CLAIM, |tx| {
            tx.set_field_amount(
                get_field_by_symbol("sfFee"),
                STAmount::from_xrp_amount(XRPAmount::from_drops(0)),
            );
        });
        assert_eq!(
            check_invariants_for_tx_with_expected_xrp_delta(
                &close,
                &tx,
                Ter::TES_SUCCESS,
                XRPAmount::from_drops(0),
                Some(0),
            ),
            Ter::TES_SUCCESS
        );
    }

    #[test]
    fn vault_invariant_min_scale_prefers_explicit_vault_scale_after_cleanup_3_2_0() {
        let before = vault_snapshot_with_scale(Some(-2));
        let after = vault_snapshot_with_scale(Some(-2));
        let delta = VaultAssetDelta {
            delta: RuntimeNumber::try_from_external_parts(12345, -4, get_mantissa_scale())
                .expect("valid delta"),
            scale: Some(-4),
        };

        assert_eq!(compute_vault_min_scale(&before, &after, delta, true), -2);
        assert_eq!(
            rounded_vault_delta(after.asset, delta, -2),
            RuntimeNumber::try_from_external_parts(123, -2, get_mantissa_scale())
                .expect("vault-scale rounded delta")
        );
    }

    #[test]
    fn vault_invariant_min_scale_preserves_legacy_coarsest_scale_before_cleanup_3_2_0() {
        let before = vault_snapshot_with_scale(Some(-2));
        let mut after = vault_snapshot_with_scale(Some(-2));
        after.assets_total =
            RuntimeNumber::try_from_external_parts(10001, -4, get_mantissa_scale())
                .expect("valid total");
        after.assets_available = after.assets_total;
        let delta = VaultAssetDelta {
            delta: RuntimeNumber::try_from_external_parts(1, -4, get_mantissa_scale())
                .expect("valid delta"),
            scale: Some(-4),
        };

        assert_eq!(compute_vault_min_scale(&before, &after, delta, false), -4);
    }
}
