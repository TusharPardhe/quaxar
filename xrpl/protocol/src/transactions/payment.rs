use std::ops::Deref;
use std::sync::Arc;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Payment {
    base: crate::TransactionBase,
}

impl Payment {
    pub const TX_TYPE: crate::TxType = crate::TxType::from_u16(0);

    pub fn new(tx: Arc<crate::STTx>) -> Result<Self, String> {
        if tx.get_txn_type() != Self::TX_TYPE {
            return Err("Invalid transaction type for Payment".to_owned());
        }
        Ok(Self {
            base: crate::TransactionBase::new(tx),
        })
    }

    pub fn get_destination(&self) -> crate::AccountID {
        self.base
            .as_sttx()
            .get_account_id(crate::get_field_by_symbol("sfDestination"))
    }

    pub fn get_amount(&self) -> crate::STAmount {
        self.base
            .as_sttx()
            .get_field_amount(crate::get_field_by_symbol("sfAmount"))
    }

    pub fn get_send_max(&self) -> Option<crate::STAmount> {
        self.has_send_max().then(|| {
            self.base
                .as_sttx()
                .get_field_amount(crate::get_field_by_symbol("sfSendMax"))
        })
    }

    pub fn has_send_max(&self) -> bool {
        self.base
            .as_sttx()
            .is_field_present(crate::get_field_by_symbol("sfSendMax"))
    }

    pub fn get_paths(&self) -> Option<crate::STPathSet> {
        self.has_paths().then(|| {
            self.base
                .as_sttx()
                .get_field_path_set(crate::get_field_by_symbol("sfPaths"))
        })
    }

    pub fn has_paths(&self) -> bool {
        self.base
            .as_sttx()
            .is_field_present(crate::get_field_by_symbol("sfPaths"))
    }

    pub fn get_invoice_id(&self) -> Option<basics::base_uint::Uint256> {
        self.has_invoice_id().then(|| {
            self.base
                .as_sttx()
                .get_field_h256(crate::get_field_by_symbol("sfInvoiceID"))
        })
    }

    pub fn has_invoice_id(&self) -> bool {
        self.base
            .as_sttx()
            .is_field_present(crate::get_field_by_symbol("sfInvoiceID"))
    }

    pub fn get_destination_tag(&self) -> Option<u32> {
        self.has_destination_tag().then(|| {
            self.base
                .as_sttx()
                .get_field_u32(crate::get_field_by_symbol("sfDestinationTag"))
        })
    }

    pub fn has_destination_tag(&self) -> bool {
        self.base
            .as_sttx()
            .is_field_present(crate::get_field_by_symbol("sfDestinationTag"))
    }

    pub fn get_deliver_min(&self) -> Option<crate::STAmount> {
        self.has_deliver_min().then(|| {
            self.base
                .as_sttx()
                .get_field_amount(crate::get_field_by_symbol("sfDeliverMin"))
        })
    }

    pub fn has_deliver_min(&self) -> bool {
        self.base
            .as_sttx()
            .is_field_present(crate::get_field_by_symbol("sfDeliverMin"))
    }

    pub fn get_credential_ids(&self) -> Option<crate::STVector256> {
        self.has_credential_ids().then(|| {
            self.base
                .as_sttx()
                .get_field_v256(crate::get_field_by_symbol("sfCredentialIDs"))
        })
    }

    pub fn has_credential_ids(&self) -> bool {
        self.base
            .as_sttx()
            .is_field_present(crate::get_field_by_symbol("sfCredentialIDs"))
    }

    pub fn get_domain_id(&self) -> Option<basics::base_uint::Uint256> {
        self.has_domain_id().then(|| {
            self.base
                .as_sttx()
                .get_field_h256(crate::get_field_by_symbol("sfDomainID"))
        })
    }

    pub fn has_domain_id(&self) -> bool {
        self.base
            .as_sttx()
            .is_field_present(crate::get_field_by_symbol("sfDomainID"))
    }
}

impl Deref for Payment {
    type Target = crate::TransactionBase;

    fn deref(&self) -> &Self::Target {
        &self.base
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaymentBuilder {
    base: crate::TransactionBuilderBase,
}

impl PaymentBuilder {
    pub fn new(
        account: crate::AccountID,
        destination: crate::AccountID,
        amount: crate::STAmount,
        sequence: Option<u32>,
        fee: Option<crate::STAmount>,
    ) -> Self {
        let mut builder = Self {
            base: crate::TransactionBuilderBase::new(Payment::TX_TYPE, account, sequence, fee),
        };
        builder = builder.set_destination(destination);
        builder = builder.set_amount(amount);
        builder
    }

    pub fn from_tx(tx: Arc<crate::STTx>) -> Result<Self, String> {
        if tx.get_txn_type() != Payment::TX_TYPE {
            return Err("Invalid transaction type for PaymentBuilder".to_owned());
        }
        Ok(Self {
            base: crate::TransactionBuilderBase::from_tx(tx),
        })
    }

    pub fn set_account(mut self, value: crate::AccountID) -> Self {
        self.base.set_account(value);
        self
    }

    pub fn set_fee(mut self, value: crate::STAmount) -> Self {
        self.base.set_fee(value);
        self
    }

    pub fn set_sequence(mut self, value: u32) -> Self {
        self.base.set_sequence(value);
        self
    }

    pub fn set_ticket_sequence(mut self, value: u32) -> Self {
        self.base.set_ticket_sequence(value);
        self
    }

    pub fn set_flags(mut self, value: u32) -> Self {
        self.base.set_flags(value);
        self
    }

    pub fn set_source_tag(mut self, value: u32) -> Self {
        self.base.set_source_tag(value);
        self
    }

    pub fn set_last_ledger_sequence(mut self, value: u32) -> Self {
        self.base.set_last_ledger_sequence(value);
        self
    }

    pub fn set_account_txn_id(mut self, value: basics::base_uint::Uint256) -> Self {
        self.base.set_account_txn_id(value);
        self
    }

    pub fn set_previous_txn_id(mut self, value: basics::base_uint::Uint256) -> Self {
        self.base.set_previous_txn_id(value);
        self
    }

    pub fn set_operation_limit(mut self, value: u32) -> Self {
        self.base.set_operation_limit(value);
        self
    }

    pub fn set_memos(mut self, value: crate::STArray) -> Self {
        self.base.set_memos(value);
        self
    }

    pub fn set_signers(mut self, value: crate::STArray) -> Self {
        self.base.set_signers(value);
        self
    }

    pub fn set_network_id(mut self, value: u32) -> Self {
        self.base.set_network_id(value);
        self
    }

    pub fn set_delegate(mut self, value: crate::AccountID) -> Self {
        self.base.set_delegate(value);
        self
    }

    pub fn get_st_object(&self) -> &crate::STObject {
        self.base.object()
    }

    pub fn set_destination(mut self, value: crate::AccountID) -> Self {
        self.base
            .object_mut()
            .set_account_id(crate::get_field_by_symbol("sfDestination"), value);
        self
    }

    pub fn set_amount(mut self, value: crate::STAmount) -> Self {
        self.base
            .object_mut()
            .set_field_amount(crate::get_field_by_symbol("sfAmount"), value);
        self
    }

    pub fn set_send_max(mut self, value: crate::STAmount) -> Self {
        self.base
            .object_mut()
            .set_field_amount(crate::get_field_by_symbol("sfSendMax"), value);
        self
    }

    pub fn set_paths(mut self, value: crate::STPathSet) -> Self {
        self.base
            .object_mut()
            .set_field_path_set(crate::get_field_by_symbol("sfPaths"), value);
        self
    }

    pub fn set_invoice_id(mut self, value: basics::base_uint::Uint256) -> Self {
        self.base
            .object_mut()
            .set_field_h256(crate::get_field_by_symbol("sfInvoiceID"), value);
        self
    }

    pub fn set_destination_tag(mut self, value: u32) -> Self {
        self.base
            .object_mut()
            .set_field_u32(crate::get_field_by_symbol("sfDestinationTag"), value);
        self
    }

    pub fn set_deliver_min(mut self, value: crate::STAmount) -> Self {
        self.base
            .object_mut()
            .set_field_amount(crate::get_field_by_symbol("sfDeliverMin"), value);
        self
    }

    pub fn set_credential_ids(mut self, value: crate::STVector256) -> Self {
        self.base
            .object_mut()
            .set_field_v256(crate::get_field_by_symbol("sfCredentialIDs"), value);
        self
    }

    pub fn set_domain_id(mut self, value: basics::base_uint::Uint256) -> Self {
        self.base
            .object_mut()
            .set_field_h256(crate::get_field_by_symbol("sfDomainID"), value);
        self
    }

    pub fn build(
        mut self,
        public_key: &crate::PublicKey,
        secret_key: &crate::SecretKey,
    ) -> Result<Payment, crate::SignError> {
        self.base.sign(public_key, secret_key)?;
        Ok(Payment::new(Arc::new(crate::STTx::from_stobject(
            self.base.into_object(),
        )))
        .expect("builder produced the matching transaction wrapper"))
    }
}
