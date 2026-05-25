use std::cell::{Cell, RefCell};

use protocol::{Ter, trans_token};
use tx::{
    DEFAULT_SIGNER_LIST_ID, LSF_ONE_OWNER_COUNT, SIGNER_LIST_SET_FLAGS_MASK,
    SIGNER_LIST_SET_MAX_MULTI_SIGNERS, SignerListSetEntry, SignerListSetLedgerEntry,
    SignerListSetLedgerSignerEntry, SignerListSetLedgerWritePlan, SignerListSetOperation,
    SignerListSetPreComputeState, SignerListSetPreflightFacts, SignerListSetWriteEntry,
    build_signer_list_set_ledger_write_plan, get_signer_list_set_flags_mask,
    run_signer_list_set_destroy_signer_list, run_signer_list_set_determine_operation,
    run_signer_list_set_do_apply, run_signer_list_set_precompute, run_signer_list_set_preflight,
    run_signer_list_set_remove_from_ledger, run_signer_list_set_replace_signer_list,
    signer_list_set_owner_count_delta,
};

#[derive(Debug, Clone, Copy)]
struct TestSignerList {
    flags: u32,
    signer_entries_len: usize,
    owner_node: u64,
}

#[derive(Debug, Clone, Copy)]
struct TestTaggedSignerEntry {
    account: &'static str,
    weight: u16,
    wallet_locator: Option<&'static str>,
}

impl SignerListSetWriteEntry for TestTaggedSignerEntry {
    type AccountId = &'static str;
    type WalletLocator = &'static str;

    fn account(&self) -> &Self::AccountId {
        &self.account
    }

    fn weight(&self) -> u16 {
        self.weight
    }

    fn wallet_locator(&self) -> Option<&Self::WalletLocator> {
        self.wallet_locator.as_ref()
    }
}

#[derive(Debug, Clone, Copy)]
struct TestAccount {
    owner_count: u32,
}

impl SignerListSetLedgerEntry for TestSignerList {
    fn flags(&self) -> u32 {
        self.flags
    }

    fn signer_entries_len(&self) -> usize {
        self.signer_entries_len
    }

    fn owner_node(&self) -> u64 {
        self.owner_node
    }
}

#[test]
fn signer_list_set_flags_mask_tracks_fix_invalid_tx_flags() {
    assert_eq!(get_signer_list_set_flags_mask(false), 0);
    assert_eq!(
        get_signer_list_set_flags_mask(true),
        SIGNER_LIST_SET_FLAGS_MASK
    );
}

#[test]
fn signer_list_set_determine_operation_sorts_on_set_and_keeps_destroy() {
    let set = run_signer_list_set_determine_operation(
        2,
        true,
        Ok(vec![
            SignerListSetEntry {
                account: "charlie",
                weight: 1,
            },
            SignerListSetEntry {
                account: "bob",
                weight: 1,
            },
        ]),
    );
    let destroy = run_signer_list_set_determine_operation(
        0,
        false,
        Ok(Vec::<SignerListSetEntry<&str>>::new()),
    );

    assert_eq!(set.result, Ter::TES_SUCCESS);
    assert_eq!(set.operation, SignerListSetOperation::Set);
    assert_eq!(set.signers[0].account, "bob");
    assert_eq!(set.signers[1].account, "charlie");
    assert_eq!(destroy.operation, SignerListSetOperation::Destroy);
}

#[test]
fn signer_list_set_determine_operation_passthroughs_deserialize_failure() {
    let result =
        run_signer_list_set_determine_operation::<&'static str>(3, true, Err(Ter::TEM_BAD_SIGNER));

    assert_eq!(result.result, Ter::TEM_BAD_SIGNER);
    assert_eq!(trans_token(result.result), "temBAD_SIGNER");
    assert_eq!(result.operation, SignerListSetOperation::Unknown);
}

#[test]
fn signer_list_set_preflight_rejects_unknown_transaction_shapes() {
    let missing_entries =
        run_signer_list_set_preflight(SignerListSetPreflightFacts::<&'static str> {
            quorum: 1,
            has_signer_entries: false,
            signer_entries: Ok(Vec::new()),
            account: "alice",
        });
    let destroy_with_entries =
        run_signer_list_set_preflight(SignerListSetPreflightFacts::<&'static str> {
            quorum: 0,
            has_signer_entries: true,
            signer_entries: Ok(vec![SignerListSetEntry {
                account: "bob",
                weight: 1,
            }]),
            account: "alice",
        });

    assert_eq!(missing_entries, Ter::TEM_MALFORMED);
    assert_eq!(destroy_with_entries, Ter::TEM_MALFORMED);
    assert_eq!(trans_token(missing_entries), "temMALFORMED");
}

#[test]
fn signer_list_set_preflight_rejects_duplicate_self_and_zero_weight() {
    let duplicate = run_signer_list_set_preflight(SignerListSetPreflightFacts::<&'static str> {
        quorum: 2,
        has_signer_entries: true,
        signer_entries: Ok(vec![
            SignerListSetEntry {
                account: "bob",
                weight: 1,
            },
            SignerListSetEntry {
                account: "bob",
                weight: 1,
            },
        ]),
        account: "alice",
    });
    let self_reference =
        run_signer_list_set_preflight(SignerListSetPreflightFacts::<&'static str> {
            quorum: 1,
            has_signer_entries: true,
            signer_entries: Ok(vec![SignerListSetEntry {
                account: "alice",
                weight: 1,
            }]),
            account: "alice",
        });
    let zero_weight = run_signer_list_set_preflight(SignerListSetPreflightFacts::<&'static str> {
        quorum: 1,
        has_signer_entries: true,
        signer_entries: Ok(vec![SignerListSetEntry {
            account: "bob",
            weight: 0,
        }]),
        account: "alice",
    });

    assert_eq!(duplicate, Ter::TEM_BAD_SIGNER);
    assert_eq!(self_reference, Ter::TEM_BAD_SIGNER);
    assert_eq!(zero_weight, Ter::TEM_BAD_WEIGHT);
}

#[test]
fn signer_list_set_preflight_rejects_unreachable_or_oversized_quorums() {
    let bad_quorum = run_signer_list_set_preflight(SignerListSetPreflightFacts::<&'static str> {
        quorum: 3,
        has_signer_entries: true,
        signer_entries: Ok(vec![SignerListSetEntry {
            account: "bob",
            weight: 2,
        }]),
        account: "alice",
    });
    let too_many = run_signer_list_set_preflight(SignerListSetPreflightFacts {
        quorum: 33,
        has_signer_entries: true,
        signer_entries: Ok((0..=SIGNER_LIST_SET_MAX_MULTI_SIGNERS)
            .map(|i| SignerListSetEntry {
                account: i,
                weight: 1,
            })
            .collect()),
        account: 999,
    });

    assert_eq!(bad_quorum, Ter::TEM_BAD_QUORUM);
    assert_eq!(too_many, Ter::TEM_MALFORMED);
    assert_eq!(trans_token(bad_quorum), "temBAD_QUORUM");
}

#[test]
fn signer_list_set_preflight_accepts_valid_set_and_destroy() {
    let set = run_signer_list_set_preflight(SignerListSetPreflightFacts::<&'static str> {
        quorum: 3,
        has_signer_entries: true,
        signer_entries: Ok(vec![
            SignerListSetEntry {
                account: "charlie",
                weight: 2,
            },
            SignerListSetEntry {
                account: "bob",
                weight: 1,
            },
        ]),
        account: "alice",
    });
    let destroy = run_signer_list_set_preflight(SignerListSetPreflightFacts::<&'static str> {
        quorum: 0,
        has_signer_entries: false,
        signer_entries: Ok(Vec::new()),
        account: "alice",
    });

    assert_eq!(set, Ter::TES_SUCCESS);
    assert_eq!(destroy, Ter::TES_SUCCESS);
}

#[test]
fn signer_list_set_precompute_copies_state_before_base_precompute() {
    let mut called = false;
    let state = run_signer_list_set_precompute(
        run_signer_list_set_determine_operation(
            3,
            true,
            Ok(vec![SignerListSetEntry {
                account: "bob",
                weight: 1,
            }]),
        ),
        || called = true,
    );

    assert!(called);
    assert_eq!(
        state,
        SignerListSetPreComputeState {
            quorum: 3,
            signers: vec![SignerListSetEntry {
                account: "bob",
                weight: 1,
            }],
            operation: SignerListSetOperation::Set,
        }
    );
}

#[test]
fn signer_list_set_do_apply_dispatches_set_and_destroy() {
    let set = run_signer_list_set_do_apply(
        SignerListSetOperation::Set,
        || Ter::TES_SUCCESS,
        || Ter::TEM_MALFORMED,
    );
    let destroy = run_signer_list_set_do_apply(
        SignerListSetOperation::Destroy,
        || Ter::TEM_MALFORMED,
        || Ter::TEC_NO_ALTERNATIVE_KEY,
    );

    assert_eq!(set, Ter::TES_SUCCESS);
    assert_eq!(destroy, Ter::TEC_NO_ALTERNATIVE_KEY);
}

#[test]
fn signer_list_set_owner_count_delta_matches_current_cpp_rule() {
    assert_eq!(signer_list_set_owner_count_delta(1), 3);
    assert_eq!(signer_list_set_owner_count_delta(8), 10);
}

#[test]
fn signer_list_set_remove_from_ledger_preserves_current_and_one_owner_case() {
    let missing = run_signer_list_set_remove_from_ledger(
        None::<TestSignerList>,
        |_| panic!("missing signer list should skip dir remove"),
        |_| panic!("missing signer list should skip owner count"),
        |_| panic!("missing signer list should skip erase"),
    );

    let events = RefCell::new(Vec::new());
    let old_style = run_signer_list_set_remove_from_ledger(
        Some(TestSignerList {
            flags: 0,
            signer_entries_len: 3,
            owner_node: 22,
        }),
        |owner_node| {
            events.borrow_mut().push(format!("dir_remove:{owner_node}"));
            true
        },
        |delta| events.borrow_mut().push(format!("adjust:{delta}")),
        |_| events.borrow_mut().push("erase".to_string()),
    );

    let one_owner_delta = Cell::new(0);
    let one_owner = run_signer_list_set_remove_from_ledger(
        Some(TestSignerList {
            flags: LSF_ONE_OWNER_COUNT,
            signer_entries_len: 5,
            owner_node: 7,
        }),
        |_| true,
        |delta| one_owner_delta.set(delta),
        |_| {},
    );

    assert_eq!(missing, Ter::TES_SUCCESS);
    assert_eq!(old_style, Ter::TES_SUCCESS);
    assert_eq!(
        events.into_inner(),
        vec![
            "dir_remove:22".to_string(),
            "adjust:-5".to_string(),
            "erase".to_string(),
        ]
    );
    assert_eq!(one_owner, Ter::TES_SUCCESS);
    assert_eq!(one_owner_delta.get(), -1);
}

#[test]
fn signer_list_set_remove_from_ledger_maps_dir_remove_failure() {
    let adjusted = Cell::new(false);
    let erased = Cell::new(false);
    let result = run_signer_list_set_remove_from_ledger(
        Some(TestSignerList {
            flags: 0,
            signer_entries_len: 1,
            owner_node: 9,
        }),
        |_| false,
        |_| adjusted.set(true),
        |_| erased.set(true),
    );

    assert_eq!(result, Ter::TEF_BAD_LEDGER);
    assert!(!adjusted.get());
    assert!(!erased.get());
}

#[test]
fn signer_list_set_destroy_signer_list_preserves_current_cpp_guards() {
    let missing_account =
        run_signer_list_set_destroy_signer_list(false, false, false, || Ter::TES_SUCCESS);
    let no_alternative =
        run_signer_list_set_destroy_signer_list(true, true, false, || Ter::TES_SUCCESS);
    let delegated = run_signer_list_set_destroy_signer_list(true, true, true, || Ter::TEC_OWNERS);

    assert_eq!(missing_account, Ter::TEF_INTERNAL);
    assert_eq!(no_alternative, Ter::TEC_NO_ALTERNATIVE_KEY);
    assert_eq!(delegated, Ter::TEC_OWNERS);
}

#[test]
fn signer_list_set_replace_signer_list_preserves_current_cpp_guards() {
    let remove_failure = run_signer_list_set_replace_signer_list(
        &10_u32,
        || Ter::TEF_BAD_LEDGER,
        Some(TestAccount { owner_count: 4 }),
        |account| account.owner_count,
        |_| panic!("remove failure should skip reserve"),
        |_| panic!("remove failure should skip prepare"),
        || panic!("remove failure should skip dir insert"),
        |_| panic!("remove failure should skip owner node"),
        |_, _| panic!("remove failure should skip owner count"),
    );
    let missing_account = run_signer_list_set_replace_signer_list(
        &10_u32,
        || Ter::TES_SUCCESS,
        None::<TestAccount>,
        |_| unreachable!(),
        |_| unreachable!(),
        |_| unreachable!(),
        || unreachable!(),
        |_| unreachable!(),
        |_, _| unreachable!(),
    );
    let reserve_shortfall = run_signer_list_set_replace_signer_list(
        &4_u32,
        || Ter::TES_SUCCESS,
        Some(TestAccount { owner_count: 2 }),
        |account| account.owner_count,
        |new_owner_count| new_owner_count + 3,
        |_| panic!("reserve failure should skip prepare"),
        || panic!("reserve failure should skip dir insert"),
        |_| panic!("reserve failure should skip owner node"),
        |_, _| panic!("reserve failure should skip owner count"),
    );

    assert_eq!(remove_failure, Ter::TEF_BAD_LEDGER);
    assert_eq!(missing_account, Ter::TEF_INTERNAL);
    assert_eq!(reserve_shortfall, Ter::TEC_INSUFFICIENT_RESERVE);
}

#[test]
fn signer_list_set_replace_signer_list_preserves_current_cpp_success_order() {
    let events = RefCell::new(Vec::new());
    let result = run_signer_list_set_replace_signer_list(
        &10_u32,
        || {
            events.borrow_mut().push("remove".to_string());
            Ter::TES_SUCCESS
        },
        Some(TestAccount { owner_count: 4 }),
        |account| account.owner_count,
        |new_owner_count| {
            events
                .borrow_mut()
                .push(format!("reserve:{new_owner_count}"));
            5_u32
        },
        |flags| events.borrow_mut().push(format!("prepare:{flags:#010x}")),
        || {
            events.borrow_mut().push("dir_insert".to_string());
            Some(33)
        },
        |page| events.borrow_mut().push(format!("set_owner_node:{page}")),
        |account, delta| {
            events
                .borrow_mut()
                .push(format!("adjust:{}:{delta}", account.owner_count));
        },
    );

    assert_eq!(result, Ter::TES_SUCCESS);
    assert_eq!(
        events.into_inner(),
        vec![
            "remove".to_string(),
            "reserve:5".to_string(),
            format!("prepare:{LSF_ONE_OWNER_COUNT:#010x}"),
            "dir_insert".to_string(),
            "set_owner_node:33".to_string(),
            "adjust:4:1".to_string(),
        ]
    );
}

#[test]
fn signer_list_set_replace_signer_list_maps_dir_insert_failure() {
    let prepared = Cell::new(false);
    let set_owner_node = Cell::new(false);
    let adjusted = Cell::new(false);
    let result = run_signer_list_set_replace_signer_list(
        &10_u32,
        || Ter::TES_SUCCESS,
        Some(TestAccount { owner_count: 1 }),
        |account| account.owner_count,
        |_| 5_u32,
        |_| prepared.set(true),
        || None,
        |_| set_owner_node.set(true),
        |_, _| adjusted.set(true),
    );

    assert_eq!(result, Ter::TEC_DIR_FULL);
    assert!(prepared.get());
    assert!(!set_owner_node.get());
    assert!(!adjusted.get());
}

#[test]
fn signer_list_set_write_plan_preserves_current_cpp_metadata() {
    let plan = build_signer_list_set_ledger_write_plan(
        true,
        "alice",
        3,
        LSF_ONE_OWNER_COUNT,
        &[TestTaggedSignerEntry {
            account: "bob",
            weight: 1,
            wallet_locator: None,
        }],
    );

    assert_eq!(
        plan,
        SignerListSetLedgerWritePlan {
            owner: Some("alice"),
            signer_quorum: 3,
            signer_list_id: DEFAULT_SIGNER_LIST_ID,
            flags: Some(LSF_ONE_OWNER_COUNT),
            signer_entries: vec![SignerListSetLedgerSignerEntry {
                account: "bob",
                weight: 1,
                wallet_locator: None,
            }],
        }
    );
}

#[test]
fn signer_list_set_write_plan_omits_default_owner_and_flags() {
    let plan = build_signer_list_set_ledger_write_plan(
        false,
        "alice",
        2,
        0,
        &[SignerListSetEntry {
            account: "bob",
            weight: 7,
        }],
    );

    assert_eq!(plan.owner, None);
    assert_eq!(plan.flags, None);
    assert_eq!(plan.signer_quorum, 2);
    assert_eq!(plan.signer_list_id, DEFAULT_SIGNER_LIST_ID);
    assert_eq!(plan.signer_entries.len(), 1);
    assert_eq!(plan.signer_entries[0].account, "bob");
    assert_eq!(plan.signer_entries[0].weight, 7);
    assert_eq!(plan.signer_entries[0].wallet_locator, None);
}

#[test]
fn signer_list_set_write_plan_preserves_entry_order_and_optional_wallet_locator() {
    let plan = build_signer_list_set_ledger_write_plan(
        false,
        "alice",
        4,
        LSF_ONE_OWNER_COUNT,
        &[
            TestTaggedSignerEntry {
                account: "charlie",
                weight: 2,
                wallet_locator: Some("tag-1"),
            },
            TestTaggedSignerEntry {
                account: "bob",
                weight: 1,
                wallet_locator: None,
            },
        ],
    );

    assert_eq!(plan.signer_entries[0].account, "charlie");
    assert_eq!(plan.signer_entries[0].wallet_locator, Some("tag-1"));
    assert_eq!(plan.signer_entries[1].account, "bob");
    assert_eq!(plan.signer_entries[1].wallet_locator, None);
}
