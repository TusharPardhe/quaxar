//! Integration tests that pin the narrowed Rust `AccountSet.cpp` metadata,
//! preflight, and delegated-permission shells to the current C++ behavior.

use std::{cell::Cell, cell::RefCell, rc::Rc};

use protocol::{SeqProxy, Ter, trans_token};
use tx::account::account_set::{FULLY_CANONICAL_SIGNATURE_FLAG, INNER_BATCH_TRANSACTION_FLAG};
use tx::{
    ACCOUNT_SET_ALLOW_XRP_FLAG, ACCOUNT_SET_DISALLOW_XRP_FLAG, ACCOUNT_SET_OPTIONAL_AUTH_FLAG,
    ACCOUNT_SET_OPTIONAL_DEST_TAG_FLAG, ACCOUNT_SET_REQUIRE_AUTH_FLAG,
    ACCOUNT_SET_REQUIRE_DEST_TAG_FLAG, ASF_ACCOUNT_TXN_ID, ASF_ALLOW_TRUST_LINE_CLAWBACK,
    ASF_ALLOW_TRUST_LINE_LOCKING, ASF_AUTHORIZED_NFTOKEN_MINTER, ASF_DEFAULT_RIPPLE,
    ASF_DEPOSIT_AUTH, ASF_DISABLE_MASTER, ASF_DISALLOW_INCOMING_CHECK,
    ASF_DISALLOW_INCOMING_NFTOKEN_OFFER, ASF_DISALLOW_INCOMING_PAY_CHAN,
    ASF_DISALLOW_INCOMING_TRUSTLINE, ASF_DISALLOW_XRP, ASF_GLOBAL_FREEZE, ASF_NO_FREEZE,
    ASF_REQUIRE_AUTH, ASF_REQUIRE_DEST, AccountSetDoApplyFacts, AccountSetDoApplyFlagFacts,
    AccountSetDoApplySink, AccountSetDoApplyTailFacts, AccountSetFieldMutation,
    AccountSetGranularPermission, AccountSetPermissionTx, AccountSetPreclaimFacts,
    AccountSetPreflightFacts, AccountSetTxnIdAction, ApplyFlags, LSF_ALLOW_TRUST_LINE_CLAWBACK,
    LSF_ALLOW_TRUST_LINE_LOCKING, LSF_DEFAULT_RIPPLE, LSF_DEPOSIT_AUTH, LSF_DISABLE_MASTER,
    LSF_DISALLOW_INCOMING_CHECK, LSF_DISALLOW_INCOMING_NFTOKEN_OFFER,
    LSF_DISALLOW_INCOMING_PAY_CHAN, LSF_DISALLOW_INCOMING_TRUSTLINE, LSF_DISALLOW_XRP,
    LSF_GLOBAL_FREEZE, LSF_NO_FREEZE, LSF_REQUIRE_AUTH, LSF_REQUIRE_DEST_TAG,
    TT_ACCOUNT_SET_PERMISSION, TxConsequencesCategory, get_account_set_flags_mask,
    run_account_set_check_permission, run_account_set_do_apply, run_account_set_do_apply_flags,
    run_account_set_do_apply_tail, run_account_set_make_tx_consequences, run_account_set_preclaim,
    run_account_set_preflight,
};

#[derive(Clone, Default)]
struct StubPermissionTx {
    account_id: &'static str,
    delegate: Option<&'static str>,
    set_flag: u32,
    clear_flag: u32,
    flags: u32,
    email_hash_present: bool,
    wallet_locator_present: bool,
    nftoken_minter_present: bool,
    message_key_present: bool,
    domain_present: bool,
    transfer_rate_present: bool,
    tick_size_present: bool,
}

impl AccountSetPermissionTx for StubPermissionTx {
    type AccountId = &'static str;

    fn account_id(&self) -> Self::AccountId {
        self.account_id
    }

    fn delegate(&self) -> Option<Self::AccountId> {
        self.delegate
    }

    fn set_flag(&self) -> u32 {
        self.set_flag
    }

    fn clear_flag(&self) -> u32 {
        self.clear_flag
    }

    fn flags(&self) -> u32 {
        self.flags
    }

    fn email_hash_present(&self) -> bool {
        self.email_hash_present
    }

    fn wallet_locator_present(&self) -> bool {
        self.wallet_locator_present
    }

    fn nftoken_minter_present(&self) -> bool {
        self.nftoken_minter_present
    }

    fn message_key_present(&self) -> bool {
        self.message_key_present
    }

    fn domain_present(&self) -> bool {
        self.domain_present
    }

    fn transfer_rate_present(&self) -> bool {
        self.transfer_rate_present
    }

    fn tick_size_present(&self) -> bool {
        self.tick_size_present
    }
}

#[derive(Default)]
struct StubDoApplySink {
    steps: Vec<String>,
}

impl AccountSetDoApplySink for StubDoApplySink {
    type AccountId = &'static str;

    fn set_account_txn_id(&mut self) {
        self.steps.push("set_account_txn_id".to_string());
    }

    fn clear_account_txn_id(&mut self) {
        self.steps.push("clear_account_txn_id".to_string());
    }

    fn set_email_hash(&mut self, value: u128) {
        self.steps.push(format!("set_email_hash={value}"));
    }

    fn clear_email_hash(&mut self) {
        self.steps.push("clear_email_hash".to_string());
    }

    fn set_wallet_locator(&mut self, value: Vec<u8>) {
        self.steps.push(format!("set_wallet_locator={value:?}"));
    }

    fn clear_wallet_locator(&mut self) {
        self.steps.push("clear_wallet_locator".to_string());
    }

    fn set_message_key(&mut self, value: Vec<u8>) {
        self.steps.push(format!(
            "set_message_key={}",
            String::from_utf8_lossy(&value)
        ));
    }

    fn clear_message_key(&mut self) {
        self.steps.push("clear_message_key".to_string());
    }

    fn set_domain(&mut self, value: Vec<u8>) {
        self.steps
            .push(format!("set_domain={}", String::from_utf8_lossy(&value)));
    }

    fn clear_domain(&mut self) {
        self.steps.push("clear_domain".to_string());
    }

    fn set_transfer_rate(&mut self, value: u32) {
        self.steps.push(format!("set_transfer_rate={value}"));
    }

    fn clear_transfer_rate(&mut self) {
        self.steps.push("clear_transfer_rate".to_string());
    }

    fn set_tick_size(&mut self, value: u8) {
        self.steps.push(format!("set_tick_size={value}"));
    }

    fn clear_tick_size(&mut self) {
        self.steps.push("clear_tick_size".to_string());
    }

    fn set_nftoken_minter(&mut self, value: Self::AccountId) {
        self.steps.push(format!("set_nftoken_minter={value}"));
    }

    fn clear_nftoken_minter(&mut self) {
        self.steps.push("clear_nftoken_minter".to_string());
    }

    fn set_account_flags(&mut self, value: u32) {
        self.steps.push(format!("set_flags={value}"));
    }

    fn update_account(&mut self) {
        self.steps.push("update_account".to_string());
    }
}

#[test]
fn account_set_make_tx_consequences_marks_only_current_auth_sensitive_updates_as_blockers() {
    for (tx_flags, set_flag, clear_flag) in [
        (ACCOUNT_SET_REQUIRE_AUTH_FLAG, 0, 0),
        (ACCOUNT_SET_OPTIONAL_AUTH_FLAG, 0, 0),
        (0, ASF_REQUIRE_AUTH, 0),
        (0, ASF_DISABLE_MASTER, 0),
        (0, ASF_ACCOUNT_TXN_ID, 0),
        (0, 0, ASF_REQUIRE_AUTH),
        (0, 0, ASF_DISABLE_MASTER),
        (0, 0, ASF_ACCOUNT_TXN_ID),
    ] {
        let consequences = run_account_set_make_tx_consequences(
            17,
            SeqProxy::sequence(4),
            tx_flags,
            set_flag,
            clear_flag,
        );

        assert_eq!(consequences.fee(), 17);
        assert_eq!(consequences.seq_proxy(), SeqProxy::sequence(4));
        assert_eq!(consequences.following_seq(), SeqProxy::sequence(5));
        assert!(consequences.is_blocker());
    }

    let normal = run_account_set_make_tx_consequences(
        17,
        SeqProxy::ticket(8),
        ACCOUNT_SET_REQUIRE_DEST_TAG_FLAG,
        ASF_REQUIRE_DEST,
        ASF_DISALLOW_XRP,
    );

    assert_eq!(normal.seq_proxy(), SeqProxy::ticket(8));
    assert_eq!(normal.following_seq(), SeqProxy::ticket(8));
    assert_eq!(normal.potential_spend(), 0);
    assert_eq!(
        TxConsequencesCategory::Normal,
        TxConsequencesCategory::Normal
    );
    assert!(!normal.is_blocker());
}

#[test]
fn account_set_flags_mask_matches_current_cpp_txflags_and_delegate_metadata() {
    assert_eq!(TT_ACCOUNT_SET_PERMISSION, 4);
    assert_eq!(get_account_set_flags_mask(), 0x3fc0_ffff);
    assert_eq!(get_account_set_flags_mask(), tx::ACCOUNT_SET_FLAGS_MASK);

    assert_eq!(
        get_account_set_flags_mask() & ACCOUNT_SET_REQUIRE_DEST_TAG_FLAG,
        0
    );
    assert_eq!(
        get_account_set_flags_mask() & ACCOUNT_SET_OPTIONAL_DEST_TAG_FLAG,
        0
    );
    assert_eq!(
        get_account_set_flags_mask() & ACCOUNT_SET_REQUIRE_AUTH_FLAG,
        0
    );
    assert_eq!(
        get_account_set_flags_mask() & ACCOUNT_SET_OPTIONAL_AUTH_FLAG,
        0
    );
    assert_eq!(
        get_account_set_flags_mask() & ACCOUNT_SET_DISALLOW_XRP_FLAG,
        0
    );
    assert_eq!(get_account_set_flags_mask() & ACCOUNT_SET_ALLOW_XRP_FLAG, 0);
    assert_eq!(
        get_account_set_flags_mask() & FULLY_CANONICAL_SIGNATURE_FLAG,
        0
    );
    assert_eq!(
        get_account_set_flags_mask() & INNER_BATCH_TRANSACTION_FLAG,
        0
    );
}

#[test]
fn account_set_preflight_rejects_current_flag_conflicts() {
    let same_flag = run_account_set_preflight(AccountSetPreflightFacts {
        set_flag: ASF_REQUIRE_DEST,
        clear_flag: ASF_REQUIRE_DEST,
        ..AccountSetPreflightFacts::default()
    });
    assert_eq!(same_flag, Ter::TEM_INVALID_FLAG);

    for facts in [
        AccountSetPreflightFacts {
            tx_flags: ACCOUNT_SET_REQUIRE_AUTH_FLAG | ACCOUNT_SET_OPTIONAL_AUTH_FLAG,
            ..AccountSetPreflightFacts::default()
        },
        AccountSetPreflightFacts {
            tx_flags: ACCOUNT_SET_REQUIRE_DEST_TAG_FLAG | ACCOUNT_SET_OPTIONAL_DEST_TAG_FLAG,
            ..AccountSetPreflightFacts::default()
        },
        AccountSetPreflightFacts {
            tx_flags: ACCOUNT_SET_DISALLOW_XRP_FLAG | ACCOUNT_SET_ALLOW_XRP_FLAG,
            ..AccountSetPreflightFacts::default()
        },
    ] {
        assert_eq!(run_account_set_preflight(facts), Ter::TEM_INVALID_FLAG);
    }
}

#[test]
fn account_set_preflight_rejects_current_transfer_rate_boundaries() {
    let too_small = run_account_set_preflight(AccountSetPreflightFacts {
        transfer_rate: Some(999_999_999),
        ..AccountSetPreflightFacts::default()
    });
    let too_large = run_account_set_preflight(AccountSetPreflightFacts {
        transfer_rate: Some(2_000_000_001),
        ..AccountSetPreflightFacts::default()
    });
    let zero = run_account_set_preflight(AccountSetPreflightFacts {
        transfer_rate: Some(0),
        ..AccountSetPreflightFacts::default()
    });
    let min_ok = run_account_set_preflight(AccountSetPreflightFacts {
        transfer_rate: Some(1_000_000_000),
        ..AccountSetPreflightFacts::default()
    });
    let max_ok = run_account_set_preflight(AccountSetPreflightFacts {
        transfer_rate: Some(2_000_000_000),
        ..AccountSetPreflightFacts::default()
    });

    assert_eq!(too_small, Ter::TEM_BAD_TRANSFER_RATE);
    assert_eq!(trans_token(too_small), "temBAD_TRANSFER_RATE");
    assert_eq!(too_large, Ter::TEM_BAD_TRANSFER_RATE);
    assert_eq!(zero, Ter::TES_SUCCESS);
    assert_eq!(min_ok, Ter::TES_SUCCESS);
    assert_eq!(max_ok, Ter::TES_SUCCESS);
}

#[test]
fn account_set_preflight_rejects_current_ticksize_domain_and_message_key_failures() {
    let too_small_tick = run_account_set_preflight(AccountSetPreflightFacts {
        tick_size: Some(2),
        ..AccountSetPreflightFacts::default()
    });
    let too_large_tick = run_account_set_preflight(AccountSetPreflightFacts {
        tick_size: Some(16),
        ..AccountSetPreflightFacts::default()
    });
    let bad_key = run_account_set_preflight(AccountSetPreflightFacts {
        message_key_present: true,
        message_key_is_valid: false,
        ..AccountSetPreflightFacts::default()
    });
    let bad_domain = run_account_set_preflight(AccountSetPreflightFacts {
        domain_len: Some(257),
        ..AccountSetPreflightFacts::default()
    });

    assert_eq!(too_small_tick, Ter::TEM_BAD_TICK_SIZE);
    assert_eq!(too_large_tick, Ter::TEM_BAD_TICK_SIZE);
    assert_eq!(trans_token(too_small_tick), "temBAD_TICK_SIZE");
    assert_eq!(bad_key, Ter::TEL_BAD_PUBLIC_KEY);
    assert_eq!(trans_token(bad_key), "telBAD_PUBLIC_KEY");
    assert_eq!(bad_domain, Ter::TEL_BAD_DOMAIN);
    assert_eq!(trans_token(bad_domain), "telBAD_DOMAIN");
}

#[test]
fn account_set_preflight_rejects_inconsistent_authorized_nft_minter_updates() {
    let missing_minter = run_account_set_preflight(AccountSetPreflightFacts {
        set_flag: ASF_AUTHORIZED_NFTOKEN_MINTER,
        ..AccountSetPreflightFacts::default()
    });
    let unexpected_minter = run_account_set_preflight(AccountSetPreflightFacts {
        clear_flag: ASF_AUTHORIZED_NFTOKEN_MINTER,
        nftoken_minter_present: true,
        ..AccountSetPreflightFacts::default()
    });
    let consistent_set = run_account_set_preflight(AccountSetPreflightFacts {
        set_flag: ASF_AUTHORIZED_NFTOKEN_MINTER,
        nftoken_minter_present: true,
        ..AccountSetPreflightFacts::default()
    });
    let consistent_clear = run_account_set_preflight(AccountSetPreflightFacts {
        clear_flag: ASF_AUTHORIZED_NFTOKEN_MINTER,
        ..AccountSetPreflightFacts::default()
    });

    assert_eq!(missing_minter, Ter::TEM_MALFORMED);
    assert_eq!(unexpected_minter, Ter::TEM_MALFORMED);
    assert_eq!(consistent_set, Ter::TES_SUCCESS);
    assert_eq!(consistent_clear, Ter::TES_SUCCESS);
}

#[test]
fn account_set_preclaim_requires_existing_account() {
    let result = run_account_set_preclaim(AccountSetPreclaimFacts::default());

    assert_eq!(result, Ter::TER_NO_ACCOUNT);
    assert_eq!(trans_token(result), "terNO_ACCOUNT");
}

#[test]
fn account_set_preclaim_splits_require_auth_owner_dir_result_by_retry() {
    let tec = run_account_set_preclaim(AccountSetPreclaimFacts {
        tx_flags: ACCOUNT_SET_REQUIRE_AUTH_FLAG,
        account_exists: true,
        owner_dir_empty: false,
        ..AccountSetPreclaimFacts::default()
    });
    let ter = run_account_set_preclaim(AccountSetPreclaimFacts {
        tx_flags: ACCOUNT_SET_REQUIRE_AUTH_FLAG,
        apply_flags: ApplyFlags::RETRY,
        account_exists: true,
        owner_dir_empty: false,
        ..AccountSetPreclaimFacts::default()
    });

    assert_eq!(tec, Ter::TEC_OWNERS);
    assert_eq!(trans_token(tec), "tecOWNERS");
    assert_eq!(ter, Ter::TER_OWNERS);
    assert_eq!(trans_token(ter), "terOWNERS");
}

#[test]
fn account_set_preclaim_skips_require_auth_owner_dir_check_when_already_enabled_or_empty() {
    let already_enabled = run_account_set_preclaim(AccountSetPreclaimFacts {
        tx_flags: ACCOUNT_SET_REQUIRE_AUTH_FLAG,
        account_exists: true,
        account_flags: LSF_REQUIRE_AUTH,
        owner_dir_empty: false,
        ..AccountSetPreclaimFacts::default()
    });
    let empty_dir = run_account_set_preclaim(AccountSetPreclaimFacts {
        set_flag: ASF_REQUIRE_AUTH,
        account_exists: true,
        owner_dir_empty: true,
        ..AccountSetPreclaimFacts::default()
    });

    assert_eq!(already_enabled, Ter::TES_SUCCESS);
    assert_eq!(empty_dir, Ter::TES_SUCCESS);
}

#[test]
fn account_set_preclaim_keeps_clawback_gates_disabled_when_feature_is_off() {
    let set_clawback = run_account_set_preclaim(AccountSetPreclaimFacts {
        set_flag: ASF_ALLOW_TRUST_LINE_CLAWBACK,
        account_exists: true,
        account_flags: LSF_NO_FREEZE,
        owner_dir_empty: false,
        feature_clawback_enabled: false,
        ..AccountSetPreclaimFacts::default()
    });
    let set_no_freeze = run_account_set_preclaim(AccountSetPreclaimFacts {
        set_flag: ASF_NO_FREEZE,
        account_exists: true,
        account_flags: LSF_ALLOW_TRUST_LINE_CLAWBACK,
        owner_dir_empty: false,
        feature_clawback_enabled: false,
        ..AccountSetPreclaimFacts::default()
    });

    assert_eq!(set_clawback, Ter::TES_SUCCESS);
    assert_eq!(set_no_freeze, Ter::TES_SUCCESS);
}

#[test]
fn account_set_preclaim_rejects_clawback_flag_with_nofreeze_or_owner_dir() {
    let no_freeze = run_account_set_preclaim(AccountSetPreclaimFacts {
        set_flag: ASF_ALLOW_TRUST_LINE_CLAWBACK,
        account_exists: true,
        account_flags: LSF_NO_FREEZE,
        owner_dir_empty: true,
        feature_clawback_enabled: true,
        ..AccountSetPreclaimFacts::default()
    });
    let owners = run_account_set_preclaim(AccountSetPreclaimFacts {
        set_flag: ASF_ALLOW_TRUST_LINE_CLAWBACK,
        account_exists: true,
        owner_dir_empty: false,
        feature_clawback_enabled: true,
        ..AccountSetPreclaimFacts::default()
    });

    assert_eq!(no_freeze, Ter::TEC_NO_PERMISSION);
    assert_eq!(trans_token(no_freeze), "tecNO_PERMISSION");
    assert_eq!(owners, Ter::TEC_OWNERS);
    assert_eq!(trans_token(owners), "tecOWNERS");
}

#[test]
fn account_set_preclaim_rejects_nofreeze_when_clawback_is_already_enabled() {
    let result = run_account_set_preclaim(AccountSetPreclaimFacts {
        set_flag: ASF_NO_FREEZE,
        account_exists: true,
        account_flags: LSF_ALLOW_TRUST_LINE_CLAWBACK,
        owner_dir_empty: true,
        feature_clawback_enabled: true,
        ..AccountSetPreclaimFacts::default()
    });

    assert_eq!(result, Ter::TEC_NO_PERMISSION);
    assert_eq!(trans_token(result), "tecNO_PERMISSION");
}

#[test]
fn account_set_do_apply_flags_requires_a_loaded_account() {
    let result = run_account_set_do_apply_flags(AccountSetDoApplyFlagFacts::default());

    assert_eq!(result, Err(Ter::TEF_INTERNAL));
}

#[test]
fn account_set_do_apply_flags_apply_the_legacy_require_and_disallow_toggles() {
    let result = run_account_set_do_apply_flags(AccountSetDoApplyFlagFacts {
        tx_flags: ACCOUNT_SET_REQUIRE_AUTH_FLAG
            | ACCOUNT_SET_REQUIRE_DEST_TAG_FLAG
            | ACCOUNT_SET_DISALLOW_XRP_FLAG,
        account_exists: true,
        ..AccountSetDoApplyFlagFacts::default()
    })
    .unwrap();
    let cleared = run_account_set_do_apply_flags(AccountSetDoApplyFlagFacts {
        tx_flags: ACCOUNT_SET_OPTIONAL_AUTH_FLAG
            | ACCOUNT_SET_OPTIONAL_DEST_TAG_FLAG
            | ACCOUNT_SET_ALLOW_XRP_FLAG,
        account_exists: true,
        account_flags: LSF_REQUIRE_AUTH | LSF_REQUIRE_DEST_TAG | LSF_DISALLOW_XRP,
        ..AccountSetDoApplyFlagFacts::default()
    })
    .unwrap();

    assert_eq!(
        result.account_flags,
        LSF_REQUIRE_AUTH | LSF_REQUIRE_DEST_TAG | LSF_DISALLOW_XRP
    );
    assert_eq!(cleared.account_flags, 0);
}

#[test]
fn account_set_do_apply_flags_enforce_disable_master_prerequisites() {
    let need_master = run_account_set_do_apply_flags(AccountSetDoApplyFlagFacts {
        set_flag: ASF_DISABLE_MASTER,
        account_exists: true,
        ..AccountSetDoApplyFlagFacts::default()
    });
    let need_alt_key = run_account_set_do_apply_flags(AccountSetDoApplyFlagFacts {
        set_flag: ASF_DISABLE_MASTER,
        account_exists: true,
        signed_with_master: true,
        ..AccountSetDoApplyFlagFacts::default()
    });
    let set_and_clear = run_account_set_do_apply_flags(AccountSetDoApplyFlagFacts {
        set_flag: ASF_DISABLE_MASTER,
        account_exists: true,
        signed_with_master: true,
        has_regular_key: true,
        ..AccountSetDoApplyFlagFacts::default()
    })
    .unwrap();
    let cleared = run_account_set_do_apply_flags(AccountSetDoApplyFlagFacts {
        clear_flag: ASF_DISABLE_MASTER,
        account_exists: true,
        account_flags: LSF_DISABLE_MASTER,
        ..AccountSetDoApplyFlagFacts::default()
    })
    .unwrap();

    assert_eq!(need_master, Err(Ter::TEC_NEED_MASTER_KEY));
    assert_eq!(need_alt_key, Err(Ter::TEC_NO_ALTERNATIVE_KEY));
    assert_eq!(set_and_clear.account_flags, LSF_DISABLE_MASTER);
    assert_eq!(cleared.account_flags, 0);
}

#[test]
fn account_set_do_apply_flags_keep_current_nofreeze_and_global_freeze_rules() {
    let need_master = run_account_set_do_apply_flags(AccountSetDoApplyFlagFacts {
        set_flag: ASF_NO_FREEZE,
        account_exists: true,
        ..AccountSetDoApplyFlagFacts::default()
    });
    let allowed_with_disabled_master = run_account_set_do_apply_flags(AccountSetDoApplyFlagFacts {
        set_flag: ASF_NO_FREEZE,
        account_exists: true,
        account_flags: LSF_DISABLE_MASTER,
        ..AccountSetDoApplyFlagFacts::default()
    })
    .unwrap();
    let clear_global_freeze = run_account_set_do_apply_flags(AccountSetDoApplyFlagFacts {
        clear_flag: ASF_GLOBAL_FREEZE,
        account_exists: true,
        account_flags: LSF_GLOBAL_FREEZE,
        ..AccountSetDoApplyFlagFacts::default()
    })
    .unwrap();
    let blocked_clear = run_account_set_do_apply_flags(AccountSetDoApplyFlagFacts {
        set_flag: ASF_NO_FREEZE,
        clear_flag: ASF_GLOBAL_FREEZE,
        account_exists: true,
        signed_with_master: true,
        account_flags: LSF_GLOBAL_FREEZE,
        ..AccountSetDoApplyFlagFacts::default()
    })
    .unwrap();

    assert_eq!(need_master, Err(Ter::TEC_NEED_MASTER_KEY));
    assert_eq!(
        allowed_with_disabled_master.account_flags,
        LSF_DISABLE_MASTER | LSF_NO_FREEZE
    );
    assert_eq!(clear_global_freeze.account_flags, 0);
    assert_eq!(
        blocked_clear.account_flags,
        LSF_GLOBAL_FREEZE | LSF_NO_FREEZE
    );
}

#[test]
fn account_set_do_apply_flags_keep_default_ripple_deposit_auth_and_account_txn_id_rules() {
    let set = run_account_set_do_apply_flags(AccountSetDoApplyFlagFacts {
        set_flag: ASF_DEFAULT_RIPPLE,
        account_exists: true,
        ..AccountSetDoApplyFlagFacts::default()
    })
    .unwrap();
    let clear = run_account_set_do_apply_flags(AccountSetDoApplyFlagFacts {
        clear_flag: ASF_DEPOSIT_AUTH,
        account_exists: true,
        account_flags: LSF_DEPOSIT_AUTH,
        ..AccountSetDoApplyFlagFacts::default()
    })
    .unwrap();
    let txn_id_set = run_account_set_do_apply_flags(AccountSetDoApplyFlagFacts {
        set_flag: ASF_ACCOUNT_TXN_ID,
        account_exists: true,
        ..AccountSetDoApplyFlagFacts::default()
    })
    .unwrap();
    let txn_id_clear = run_account_set_do_apply_flags(AccountSetDoApplyFlagFacts {
        clear_flag: ASF_ACCOUNT_TXN_ID,
        account_exists: true,
        account_txn_id_present: true,
        ..AccountSetDoApplyFlagFacts::default()
    })
    .unwrap();

    assert_eq!(set.account_flags, LSF_DEFAULT_RIPPLE);
    assert_eq!(clear.account_flags, 0);
    assert_eq!(txn_id_set.account_txn_id_action, AccountSetTxnIdAction::Set);
    assert_eq!(
        txn_id_clear.account_txn_id_action,
        AccountSetTxnIdAction::Clear
    );
}

#[test]
fn account_set_do_apply_tail_preserves_field_set_and_clear_rules() {
    let result = run_account_set_do_apply_tail(AccountSetDoApplyTailFacts {
        email_hash: Some(5),
        wallet_locator: Some(vec![1, 2, 3]),
        message_key: Some(vec![4, 5]),
        domain: Some(b"example.com".to_vec()),
        transfer_rate: Some(2_000_000_000),
        tick_size: Some(8),
        nftoken_minter: Some("bob"),
        set_flag: ASF_AUTHORIZED_NFTOKEN_MINTER,
        quality_one: 1_000_000_000,
        max_tick_size: 15,
        ..AccountSetDoApplyTailFacts::default()
    });
    let cleared = run_account_set_do_apply_tail(AccountSetDoApplyTailFacts::<&'static str> {
        email_hash: Some(0),
        wallet_locator: Some(Vec::new()),
        message_key: Some(Vec::new()),
        domain: Some(Vec::new()),
        transfer_rate: Some(1_000_000_000),
        tick_size: Some(15),
        clear_flag: ASF_AUTHORIZED_NFTOKEN_MINTER,
        nftoken_minter_present_on_account: true,
        quality_one: 1_000_000_000,
        max_tick_size: 15,
        ..AccountSetDoApplyTailFacts::default()
    });

    assert_eq!(result.email_hash_action, AccountSetFieldMutation::Set(5));
    assert_eq!(
        result.wallet_locator_action,
        AccountSetFieldMutation::Set(vec![1, 2, 3])
    );
    assert_eq!(
        result.message_key_action,
        AccountSetFieldMutation::Set(vec![4, 5])
    );
    assert_eq!(
        result.domain_action,
        AccountSetFieldMutation::Set(b"example.com".to_vec())
    );
    assert_eq!(
        result.transfer_rate_action,
        AccountSetFieldMutation::Set(2_000_000_000)
    );
    assert_eq!(result.tick_size_action, AccountSetFieldMutation::Set(8));
    assert_eq!(
        result.nftoken_minter_action,
        AccountSetFieldMutation::Set("bob")
    );

    assert_eq!(cleared.email_hash_action, AccountSetFieldMutation::Clear);
    assert_eq!(
        cleared.wallet_locator_action,
        AccountSetFieldMutation::Clear
    );
    assert_eq!(cleared.message_key_action, AccountSetFieldMutation::Clear);
    assert_eq!(cleared.domain_action, AccountSetFieldMutation::Clear);
    assert_eq!(cleared.transfer_rate_action, AccountSetFieldMutation::Clear);
    assert_eq!(cleared.tick_size_action, AccountSetFieldMutation::Clear);
    assert_eq!(
        cleared.nftoken_minter_action,
        AccountSetFieldMutation::Clear
    );
}

#[test]
fn account_set_do_apply_tail_preserves_disallow_incoming_toggle_order() {
    let set_offer = run_account_set_do_apply_tail(AccountSetDoApplyTailFacts::<&'static str> {
        set_flag: ASF_DISALLOW_INCOMING_NFTOKEN_OFFER,
        ..AccountSetDoApplyTailFacts::default()
    });
    let clear_check = run_account_set_do_apply_tail(AccountSetDoApplyTailFacts::<&'static str> {
        clear_flag: ASF_DISALLOW_INCOMING_CHECK,
        account_flags: LSF_DISALLOW_INCOMING_CHECK,
        ..AccountSetDoApplyTailFacts::default()
    });
    let set_paychan = run_account_set_do_apply_tail(AccountSetDoApplyTailFacts::<&'static str> {
        set_flag: ASF_DISALLOW_INCOMING_PAY_CHAN,
        ..AccountSetDoApplyTailFacts::default()
    });
    let clear_trustline =
        run_account_set_do_apply_tail(AccountSetDoApplyTailFacts::<&'static str> {
            clear_flag: ASF_DISALLOW_INCOMING_TRUSTLINE,
            account_flags: LSF_DISALLOW_INCOMING_TRUSTLINE,
            ..AccountSetDoApplyTailFacts::default()
        });

    assert_eq!(set_offer.account_flags, LSF_DISALLOW_INCOMING_NFTOKEN_OFFER);
    assert_eq!(clear_check.account_flags, 0);
    assert_eq!(set_paychan.account_flags, LSF_DISALLOW_INCOMING_PAY_CHAN);
    assert_eq!(clear_trustline.account_flags, 0);
}

#[test]
fn account_set_do_apply_tail_gates_locking_and_clawback_flags() {
    let locking_disabled =
        run_account_set_do_apply_tail(AccountSetDoApplyTailFacts::<&'static str> {
            set_flag: ASF_ALLOW_TRUST_LINE_LOCKING,
            feature_token_escrow_enabled: false,
            ..AccountSetDoApplyTailFacts::default()
        });
    let locking_enabled =
        run_account_set_do_apply_tail(AccountSetDoApplyTailFacts::<&'static str> {
            set_flag: ASF_ALLOW_TRUST_LINE_LOCKING,
            feature_token_escrow_enabled: true,
            ..AccountSetDoApplyTailFacts::default()
        });
    let clawback_enabled =
        run_account_set_do_apply_tail(AccountSetDoApplyTailFacts::<&'static str> {
            set_flag: ASF_ALLOW_TRUST_LINE_CLAWBACK,
            feature_clawback_enabled: true,
            ..AccountSetDoApplyTailFacts::default()
        });
    let clawback_clear =
        run_account_set_do_apply_tail(AccountSetDoApplyTailFacts::<&'static str> {
            clear_flag: ASF_ALLOW_TRUST_LINE_CLAWBACK,
            account_flags: LSF_ALLOW_TRUST_LINE_CLAWBACK,
            feature_clawback_enabled: true,
            ..AccountSetDoApplyTailFacts::default()
        });

    assert_eq!(locking_disabled.account_flags, 0);
    assert_eq!(locking_enabled.account_flags, LSF_ALLOW_TRUST_LINE_LOCKING);
    assert_eq!(
        clawback_enabled.account_flags,
        LSF_ALLOW_TRUST_LINE_CLAWBACK
    );
    assert_eq!(clawback_clear.account_flags, LSF_ALLOW_TRUST_LINE_CLAWBACK);
}

#[test]
fn account_set_do_apply_outer_composition_uses_current_cpp_success_order() {
    let mut sink = StubDoApplySink::default();

    let result = run_account_set_do_apply(
        &mut sink,
        AccountSetDoApplyFacts {
            flag_facts: AccountSetDoApplyFlagFacts {
                tx_flags: ACCOUNT_SET_REQUIRE_DEST_TAG_FLAG,
                clear_flag: ASF_ACCOUNT_TXN_ID,
                account_exists: true,
                account_txn_id_present: true,
                ..AccountSetDoApplyFlagFacts::default()
            },
            tail_facts: AccountSetDoApplyTailFacts {
                account_flags: u32::MAX,
                message_key: Some(Vec::new()),
                domain: Some(b"example.com".to_vec()),
                ..AccountSetDoApplyTailFacts::default()
            },
        },
    );

    assert_eq!(result, Ter::TES_SUCCESS);
    assert_eq!(
        sink.steps,
        vec![
            "clear_account_txn_id".to_string(),
            "clear_message_key".to_string(),
            "set_domain=example.com".to_string(),
            format!("set_flags={LSF_REQUIRE_DEST_TAG}"),
            "update_account".to_string(),
        ]
    );
}

#[test]
fn account_set_do_apply_outer_composition_still_updates_without_flag_write() {
    let mut sink = StubDoApplySink::default();

    let result = run_account_set_do_apply(
        &mut sink,
        AccountSetDoApplyFacts {
            flag_facts: AccountSetDoApplyFlagFacts {
                account_exists: true,
                ..AccountSetDoApplyFlagFacts::default()
            },
            tail_facts: AccountSetDoApplyTailFacts::default(),
        },
    );

    assert_eq!(result, Ter::TES_SUCCESS);
    assert_eq!(sink.steps, vec!["update_account".to_string()]);
}

#[test]
fn account_set_do_apply_outer_composition_returns_first_flag_failure_unchanged() {
    let mut sink = StubDoApplySink::default();

    let result = run_account_set_do_apply(
        &mut sink,
        AccountSetDoApplyFacts {
            flag_facts: AccountSetDoApplyFlagFacts {
                account_exists: true,
                set_flag: ASF_DISABLE_MASTER,
                ..AccountSetDoApplyFlagFacts::default()
            },
            tail_facts: AccountSetDoApplyTailFacts {
                domain: Some(b"unreachable".to_vec()),
                ..AccountSetDoApplyTailFacts::default()
            },
        },
    );

    assert_eq!(result, Ter::TEC_NEED_MASTER_KEY);
    assert!(sink.steps.is_empty());
}

#[test]
fn account_set_check_permission_passthroughs_success_without_delegate() {
    let delegate_read = Cell::new(false);
    let permission_check = Cell::new(false);
    let tx = StubPermissionTx {
        account_id: "alice",
        ..StubPermissionTx::default()
    };

    let result = run_account_set_check_permission(
        &tx,
        |_account, _delegate| {
            delegate_read.set(true);
            Some(Vec::<AccountSetGranularPermission>::new())
        },
        |_state, _permission| {
            permission_check.set(true);
            true
        },
    );

    assert_eq!(result, Ter::TES_SUCCESS);
    assert!(!delegate_read.get());
    assert!(!permission_check.get());
}

#[test]
fn account_set_check_permission_rejects_missing_delegate_entry() {
    let tx = StubPermissionTx {
        account_id: "alice",
        delegate: Some("bob"),
        domain_present: true,
        ..StubPermissionTx::default()
    };

    let result = run_account_set_check_permission(
        &tx,
        |_account, _delegate| None::<Vec<AccountSetGranularPermission>>,
        |_state, _| true,
    );

    assert_eq!(result, Ter::TER_NO_DELEGATE_PERMISSION);
    assert_eq!(trans_token(result), "terNO_DELEGATE_PERMISSION");
}

#[test]
fn account_set_check_permission_rejects_flag_based_and_non_granular_updates() {
    for tx in [
        StubPermissionTx {
            account_id: "alice",
            delegate: Some("bob"),
            set_flag: ASF_REQUIRE_AUTH,
            ..StubPermissionTx::default()
        },
        StubPermissionTx {
            account_id: "alice",
            delegate: Some("bob"),
            clear_flag: ASF_REQUIRE_DEST,
            ..StubPermissionTx::default()
        },
        StubPermissionTx {
            account_id: "alice",
            delegate: Some("bob"),
            flags: ACCOUNT_SET_REQUIRE_DEST_TAG_FLAG,
            ..StubPermissionTx::default()
        },
        StubPermissionTx {
            account_id: "alice",
            delegate: Some("bob"),
            wallet_locator_present: true,
            ..StubPermissionTx::default()
        },
        StubPermissionTx {
            account_id: "alice",
            delegate: Some("bob"),
            nftoken_minter_present: true,
            ..StubPermissionTx::default()
        },
    ] {
        let requested = Rc::new(RefCell::new(Vec::new()));
        let requested_clone = requested.clone();
        let result = run_account_set_check_permission(
            &tx,
            |_account, _delegate| Some(vec![AccountSetGranularPermission::DomainSet]),
            |_state, permission| {
                requested_clone.borrow_mut().push(permission);
                true
            },
        );

        assert_eq!(result, Ter::TER_NO_DELEGATE_PERMISSION);
        assert!(requested.borrow().is_empty());
    }
}

#[test]
fn account_set_check_permission_keeps_current_granular_permission_mapping_and_order() {
    let requested = Rc::new(RefCell::new(Vec::new()));
    let requested_clone = requested.clone();
    let tx = StubPermissionTx {
        account_id: "alice",
        delegate: Some("bob"),
        flags: FULLY_CANONICAL_SIGNATURE_FLAG,
        email_hash_present: true,
        message_key_present: true,
        domain_present: true,
        transfer_rate_present: true,
        tick_size_present: true,
        ..StubPermissionTx::default()
    };

    let permissions = vec![
        AccountSetGranularPermission::EmailHashSet,
        AccountSetGranularPermission::MessageKeySet,
        AccountSetGranularPermission::DomainSet,
        AccountSetGranularPermission::TransferRateSet,
    ];

    let result = run_account_set_check_permission(
        &tx,
        |_account, _delegate| Some(permissions.clone()),
        |state, permission| {
            requested_clone.borrow_mut().push(permission);
            state.contains(&permission)
        },
    );

    assert_eq!(result, Ter::TER_NO_DELEGATE_PERMISSION);
    assert_eq!(
        requested.borrow().as_slice(),
        [
            AccountSetGranularPermission::EmailHashSet,
            AccountSetGranularPermission::MessageKeySet,
            AccountSetGranularPermission::DomainSet,
            AccountSetGranularPermission::TransferRateSet,
            AccountSetGranularPermission::TickSizeSet,
        ]
    );
}

#[test]
fn account_set_check_permission_accepts_universal_flags_and_matching_granular_permissions() {
    let tx = StubPermissionTx {
        account_id: "alice",
        delegate: Some("bob"),
        flags: FULLY_CANONICAL_SIGNATURE_FLAG,
        domain_present: true,
        email_hash_present: true,
        ..StubPermissionTx::default()
    };

    let result = run_account_set_check_permission(
        &tx,
        |_account, _delegate| {
            Some(vec![
                AccountSetGranularPermission::DomainSet,
                AccountSetGranularPermission::EmailHashSet,
            ])
        },
        |state, permission| state.contains(&permission),
    );

    assert_eq!(result, Ter::TES_SUCCESS);
    assert_eq!(trans_token(result), "tesSUCCESS");
}
