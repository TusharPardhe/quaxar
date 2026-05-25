//! Integration tests that pin the public vault-family preflight composition
//! shell to the current C++ exception, unknown-type, and success-consequence
//! behavior.

use std::cell::RefCell;

use protocol::{Rules, SeqProxy, Ter, TxType};
use tx::{
    ApplyFlags, HasTxnType, PreflightContext, TxConsequences, UNKNOWN_TRANSACTION_TYPE_TER,
    run_vault_public_preflight_for_txn_source, run_vault_public_preflight_for_txn_type,
};

fn empty_rules() -> Rules {
    Rules::new(std::iter::empty())
}

struct StubTx {
    txn_type: TxType,
}

impl HasTxnType for StubTx {
    fn txn_type(&self) -> TxType {
        self.txn_type
    }
}

#[test]
fn vault_public_preflight_routes_create_path_and_uses_normal_success_consequences() {
    let trace = RefCell::new(Vec::new());

    let observed = run_vault_public_preflight_for_txn_type(
        PreflightContext::<_, _, _, ()>::new(
            "registry",
            "tx",
            empty_rules(),
            ApplyFlags::NONE,
            "journal",
        ),
        true,
        TxType::VAULT_CREATE,
        19,
        SeqProxy::sequence(6),
        || {
            trace.borrow_mut().push("create-extra".to_string());
            true
        },
        || panic!("create path should not query set extra-features"),
        || {
            trace.borrow_mut().push("create-mask".to_string());
            0x3ffc_ffff
        },
        |_| {
            trace.borrow_mut().push("preflight1".to_string());
            Ter::TES_SUCCESS
        },
        || {
            trace.borrow_mut().push("create-preflight".to_string());
            Ter::TES_SUCCESS
        },
        || panic!("wrong vault preflight selected"),
        || panic!("wrong vault preflight selected"),
        || panic!("wrong vault preflight selected"),
        || panic!("wrong vault preflight selected"),
        || panic!("wrong vault preflight selected"),
        || {
            trace.borrow_mut().push("preflight2".to_string());
            Ter::TES_SUCCESS
        },
    );

    assert_eq!(observed.ter, Ter::TES_SUCCESS);
    assert_eq!(
        observed.consequences,
        TxConsequences::new(19, SeqProxy::sequence(6))
    );
    assert_eq!(
        trace.into_inner(),
        vec![
            "create-extra",
            "create-mask",
            "preflight1",
            "create-preflight",
            "preflight2"
        ]
    );
}

#[test]
fn vault_public_preflight_preserves_failure_consequences() {
    let observed = run_vault_public_preflight_for_txn_type(
        PreflightContext::<_, _, _, ()>::new(
            "registry",
            "tx",
            empty_rules(),
            ApplyFlags::NONE,
            "journal",
        ),
        true,
        TxType::VAULT_WITHDRAW,
        19,
        SeqProxy::sequence(6),
        || true,
        || true,
        || panic!("withdraw path should not read create mask"),
        |_| Ter::TES_SUCCESS,
        || panic!("wrong vault preflight selected"),
        || panic!("wrong vault preflight selected"),
        || panic!("wrong vault preflight selected"),
        || panic!("wrong vault preflight selected"),
        || Ter::TEM_INVALID_FLAG,
        || panic!("wrong vault preflight selected"),
        || panic!("failed withdraw preflight should skip preflight2"),
    );

    assert_eq!(observed.ter, Ter::TEM_INVALID_FLAG);
    assert_eq!(
        observed.consequences,
        TxConsequences::from_preflight_result(Ter::TEM_INVALID_FLAG)
    );
}

#[test]
fn vault_public_preflight_source_wrapper_preserves_batch_context() {
    let observed = run_vault_public_preflight_for_txn_source(
        PreflightContext::new_batch(
            "registry",
            StubTx {
                txn_type: TxType::VAULT_SET,
            },
            "batch-2",
            empty_rules(),
            ApplyFlags::BATCH,
            "journal",
        ),
        true,
        23,
        SeqProxy::ticket(9),
        || true,
        || true,
        || panic!("set path should not read create mask"),
        |_| Ter::TES_SUCCESS,
        || panic!("wrong vault preflight selected"),
        || Ter::TES_SUCCESS,
        || panic!("wrong vault preflight selected"),
        || panic!("wrong vault preflight selected"),
        || panic!("wrong vault preflight selected"),
        || panic!("wrong vault preflight selected"),
        || Ter::TES_SUCCESS,
    );

    assert_eq!(observed.parent_batch_id, Some("batch-2"));
    assert_eq!(observed.flags, ApplyFlags::BATCH);
    assert_eq!(observed.ter, Ter::TES_SUCCESS);
    assert_eq!(
        observed.consequences,
        TxConsequences::new(23, SeqProxy::ticket(9))
    );
}

#[test]
fn vault_public_preflight_maps_non_vault_type_to_temunknown() {
    let observed = run_vault_public_preflight_for_txn_type(
        PreflightContext::<_, _, _, ()>::new(
            "registry",
            "tx",
            empty_rules(),
            ApplyFlags::NONE,
            "journal",
        ),
        true,
        TxType::PAYMENT,
        13,
        SeqProxy::sequence(4),
        || panic!("unknown type should not query create extra-features"),
        || panic!("unknown type should not query set extra-features"),
        || panic!("unknown type should not query create mask"),
        |_| panic!("unknown type should not run preflight1"),
        || panic!("unknown type should not run create preflight"),
        || panic!("unknown type should not run set preflight"),
        || panic!("unknown type should not run delete preflight"),
        || panic!("unknown type should not run deposit preflight"),
        || panic!("unknown type should not run withdraw preflight"),
        || panic!("unknown type should not run clawback preflight"),
        || panic!("unknown type should not run preflight2"),
    );

    assert_eq!(observed.ter, UNKNOWN_TRANSACTION_TYPE_TER);
    assert_eq!(
        observed.consequences,
        TxConsequences::from_preflight_result(UNKNOWN_TRANSACTION_TYPE_TER)
    );
}
