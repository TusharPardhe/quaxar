//! Next the reference implementation share-issuance creation shell.
//!
//! This ports the deterministic behavior around:
//!
//! - building the current share-issuance request with `priorBalance = nullopt`,
//!   `sequence = 1`, the supplied `pseudoId`, `mptFlags`, `scale`,
//!   metadata, and domain id,
//! - invoking the creation helper exactly once,
//! - and returning either the created issuance id or the helper error
//!   unchanged.

use protocol::Ter;

pub const VAULT_SHARE_ISSUANCE_SEQUENCE: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VaultCreateShareIssuanceInputs<PseudoId, Metadata, DomainId> {
    pub pseudo_id: PseudoId,
    pub mpt_flags: u32,
    pub scale: u8,
    pub metadata: Option<Metadata>,
    pub domain_id: Option<DomainId>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VaultCreateShareIssuanceRequest<'a, PseudoId, Metadata, DomainId> {
    pub prior_balance: Option<()>,
    pub account: &'a PseudoId,
    pub sequence: u32,
    pub flags: u32,
    pub asset_scale: u8,
    pub metadata: Option<&'a Metadata>,
    pub domain_id: Option<&'a DomainId>,
}

pub fn run_vault_create_do_apply_share_issuance<
    PseudoId,
    Metadata,
    DomainId,
    ShareId,
    CreateShare,
>(
    inputs: &VaultCreateShareIssuanceInputs<PseudoId, Metadata, DomainId>,
    create_share: CreateShare,
) -> Result<ShareId, Ter>
where
    CreateShare: FnOnce(
        VaultCreateShareIssuanceRequest<'_, PseudoId, Metadata, DomainId>,
    ) -> Result<ShareId, Ter>,
{
    create_share(VaultCreateShareIssuanceRequest {
        prior_balance: None,
        account: &inputs.pseudo_id,
        sequence: VAULT_SHARE_ISSUANCE_SEQUENCE,
        flags: inputs.mpt_flags,
        asset_scale: inputs.scale,
        metadata: inputs.metadata.as_ref(),
        domain_id: inputs.domain_id.as_ref(),
    })
}

#[cfg(test)]
mod tests {
    use std::{cell::Cell, rc::Rc};

    use protocol::{Ter, trans_token};

    use super::{
        VAULT_SHARE_ISSUANCE_SEQUENCE, VaultCreateShareIssuanceInputs,
        VaultCreateShareIssuanceRequest, run_vault_create_do_apply_share_issuance,
    };

    #[test]
    fn vault_create_do_apply_share_issuance_builds_current_cpp_request() {
        let inputs = VaultCreateShareIssuanceInputs {
            pseudo_id: "pseudo",
            mpt_flags: 0x34,
            scale: 6,
            metadata: Some("meta"),
            domain_id: Some("domain"),
        };

        let result = run_vault_create_do_apply_share_issuance(&inputs, |request| {
            assert_eq!(
                request,
                VaultCreateShareIssuanceRequest {
                    prior_balance: None,
                    account: &"pseudo",
                    sequence: VAULT_SHARE_ISSUANCE_SEQUENCE,
                    flags: 0x34,
                    asset_scale: 6,
                    metadata: Some(&"meta"),
                    domain_id: Some(&"domain"),
                }
            );
            Ok::<_, Ter>("share-id")
        });

        assert_eq!(result, Ok("share-id"));
    }

    #[test]
    fn vault_create_do_apply_share_issuance_keeps_optional_fields_absent() {
        let inputs = VaultCreateShareIssuanceInputs {
            pseudo_id: "pseudo",
            mpt_flags: 0,
            scale: 0,
            metadata: None::<&'static str>,
            domain_id: None::<&'static str>,
        };

        let result = run_vault_create_do_apply_share_issuance(&inputs, |request| {
            assert_eq!(request.prior_balance, None);
            assert_eq!(request.metadata, None);
            assert_eq!(request.domain_id, None);
            Ok::<_, Ter>("share-id")
        });

        assert_eq!(result, Ok("share-id"));
    }

    #[test]
    fn vault_create_do_apply_share_issuance_returns_create_failure_unchanged() {
        let inputs = VaultCreateShareIssuanceInputs {
            pseudo_id: "pseudo",
            mpt_flags: 9,
            scale: 6,
            metadata: Some("meta"),
            domain_id: Some("domain"),
        };

        let result = run_vault_create_do_apply_share_issuance(&inputs, |_| {
            Err::<&'static str, _>(Ter::TEC_INSUFFICIENT_RESERVE)
        });

        assert_eq!(result, Err(Ter::TEC_INSUFFICIENT_RESERVE));
        assert_eq!(trans_token(result.unwrap_err()), "tecINSUFFICIENT_RESERVE");
    }

    #[test]
    fn vault_create_do_apply_share_issuance_calls_create_once() {
        let calls = Cell::new(0_u32);
        let inputs = VaultCreateShareIssuanceInputs {
            pseudo_id: "pseudo",
            mpt_flags: 9,
            scale: 6,
            metadata: Some("meta"),
            domain_id: Some("domain"),
        };

        let result = run_vault_create_do_apply_share_issuance(&inputs, |_| {
            calls.set(calls.get() + 1);
            Ok::<_, Ter>("share-id")
        });

        assert_eq!(result, Ok("share-id"));
        assert_eq!(calls.get(), 1);
    }

    #[test]
    fn vault_create_do_apply_share_issuance_uses_given_inputs_without_reordering() {
        let seen = Rc::new(std::cell::RefCell::new(Vec::new()));
        let inputs = VaultCreateShareIssuanceInputs {
            pseudo_id: "pseudo",
            mpt_flags: 0x44,
            scale: 18,
            metadata: Some("meta"),
            domain_id: Some("domain"),
        };

        let result = run_vault_create_do_apply_share_issuance(&inputs, {
            let seen = Rc::clone(&seen);
            move |request| {
                seen.borrow_mut().push("create");
                assert_eq!(request.flags, 0x44);
                assert_eq!(request.asset_scale, 18);
                Ok::<_, Ter>("share-id")
            }
        });

        assert_eq!(result, Ok("share-id"));
        assert_eq!(seen.borrow().as_slice(), ["create"]);
    }
}
