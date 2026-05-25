//! Narrow the reference implementation compatibility surface.
//!
//! This ports the current deterministic behavior for:
//!
//! - the `fixInvalidTxFlags`-gated `getFlagsMask(...)` return,
//! - `preflight(...)` issuer / credential-type validation,
//! - `preclaim(...)` issuer, existence, and duplicate-accept checks,
//! - and the current loaded `doApply()` sequencing around reserve,
//!   expiration cleanup, flag mutation, and owner-count transfer.

use crate::credential_create::{CREDENTIAL_ACCEPTED_FLAG, CREDENTIAL_MAX_TYPE_LENGTH};
use protocol::{NotTec, Ter, tfUniversalMask};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CredentialAcceptPreflightFacts {
    pub issuer_present: bool,
    pub credential_type_len: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CredentialAcceptPreclaimFacts {
    pub issuer_exists: bool,
    pub credential_exists: bool,
    pub credential_accepted: bool,
}

pub trait CredentialAcceptApplySink {
    fn subject_exists(&mut self) -> bool;
    fn issuer_exists(&mut self) -> bool;
    fn subject_owner_count(&mut self) -> u32;
    fn subject_has_reserve(&mut self, owner_count_after: u32) -> bool;
    fn credential_exists(&mut self) -> bool;
    fn credential_expired(&mut self) -> bool;
    fn delete_credential(&mut self) -> Ter;
    fn set_credential_flags(&mut self, flags: u32);
    fn update_credential(&mut self);
    fn adjust_issuer_owner_count(&mut self, delta: i32);
    fn adjust_subject_owner_count(&mut self, delta: i32);
}

pub const fn get_credential_accept_flags_mask(fix_invalid_tx_flags_enabled: bool) -> u32 {
    if fix_invalid_tx_flags_enabled {
        tfUniversalMask
    } else {
        0
    }
}

pub fn run_credential_accept_preflight(facts: CredentialAcceptPreflightFacts) -> NotTec {
    if !facts.issuer_present {
        return Ter::TEM_INVALID_ACCOUNT_ID;
    }

    if facts.credential_type_len == 0 || facts.credential_type_len > CREDENTIAL_MAX_TYPE_LENGTH {
        return Ter::TEM_MALFORMED;
    }

    Ter::TES_SUCCESS
}

pub fn run_credential_accept_preclaim(facts: CredentialAcceptPreclaimFacts) -> Ter {
    if !facts.issuer_exists {
        return Ter::TEC_NO_ISSUER;
    }

    if !facts.credential_exists {
        return Ter::TEC_NO_ENTRY;
    }

    if facts.credential_accepted {
        return Ter::TEC_DUPLICATE;
    }

    Ter::TES_SUCCESS
}

pub fn run_credential_accept_do_apply<S>(sink: &mut S) -> Ter
where
    S: CredentialAcceptApplySink,
{
    let subject_exists = sink.subject_exists();
    let issuer_exists = sink.issuer_exists();
    if !subject_exists || !issuer_exists {
        return Ter::TEF_INTERNAL;
    }

    let owner_count_after = sink.subject_owner_count().saturating_add(1);
    if !sink.subject_has_reserve(owner_count_after) {
        return Ter::TEC_INSUFFICIENT_RESERVE;
    }

    if !sink.credential_exists() {
        return Ter::TEF_INTERNAL;
    }

    if sink.credential_expired() {
        let err = sink.delete_credential();
        return if err == Ter::TES_SUCCESS {
            Ter::TEC_EXPIRED
        } else {
            err
        };
    }

    sink.set_credential_flags(CREDENTIAL_ACCEPTED_FLAG);
    sink.update_credential();
    sink.adjust_issuer_owner_count(-1);
    sink.adjust_subject_owner_count(1);
    Ter::TES_SUCCESS
}

#[cfg(test)]
mod tests {
    use super::{CredentialAcceptPreclaimFacts, run_credential_accept_preclaim};
    use protocol::Ter;

    #[test]
    fn credential_accept_preclaim_rejects_no_issuer_before_no_entry() {
        // When both issuer and credential are absent, tecNO_ISSUER is returned (not tecNO_ENTRY).
        let result = run_credential_accept_preclaim(CredentialAcceptPreclaimFacts {
            issuer_exists: false,
            credential_exists: false,
            credential_accepted: false,
        });

        assert_eq!(result, Ter::TEC_NO_ISSUER);
    }

    #[test]
    fn credential_accept_preclaim_rejects_no_entry_when_issuer_exists() {
        let result = run_credential_accept_preclaim(CredentialAcceptPreclaimFacts {
            issuer_exists: true,
            credential_exists: false,
            credential_accepted: false,
        });

        assert_eq!(result, Ter::TEC_NO_ENTRY);
    }

    #[test]
    fn credential_accept_preclaim_rejects_duplicate_when_already_accepted() {
        let result = run_credential_accept_preclaim(CredentialAcceptPreclaimFacts {
            issuer_exists: true,
            credential_exists: true,
            credential_accepted: true,
        });

        assert_eq!(result, Ter::TEC_DUPLICATE);
    }

    #[test]
    fn credential_accept_preclaim_succeeds_when_all_checks_pass() {
        let result = run_credential_accept_preclaim(CredentialAcceptPreclaimFacts {
            issuer_exists: true,
            credential_exists: true,
            credential_accepted: false,
        });

        assert_eq!(result, Ter::TES_SUCCESS);
    }
}
