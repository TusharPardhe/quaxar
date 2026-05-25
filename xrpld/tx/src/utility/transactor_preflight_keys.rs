//! Current Rust helpers mirroring the shared signing-key and simulate-key
//! helpers in the reference implementation.
//!
//! This module preserves the exact deterministic behavior around:
//!
//! - rejecting a non-empty signing public key when its type is unknown, and
//! - the dry-run-only signature-material rules used before later signature
//!   validity checks.

use protocol::{NotTec, Ter};

use crate::{ApplyFlags, any_apply_flags};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TransactorPreflightSigningKeyFacts {
    pub signing_pub_key_is_empty: bool,
    pub signing_pub_key_type_known: bool,
}

impl Default for TransactorPreflightSigningKeyFacts {
    fn default() -> Self {
        Self {
            signing_pub_key_is_empty: true,
            signing_pub_key_type_known: false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TransactorPreflightSimulateSignerFacts {
    pub txn_signature_present: bool,
    pub txn_signature_is_empty: bool,
}

impl Default for TransactorPreflightSimulateSignerFacts {
    fn default() -> Self {
        Self {
            txn_signature_present: false,
            txn_signature_is_empty: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransactorPreflightSimulateKeysFacts {
    pub txn_signature_present: bool,
    pub txn_signature_is_empty: bool,
    pub signers_present: bool,
    pub signer_facts: Vec<TransactorPreflightSimulateSignerFacts>,
    pub signing_pub_key_is_empty: bool,
}

impl Default for TransactorPreflightSimulateKeysFacts {
    fn default() -> Self {
        Self {
            txn_signature_present: false,
            txn_signature_is_empty: true,
            signers_present: false,
            signer_facts: Vec::new(),
            signing_pub_key_is_empty: true,
        }
    }
}

pub const fn run_preflight_check_signing_key(facts: TransactorPreflightSigningKeyFacts) -> NotTec {
    if !facts.signing_pub_key_is_empty && !facts.signing_pub_key_type_known {
        return Ter::TEM_BAD_SIGNATURE;
    }

    Ter::TES_SUCCESS
}

pub fn run_preflight_check_simulate_keys(
    flags: ApplyFlags,
    facts: &TransactorPreflightSimulateKeysFacts,
) -> Option<NotTec> {
    if !any_apply_flags(flags & ApplyFlags::DRY_RUN) {
        return None;
    }

    if facts.txn_signature_present && !facts.txn_signature_is_empty {
        return Some(Ter::TEM_INVALID);
    }

    if !facts.signers_present {
        return Some(Ter::TES_SUCCESS);
    }

    for signer in &facts.signer_facts {
        if signer.txn_signature_present && !signer.txn_signature_is_empty {
            return Some(Ter::TEM_INVALID);
        }
    }

    if !facts.signing_pub_key_is_empty {
        return Some(Ter::TEM_INVALID);
    }

    Some(Ter::TES_SUCCESS)
}

#[cfg(test)]
mod tests {
    use protocol::{Ter, trans_token};

    use super::{
        TransactorPreflightSigningKeyFacts, TransactorPreflightSimulateKeysFacts,
        TransactorPreflightSimulateSignerFacts, run_preflight_check_signing_key,
        run_preflight_check_simulate_keys,
    };
    use crate::ApplyFlags;

    #[test]
    fn preflight_check_signing_key_rejects_unknown_nonempty_pubkey() {
        let result = run_preflight_check_signing_key(TransactorPreflightSigningKeyFacts {
            signing_pub_key_is_empty: false,
            signing_pub_key_type_known: false,
        });

        assert_eq!(result, Ter::TEM_BAD_SIGNATURE);
        assert_eq!(trans_token(result), "temBAD_SIGNATURE");
    }

    #[test]
    fn preflight_check_signing_key_accepts_empty_or_known_pubkey() {
        assert_eq!(
            run_preflight_check_signing_key(TransactorPreflightSigningKeyFacts {
                signing_pub_key_is_empty: true,
                signing_pub_key_type_known: false,
            }),
            Ter::TES_SUCCESS
        );
        assert_eq!(
            run_preflight_check_signing_key(TransactorPreflightSigningKeyFacts {
                signing_pub_key_is_empty: false,
                signing_pub_key_type_known: true,
            }),
            Ter::TES_SUCCESS
        );
    }

    #[test]
    fn preflight_check_simulate_keys_skips_non_dry_run() {
        let result = run_preflight_check_simulate_keys(
            ApplyFlags::NONE,
            &TransactorPreflightSimulateKeysFacts {
                txn_signature_present: true,
                txn_signature_is_empty: false,
                ..TransactorPreflightSimulateKeysFacts::default()
            },
        );

        assert_eq!(result, None);
    }

    #[test]
    fn preflight_check_simulate_keys_rejects_nonempty_top_level_signature() {
        let result = run_preflight_check_simulate_keys(
            ApplyFlags::DRY_RUN,
            &TransactorPreflightSimulateKeysFacts {
                txn_signature_present: true,
                txn_signature_is_empty: false,
                ..TransactorPreflightSimulateKeysFacts::default()
            },
        );

        assert_eq!(result, Some(Ter::TEM_INVALID));
        assert_eq!(trans_token(result.unwrap()), "temINVALID");
    }

    #[test]
    fn preflight_check_simulate_keys_accepts_missing_signers_and_signature() {
        let result = run_preflight_check_simulate_keys(
            ApplyFlags::DRY_RUN,
            &TransactorPreflightSimulateKeysFacts::default(),
        );

        assert_eq!(result, Some(Ter::TES_SUCCESS));
    }

    #[test]
    fn preflight_check_simulate_keys_rejects_signer_signature_material() {
        let result = run_preflight_check_simulate_keys(
            ApplyFlags::DRY_RUN,
            &TransactorPreflightSimulateKeysFacts {
                signers_present: true,
                signer_facts: vec![TransactorPreflightSimulateSignerFacts {
                    txn_signature_present: true,
                    txn_signature_is_empty: false,
                }],
                ..TransactorPreflightSimulateKeysFacts::default()
            },
        );

        assert_eq!(result, Some(Ter::TEM_INVALID));
    }

    #[test]
    fn preflight_check_simulate_keys_rejects_combined_single_and_multi_signing() {
        let result = run_preflight_check_simulate_keys(
            ApplyFlags::DRY_RUN,
            &TransactorPreflightSimulateKeysFacts {
                signers_present: true,
                signing_pub_key_is_empty: false,
                ..TransactorPreflightSimulateKeysFacts::default()
            },
        );

        assert_eq!(result, Some(Ter::TEM_INVALID));
    }

    #[test]
    fn preflight_check_simulate_keys_accepts_multisign_without_signature_material() {
        let result = run_preflight_check_simulate_keys(
            ApplyFlags::DRY_RUN,
            &TransactorPreflightSimulateKeysFacts {
                signers_present: true,
                signer_facts: vec![TransactorPreflightSimulateSignerFacts {
                    txn_signature_present: true,
                    txn_signature_is_empty: true,
                }],
                ..TransactorPreflightSimulateKeysFacts::default()
            },
        );

        assert_eq!(result, Some(Ter::TES_SUCCESS));
    }
}
