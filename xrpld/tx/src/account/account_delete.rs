//! Deterministic the reference implementation preflight-side shells.
//!
//! This ports the exact current behavior around:
//!
//! - `checkExtraFeatures(...)` credentials gating, and
//! - `preflight(...)` destination/self rejection plus credentials-field checks,
//! - the front `preclaim(...)` destination/auth/account checks,
//! - the next loaded-account NFT/sequence preclaim guards,
//! - the owner-directory scan/counting preclaim shell,
//! - and the first `doApply()` front control-flow shell.

use protocol::{NotTec, Ter, is_tes_success};
use std::ops::{Add, Sub};

pub const ACCOUNT_DELETE_REQUIRE_DEST_TAG_FLAG: u32 = 0x0002_0000;
pub const ACCOUNT_DELETE_DEPOSIT_AUTH_FLAG: u32 = 0x0100_0000;
pub const ACCOUNT_DELETE_PRECLAIM_SEQUENCE_DELTA: u32 = 255;
pub const ACCOUNT_DELETE_MAX_DELETABLE_DIR_ENTRIES: i32 = 1000;
pub const ACCOUNT_DELETE_PASSWORD_SPENT_FLAG: u32 = 0x0001_0000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountDeletePreflightFacts<AccountId> {
    pub account: AccountId,
    pub destination: AccountId,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct AccountDeletePreclaimFrontFacts {
    pub destination_exists: bool,
    pub destination_flags: u32,
    pub destination_tag_present: bool,
    pub credential_ids_present: bool,
    pub source_account_exists: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct AccountDeletePreclaimNftAndSequenceFacts {
    pub minted_nftokens: u32,
    pub burned_nftokens: u32,
    pub owned_nft_page_present: bool,
    pub account_sequence: u32,
    pub ledger_sequence: u32,
    pub first_nftoken_sequence: Option<u32>,
    pub owner_dir_empty: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccountDeletePreclaimScanState {
    Return(Ter),
    ContinueToDirectoryScan,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccountDeleteDirectoryEntryDisposition {
    MissingObject,
    Undeletable,
    Deletable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct AccountDeleteDoApplyFrontFacts {
    pub source_exists: bool,
    pub destination_exists: bool,
    pub credential_ids_present: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccountDeleteDoApplyStage {
    Return(Ter),
    ContinueToCleanup,
}

pub trait AccountDeleteDoApplyTailSink {
    type Amount: Default
        + PartialOrd
        + Add<Output = Self::Amount>
        + Sub<Output = Self::Amount>
        + Clone;

    fn source_balance(&mut self) -> Self::Amount;
    fn destination_balance(&mut self) -> Self::Amount;
    fn set_source_balance(&mut self, amount: Self::Amount);
    fn set_destination_balance(&mut self, amount: Self::Amount);
    fn deliver(&mut self, amount: Self::Amount);
    fn owner_dir_exists(&mut self) -> bool;
    fn empty_dir_delete(&mut self) -> bool;
    fn destination_password_spent(&mut self) -> bool;
    fn clear_destination_password_spent(&mut self);
    fn update_destination(&mut self);
    fn erase_source(&mut self);
}

pub fn account_delete_check_extra_features(
    credential_ids_present: bool,
    feature_credentials_enabled: bool,
) -> bool {
    !credential_ids_present || feature_credentials_enabled
}

pub fn run_account_delete_preflight<AccountId: Eq>(
    facts: AccountDeletePreflightFacts<AccountId>,
    check_credentials_fields: impl FnOnce() -> NotTec,
) -> NotTec {
    if facts.account == facts.destination {
        return Ter::TEM_DST_IS_SRC;
    }

    let err = check_credentials_fields();
    if err != Ter::TES_SUCCESS {
        return err;
    }

    Ter::TES_SUCCESS
}

pub fn run_account_delete_preclaim_front(
    facts: AccountDeletePreclaimFrontFacts,
    validate_credentials: impl FnOnce() -> Ter,
    has_deposit_preauth: impl FnOnce() -> bool,
) -> Ter {
    if !facts.destination_exists {
        return Ter::TEC_NO_DST;
    }

    if (facts.destination_flags & ACCOUNT_DELETE_REQUIRE_DEST_TAG_FLAG) != 0
        && !facts.destination_tag_present
    {
        return Ter::TEC_DST_TAG_NEEDED;
    }

    let err = validate_credentials();
    if err != Ter::TES_SUCCESS {
        return err;
    }

    if !facts.credential_ids_present
        && (facts.destination_flags & ACCOUNT_DELETE_DEPOSIT_AUTH_FLAG) != 0
        && !has_deposit_preauth()
    {
        return Ter::TEC_NO_PERMISSION;
    }

    if !facts.source_account_exists {
        return Ter::TER_NO_ACCOUNT;
    }

    Ter::TES_SUCCESS
}

pub fn run_account_delete_preclaim_nft_and_sequence(
    facts: AccountDeletePreclaimNftAndSequenceFacts,
) -> AccountDeletePreclaimScanState {
    if facts.minted_nftokens != facts.burned_nftokens {
        return AccountDeletePreclaimScanState::Return(Ter::TEC_HAS_OBLIGATIONS);
    }

    if facts.owned_nft_page_present {
        return AccountDeletePreclaimScanState::Return(Ter::TEC_HAS_OBLIGATIONS);
    }

    if facts
        .account_sequence
        .wrapping_add(ACCOUNT_DELETE_PRECLAIM_SEQUENCE_DELTA)
        > facts.ledger_sequence
    {
        return AccountDeletePreclaimScanState::Return(Ter::TEC_TOO_SOON);
    }

    if facts
        .first_nftoken_sequence
        .unwrap_or(0)
        .wrapping_add(facts.minted_nftokens)
        .wrapping_add(ACCOUNT_DELETE_PRECLAIM_SEQUENCE_DELTA)
        > facts.ledger_sequence
    {
        return AccountDeletePreclaimScanState::Return(Ter::TEC_TOO_SOON);
    }

    if facts.owner_dir_empty {
        return AccountDeletePreclaimScanState::Return(Ter::TES_SUCCESS);
    }

    AccountDeletePreclaimScanState::ContinueToDirectoryScan
}

pub fn run_account_delete_preclaim_directory_scan(
    cdir_first_found: bool,
    entries: &[AccountDeleteDirectoryEntryDisposition],
) -> Ter {
    if !cdir_first_found {
        return Ter::TES_SUCCESS;
    }

    let mut deletable_dir_entry_count = 0_i32;

    for entry in entries {
        match entry {
            AccountDeleteDirectoryEntryDisposition::MissingObject => {
                return Ter::TEF_BAD_LEDGER;
            }
            AccountDeleteDirectoryEntryDisposition::Undeletable => {
                return Ter::TEC_HAS_OBLIGATIONS;
            }
            AccountDeleteDirectoryEntryDisposition::Deletable => {
                deletable_dir_entry_count += 1;
                if deletable_dir_entry_count > ACCOUNT_DELETE_MAX_DELETABLE_DIR_ENTRIES {
                    return Ter::TEF_TOO_BIG;
                }
            }
        }
    }

    Ter::TES_SUCCESS
}

pub fn run_account_delete_do_apply_front(
    facts: AccountDeleteDoApplyFrontFacts,
    verify_deposit_preauth: impl FnOnce() -> Ter,
) -> AccountDeleteDoApplyStage {
    if !facts.source_exists || !facts.destination_exists {
        return AccountDeleteDoApplyStage::Return(Ter::TEF_BAD_LEDGER);
    }

    if facts.credential_ids_present {
        let err = verify_deposit_preauth();
        if err != Ter::TES_SUCCESS {
            return AccountDeleteDoApplyStage::Return(err);
        }
    }

    AccountDeleteDoApplyStage::ContinueToCleanup
}

pub fn run_account_delete_cleanup_callback(deleter_result: Option<Ter>) -> Ter {
    deleter_result.unwrap_or(Ter::TEC_HAS_OBLIGATIONS)
}

pub fn run_account_delete_do_apply<Sink, VerifyDepositPreauth, CleanupOnAccountDelete>(
    facts: AccountDeleteDoApplyFrontFacts,
    verify_deposit_preauth: VerifyDepositPreauth,
    cleanup_on_account_delete: CleanupOnAccountDelete,
    sink: &mut Sink,
) -> Ter
where
    Sink: AccountDeleteDoApplyTailSink,
    VerifyDepositPreauth: FnOnce() -> Ter,
    CleanupOnAccountDelete: FnOnce() -> Ter,
{
    match run_account_delete_do_apply_front(facts, verify_deposit_preauth) {
        AccountDeleteDoApplyStage::Return(err) => return err,
        AccountDeleteDoApplyStage::ContinueToCleanup => {}
    }

    let cleanup_result = cleanup_on_account_delete();
    if !is_tes_success(cleanup_result) {
        return cleanup_result;
    }

    run_account_delete_do_apply_tail(sink)
}

pub fn run_account_delete_do_apply_tail<Sink>(sink: &mut Sink) -> Ter
where
    Sink: AccountDeleteDoApplyTailSink,
{
    let remaining_balance = sink.source_balance();
    let destination_balance = sink.destination_balance();

    sink.set_destination_balance(destination_balance.clone() + remaining_balance.clone());
    sink.set_source_balance(Sink::Amount::default());
    sink.deliver(remaining_balance.clone());

    if sink.owner_dir_exists() && !sink.empty_dir_delete() {
        return Ter::TEC_HAS_OBLIGATIONS;
    }

    if remaining_balance > Sink::Amount::default() && sink.destination_password_spent() {
        sink.clear_destination_password_spent();
    }

    sink.update_destination();
    sink.erase_source();

    Ter::TES_SUCCESS
}

#[cfg(test)]
mod tests {
    use protocol::trans_token;
    use std::{
        cell::{Cell, RefCell},
        rc::Rc,
    };

    use super::{
        ACCOUNT_DELETE_DEPOSIT_AUTH_FLAG, ACCOUNT_DELETE_MAX_DELETABLE_DIR_ENTRIES,
        ACCOUNT_DELETE_PASSWORD_SPENT_FLAG, ACCOUNT_DELETE_PRECLAIM_SEQUENCE_DELTA,
        ACCOUNT_DELETE_REQUIRE_DEST_TAG_FLAG, AccountDeleteDirectoryEntryDisposition,
        AccountDeleteDoApplyFrontFacts, AccountDeleteDoApplyStage, AccountDeleteDoApplyTailSink,
        AccountDeletePreclaimFrontFacts, AccountDeletePreclaimNftAndSequenceFacts,
        AccountDeletePreclaimScanState, AccountDeletePreflightFacts,
        account_delete_check_extra_features, run_account_delete_cleanup_callback,
        run_account_delete_do_apply, run_account_delete_do_apply_front,
        run_account_delete_do_apply_tail, run_account_delete_preclaim_directory_scan,
        run_account_delete_preclaim_front, run_account_delete_preclaim_nft_and_sequence,
        run_account_delete_preflight,
    };

    #[test]
    fn account_delete_check_extra_features_credentials_gate() {
        assert!(account_delete_check_extra_features(false, false));
        assert!(account_delete_check_extra_features(false, true));
        assert!(!account_delete_check_extra_features(true, false));
        assert!(account_delete_check_extra_features(true, true));
    }

    #[test]
    fn account_delete_preflight_rejects_self_destination_before_credentials() {
        let result = run_account_delete_preflight(
            AccountDeletePreflightFacts {
                account: "alice",
                destination: "alice",
            },
            || panic!("self-destination should short-circuit before credentials"),
        );

        assert_eq!(result, protocol::Ter::TEM_DST_IS_SRC);
        assert_eq!(trans_token(result), "temDST_IS_SRC");
    }

    #[test]
    fn account_delete_preflight_passes_through_credentials_result() {
        let error = run_account_delete_preflight(
            AccountDeletePreflightFacts {
                account: "alice",
                destination: "bob",
            },
            || protocol::Ter::TEM_BAD_AMOUNT,
        );
        let success = run_account_delete_preflight(
            AccountDeletePreflightFacts {
                account: "alice",
                destination: "bob",
            },
            || protocol::Ter::TES_SUCCESS,
        );

        assert_eq!(error, protocol::Ter::TEM_BAD_AMOUNT);
        assert_eq!(success, protocol::Ter::TES_SUCCESS);
    }

    #[test]
    fn account_delete_preclaim_front_rejects_missing_destination_before_callbacks() {
        let checked_credentials = Cell::new(false);
        let checked_preauth = Cell::new(false);

        let result = run_account_delete_preclaim_front(
            AccountDeletePreclaimFrontFacts {
                source_account_exists: true,
                ..AccountDeletePreclaimFrontFacts::default()
            },
            || {
                checked_credentials.set(true);
                protocol::Ter::TES_SUCCESS
            },
            || {
                checked_preauth.set(true);
                true
            },
        );

        assert_eq!(result, protocol::Ter::TEC_NO_DST);
        assert_eq!(trans_token(result), "tecNO_DST");
        assert!(!checked_credentials.get());
        assert!(!checked_preauth.get());
    }

    #[test]
    fn account_delete_preclaim_front_rejects_missing_destination_tag_before_credentials() {
        let checked_credentials = Cell::new(false);

        let result = run_account_delete_preclaim_front(
            AccountDeletePreclaimFrontFacts {
                source_account_exists: true,
                destination_exists: true,
                destination_flags: ACCOUNT_DELETE_REQUIRE_DEST_TAG_FLAG,
                ..AccountDeletePreclaimFrontFacts::default()
            },
            || {
                checked_credentials.set(true);
                protocol::Ter::TES_SUCCESS
            },
            || true,
        );

        assert_eq!(result, protocol::Ter::TEC_DST_TAG_NEEDED);
        assert_eq!(trans_token(result), "tecDST_TAG_NEEDED");
        assert!(!checked_credentials.get());
    }

    #[test]
    fn account_delete_preclaim_front_passes_through_credentials_failure() {
        let checked_preauth = Cell::new(false);

        let result = run_account_delete_preclaim_front(
            AccountDeletePreclaimFrontFacts {
                destination_exists: true,
                source_account_exists: true,
                ..AccountDeletePreclaimFrontFacts::default()
            },
            || protocol::Ter::TEC_BAD_CREDENTIALS,
            || {
                checked_preauth.set(true);
                true
            },
        );

        assert_eq!(result, protocol::Ter::TEC_BAD_CREDENTIALS);
        assert_eq!(trans_token(result), "tecBAD_CREDENTIALS");
        assert!(!checked_preauth.get());
    }

    #[test]
    fn account_delete_preclaim_front_requires_deposit_preauth_without_credentials() {
        let checked_preauth = Cell::new(false);

        let result = run_account_delete_preclaim_front(
            AccountDeletePreclaimFrontFacts {
                destination_exists: true,
                destination_flags: ACCOUNT_DELETE_DEPOSIT_AUTH_FLAG,
                source_account_exists: true,
                ..AccountDeletePreclaimFrontFacts::default()
            },
            || protocol::Ter::TES_SUCCESS,
            || {
                checked_preauth.set(true);
                false
            },
        );

        assert_eq!(result, protocol::Ter::TEC_NO_PERMISSION);
        assert_eq!(trans_token(result), "tecNO_PERMISSION");
        assert!(checked_preauth.get());
    }

    #[test]
    fn account_delete_preclaim_front_skips_deposit_preauth_when_credentials_present() {
        let checked_preauth = Cell::new(false);

        let result = run_account_delete_preclaim_front(
            AccountDeletePreclaimFrontFacts {
                destination_exists: true,
                destination_flags: ACCOUNT_DELETE_DEPOSIT_AUTH_FLAG,
                credential_ids_present: true,
                source_account_exists: true,
                ..AccountDeletePreclaimFrontFacts::default()
            },
            || protocol::Ter::TES_SUCCESS,
            || {
                checked_preauth.set(true);
                false
            },
        );

        assert_eq!(result, protocol::Ter::TES_SUCCESS);
        assert!(!checked_preauth.get());
    }

    #[test]
    fn account_delete_preclaim_front_requires_source_account_after_front_checks() {
        // credentials, and deposit-preauth checks — matching AccountDelete::preclaim.
        let result = run_account_delete_preclaim_front(
            AccountDeletePreclaimFrontFacts {
                destination_exists: true,
                source_account_exists: false,
                ..AccountDeletePreclaimFrontFacts::default()
            },
            || protocol::Ter::TES_SUCCESS,
            || true,
        );

        assert_eq!(result, protocol::Ter::TER_NO_ACCOUNT);
        assert_eq!(trans_token(result), "terNO_ACCOUNT");
    }

    #[test]
    fn account_delete_preclaim_nft_and_sequence_rejects_issued_nfts() {
        let result = run_account_delete_preclaim_nft_and_sequence(
            AccountDeletePreclaimNftAndSequenceFacts {
                minted_nftokens: 2,
                burned_nftokens: 1,
                ..AccountDeletePreclaimNftAndSequenceFacts::default()
            },
        );

        assert_eq!(
            result,
            AccountDeletePreclaimScanState::Return(protocol::Ter::TEC_HAS_OBLIGATIONS)
        );
    }

    #[test]
    fn account_delete_preclaim_nft_and_sequence_rejects_owned_nft_page() {
        let result = run_account_delete_preclaim_nft_and_sequence(
            AccountDeletePreclaimNftAndSequenceFacts {
                owned_nft_page_present: true,
                ..AccountDeletePreclaimNftAndSequenceFacts::default()
            },
        );

        assert_eq!(
            result,
            AccountDeletePreclaimScanState::Return(protocol::Ter::TEC_HAS_OBLIGATIONS)
        );
    }

    #[test]
    fn account_delete_preclaim_nft_and_sequence_rejects_recent_account_sequence() {
        let result = run_account_delete_preclaim_nft_and_sequence(
            AccountDeletePreclaimNftAndSequenceFacts {
                account_sequence: 100,
                ledger_sequence: 100 + ACCOUNT_DELETE_PRECLAIM_SEQUENCE_DELTA - 1,
                ..AccountDeletePreclaimNftAndSequenceFacts::default()
            },
        );

        assert_eq!(
            result,
            AccountDeletePreclaimScanState::Return(protocol::Ter::TEC_TOO_SOON)
        );
    }

    #[test]
    fn account_delete_preclaim_nft_and_sequence_rejects_recent_nft_sequence() {
        let result = run_account_delete_preclaim_nft_and_sequence(
            AccountDeletePreclaimNftAndSequenceFacts {
                minted_nftokens: 4,
                burned_nftokens: 4,
                first_nftoken_sequence: Some(100),
                ledger_sequence: 100 + 4 + ACCOUNT_DELETE_PRECLAIM_SEQUENCE_DELTA - 1,
                ..AccountDeletePreclaimNftAndSequenceFacts::default()
            },
        );

        assert_eq!(
            result,
            AccountDeletePreclaimScanState::Return(protocol::Ter::TEC_TOO_SOON)
        );
    }

    #[test]
    fn account_delete_preclaim_nft_and_sequence_returns_success_when_owner_dir_is_empty() {
        let result = run_account_delete_preclaim_nft_and_sequence(
            AccountDeletePreclaimNftAndSequenceFacts {
                account_sequence: 100,
                ledger_sequence: 100 + 2 + ACCOUNT_DELETE_PRECLAIM_SEQUENCE_DELTA,
                minted_nftokens: 2,
                burned_nftokens: 2,
                first_nftoken_sequence: Some(100),
                owner_dir_empty: true,
                ..AccountDeletePreclaimNftAndSequenceFacts::default()
            },
        );

        assert_eq!(
            result,
            AccountDeletePreclaimScanState::Return(protocol::Ter::TES_SUCCESS)
        );
    }

    #[test]
    fn account_delete_preclaim_nft_and_sequence_continues_to_directory_scan() {
        let result = run_account_delete_preclaim_nft_and_sequence(
            AccountDeletePreclaimNftAndSequenceFacts {
                account_sequence: 100,
                ledger_sequence: 100 + ACCOUNT_DELETE_PRECLAIM_SEQUENCE_DELTA + 10,
                minted_nftokens: 2,
                burned_nftokens: 2,
                first_nftoken_sequence: Some(100),
                owner_dir_empty: false,
                ..AccountDeletePreclaimNftAndSequenceFacts::default()
            },
        );

        assert_eq!(
            result,
            AccountDeletePreclaimScanState::ContinueToDirectoryScan
        );
    }

    #[test]
    fn account_delete_preclaim_directory_scan_returns_success_when_cdir_first_fails() {
        let result = run_account_delete_preclaim_directory_scan(false, &[]);

        assert_eq!(result, protocol::Ter::TES_SUCCESS);
    }

    #[test]
    fn account_delete_preclaim_directory_scan_rejects_missing_child() {
        let result = run_account_delete_preclaim_directory_scan(
            true,
            &[AccountDeleteDirectoryEntryDisposition::MissingObject],
        );

        assert_eq!(result, protocol::Ter::TEF_BAD_LEDGER);
    }

    #[test]
    fn account_delete_preclaim_directory_scan_rejects_undeletable_entry() {
        let result = run_account_delete_preclaim_directory_scan(
            true,
            &[AccountDeleteDirectoryEntryDisposition::Undeletable],
        );

        assert_eq!(result, protocol::Ter::TEC_HAS_OBLIGATIONS);
    }

    #[test]
    fn account_delete_preclaim_directory_scan_rejects_after_too_many_deletable_entries() {
        let entries = vec![
            AccountDeleteDirectoryEntryDisposition::Deletable;
            (ACCOUNT_DELETE_MAX_DELETABLE_DIR_ENTRIES as usize) + 1
        ];

        let result = run_account_delete_preclaim_directory_scan(true, &entries);

        assert_eq!(result, protocol::Ter::TEF_TOO_BIG);
    }

    #[test]
    fn account_delete_preclaim_directory_scan_accepts_deletable_entries_within_limit() {
        let entries = vec![
            AccountDeleteDirectoryEntryDisposition::Deletable;
            ACCOUNT_DELETE_MAX_DELETABLE_DIR_ENTRIES as usize
        ];

        let result = run_account_delete_preclaim_directory_scan(true, &entries);

        assert_eq!(result, protocol::Ter::TES_SUCCESS);
    }

    #[test]
    fn account_delete_do_apply_front_rejects_missing_loaded_accounts() {
        let called = Cell::new(false);

        let src_missing = run_account_delete_do_apply_front(
            AccountDeleteDoApplyFrontFacts {
                source_exists: false,
                destination_exists: true,
                ..AccountDeleteDoApplyFrontFacts::default()
            },
            || {
                called.set(true);
                protocol::Ter::TES_SUCCESS
            },
        );
        let dst_missing = run_account_delete_do_apply_front(
            AccountDeleteDoApplyFrontFacts {
                source_exists: true,
                destination_exists: false,
                ..AccountDeleteDoApplyFrontFacts::default()
            },
            || {
                called.set(true);
                protocol::Ter::TES_SUCCESS
            },
        );

        assert_eq!(
            src_missing,
            AccountDeleteDoApplyStage::Return(protocol::Ter::TEF_BAD_LEDGER)
        );
        assert_eq!(
            dst_missing,
            AccountDeleteDoApplyStage::Return(protocol::Ter::TEF_BAD_LEDGER)
        );
        assert!(!called.get());
    }

    #[test]
    fn account_delete_do_apply_front_skips_verify_without_credentials() {
        let called = Cell::new(false);

        let result = run_account_delete_do_apply_front(
            AccountDeleteDoApplyFrontFacts {
                source_exists: true,
                destination_exists: true,
                credential_ids_present: false,
            },
            || {
                called.set(true);
                protocol::Ter::TES_SUCCESS
            },
        );

        assert_eq!(result, AccountDeleteDoApplyStage::ContinueToCleanup);
        assert!(!called.get());
    }

    #[test]
    fn account_delete_do_apply_front_passes_through_verify_result() {
        let failure = run_account_delete_do_apply_front(
            AccountDeleteDoApplyFrontFacts {
                source_exists: true,
                destination_exists: true,
                credential_ids_present: true,
            },
            || protocol::Ter::TEC_NO_PERMISSION,
        );
        let success = run_account_delete_do_apply_front(
            AccountDeleteDoApplyFrontFacts {
                source_exists: true,
                destination_exists: true,
                credential_ids_present: true,
            },
            || protocol::Ter::TES_SUCCESS,
        );

        assert_eq!(
            failure,
            AccountDeleteDoApplyStage::Return(protocol::Ter::TEC_NO_PERMISSION)
        );
        assert_eq!(success, AccountDeleteDoApplyStage::ContinueToCleanup);
    }

    #[test]
    fn account_delete_cleanup_callback_deleter_dispatch() {
        let deletable = run_account_delete_cleanup_callback(Some(protocol::Ter::TEF_BAD_LEDGER));
        let undeletable = run_account_delete_cleanup_callback(None);

        assert_eq!(deletable, protocol::Ter::TEF_BAD_LEDGER);
        assert_eq!(undeletable, protocol::Ter::TEC_HAS_OBLIGATIONS);
    }

    struct TestDoApplyTailSink {
        source_balance: i64,
        destination_balance: i64,
        destination_flags: u32,
        owner_dir_exists: bool,
        empty_dir_delete_result: bool,
        delivered: Vec<i64>,
        steps: Rc<RefCell<Vec<String>>>,
    }

    impl AccountDeleteDoApplyTailSink for TestDoApplyTailSink {
        type Amount = i64;

        fn source_balance(&mut self) -> Self::Amount {
            self.steps.borrow_mut().push("source_balance".to_string());
            self.source_balance
        }

        fn destination_balance(&mut self) -> Self::Amount {
            self.steps
                .borrow_mut()
                .push("destination_balance".to_string());
            self.destination_balance
        }

        fn set_source_balance(&mut self, amount: Self::Amount) {
            self.steps
                .borrow_mut()
                .push(format!("set_source_balance={amount}"));
            self.source_balance = amount;
        }

        fn set_destination_balance(&mut self, amount: Self::Amount) {
            self.steps
                .borrow_mut()
                .push(format!("set_destination_balance={amount}"));
            self.destination_balance = amount;
        }

        fn deliver(&mut self, amount: Self::Amount) {
            self.steps.borrow_mut().push(format!("deliver={amount}"));
            self.delivered.push(amount);
        }

        fn owner_dir_exists(&mut self) -> bool {
            self.steps.borrow_mut().push("owner_dir_exists".to_string());
            self.owner_dir_exists
        }

        fn empty_dir_delete(&mut self) -> bool {
            self.steps.borrow_mut().push("empty_dir_delete".to_string());
            self.empty_dir_delete_result
        }

        fn destination_password_spent(&mut self) -> bool {
            self.steps
                .borrow_mut()
                .push("destination_password_spent".to_string());
            (self.destination_flags & ACCOUNT_DELETE_PASSWORD_SPENT_FLAG) != 0
        }

        fn clear_destination_password_spent(&mut self) {
            self.steps
                .borrow_mut()
                .push("clear_destination_password_spent".to_string());
            self.destination_flags &= !ACCOUNT_DELETE_PASSWORD_SPENT_FLAG;
        }

        fn update_destination(&mut self) {
            self.steps
                .borrow_mut()
                .push("update_destination".to_string());
        }

        fn erase_source(&mut self) {
            self.steps.borrow_mut().push("erase_source".to_string());
        }
    }

    #[test]
    fn account_delete_do_apply_tail_transfers_balance_and_finishes() {
        let steps = Rc::new(RefCell::new(Vec::new()));
        let mut sink = TestDoApplyTailSink {
            source_balance: 30,
            destination_balance: 12,
            destination_flags: 0,
            owner_dir_exists: false,
            empty_dir_delete_result: true,
            delivered: Vec::new(),
            steps: steps.clone(),
        };

        let result = run_account_delete_do_apply_tail(&mut sink);

        assert_eq!(result, protocol::Ter::TES_SUCCESS);
        assert_eq!(sink.source_balance, 0);
        assert_eq!(sink.destination_balance, 42);
        assert_eq!(sink.delivered, vec![30]);
        assert_eq!(
            steps.borrow().as_slice(),
            [
                "source_balance",
                "destination_balance",
                "set_destination_balance=42",
                "set_source_balance=0",
                "deliver=30",
                "owner_dir_exists",
                "destination_password_spent",
                "update_destination",
                "erase_source",
            ]
        );
    }

    #[test]
    fn account_delete_do_apply_tail_checks_root_dir_after_transfer() {
        let steps = Rc::new(RefCell::new(Vec::new()));
        let mut sink = TestDoApplyTailSink {
            source_balance: 30,
            destination_balance: 12,
            destination_flags: ACCOUNT_DELETE_PASSWORD_SPENT_FLAG,
            owner_dir_exists: true,
            empty_dir_delete_result: false,
            delivered: Vec::new(),
            steps: steps.clone(),
        };

        let result = run_account_delete_do_apply_tail(&mut sink);

        assert_eq!(result, protocol::Ter::TEC_HAS_OBLIGATIONS);
        assert_eq!(sink.source_balance, 0);
        assert_eq!(sink.destination_balance, 42);
        assert_eq!(sink.destination_flags, ACCOUNT_DELETE_PASSWORD_SPENT_FLAG);
        assert_eq!(sink.delivered, vec![30]);
        assert_eq!(
            steps.borrow().as_slice(),
            [
                "source_balance",
                "destination_balance",
                "set_destination_balance=42",
                "set_source_balance=0",
                "deliver=30",
                "owner_dir_exists",
                "empty_dir_delete",
            ]
        );
    }

    #[test]
    fn account_delete_do_apply_tail_clears_password_spent_only_after_positive_delivery() {
        let steps = Rc::new(RefCell::new(Vec::new()));
        let mut sink = TestDoApplyTailSink {
            source_balance: 30,
            destination_balance: 12,
            destination_flags: ACCOUNT_DELETE_PASSWORD_SPENT_FLAG,
            owner_dir_exists: true,
            empty_dir_delete_result: true,
            delivered: Vec::new(),
            steps: steps.clone(),
        };

        let result = run_account_delete_do_apply_tail(&mut sink);

        assert_eq!(result, protocol::Ter::TES_SUCCESS);
        assert_eq!(sink.destination_flags, 0);
        assert_eq!(
            steps.borrow().as_slice(),
            [
                "source_balance",
                "destination_balance",
                "set_destination_balance=42",
                "set_source_balance=0",
                "deliver=30",
                "owner_dir_exists",
                "empty_dir_delete",
                "destination_password_spent",
                "clear_destination_password_spent",
                "update_destination",
                "erase_source",
            ]
        );
    }

    #[test]
    fn account_delete_do_apply_tail_skips_password_rearm_when_no_xrp_moves() {
        let steps = Rc::new(RefCell::new(Vec::new()));
        let mut sink = TestDoApplyTailSink {
            source_balance: 0,
            destination_balance: 12,
            destination_flags: ACCOUNT_DELETE_PASSWORD_SPENT_FLAG,
            owner_dir_exists: false,
            empty_dir_delete_result: true,
            delivered: Vec::new(),
            steps: steps.clone(),
        };

        let result = run_account_delete_do_apply_tail(&mut sink);

        assert_eq!(result, protocol::Ter::TES_SUCCESS);
        assert_eq!(sink.destination_flags, ACCOUNT_DELETE_PASSWORD_SPENT_FLAG);
        assert_eq!(sink.delivered, vec![0]);
        assert_eq!(
            steps.borrow().as_slice(),
            [
                "source_balance",
                "destination_balance",
                "set_destination_balance=12",
                "set_source_balance=0",
                "deliver=0",
                "owner_dir_exists",
                "update_destination",
                "erase_source",
            ]
        );
    }

    #[test]
    fn account_delete_do_apply_runs_current_cpp_stage_order() {
        let steps = Rc::new(RefCell::new(Vec::new()));
        let mut sink = TestDoApplyTailSink {
            source_balance: 30,
            destination_balance: 12,
            destination_flags: 0,
            owner_dir_exists: false,
            empty_dir_delete_result: true,
            delivered: Vec::new(),
            steps: steps.clone(),
        };

        let result = run_account_delete_do_apply(
            AccountDeleteDoApplyFrontFacts {
                source_exists: true,
                destination_exists: true,
                credential_ids_present: true,
            },
            {
                let steps = steps.clone();
                move || {
                    steps.borrow_mut().push("verify".to_string());
                    protocol::Ter::TES_SUCCESS
                }
            },
            {
                let steps = steps.clone();
                move || {
                    steps.borrow_mut().push("cleanup".to_string());
                    protocol::Ter::TES_SUCCESS
                }
            },
            &mut sink,
        );

        assert_eq!(result, protocol::Ter::TES_SUCCESS);
        assert_eq!(
            steps.borrow().as_slice(),
            [
                "verify",
                "cleanup",
                "source_balance",
                "destination_balance",
                "set_destination_balance=42",
                "set_source_balance=0",
                "deliver=30",
                "owner_dir_exists",
                "destination_password_spent",
                "update_destination",
                "erase_source",
            ]
        );
    }

    #[test]
    fn account_delete_do_apply_returns_front_failure_before_cleanup_and_tail() {
        let verify_called = Cell::new(false);
        let cleanup_called = Cell::new(false);
        let steps = Rc::new(RefCell::new(Vec::new()));
        let mut sink = TestDoApplyTailSink {
            source_balance: 30,
            destination_balance: 12,
            destination_flags: 0,
            owner_dir_exists: false,
            empty_dir_delete_result: true,
            delivered: Vec::new(),
            steps: steps.clone(),
        };

        let result = run_account_delete_do_apply(
            AccountDeleteDoApplyFrontFacts {
                source_exists: false,
                destination_exists: true,
                credential_ids_present: true,
            },
            || {
                verify_called.set(true);
                protocol::Ter::TES_SUCCESS
            },
            || {
                cleanup_called.set(true);
                protocol::Ter::TES_SUCCESS
            },
            &mut sink,
        );

        assert_eq!(result, protocol::Ter::TEF_BAD_LEDGER);
        assert!(!verify_called.get());
        assert!(!cleanup_called.get());
        assert!(steps.borrow().is_empty());
    }

    #[test]
    fn account_delete_do_apply_returns_cleanup_failure_before_tail() {
        let steps = Rc::new(RefCell::new(Vec::new()));
        let mut sink = TestDoApplyTailSink {
            source_balance: 30,
            destination_balance: 12,
            destination_flags: 0,
            owner_dir_exists: false,
            empty_dir_delete_result: true,
            delivered: Vec::new(),
            steps: steps.clone(),
        };

        let result = run_account_delete_do_apply(
            AccountDeleteDoApplyFrontFacts {
                source_exists: true,
                destination_exists: true,
                credential_ids_present: true,
            },
            {
                let steps = steps.clone();
                move || {
                    steps.borrow_mut().push("verify".to_string());
                    protocol::Ter::TES_SUCCESS
                }
            },
            {
                let steps = steps.clone();
                move || {
                    steps.borrow_mut().push("cleanup".to_string());
                    protocol::Ter::TEC_HAS_OBLIGATIONS
                }
            },
            &mut sink,
        );

        assert_eq!(result, protocol::Ter::TEC_HAS_OBLIGATIONS);
        assert_eq!(steps.borrow().as_slice(), ["verify", "cleanup"]);
    }

    #[test]
    fn account_delete_do_apply_returns_tail_failure_after_successful_cleanup() {
        let steps = Rc::new(RefCell::new(Vec::new()));
        let mut sink = TestDoApplyTailSink {
            source_balance: 30,
            destination_balance: 12,
            destination_flags: 0,
            owner_dir_exists: true,
            empty_dir_delete_result: false,
            delivered: Vec::new(),
            steps: steps.clone(),
        };

        let result = run_account_delete_do_apply(
            AccountDeleteDoApplyFrontFacts {
                source_exists: true,
                destination_exists: true,
                credential_ids_present: false,
            },
            || panic!("verifyDepositPreauth should not run without credentials"),
            {
                let steps = steps.clone();
                move || {
                    steps.borrow_mut().push("cleanup".to_string());
                    protocol::Ter::TES_SUCCESS
                }
            },
            &mut sink,
        );

        assert_eq!(result, protocol::Ter::TEC_HAS_OBLIGATIONS);
        assert_eq!(
            steps.borrow().as_slice(),
            [
                "cleanup",
                "source_balance",
                "destination_balance",
                "set_destination_balance=42",
                "set_source_balance=0",
                "deliver=30",
                "owner_dir_exists",
                "empty_dir_delete",
            ]
        );
    }
}
