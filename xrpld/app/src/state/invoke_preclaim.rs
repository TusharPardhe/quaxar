//! Concrete application adapter for rippled `applySteps.cpp::invokePreclaim`.
//!
//! The typed family preclaim tail remains owned by its family dispatchers. This
//! module owns the shared ledger-backed checks and invokes that tail only after
//! the exact generic `invokePreclaim` gate has succeeded.

use basics::base_uint::{Uint160, Uint256};
use ledger::ReadView;
use protocol::{
    AccountID, ApplyFlags, NotTec, PublicKey, STLedgerEntry, STObject, STTx, Ter, calc_account_id,
    feature_batch, feature_lending_protocol, get_field_by_symbol, lsfDisableMaster,
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
pub(crate) fn invoke_preclaim<V, CalculateBaseFee, TypedPreclaimTail>(
    view: &V,
    tx: &STTx,
    current_ledger_seq: u32,
    flags: ApplyFlags,
    calculate_base_fee: CalculateBaseFee,
    typed_preclaim_tail: TypedPreclaimTail,
) -> Ter
where
    V: ReadView,
    CalculateBaseFee: FnOnce() -> i64,
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
        || {
            run_transactor_check_permission(
                &adapted,
                |account, delegate| {
                    view.read(protocol::delegate_keylet(
                        Uint160::from_void(account.data()),
                        Uint160::from_void(delegate.data()),
                    ))
                    .ok()
                    .flatten()
                    .map(LedgerDelegatePermissions::from_entry)
                },
                |delegate, delegated_tx| {
                    run_check_tx_permission(
                        Some(&delegate.permissions),
                        u16::from(delegated_tx.tx.get_txn_type()),
                    )
                },
            )
        },
        || check_ledger_signer_authorization(view, &adapted, flags),
        calculate_base_fee,
        |base_fee| {
            run_transactor_check_fee(
                flags,
                view.open(),
                &adapted,
                base_fee,
                0_i64,
                |_| true,
                |base_fee| base_fee,
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
