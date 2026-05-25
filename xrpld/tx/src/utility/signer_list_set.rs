//! Deterministic the reference implementation shells.
//!
//! This ports the exact current behavior around:
//!
//! - `determineOperation(...)` set-versus-destroy shape selection,
//! - sorting deserialized signer entries on the set path,
//! - the `fixInvalidTxFlags`-gated `getFlagsMask(...)` return,
//! - quorum and signer-entry validation,
//! - the top-level `preflight(...)` malformed versus validation flow,
//! - `preCompute()` and outer `doApply()` dispatch,
//! - the first signer-list destroy/remove ledger shells,
//! - and the higher `replaceSignerList()` wrapper ordering.

use protocol::{NotTec, Ter};

pub const FULLY_CANONICAL_SIGNATURE_FLAG: u32 = 0x8000_0000;
pub const INNER_BATCH_TRANSACTION_FLAG: u32 = 0x4000_0000;
pub const SIGNER_LIST_SET_FLAGS_MASK: u32 =
    !(FULLY_CANONICAL_SIGNATURE_FLAG | INNER_BATCH_TRANSACTION_FLAG);

pub const MIN_MULTI_SIGNERS: usize = 1;
pub const MAX_MULTI_SIGNERS: usize = 32;
pub const LSF_ONE_OWNER_COUNT: u32 = 0x0001_0000;
pub const DEFAULT_SIGNER_LIST_ID: u32 = 0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SignerListSetOperation {
    #[default]
    Unknown,
    Set,
    Destroy,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct SignerListSetEntry<AccountId> {
    pub account: AccountId,
    pub weight: u16,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignerListSetDetermineOperationResult<AccountId> {
    pub result: NotTec,
    pub quorum: u32,
    pub signers: Vec<SignerListSetEntry<AccountId>>,
    pub operation: SignerListSetOperation,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignerListSetPreflightFacts<AccountId> {
    pub quorum: u32,
    pub has_signer_entries: bool,
    pub signer_entries: Result<Vec<SignerListSetEntry<AccountId>>, NotTec>,
    pub account: AccountId,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignerListSetPreComputeState<AccountId> {
    pub quorum: u32,
    pub signers: Vec<SignerListSetEntry<AccountId>>,
    pub operation: SignerListSetOperation,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignerListSetLedgerSignerEntry<AccountId, WalletLocator> {
    pub account: AccountId,
    pub weight: u16,
    pub wallet_locator: Option<WalletLocator>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignerListSetLedgerWritePlan<AccountId, WalletLocator> {
    pub owner: Option<AccountId>,
    pub signer_quorum: u32,
    pub signer_list_id: u32,
    pub flags: Option<u32>,
    pub signer_entries: Vec<SignerListSetLedgerSignerEntry<AccountId, WalletLocator>>,
}

pub fn get_signer_list_set_flags_mask(fix_invalid_tx_flags_enabled: bool) -> u32 {
    if fix_invalid_tx_flags_enabled {
        SIGNER_LIST_SET_FLAGS_MASK
    } else {
        0
    }
}

pub fn run_signer_list_set_determine_operation<AccountId: Ord>(
    quorum: u32,
    has_signer_entries: bool,
    signer_entries: Result<Vec<SignerListSetEntry<AccountId>>, NotTec>,
) -> SignerListSetDetermineOperationResult<AccountId> {
    let mut signers = Vec::new();
    let mut operation = SignerListSetOperation::Unknown;

    if quorum != 0 && has_signer_entries {
        signers = match signer_entries {
            Ok(signers) => signers,
            Err(err) => {
                return SignerListSetDetermineOperationResult {
                    result: err,
                    quorum,
                    signers,
                    operation,
                };
            }
        };
        signers.sort();
        operation = SignerListSetOperation::Set;
    } else if quorum == 0 && !has_signer_entries {
        operation = SignerListSetOperation::Destroy;
    }

    SignerListSetDetermineOperationResult {
        result: Ter::TES_SUCCESS,
        quorum,
        signers,
        operation,
    }
}

pub fn run_signer_list_set_validate_quorum_and_signer_entries<AccountId: Eq>(
    quorum: u32,
    signers: &[SignerListSetEntry<AccountId>],
    account: &AccountId,
) -> NotTec {
    if signers.len() < MIN_MULTI_SIGNERS || signers.len() > MAX_MULTI_SIGNERS {
        return Ter::TEM_MALFORMED;
    }

    if signers.windows(2).any(|pair| pair[0] == pair[1]) {
        return Ter::TEM_BAD_SIGNER;
    }

    let mut all_signers_weight = 0_u64;
    for signer in signers {
        if signer.weight == 0 {
            return Ter::TEM_BAD_WEIGHT;
        }

        all_signers_weight += u64::from(signer.weight);

        if &signer.account == account {
            return Ter::TEM_BAD_SIGNER;
        }
    }

    if quorum == 0 || all_signers_weight < u64::from(quorum) {
        return Ter::TEM_BAD_QUORUM;
    }

    Ter::TES_SUCCESS
}

pub fn run_signer_list_set_preflight<AccountId: Ord + Eq>(
    facts: SignerListSetPreflightFacts<AccountId>,
) -> NotTec {
    let operation = run_signer_list_set_determine_operation(
        facts.quorum,
        facts.has_signer_entries,
        facts.signer_entries,
    );

    if operation.result != Ter::TES_SUCCESS {
        return operation.result;
    }

    match operation.operation {
        SignerListSetOperation::Unknown => Ter::TEM_MALFORMED,
        SignerListSetOperation::Destroy => Ter::TES_SUCCESS,
        SignerListSetOperation::Set => run_signer_list_set_validate_quorum_and_signer_entries(
            operation.quorum,
            &operation.signers,
            &facts.account,
        ),
    }
}

pub fn run_signer_list_set_precompute<AccountId, Precompute>(
    result: SignerListSetDetermineOperationResult<AccountId>,
    mut run_transactor_precompute: Precompute,
) -> SignerListSetPreComputeState<AccountId>
where
    Precompute: FnMut(),
{
    assert_eq!(
        result.result,
        Ter::TES_SUCCESS,
        "SignerListSet::preCompute expects determineOperation success"
    );
    assert_ne!(
        result.operation,
        SignerListSetOperation::Unknown,
        "SignerListSet::preCompute expects known operation"
    );

    let state = SignerListSetPreComputeState {
        quorum: result.quorum,
        signers: result.signers,
        operation: result.operation,
    };

    run_transactor_precompute();
    state
}

pub fn run_signer_list_set_do_apply<Replace, Destroy>(
    operation: SignerListSetOperation,
    replace_signer_list: Replace,
    destroy_signer_list: Destroy,
) -> Ter
where
    Replace: FnOnce() -> Ter,
    Destroy: FnOnce() -> Ter,
{
    match operation {
        SignerListSetOperation::Set => replace_signer_list(),
        SignerListSetOperation::Destroy => destroy_signer_list(),
        SignerListSetOperation::Unknown => {
            panic!("SignerListSet::doApply expects known operation")
        }
    }
}

pub fn signer_list_set_owner_count_delta(entry_count: usize) -> i32 {
    assert!(
        (MIN_MULTI_SIGNERS..=MAX_MULTI_SIGNERS).contains(&entry_count),
        "SignerListSet::signerCountBasedOwnerCountDelta expects signer count in range"
    );
    2 + entry_count as i32
}

pub trait SignerListSetLedgerEntry {
    fn flags(&self) -> u32;
    fn signer_entries_len(&self) -> usize;
    fn owner_node(&self) -> u64;
}

pub trait SignerListSetWriteEntry {
    type AccountId;
    type WalletLocator;

    fn account(&self) -> &Self::AccountId;
    fn weight(&self) -> u16;
    fn wallet_locator(&self) -> Option<&Self::WalletLocator>;
}

impl<AccountId> SignerListSetWriteEntry for SignerListSetEntry<AccountId> {
    type AccountId = AccountId;
    type WalletLocator = ();

    fn account(&self) -> &Self::AccountId {
        &self.account
    }

    fn weight(&self) -> u16 {
        self.weight
    }

    fn wallet_locator(&self) -> Option<&Self::WalletLocator> {
        None
    }
}

pub fn run_signer_list_set_remove_from_ledger<SignerList>(
    signer_list: Option<SignerList>,
    dir_remove: impl FnOnce(u64) -> bool,
    adjust_owner_count: impl FnOnce(i32),
    erase_signer_list: impl FnOnce(SignerList),
) -> Ter
where
    SignerList: SignerListSetLedgerEntry,
{
    let Some(signer_list) = signer_list else {
        return Ter::TES_SUCCESS;
    };

    let mut remove_from_owner_count = -1;
    if signer_list.flags() & LSF_ONE_OWNER_COUNT == 0 {
        remove_from_owner_count =
            -signer_list_set_owner_count_delta(signer_list.signer_entries_len());
    }

    if !dir_remove(signer_list.owner_node()) {
        return Ter::TEF_BAD_LEDGER;
    }

    adjust_owner_count(remove_from_owner_count);
    erase_signer_list(signer_list);
    Ter::TES_SUCCESS
}

pub fn run_signer_list_set_destroy_signer_list(
    account_present: bool,
    master_disabled: bool,
    regular_key_present: bool,
    remove_from_ledger: impl FnOnce() -> Ter,
) -> Ter {
    if !account_present {
        return Ter::TEF_INTERNAL;
    }

    if master_disabled && !regular_key_present {
        return Ter::TEC_NO_ALTERNATIVE_KEY;
    }

    remove_from_ledger()
}

pub fn run_signer_list_set_replace_signer_list<
    Balance,
    Account,
    RemoveFromLedger,
    OwnerCount,
    AccountReserve,
    PrepareSignerList,
    DirInsert,
    SetOwnerNode,
    AdjustOwnerCount,
>(
    pre_fee_balance: &Balance,
    remove_from_ledger: RemoveFromLedger,
    account: Option<Account>,
    owner_count: OwnerCount,
    account_reserve: AccountReserve,
    prepare_signer_list: PrepareSignerList,
    dir_insert: DirInsert,
    set_owner_node: SetOwnerNode,
    adjust_owner_count: AdjustOwnerCount,
) -> Ter
where
    Balance: PartialOrd,
    RemoveFromLedger: FnOnce() -> Ter,
    OwnerCount: FnOnce(&Account) -> u32,
    AccountReserve: FnOnce(u32) -> Balance,
    PrepareSignerList: FnOnce(u32),
    DirInsert: FnOnce() -> Option<u64>,
    SetOwnerNode: FnOnce(u64),
    AdjustOwnerCount: FnOnce(Account, i32),
{
    let ter = remove_from_ledger();
    if ter != Ter::TES_SUCCESS {
        return ter;
    }

    let Some(account) = account else {
        return Ter::TEF_INTERNAL;
    };

    let old_owner_count = owner_count(&account);
    const ADDED_OWNER_COUNT: i32 = 1;
    let new_reserve = account_reserve(old_owner_count + ADDED_OWNER_COUNT as u32);

    if pre_fee_balance < &new_reserve {
        return Ter::TEC_INSUFFICIENT_RESERVE;
    }

    prepare_signer_list(LSF_ONE_OWNER_COUNT);

    let Some(page) = dir_insert() else {
        return Ter::TEC_DIR_FULL;
    };

    set_owner_node(page);
    adjust_owner_count(account, ADDED_OWNER_COUNT);
    Ter::TES_SUCCESS
}

pub fn build_signer_list_set_ledger_write_plan<Entry>(
    fix_include_keylet_fields_enabled: bool,
    account: Entry::AccountId,
    quorum: u32,
    flags: u32,
    signers: &[Entry],
) -> SignerListSetLedgerWritePlan<Entry::AccountId, Entry::WalletLocator>
where
    Entry: SignerListSetWriteEntry,
    Entry::AccountId: Clone,
    Entry::WalletLocator: Clone,
{
    SignerListSetLedgerWritePlan {
        owner: fix_include_keylet_fields_enabled.then_some(account),
        signer_quorum: quorum,
        signer_list_id: DEFAULT_SIGNER_LIST_ID,
        flags: (flags != 0).then_some(flags),
        signer_entries: signers
            .iter()
            .map(|entry| SignerListSetLedgerSignerEntry {
                account: entry.account().clone(),
                weight: entry.weight(),
                wallet_locator: entry.wallet_locator().cloned(),
            })
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};

    use protocol::trans_token;

    use super::{
        DEFAULT_SIGNER_LIST_ID, LSF_ONE_OWNER_COUNT, MAX_MULTI_SIGNERS, SIGNER_LIST_SET_FLAGS_MASK,
        SignerListSetDetermineOperationResult, SignerListSetEntry, SignerListSetLedgerEntry,
        SignerListSetLedgerSignerEntry, SignerListSetLedgerWritePlan, SignerListSetOperation,
        SignerListSetPreflightFacts, SignerListSetWriteEntry,
        build_signer_list_set_ledger_write_plan, get_signer_list_set_flags_mask,
        run_signer_list_set_destroy_signer_list, run_signer_list_set_determine_operation,
        run_signer_list_set_do_apply, run_signer_list_set_precompute,
        run_signer_list_set_preflight, run_signer_list_set_remove_from_ledger,
        run_signer_list_set_replace_signer_list,
        run_signer_list_set_validate_quorum_and_signer_entries, signer_list_set_owner_count_delta,
    };

    #[derive(Debug, Clone, Copy)]
    struct TestSignerList {
        flags: u32,
        signer_entries_len: usize,
        owner_node: u64,
    }

    #[derive(Debug, Clone, Copy)]
    struct TestAccount {
        owner_count: u32,
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    struct TaggedSignerEntry {
        account: &'static str,
        weight: u16,
        wallet_locator: Option<&'static str>,
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

    impl SignerListSetWriteEntry for TaggedSignerEntry {
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

    #[test]
    fn signer_list_set_flags_mask_fix_gate() {
        assert_eq!(get_signer_list_set_flags_mask(false), 0);
        assert_eq!(
            get_signer_list_set_flags_mask(true),
            SIGNER_LIST_SET_FLAGS_MASK
        );
    }

    #[test]
    fn signer_list_set_determine_operation_sorts_set_entries() {
        let result = run_signer_list_set_determine_operation(
            3,
            true,
            Ok(vec![
                SignerListSetEntry {
                    account: "charlie",
                    weight: 1,
                },
                SignerListSetEntry {
                    account: "alice",
                    weight: 1,
                },
            ]),
        );

        assert_eq!(result.result, protocol::Ter::TES_SUCCESS);
        assert_eq!(result.operation, SignerListSetOperation::Set);
        assert_eq!(
            result.signers,
            vec![
                SignerListSetEntry {
                    account: "alice",
                    weight: 1,
                },
                SignerListSetEntry {
                    account: "charlie",
                    weight: 1,
                },
            ]
        );
    }

    #[test]
    fn signer_list_set_validate_quorum_and_entries_preserves_current_cpp_errors() {
        let duplicate = run_signer_list_set_validate_quorum_and_signer_entries(
            1,
            &[
                SignerListSetEntry {
                    account: "bob",
                    weight: 1,
                },
                SignerListSetEntry {
                    account: "bob",
                    weight: 1,
                },
            ],
            &"alice",
        );
        let zero_weight = run_signer_list_set_validate_quorum_and_signer_entries(
            1,
            &[SignerListSetEntry {
                account: "bob",
                weight: 0,
            }],
            &"alice",
        );
        let unreachable = run_signer_list_set_validate_quorum_and_signer_entries(
            3,
            &[SignerListSetEntry {
                account: "bob",
                weight: 2,
            }],
            &"alice",
        );

        assert_eq!(duplicate, protocol::Ter::TEM_BAD_SIGNER);
        assert_eq!(zero_weight, protocol::Ter::TEM_BAD_WEIGHT);
        assert_eq!(unreachable, protocol::Ter::TEM_BAD_QUORUM);
        assert_eq!(trans_token(zero_weight), "temBAD_WEIGHT");
        assert_eq!(trans_token(unreachable), "temBAD_QUORUM");
    }

    #[test]
    fn signer_list_set_preflight_accepts_destroy_and_valid_set() {
        let destroy = run_signer_list_set_preflight(SignerListSetPreflightFacts::<&'static str> {
            quorum: 0,
            has_signer_entries: false,
            signer_entries: Ok(Vec::new()),
            account: "alice",
        });
        let set = run_signer_list_set_preflight(SignerListSetPreflightFacts {
            quorum: 3,
            has_signer_entries: true,
            signer_entries: Ok(vec![
                SignerListSetEntry {
                    account: "bob",
                    weight: 1,
                },
                SignerListSetEntry {
                    account: "charlie",
                    weight: 2,
                },
            ]),
            account: "alice",
        });

        assert_eq!(destroy, protocol::Ter::TES_SUCCESS);
        assert_eq!(set, protocol::Ter::TES_SUCCESS);
    }

    #[test]
    fn signer_list_set_preflight_rejects_unknown_or_bad_lists() {
        let malformed =
            run_signer_list_set_preflight(SignerListSetPreflightFacts::<&'static str> {
                quorum: 1,
                has_signer_entries: false,
                signer_entries: Ok(Vec::new()),
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
        let too_many = run_signer_list_set_preflight(SignerListSetPreflightFacts {
            quorum: 33,
            has_signer_entries: true,
            signer_entries: Ok((0..=MAX_MULTI_SIGNERS)
                .map(|i| SignerListSetEntry {
                    account: i,
                    weight: 1,
                })
                .collect()),
            account: 999,
        });

        assert_eq!(malformed, protocol::Ter::TEM_MALFORMED);
        assert_eq!(self_reference, protocol::Ter::TEM_BAD_SIGNER);
        assert_eq!(too_many, protocol::Ter::TEM_MALFORMED);
    }

    #[test]
    fn signer_list_set_precompute_copies_state_then_runs_base_precompute() {
        let mut called = false;
        let state = run_signer_list_set_precompute(
            SignerListSetDetermineOperationResult {
                result: protocol::Ter::TES_SUCCESS,
                quorum: 3,
                signers: vec![SignerListSetEntry {
                    account: "bob",
                    weight: 1,
                }],
                operation: SignerListSetOperation::Set,
            },
            || called = true,
        );

        assert!(called);
        assert_eq!(state.quorum, 3);
        assert_eq!(state.operation, SignerListSetOperation::Set);
        assert_eq!(state.signers.len(), 1);
    }

    #[test]
    #[should_panic(expected = "SignerListSet::preCompute expects determineOperation success")]
    fn signer_list_set_precompute_panics_on_failed_determine_operation_assert() {
        let _ = run_signer_list_set_precompute(
            SignerListSetDetermineOperationResult::<&'static str> {
                result: protocol::Ter::TEM_BAD_SIGNER,
                quorum: 0,
                signers: Vec::new(),
                operation: SignerListSetOperation::Unknown,
            },
            || {},
        );
    }

    #[test]
    fn signer_list_set_do_apply_dispatches_current_cpp_operations() {
        let set = run_signer_list_set_do_apply(
            SignerListSetOperation::Set,
            || protocol::Ter::TES_SUCCESS,
            || protocol::Ter::TEM_MALFORMED,
        );
        let destroy = run_signer_list_set_do_apply(
            SignerListSetOperation::Destroy,
            || protocol::Ter::TEM_MALFORMED,
            || protocol::Ter::TEC_NO_ALTERNATIVE_KEY,
        );

        assert_eq!(set, protocol::Ter::TES_SUCCESS);
        assert_eq!(destroy, protocol::Ter::TEC_NO_ALTERNATIVE_KEY);
    }

    #[test]
    #[should_panic(expected = "SignerListSet::doApply expects known operation")]
    fn signer_list_set_do_apply_panics_on_unknown_operation_unreachable() {
        let _ = run_signer_list_set_do_apply(
            SignerListSetOperation::Unknown,
            || protocol::Ter::TES_SUCCESS,
            || protocol::Ter::TES_SUCCESS,
        );
    }

    #[test]
    fn signer_list_set_owner_count_delta_matches_current_cpp_rule() {
        assert_eq!(signer_list_set_owner_count_delta(1), 3);
        assert_eq!(signer_list_set_owner_count_delta(8), 10);
    }

    #[test]
    fn signer_list_set_remove_from_ledger_preserves_current_cpp_paths() {
        let remove_missing = run_signer_list_set_remove_from_ledger(
            None::<TestSignerList>,
            |_| panic!("missing list should skip dir remove"),
            |_| panic!("missing list should skip owner count"),
            |_| panic!("missing list should skip erase"),
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
        let one_owner = Cell::new(0);
        let one_owner_result = run_signer_list_set_remove_from_ledger(
            Some(TestSignerList {
                flags: LSF_ONE_OWNER_COUNT,
                signer_entries_len: 5,
                owner_node: 7,
            }),
            |_| true,
            |delta| one_owner.set(delta),
            |_| {},
        );

        assert_eq!(remove_missing, protocol::Ter::TES_SUCCESS);
        assert_eq!(old_style, protocol::Ter::TES_SUCCESS);
        assert_eq!(
            events.into_inner(),
            vec![
                "dir_remove:22".to_string(),
                "adjust:-5".to_string(),
                "erase".to_string(),
            ]
        );
        assert_eq!(one_owner_result, protocol::Ter::TES_SUCCESS);
        assert_eq!(one_owner.get(), -1);
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

        assert_eq!(result, protocol::Ter::TEF_BAD_LEDGER);
        assert!(!adjusted.get());
        assert!(!erased.get());
    }

    #[test]
    fn signer_list_set_destroy_signer_list_preserves_current_cpp_guards() {
        let internal = run_signer_list_set_destroy_signer_list(false, false, false, || {
            protocol::Ter::TES_SUCCESS
        });
        let no_alternative = run_signer_list_set_destroy_signer_list(true, true, false, || {
            protocol::Ter::TES_SUCCESS
        });
        let delegated =
            run_signer_list_set_destroy_signer_list(true, true, true, || protocol::Ter::TEC_OWNERS);

        assert_eq!(internal, protocol::Ter::TEF_INTERNAL);
        assert_eq!(no_alternative, protocol::Ter::TEC_NO_ALTERNATIVE_KEY);
        assert_eq!(delegated, protocol::Ter::TEC_OWNERS);
    }

    #[test]
    fn signer_list_set_replace_signer_list_preserves_current_cpp_guards() {
        let remove_failure = run_signer_list_set_replace_signer_list(
            &10_u32,
            || protocol::Ter::TEF_BAD_LEDGER,
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
            || protocol::Ter::TES_SUCCESS,
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
            || protocol::Ter::TES_SUCCESS,
            Some(TestAccount { owner_count: 2 }),
            |account| account.owner_count,
            |new_owner_count| new_owner_count + 3,
            |_| panic!("reserve failure should skip prepare"),
            || panic!("reserve failure should skip dir insert"),
            |_| panic!("reserve failure should skip owner node"),
            |_, _| panic!("reserve failure should skip owner count"),
        );

        assert_eq!(remove_failure, protocol::Ter::TEF_BAD_LEDGER);
        assert_eq!(missing_account, protocol::Ter::TEF_INTERNAL);
        assert_eq!(reserve_shortfall, protocol::Ter::TEC_INSUFFICIENT_RESERVE);
    }

    #[test]
    fn signer_list_set_replace_signer_list_preserves_current_cpp_success_order() {
        let events = RefCell::new(Vec::new());
        let result = run_signer_list_set_replace_signer_list(
            &10_u32,
            || {
                events.borrow_mut().push("remove".to_string());
                protocol::Ter::TES_SUCCESS
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

        assert_eq!(result, protocol::Ter::TES_SUCCESS);
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
            || protocol::Ter::TES_SUCCESS,
            Some(TestAccount { owner_count: 1 }),
            |account| account.owner_count,
            |_| 5_u32,
            |_| prepared.set(true),
            || None,
            |_| set_owner_node.set(true),
            |_, _| adjusted.set(true),
        );

        assert_eq!(result, protocol::Ter::TEC_DIR_FULL);
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
            &[TaggedSignerEntry {
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
                TaggedSignerEntry {
                    account: "charlie",
                    weight: 2,
                    wallet_locator: Some("tag-1"),
                },
                TaggedSignerEntry {
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
}
