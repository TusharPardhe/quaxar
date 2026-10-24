//! Immutable `ReadView` preclaim helpers for the domain families (the XChain
//! bridge family was removed with rippled #8255).
//!
//! This module owns only Oracle, DID, PermissionedDomain, and
//! Credential transaction types. It mirrors the corresponding rippled
//! `preclaim(...)` methods with immutable reads and deliberately returns
//! `None` for every transaction type it does not own. It never invokes apply
//! code or opens a sandbox.

use std::sync::Arc;

use basics::base_uint::Uint160;
use ledger::ReadView;
use protocol::{AccountID, STLedgerEntry, STTx, Ter, TxType, get_field_by_symbol};

use crate::{
    CredentialAcceptPreclaimFacts, CredentialCreatePreclaimFacts, CredentialDeletePreclaimFacts,
    OracleDeletePreclaimFacts, OracleSetPreclaimFacts, OracleSetPreclaimFrontFacts,
    OracleSetReserveSink, OracleSetSeriesEntry, OracleTokenPair,
    PermissionedDomainDeletePreclaimFacts, PermissionedDomainSetPreclaimFacts,
    run_credential_accept_preclaim, run_credential_create_preclaim, run_credential_delete_preclaim,
    run_oracle_delete_preclaim, run_oracle_set_preclaim, run_oracle_set_preclaim_front,
    run_permissioned_domain_delete_preclaim, run_permissioned_domain_set_preclaim,
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






fn account_has_reserve<V: ReadView>(view: &V, account: &STLedgerEntry, adjustment: i8) -> bool {
    account.get_field_amount(sf("sfBalance")).xrp().drops()
        >= ledger::effective_account_reserve(view.fees(), account, i32::from(adjustment), 0) as i64
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

fn oracle_series(series: &protocol::STArray) -> Vec<OracleSetSeriesEntry> {
    series
        .iter()
        .map(|entry| OracleSetSeriesEntry {
            pair: OracleTokenPair {
                base_asset: entry.get_field_currency(sf("sfBaseAsset")).currency(),
                quote_asset: entry.get_field_currency(sf("sfQuoteAsset")).currency(),
            },
            asset_price: entry
                .is_field_present(sf("sfAssetPrice"))
                .then(|| entry.get_field_u64(sf("sfAssetPrice"))),
            scale: entry
                .is_field_present(sf("sfScale"))
                .then(|| u16::from(entry.get_field_u8(sf("sfScale")))),
        })
        .collect()
}








fn preclaim_oracle_set<V: ReadView>(view: &V, tx: &STTx) -> Result<Ter, Ter> {
    let account = tx.get_account_id(sf("sfAccount"));
    let Some(setter) = read_account(view, account)? else {
        return Ok(Ter::TER_NO_ACCOUNT);
    };
    let front = OracleSetPreclaimFrontFacts {
        account_exists: true,
        close_time_secs: u64::from(view.parent_close_time().as_seconds()),
        last_update_time_secs: u64::from(tx.get_field_u32(sf("sfLastUpdateTime"))),
    };
    let front_result = run_oracle_set_preclaim_front(front);
    if front_result != Ter::TES_SUCCESS {
        return Ok(front_result);
    }
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
            front,
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
    if read_account(view, account)?.is_none() {
        return Ok(Ter::TER_NO_ACCOUNT);
    }
    let oracle = view
        .read(protocol::oracle_keylet(
            Uint160::from_void(account.data()),
            tx.get_field_u32(sf("sfOracleDocumentID")),
        ))
        .map_err(|_| read_error())?;
    Ok(run_oracle_delete_preclaim(OracleDeletePreclaimFacts {
        account_exists: true,
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
    let mut issuers = Vec::new();
    for credential in tx.get_field_array(sf("sfAcceptedCredentials")).iter() {
        if read_account(view, credential.get_account_id(sf("sfIssuer")))?.is_none() {
            return Ok(Ter::TEC_NO_ISSUER);
        }
        issuers.push(true);
    }
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
    // CredentialCreate reads the subject first, then the credential key, and
    // only then applies fixCleanup3_3_0's pseudo-account prohibition.
    let Some(subject_sle) = read_account(view, subject)? else {
        return Ok(Ter::TEC_NO_TARGET);
    };
    let credential_exists = view
        .exists(credential_keylet(tx, subject, issuer))
        .map_err(|_| read_error())?;
    Ok(run_credential_create_preclaim(
        CredentialCreatePreclaimFacts {
            subject_exists: true,
            credential_exists,
            reject_pseudo_subject: view.rules().enabled(&protocol::fix_cleanup_3_3_0()),
            subject_is_pseudo_account: {
                ["sfAMMID", "sfVaultID", "sfLoanBrokerID"]
                    .into_iter()
                    .any(|field| subject_sle.is_field_present(sf(field)))
            },
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
    let subject = if tx.is_field_present(sf("sfSubject")) {
        tx.get_account_id(sf("sfSubject"))
    } else {
        account
    };
    let issuer = if tx.is_field_present(sf("sfIssuer")) {
        tx.get_account_id(sf("sfIssuer"))
    } else {
        account
    };
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
    use std::collections::{BTreeMap, BTreeSet};
    use std::sync::Arc;

    use super::run_bridge_domain_read_view_preclaim;
    use basics::base_uint::{Uint160, Uint256};
    use ledger::{Fees, LedgerHeader, ReadView, ReadViewTx, Rules, ViewError};
    use protocol::{
        AccountID, LedgerEntryType, STAmount, STLedgerEntry, STTx, Ter, TxType,
        XRPAmount, get_field_by_symbol,
    };

    #[derive(Debug, Default)]
    struct View {
        entries: BTreeMap<Uint256, Arc<STLedgerEntry>>,
        fail_reads: BTreeSet<Uint256>,
        header: LedgerHeader,
        rules: Rules,
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
            self.rules.clone()
        }
        fn exists(&self, keylet: protocol::Keylet) -> Result<bool, ViewError> {
            if self.fail_reads.contains(&keylet.key) {
                return Err(ViewError::Conversion("fault-injected exists".into()));
            }
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
            if self.fail_reads.contains(&keylet.key) {
                return Err(ViewError::Conversion("fault-injected read".into()));
            }
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
    fn credential_create_preserves_subject_duplicate_and_pseudo_order() {
        let issuer = account(1);
        let subject = account(2);
        let credential_type = vec![0x41];
        let tx = STTx::new(TxType::CREDENTIAL_CREATE, |tx| {
            tx.set_account_id(sf("sfAccount"), issuer);
            tx.set_account_id(sf("sfSubject"), subject);
            tx.set_field_vl(sf("sfCredentialType"), &credential_type);
        });
        let credential_key = protocol::credential_keylet(
            Uint160::from_void(subject.data()),
            Uint160::from_void(issuer.data()),
            &credential_type,
        )
        .key;
        let mut missing = View::default();
        missing.fail_reads.insert(credential_key);
        assert_eq!(
            run_bridge_domain_read_view_preclaim(&missing, &tx, TxType::CREDENTIAL_CREATE),
            Some(Ter::TEC_NO_TARGET),
            "missing subject must short-circuit before credential existence"
        );

        let mut pseudo = View {
            rules: Rules::new([protocol::fix_cleanup_3_3_0()]),
            ..View::default()
        };
        let mut subject_root = account_entry(subject);
        subject_root.set_field_h256(sf("sfAMMID"), Uint256::from_u64(7));
        pseudo.insert(subject_root);
        assert_eq!(
            run_bridge_domain_read_view_preclaim(&pseudo, &tx, TxType::CREDENTIAL_CREATE),
            Some(Ter::TEC_PSEUDO_ACCOUNT)
        );

        let mut credential =
            STLedgerEntry::from_type_and_key(LedgerEntryType::Credential, credential_key);
        credential.set_account_id(sf("sfSubject"), subject);
        credential.set_account_id(sf("sfIssuer"), issuer);
        credential.set_field_vl(sf("sfCredentialType"), &credential_type);
        pseudo.insert(credential);
        assert_eq!(
            run_bridge_domain_read_view_preclaim(&pseudo, &tx, TxType::CREDENTIAL_CREATE),
            Some(Ter::TEC_DUPLICATE),
            "duplicate precedes the pseudo-account prohibition"
        );
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
        let mut oracle_set_front = View::default();
        oracle_set_front.insert(account_entry(account(4)));
        oracle_set_front
            .fail_reads
            .insert(protocol::oracle_keylet(Uint160::from_void(account(4).data()), 1).key);
        assert_eq!(
            run_bridge_domain_read_view_preclaim(&oracle_set_front, &oracle, TxType::ORACLE_SET,),
            Some(Ter::TEC_INVALID_UPDATE_TIME),
            "invalid update time precedes the oracle-object read"
        );
        let oracle_delete = STTx::new(TxType::ORACLE_DELETE, |tx| {
            tx.set_account_id(sf("sfAccount"), account(4));
            tx.set_field_u32(sf("sfOracleDocumentID"), 1);
        });
        let mut oracle_fault = View::default();
        oracle_fault
            .fail_reads
            .insert(protocol::oracle_keylet(Uint160::from_void(account(4).data()), 1).key);
        assert_eq!(
            run_bridge_domain_read_view_preclaim(
                &oracle_fault,
                &oracle_delete,
                TxType::ORACLE_DELETE,
            ),
            Some(Ter::TER_NO_ACCOUNT),
            "missing owner precedes the oracle-object read"
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

        let owner = account(6);
        let first_missing = account(7);
        let later_fault = account(8);
        let mut staged = View::default();
        staged.insert(account_entry(owner));
        staged
            .fail_reads
            .insert(protocol::account_keylet(Uint160::from_void(later_fault.data())).key);
        let ordered = STTx::new(TxType::PERMISSIONED_DOMAIN_SET, |tx| {
            tx.set_account_id(sf("sfAccount"), owner);
            let mut credentials = protocol::STArray::new(sf("sfAcceptedCredentials"));
            for issuer in [first_missing, later_fault] {
                let mut credential = protocol::STObject::make_inner_object(sf("sfCredential"));
                credential.set_account_id(sf("sfIssuer"), issuer);
                credential.set_field_vl(sf("sfCredentialType"), b"kyc");
                credentials.push_back(credential);
            }
            tx.set_field_array(sf("sfAcceptedCredentials"), credentials);
        });
        assert_eq!(
            run_bridge_domain_read_view_preclaim(
                &staged,
                &ordered,
                TxType::PERMISSIONED_DOMAIN_SET,
            ),
            Some(Ter::TEC_NO_ISSUER),
            "the first missing issuer short-circuits later issuer storage faults"
        );
    }
}
