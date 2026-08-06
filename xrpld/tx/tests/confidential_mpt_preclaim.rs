//! Quaxar-only Confidential-MPT typed-preclaim contract.
//!
//! The locally audited `../rippled` checkout has no Confidential-MPT
//! transactors. These tests intentionally pin Quaxar's fact-only extension
//! contract rather than claiming rippled parity. They invoke only typed
//! preclaim helpers: no `doApply`, dry-run, or application dispatcher path is
//! used.

use protocol::Ter;
use tx::{
    ConfidentialMPTClawbackPreclaimFacts, ConfidentialMPTConvertBackPreclaimFacts,
    ConfidentialMPTConvertPreclaimFacts, ConfidentialMPTMergeInboxPreclaimFacts,
    ConfidentialMPTSendPreclaimFacts, run_confidential_mpt_clawback_preclaim,
    run_confidential_mpt_convert_back_preclaim, run_confidential_mpt_convert_preclaim,
    run_confidential_mpt_merge_inbox_preclaim, run_confidential_mpt_send_preclaim,
};

fn convert_base() -> ConfidentialMPTConvertPreclaimFacts {
    ConfidentialMPTConvertPreclaimFacts {
        issuance_exists: true,
        issuance_can_hold_confidential_balance: true,
        issuance_has_issuer_encryption_key: true,
        issuance_issuer_equals_account: false,
        has_auditor_encrypted_amount: false,
        issuance_has_auditor_encryption_key: false,
        mptoken_exists: true,
        account_frozen: false,
        account_authorized: true,
        account_has_sufficient_balance: true,
        holder_key_on_ledger: true,
        holder_key_in_tx: false,
        schnorr_proof_valid: true,
        revealed_amount_valid: true,
    }
}

fn merge_inbox_base() -> ConfidentialMPTMergeInboxPreclaimFacts {
    ConfidentialMPTMergeInboxPreclaimFacts {
        issuance_exists: true,
        issuance_can_hold_confidential_balance: true,
        issuance_issuer_equals_account: false,
        mptoken_exists: true,
        mptoken_has_inbox: true,
        mptoken_has_spending_balance: true,
        mptoken_has_holder_encryption_key: true,
        account_frozen: false,
        account_authorized: true,
    }
}

fn convert_back_base() -> ConfidentialMPTConvertBackPreclaimFacts {
    ConfidentialMPTConvertBackPreclaimFacts {
        issuance_exists: true,
        issuance_can_hold_confidential_balance: true,
        issuance_has_issuer_encryption_key: true,
        issuance_issuer_equals_account: false,
        has_auditor_encrypted_amount: false,
        issuance_has_auditor_encryption_key: false,
        mptoken_exists: true,
        mptoken_has_holder_encryption_key: true,
        mptoken_has_spending_balance: true,
        mptoken_has_issuer_encrypted_balance: true,
        mptoken_has_auditor_encrypted_balance: true,
        confidential_outstanding_sufficient: true,
        account_frozen: false,
        account_authorized: true,
        proofs_valid: true,
    }
}

fn send_base() -> ConfidentialMPTSendPreclaimFacts {
    ConfidentialMPTSendPreclaimFacts {
        account_exists: true,
        destination_exists: true,
        destination_requires_dest_tag: false,
        has_destination_tag: false,
        issuance_exists: true,
        issuance_can_transfer: true,
        issuance_can_hold_confidential_balance: true,
        issuance_has_transfer_fee: false,
        issuance_has_issuer_encryption_key: true,
        has_auditor_encrypted_amount: false,
        issuance_has_auditor_encryption_key: false,
        sender_mptoken_exists: true,
        sender_has_holder_encryption_key: true,
        sender_has_spending_balance: true,
        sender_has_issuer_encrypted_balance: true,
        destination_mptoken_exists: true,
        destination_has_holder_encryption_key: true,
        destination_has_inbox: true,
        destination_has_issuer_encrypted_balance: true,
        sender_frozen: false,
        destination_frozen: false,
        sender_authorized: true,
        destination_authorized: true,
        proof_valid: true,
    }
}

fn clawback_base() -> ConfidentialMPTClawbackPreclaimFacts {
    ConfidentialMPTClawbackPreclaimFacts {
        account_exists: true,
        holder_exists: true,
        issuance_exists: true,
        issuance_issuer_matches_account: true,
        issuance_has_issuer_encryption_key: true,
        issuance_can_clawback: true,
        issuance_can_hold_confidential_balance: true,
        holder_mptoken_exists: true,
        holder_has_issuer_encrypted_balance: true,
        holder_has_holder_encryption_key: true,
        claw_amount_within_confidential_outstanding: true,
        claw_amount_within_total_outstanding: true,
        proof_valid: true,
    }
}

#[test]
fn quaxar_confidential_mpt_convert_preclaim_preserves_guard_order() {
    assert_eq!(
        run_confidential_mpt_convert_preclaim(&ConfidentialMPTConvertPreclaimFacts {
            issuance_exists: false,
            mptoken_exists: false,
            account_frozen: true,
            ..convert_base()
        }),
        Ter::TEC_OBJECT_NOT_FOUND
    );
    assert_eq!(
        run_confidential_mpt_convert_preclaim(&ConfidentialMPTConvertPreclaimFacts {
            issuance_can_hold_confidential_balance: false,
            issuance_issuer_equals_account: true,
            ..convert_base()
        }),
        Ter::TEC_NO_PERMISSION
    );
    assert_eq!(
        run_confidential_mpt_convert_preclaim(&ConfidentialMPTConvertPreclaimFacts {
            issuance_issuer_equals_account: true,
            has_auditor_encrypted_amount: true,
            mptoken_exists: false,
            ..convert_base()
        }),
        Ter::TEF_INTERNAL
    );
    assert_eq!(
        run_confidential_mpt_convert_preclaim(&ConfidentialMPTConvertPreclaimFacts {
            issuance_has_auditor_encryption_key: true,
            has_auditor_encrypted_amount: false,
            mptoken_exists: false,
            ..convert_base()
        }),
        Ter::TEC_NO_PERMISSION
    );
    assert_eq!(
        run_confidential_mpt_convert_preclaim(&ConfidentialMPTConvertPreclaimFacts {
            mptoken_exists: false,
            account_frozen: true,
            ..convert_base()
        }),
        Ter::TEC_OBJECT_NOT_FOUND
    );
    assert_eq!(
        run_confidential_mpt_convert_preclaim(&ConfidentialMPTConvertPreclaimFacts {
            account_frozen: true,
            account_authorized: false,
            account_has_sufficient_balance: false,
            ..convert_base()
        }),
        Ter::TEC_FROZEN
    );
    assert_eq!(
        run_confidential_mpt_convert_preclaim(&ConfidentialMPTConvertPreclaimFacts {
            account_authorized: false,
            account_has_sufficient_balance: false,
            ..convert_base()
        }),
        Ter::TEC_NO_AUTH
    );
    assert_eq!(
        run_confidential_mpt_convert_preclaim(&ConfidentialMPTConvertPreclaimFacts {
            account_has_sufficient_balance: false,
            holder_key_on_ledger: false,
            schnorr_proof_valid: false,
            ..convert_base()
        }),
        Ter::TEC_INSUFFICIENT_FUNDS
    );
    assert_eq!(
        run_confidential_mpt_convert_preclaim(&ConfidentialMPTConvertPreclaimFacts {
            holder_key_on_ledger: false,
            holder_key_in_tx: false,
            schnorr_proof_valid: false,
            ..convert_base()
        }),
        Ter::TEC_NO_PERMISSION
    );
    assert_eq!(
        run_confidential_mpt_convert_preclaim(&ConfidentialMPTConvertPreclaimFacts {
            holder_key_in_tx: true,
            schnorr_proof_valid: false,
            ..convert_base()
        }),
        Ter::TEC_DUPLICATE
    );
    assert_eq!(
        run_confidential_mpt_convert_preclaim(&ConfidentialMPTConvertPreclaimFacts {
            schnorr_proof_valid: false,
            revealed_amount_valid: false,
            ..convert_base()
        }),
        Ter::TEC_BAD_PROOF
    );
    assert_eq!(
        run_confidential_mpt_convert_preclaim(&convert_base()),
        Ter::TES_SUCCESS
    );
}

#[test]
fn quaxar_confidential_mpt_merge_inbox_preclaim_preserves_guard_order() {
    assert_eq!(
        run_confidential_mpt_merge_inbox_preclaim(&ConfidentialMPTMergeInboxPreclaimFacts {
            issuance_exists: false,
            mptoken_exists: false,
            account_frozen: true,
            ..merge_inbox_base()
        }),
        Ter::TEC_OBJECT_NOT_FOUND
    );
    assert_eq!(
        run_confidential_mpt_merge_inbox_preclaim(&ConfidentialMPTMergeInboxPreclaimFacts {
            issuance_can_hold_confidential_balance: false,
            issuance_issuer_equals_account: true,
            ..merge_inbox_base()
        }),
        Ter::TEC_NO_PERMISSION
    );
    assert_eq!(
        run_confidential_mpt_merge_inbox_preclaim(&ConfidentialMPTMergeInboxPreclaimFacts {
            issuance_issuer_equals_account: true,
            mptoken_exists: false,
            ..merge_inbox_base()
        }),
        Ter::TEF_INTERNAL
    );
    assert_eq!(
        run_confidential_mpt_merge_inbox_preclaim(&ConfidentialMPTMergeInboxPreclaimFacts {
            mptoken_exists: false,
            mptoken_has_inbox: false,
            ..merge_inbox_base()
        }),
        Ter::TEC_OBJECT_NOT_FOUND
    );
    assert_eq!(
        run_confidential_mpt_merge_inbox_preclaim(&ConfidentialMPTMergeInboxPreclaimFacts {
            mptoken_has_inbox: false,
            account_frozen: true,
            ..merge_inbox_base()
        }),
        Ter::TEC_NO_PERMISSION
    );
    assert_eq!(
        run_confidential_mpt_merge_inbox_preclaim(&ConfidentialMPTMergeInboxPreclaimFacts {
            account_frozen: true,
            account_authorized: false,
            ..merge_inbox_base()
        }),
        Ter::TEC_FROZEN
    );
    assert_eq!(
        run_confidential_mpt_merge_inbox_preclaim(&ConfidentialMPTMergeInboxPreclaimFacts {
            account_authorized: false,
            ..merge_inbox_base()
        }),
        Ter::TEC_NO_AUTH
    );
    assert_eq!(
        run_confidential_mpt_merge_inbox_preclaim(&merge_inbox_base()),
        Ter::TES_SUCCESS
    );
}

#[test]
fn quaxar_confidential_mpt_convert_back_preclaim_preserves_guard_order() {
    assert_eq!(
        run_confidential_mpt_convert_back_preclaim(&ConfidentialMPTConvertBackPreclaimFacts {
            issuance_exists: false,
            issuance_issuer_equals_account: true,
            ..convert_back_base()
        }),
        Ter::TEC_OBJECT_NOT_FOUND
    );
    assert_eq!(
        run_confidential_mpt_convert_back_preclaim(&ConfidentialMPTConvertBackPreclaimFacts {
            issuance_has_issuer_encryption_key: false,
            issuance_has_auditor_encryption_key: true,
            has_auditor_encrypted_amount: false,
            ..convert_back_base()
        }),
        Ter::TEC_NO_PERMISSION
    );
    assert_eq!(
        run_confidential_mpt_convert_back_preclaim(&ConfidentialMPTConvertBackPreclaimFacts {
            issuance_has_auditor_encryption_key: true,
            has_auditor_encrypted_amount: false,
            issuance_issuer_equals_account: true,
            ..convert_back_base()
        }),
        Ter::TEC_NO_PERMISSION
    );
    assert_eq!(
        run_confidential_mpt_convert_back_preclaim(&ConfidentialMPTConvertBackPreclaimFacts {
            issuance_issuer_equals_account: true,
            mptoken_exists: false,
            ..convert_back_base()
        }),
        Ter::TEF_INTERNAL
    );
    assert_eq!(
        run_confidential_mpt_convert_back_preclaim(&ConfidentialMPTConvertBackPreclaimFacts {
            mptoken_exists: false,
            mptoken_has_holder_encryption_key: false,
            ..convert_back_base()
        }),
        Ter::TEC_OBJECT_NOT_FOUND
    );
    assert_eq!(
        run_confidential_mpt_convert_back_preclaim(&ConfidentialMPTConvertBackPreclaimFacts {
            mptoken_has_holder_encryption_key: false,
            confidential_outstanding_sufficient: false,
            ..convert_back_base()
        }),
        Ter::TEC_NO_PERMISSION
    );
    assert_eq!(
        run_confidential_mpt_convert_back_preclaim(&ConfidentialMPTConvertBackPreclaimFacts {
            issuance_has_auditor_encryption_key: true,
            has_auditor_encrypted_amount: true,
            mptoken_has_auditor_encrypted_balance: false,
            confidential_outstanding_sufficient: false,
            ..convert_back_base()
        }),
        Ter::TEF_INTERNAL
    );
    assert_eq!(
        run_confidential_mpt_convert_back_preclaim(&ConfidentialMPTConvertBackPreclaimFacts {
            confidential_outstanding_sufficient: false,
            account_frozen: true,
            ..convert_back_base()
        }),
        Ter::TEC_INSUFFICIENT_FUNDS
    );
    assert_eq!(
        run_confidential_mpt_convert_back_preclaim(&ConfidentialMPTConvertBackPreclaimFacts {
            account_frozen: true,
            account_authorized: false,
            proofs_valid: false,
            ..convert_back_base()
        }),
        Ter::TEC_FROZEN
    );
    assert_eq!(
        run_confidential_mpt_convert_back_preclaim(&ConfidentialMPTConvertBackPreclaimFacts {
            account_authorized: false,
            proofs_valid: false,
            ..convert_back_base()
        }),
        Ter::TEC_NO_AUTH
    );
    assert_eq!(
        run_confidential_mpt_convert_back_preclaim(&ConfidentialMPTConvertBackPreclaimFacts {
            proofs_valid: false,
            ..convert_back_base()
        }),
        Ter::TEC_BAD_PROOF
    );
    assert_eq!(
        run_confidential_mpt_convert_back_preclaim(&convert_back_base()),
        Ter::TES_SUCCESS
    );
}

#[test]
fn quaxar_confidential_mpt_send_preclaim_preserves_guard_order() {
    assert_eq!(
        run_confidential_mpt_send_preclaim(&ConfidentialMPTSendPreclaimFacts {
            account_exists: false,
            destination_exists: false,
            ..send_base()
        }),
        Ter::TER_NO_ACCOUNT
    );
    assert_eq!(
        run_confidential_mpt_send_preclaim(&ConfidentialMPTSendPreclaimFacts {
            destination_exists: false,
            destination_requires_dest_tag: true,
            ..send_base()
        }),
        Ter::TEC_NO_TARGET
    );
    assert_eq!(
        run_confidential_mpt_send_preclaim(&ConfidentialMPTSendPreclaimFacts {
            destination_requires_dest_tag: true,
            has_destination_tag: false,
            issuance_exists: false,
            ..send_base()
        }),
        Ter::TEC_DST_TAG_NEEDED
    );
    assert_eq!(
        run_confidential_mpt_send_preclaim(&ConfidentialMPTSendPreclaimFacts {
            issuance_exists: false,
            issuance_can_transfer: false,
            ..send_base()
        }),
        Ter::TEC_OBJECT_NOT_FOUND
    );
    assert_eq!(
        run_confidential_mpt_send_preclaim(&ConfidentialMPTSendPreclaimFacts {
            issuance_can_transfer: false,
            issuance_can_hold_confidential_balance: false,
            ..send_base()
        }),
        Ter::TEC_NO_AUTH
    );
    assert_eq!(
        run_confidential_mpt_send_preclaim(&ConfidentialMPTSendPreclaimFacts {
            issuance_can_hold_confidential_balance: false,
            issuance_has_transfer_fee: true,
            ..send_base()
        }),
        Ter::TEC_NO_PERMISSION
    );
    assert_eq!(
        run_confidential_mpt_send_preclaim(&ConfidentialMPTSendPreclaimFacts {
            issuance_has_transfer_fee: true,
            issuance_has_issuer_encryption_key: false,
            ..send_base()
        }),
        Ter::TEC_NO_PERMISSION
    );
    assert_eq!(
        run_confidential_mpt_send_preclaim(&ConfidentialMPTSendPreclaimFacts {
            issuance_has_issuer_encryption_key: false,
            issuance_has_auditor_encryption_key: true,
            has_auditor_encrypted_amount: false,
            ..send_base()
        }),
        Ter::TEC_NO_PERMISSION
    );
    assert_eq!(
        run_confidential_mpt_send_preclaim(&ConfidentialMPTSendPreclaimFacts {
            issuance_has_auditor_encryption_key: true,
            has_auditor_encrypted_amount: false,
            sender_mptoken_exists: false,
            ..send_base()
        }),
        Ter::TEC_NO_PERMISSION
    );
    assert_eq!(
        run_confidential_mpt_send_preclaim(&ConfidentialMPTSendPreclaimFacts {
            sender_mptoken_exists: false,
            sender_has_holder_encryption_key: false,
            ..send_base()
        }),
        Ter::TEC_OBJECT_NOT_FOUND
    );
    assert_eq!(
        run_confidential_mpt_send_preclaim(&ConfidentialMPTSendPreclaimFacts {
            sender_has_holder_encryption_key: false,
            destination_mptoken_exists: false,
            ..send_base()
        }),
        Ter::TEC_NO_PERMISSION
    );
    assert_eq!(
        run_confidential_mpt_send_preclaim(&ConfidentialMPTSendPreclaimFacts {
            destination_mptoken_exists: false,
            destination_has_holder_encryption_key: false,
            ..send_base()
        }),
        Ter::TEC_OBJECT_NOT_FOUND
    );
    assert_eq!(
        run_confidential_mpt_send_preclaim(&ConfidentialMPTSendPreclaimFacts {
            destination_has_holder_encryption_key: false,
            sender_frozen: true,
            ..send_base()
        }),
        Ter::TEC_NO_PERMISSION
    );
    assert_eq!(
        run_confidential_mpt_send_preclaim(&ConfidentialMPTSendPreclaimFacts {
            sender_frozen: true,
            destination_frozen: true,
            sender_authorized: false,
            ..send_base()
        }),
        Ter::TEC_FROZEN
    );
    assert_eq!(
        run_confidential_mpt_send_preclaim(&ConfidentialMPTSendPreclaimFacts {
            destination_frozen: true,
            sender_authorized: false,
            ..send_base()
        }),
        Ter::TEC_FROZEN
    );
    assert_eq!(
        run_confidential_mpt_send_preclaim(&ConfidentialMPTSendPreclaimFacts {
            sender_authorized: false,
            destination_authorized: false,
            proof_valid: false,
            ..send_base()
        }),
        Ter::TEC_NO_AUTH
    );
    assert_eq!(
        run_confidential_mpt_send_preclaim(&ConfidentialMPTSendPreclaimFacts {
            destination_authorized: false,
            proof_valid: false,
            ..send_base()
        }),
        Ter::TEC_NO_AUTH
    );
    assert_eq!(
        run_confidential_mpt_send_preclaim(&ConfidentialMPTSendPreclaimFacts {
            proof_valid: false,
            ..send_base()
        }),
        Ter::TEC_BAD_PROOF
    );
    assert_eq!(
        run_confidential_mpt_send_preclaim(&send_base()),
        Ter::TES_SUCCESS
    );
}

#[test]
fn quaxar_confidential_mpt_clawback_preclaim_preserves_guard_order() {
    assert_eq!(
        run_confidential_mpt_clawback_preclaim(&ConfidentialMPTClawbackPreclaimFacts {
            account_exists: false,
            holder_exists: false,
            ..clawback_base()
        }),
        Ter::TER_NO_ACCOUNT
    );
    assert_eq!(
        run_confidential_mpt_clawback_preclaim(&ConfidentialMPTClawbackPreclaimFacts {
            holder_exists: false,
            issuance_exists: false,
            ..clawback_base()
        }),
        Ter::TEC_NO_TARGET
    );
    assert_eq!(
        run_confidential_mpt_clawback_preclaim(&ConfidentialMPTClawbackPreclaimFacts {
            issuance_exists: false,
            issuance_issuer_matches_account: false,
            ..clawback_base()
        }),
        Ter::TEC_OBJECT_NOT_FOUND
    );
    assert_eq!(
        run_confidential_mpt_clawback_preclaim(&ConfidentialMPTClawbackPreclaimFacts {
            issuance_issuer_matches_account: false,
            issuance_has_issuer_encryption_key: false,
            ..clawback_base()
        }),
        Ter::TEF_INTERNAL
    );
    assert_eq!(
        run_confidential_mpt_clawback_preclaim(&ConfidentialMPTClawbackPreclaimFacts {
            issuance_has_issuer_encryption_key: false,
            issuance_can_clawback: false,
            ..clawback_base()
        }),
        Ter::TEC_NO_PERMISSION
    );
    assert_eq!(
        run_confidential_mpt_clawback_preclaim(&ConfidentialMPTClawbackPreclaimFacts {
            issuance_can_clawback: false,
            issuance_can_hold_confidential_balance: false,
            ..clawback_base()
        }),
        Ter::TEC_NO_PERMISSION
    );
    assert_eq!(
        run_confidential_mpt_clawback_preclaim(&ConfidentialMPTClawbackPreclaimFacts {
            issuance_can_hold_confidential_balance: false,
            holder_mptoken_exists: false,
            ..clawback_base()
        }),
        Ter::TEC_NO_PERMISSION
    );
    assert_eq!(
        run_confidential_mpt_clawback_preclaim(&ConfidentialMPTClawbackPreclaimFacts {
            holder_mptoken_exists: false,
            holder_has_issuer_encrypted_balance: false,
            ..clawback_base()
        }),
        Ter::TEC_OBJECT_NOT_FOUND
    );
    assert_eq!(
        run_confidential_mpt_clawback_preclaim(&ConfidentialMPTClawbackPreclaimFacts {
            holder_has_issuer_encrypted_balance: false,
            holder_has_holder_encryption_key: false,
            ..clawback_base()
        }),
        Ter::TEC_NO_PERMISSION
    );
    assert_eq!(
        run_confidential_mpt_clawback_preclaim(&ConfidentialMPTClawbackPreclaimFacts {
            holder_has_holder_encryption_key: false,
            claw_amount_within_confidential_outstanding: false,
            ..clawback_base()
        }),
        Ter::TEC_NO_PERMISSION
    );
    assert_eq!(
        run_confidential_mpt_clawback_preclaim(&ConfidentialMPTClawbackPreclaimFacts {
            claw_amount_within_confidential_outstanding: false,
            claw_amount_within_total_outstanding: false,
            proof_valid: false,
            ..clawback_base()
        }),
        Ter::TEC_INSUFFICIENT_FUNDS
    );
    assert_eq!(
        run_confidential_mpt_clawback_preclaim(&ConfidentialMPTClawbackPreclaimFacts {
            proof_valid: false,
            ..clawback_base()
        }),
        Ter::TEC_BAD_PROOF
    );
    assert_eq!(
        run_confidential_mpt_clawback_preclaim(&clawback_base()),
        Ter::TES_SUCCESS
    );
}
