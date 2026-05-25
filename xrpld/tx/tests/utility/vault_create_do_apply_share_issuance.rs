//! Integration tests that pin the narrowed Rust
//! `VaultCreate.cpp::doApply()` share-issuance creation shell to the current
//! C++ behavior.

use std::{cell::Cell, rc::Rc};

use protocol::{Ter, trans_token};
use tx::{
    VAULT_SHARE_ISSUANCE_SEQUENCE, VaultCreateShareIssuanceInputs, VaultCreateShareIssuanceRequest,
    run_vault_create_do_apply_share_issuance,
};

#[test]
fn vault_create_do_apply_share_issuance_builds_current_cpp_request() {
    let inputs = VaultCreateShareIssuanceInputs {
        pseudo_id: "pseudo",
        mpt_flags: 0x34,
        scale: 6,
        metadata: Some("meta"),
        domain_id: Some("domain"),
    };

    let result = run_vault_create_do_apply_share_issuance(&inputs, |request| {
        assert_eq!(
            request,
            VaultCreateShareIssuanceRequest {
                prior_balance: None,
                account: &"pseudo",
                sequence: VAULT_SHARE_ISSUANCE_SEQUENCE,
                flags: 0x34,
                asset_scale: 6,
                metadata: Some(&"meta"),
                domain_id: Some(&"domain"),
            }
        );
        Ok::<_, Ter>("share-id")
    });

    assert_eq!(result, Ok("share-id"));
}

#[test]
fn vault_create_do_apply_share_issuance_keeps_optional_fields_absent() {
    let inputs = VaultCreateShareIssuanceInputs {
        pseudo_id: "pseudo",
        mpt_flags: 0,
        scale: 0,
        metadata: None::<&'static str>,
        domain_id: None::<&'static str>,
    };

    let result = run_vault_create_do_apply_share_issuance(&inputs, |request| {
        assert_eq!(request.prior_balance, None);
        assert_eq!(request.metadata, None);
        assert_eq!(request.domain_id, None);
        Ok::<_, Ter>("share-id")
    });

    assert_eq!(result, Ok("share-id"));
}

#[test]
fn vault_create_do_apply_share_issuance_returns_create_failure_unchanged() {
    let inputs = VaultCreateShareIssuanceInputs {
        pseudo_id: "pseudo",
        mpt_flags: 9,
        scale: 6,
        metadata: Some("meta"),
        domain_id: Some("domain"),
    };

    let result = run_vault_create_do_apply_share_issuance(&inputs, |_| {
        Err::<&'static str, _>(Ter::TEC_INSUFFICIENT_RESERVE)
    });

    assert_eq!(result, Err(Ter::TEC_INSUFFICIENT_RESERVE));
    assert_eq!(trans_token(result.unwrap_err()), "tecINSUFFICIENT_RESERVE");
}

#[test]
fn vault_create_do_apply_share_issuance_calls_create_once() {
    let calls = Cell::new(0_u32);
    let inputs = VaultCreateShareIssuanceInputs {
        pseudo_id: "pseudo",
        mpt_flags: 9,
        scale: 6,
        metadata: Some("meta"),
        domain_id: Some("domain"),
    };

    let result = run_vault_create_do_apply_share_issuance(&inputs, |_| {
        calls.set(calls.get() + 1);
        Ok::<_, Ter>("share-id")
    });

    assert_eq!(result, Ok("share-id"));
    assert_eq!(calls.get(), 1);
}

#[test]
fn vault_create_do_apply_share_issuance_uses_given_inputs_without_reordering() {
    let seen = Rc::new(std::cell::RefCell::new(Vec::new()));
    let inputs = VaultCreateShareIssuanceInputs {
        pseudo_id: "pseudo",
        mpt_flags: 0x44,
        scale: 18,
        metadata: Some("meta"),
        domain_id: Some("domain"),
    };

    let result = run_vault_create_do_apply_share_issuance(&inputs, {
        let seen = Rc::clone(&seen);
        move |request| {
            seen.borrow_mut().push("create");
            assert_eq!(request.flags, 0x44);
            assert_eq!(request.asset_scale, 18);
            Ok::<_, Ter>("share-id")
        }
    });

    assert_eq!(result, Ok("share-id"));
    assert_eq!(seen.borrow().as_slice(), ["create"]);
}
