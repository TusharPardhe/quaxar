use protocol::confidential_transfer::{
    EC_CONVERT_BACK_PROOF_LENGTH, EC_GAMAL_ENCRYPTED_TOTAL_LENGTH,
};
use protocol::{NotTec, Ter};

pub const MAX_MPTOKEN_AMOUNT_CONVERT_BACK: u64 = 0x7fff_ffff_ffff_ffff;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfidentialMPTConvertBackPreflightFacts {
    pub confidential_transfer_enabled: bool,
    pub account_is_issuer: bool,
    pub mpt_amount: u64,
    pub balance_commitment_valid: bool,
    pub holder_encrypted_amount_len: usize,
    pub issuer_encrypted_amount_len: usize,
    pub has_auditor_encrypted_amount: bool,
    pub auditor_encrypted_amount_len: usize,
    pub holder_ciphertext_valid: bool,
    pub issuer_ciphertext_valid: bool,
    pub auditor_ciphertext_valid: bool,
    pub zk_proof_len: usize,
}

pub fn run_confidential_mpt_convert_back_preflight(
    facts: &ConfidentialMPTConvertBackPreflightFacts,
) -> NotTec {
    if !facts.confidential_transfer_enabled {
        return Ter::TEM_DISABLED;
    }

    if facts.account_is_issuer {
        return Ter::TEM_MALFORMED;
    }

    if facts.mpt_amount == 0 || facts.mpt_amount > MAX_MPTOKEN_AMOUNT_CONVERT_BACK {
        return Ter::TEM_BAD_AMOUNT;
    }

    if !facts.balance_commitment_valid {
        return Ter::TEM_MALFORMED;
    }

    if facts.holder_encrypted_amount_len != EC_GAMAL_ENCRYPTED_TOTAL_LENGTH
        || facts.issuer_encrypted_amount_len != EC_GAMAL_ENCRYPTED_TOTAL_LENGTH
    {
        return Ter::TEM_BAD_CIPHERTEXT;
    }

    if facts.has_auditor_encrypted_amount
        && facts.auditor_encrypted_amount_len != EC_GAMAL_ENCRYPTED_TOTAL_LENGTH
    {
        return Ter::TEM_BAD_CIPHERTEXT;
    }

    if !facts.holder_ciphertext_valid || !facts.issuer_ciphertext_valid {
        return Ter::TEM_BAD_CIPHERTEXT;
    }

    if facts.has_auditor_encrypted_amount && !facts.auditor_ciphertext_valid {
        return Ter::TEM_BAD_CIPHERTEXT;
    }

    if facts.zk_proof_len != EC_CONVERT_BACK_PROOF_LENGTH {
        return Ter::TEM_MALFORMED;
    }

    Ter::TES_SUCCESS
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfidentialMPTConvertBackPreclaimFacts {
    pub issuance_exists: bool,
    pub issuance_can_hold_confidential_balance: bool,
    pub issuance_has_issuer_encryption_key: bool,
    pub issuance_issuer_equals_account: bool,
    pub has_auditor_encrypted_amount: bool,
    pub issuance_has_auditor_encryption_key: bool,
    pub mptoken_exists: bool,
    pub mptoken_has_holder_encryption_key: bool,
    pub mptoken_has_spending_balance: bool,
    pub mptoken_has_issuer_encrypted_balance: bool,
    pub mptoken_has_auditor_encrypted_balance: bool,
    pub confidential_outstanding_sufficient: bool,
    pub account_frozen: bool,
    pub account_authorized: bool,
    pub proofs_valid: bool,
}

pub fn run_confidential_mpt_convert_back_preclaim(
    facts: &ConfidentialMPTConvertBackPreclaimFacts,
) -> Ter {
    if !facts.issuance_exists {
        return Ter::TEC_OBJECT_NOT_FOUND;
    }

    if !facts.issuance_can_hold_confidential_balance
        || !facts.issuance_has_issuer_encryption_key
    {
        return Ter::TEC_NO_PERMISSION;
    }

    let requires_auditor = facts.issuance_has_auditor_encryption_key;
    if requires_auditor != facts.has_auditor_encrypted_amount {
        return Ter::TEC_NO_PERMISSION;
    }

    if facts.issuance_issuer_equals_account {
        return Ter::TEF_INTERNAL;
    }

    if !facts.mptoken_exists {
        return Ter::TEC_OBJECT_NOT_FOUND;
    }

    if !facts.mptoken_has_holder_encryption_key
        || !facts.mptoken_has_spending_balance
        || !facts.mptoken_has_issuer_encrypted_balance
    {
        return Ter::TEC_NO_PERMISSION;
    }

    if requires_auditor && !facts.mptoken_has_auditor_encrypted_balance {
        return Ter::TEF_INTERNAL;
    }

    if !facts.confidential_outstanding_sufficient {
        return Ter::TEC_INSUFFICIENT_FUNDS;
    }

    if facts.account_frozen {
        return Ter::TEC_FROZEN;
    }

    if !facts.account_authorized {
        return Ter::TEC_NO_AUTH;
    }

    if !facts.proofs_valid {
        return Ter::TEC_BAD_PROOF;
    }

    Ter::TES_SUCCESS
}
