use std::cell::Cell;
use std::{cell::RefCell, rc::Rc};

use protocol::{Ter, trans_token};
use tx::{
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
    let called = Cell::new(false);
    let result = run_account_delete_preflight(
        AccountDeletePreflightFacts {
            account: "alice",
            destination: "alice",
        },
        || {
            called.set(true);
            Ter::TES_SUCCESS
        },
    );

    assert_eq!(result, Ter::TEM_DST_IS_SRC);
    assert_eq!(trans_token(result), "temDST_IS_SRC");
    assert!(!called.get());
}

#[test]
fn account_delete_preflight_passes_through_credentials_failure() {
    let result = run_account_delete_preflight(
        AccountDeletePreflightFacts {
            account: "alice",
            destination: "bob",
        },
        || Ter::TEM_BAD_AMOUNT,
    );

    assert_eq!(result, Ter::TEM_BAD_AMOUNT);
    assert_eq!(trans_token(result), "temBAD_AMOUNT");
}

#[test]
fn account_delete_preflight_accepts_distinct_accounts_after_credentials() {
    let called = Cell::new(false);
    let result = run_account_delete_preflight(
        AccountDeletePreflightFacts {
            account: "alice",
            destination: "bob",
        },
        || {
            called.set(true);
            Ter::TES_SUCCESS
        },
    );

    assert_eq!(result, Ter::TES_SUCCESS);
    assert!(called.get());
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
            Ter::TES_SUCCESS
        },
        || {
            checked_preauth.set(true);
            true
        },
    );

    assert_eq!(result, Ter::TEC_NO_DST);
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
            Ter::TES_SUCCESS
        },
        || true,
    );

    assert_eq!(result, Ter::TEC_DST_TAG_NEEDED);
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
        || Ter::TEC_BAD_CREDENTIALS,
        || {
            checked_preauth.set(true);
            true
        },
    );

    assert_eq!(result, Ter::TEC_BAD_CREDENTIALS);
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
        || Ter::TES_SUCCESS,
        || {
            checked_preauth.set(true);
            false
        },
    );

    assert_eq!(result, Ter::TEC_NO_PERMISSION);
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
        || Ter::TES_SUCCESS,
        || {
            checked_preauth.set(true);
            false
        },
    );

    assert_eq!(result, Ter::TES_SUCCESS);
    assert!(!checked_preauth.get());
}

#[test]
fn account_delete_preclaim_front_requires_source_account_after_front_checks() {
    // C++ parity: source account check comes AFTER destination, dest-tag,
    // credentials, and deposit-preauth checks — matching AccountDelete::preclaim.
    // All earlier checks pass (destination present, credentials succeed, preauth ok),
    // so the missing source account is what triggers terNO_ACCOUNT.
    let result = run_account_delete_preclaim_front(
        AccountDeletePreclaimFrontFacts {
            destination_exists: true,
            source_account_exists: false,
            ..AccountDeletePreclaimFrontFacts::default()
        },
        || Ter::TES_SUCCESS,
        || true,
    );

    assert_eq!(result, Ter::TER_NO_ACCOUNT);
    assert_eq!(trans_token(result), "terNO_ACCOUNT");
}

#[test]
fn account_delete_preclaim_nft_and_sequence_rejects_issued_nfts() {
    let result =
        run_account_delete_preclaim_nft_and_sequence(AccountDeletePreclaimNftAndSequenceFacts {
            minted_nftokens: 2,
            burned_nftokens: 1,
            ..AccountDeletePreclaimNftAndSequenceFacts::default()
        });

    assert_eq!(
        result,
        AccountDeletePreclaimScanState::Return(Ter::TEC_HAS_OBLIGATIONS)
    );
}

#[test]
fn account_delete_preclaim_nft_and_sequence_rejects_owned_nft_page() {
    let result =
        run_account_delete_preclaim_nft_and_sequence(AccountDeletePreclaimNftAndSequenceFacts {
            owned_nft_page_present: true,
            ..AccountDeletePreclaimNftAndSequenceFacts::default()
        });

    assert_eq!(
        result,
        AccountDeletePreclaimScanState::Return(Ter::TEC_HAS_OBLIGATIONS)
    );
}

#[test]
fn account_delete_preclaim_nft_and_sequence_rejects_recent_account_sequence() {
    let result =
        run_account_delete_preclaim_nft_and_sequence(AccountDeletePreclaimNftAndSequenceFacts {
            account_sequence: 100,
            ledger_sequence: 100 + ACCOUNT_DELETE_PRECLAIM_SEQUENCE_DELTA - 1,
            ..AccountDeletePreclaimNftAndSequenceFacts::default()
        });

    assert_eq!(
        result,
        AccountDeletePreclaimScanState::Return(Ter::TEC_TOO_SOON)
    );
}

#[test]
fn account_delete_preclaim_nft_and_sequence_rejects_recent_nft_sequence() {
    let result =
        run_account_delete_preclaim_nft_and_sequence(AccountDeletePreclaimNftAndSequenceFacts {
            minted_nftokens: 4,
            burned_nftokens: 4,
            first_nftoken_sequence: Some(100),
            ledger_sequence: 100 + 4 + ACCOUNT_DELETE_PRECLAIM_SEQUENCE_DELTA - 1,
            ..AccountDeletePreclaimNftAndSequenceFacts::default()
        });

    assert_eq!(
        result,
        AccountDeletePreclaimScanState::Return(Ter::TEC_TOO_SOON)
    );
}

#[test]
fn account_delete_preclaim_nft_and_sequence_returns_success_when_owner_dir_is_empty() {
    let result =
        run_account_delete_preclaim_nft_and_sequence(AccountDeletePreclaimNftAndSequenceFacts {
            account_sequence: 100,
            ledger_sequence: 100 + 2 + ACCOUNT_DELETE_PRECLAIM_SEQUENCE_DELTA,
            minted_nftokens: 2,
            burned_nftokens: 2,
            first_nftoken_sequence: Some(100),
            owner_dir_empty: true,
            ..AccountDeletePreclaimNftAndSequenceFacts::default()
        });

    assert_eq!(
        result,
        AccountDeletePreclaimScanState::Return(Ter::TES_SUCCESS)
    );
}

#[test]
fn account_delete_preclaim_nft_and_sequence_continues_to_directory_scan() {
    let result =
        run_account_delete_preclaim_nft_and_sequence(AccountDeletePreclaimNftAndSequenceFacts {
            account_sequence: 100,
            ledger_sequence: 100 + ACCOUNT_DELETE_PRECLAIM_SEQUENCE_DELTA + 10,
            minted_nftokens: 2,
            burned_nftokens: 2,
            first_nftoken_sequence: Some(100),
            owner_dir_empty: false,
            ..AccountDeletePreclaimNftAndSequenceFacts::default()
        });

    assert_eq!(
        result,
        AccountDeletePreclaimScanState::ContinueToDirectoryScan
    );
}

#[test]
fn account_delete_preclaim_directory_scan_returns_success_when_cdir_first_fails() {
    let result = run_account_delete_preclaim_directory_scan(false, &[]);

    assert_eq!(result, Ter::TES_SUCCESS);
}

#[test]
fn account_delete_preclaim_directory_scan_rejects_missing_child() {
    let result = run_account_delete_preclaim_directory_scan(
        true,
        &[AccountDeleteDirectoryEntryDisposition::MissingObject],
    );

    assert_eq!(result, Ter::TEF_BAD_LEDGER);
    assert_eq!(trans_token(result), "tefBAD_LEDGER");
}

#[test]
fn account_delete_preclaim_directory_scan_rejects_undeletable_entry() {
    let result = run_account_delete_preclaim_directory_scan(
        true,
        &[AccountDeleteDirectoryEntryDisposition::Undeletable],
    );

    assert_eq!(result, Ter::TEC_HAS_OBLIGATIONS);
    assert_eq!(trans_token(result), "tecHAS_OBLIGATIONS");
}

#[test]
fn account_delete_preclaim_directory_scan_rejects_after_too_many_deletable_entries() {
    let entries = vec![
        AccountDeleteDirectoryEntryDisposition::Deletable;
        (ACCOUNT_DELETE_MAX_DELETABLE_DIR_ENTRIES as usize) + 1
    ];

    let result = run_account_delete_preclaim_directory_scan(true, &entries);

    assert_eq!(result, Ter::TEF_TOO_BIG);
    assert_eq!(trans_token(result), "tefTOO_BIG");
}

#[test]
fn account_delete_preclaim_directory_scan_accepts_deletable_entries_within_limit() {
    let entries = vec![
        AccountDeleteDirectoryEntryDisposition::Deletable;
        ACCOUNT_DELETE_MAX_DELETABLE_DIR_ENTRIES as usize
    ];

    let result = run_account_delete_preclaim_directory_scan(true, &entries);

    assert_eq!(result, Ter::TES_SUCCESS);
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
            Ter::TES_SUCCESS
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
            Ter::TES_SUCCESS
        },
    );

    assert_eq!(
        src_missing,
        AccountDeleteDoApplyStage::Return(Ter::TEF_BAD_LEDGER)
    );
    assert_eq!(
        dst_missing,
        AccountDeleteDoApplyStage::Return(Ter::TEF_BAD_LEDGER)
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
            Ter::TES_SUCCESS
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
        || Ter::TEC_NO_PERMISSION,
    );
    let success = run_account_delete_do_apply_front(
        AccountDeleteDoApplyFrontFacts {
            source_exists: true,
            destination_exists: true,
            credential_ids_present: true,
        },
        || Ter::TES_SUCCESS,
    );

    assert_eq!(
        failure,
        AccountDeleteDoApplyStage::Return(Ter::TEC_NO_PERMISSION)
    );
    assert_eq!(success, AccountDeleteDoApplyStage::ContinueToCleanup);
}

#[test]
fn account_delete_cleanup_callback_deleter_dispatch() {
    let deletable = run_account_delete_cleanup_callback(Some(Ter::TEF_BAD_LEDGER));
    let undeletable = run_account_delete_cleanup_callback(None);

    assert_eq!(deletable, Ter::TEF_BAD_LEDGER);
    assert_eq!(undeletable, Ter::TEC_HAS_OBLIGATIONS);
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

    assert_eq!(result, Ter::TES_SUCCESS);
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

    assert_eq!(result, Ter::TEC_HAS_OBLIGATIONS);
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

    assert_eq!(result, Ter::TES_SUCCESS);
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

    assert_eq!(result, Ter::TES_SUCCESS);
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
                Ter::TES_SUCCESS
            }
        },
        {
            let steps = steps.clone();
            move || {
                steps.borrow_mut().push("cleanup".to_string());
                Ter::TES_SUCCESS
            }
        },
        &mut sink,
    );

    assert_eq!(result, Ter::TES_SUCCESS);
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
            Ter::TES_SUCCESS
        },
        || {
            cleanup_called.set(true);
            Ter::TES_SUCCESS
        },
        &mut sink,
    );

    assert_eq!(result, Ter::TEF_BAD_LEDGER);
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
                Ter::TES_SUCCESS
            }
        },
        {
            let steps = steps.clone();
            move || {
                steps.borrow_mut().push("cleanup".to_string());
                Ter::TEC_HAS_OBLIGATIONS
            }
        },
        &mut sink,
    );

    assert_eq!(result, Ter::TEC_HAS_OBLIGATIONS);
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
                Ter::TES_SUCCESS
            }
        },
        &mut sink,
    );

    assert_eq!(result, Ter::TEC_HAS_OBLIGATIONS);
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
