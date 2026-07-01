use protocol::confidential_transfer::EC_CLAWBACK_PROOF_LENGTH;
use protocol::{NotTec, Ter};

pub const MAX_MPTOKEN_AMOUNT_CLAWBACK: u64 = 0x7fff_ffff_ffff_ffff;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfidentialMPTClawbackPreflightFacts {
    pub confidential_transfer_enabled: bool,
    pub account_is_issuer: bool,
    pub account_equals_holder: bool,
    pub claw_amount: u64,
    pub zk_proof_len: usize,
}

pub fn run_confidential_mpt_clawback_preflight(
    facts: &ConfidentialMPTClawbackPreflightFacts,
) -> NotTec {
    if !facts.confidential_transfer_enabled {
        return Ter::TEM_DISABLED;
    }

    if !facts.account_is_issuer {
        return Ter::TEM_MALFORMED;
    }

    if facts.account_equals_holder {
        return Ter::TEM_MALFORMED;
    }

    if facts.claw_amount == 0 || facts.claw_amount > MAX_MPTOKEN_AMOUNT_CLAWBACK {
        return Ter::TEM_BAD_AMOUNT;
    }

    if facts.zk_proof_len != EC_CLAWBACK_PROOF_LENGTH {
        return Ter::TEM_MALFORMED;
    }

    Ter::TES_SUCCESS
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfidentialMPTClawbackPreclaimFacts {
    pub account_exists: bool,
    pub holder_exists: bool,
    pub issuance_exists: bool,
    pub issuance_issuer_matches_account: bool,
    pub issuance_has_issuer_encryption_key: bool,
    pub issuance_can_clawback: bool,
    pub issuance_can_hold_confidential_balance: bool,
    pub holder_mptoken_exists: bool,
    pub holder_has_issuer_encrypted_balance: bool,
    pub holder_has_holder_encryption_key: bool,
    pub claw_amount_within_confidential_outstanding: bool,
    pub claw_amount_within_total_outstanding: bool,
    pub proof_valid: bool,
}

pub fn run_confidential_mpt_clawback_preclaim(
    facts: &ConfidentialMPTClawbackPreclaimFacts,
) -> Ter {
    if !facts.account_exists {
        return Ter::TER_NO_ACCOUNT;
    }

    if !facts.holder_exists {
        return Ter::TEC_NO_TARGET;
    }

    if !facts.issuance_exists {
        return Ter::TEC_OBJECT_NOT_FOUND;
    }

    if !facts.issuance_issuer_matches_account {
        return Ter::TEF_INTERNAL;
    }

    if !facts.issuance_has_issuer_encryption_key {
        return Ter::TEC_NO_PERMISSION;
    }

    if !facts.issuance_can_clawback {
        return Ter::TEC_NO_PERMISSION;
    }

    if !facts.issuance_can_hold_confidential_balance {
        return Ter::TEC_NO_PERMISSION;
    }

    if !facts.holder_mptoken_exists {
        return Ter::TEC_OBJECT_NOT_FOUND;
    }

    if !facts.holder_has_issuer_encrypted_balance {
        return Ter::TEC_NO_PERMISSION;
    }

    if !facts.holder_has_holder_encryption_key {
        return Ter::TEC_NO_PERMISSION;
    }

    if !facts.claw_amount_within_confidential_outstanding
        || !facts.claw_amount_within_total_outstanding
    {
        return Ter::TEC_INSUFFICIENT_FUNDS;
    }

    if !facts.proof_valid {
        return Ter::TEC_BAD_PROOF;
    }

    Ter::TES_SUCCESS
}
