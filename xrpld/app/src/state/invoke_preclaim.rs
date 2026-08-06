//! Concrete application adapter for rippled `applySteps.cpp::invokePreclaim`.
//!
//! The typed family preclaim tail remains owned by its family dispatchers. This
//! module owns the shared ledger-backed checks and invokes that tail only after
//! the exact generic `invokePreclaim` gate has succeeded.

use basics::base_uint::{Uint160, Uint256};
use ledger::ReadView;
use protocol::{
    AccountID, ApplyFlags, NotTec, PublicKey, STLedgerEntry, STObject, STTx, Ter, calc_account_id,
    feature_batch, feature_lending_protocol, get_field_by_symbol, is_tes_success, lsfDisableMaster,
};
use tx::{
    TransactorCheckFeeTx, TransactorCheckPermissionTx, TransactorCheckPriorTxAndLastLedgerTx,
    TransactorCheckSeqProxyTx, TransactorMultiSignAccountSigner, TransactorMultiSignSignerList,
    TransactorMultiSignTxSigner, TransactorSignMultiSignObject, TransactorSignObject,
    TransactorSignTx, TransactorSingleSignAccountState, run_check_tx_permission,
    run_transactor_check_fee, run_transactor_check_permission,
    run_transactor_check_prior_tx_and_last_ledger, run_transactor_check_seq_proxy,
    run_transactor_invoke_preclaim, run_transactor_preclaim_check_sign,
};

fn sf(name: &str) -> &'static protocol::SField {
    get_field_by_symbol(name)
}

fn account_keylet(account: AccountID) -> protocol::Keylet {
    protocol::account_keylet(Uint160::from_void(account.data()))
}

fn read_account<V: ReadView>(
    view: &V,
    account: &AccountID,
) -> Option<std::sync::Arc<STLedgerEntry>> {
    view.read(account_keylet(*account)).ok().flatten()
}

fn is_pseudo_account(account: Option<&LedgerSignAccountState>) -> bool {
    account.is_some_and(|account| account.is_pseudo)
}

/// Runs the concrete ledger-backed form of rippled `invokePreclaim`.
///
/// `calculate_base_fee` is deliberately lazy: the shared shell must not
/// calculate a fee until sequence, prior-transaction, permission, and signer
/// checks have all succeeded. `typed_preclaim_tail` is likewise deferred until
/// every generic guard succeeds. A zero account preserves rippled's pseudo
/// transaction exception by skipping the complete generic block.
#[allow(clippy::too_many_arguments)]
pub(crate) fn invoke_preclaim<V, CalculateBaseFee, MinimumFee, TypedPreclaimTail>(
    view: &V,
    tx: &STTx,
    current_ledger_seq: u32,
    flags: ApplyFlags,
    calculate_base_fee: CalculateBaseFee,
    minimum_fee: MinimumFee,
    typed_preclaim_tail: TypedPreclaimTail,
) -> Ter
where
    V: ReadView,
    CalculateBaseFee: FnOnce() -> i64,
    MinimumFee: FnOnce(i64) -> i64,
    TypedPreclaimTail: FnOnce() -> Ter,
{
    let adapted = LedgerPreclaimTx { tx };
    let account_is_zero = tx.get_account_id(sf("sfAccount")).is_zero();

    run_transactor_invoke_preclaim(
        account_is_zero,
        || {
            run_transactor_check_seq_proxy(
                &adapted,
                |account| read_account(view, account).map(LedgerSignAccountState::from),
                |account| account.sequence,
                |account, seq_proxy| {
                    view.exists(protocol::ticket_keylet_from_seq_proxy(
                        Uint160::from_void(account.data()),
                        seq_proxy,
                    ))
                    .unwrap_or(false)
                },
            )
        },
        || {
            run_transactor_check_prior_tx_and_last_ledger(
                current_ledger_seq,
                &adapted,
                |account| read_account(view, account),
                |account| {
                    if account.is_field_present(sf("sfAccountTxnID")) {
                        account.get_field_h256(sf("sfAccountTxnID"))
                    } else {
                        Uint256::zero()
                    }
                },
                |tx_id| view.tx_exists(*tx_id).unwrap_or(false),
            )
        },
        || typed_check_permission(view, tx, &adapted),
        || {
            if tx.get_txn_type() == protocol::TxType::LOAN_SET {
                check_loan_set_counterparty_sign(view, tx, flags, || {
                    check_ledger_signer_authorization(view, &adapted, flags)
                })
            } else {
                check_ledger_signer_authorization(view, &adapted, flags)
            }
        },
        calculate_base_fee,
        |base_fee| {
            run_transactor_check_fee(
                flags,
                view.open(),
                &adapted,
                base_fee,
                0_i64,
                |_| true,
                minimum_fee,
                |payer| read_account(view, payer).map(LedgerSignAccountState::from),
                |account| account.balance,
            )
        },
        typed_preclaim_tail,
    )
}

#[derive(Clone, Copy)]
struct LedgerPreclaimTx<'a> {
    tx: &'a STTx,
}

pub(crate) fn scale_fee_load(
    fee: i64,
    mut fee_factor: u32,
    remote_fee_factor: u32,
    load_base: u32,
    unlimited: bool,
) -> i64 {
    if fee == 0 {
        return fee;
    }

    if unlimited
        && fee_factor > remote_fee_factor
        && u64::from(fee_factor) < 4_u64.saturating_mul(u64::from(remote_fee_factor))
    {
        fee_factor = remote_fee_factor;
    }

    let denominator = u128::from(load_base);
    if denominator == 0 {
        return i64::MAX;
    }
    let Some(scaled) = u128::try_from(fee)
        .ok()
        .and_then(|fee| fee.checked_mul(u128::from(fee_factor)))
        .map(|numerator| numerator / denominator)
    else {
        return i64::MAX;
    };

    i64::try_from(scaled).unwrap_or(i64::MAX)
}

fn read_delegate_permissions<V: ReadView>(
    view: &V,
    account: AccountID,
    delegate: AccountID,
) -> Option<LedgerDelegatePermissions> {
    view.read(protocol::delegate_keylet(
        Uint160::from_void(account.data()),
        Uint160::from_void(delegate.data()),
    ))
    .ok()
    .flatten()
    .map(LedgerDelegatePermissions::from_entry)
}

fn tx_permission_result(delegate: &LedgerDelegatePermissions, tx: &STTx) -> NotTec {
    run_check_tx_permission(Some(&delegate.permissions), u16::from(tx.get_txn_type()))
}

fn typed_check_permission<V: ReadView>(
    view: &V,
    tx: &STTx,
    adapted: &LedgerPreclaimTx<'_>,
) -> NotTec {
    let account = tx.get_account_id(sf("sfAccount"));
    let delegate = tx
        .is_field_present(sf("sfDelegate"))
        .then(|| tx.get_account_id(sf("sfDelegate")));
    let delegate_permissions =
        delegate.and_then(|delegate| read_delegate_permissions(view, account, delegate));

    match tx.get_txn_type() {
        protocol::TxType::ACCOUNT_SET => tx::run_account_set_check_permission(
            adapted,
            |account, delegate| read_delegate_permissions(view, *account, *delegate),
            |permissions, permission| permissions.permissions.contains(&(permission as u32)),
        ),
        protocol::TxType::PAYMENT => {
            let amount = tx.get_field_amount(sf("sfAmount"));
            let asset = amount.asset();
            let issuer = asset.issuer();
            tx::run_payment_check_permission(tx::PaymentCheckPermissionFacts {
                delegate_present: delegate.is_some(),
                delegate_entry_exists: delegate_permissions.is_some(),
                check_tx_permission_result: delegate_permissions
                    .as_ref()
                    .map_or(Ter::TER_NO_DELEGATE_PERMISSION, |permissions| {
                        tx_permission_result(permissions, tx)
                    }),
                send_max_present: tx.is_field_present(sf("sfSendMax")),
                send_max_asset_matches_amount: !tx.is_field_present(sf("sfSendMax"))
                    || tx.get_field_amount(sf("sfSendMax")).asset() == asset,
                paths_present: tx.is_field_present(sf("sfPaths")),
                payment_mint_permission: delegate_permissions.as_ref().is_some_and(|permissions| {
                    permissions
                        .permissions
                        .contains(&(protocol::GranularPermissionType::PaymentMint as u32))
                }),
                payment_burn_permission: delegate_permissions.as_ref().is_some_and(|permissions| {
                    permissions
                        .permissions
                        .contains(&(protocol::GranularPermissionType::PaymentBurn as u32))
                }),
                amount_is_xrp: amount.native(),
                is_mpt: matches!(asset, protocol::Asset::MPTIssue(_)),
                amount_issuer_is_source: !amount.native() && issuer == account,
                amount_issuer_is_destination: !amount.native()
                    && issuer == tx.get_account_id(sf("sfDestination")),
                trustline_exists: match asset {
                    protocol::Asset::Issue(issue) if !issue.native() => view
                        .read(protocol::line(account, issuer, issue.currency))
                        .ok()
                        .flatten()
                        .is_some(),
                    _ => false,
                },
                account_is_holder: None,
                dest_limit_positive: None,
            })
        }
        protocol::TxType::TRUST_SET => {
            let limit = tx.get_field_amount(sf("sfLimitAmount"));
            let issuer = limit.issue().account;
            let trustline = view
                .read(protocol::line(account, issuer, limit.issue().currency))
                .ok()
                .flatten();
            let current_limit_equals_proposed_limit = trustline.as_ref().is_some_and(|line| {
                let current = line.get_field_amount(if account > issuer {
                    sf("sfHighLimit")
                } else {
                    sf("sfLowLimit")
                });
                let mut proposed = limit.clone();
                proposed.set_issuer(account);
                current == proposed
            });
            tx::run_trust_set_check_permission(tx::TrustSetCheckPermissionFacts {
                delegate_present: delegate.is_some(),
                delegate_entry_exists: delegate_permissions.is_some(),
                check_tx_permission_result: delegate_permissions
                    .as_ref()
                    .map_or(Ter::TER_NO_DELEGATE_PERMISSION, |permissions| {
                        tx_permission_result(permissions, tx)
                    }),
                tx_flags: tx.get_flags(),
                quality_in_present: tx.is_field_present(sf("sfQualityIn")),
                quality_out_present: tx.is_field_present(sf("sfQualityOut")),
                trustline_exists: trustline.is_some(),
                granular_trustline_authorize: delegate_permissions.as_ref().is_some_and(
                    |permissions| {
                        permissions.permissions.contains(
                            &(protocol::GranularPermissionType::TrustlineAuthorize as u32),
                        )
                    },
                ),
                granular_trustline_freeze: delegate_permissions.as_ref().is_some_and(
                    |permissions| {
                        permissions
                            .permissions
                            .contains(&(protocol::GranularPermissionType::TrustlineFreeze as u32))
                    },
                ),
                granular_trustline_unfreeze: delegate_permissions.as_ref().is_some_and(
                    |permissions| {
                        permissions
                            .permissions
                            .contains(&(protocol::GranularPermissionType::TrustlineUnfreeze as u32))
                    },
                ),
                current_limit_equals_proposed_limit,
            })
        }
        protocol::TxType::MPTOKEN_ISSUANCE_SET => {
            let mut granular_permissions = std::collections::BTreeSet::new();
            if delegate_permissions.as_ref().is_some_and(|permissions| {
                permissions
                    .permissions
                    .contains(&(protocol::GranularPermissionType::MPTokenIssuanceLock as u32))
            }) {
                granular_permissions.insert(tx::MPTokenIssuanceSetGranularPermission::Lock);
            }
            if delegate_permissions.as_ref().is_some_and(|permissions| {
                permissions
                    .permissions
                    .contains(&(protocol::GranularPermissionType::MPTokenIssuanceUnlock as u32))
            }) {
                granular_permissions.insert(tx::MPTokenIssuanceSetGranularPermission::Unlock);
            }
            tx::run_mp_token_issuance_set_check_permission(tx::MPTokenIssuanceSetPermissionFacts {
                delegate_present: delegate.is_some(),
                delegate_entry_exists: delegate_permissions.is_some(),
                broad_permission_granted: delegate_permissions.as_ref().is_some_and(
                    |permissions| is_tes_success(tx_permission_result(permissions, tx)),
                ),
                tx_flags: tx.get_flags(),
                granular_permissions,
            })
        }
        _ => run_transactor_check_permission(
            adapted,
            |account, delegate| read_delegate_permissions(view, *account, *delegate),
            |permissions, delegated_tx| tx_permission_result(&permissions, delegated_tx.tx),
        ),
    }
}

impl tx::AccountSetPermissionTx for LedgerPreclaimTx<'_> {
    type AccountId = AccountID;

    fn account_id(&self) -> Self::AccountId {
        self.tx.get_account_id(sf("sfAccount"))
    }

    fn delegate(&self) -> Option<Self::AccountId> {
        self.tx
            .is_field_present(sf("sfDelegate"))
            .then(|| self.tx.get_account_id(sf("sfDelegate")))
    }

    fn set_flag(&self) -> u32 {
        self.tx.get_field_u32(sf("sfSetFlag"))
    }

    fn clear_flag(&self) -> u32 {
        self.tx.get_field_u32(sf("sfClearFlag"))
    }

    fn flags(&self) -> u32 {
        self.tx.get_flags()
    }

    fn email_hash_present(&self) -> bool {
        self.tx.is_field_present(sf("sfEmailHash"))
    }

    fn wallet_locator_present(&self) -> bool {
        self.tx.is_field_present(sf("sfWalletLocator"))
    }

    fn nftoken_minter_present(&self) -> bool {
        self.tx.is_field_present(sf("sfNFTokenMinter"))
    }

    fn message_key_present(&self) -> bool {
        self.tx.is_field_present(sf("sfMessageKey"))
    }

    fn domain_present(&self) -> bool {
        self.tx.is_field_present(sf("sfDomain"))
    }

    fn transfer_rate_present(&self) -> bool {
        self.tx.is_field_present(sf("sfTransferRate"))
    }

    fn tick_size_present(&self) -> bool {
        self.tx.is_field_present(sf("sfTickSize"))
    }
}

impl TransactorCheckSeqProxyTx for LedgerPreclaimTx<'_> {
    type AccountId = AccountID;

    fn account_id(&self) -> Self::AccountId {
        self.tx.get_account_id(sf("sfAccount"))
    }

    fn seq_proxy(&self) -> protocol::SeqProxy {
        self.tx.get_seq_proxy()
    }

    fn ticket_sequence_present(&self) -> bool {
        self.tx.is_field_present(sf("sfTicketSequence"))
    }
}

impl TransactorCheckPriorTxAndLastLedgerTx for LedgerPreclaimTx<'_> {
    type AccountId = AccountID;
    type TxId = Uint256;

    fn account_id(&self) -> Self::AccountId {
        self.tx.get_account_id(sf("sfAccount"))
    }

    fn account_txn_id(&self) -> Option<Self::TxId> {
        self.tx
            .is_field_present(sf("sfAccountTxnID"))
            .then(|| self.tx.get_field_h256(sf("sfAccountTxnID")))
    }

    fn last_ledger_sequence(&self) -> Option<u32> {
        self.tx
            .is_field_present(sf("sfLastLedgerSequence"))
            .then(|| self.tx.get_field_u32(sf("sfLastLedgerSequence")))
    }

    fn transaction_id(&self) -> Self::TxId {
        self.tx.get_transaction_id()
    }
}

impl TransactorCheckPermissionTx for LedgerPreclaimTx<'_> {
    type AccountId = AccountID;

    fn account_id(&self) -> Self::AccountId {
        self.tx.get_account_id(sf("sfAccount"))
    }

    fn delegate(&self) -> Option<Self::AccountId> {
        self.tx
            .is_field_present(sf("sfDelegate"))
            .then(|| self.tx.get_account_id(sf("sfDelegate")))
    }
}

impl TransactorCheckFeeTx for LedgerPreclaimTx<'_> {
    type AccountId = AccountID;
    type Amount = i64;

    fn fee_is_native(&self) -> bool {
        !self.tx.is_field_present(sf("sfFee")) || self.tx.get_field_amount(sf("sfFee")).native()
    }

    fn fee_paid(&self) -> Self::Amount {
        self.tx
            .is_field_present(sf("sfFee"))
            .then(|| self.tx.get_field_amount(sf("sfFee")).xrp().drops())
            .unwrap_or(0)
    }

    fn fee_payer(&self) -> Self::AccountId {
        self.tx
            .is_field_present(sf("sfSponsor"))
            .then(|| self.tx.get_account_id(sf("sfSponsor")))
            .unwrap_or_else(|| self.tx.get_account_id(sf("sfAccount")))
    }
}

impl TransactorSignTx for LedgerPreclaimTx<'_> {
    type AccountId = AccountID;

    fn has_delegate(&self) -> bool {
        self.tx.is_field_present(sf("sfDelegate"))
    }

    fn delegate_account_id(&self) -> Self::AccountId {
        self.tx.get_account_id(sf("sfDelegate"))
    }

    fn account_id(&self) -> Self::AccountId {
        self.tx.get_account_id(sf("sfAccount"))
    }
}

struct LedgerDelegatePermissions {
    permissions: Vec<u32>,
}

impl LedgerDelegatePermissions {
    fn from_entry(entry: std::sync::Arc<STLedgerEntry>) -> Self {
        Self {
            permissions: entry
                .get_field_array(sf("sfPermissions"))
                .iter()
                .map(|permission| permission.get_field_u32(sf("sfPermissionValue")))
                .collect(),
        }
    }
}

#[derive(Clone)]
struct LedgerSignAccountState {
    sequence: u32,
    balance: i64,
    regular_key: Option<AccountID>,
    master_disabled: bool,
    is_pseudo: bool,
}

impl From<std::sync::Arc<STLedgerEntry>> for LedgerSignAccountState {
    fn from(entry: std::sync::Arc<STLedgerEntry>) -> Self {
        let regular_key = entry
            .is_field_present(sf("sfRegularKey"))
            .then(|| entry.get_account_id(sf("sfRegularKey")));
        Self {
            sequence: entry.get_field_u32(sf("sfSequence")),
            balance: entry.get_field_amount(sf("sfBalance")).xrp().drops(),
            regular_key,
            master_disabled: entry.get_field_u32(sf("sfFlags")) & lsfDisableMaster != 0,
            is_pseudo: ["sfAMMID", "sfVaultID", "sfLoanBrokerID"]
                .iter()
                .any(|field| entry.is_field_present(sf(field))),
        }
    }
}

impl TransactorSingleSignAccountState<AccountID> for LedgerSignAccountState {
    fn regular_key(&self) -> Option<&AccountID> {
        self.regular_key.as_ref()
    }

    fn is_master_disabled(&self) -> bool {
        self.master_disabled
    }
}

#[derive(Clone)]
struct LedgerAccountSigner {
    account: AccountID,
    weight: u32,
}

impl TransactorMultiSignAccountSigner<AccountID> for LedgerAccountSigner {
    fn account_id(&self) -> &AccountID {
        &self.account
    }

    fn weight(&self) -> u32 {
        self.weight
    }
}

#[derive(Clone)]
struct LedgerSignerList {
    signer_list_id_present: bool,
    signer_list_id: u32,
    quorum: u32,
    entries: Vec<LedgerAccountSigner>,
}

impl From<std::sync::Arc<STLedgerEntry>> for LedgerSignerList {
    fn from(entry: std::sync::Arc<STLedgerEntry>) -> Self {
        Self {
            signer_list_id_present: entry.is_field_present(sf("sfSignerListID")),
            signer_list_id: entry.get_field_u32(sf("sfSignerListID")),
            quorum: entry.get_field_u32(sf("sfSignerQuorum")),
            entries: entry
                .get_field_array(sf("sfSignerEntries"))
                .iter()
                .map(|signer| LedgerAccountSigner {
                    account: signer.get_account_id(sf("sfAccount")),
                    weight: u32::from(signer.get_field_u16(sf("sfSignerWeight"))),
                })
                .collect(),
        }
    }
}

impl TransactorMultiSignSignerList<LedgerAccountSigner> for LedgerSignerList {
    type Entries = Vec<LedgerAccountSigner>;

    fn signer_list_id_present(&self) -> bool {
        self.signer_list_id_present
    }

    fn signer_list_id(&self) -> u32 {
        self.signer_list_id
    }

    fn signer_quorum(&self) -> u32 {
        self.quorum
    }

    fn signer_entries(self) -> Result<Self::Entries, NotTec> {
        Ok(self.entries)
    }
}

#[derive(Clone)]
struct LedgerTxSigner {
    object: STObject,
}

impl TransactorMultiSignTxSigner<AccountID> for LedgerTxSigner {
    fn account_id(&self) -> AccountID {
        self.object.get_account_id(sf("sfAccount"))
    }

    fn signing_pub_key_is_empty(&self) -> bool {
        self.object.get_field_vl(sf("sfSigningPubKey")).is_empty()
    }
}

struct LedgerSignatureObject<'a> {
    tx: &'a STTx,
}

impl TransactorSignObject for LedgerSignatureObject<'_> {
    fn signing_pub_key_is_empty(&self) -> bool {
        self.tx.get_field_vl(sf("sfSigningPubKey")).is_empty()
    }

    fn has_signers(&self) -> bool {
        self.tx.is_field_present(sf("sfSigners"))
    }

    fn has_txn_signature(&self) -> bool {
        self.tx.is_field_present(sf("sfTxnSignature"))
    }
}

impl TransactorSignMultiSignObject<AccountID> for LedgerSignatureObject<'_> {
    type TxSigner = LedgerTxSigner;
    type TxSigners = Vec<LedgerTxSigner>;

    fn tx_signers(&self) -> Self::TxSigners {
        self.tx
            .get_field_array(sf("sfSigners"))
            .iter()
            .cloned()
            .map(|object| LedgerTxSigner { object })
            .collect()
    }
}

struct LedgerLoanSetSignTx<'a> {
    tx: &'a STTx,
    counterparty_signature: STObject,
}

impl tx::LoanSetSignTx for LedgerLoanSetSignTx<'_> {
    type AccountId = AccountID;
    type CounterpartySignature = STObject;

    fn counterparty(&self) -> Option<Self::AccountId> {
        self.tx
            .is_field_present(sf("sfCounterparty"))
            .then(|| self.tx.get_account_id(sf("sfCounterparty")))
    }

    fn has_counterparty_signature(&self) -> bool {
        self.tx.is_field_present(sf("sfCounterpartySignature"))
    }

    fn counterparty_signature(&self) -> &Self::CounterpartySignature {
        &self.counterparty_signature
    }
}

struct LedgerCounterpartySignTx {
    account: AccountID,
}

impl TransactorSignTx for LedgerCounterpartySignTx {
    type AccountId = AccountID;

    fn has_delegate(&self) -> bool {
        false
    }

    fn delegate_account_id(&self) -> Self::AccountId {
        self.account
    }

    fn account_id(&self) -> Self::AccountId {
        self.account
    }
}

struct LedgerCounterpartySignatureObject<'a> {
    object: &'a STObject,
}

impl TransactorSignObject for LedgerCounterpartySignatureObject<'_> {
    fn signing_pub_key_is_empty(&self) -> bool {
        self.object.get_field_vl(sf("sfSigningPubKey")).is_empty()
    }

    fn has_signers(&self) -> bool {
        self.object.is_field_present(sf("sfSigners"))
    }

    fn has_txn_signature(&self) -> bool {
        self.object.is_field_present(sf("sfTxnSignature"))
    }
}

impl TransactorSignMultiSignObject<AccountID> for LedgerCounterpartySignatureObject<'_> {
    type TxSigner = LedgerTxSigner;
    type TxSigners = Vec<LedgerTxSigner>;

    fn tx_signers(&self) -> Self::TxSigners {
        self.object
            .get_field_array(sf("sfSigners"))
            .iter()
            .cloned()
            .map(|object| LedgerTxSigner { object })
            .collect()
    }
}

/// `../rippled/src/libxrpl/tx/transactors/lending/LoanSet.cpp::LoanSet::checkSign`: runs the `CounterpartySignature` authorization part.
fn check_loan_set_counterparty_sign<V: ReadView>(
    view: &V,
    tx: &STTx,
    flags: ApplyFlags,
    check_primary_sign: impl FnOnce() -> NotTec,
) -> NotTec {
    let adapted = LedgerLoanSetSignTx {
        tx,
        counterparty_signature: tx.get_field_object(sf("sfCounterpartySignature")),
    };
    tx::run_loan_set_check_sign(
        &adapted,
        || {
            view.read(protocol::loan_broker_keylet_from_key(
                tx.get_field_h256(sf("sfLoanBrokerID")),
            ))
            .ok()
            .flatten()
            .map(|broker| broker.get_account_id(sf("sfOwner")))
        },
        check_primary_sign,
        |counter_signer, signature| {
            check_ledger_counterparty_signer_authorization(view, counter_signer, signature, flags)
        },
    )
}

fn check_ledger_counterparty_signer_authorization<V: ReadView>(
    view: &V,
    counter_signer: AccountID,
    signature: &STObject,
    flags: ApplyFlags,
) -> NotTec {
    let signer_tx = LedgerCounterpartySignTx {
        account: counter_signer,
    };
    let signature = LedgerCounterpartySignatureObject { object: signature };
    tx::run_transactor_preclaim_check_sign(
        flags,
        false,
        view.rules().enabled(&feature_batch()),
        view.rules().enabled(&feature_lending_protocol()),
        &signer_tx,
        &signature,
        |account| read_account(view, account).map(LedgerSignAccountState::from),
        |account| {
            view.read(protocol::signers_keylet(Uint160::from_void(account.data())))
                .ok()
                .flatten()
                .map(LedgerSignerList::from)
        },
        is_pseudo_account,
        |signature| {
            PublicKey::from_slice(&signature.object.get_field_vl(sf("sfSigningPubKey"))).is_ok()
        },
        |signature| {
            let key = PublicKey::from_slice(&signature.object.get_field_vl(sf("sfSigningPubKey")))
                .expect("public-key type was checked before account derivation");
            calc_account_id(key.as_bytes())
        },
        |signer| PublicKey::from_slice(&signer.object.get_field_vl(sf("sfSigningPubKey"))).is_ok(),
        |signer| {
            let key = PublicKey::from_slice(&signer.object.get_field_vl(sf("sfSigningPubKey")))
                .expect("public-key type was checked before account derivation");
            calc_account_id(key.as_bytes())
        },
    )
}

fn check_ledger_signer_authorization<V: ReadView>(
    view: &V,
    tx: &LedgerPreclaimTx<'_>,
    flags: ApplyFlags,
) -> NotTec {
    // Batch owns a distinct ledger-backed BatchSigners authorization tail.
    // Do not feed its intentionally empty outer SigningPubKey through the
    // ordinary account-signature checker before that tail runs.
    if tx.tx.get_txn_type() == protocol::TxType::BATCH {
        return Ter::TES_SUCCESS;
    }

    // The preflight caller validates cryptographic signatures. This step is
    // deliberately separate: it validates that those keys are authorized by
    // the current ledger's account root or signer list.
    if tx.tx.check_sign(&view.rules()).is_err() {
        return Ter::TEM_BAD_SIGNATURE;
    }

    let signature = LedgerSignatureObject { tx: tx.tx };
    run_transactor_preclaim_check_sign(
        flags,
        false,
        view.rules().enabled(&feature_batch()),
        view.rules().enabled(&feature_lending_protocol()),
        tx,
        &signature,
        |account| read_account(view, account).map(LedgerSignAccountState::from),
        |account| {
            view.read(protocol::signers_keylet(Uint160::from_void(account.data())))
                .ok()
                .flatten()
                .map(LedgerSignerList::from)
        },
        is_pseudo_account,
        |signature| {
            PublicKey::from_slice(&signature.tx.get_field_vl(sf("sfSigningPubKey"))).is_ok()
        },
        |signature| {
            let key = PublicKey::from_slice(&signature.tx.get_field_vl(sf("sfSigningPubKey")))
                .expect("public-key type was checked before account derivation");
            calc_account_id(key.as_bytes())
        },
        |signer| PublicKey::from_slice(&signer.object.get_field_vl(sf("sfSigningPubKey"))).is_ok(),
        |signer| {
            let key = PublicKey::from_slice(&signer.object.get_field_vl(sf("sfSigningPubKey")))
                .expect("public-key type was checked before account derivation");
            calc_account_id(key.as_bytes())
        },
    )
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use basics::base_uint::Uint160;
    use ledger::{ApplyView, Ledger};
    use protocol::{
        AccountID, ApplyFlags, LedgerEntryType, STAmount, STArray, STLedgerEntry, STObject, STTx,
        Ter, TxType, get_field_by_symbol,
    };

    use super::{LedgerPreclaimTx, scale_fee_load, typed_check_permission};

    fn insert_delegate(
        view: &mut ledger::Sandbox<ledger::Ledger>,
        account: AccountID,
        delegate: AccountID,
        permissions: &[u32],
    ) {
        let keylet = protocol::delegate_keylet(
            Uint160::from_void(account.data()),
            Uint160::from_void(delegate.data()),
        );
        let mut permission_entries = STArray::new(get_field_by_symbol("sfPermissions"));
        for permission in permissions {
            let mut entry = STObject::make_inner_object(get_field_by_symbol("sfPermission"));
            entry.set_field_u32(get_field_by_symbol("sfPermissionValue"), *permission);
            permission_entries.push_back(entry);
        }
        let mut entry = STLedgerEntry::from_type_and_key(LedgerEntryType::Delegate, keylet.key);
        entry.set_account_id(get_field_by_symbol("sfAccount"), account);
        entry.set_account_id(get_field_by_symbol("sfAuthorize"), delegate);
        entry.set_field_array(get_field_by_symbol("sfPermissions"), permission_entries);
        view.insert(Arc::new(entry))
            .expect("delegate entry should insert into the preclaim view");
    }

    #[test]
    fn shared_preclaim_dispatches_account_payment_trust_and_mpt_permissions_polymorphically() {
        // ../rippled/src/libxrpl/tx/transactors/account/AccountSet.cpp::AccountSet::checkPermission (lines 173-217); ../rippled/src/libxrpl/tx/transactors/payment/Payment.cpp::Payment::checkPermission (lines 276-312); ../rippled/src/libxrpl/tx/transactors/token/TrustSet.cpp::TrustSet::checkPermission (lines 128-184); ../rippled/src/libxrpl/tx/transactors/token/MPTokenIssuanceSet.cpp::MPTokenIssuanceSet::checkPermission (lines 143-172).
        let account = AccountID::from_array([0xA1; 20]);
        let broad_delegate = AccountID::from_array([0xA2; 20]);
        let granular_delegate = AccountID::from_array([0xA3; 20]);
        let mut view = ledger::Sandbox::new(
            Arc::new(ledger::Ledger::from_ledger_seq_and_close_time(1, 0, false)),
            ApplyFlags::NONE,
        );
        insert_delegate(
            &mut view,
            account,
            broad_delegate,
            &[
                protocol::Permission::tx_to_permission_type(TxType::ACCOUNT_SET),
                protocol::Permission::tx_to_permission_type(TxType::PAYMENT),
                protocol::Permission::tx_to_permission_type(TxType::TRUST_SET),
            ],
        );
        insert_delegate(
            &mut view,
            account,
            granular_delegate,
            &[protocol::GranularPermissionType::MPTokenIssuanceLock as u32],
        );

        let account_set = STTx::new(TxType::ACCOUNT_SET, |tx| {
            tx.set_account_id(get_field_by_symbol("sfAccount"), account);
            tx.set_account_id(get_field_by_symbol("sfDelegate"), broad_delegate);
            tx.set_field_vl(get_field_by_symbol("sfDomain"), b"example.org");
        });
        let payment = STTx::new(TxType::PAYMENT, |tx| {
            tx.set_account_id(get_field_by_symbol("sfAccount"), account);
            tx.set_account_id(get_field_by_symbol("sfDelegate"), broad_delegate);
            tx.set_account_id(
                get_field_by_symbol("sfDestination"),
                AccountID::from_array([0xB1; 20]),
            );
            tx.set_field_amount(
                get_field_by_symbol("sfAmount"),
                STAmount::new_native(1, false),
            );
        });
        let trust_set = STTx::new(TxType::TRUST_SET, |tx| {
            tx.set_account_id(get_field_by_symbol("sfAccount"), account);
            tx.set_account_id(get_field_by_symbol("sfDelegate"), broad_delegate);
            tx.set_field_amount(
                get_field_by_symbol("sfLimitAmount"),
                STAmount::new_native(1, false),
            );
        });
        let mpt_set = STTx::new(TxType::MPTOKEN_ISSUANCE_SET, |tx| {
            tx.set_account_id(get_field_by_symbol("sfAccount"), account);
            tx.set_account_id(get_field_by_symbol("sfDelegate"), granular_delegate);
            tx.set_field_u32(get_field_by_symbol("sfFlags"), protocol::tfMPTLock);
        });

        assert_eq!(
            typed_check_permission(&view, &account_set, &LedgerPreclaimTx { tx: &account_set }),
            Ter::TER_NO_DELEGATE_PERMISSION,
            "AccountSet must not accept a transaction-level permission"
        );
        assert_eq!(
            typed_check_permission(&view, &payment, &LedgerPreclaimTx { tx: &payment }),
            Ter::TES_SUCCESS
        );
        assert_eq!(
            typed_check_permission(&view, &trust_set, &LedgerPreclaimTx { tx: &trust_set }),
            Ter::TES_SUCCESS
        );
        assert_eq!(
            typed_check_permission(&view, &mpt_set, &LedgerPreclaimTx { tx: &mpt_set }),
            Ter::TES_SUCCESS,
            "MPTokenIssuanceSet must accept its lock granular permission"
        );
    }

    #[test]
    fn shared_preclaim_scales_minimum_fee_and_honors_unlimited_load_window() {
        // ../rippled/src/libxrpl/tx/Transactor.cpp::Transactor::minimumFee (lines 354-362) and ../rippled/src/libxrpl/server/LoadFeeTrack.cpp::scaleFeeLoad (lines 62-85).
        assert_eq!(scale_fee_load(10, 512, 256, 256, false), 20);
        assert_eq!(scale_fee_load(10, 512, 256, 256, true), 10);
        assert_eq!(scale_fee_load(10, 1_024, 256, 256, true), 40);
    }

    use super::check_loan_set_counterparty_sign;

    #[test]
    fn loan_set_check_sign_requires_a_counter_signer_before_optional_signature_success() {
        // ../rippled/src/libxrpl/tx/transactors/lending/LoanSet.cpp::LoanSet::checkSign: resolves the counter-signer before accepting an absent CounterpartySignature.
        let account = AccountID::from_array([0xC5; 20]);
        let tx = STTx::new(TxType::LOAN_SET, |tx| {
            tx.set_account_id(get_field_by_symbol("sfAccount"), account);
        });
        let view = Ledger::from_ledger_seq_and_close_time(1, 0, false);

        assert_eq!(
            check_loan_set_counterparty_sign(&view, &tx, ApplyFlags::NONE, || Ter::TES_SUCCESS),
            Ter::TEM_BAD_SIGNER
        );
    }
}
