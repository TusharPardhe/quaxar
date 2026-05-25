//! Integration tests that pin the narrowed Rust
//! `VaultDelete.cpp::doApply()` issuance-destruction shell to the current C++
//! behavior.

use protocol::{Ter, trans_token};
use tx::{VaultDeleteDoApplyIssuanceVault, run_vault_delete_do_apply_issuance};

#[derive(Debug, Clone, PartialEq, Eq)]
struct TestVault {
    share_mpt_id: &'static str,
}

impl VaultDeleteDoApplyIssuanceVault for TestVault {
    type ShareMptId = &'static str;

    fn share_mpt_id(&self) -> &Self::ShareMptId {
        &self.share_mpt_id
    }
}

fn test_vault() -> TestVault {
    TestVault {
        share_mpt_id: "share-mpt",
    }
}

#[test]
fn vault_delete_do_apply_issuance_returns_tefinternal_when_issuance_is_missing() {
    let result = run_vault_delete_do_apply_issuance(
        &test_vault(),
        |_| None::<&'static str>,
        |_| false,
        |_| Ter::TES_SUCCESS,
        |_| true,
        || {},
        |_| {},
    );

    assert_eq!(result, Ter::TEF_INTERNAL);
    assert_eq!(trans_token(result), "tefINTERNAL");
}

#[test]
fn vault_delete_do_apply_issuance_returns_owner_holding_failure_unchanged() {
    let result = run_vault_delete_do_apply_issuance(
        &test_vault(),
        |_| Some("issuance"),
        |_| true,
        |share_mpt_id| {
            assert_eq!(*share_mpt_id, "share-mpt");
            Ter::TEC_INTERNAL
        },
        |_| true,
        || {},
        |_| {},
    );

    assert_eq!(result, Ter::TEC_INTERNAL);
    assert_eq!(trans_token(result), "tecINTERNAL");
}

#[test]
fn vault_delete_do_apply_issuance_maps_dir_remove_failure_to_tefbad_ledger() {
    let result = run_vault_delete_do_apply_issuance(
        &test_vault(),
        |_| Some("issuance"),
        |_| false,
        |_| Ter::TES_SUCCESS,
        |_| false,
        || unreachable!("dir remove failure should skip owner-count adjustment"),
        |_| unreachable!("dir remove failure should skip issuance erase"),
    );

    assert_eq!(result, Ter::TEF_BAD_LEDGER);
    assert_eq!(trans_token(result), "tefBAD_LEDGER");
}

#[test]
fn vault_delete_do_apply_issuance_runs_success_path() {
    let mut adjusted = false;
    let mut erased = false;

    let result = run_vault_delete_do_apply_issuance(
        &test_vault(),
        |share_mpt_id| {
            assert_eq!(*share_mpt_id, "share-mpt");
            Some("issuance")
        },
        |_| true,
        |_| Ter::TES_SUCCESS,
        |issuance| {
            assert_eq!(*issuance, "issuance");
            true
        },
        || adjusted = true,
        |issuance| {
            assert_eq!(issuance, "issuance");
            erased = true;
        },
    );

    assert_eq!(result, Ter::TES_SUCCESS);
    assert!(adjusted);
    assert!(erased);
}
