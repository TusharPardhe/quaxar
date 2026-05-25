use std::ops::Deref;
use std::sync::Arc;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct XChainAddAccountCreateAttestation {
    base: crate::TransactionBase,
}

impl XChainAddAccountCreateAttestation {
    pub const TX_TYPE: crate::TxType = crate::TxType::from_u16(46);

    #[allow(clippy::too_many_arguments)]
    pub fn new(tx: Arc<crate::STTx>) -> Result<Self, String> {
        if tx.get_txn_type() != Self::TX_TYPE {
            return Err(
                "Invalid transaction type for XChainAddAccountCreateAttestation".to_owned(),
            );
        }
        Ok(Self {
            base: crate::TransactionBase::new(tx),
        })
    }

    pub fn get_x_chain_bridge(&self) -> crate::STXChainBridge {
        self.base
            .as_sttx()
            .get_field_xchain_bridge(crate::get_field_by_symbol("sfXChainBridge"))
    }

    pub fn get_attestation_signer_account(&self) -> crate::AccountID {
        self.base
            .as_sttx()
            .get_account_id(crate::get_field_by_symbol("sfAttestationSignerAccount"))
    }

    pub fn get_public_key(&self) -> Vec<u8> {
        self.base
            .as_sttx()
            .get_field_vl(crate::get_field_by_symbol("sfPublicKey"))
    }

    pub fn get_signature(&self) -> Vec<u8> {
        self.base
            .as_sttx()
            .get_field_vl(crate::get_field_by_symbol("sfSignature"))
    }

    pub fn get_other_chain_source(&self) -> crate::AccountID {
        self.base
            .as_sttx()
            .get_account_id(crate::get_field_by_symbol("sfOtherChainSource"))
    }

    pub fn get_amount(&self) -> crate::STAmount {
        self.base
            .as_sttx()
            .get_field_amount(crate::get_field_by_symbol("sfAmount"))
    }

    pub fn get_attestation_reward_account(&self) -> crate::AccountID {
        self.base
            .as_sttx()
            .get_account_id(crate::get_field_by_symbol("sfAttestationRewardAccount"))
    }

    pub fn get_was_locking_chain_send(&self) -> u8 {
        self.base
            .as_sttx()
            .get_field_u8(crate::get_field_by_symbol("sfWasLockingChainSend"))
    }

    pub fn get_x_chain_account_create_count(&self) -> u64 {
        self.base
            .as_sttx()
            .get_field_u64(crate::get_field_by_symbol("sfXChainAccountCreateCount"))
    }

    pub fn get_destination(&self) -> crate::AccountID {
        self.base
            .as_sttx()
            .get_account_id(crate::get_field_by_symbol("sfDestination"))
    }

    pub fn get_signature_reward(&self) -> crate::STAmount {
        self.base
            .as_sttx()
            .get_field_amount(crate::get_field_by_symbol("sfSignatureReward"))
    }
}

impl Deref for XChainAddAccountCreateAttestation {
    type Target = crate::TransactionBase;

    fn deref(&self) -> &Self::Target {
        &self.base
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct XChainAddAccountCreateAttestationBuilder {
    base: crate::TransactionBuilderBase,
}

impl XChainAddAccountCreateAttestationBuilder {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        account: crate::AccountID,
        x_chain_bridge: crate::STXChainBridge,
        attestation_signer_account: crate::AccountID,
        public_key: impl AsRef<[u8]>,
        signature: impl AsRef<[u8]>,
        other_chain_source: crate::AccountID,
        amount: crate::STAmount,
        attestation_reward_account: crate::AccountID,
        was_locking_chain_send: u8,
        x_chain_account_create_count: u64,
        destination: crate::AccountID,
        signature_reward: crate::STAmount,
        sequence: Option<u32>,
        fee: Option<crate::STAmount>,
    ) -> Self {
        let mut builder = Self {
            base: crate::TransactionBuilderBase::new(
                XChainAddAccountCreateAttestation::TX_TYPE,
                account,
                sequence,
                fee,
            ),
        };
        builder = builder.set_x_chain_bridge(x_chain_bridge);
        builder = builder.set_attestation_signer_account(attestation_signer_account);
        builder = builder.set_public_key(public_key);
        builder = builder.set_signature(signature);
        builder = builder.set_other_chain_source(other_chain_source);
        builder = builder.set_amount(amount);
        builder = builder.set_attestation_reward_account(attestation_reward_account);
        builder = builder.set_was_locking_chain_send(was_locking_chain_send);
        builder = builder.set_x_chain_account_create_count(x_chain_account_create_count);
        builder = builder.set_destination(destination);
        builder = builder.set_signature_reward(signature_reward);
        builder
    }

    pub fn from_tx(tx: Arc<crate::STTx>) -> Result<Self, String> {
        if tx.get_txn_type() != XChainAddAccountCreateAttestation::TX_TYPE {
            return Err(
                "Invalid transaction type for XChainAddAccountCreateAttestationBuilder".to_owned(),
            );
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

    pub fn set_x_chain_bridge(mut self, value: crate::STXChainBridge) -> Self {
        self.base
            .object_mut()
            .set_field_xchain_bridge(crate::get_field_by_symbol("sfXChainBridge"), value);
        self
    }

    pub fn set_attestation_signer_account(mut self, value: crate::AccountID) -> Self {
        self.base.object_mut().set_account_id(
            crate::get_field_by_symbol("sfAttestationSignerAccount"),
            value,
        );
        self
    }

    pub fn set_public_key(mut self, value: impl AsRef<[u8]>) -> Self {
        self.base
            .object_mut()
            .set_field_vl(crate::get_field_by_symbol("sfPublicKey"), value.as_ref());
        self
    }

    pub fn set_signature(mut self, value: impl AsRef<[u8]>) -> Self {
        self.base
            .object_mut()
            .set_field_vl(crate::get_field_by_symbol("sfSignature"), value.as_ref());
        self
    }

    pub fn set_other_chain_source(mut self, value: crate::AccountID) -> Self {
        self.base
            .object_mut()
            .set_account_id(crate::get_field_by_symbol("sfOtherChainSource"), value);
        self
    }

    pub fn set_amount(mut self, value: crate::STAmount) -> Self {
        self.base
            .object_mut()
            .set_field_amount(crate::get_field_by_symbol("sfAmount"), value);
        self
    }

    pub fn set_attestation_reward_account(mut self, value: crate::AccountID) -> Self {
        self.base.object_mut().set_account_id(
            crate::get_field_by_symbol("sfAttestationRewardAccount"),
            value,
        );
        self
    }

    pub fn set_was_locking_chain_send(mut self, value: u8) -> Self {
        self.base
            .object_mut()
            .set_field_u8(crate::get_field_by_symbol("sfWasLockingChainSend"), value);
        self
    }

    pub fn set_x_chain_account_create_count(mut self, value: u64) -> Self {
        self.base.object_mut().set_field_u64(
            crate::get_field_by_symbol("sfXChainAccountCreateCount"),
            value,
        );
        self
    }

    pub fn set_destination(mut self, value: crate::AccountID) -> Self {
        self.base
            .object_mut()
            .set_account_id(crate::get_field_by_symbol("sfDestination"), value);
        self
    }

    pub fn set_signature_reward(mut self, value: crate::STAmount) -> Self {
        self.base
            .object_mut()
            .set_field_amount(crate::get_field_by_symbol("sfSignatureReward"), value);
        self
    }

    pub fn build(
        mut self,
        public_key: &crate::PublicKey,
        secret_key: &crate::SecretKey,
    ) -> Result<XChainAddAccountCreateAttestation, crate::SignError> {
        self.base.sign(public_key, secret_key)?;
        Ok(
            XChainAddAccountCreateAttestation::new(Arc::new(crate::STTx::from_stobject(
                self.base.into_object(),
            )))
            .expect("builder produced the matching transaction wrapper"),
        )
    }
}
