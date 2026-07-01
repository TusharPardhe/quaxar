use protocol::confidential_transfer::{
    EC_GAMAL_ENCRYPTED_TOTAL_LENGTH, EC_SCHNORR_PROOF_LENGTH,
};
use protocol::{NotTec, Ter};

pub const MAX_MPTOKEN_AMOUNT_CONVERT: u64 = 0x7fff_ffff_ffff_ffff;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfidentialMPTConvertPreflightFacts {
    pub confidential_transfer_enabled: bool,
    pub account_is_issuer: bool,
    pub mpt_amount: u64,
    pub has_holder_encryption_key: bool,
    pub holder_encryption_key_valid: bool,
    pub has_zk_proof: bool,
    pub zk_proof_len: usize,
    pub holder_encrypted_amount_len: usize,
    pub issuer_encrypted_amount_len: usize,
    pub has_auditor_encrypted_amount: bool,
    pub auditor_encrypted_amount_len: usize,
    pub holder_ciphertext_valid: bool,
    pub issuer_ciphertext_valid: bool,
    pub auditor_ciphertext_valid: bool,
}

pub fn run_confidential_mpt_convert_preflight(
    facts: &ConfidentialMPTConvertPreflightFacts,
) -> NotTec {
    if !facts.confidential_transfer_enabled {
        return Ter::TEM_DISABLED;
    }

    if facts.account_is_issuer {
        return Ter::TEM_MALFORMED;
    }

    if facts.mpt_amount > MAX_MPTOKEN_AMOUNT_CONVERT {
        return Ter::TEM_BAD_AMOUNT;
    }

    if facts.has_holder_encryption_key {
        if !facts.holder_encryption_key_valid {
            return Ter::TEM_MALFORMED;
        }
        if !facts.has_zk_proof {
            return Ter::TEM_MALFORMED;
        }
        if facts.zk_proof_len != EC_SCHNORR_PROOF_LENGTH {
            return Ter::TEM_MALFORMED;
        }
    } else if facts.has_zk_proof {
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

    Ter::TES_SUCCESS
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfidentialMPTConvertPreclaimFacts {
    pub issuance_exists: bool,
    pub issuance_can_hold_confidential_balance: bool,
    pub issuance_has_issuer_encryption_key: bool,
    pub issuance_issuer_equals_account: bool,
    pub has_auditor_encrypted_amount: bool,
    pub issuance_has_auditor_encryption_key: bool,
    pub mptoken_exists: bool,
    pub account_frozen: bool,
    pub account_authorized: bool,
    pub account_has_sufficient_balance: bool,
    pub holder_key_on_ledger: bool,
    pub holder_key_in_tx: bool,
    pub schnorr_proof_valid: bool,
    pub revealed_amount_valid: bool,
}

pub fn run_confidential_mpt_convert_preclaim(
    facts: &ConfidentialMPTConvertPreclaimFacts,
) -> Ter {
    if !facts.issuance_exists {
        return Ter::TEC_OBJECT_NOT_FOUND;
    }

    if !facts.issuance_can_hold_confidential_balance
        || !facts.issuance_has_issuer_encryption_key
    {
        return Ter::TEC_NO_PERMISSION;
    }

    if facts.issuance_issuer_equals_account {
        return Ter::TEF_INTERNAL;
    }

    let requires_auditor = facts.issuance_has_auditor_encryption_key;
    if requires_auditor != facts.has_auditor_encrypted_amount {
        return Ter::TEC_NO_PERMISSION;
    }

    if !facts.mptoken_exists {
        return Ter::TEC_OBJECT_NOT_FOUND;
    }

    if facts.account_frozen {
        return Ter::TEC_FROZEN;
    }

    if !facts.account_authorized {
        return Ter::TEC_NO_AUTH;
    }

    if !facts.account_has_sufficient_balance {
        return Ter::TEC_INSUFFICIENT_FUNDS;
    }

    if !facts.holder_key_on_ledger && !facts.holder_key_in_tx {
        return Ter::TEC_NO_PERMISSION;
    }

    if facts.holder_key_on_ledger && facts.holder_key_in_tx {
        return Ter::TEC_DUPLICATE;
    }

    if !facts.schnorr_proof_valid || !facts.revealed_amount_valid {
        return Ter::TEC_BAD_PROOF;
    }

    Ter::TES_SUCCESS
}
