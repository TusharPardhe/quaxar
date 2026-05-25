//! Integration tests that pin the shared transactor preflight shells to the
//! current C++ `Transactor.cpp` behavior.

use protocol::{Ter, trans_token};
use tx::{
    LEGACY_NETWORK_ID_MAX, TransactorPreflight0Facts, TransactorPreflight1Facts,
    TransactorPreflight2Facts, Validity, run_transactor_preflight0, run_transactor_preflight1,
    run_transactor_preflight2,
};

#[test]
fn transactor_preflight0_matches_current_network_id_and_flag_rules() {
    let legacy = run_transactor_preflight0(
        TransactorPreflight0Facts {
            node_network_id: LEGACY_NETWORK_ID_MAX,
            network_id_present: true,
            tx_network_id: Some(99),
            ..TransactorPreflight0Facts::default()
        },
        0,
    );
    let modern_missing = run_transactor_preflight0(
        TransactorPreflight0Facts {
            node_network_id: LEGACY_NETWORK_ID_MAX + 1,
            ..TransactorPreflight0Facts::default()
        },
        0,
    );
    let modern_mismatch = run_transactor_preflight0(
        TransactorPreflight0Facts {
            node_network_id: LEGACY_NETWORK_ID_MAX + 1,
            network_id_present: true,
            tx_network_id: Some(7),
            ..TransactorPreflight0Facts::default()
        },
        0,
    );
    let invalid_flags = run_transactor_preflight0(
        TransactorPreflight0Facts {
            tx_network_id: Some(LEGACY_NETWORK_ID_MAX + 1),
            node_network_id: LEGACY_NETWORK_ID_MAX + 1,
            tx_flags: 0x0002_0000,
            ..TransactorPreflight0Facts::default()
        },
        0x0002_0000,
    );

    assert_eq!(legacy, Ter::TEL_NETWORK_ID_MAKES_TX_NON_CANONICAL);
    assert_eq!(modern_missing, Ter::TEL_REQUIRES_NETWORK_ID);
    assert_eq!(modern_mismatch, Ter::TEL_WRONG_NETWORK);
    assert_eq!(invalid_flags, Ter::TEM_INVALID_FLAG);
    assert_eq!(trans_token(modern_missing), "telREQUIRES_NETWORK_ID");
}

#[test]
fn transactor_preflight1_rejects_delegate_and_malformed_shapes_in_current_order() {
    let disabled_delegate = run_transactor_preflight1(
        TransactorPreflight1Facts {
            delegate_present: true,
            permission_delegation_enabled: false,
            ..TransactorPreflight1Facts::default()
        },
        || panic!("disabled delegate should skip preflight0"),
        || panic!("disabled delegate should skip signing-key helper"),
    );
    let self_delegate = run_transactor_preflight1(
        TransactorPreflight1Facts {
            delegate_present: true,
            permission_delegation_enabled: true,
            delegate_equals_account: true,
            ..TransactorPreflight1Facts::default()
        },
        || panic!("self-delegate should skip preflight0"),
        || panic!("self-delegate should skip signing-key helper"),
    );
    let bad_account = run_transactor_preflight1(
        TransactorPreflight1Facts {
            account_is_zero: true,
            fee_is_native: true,
            fee_is_legal: true,
            ..TransactorPreflight1Facts::default()
        },
        || Ter::TES_SUCCESS,
        || Ter::TES_SUCCESS,
    );

    assert_eq!(disabled_delegate, Ter::TEM_DISABLED);
    assert_eq!(self_delegate, Ter::TEM_BAD_SIGNER);
    assert_eq!(bad_account, Ter::TEM_BAD_SRC_ACCOUNT);
}

#[test]
fn transactor_preflight1_preserves_fee_signing_key_and_batch_guards() {
    let bad_fee = run_transactor_preflight1(
        TransactorPreflight1Facts {
            fee_is_native: false,
            ..TransactorPreflight1Facts::default()
        },
        || Ter::TES_SUCCESS,
        || panic!("bad fee should skip signing-key helper"),
    );
    let bad_signing_key = run_transactor_preflight1(
        TransactorPreflight1Facts {
            fee_is_native: true,
            fee_is_legal: true,
            ..TransactorPreflight1Facts::default()
        },
        || Ter::TES_SUCCESS,
        || Ter::TEM_BAD_SIGNATURE,
    );
    let batch_disabled = run_transactor_preflight1(
        TransactorPreflight1Facts {
            fee_is_native: true,
            fee_is_legal: true,
            inner_batch_flag_set: true,
            ..TransactorPreflight1Facts::default()
        },
        || Ter::TES_SUCCESS,
        || Ter::TES_SUCCESS,
    );

    assert_eq!(bad_fee, Ter::TEM_BAD_FEE);
    assert_eq!(bad_signing_key, Ter::TEM_BAD_SIGNATURE);
    assert_eq!(batch_disabled, Ter::TEM_INVALID_FLAG);
}

#[test]
fn transactor_preflight2_keeps_simulate_and_inner_batch_bypass_rules() {
    let simulate = run_transactor_preflight2(
        TransactorPreflight2Facts::default(),
        || Some(Ter::TEM_INVALID),
        || panic!("simulate short-circuit should skip validity"),
    );
    let inner_batch = run_transactor_preflight2(
        TransactorPreflight2Facts {
            inner_batch_flag_set: true,
            batch_enabled: true,
        },
        || None,
        || panic!("inner-batch bypass should skip validity"),
    );

    assert_eq!(simulate, Ter::TEM_INVALID);
    assert_eq!(inner_batch, Ter::TES_SUCCESS);
}

#[test]
fn transactor_preflight2_maps_sigbad_and_accepts_other_validities() {
    let sig_bad = run_transactor_preflight2(
        TransactorPreflight2Facts::default(),
        || None,
        || Validity::SigBad,
    );
    let sig_good = run_transactor_preflight2(
        TransactorPreflight2Facts::default(),
        || None,
        || Validity::SigGoodOnly,
    );

    assert_eq!(sig_bad, Ter::TEM_INVALID);
    assert_eq!(sig_good, Ter::TES_SUCCESS);
}
