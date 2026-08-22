use std::sync::Arc;

use basics::base_uint::Uint256;
use ledger::{ReadView, Sandbox};
use protocol::{ApplyFlags, LedgerEntryType, STTx, Ter, TxType, get_field_by_symbol};

use super::transactor_dispatcher::handle_real_dispatch;

fn sf(name: &str) -> &'static protocol::SField {
    get_field_by_symbol(name)
}

fn view_at(seq: u32) -> Sandbox<ledger::Ledger> {
    Sandbox::new(
        Arc::new(ledger::Ledger::from_ledger_seq_and_close_time(
            seq, 1_000, false,
        )),
        ApplyFlags::NONE,
    )
}

fn amendment_tx(amendment: Uint256, flags: u32) -> STTx {
    STTx::new(TxType::AMENDMENT, |tx| {
        tx.set_field_h256(sf("sfAmendment"), amendment);
        tx.set_field_u32(sf("sfFlags"), flags);
    })
}

fn unl_modify_tx(seq: u32, disabling: u8, validator: &[u8]) -> STTx {
    STTx::new(TxType::UNL_MODIFY, |tx| {
        tx.set_field_u32(sf("sfLedgerSequence"), seq);
        tx.set_field_u8(sf("sfUNLModifyDisabling"), disabling);
        tx.set_field_vl(sf("sfUNLModifyValidator"), validator);
    })
}

#[test]
fn production_amendment_dispatch_preserves_exact_failure_semantics() {
    let amendment = Uint256::from_array([0x11; 32]);
    let other = Uint256::from_array([0x22; 32]);
    let mut view = view_at(256);

    assert_eq!(
        handle_real_dispatch(
            &mut view,
            &amendment_tx(amendment, protocol::ENABLE_AMENDMENT_GOT_MAJORITY_FLAG),
            TxType::AMENDMENT,
            None,
        ),
        Ter::TES_SUCCESS
    );
    let sle = view
        .read(protocol::amendments_keylet())
        .expect("amendments read should succeed")
        .expect("got-majority must insert typed amendments SLE");
    assert_eq!(sle.get_type(), LedgerEntryType::Amendments);
    assert!(
        !sle.is_field_present(sf("sfAmendments")),
        "a got-majority-only entry must preserve rippled's absent empty vector"
    );
    let majorities = sle.get_field_array(sf("sfMajorities"));
    assert_eq!(majorities.len(), 1);
    assert_eq!(
        majorities
            .iter()
            .next()
            .expect("majority entry should exist")
            .get_field_h256(sf("sfAmendment")),
        amendment
    );

    assert_eq!(
        handle_real_dispatch(
            &mut view,
            &amendment_tx(amendment, protocol::ENABLE_AMENDMENT_GOT_MAJORITY_FLAG),
            TxType::AMENDMENT,
            None,
        ),
        Ter::TEF_ALREADY
    );
    assert_eq!(
        handle_real_dispatch(
            &mut view,
            &amendment_tx(
                other,
                protocol::ENABLE_AMENDMENT_GOT_MAJORITY_FLAG
                    | protocol::ENABLE_AMENDMENT_LOST_MAJORITY_FLAG,
            ),
            TxType::AMENDMENT,
            None,
        ),
        Ter::TEM_INVALID_FLAG
    );
    assert_eq!(
        view.read(protocol::amendments_keylet())
            .unwrap()
            .unwrap()
            .get_field_array(sf("sfMajorities"))
            .len(),
        1,
        "failed pseudo transactions must not mutate the amendments SLE"
    );
}

#[test]
fn production_amendment_dispatch_rejects_duplicate_enable_and_missing_lost_majority() {
    let amendment = Uint256::from_array([0x33; 32]);
    let mut view = view_at(256);
    let enable = amendment_tx(amendment, 0);

    assert_eq!(
        handle_real_dispatch(&mut view, &enable, TxType::AMENDMENT, None),
        Ter::TES_SUCCESS
    );
    assert_eq!(
        handle_real_dispatch(&mut view, &enable, TxType::AMENDMENT, None),
        Ter::TEF_ALREADY
    );
    assert_eq!(
        handle_real_dispatch(
            &mut view,
            &amendment_tx(
                Uint256::from_array([0x44; 32]),
                protocol::ENABLE_AMENDMENT_LOST_MAJORITY_FLAG,
            ),
            TxType::AMENDMENT,
            None,
        ),
        Ter::TEF_ALREADY
    );
    assert_eq!(
        view.read(protocol::amendments_keylet())
            .unwrap()
            .unwrap()
            .get_field_v256(sf("sfAmendments"))
            .value(),
        &[amendment]
    );
}

#[test]
fn production_unl_modify_inserts_typed_sle_and_rejects_conflicts() {
    let mut validator = [0x55; protocol::PUBLIC_KEY_LENGTH];
    validator[0] = 0xED;
    let mut view = view_at(256);
    let disable = unl_modify_tx(256, 1, &validator);

    assert_eq!(
        handle_real_dispatch(&mut view, &disable, TxType::UNL_MODIFY, None),
        Ter::TES_SUCCESS
    );
    let sle = view
        .read(protocol::negative_unl_keylet())
        .expect("negative UNL read should succeed")
        .expect("first UNLModify must insert a typed NegativeUNL SLE");
    assert_eq!(sle.get_type(), LedgerEntryType::NegativeUnl);
    assert_eq!(sle.get_field_vl(sf("sfValidatorToDisable")), validator);

    assert_eq!(
        handle_real_dispatch(&mut view, &disable, TxType::UNL_MODIFY, None),
        Ter::TEF_FAILURE
    );
    assert_eq!(
        view.read(protocol::negative_unl_keylet())
            .unwrap()
            .unwrap()
            .get_field_vl(sf("sfValidatorToDisable")),
        validator
    );
}

#[test]
fn production_unl_modify_rejects_non_flag_wrong_sequence_and_bad_key() {
    let mut validator = [0x66; protocol::PUBLIC_KEY_LENGTH];
    validator[0] = 0x02;

    let mut non_flag = view_at(255);
    assert_eq!(
        handle_real_dispatch(
            &mut non_flag,
            &unl_modify_tx(255, 1, &validator),
            TxType::UNL_MODIFY,
            None,
        ),
        Ter::TEF_FAILURE
    );
    assert!(
        non_flag
            .read(protocol::negative_unl_keylet())
            .unwrap()
            .is_none()
    );

    let mut flag = view_at(256);
    assert_eq!(
        handle_real_dispatch(
            &mut flag,
            &unl_modify_tx(255, 1, &validator),
            TxType::UNL_MODIFY,
            None,
        ),
        Ter::TEF_FAILURE
    );
    let bad_key = [0x77; protocol::PUBLIC_KEY_LENGTH];
    assert_eq!(
        handle_real_dispatch(
            &mut flag,
            &unl_modify_tx(256, 1, &bad_key),
            TxType::UNL_MODIFY,
            None,
        ),
        Ter::TEF_FAILURE
    );
    assert!(
        flag.read(protocol::negative_unl_keylet())
            .unwrap()
            .is_none()
    );
}
