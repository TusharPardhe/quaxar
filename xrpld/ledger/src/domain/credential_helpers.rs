//! the reference implementation parity — credential validation, expiration, and
//! deposit preauth checks.

use crate::views::apply_view::ApplyView;
use crate::views::read_view::{ReadView, ViewError};
use crate::{adjust_owner_count, dir_remove};
use basics::base_uint::{Uint160, Uint256};
use protocol::{
    AccountID, STArray, STLedgerEntry, STTx, STVector256, Ter, account_keylet, credential_keylet,
    credential_keylet_from_key, deposit_preauth_credentials_keylet, get_field_by_symbol,
    lsfAccepted, lsfDepositAuth, owner_dir_keylet, permissioned_domain_keylet_from_id,
    sha512_half_slices,
};
use std::collections::HashSet;
use std::sync::Arc;

fn sf(name: &str) -> &'static protocol::SField {
    get_field_by_symbol(name)
}

fn to_uint160(account: AccountID) -> Uint160 {
    Uint160::from_slice(account.data()).expect("account width should match")
}

/// Maximum number of credentials allowed in a single array.
pub const MAX_CREDENTIALS_ARRAY_SIZE: usize = 8;

/// Maximum credential type length in bytes.
pub const MAX_CREDENTIAL_TYPE_LENGTH: usize = 64;

/// Checks if a credential has expired relative to the given close time.
///
pub fn check_expired(sle_credential: &STLedgerEntry, close_time: u32) -> bool {
    let exp = if sle_credential.is_field_present(sf("sfExpiration")) {
        sle_credential.get_field_u32(sf("sfExpiration"))
    } else {
        u32::MAX
    };
    close_time > exp
}

/// Deletes a credential SLE from the ledger, removing it from both issuer
/// and subject owner directories.
///
pub fn delete_sle(
    view: &mut dyn ApplyView,
    sle_credential: Arc<STLedgerEntry>,
) -> Result<Ter, ViewError> {
    let issuer = sle_credential.get_account_id(sf("sfIssuer"));
    let subject = sle_credential.get_account_id(sf("sfSubject"));
    let accepted = sle_credential.is_flag(lsfAccepted);

    // Remove from issuer's directory
    let issuer_page = sle_credential.get_field_u64(sf("sfIssuerNode"));
    if !dir_remove(
        view,
        &owner_dir_keylet(to_uint160(issuer)),
        issuer_page,
        *sle_credential.key(),
        false,
    )? {
        return Ok(Ter::TEF_BAD_LEDGER);
    }

    // Adjust owner count for issuer if they are the owner
    let issuer_is_owner = !accepted || (subject == issuer);
    if issuer_is_owner {
        let Some(issuer_sle) = view.peek(account_keylet(to_uint160(issuer)))? else {
            return Ok(Ter::TEF_BAD_LEDGER);
        };
        adjust_owner_count(view, &issuer_sle, -1)?;
    }

    // Remove from subject's directory (if different from issuer)
    if subject != issuer {
        let subject_page = sle_credential.get_field_u64(sf("sfSubjectNode"));
        if !dir_remove(
            view,
            &owner_dir_keylet(to_uint160(subject)),
            subject_page,
            *sle_credential.key(),
            false,
        )? {
            return Ok(Ter::TEF_BAD_LEDGER);
        }

        // Always decrement subject OwnerCount when subject != issuer
        // (OwnerCount was incremented on create when added to subject's directory)
        let Some(subject_sle) = view.peek(account_keylet(to_uint160(subject)))? else {
            return Ok(Ter::TEF_BAD_LEDGER);
        };
        adjust_owner_count(view, &subject_sle, -1)?;
    }

    view.erase(sle_credential)?;
    Ok(Ter::TES_SUCCESS)
}

/// Validates the `sfCredentialIDs` field on a transaction (preflight check).
///
pub fn check_fields(tx: &STTx) -> Ter {
    if !tx.is_field_present(sf("sfCredentialIDs")) {
        return Ter::TES_SUCCESS;
    }

    let credentials = tx.get_field_v256(sf("sfCredentialIDs"));
    if credentials.value().is_empty() || credentials.value().len() > MAX_CREDENTIALS_ARRAY_SIZE {
        return Ter::TEM_MALFORMED;
    }

    let mut seen = HashSet::new();
    for cred in credentials.value() {
        if !seen.insert(*cred) {
            return Ter::TEM_MALFORMED;
        }
    }

    Ter::TES_SUCCESS
}

/// Validates credentials in preclaim: checks existence, ownership, and
/// acceptance status.
///
pub fn valid(view: &dyn ReadView, tx: &STTx, src: &AccountID) -> Result<Ter, ViewError> {
    if !tx.is_field_present(sf("sfCredentialIDs")) {
        return Ok(Ter::TES_SUCCESS);
    }

    let cred_ids = tx.get_field_v256(sf("sfCredentialIDs"));
    for h in cred_ids.value() {
        let Some(sle_cred) = view.read(credential_keylet_from_key(*h))? else {
            return Ok(Ter::TEC_BAD_CREDENTIALS);
        };

        if &sle_cred.get_account_id(sf("sfSubject")) != src {
            return Ok(Ter::TEC_BAD_CREDENTIALS);
        }

        if !sle_cred.is_flag(lsfAccepted) {
            return Ok(Ter::TEC_BAD_CREDENTIALS);
        }
    }

    Ok(Ter::TES_SUCCESS)
}

/// Validates that an account has a valid (non-expired, accepted) credential
/// for a permissioned domain.
///
pub fn valid_domain(
    view: &dyn ReadView,
    domain_id: Uint256,
    subject: &AccountID,
) -> Result<Ter, ViewError> {
    let Some(sle_pd) = view.read(permissioned_domain_keylet_from_id(domain_id))? else {
        return Ok(Ter::TEC_OBJECT_NOT_FOUND);
    };

    let close_time = view.header().parent_close_time;
    let credentials = sle_pd.get_field_array(sf("sfAcceptedCredentials"));
    let mut found_expired = false;

    for h in credentials.iter() {
        let issuer = h.get_account_id(sf("sfIssuer"));
        let cred_type = h.get_field_vl(sf("sfCredentialType"));
        let keylet = credential_keylet(to_uint160(*subject), to_uint160(issuer), &cred_type);

        let Some(sle_credential) = view.read(keylet)? else {
            continue;
        };

        if check_expired(&sle_credential, close_time) {
            found_expired = true;
            continue;
        }

        if sle_credential.is_flag(lsfAccepted) {
            return Ok(Ter::TES_SUCCESS);
        }
    }

    Ok(if found_expired {
        Ter::TEC_EXPIRED
    } else {
        Ter::TEC_NO_AUTH
    })
}

/// Checks deposit preauth using credentials.
///
pub fn authorized_deposit_preauth(
    view: &dyn ReadView,
    cred_ids: &STVector256,
    dst: &AccountID,
) -> Result<Ter, ViewError> {
    let mut sorted_hashes: Vec<Uint256> = Vec::new();

    for h in cred_ids.value() {
        let Some(sle_cred) = view.read(credential_keylet_from_key(*h))? else {
            return Ok(Ter::TEF_INTERNAL);
        };

        let issuer = sle_cred.get_account_id(sf("sfIssuer"));
        let cred_type = sle_cred.get_field_vl(sf("sfCredentialType"));
        let hash = sha512_half_slices(&[issuer.data(), &cred_type]);
        sorted_hashes.push(hash);
    }

    sorted_hashes.sort();

    let keylet = deposit_preauth_credentials_keylet(to_uint160(*dst), &sorted_hashes);
    if !view.exists(keylet)? {
        return Ok(Ter::TEC_NO_PERMISSION);
    }

    Ok(Ter::TES_SUCCESS)
}

/// Validates a credential array (used in transaction preflight).
///
pub fn check_array(credentials: &STArray, max_size: usize) -> Ter {
    let count = credentials.iter().count();
    if count == 0 {
        return Ter::TEM_ARRAY_EMPTY;
    }
    if count > max_size {
        return Ter::TEM_ARRAY_TOO_LARGE;
    }

    let mut seen = HashSet::new();
    for credential in credentials.iter() {
        let issuer = credential.get_account_id(sf("sfIssuer"));
        if issuer == AccountID::default() {
            return Ter::TEM_INVALID_ACCOUNT_ID;
        }

        let ct = credential.get_field_vl(sf("sfCredentialType"));
        if ct.is_empty() || ct.len() > MAX_CREDENTIAL_TYPE_LENGTH {
            return Ter::TEM_MALFORMED;
        }

        let hash = sha512_half_slices(&[issuer.data(), &ct]);
        if !seen.insert(hash) {
            return Ter::TEM_MALFORMED;
        }
    }

    Ter::TES_SUCCESS
}

/// Verifies that an account has valid domain credentials in doApply,
/// removing any expired credentials encountered.
///
pub fn verify_valid_domain(
    view: &mut dyn ApplyView,
    account: &AccountID,
    domain_id: Uint256,
) -> Result<Ter, ViewError> {
    let Some(sle_pd) = view.read(permissioned_domain_keylet_from_id(domain_id))? else {
        return Ok(Ter::TEC_OBJECT_NOT_FOUND);
    };

    let accepted_creds = sle_pd.get_field_array(sf("sfAcceptedCredentials"));
    let mut credential_keys = Vec::new();

    for h in accepted_creds.iter() {
        let issuer = h.get_account_id(sf("sfIssuer"));
        let cred_type = h.get_field_vl(sf("sfCredentialType"));
        let keylet = credential_keylet(to_uint160(*account), to_uint160(issuer), &cred_type);
        if view.exists(keylet)? {
            credential_keys.push(keylet.key);
        }
    }

    // Remove expired credentials
    let close_time = view.header().parent_close_time;
    let mut found_expired = false;
    for key in &credential_keys {
        if let Some(sle_cred) = view.peek(credential_keylet_from_key(*key))?
            && check_expired(&sle_cred, close_time)
        {
            found_expired = true;
            let result = delete_sle(view, sle_cred)?;
            if !protocol::is_tes_success(result) {
                return Ok(result);
            }
        }
    }

    // Check remaining credentials for acceptance
    for key in &credential_keys {
        let Some(sle_cred) = view.read(credential_keylet_from_key(*key))? else {
            continue; // deleted as expired
        };
        if sle_cred.is_flag(lsfAccepted) {
            return Ok(Ter::TES_SUCCESS);
        }
    }

    Ok(if found_expired {
        Ter::TEC_EXPIRED
    } else {
        Ter::TEC_NO_PERMISSION
    })
}

/// Verifies deposit preauth with credential expiration handling.
///
pub fn verify_deposit_preauth(
    tx: &STTx,
    view: &mut dyn ApplyView,
    src: &AccountID,
    dst: &AccountID,
    sle_dst: Option<&STLedgerEntry>,
) -> Result<Ter, ViewError> {
    let credentials_present = tx.is_field_present(sf("sfCredentialIDs"));

    if credentials_present {
        let cred_ids = tx.get_field_v256(sf("sfCredentialIDs"));
        let close_time = view.header().parent_close_time;
        let mut found_expired = false;

        for h in cred_ids.value() {
            if let Some(sle_cred) = view.peek(credential_keylet_from_key(*h))?
                && check_expired(&sle_cred, close_time)
            {
                found_expired = true;
                let result = delete_sle(view, sle_cred)?;
                if !protocol::is_tes_success(result) {
                    return Ok(result);
                }
            }
        }

        if found_expired {
            return Ok(Ter::TEC_EXPIRED);
        }
    }

    if let Some(dst_sle) = sle_dst
        && dst_sle.is_flag(lsfDepositAuth)
        && src != dst
    {
        let deposit_kl = protocol::deposit_preauth_keylet(to_uint160(*dst), to_uint160(*src));
        if !view.exists(deposit_kl)? {
            if !credentials_present {
                return Ok(Ter::TEC_NO_PERMISSION);
            }
            let cred_ids = tx.get_field_v256(sf("sfCredentialIDs"));
            return authorized_deposit_preauth(view, &cred_ids, dst);
        }
    }

    Ok(Ter::TES_SUCCESS)
}

/// Create a sorted set of (issuer, credential_type) pairs from an STArray.
/// Returns empty set if duplicates are found.
///
pub fn make_sorted(credentials: &STArray) -> std::collections::BTreeSet<(AccountID, Vec<u8>)> {
    let mut out = std::collections::BTreeSet::new();
    for cred in credentials.iter() {
        let issuer = cred.get_account_id(sf("sfIssuer"));
        let cred_type = cred.get_field_vl(sf("sfCredentialType"));
        if !out.insert((issuer, cred_type)) {
            return std::collections::BTreeSet::new();
        }
    }
    out
}
