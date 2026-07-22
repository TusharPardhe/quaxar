use protocol::confidential_transfer::{EC_GAMAL_ENCRYPTED_TOTAL_LENGTH, EC_SEND_PROOF_LENGTH};
use protocol::{NotTec, Ter};

pub const MAX_MPTOKEN_AMOUNT: u64 = 0x7fff_ffff_ffff_ffff;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfidentialMPTSendPreflightFacts {
    pub confidential_transfer_enabled: bool,
    pub credentials_enabled: bool,
    pub has_credential_ids: bool,
    pub account_is_issuer: bool,
    pub account_equals_destination: bool,
    pub destination_is_issuer: bool,
    pub sender_encrypted_amount_len: usize,
    pub destination_encrypted_amount_len: usize,
    pub issuer_encrypted_amount_len: usize,
    pub has_auditor_encrypted_amount: bool,
    pub auditor_encrypted_amount_len: usize,
    pub zk_proof_len: usize,
    pub balance_commitment_valid: bool,
    pub amount_commitment_valid: bool,
    pub sender_ciphertext_valid: bool,
    pub destination_ciphertext_valid: bool,
    pub issuer_ciphertext_valid: bool,
    pub auditor_ciphertext_valid: bool,
}

pub fn run_confidential_mpt_send_preflight(facts: &ConfidentialMPTSendPreflightFacts) -> NotTec {
    if !facts.confidential_transfer_enabled {
        return Ter::TEM_DISABLED;
    }

    if facts.account_is_issuer {
        return Ter::TEM_MALFORMED;
    }

    if facts.account_equals_destination {
        return Ter::TEM_MALFORMED;
    }

    if facts.destination_is_issuer {
        return Ter::TEM_MALFORMED;
    }

    if facts.sender_encrypted_amount_len != EC_GAMAL_ENCRYPTED_TOTAL_LENGTH
        || facts.destination_encrypted_amount_len != EC_GAMAL_ENCRYPTED_TOTAL_LENGTH
        || facts.issuer_encrypted_amount_len != EC_GAMAL_ENCRYPTED_TOTAL_LENGTH
    {
        return Ter::TEM_BAD_CIPHERTEXT;
    }

    if facts.has_auditor_encrypted_amount
        && facts.auditor_encrypted_amount_len != EC_GAMAL_ENCRYPTED_TOTAL_LENGTH
    {
        return Ter::TEM_BAD_CIPHERTEXT;
    }

    if facts.zk_proof_len != EC_SEND_PROOF_LENGTH {
        return Ter::TEM_MALFORMED;
    }

    if !facts.balance_commitment_valid || !facts.amount_commitment_valid {
        return Ter::TEM_MALFORMED;
    }

    if !facts.sender_ciphertext_valid
        || !facts.destination_ciphertext_valid
        || !facts.issuer_ciphertext_valid
    {
        return Ter::TEM_BAD_CIPHERTEXT;
    }

    if facts.has_auditor_encrypted_amount && !facts.auditor_ciphertext_valid {
        return Ter::TEM_BAD_CIPHERTEXT;
    }

    if facts.has_credential_ids && !facts.credentials_enabled {
        return Ter::TEM_DISABLED;
    }

    Ter::TES_SUCCESS
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfidentialMPTSendPreclaimFacts {
    pub account_exists: bool,
    pub destination_exists: bool,
    pub destination_requires_dest_tag: bool,
    pub has_destination_tag: bool,
    pub issuance_exists: bool,
    pub issuance_can_transfer: bool,
    pub issuance_can_hold_confidential_balance: bool,
    pub issuance_has_transfer_fee: bool,
    pub issuance_has_issuer_encryption_key: bool,
    pub has_auditor_encrypted_amount: bool,
    pub issuance_has_auditor_encryption_key: bool,
    pub sender_mptoken_exists: bool,
    pub sender_has_holder_encryption_key: bool,
    pub sender_has_spending_balance: bool,
    pub sender_has_issuer_encrypted_balance: bool,
    pub destination_mptoken_exists: bool,
    pub destination_has_holder_encryption_key: bool,
    pub destination_has_inbox: bool,
    pub destination_has_issuer_encrypted_balance: bool,
    pub sender_frozen: bool,
    pub destination_frozen: bool,
    pub sender_authorized: bool,
    pub destination_authorized: bool,
    pub proof_valid: bool,
}

pub fn run_confidential_mpt_send_preclaim(facts: &ConfidentialMPTSendPreclaimFacts) -> Ter {
    if !facts.account_exists {
        return Ter::TER_NO_ACCOUNT;
    }

    if !facts.destination_exists {
        return Ter::TEC_NO_TARGET;
    }

    if facts.destination_requires_dest_tag && !facts.has_destination_tag {
        return Ter::TEC_DST_TAG_NEEDED;
    }

    if !facts.issuance_exists {
        return Ter::TEC_OBJECT_NOT_FOUND;
    }

    if !facts.issuance_can_transfer {
        return Ter::TEC_NO_AUTH;
    }

    if !facts.issuance_can_hold_confidential_balance {
        return Ter::TEC_NO_PERMISSION;
    }

    if facts.issuance_has_transfer_fee {
        return Ter::TEC_NO_PERMISSION;
    }

    if !facts.issuance_has_issuer_encryption_key {
        return Ter::TEC_NO_PERMISSION;
    }

    let requires_auditor = facts.issuance_has_auditor_encryption_key;
    if requires_auditor != facts.has_auditor_encrypted_amount {
        return Ter::TEC_NO_PERMISSION;
    }

    if !facts.sender_mptoken_exists {
        return Ter::TEC_OBJECT_NOT_FOUND;
    }

    if !facts.sender_has_holder_encryption_key
        || !facts.sender_has_spending_balance
        || !facts.sender_has_issuer_encrypted_balance
    {
        return Ter::TEC_NO_PERMISSION;
    }

    if !facts.destination_mptoken_exists {
        return Ter::TEC_OBJECT_NOT_FOUND;
    }

    if !facts.destination_has_holder_encryption_key
        || !facts.destination_has_inbox
        || !facts.destination_has_issuer_encrypted_balance
    {
        return Ter::TEC_NO_PERMISSION;
    }

    if facts.sender_frozen {
        return Ter::TEC_FROZEN;
    }

    if facts.destination_frozen {
        return Ter::TEC_FROZEN;
    }

    if !facts.sender_authorized {
        return Ter::TEC_NO_AUTH;
    }

    if !facts.destination_authorized {
        return Ter::TEC_NO_AUTH;
    }

    if !facts.proof_valid {
        return Ter::TEC_BAD_PROOF;
    }

    Ter::TES_SUCCESS
}
