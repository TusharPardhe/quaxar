use std::{ops::Deref, sync::Arc};

use basics::base_uint::Uint256;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Credential {
    base: crate::LedgerEntryBase,
}

impl Credential {
    pub const ENTRY_TYPE: crate::LedgerEntryType = crate::LedgerEntryType::Credential;

    pub fn new(sle: Arc<crate::STLedgerEntry>) -> Result<Self, String> {
        if sle.get_type() != Self::ENTRY_TYPE {
            return Err("Invalid ledger entry type for Credential".to_owned());
        }

        Ok(Self {
            base: crate::LedgerEntryBase::new(sle),
        })
    }

    pub fn get_subject(&self) -> crate::AccountID {
        self.base
            .as_st_ledger_entry()
            .get_account_id(crate::get_field_by_symbol("sfSubject"))
    }

    pub fn get_issuer(&self) -> crate::AccountID {
        self.base
            .as_st_ledger_entry()
            .get_account_id(crate::get_field_by_symbol("sfIssuer"))
    }

    pub fn get_credential_type(&self) -> Vec<u8> {
        self.base
            .as_st_ledger_entry()
            .get_field_vl(crate::get_field_by_symbol("sfCredentialType"))
    }

    pub fn get_expiration(&self) -> Option<u32> {
        self.has_expiration().then(|| {
            self.base
                .as_st_ledger_entry()
                .get_field_u32(crate::get_field_by_symbol("sfExpiration"))
        })
    }

    pub fn has_expiration(&self) -> bool {
        self.base
            .as_st_ledger_entry()
            .is_field_present(crate::get_field_by_symbol("sfExpiration"))
    }

    pub fn get_uri(&self) -> Option<Vec<u8>> {
        self.has_uri().then(|| {
            self.base
                .as_st_ledger_entry()
                .get_field_vl(crate::get_field_by_symbol("sfURI"))
        })
    }

    pub fn has_uri(&self) -> bool {
        self.base
            .as_st_ledger_entry()
            .is_field_present(crate::get_field_by_symbol("sfURI"))
    }

    pub fn get_issuer_node(&self) -> u64 {
        self.base
            .as_st_ledger_entry()
            .get_field_u64(crate::get_field_by_symbol("sfIssuerNode"))
    }

    pub fn get_subject_node(&self) -> Option<u64> {
        self.has_subject_node().then(|| {
            self.base
                .as_st_ledger_entry()
                .get_field_u64(crate::get_field_by_symbol("sfSubjectNode"))
        })
    }

    pub fn has_subject_node(&self) -> bool {
        self.base
            .as_st_ledger_entry()
            .is_field_present(crate::get_field_by_symbol("sfSubjectNode"))
    }

    pub fn get_previous_txn_id(&self) -> basics::base_uint::Uint256 {
        self.base
            .as_st_ledger_entry()
            .get_field_h256(crate::get_field_by_symbol("sfPreviousTxnID"))
    }

    pub fn get_previous_txn_lgr_seq(&self) -> u32 {
        self.base
            .as_st_ledger_entry()
            .get_field_u32(crate::get_field_by_symbol("sfPreviousTxnLgrSeq"))
    }
}

impl Deref for Credential {
    type Target = crate::LedgerEntryBase;

    fn deref(&self) -> &Self::Target {
        &self.base
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CredentialBuilder {
    base: crate::LedgerEntryBuilderBase,
}

impl CredentialBuilder {
    pub fn new(
        subject: crate::AccountID,
        issuer: crate::AccountID,
        credential_type: impl AsRef<[u8]>,
        issuer_node: u64,
        previous_txn_id: basics::base_uint::Uint256,
        previous_txn_lgr_seq: u32,
    ) -> Self {
        let mut builder = Self {
            base: crate::LedgerEntryBuilderBase::new(Credential::ENTRY_TYPE),
        };
        builder = builder.set_subject(subject);
        builder = builder.set_issuer(issuer);
        builder = builder.set_credential_type(credential_type);
        builder = builder.set_issuer_node(issuer_node);
        builder = builder.set_previous_txn_id(previous_txn_id);
        builder = builder.set_previous_txn_lgr_seq(previous_txn_lgr_seq);
        builder
    }

    pub fn from_sle(sle: Arc<crate::STLedgerEntry>) -> Result<Self, String> {
        if sle.get_type() != Credential::ENTRY_TYPE {
            return Err("Invalid ledger entry type for Credential".to_owned());
        }

        Ok(Self {
            base: crate::LedgerEntryBuilderBase::from_sle(sle),
        })
    }

    pub fn set_ledger_index(mut self, value: Uint256) -> Self {
        self.base.set_ledger_index(value);
        self
    }

    pub fn set_flags(mut self, value: u32) -> Self {
        self.base.set_flags(value);
        self
    }

    pub fn set_subject(mut self, value: crate::AccountID) -> Self {
        self.base
            .object_mut()
            .set_account_id(crate::get_field_by_symbol("sfSubject"), value);
        self
    }

    pub fn set_issuer(mut self, value: crate::AccountID) -> Self {
        self.base
            .object_mut()
            .set_account_id(crate::get_field_by_symbol("sfIssuer"), value);
        self
    }

    pub fn set_credential_type(mut self, value: impl AsRef<[u8]>) -> Self {
        self.base.object_mut().set_field_vl(
            crate::get_field_by_symbol("sfCredentialType"),
            value.as_ref(),
        );
        self
    }

    pub fn set_expiration(mut self, value: u32) -> Self {
        self.base
            .object_mut()
            .set_field_u32(crate::get_field_by_symbol("sfExpiration"), value);
        self
    }

    pub fn set_uri(mut self, value: impl AsRef<[u8]>) -> Self {
        self.base
            .object_mut()
            .set_field_vl(crate::get_field_by_symbol("sfURI"), value.as_ref());
        self
    }

    pub fn set_issuer_node(mut self, value: u64) -> Self {
        self.base
            .object_mut()
            .set_field_u64(crate::get_field_by_symbol("sfIssuerNode"), value);
        self
    }

    pub fn set_subject_node(mut self, value: u64) -> Self {
        self.base
            .object_mut()
            .set_field_u64(crate::get_field_by_symbol("sfSubjectNode"), value);
        self
    }

    pub fn set_previous_txn_id(mut self, value: basics::base_uint::Uint256) -> Self {
        self.base
            .object_mut()
            .set_field_h256(crate::get_field_by_symbol("sfPreviousTxnID"), value);
        self
    }

    pub fn set_previous_txn_lgr_seq(mut self, value: u32) -> Self {
        self.base
            .object_mut()
            .set_field_u32(crate::get_field_by_symbol("sfPreviousTxnLgrSeq"), value);
        self
    }

    pub fn build(self, index: Uint256) -> Credential {
        Credential::new(Arc::new(crate::STLedgerEntry::from_stobject(
            self.base.into_object(),
            index,
        )))
        .expect("builder produced the matching ledger entry wrapper")
    }
}

impl Deref for CredentialBuilder {
    type Target = crate::LedgerEntryBuilderBase;

    fn deref(&self) -> &Self::Target {
        &self.base
    }
}
