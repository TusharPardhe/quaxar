//! the reference implementation parity — domain membership checks for the
//! permissioned DEX.

use crate::views::read_view::{ReadView, ViewError};
use basics::base_uint::{Uint160, Uint256};
use protocol::{
    AccountID, credential_keylet, get_field_by_symbol, lsfAccepted, lsfHybrid,
    offer_keylet_from_key, permissioned_domain_keylet_from_id,
};

fn sf(name: &str) -> &'static protocol::SField {
    get_field_by_symbol(name)
}

fn to_uint160(account: AccountID) -> Uint160 {
    Uint160::from_slice(account.data()).expect("account width should match")
}

/// Domain type alias matching reference `Domain` = `uint256`.
pub type Domain = Uint256;

/// Checks whether an account is a member of a permissioned domain.
/// An account is in the domain if:
/// 1. It is the domain owner, OR
/// 2. It holds an accepted, non-expired credential matching one of the
///    domain's accepted credentials.
///
pub fn account_in_domain(
    view: &dyn ReadView,
    account: &AccountID,
    domain_id: &Domain,
) -> Result<bool, ViewError> {
    let Some(sle_domain) = view.read(permissioned_domain_keylet_from_id(*domain_id))? else {
        return Ok(false);
    };

    // Domain owner is always in the domain
    let owner = sle_domain.get_account_id(sf("sfOwner"));
    if &owner == account {
        return Ok(true);
    }

    let credentials = sle_domain.get_field_array(sf("sfAcceptedCredentials"));
    let close_time = view.header().parent_close_time;

    for credential in credentials.iter() {
        let issuer = credential.get_account_id(sf("sfIssuer"));
        let cred_type = credential.get_field_vl(sf("sfCredentialType"));

        let cred_kl = credential_keylet(to_uint160(*account), to_uint160(issuer), &cred_type);
        let Some(sle_cred) = view.read(cred_kl)? else {
            continue;
        };

        if !sle_cred.is_flag(lsfAccepted) {
            continue;
        }

        // Check expiration
        if !credential_expired(&sle_cred, close_time) {
            return Ok(true);
        }
    }

    Ok(false)
}

/// Check if a credential has expired relative to the given close time.
fn credential_expired(sle_credential: &protocol::STLedgerEntry, close_time: u32) -> bool {
    let exp = if sle_credential.is_field_present(sf("sfExpiration")) {
        sle_credential.get_field_u32(sf("sfExpiration"))
    } else {
        u32::MAX
    };
    close_time > exp
}

/// Checks whether an offer belongs to a permissioned domain and its owner is
/// still a valid member of that domain.
///
pub fn offer_in_domain(
    view: &dyn ReadView,
    offer_id: &Uint256,
    domain_id: &Domain,
) -> Result<bool, ViewError> {
    let Some(sle_offer) = view.read(offer_keylet_from_key(*offer_id))? else {
        return Ok(false);
    };

    if !sle_offer.is_field_present(sf("sfDomainID")) {
        return Ok(false);
    }

    if sle_offer.get_field_h256(sf("sfDomainID")) != *domain_id {
        return Ok(false);
    }

    // Validate hybrid offer structure
    if sle_offer.is_flag(lsfHybrid) && !sle_offer.is_field_present(sf("sfAdditionalBooks")) {
        return Ok(false);
    }

    let account = sle_offer.get_account_id(sf("sfAccount"));
    account_in_domain(view, &account, domain_id)
}
