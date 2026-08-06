//! Immutable `ReadView` preclaim helpers for the bridge and domain families.
//!
//! This module owns only XChain, Oracle, DID, PermissionedDomain, and
//! Credential transaction types. It mirrors the corresponding rippled
//! `preclaim(...)` methods with immutable reads and deliberately returns
//! `None` for every transaction type it does not own. It never invokes apply
//! code or opens a sandbox.

use std::sync::Arc;

use basics::base_uint::Uint160;
use ledger::ReadView;
use protocol::{
    AccountID, Asset, Issue, PublicKey, STLedgerEntry, STTx, STXChainBridge, Ter, TxType,
    XChainBridgeChainType, calc_account_id, get_field_by_symbol, lsfAllowTrustLineClawback,
    lsfDisableMaster,
};

use crate::{
    CredentialAcceptPreclaimFacts, CredentialCreatePreclaimFacts, CredentialDeletePreclaimFacts,
    OracleDeletePreclaimFacts, OracleSetPreclaimFacts, OracleSetPreclaimFrontFacts,
    OracleSetReserveSink, OracleSetSeriesEntry, OracleTokenPair,
    PermissionedDomainDeletePreclaimFacts, PermissionedDomainSetPreclaimFacts,
    XChainCreateBridgePreclaimFacts, XChainModifyBridgePreclaimFacts,
    run_credential_accept_preclaim, run_credential_create_preclaim, run_credential_delete_preclaim,
    run_oracle_delete_preclaim, run_oracle_set_preclaim, run_permissioned_domain_delete_preclaim,
    run_permissioned_domain_set_preclaim, run_xchain_create_bridge_preclaim,
    run_xchain_modify_bridge_preclaim,
};

fn sf(name: &str) -> &'static protocol::SField {
    get_field_by_symbol(name)
}

fn read_error() -> Ter {
    Ter::TEF_BAD_LEDGER
}

fn account_keylet(account: AccountID) -> protocol::Keylet {
    protocol::account_keylet(Uint160::from_void(account.data()))
}

fn read_account<V: ReadView>(
    view: &V,
    account: AccountID,
) -> Result<Option<Arc<STLedgerEntry>>, Ter> {
    view.read(account_keylet(account)).map_err(|_| read_error())
}

fn bridge_issue(bridge: &STXChainBridge, chain: XChainBridgeChainType) -> Asset {
    bridge.issue(chain)
}

fn bridge_keylet(bridge: &STXChainBridge, chain: XChainBridgeChainType) -> protocol::Keylet {
    protocol::bridge_keylet_from_door_issue(
        Uint160::from_void(bridge.door(chain).data()),
        *bridge.issue(chain).get::<Issue>(),
    )
}

fn read_bridge<V: ReadView>(
    view: &V,
    bridge: &STXChainBridge,
) -> Result<Option<Arc<STLedgerEntry>>, Ter> {
    for chain in [
        XChainBridgeChainType::Locking,
        XChainBridgeChainType::Issuing,
    ] {
        if let Some(entry) = view
            .read(bridge_keylet(bridge, chain))
            .map_err(|_| read_error())?
            && entry.get_field_xchain_bridge(sf("sfXChainBridge")) == *bridge
        {
            return Ok(Some(entry));
        }
    }
    Ok(None)
}

fn xchain_claim_id_keylet(bridge: &STXChainBridge, claim_id: u64) -> protocol::Keylet {
    protocol::xchain_owned_claim_id_keylet_from_bridge(
        Uint160::from_void(bridge.locking_chain_door().data()),
        *bridge.locking_chain_issue().get::<Issue>(),
        Uint160::from_void(bridge.issuing_chain_door().data()),
        *bridge.issuing_chain_issue().get::<Issue>(),
        claim_id,
    )
}

fn bridge_chain_for_door(
    bridge: &STXChainBridge,
    door: AccountID,
) -> Result<XChainBridgeChainType, Ter> {
    if door == bridge.locking_chain_door() {
        Ok(XChainBridgeChainType::Locking)
    } else if door == bridge.issuing_chain_door() {
        Ok(XChainBridgeChainType::Issuing)
    } else {
        Err(Ter::TEC_INTERNAL)
    }
}

fn account_has_reserve<V: ReadView>(view: &V, account: &STLedgerEntry, adjustment: i8) -> bool {
    let owner_count = i64::from(account.get_field_u32(sf("sfOwnerCount"))) + i64::from(adjustment);
    let Ok(owner_count) = usize::try_from(owner_count) else {
        return false;
    };
    account.get_field_amount(sf("sfBalance")).xrp().drops()
        >= view.fees().account_reserve(owner_count) as i64
}

struct OracleReserve<'a, V> {
    view: &'a V,
    setter: &'a STLedgerEntry,
}

impl<V: ReadView> OracleSetReserveSink for OracleReserve<'_, V> {
    fn is_reserve_sufficient(&mut self, adjust_reserve: i8) -> bool {
        account_has_reserve(self.view, self.setter, adjust_reserve)
    }
}

fn oracle_pair(entry: &protocol::STObject) -> OracleTokenPair {
    OracleTokenPair {
        base_asset: format!(
            "{:?}",
            entry.get_field_currency(sf("sfBaseAsset")).currency()
        ),
        quote_asset: format!(
            "{:?}",
            entry.get_field_currency(sf("sfQuoteAsset")).currency()
        ),
    }
}

fn oracle_series(series: &protocol::STArray) -> Vec<OracleSetSeriesEntry> {
    series
        .iter()
        .map(|entry| OracleSetSeriesEntry {
            pair: oracle_pair(entry),
            asset_price: entry
                .is_field_present(sf("sfAssetPrice"))
                .then(|| entry.get_field_u64(sf("sfAssetPrice"))),
            scale: entry
                .is_field_present(sf("sfScale"))
                .then(|| u16::from(entry.get_field_u8(sf("sfScale")))),
        })
        .collect()
}

fn preclaim_xchain_create_bridge<V: ReadView>(view: &V, tx: &STTx) -> Result<Ter, Ter> {
    let account = tx.get_account_id(sf("sfAccount"));
    let bridge = tx.get_field_xchain_bridge(sf("sfXChainBridge"));
    // rippled queries issuing first, then locking, and returns duplicate before
    // its issuer or reserve reads.
    if view
        .exists(bridge_keylet(&bridge, XChainBridgeChainType::Issuing))
        .map_err(|_| read_error())?
        || view
            .exists(bridge_keylet(&bridge, XChainBridgeChainType::Locking))
            .map_err(|_| read_error())?
    {
        return Ok(Ter::TEC_DUPLICATE);
    }

    let source_chain = STXChainBridge::src_chain(account == bridge.locking_chain_door());
    let source_issue = bridge_issue(&bridge, source_chain);
    let source_issuer = match source_issue {
        Asset::Issue(issue) if !issue.native() => read_account(view, issue.account)?,
        _ => None,
    };
    let account_entry = read_account(view, account)?;

    Ok(run_xchain_create_bridge_preclaim(
        XChainCreateBridgePreclaimFacts {
            account,
            bridge: crate::XChainBridgeSpec {
                locking_chain_door: bridge.locking_chain_door(),
                locking_chain_issue: *bridge.locking_chain_issue().get::<Issue>(),
                issuing_chain_door: bridge.issuing_chain_door(),
                issuing_chain_issue: *bridge.issuing_chain_issue().get::<Issue>(),
            },
            bridge_exists_on_locking: false,
            bridge_exists_on_issuing: false,
            source_issue_issuer_exists: source_issuer.is_some(),
            source_issue_allows_clawback: source_issuer
                .is_some_and(|issuer| issuer.is_flag(lsfAllowTrustLineClawback)),
            account_exists: account_entry.is_some(),
            reserve_sufficient: account_entry
                .as_ref()
                .is_some_and(|entry| account_has_reserve(view, entry, 1)),
        },
    ))
}

fn preclaim_xchain_modify_bridge<V: ReadView>(view: &V, tx: &STTx) -> Result<Ter, Ter> {
    let account = tx.get_account_id(sf("sfAccount"));
    let bridge = tx.get_field_xchain_bridge(sf("sfXChainBridge"));
    let chain = STXChainBridge::src_chain(account == bridge.locking_chain_door());
    Ok(run_xchain_modify_bridge_preclaim(
        XChainModifyBridgePreclaimFacts {
            bridge_exists: view
                .read(bridge_keylet(&bridge, chain))
                .map_err(|_| read_error())?
                .is_some(),
        },
    ))
}

fn preclaim_xchain_claim<V: ReadView>(view: &V, tx: &STTx) -> Result<Ter, Ter> {
    let account = tx.get_account_id(sf("sfAccount"));
    let bridge = tx.get_field_xchain_bridge(sf("sfXChainBridge"));
    let amount = tx.get_field_amount(sf("sfAmount"));
    let Some(bridge_entry) = read_bridge(view, &bridge)? else {
        return Ok(Ter::TEC_NO_ENTRY);
    };
    if read_account(view, tx.get_account_id(sf("sfDestination")))?.is_none() {
        return Ok(Ter::TEC_NO_DST);
    }
    let chain = bridge_chain_for_door(&bridge, bridge_entry.get_account_id(sf("sfAccount")))?;
    if amount.asset() != bridge_issue(&bridge, chain) {
        return Ok(Ter::TEC_XCHAIN_BAD_TRANSFER_ISSUE);
    }
    if bridge_issue(&bridge, XChainBridgeChainType::Locking).native()
        != bridge_issue(&bridge, XChainBridgeChainType::Issuing).native()
    {
        return Ok(Ter::TEC_INTERNAL);
    }
    let Some(claim) = view
        .read(xchain_claim_id_keylet(
            &bridge,
            tx.get_field_u64(sf("sfXChainClaimID")),
        ))
        .map_err(|_| read_error())?
    else {
        return Ok(Ter::TEC_XCHAIN_NO_CLAIM_ID);
    };
    Ok(if claim.get_account_id(sf("sfAccount")) == account {
        Ter::TES_SUCCESS
    } else {
        Ter::TEC_XCHAIN_BAD_CLAIM_ID
    })
}

fn preclaim_xchain_commit<V: ReadView>(view: &V, tx: &STTx) -> Result<Ter, Ter> {
    let account = tx.get_account_id(sf("sfAccount"));
    let bridge = tx.get_field_xchain_bridge(sf("sfXChainBridge"));
    let Some(bridge_entry) = read_bridge(view, &bridge)? else {
        return Ok(Ter::TEC_NO_ENTRY);
    };
    let door = bridge_entry.get_account_id(sf("sfAccount"));
    if door == account {
        return Ok(Ter::TEC_XCHAIN_SELF_COMMIT);
    }
    let chain = bridge_chain_for_door(&bridge, door)?;
    Ok(
        if tx.get_field_amount(sf("sfAmount")).asset() == bridge_issue(&bridge, chain) {
            Ter::TES_SUCCESS
        } else {
            Ter::TEC_XCHAIN_BAD_TRANSFER_ISSUE
        },
    )
}

fn preclaim_xchain_create_claim_id<V: ReadView>(view: &V, tx: &STTx) -> Result<Ter, Ter> {
    let account = tx.get_account_id(sf("sfAccount"));
    let bridge = tx.get_field_xchain_bridge(sf("sfXChainBridge"));
    let Some(bridge_entry) = read_bridge(view, &bridge)? else {
        return Ok(Ter::TEC_NO_ENTRY);
    };
    if tx.get_field_amount(sf("sfSignatureReward"))
        != bridge_entry.get_field_amount(sf("sfSignatureReward"))
    {
        return Ok(Ter::TEC_XCHAIN_REWARD_MISMATCH);
    }
    let Some(account_entry) = read_account(view, account)? else {
        return Ok(Ter::TER_NO_ACCOUNT);
    };
    Ok(if account_has_reserve(view, &account_entry, 1) {
        Ter::TES_SUCCESS
    } else {
        Ter::TEC_INSUFFICIENT_RESERVE
    })
}

fn preclaim_xchain_create_account_commit<V: ReadView>(view: &V, tx: &STTx) -> Result<Ter, Ter> {
    let account = tx.get_account_id(sf("sfAccount"));
    let bridge = tx.get_field_xchain_bridge(sf("sfXChainBridge"));
    let amount = tx.get_field_amount(sf("sfAmount"));
    let Some(bridge_entry) = read_bridge(view, &bridge)? else {
        return Ok(Ter::TEC_NO_ENTRY);
    };
    if tx.get_field_amount(sf("sfSignatureReward"))
        != bridge_entry.get_field_amount(sf("sfSignatureReward"))
    {
        return Ok(Ter::TEC_XCHAIN_REWARD_MISMATCH);
    }
    if !bridge_entry.is_field_present(sf("sfMinAccountCreateAmount")) {
        return Ok(Ter::TEC_XCHAIN_CREATE_ACCOUNT_DISABLED);
    }
    let min_create = bridge_entry.get_field_amount(sf("sfMinAccountCreateAmount"));
    if amount < min_create {
        return Ok(Ter::TEC_XCHAIN_INSUFF_CREATE_AMOUNT);
    }
    if amount.asset() != min_create.asset() {
        return Ok(Ter::TEC_XCHAIN_BAD_TRANSFER_ISSUE);
    }
    let door = bridge_entry.get_account_id(sf("sfAccount"));
    if door == account {
        return Ok(Ter::TEC_XCHAIN_SELF_COMMIT);
    }
    let source = bridge_chain_for_door(&bridge, door)?;
    let destination = STXChainBridge::other_chain(source);
    if amount.asset() != bridge_issue(&bridge, source) {
        return Ok(Ter::TEC_XCHAIN_BAD_TRANSFER_ISSUE);
    }
    Ok(if bridge_issue(&bridge, destination).native() {
        Ter::TES_SUCCESS
    } else {
        Ter::TEC_XCHAIN_CREATE_ACCOUNT_NONXRP_ISSUE
    })
}

fn preclaim_xchain_attestation<V: ReadView>(view: &V, tx: &STTx) -> Result<Ter, Ter> {
    let bridge = tx.get_field_xchain_bridge(sf("sfXChainBridge"));
    let Some(bridge_entry) = read_bridge(view, &bridge)? else {
        return Ok(Ter::TEC_NO_ENTRY);
    };
    let door = bridge_entry.get_account_id(sf("sfAccount"));
    if read_account(view, door)?.is_none() {
        return Ok(Ter::TEC_INTERNAL);
    }
    let Some(signer_list) = view
        .read(protocol::signers_keylet(Uint160::from_void(door.data())))
        .map_err(|_| read_error())?
    else {
        return Ok(Ter::TEC_XCHAIN_NO_SIGNERS_LIST);
    };
    let signer = tx.get_account_id(sf("sfAttestationSignerAccount"));
    if !signer_list
        .get_field_array(sf("sfSignerEntries"))
        .iter()
        .any(|entry| entry.get_account_id(sf("sfAccount")) == signer)
    {
        return Ok(Ter::TEC_NO_PERMISSION);
    }
    let Ok(public_key) = PublicKey::from_slice(&tx.get_field_vl(sf("sfPublicKey"))) else {
        return Ok(Ter::TEC_INTERNAL);
    };
    let account_from_key = calc_account_id(public_key.as_bytes());
    let signer_account = read_account(view, signer)?;
    Ok(match signer_account {
        Some(account_entry) if account_from_key == signer => {
            if account_entry.is_flag(lsfDisableMaster) {
                Ter::TEC_XCHAIN_BAD_PUBLIC_KEY_ACCOUNT_PAIR
            } else {
                Ter::TES_SUCCESS
            }
        }
        Some(account_entry)
            if account_entry.is_field_present(sf("sfRegularKey"))
                && account_entry.get_account_id(sf("sfRegularKey")) == account_from_key =>
        {
            Ter::TES_SUCCESS
        }
        Some(_) => Ter::TEC_XCHAIN_BAD_PUBLIC_KEY_ACCOUNT_PAIR,
        None if account_from_key == signer => Ter::TES_SUCCESS,
        None => Ter::TEC_XCHAIN_BAD_PUBLIC_KEY_ACCOUNT_PAIR,
    })
}

fn preclaim_oracle_set<V: ReadView>(view: &V, tx: &STTx) -> Result<Ter, Ter> {
    let account = tx.get_account_id(sf("sfAccount"));
    let Some(setter) = read_account(view, account)? else {
        return Ok(Ter::TER_NO_ACCOUNT);
    };
    let oracle = view
        .read(protocol::oracle_keylet(
            Uint160::from_void(account.data()),
            tx.get_field_u32(sf("sfOracleDocumentID")),
        ))
        .map_err(|_| read_error())?;
    let tx_provider = tx.is_field_present(sf("sfProvider"));
    let tx_asset_class = tx.is_field_present(sf("sfAssetClass"));
    let (
        oracle_exists,
        tx_provider_matches_existing,
        tx_asset_class_matches_existing,
        previous_last_update_time_secs,
        existing_pairs,
    ) = match oracle {
        Some(oracle) => (
            true,
            !tx_provider
                || tx.get_field_vl(sf("sfProvider")) == oracle.get_field_vl(sf("sfProvider")),
            !tx_asset_class
                || tx.get_field_vl(sf("sfAssetClass")) == oracle.get_field_vl(sf("sfAssetClass")),
            u64::from(oracle.get_field_u32(sf("sfLastUpdateTime"))),
            oracle_series(&oracle.get_field_array(sf("sfPriceDataSeries")))
                .into_iter()
                .map(|entry| entry.pair)
                .collect(),
        ),
        None => (false, true, true, 0, Vec::new()),
    };
    let mut reserve = OracleReserve {
        view,
        setter: &setter,
    };
    Ok(run_oracle_set_preclaim(
        OracleSetPreclaimFacts {
            front: OracleSetPreclaimFrontFacts {
                account_exists: true,
                close_time_secs: u64::from(view.parent_close_time().as_seconds()),
                last_update_time_secs: u64::from(tx.get_field_u32(sf("sfLastUpdateTime"))),
            },
            oracle_exists,
            tx_provider_present: tx_provider,
            tx_asset_class_present: tx_asset_class,
            tx_provider_matches_existing,
            tx_asset_class_matches_existing,
            previous_last_update_time_secs,
            tx_series: oracle_series(&tx.get_field_array(sf("sfPriceDataSeries"))),
            existing_pairs,
        },
        &mut reserve,
    ))
}

fn preclaim_oracle_delete<V: ReadView>(view: &V, tx: &STTx) -> Result<Ter, Ter> {
    let account = tx.get_account_id(sf("sfAccount"));
    let account_exists = read_account(view, account)?.is_some();
    let oracle = view
        .read(protocol::oracle_keylet(
            Uint160::from_void(account.data()),
            tx.get_field_u32(sf("sfOracleDocumentID")),
        ))
        .map_err(|_| read_error())?;
    Ok(run_oracle_delete_preclaim(OracleDeletePreclaimFacts {
        account_exists,
        oracle_exists: oracle.is_some(),
        tx_account_matches_owner: oracle
            .is_some_and(|entry| entry.get_account_id(sf("sfOwner")) == account),
    }))
}

fn preclaim_permissioned_domain_set<V: ReadView>(view: &V, tx: &STTx) -> Result<Ter, Ter> {
    let account = tx.get_account_id(sf("sfAccount"));
    // Preserve rippled's early return: a missing owner is tefINTERNAL before
    // any accepted-credential issuer or domain lookup is attempted.
    if read_account(view, account)?.is_none() {
        return Ok(Ter::TEF_INTERNAL);
    }
    let issuers = tx
        .get_field_array(sf("sfAcceptedCredentials"))
        .iter()
        .map(|credential| {
            read_account(view, credential.get_account_id(sf("sfIssuer")))
                .map(|entry| entry.is_some())
        })
        .collect::<Result<Vec<_>, _>>()?;
    let domain_id_present = tx.is_field_present(sf("sfDomainID"));
    let domain = if domain_id_present {
        Some(
            view.read(protocol::permissioned_domain_keylet_from_id(
                tx.get_field_h256(sf("sfDomainID")),
            ))
            .map_err(|_| read_error())?,
        )
    } else {
        None
    };
    let domain = domain.flatten();
    Ok(run_permissioned_domain_set_preclaim(
        PermissionedDomainSetPreclaimFacts {
            account_exists: true,
            domain_id_present,
            domain_exists: domain.is_some(),
            domain_owned_by_account: domain
                .is_some_and(|entry| entry.get_account_id(sf("sfOwner")) == account),
        },
        issuers,
    ))
}

fn preclaim_permissioned_domain_delete<V: ReadView>(view: &V, tx: &STTx) -> Result<Ter, Ter> {
    let account = tx.get_account_id(sf("sfAccount"));
    let domain = view
        .read(protocol::permissioned_domain_keylet_from_id(
            tx.get_field_h256(sf("sfDomainID")),
        ))
        .map_err(|_| read_error())?;
    Ok(run_permissioned_domain_delete_preclaim(
        PermissionedDomainDeletePreclaimFacts {
            domain_exists: domain.is_some(),
            tx_account_matches_owner: domain
                .is_some_and(|entry| entry.get_account_id(sf("sfOwner")) == account),
        },
    ))
}

fn credential_keylet(tx: &STTx, subject: AccountID, issuer: AccountID) -> protocol::Keylet {
    protocol::credential_keylet(
        Uint160::from_void(subject.data()),
        Uint160::from_void(issuer.data()),
        &tx.get_field_vl(sf("sfCredentialType")),
    )
}

fn preclaim_credential_create<V: ReadView>(view: &V, tx: &STTx) -> Result<Ter, Ter> {
    let subject = tx.get_account_id(sf("sfSubject"));
    let issuer = tx.get_account_id(sf("sfAccount"));
    Ok(run_credential_create_preclaim(
        CredentialCreatePreclaimFacts {
            subject_exists: read_account(view, subject)?.is_some(),
            credential_exists: view
                .exists(credential_keylet(tx, subject, issuer))
                .map_err(|_| read_error())?,
        },
    ))
}

fn preclaim_credential_accept<V: ReadView>(view: &V, tx: &STTx) -> Result<Ter, Ter> {
    let subject = tx.get_account_id(sf("sfAccount"));
    let issuer = tx.get_account_id(sf("sfIssuer"));
    // rippled checks the issuer before it reads the credential object.
    if read_account(view, issuer)?.is_none() {
        return Ok(Ter::TEC_NO_ISSUER);
    }
    let credential = view
        .read(credential_keylet(tx, subject, issuer))
        .map_err(|_| read_error())?;
    Ok(run_credential_accept_preclaim(
        CredentialAcceptPreclaimFacts {
            issuer_exists: true,
            credential_exists: credential.is_some(),
            credential_accepted: credential
                .is_some_and(|entry| entry.is_flag(protocol::lsfAccepted)),
        },
    ))
}

fn preclaim_credential_delete<V: ReadView>(view: &V, tx: &STTx) -> Result<Ter, Ter> {
    let account = tx.get_account_id(sf("sfAccount"));
    let subject = tx
        .is_field_present(sf("sfSubject"))
        .then(|| tx.get_account_id(sf("sfSubject")))
        .unwrap_or(account);
    let issuer = tx
        .is_field_present(sf("sfIssuer"))
        .then(|| tx.get_account_id(sf("sfIssuer")))
        .unwrap_or(account);
    Ok(run_credential_delete_preclaim(
        CredentialDeletePreclaimFacts {
            credential_exists: view
                .exists(credential_keylet(tx, subject, issuer))
                .map_err(|_| read_error())?,
        },
    ))
}

/// Runs the complete, family-local typed preclaim tail for bridge and domain
/// transaction types. `None` means the type is not owned by this module.
pub fn run_bridge_domain_read_view_preclaim<V: ReadView>(
    view: &V,
    tx: &STTx,
    txn_type: TxType,
) -> Option<Ter> {
    let result = match txn_type {
        TxType::XCHAIN_CREATE_BRIDGE => preclaim_xchain_create_bridge(view, tx),
        TxType::XCHAIN_MODIFY_BRIDGE => preclaim_xchain_modify_bridge(view, tx),
        TxType::XCHAIN_CLAIM => preclaim_xchain_claim(view, tx),
        TxType::XCHAIN_COMMIT => preclaim_xchain_commit(view, tx),
        TxType::XCHAIN_CREATE_CLAIM_ID => preclaim_xchain_create_claim_id(view, tx),
        TxType::XCHAIN_ADD_CLAIM_ATTESTATION | TxType::XCHAIN_ADD_ACCOUNT_CREATE_ATTESTATION => {
            preclaim_xchain_attestation(view, tx)
        }
        TxType::XCHAIN_ACCOUNT_CREATE_COMMIT => preclaim_xchain_create_account_commit(view, tx),
        TxType::ORACLE_SET => preclaim_oracle_set(view, tx),
        TxType::ORACLE_DELETE => preclaim_oracle_delete(view, tx),
        // DIDSet and DIDDelete inherit Transactor::preclaim unchanged in
        // rippled. This explicit arm documents that audited no-op rather than
        // allowing an unowned/default-success path.
        TxType::DID_SET | TxType::DID_DELETE => return Some(Ter::TES_SUCCESS),
        TxType::PERMISSIONED_DOMAIN_SET => preclaim_permissioned_domain_set(view, tx),
        TxType::PERMISSIONED_DOMAIN_DELETE => preclaim_permissioned_domain_delete(view, tx),
        TxType::CREDENTIAL_CREATE => preclaim_credential_create(view, tx),
        TxType::CREDENTIAL_ACCEPT => preclaim_credential_accept(view, tx),
        TxType::CREDENTIAL_DELETE => preclaim_credential_delete(view, tx),
        _ => return None,
    };
    Some(result.unwrap_or_else(|ter| ter))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::sync::Arc;

    use super::run_bridge_domain_read_view_preclaim;
    use basics::base_uint::{Uint160, Uint256};
    use ledger::{Fees, LedgerHeader, ReadView, ReadViewTx, Rules, ViewError};
    use protocol::{
        AccountID, LedgerEntryType, STAmount, STLedgerEntry, STTx, STXChainBridge, Ter, TxType,
        XRPAmount, get_field_by_symbol, xrp_issue,
    };

    #[derive(Debug, Default)]
    struct View {
        entries: BTreeMap<Uint256, Arc<STLedgerEntry>>,
        header: LedgerHeader,
    }

    impl View {
        fn insert(&mut self, entry: STLedgerEntry) {
            self.entries.insert(*entry.key(), Arc::new(entry));
        }
    }

    impl ReadView for View {
        fn open(&self) -> bool {
            false
        }
        fn header(&self) -> LedgerHeader {
            self.header.clone()
        }
        fn fees(&self) -> Fees {
            Fees::default()
        }
        fn rules(&self) -> Rules {
            Rules::default()
        }
        fn exists(&self, keylet: protocol::Keylet) -> Result<bool, ViewError> {
            Ok(self.entries.contains_key(&keylet.key))
        }
        fn succ(
            &self,
            _key: Uint256,
            _last: Option<Uint256>,
        ) -> Result<Option<Uint256>, ViewError> {
            Ok(None)
        }
        fn read(&self, keylet: protocol::Keylet) -> Result<Option<Arc<STLedgerEntry>>, ViewError> {
            Ok(self.entries.get(&keylet.key).cloned())
        }
        fn sles(&self) -> Result<Vec<Arc<STLedgerEntry>>, ViewError> {
            Ok(self.entries.values().cloned().collect())
        }
        fn tx_exists(&self, _key: Uint256) -> Result<bool, ViewError> {
            Ok(false)
        }
        fn tx_read(&self, _key: Uint256) -> Result<Option<ReadViewTx>, ViewError> {
            Ok(None)
        }
        fn txs(&self) -> Result<Vec<ReadViewTx>, ViewError> {
            Ok(Vec::new())
        }
    }

    fn sf(name: &str) -> &'static protocol::SField {
        get_field_by_symbol(name)
    }

    fn account(fill: u8) -> AccountID {
        AccountID::from_array([fill; 20])
    }

    fn account_entry(id: AccountID) -> STLedgerEntry {
        let mut entry = STLedgerEntry::from_type_and_key(
            LedgerEntryType::AccountRoot,
            protocol::account_keylet(Uint160::from_void(id.data())).key,
        );
        entry.set_account_id(sf("sfAccount"), id);
        entry.set_field_u32(sf("sfOwnerCount"), 0);
        entry.set_field_u32(sf("sfFlags"), 0);
        entry.set_field_amount(
            sf("sfBalance"),
            STAmount::from_xrp_amount(XRPAmount::from_drops(1_000_000)),
        );
        entry
    }

    fn bridge() -> STXChainBridge {
        STXChainBridge::from_parts(account(1), xrp_issue(), account(2), xrp_issue())
    }

    fn xchain_tx(txn_type: TxType) -> STTx {
        STTx::new(txn_type, |tx| {
            tx.set_account_id(sf("sfAccount"), account(3));
            tx.set_field_xchain_bridge(sf("sfXChainBridge"), bridge());
            tx.set_field_amount(
                sf("sfAmount"),
                STAmount::from_xrp_amount(XRPAmount::from_drops(1)),
            );
            tx.set_field_amount(
                sf("sfSignatureReward"),
                STAmount::from_xrp_amount(XRPAmount::from_drops(0)),
            );
        })
    }

    #[test]
    fn only_owned_types_have_results_and_did_noops_are_explicit() {
        let view = View::default();
        let payment = STTx::new(TxType::PAYMENT, |_| {});
        let did = STTx::new(TxType::DID_SET, |_| {});
        assert_eq!(
            run_bridge_domain_read_view_preclaim(&view, &payment, TxType::PAYMENT),
            None,
            "unowned transaction types must never receive a success default"
        );
        assert_eq!(
            run_bridge_domain_read_view_preclaim(&view, &did, TxType::DID_SET),
            Some(Ter::TES_SUCCESS),
            "DID inherits rippled Transactor::preclaim unchanged"
        );
        assert!(view.entries.is_empty(), "preclaim must not mutate the view");
    }

    #[test]
    fn xchain_routes_every_owned_type_through_exact_initial_read_checks() {
        let view = View::default();
        for txn_type in [
            TxType::XCHAIN_MODIFY_BRIDGE,
            TxType::XCHAIN_CLAIM,
            TxType::XCHAIN_COMMIT,
            TxType::XCHAIN_CREATE_CLAIM_ID,
            TxType::XCHAIN_ADD_CLAIM_ATTESTATION,
            TxType::XCHAIN_ADD_ACCOUNT_CREATE_ATTESTATION,
            TxType::XCHAIN_ACCOUNT_CREATE_COMMIT,
        ] {
            assert_eq!(
                run_bridge_domain_read_view_preclaim(&view, &xchain_tx(txn_type), txn_type),
                Some(Ter::TEC_NO_ENTRY),
                "{txn_type:?} must fail at rippled readBridge"
            );
        }
        assert_eq!(
            run_bridge_domain_read_view_preclaim(
                &view,
                &xchain_tx(TxType::XCHAIN_CREATE_BRIDGE),
                TxType::XCHAIN_CREATE_BRIDGE,
            ),
            Some(Ter::TER_NO_ACCOUNT)
        );
    }

    #[test]
    fn xchain_commit_preserves_self_commit_before_transfer_issue_check() {
        let bridge = bridge();
        let mut view = View::default();
        let mut entry = STLedgerEntry::from_type_and_key(
            LedgerEntryType::Bridge,
            protocol::bridge_keylet_from_door_issue(
                Uint160::from_void(bridge.locking_chain_door().data()),
                *bridge.locking_chain_issue().get::<protocol::Issue>(),
            )
            .key,
        );
        entry.set_account_id(sf("sfAccount"), bridge.locking_chain_door());
        entry.set_field_xchain_bridge(sf("sfXChainBridge"), bridge.clone());
        view.insert(entry);
        let tx = STTx::new(TxType::XCHAIN_COMMIT, |tx| {
            tx.set_account_id(sf("sfAccount"), bridge.locking_chain_door());
            tx.set_field_xchain_bridge(sf("sfXChainBridge"), bridge);
            tx.set_field_amount(
                sf("sfAmount"),
                STAmount::from_xrp_amount(XRPAmount::from_drops(1)),
            );
        });

        assert_eq!(
            run_bridge_domain_read_view_preclaim(&view, &tx, TxType::XCHAIN_COMMIT),
            Some(Ter::TEC_XCHAIN_SELF_COMMIT)
        );
        assert_eq!(view.entries.len(), 1, "preclaim remains immutable");
    }

    #[test]
    fn oracle_domain_and_credential_families_use_read_view_facts() {
        let view = View::default();
        let oracle = STTx::new(TxType::ORACLE_SET, |tx| {
            tx.set_account_id(sf("sfAccount"), account(4));
            tx.set_field_u32(sf("sfOracleDocumentID"), 1);
        });
        assert_eq!(
            run_bridge_domain_read_view_preclaim(&view, &oracle, TxType::ORACLE_SET),
            Some(Ter::TER_NO_ACCOUNT)
        );

        let domain = STTx::new(TxType::PERMISSIONED_DOMAIN_DELETE, |tx| {
            tx.set_account_id(sf("sfAccount"), account(4));
            tx.set_field_h256(sf("sfDomainID"), Uint256::from_u64(7));
        });
        assert_eq!(
            run_bridge_domain_read_view_preclaim(
                &view,
                &domain,
                TxType::PERMISSIONED_DOMAIN_DELETE
            ),
            Some(Ter::TEC_NO_ENTRY)
        );

        let credential = STTx::new(TxType::CREDENTIAL_CREATE, |tx| {
            tx.set_account_id(sf("sfAccount"), account(4));
            tx.set_account_id(sf("sfSubject"), account(5));
            tx.set_field_vl(sf("sfCredentialType"), b"kyc");
        });
        assert_eq!(
            run_bridge_domain_read_view_preclaim(&view, &credential, TxType::CREDENTIAL_CREATE),
            Some(Ter::TEC_NO_TARGET)
        );
    }

    #[test]
    fn permissioned_domain_set_checks_account_before_credential_issuers() {
        let view = View::default();
        let tx = STTx::new(TxType::PERMISSIONED_DOMAIN_SET, |tx| {
            tx.set_account_id(sf("sfAccount"), account(6));
            let mut credential = protocol::STObject::make_inner_object(sf("sfCredential"));
            credential.set_account_id(sf("sfIssuer"), account(7));
            credential.set_field_vl(sf("sfCredentialType"), b"kyc");
            let mut credentials = protocol::STArray::new(sf("sfAcceptedCredentials"));
            credentials.push_back(credential);
            tx.set_field_array(sf("sfAcceptedCredentials"), credentials);
        });

        assert_eq!(
            run_bridge_domain_read_view_preclaim(&view, &tx, TxType::PERMISSIONED_DOMAIN_SET),
            Some(Ter::TEF_INTERNAL)
        );
    }
}
