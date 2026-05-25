//! Narrow the reference implementation compatibility surface.
//!
//! This ports the current deterministic behavior for:
//!
//! - the `fixInvalidTxFlags`-gated `getFlagsMask(...)` return,
//! - `preflight(...)` optional subject/issuer field validation,
//! - `preclaim(...)` credential-existence checks after current participant
//!   defaulting,
//! - and the current loaded `doApply()` permission / expiration gate around
//!   delegated credential deletion.

use crate::credential_create::CREDENTIAL_MAX_TYPE_LENGTH;
use protocol::{NotTec, Ter, tfUniversalMask};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CredentialOptionalAccountField {
    Missing,
    Zero,
    Present,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CredentialDeletePreflightFacts {
    pub subject: CredentialOptionalAccountField,
    pub issuer: CredentialOptionalAccountField,
    pub credential_type_len: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CredentialDeletePreclaimFacts {
    pub credential_exists: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CredentialDeleteDoApplyFacts {
    pub actor_is_subject: bool,
    pub actor_is_issuer: bool,
}

pub trait CredentialDeleteApplySink {
    fn credential_exists(&mut self) -> bool;
    fn credential_expired(&mut self) -> bool;
    fn delete_credential(&mut self) -> Ter;
}

pub const fn get_credential_delete_flags_mask(fix_invalid_tx_flags_enabled: bool) -> u32 {
    if fix_invalid_tx_flags_enabled {
        tfUniversalMask
    } else {
        0
    }
}

pub fn run_credential_delete_preflight(facts: CredentialDeletePreflightFacts) -> NotTec {
    if facts.subject == CredentialOptionalAccountField::Missing
        && facts.issuer == CredentialOptionalAccountField::Missing
    {
        return Ter::TEM_MALFORMED;
    }

    if matches!(facts.subject, CredentialOptionalAccountField::Zero)
        || matches!(facts.issuer, CredentialOptionalAccountField::Zero)
    {
        return Ter::TEM_INVALID_ACCOUNT_ID;
    }

    if facts.credential_type_len == 0 || facts.credential_type_len > CREDENTIAL_MAX_TYPE_LENGTH {
        return Ter::TEM_MALFORMED;
    }

    Ter::TES_SUCCESS
}

pub fn run_credential_delete_preclaim(facts: CredentialDeletePreclaimFacts) -> Ter {
    if !facts.credential_exists {
        return Ter::TEC_NO_ENTRY;
    }

    Ter::TES_SUCCESS
}

pub fn run_credential_delete_do_apply<S>(facts: CredentialDeleteDoApplyFacts, sink: &mut S) -> Ter
where
    S: CredentialDeleteApplySink,
{
    if !sink.credential_exists() {
        return Ter::TEF_INTERNAL;
    }

    if !facts.actor_is_subject && !facts.actor_is_issuer && !sink.credential_expired() {
        return Ter::TEC_NO_PERMISSION;
    }

    sink.delete_credential()
}
