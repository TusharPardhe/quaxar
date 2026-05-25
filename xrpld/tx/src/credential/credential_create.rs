//! Narrow the reference implementation compatibility surface.
//!
//! This ports the current deterministic behavior for:
//!
//! - the `fixInvalidTxFlags`-gated `getFlagsMask(...)` return,
//! - `preflight(...)` subject / URI / credential-type validation,
//! - `preclaim(...)` subject-target and duplicate checks,
//! - and the current loaded `doApply()` sequencing around expiration,
//!   reserve, owner-directory linking, and self-accept handling.

use protocol::{ACCEPTED_LEDGER_FLAG, NotTec, Ter, tfUniversalMask};

pub const CREDENTIAL_MAX_URI_LENGTH: usize = 256;
pub const CREDENTIAL_MAX_TYPE_LENGTH: usize = 64;
pub const CREDENTIAL_ACCEPTED_FLAG: u32 = ACCEPTED_LEDGER_FLAG;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CredentialCreatePreflightFacts {
    pub subject_present: bool,
    pub uri_len: Option<usize>,
    pub credential_type_len: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CredentialCreatePreclaimFacts {
    pub subject_exists: bool,
    pub credential_exists: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CredentialCreateApplyFacts<AccountId, CredentialType, Uri> {
    pub subject: AccountId,
    pub issuer: AccountId,
    pub credential_type: CredentialType,
    pub uri: Option<Uri>,
    pub expiration: Option<u32>,
    pub close_time: u32,
}

pub trait CredentialCreateApplySink {
    type AccountId: Clone + Eq;
    type CredentialType: Clone;
    type Uri: Clone;
    type OwnerNode: Copy;

    fn begin_credential(&mut self) -> bool;
    fn set_expiration(&mut self, expiration: u32);
    fn issuer_exists(&mut self) -> bool;
    fn issuer_owner_count(&mut self) -> u32;
    fn issuer_has_reserve(&mut self, owner_count_after: u32) -> bool;
    fn set_subject(&mut self, subject: Self::AccountId);
    fn set_issuer(&mut self, issuer: Self::AccountId);
    fn set_credential_type(&mut self, credential_type: Self::CredentialType);
    fn set_uri(&mut self, uri: Self::Uri);
    fn insert_issuer_directory(&mut self) -> Option<Self::OwnerNode>;
    fn set_issuer_node(&mut self, page: Self::OwnerNode);
    fn adjust_issuer_owner_count(&mut self, delta: i32);
    fn set_flags(&mut self, flags: u32);
    fn insert_subject_directory(&mut self) -> Option<Self::OwnerNode>;
    fn set_subject_node(&mut self, page: Self::OwnerNode);
    fn insert_credential(&mut self);
}

pub const fn get_credential_create_flags_mask(fix_invalid_tx_flags_enabled: bool) -> u32 {
    if fix_invalid_tx_flags_enabled {
        tfUniversalMask
    } else {
        0
    }
}

pub fn run_credential_create_preflight(facts: CredentialCreatePreflightFacts) -> NotTec {
    if !facts.subject_present {
        return Ter::TEM_MALFORMED;
    }

    if let Some(uri_len) = facts.uri_len
        && (uri_len == 0 || uri_len > CREDENTIAL_MAX_URI_LENGTH)
    {
        return Ter::TEM_MALFORMED;
    }

    if facts.credential_type_len == 0 || facts.credential_type_len > CREDENTIAL_MAX_TYPE_LENGTH {
        return Ter::TEM_MALFORMED;
    }

    Ter::TES_SUCCESS
}

pub fn run_credential_create_preclaim(facts: CredentialCreatePreclaimFacts) -> Ter {
    if !facts.subject_exists {
        return Ter::TEC_NO_TARGET;
    }

    if facts.credential_exists {
        return Ter::TEC_DUPLICATE;
    }

    Ter::TES_SUCCESS
}

pub fn run_credential_create_do_apply<S>(
    facts: CredentialCreateApplyFacts<S::AccountId, S::CredentialType, S::Uri>,
    sink: &mut S,
) -> Ter
where
    S: CredentialCreateApplySink,
{
    if !sink.begin_credential() {
        return Ter::TEF_INTERNAL;
    }

    if let Some(expiration) = facts.expiration {
        if facts.close_time > expiration {
            return Ter::TEC_EXPIRED;
        }
        sink.set_expiration(expiration);
    }

    if !sink.issuer_exists() {
        return Ter::TEF_INTERNAL;
    }

    let owner_count_after = sink.issuer_owner_count().saturating_add(1);
    if !sink.issuer_has_reserve(owner_count_after) {
        return Ter::TEC_INSUFFICIENT_RESERVE;
    }

    sink.set_subject(facts.subject.clone());
    sink.set_issuer(facts.issuer.clone());
    sink.set_credential_type(facts.credential_type.clone());

    if let Some(uri) = facts.uri {
        sink.set_uri(uri);
    }

    let Some(issuer_page) = sink.insert_issuer_directory() else {
        return Ter::TEC_DIR_FULL;
    };
    sink.set_issuer_node(issuer_page);
    sink.adjust_issuer_owner_count(1);

    if facts.subject == facts.issuer {
        sink.set_flags(CREDENTIAL_ACCEPTED_FLAG);
    } else {
        let Some(subject_page) = sink.insert_subject_directory() else {
            return Ter::TEC_DIR_FULL;
        };
        sink.set_subject_node(subject_page);
    }

    sink.insert_credential();
    Ter::TES_SUCCESS
}

#[cfg(test)]
mod tests {
    use super::{CredentialCreatePreclaimFacts, run_credential_create_preclaim};
    use protocol::Ter;

    #[test]
    fn credential_create_preclaim_rejects_missing_subject() {
        let result = run_credential_create_preclaim(CredentialCreatePreclaimFacts {
            subject_exists: false,
            credential_exists: false,
        });

        assert_eq!(result, Ter::TEC_NO_TARGET);
    }

    #[test]
    fn credential_create_preclaim_returns_no_target_even_when_credential_exists() {
        // exists, the missing-subject path returns tecNO_TARGET — not tecDUPLICATE.
        let result = run_credential_create_preclaim(CredentialCreatePreclaimFacts {
            subject_exists: false,
            credential_exists: true,
        });

        assert_eq!(result, Ter::TEC_NO_TARGET);
    }

    #[test]
    fn credential_create_preclaim_rejects_duplicate_when_subject_exists() {
        let result = run_credential_create_preclaim(CredentialCreatePreclaimFacts {
            subject_exists: true,
            credential_exists: true,
        });

        assert_eq!(result, Ter::TEC_DUPLICATE);
    }

    #[test]
    fn credential_create_preclaim_succeeds_when_subject_exists_and_no_duplicate() {
        let result = run_credential_create_preclaim(CredentialCreatePreclaimFacts {
            subject_exists: true,
            credential_exists: false,
        });

        assert_eq!(result, Ter::TES_SUCCESS);
    }
}
