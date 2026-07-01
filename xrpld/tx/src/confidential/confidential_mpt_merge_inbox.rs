use protocol::{NotTec, Ter};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfidentialMPTMergeInboxPreflightFacts {
    pub confidential_transfer_enabled: bool,
    pub account_is_issuer: bool,
}

pub fn run_confidential_mpt_merge_inbox_preflight(
    facts: &ConfidentialMPTMergeInboxPreflightFacts,
) -> NotTec {
    if !facts.confidential_transfer_enabled {
        return Ter::TEM_DISABLED;
    }

    if facts.account_is_issuer {
        return Ter::TEM_MALFORMED;
    }

    Ter::TES_SUCCESS
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfidentialMPTMergeInboxPreclaimFacts {
    pub issuance_exists: bool,
    pub issuance_can_hold_confidential_balance: bool,
    pub issuance_issuer_equals_account: bool,
    pub mptoken_exists: bool,
    pub mptoken_has_inbox: bool,
    pub mptoken_has_spending_balance: bool,
    pub mptoken_has_holder_encryption_key: bool,
    pub account_frozen: bool,
    pub account_authorized: bool,
}

pub fn run_confidential_mpt_merge_inbox_preclaim(
    facts: &ConfidentialMPTMergeInboxPreclaimFacts,
) -> Ter {
    if !facts.issuance_exists {
        return Ter::TEC_OBJECT_NOT_FOUND;
    }

    if !facts.issuance_can_hold_confidential_balance {
        return Ter::TEC_NO_PERMISSION;
    }

    if facts.issuance_issuer_equals_account {
        return Ter::TEF_INTERNAL;
    }

    if !facts.mptoken_exists {
        return Ter::TEC_OBJECT_NOT_FOUND;
    }

    if !facts.mptoken_has_inbox
        || !facts.mptoken_has_spending_balance
        || !facts.mptoken_has_holder_encryption_key
    {
        return Ter::TEC_NO_PERMISSION;
    }

    if facts.account_frozen {
        return Ter::TEC_FROZEN;
    }

    if !facts.account_authorized {
        return Ter::TEC_NO_AUTH;
    }

    Ter::TES_SUCCESS
}
