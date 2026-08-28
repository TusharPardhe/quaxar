//! Typed wrappers and builders for the five ConfidentialTransfer transactions.

use std::{ops::Deref, sync::Arc};

macro_rules! confidential_transaction {
    ($wrapper:ident, $builder:ident, $tx_type:expr) => {
        #[derive(Debug, Clone, PartialEq, Eq)]
        pub struct $wrapper {
            base: crate::TransactionBase,
        }

        impl $wrapper {
            pub const TX_TYPE: crate::TxType = $tx_type;

            pub fn new(tx: Arc<crate::STTx>) -> Result<Self, String> {
                if tx.get_txn_type() != Self::TX_TYPE {
                    return Err(
                        concat!("Invalid transaction type for ", stringify!($wrapper)).into(),
                    );
                }
                Ok(Self {
                    base: crate::TransactionBase::new(tx),
                })
            }
        }

        impl Deref for $wrapper {
            type Target = crate::TransactionBase;
            fn deref(&self) -> &Self::Target {
                &self.base
            }
        }

        #[derive(Debug, Clone, PartialEq, Eq)]
        pub struct $builder {
            base: crate::TransactionBuilderBase,
        }

        impl $builder {
            fn base(
                account: crate::AccountID,
                sequence: Option<u32>,
                fee: Option<crate::STAmount>,
            ) -> Self {
                Self {
                    base: crate::TransactionBuilderBase::new(
                        $wrapper::TX_TYPE,
                        account,
                        sequence,
                        fee,
                    ),
                }
            }

            pub fn from_tx(tx: Arc<crate::STTx>) -> Result<Self, String> {
                if tx.get_txn_type() != $wrapper::TX_TYPE {
                    return Err(
                        concat!("Invalid transaction type for ", stringify!($builder)).into(),
                    );
                }
                Ok(Self {
                    base: crate::TransactionBuilderBase::from_tx(tx),
                })
            }

            pub fn set_flags(mut self, value: u32) -> Self {
                self.base.set_flags(value);
                self
            }

            pub fn set_delegate(mut self, value: crate::AccountID) -> Self {
                self.base.set_delegate(value);
                self
            }

            pub fn set_credential_ids(mut self, value: crate::STVector256) -> Self {
                self.base
                    .object_mut()
                    .set_field_v256(crate::get_field_by_symbol("sfCredentialIDs"), value);
                self
            }

            pub fn build(
                mut self,
                public_key: &crate::PublicKey,
                secret_key: &crate::SecretKey,
            ) -> Result<$wrapper, crate::SignError> {
                self.base.sign(public_key, secret_key)?;
                Ok($wrapper::new(Arc::new(crate::STTx::from_stobject(
                    self.base.into_object(),
                )))
                .expect("builder always preserves its transaction type"))
            }
        }
    };
}

confidential_transaction!(
    ConfidentialMPTConvert,
    ConfidentialMPTConvertBuilder,
    crate::TxType::CONFIDENTIAL_MPT_CONVERT
);
confidential_transaction!(
    ConfidentialMPTMergeInbox,
    ConfidentialMPTMergeInboxBuilder,
    crate::TxType::CONFIDENTIAL_MPT_MERGE_INBOX
);
confidential_transaction!(
    ConfidentialMPTConvertBack,
    ConfidentialMPTConvertBackBuilder,
    crate::TxType::CONFIDENTIAL_MPT_CONVERT_BACK
);
confidential_transaction!(
    ConfidentialMPTSend,
    ConfidentialMPTSendBuilder,
    crate::TxType::CONFIDENTIAL_MPT_SEND
);
confidential_transaction!(
    ConfidentialMPTClawback,
    ConfidentialMPTClawbackBuilder,
    crate::TxType::CONFIDENTIAL_MPT_CLAWBACK
);

fn field(name: &str) -> &'static crate::SField {
    crate::get_field_by_symbol(name)
}

macro_rules! required_getters {
    ($wrapper:ident) => {
        impl $wrapper {
            pub fn get_mp_token_issuance_id(&self) -> crate::MPTID {
                self.base
                    .as_sttx()
                    .get_field_h192(field("sfMPTokenIssuanceID"))
            }
        }
    };
}

required_getters!(ConfidentialMPTConvert);
required_getters!(ConfidentialMPTMergeInbox);
required_getters!(ConfidentialMPTConvertBack);
required_getters!(ConfidentialMPTSend);
required_getters!(ConfidentialMPTClawback);

impl ConfidentialMPTConvert {
    pub fn get_mpt_amount(&self) -> u64 {
        self.base.as_sttx().get_field_u64(field("sfMPTAmount"))
    }
    pub fn get_holder_encryption_key(&self) -> Option<Vec<u8>> {
        let f = field("sfHolderEncryptionKey");
        self.base
            .as_sttx()
            .is_field_present(f)
            .then(|| self.base.as_sttx().get_field_vl(f))
    }
    pub fn get_holder_encrypted_amount(&self) -> Vec<u8> {
        self.base
            .as_sttx()
            .get_field_vl(field("sfHolderEncryptedAmount"))
    }
    pub fn get_issuer_encrypted_amount(&self) -> Vec<u8> {
        self.base
            .as_sttx()
            .get_field_vl(field("sfIssuerEncryptedAmount"))
    }
    pub fn get_auditor_encrypted_amount(&self) -> Option<Vec<u8>> {
        let f = field("sfAuditorEncryptedAmount");
        self.base
            .as_sttx()
            .is_field_present(f)
            .then(|| self.base.as_sttx().get_field_vl(f))
    }
    pub fn get_blinding_factor(&self) -> basics::base_uint::Uint256 {
        self.base
            .as_sttx()
            .get_field_h256(field("sfBlindingFactor"))
    }
    pub fn get_zk_proof(&self) -> Option<Vec<u8>> {
        let f = field("sfZKProof");
        self.base
            .as_sttx()
            .is_field_present(f)
            .then(|| self.base.as_sttx().get_field_vl(f))
    }
}

impl ConfidentialMPTConvertBuilder {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        account: crate::AccountID,
        issuance: crate::MPTID,
        amount: u64,
        holder: Vec<u8>,
        issuer: Vec<u8>,
        blinding: basics::base_uint::Uint256,
        sequence: Option<u32>,
        fee: Option<crate::STAmount>,
    ) -> Self {
        Self::base(account, sequence, fee)
            .set_mp_token_issuance_id(issuance)
            .set_mpt_amount(amount)
            .set_holder_encrypted_amount(holder)
            .set_issuer_encrypted_amount(issuer)
            .set_blinding_factor(blinding)
    }
    pub fn set_mp_token_issuance_id(mut self, value: crate::MPTID) -> Self {
        self.base
            .object_mut()
            .set_field_h192(field("sfMPTokenIssuanceID"), value);
        self
    }
    pub fn set_mpt_amount(mut self, value: u64) -> Self {
        self.base
            .object_mut()
            .set_field_u64(field("sfMPTAmount"), value);
        self
    }
    pub fn set_holder_encryption_key(mut self, value: Vec<u8>) -> Self {
        self.base
            .object_mut()
            .set_field_vl(field("sfHolderEncryptionKey"), &value);
        self
    }
    pub fn set_holder_encrypted_amount(mut self, value: Vec<u8>) -> Self {
        self.base
            .object_mut()
            .set_field_vl(field("sfHolderEncryptedAmount"), &value);
        self
    }
    pub fn set_issuer_encrypted_amount(mut self, value: Vec<u8>) -> Self {
        self.base
            .object_mut()
            .set_field_vl(field("sfIssuerEncryptedAmount"), &value);
        self
    }
    pub fn set_auditor_encrypted_amount(mut self, value: Vec<u8>) -> Self {
        self.base
            .object_mut()
            .set_field_vl(field("sfAuditorEncryptedAmount"), &value);
        self
    }
    pub fn set_blinding_factor(mut self, value: basics::base_uint::Uint256) -> Self {
        self.base
            .object_mut()
            .set_field_h256(field("sfBlindingFactor"), value);
        self
    }
    pub fn set_zk_proof(mut self, value: Vec<u8>) -> Self {
        self.base
            .object_mut()
            .set_field_vl(field("sfZKProof"), &value);
        self
    }
}

impl ConfidentialMPTMergeInboxBuilder {
    pub fn new(
        account: crate::AccountID,
        issuance: crate::MPTID,
        sequence: Option<u32>,
        fee: Option<crate::STAmount>,
    ) -> Self {
        Self::base(account, sequence, fee).set_mp_token_issuance_id(issuance)
    }
    pub fn set_mp_token_issuance_id(mut self, value: crate::MPTID) -> Self {
        self.base
            .object_mut()
            .set_field_h192(field("sfMPTokenIssuanceID"), value);
        self
    }
}

impl ConfidentialMPTConvertBack {
    pub fn get_mpt_amount(&self) -> u64 {
        self.base.as_sttx().get_field_u64(field("sfMPTAmount"))
    }
    pub fn get_holder_encrypted_amount(&self) -> Vec<u8> {
        self.base
            .as_sttx()
            .get_field_vl(field("sfHolderEncryptedAmount"))
    }
    pub fn get_issuer_encrypted_amount(&self) -> Vec<u8> {
        self.base
            .as_sttx()
            .get_field_vl(field("sfIssuerEncryptedAmount"))
    }
    pub fn get_auditor_encrypted_amount(&self) -> Option<Vec<u8>> {
        let f = field("sfAuditorEncryptedAmount");
        self.base
            .as_sttx()
            .is_field_present(f)
            .then(|| self.base.as_sttx().get_field_vl(f))
    }
    pub fn get_blinding_factor(&self) -> basics::base_uint::Uint256 {
        self.base
            .as_sttx()
            .get_field_h256(field("sfBlindingFactor"))
    }
    pub fn get_zk_proof(&self) -> Vec<u8> {
        self.base.as_sttx().get_field_vl(field("sfZKProof"))
    }
    pub fn get_balance_commitment(&self) -> Vec<u8> {
        self.base
            .as_sttx()
            .get_field_vl(field("sfBalanceCommitment"))
    }
}

impl ConfidentialMPTConvertBackBuilder {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        account: crate::AccountID,
        issuance: crate::MPTID,
        amount: u64,
        holder: Vec<u8>,
        issuer: Vec<u8>,
        blinding: basics::base_uint::Uint256,
        proof: Vec<u8>,
        commitment: Vec<u8>,
        sequence: Option<u32>,
        fee: Option<crate::STAmount>,
    ) -> Self {
        Self::base(account, sequence, fee)
            .set_mp_token_issuance_id(issuance)
            .set_mpt_amount(amount)
            .set_holder_encrypted_amount(holder)
            .set_issuer_encrypted_amount(issuer)
            .set_blinding_factor(blinding)
            .set_zk_proof(proof)
            .set_balance_commitment(commitment)
    }
    pub fn set_mp_token_issuance_id(mut self, value: crate::MPTID) -> Self {
        self.base
            .object_mut()
            .set_field_h192(field("sfMPTokenIssuanceID"), value);
        self
    }
    pub fn set_mpt_amount(mut self, value: u64) -> Self {
        self.base
            .object_mut()
            .set_field_u64(field("sfMPTAmount"), value);
        self
    }
    pub fn set_holder_encrypted_amount(mut self, value: Vec<u8>) -> Self {
        self.base
            .object_mut()
            .set_field_vl(field("sfHolderEncryptedAmount"), &value);
        self
    }
    pub fn set_issuer_encrypted_amount(mut self, value: Vec<u8>) -> Self {
        self.base
            .object_mut()
            .set_field_vl(field("sfIssuerEncryptedAmount"), &value);
        self
    }
    pub fn set_auditor_encrypted_amount(mut self, value: Vec<u8>) -> Self {
        self.base
            .object_mut()
            .set_field_vl(field("sfAuditorEncryptedAmount"), &value);
        self
    }
    pub fn set_blinding_factor(mut self, value: basics::base_uint::Uint256) -> Self {
        self.base
            .object_mut()
            .set_field_h256(field("sfBlindingFactor"), value);
        self
    }
    pub fn set_zk_proof(mut self, value: Vec<u8>) -> Self {
        self.base
            .object_mut()
            .set_field_vl(field("sfZKProof"), &value);
        self
    }
    pub fn set_balance_commitment(mut self, value: Vec<u8>) -> Self {
        self.base
            .object_mut()
            .set_field_vl(field("sfBalanceCommitment"), &value);
        self
    }
}

impl ConfidentialMPTSend {
    pub fn get_destination(&self) -> crate::AccountID {
        self.base.as_sttx().get_account_id(field("sfDestination"))
    }
    pub fn get_destination_tag(&self) -> Option<u32> {
        let f = field("sfDestinationTag");
        self.base
            .as_sttx()
            .is_field_present(f)
            .then(|| self.base.as_sttx().get_field_u32(f))
    }
    pub fn get_sender_encrypted_amount(&self) -> Vec<u8> {
        self.base
            .as_sttx()
            .get_field_vl(field("sfSenderEncryptedAmount"))
    }
    pub fn get_destination_encrypted_amount(&self) -> Vec<u8> {
        self.base
            .as_sttx()
            .get_field_vl(field("sfDestinationEncryptedAmount"))
    }
    pub fn get_issuer_encrypted_amount(&self) -> Vec<u8> {
        self.base
            .as_sttx()
            .get_field_vl(field("sfIssuerEncryptedAmount"))
    }
    pub fn get_zk_proof(&self) -> Vec<u8> {
        self.base.as_sttx().get_field_vl(field("sfZKProof"))
    }
    pub fn get_amount_commitment(&self) -> Vec<u8> {
        self.base
            .as_sttx()
            .get_field_vl(field("sfAmountCommitment"))
    }
    pub fn get_balance_commitment(&self) -> Vec<u8> {
        self.base
            .as_sttx()
            .get_field_vl(field("sfBalanceCommitment"))
    }
}

impl ConfidentialMPTSendBuilder {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        account: crate::AccountID,
        issuance: crate::MPTID,
        destination: crate::AccountID,
        sender: Vec<u8>,
        receiver: Vec<u8>,
        issuer: Vec<u8>,
        proof: Vec<u8>,
        amount_commitment: Vec<u8>,
        balance_commitment: Vec<u8>,
        sequence: Option<u32>,
        fee: Option<crate::STAmount>,
    ) -> Self {
        Self::base(account, sequence, fee)
            .set_mp_token_issuance_id(issuance)
            .set_destination(destination)
            .set_sender_encrypted_amount(sender)
            .set_destination_encrypted_amount(receiver)
            .set_issuer_encrypted_amount(issuer)
            .set_zk_proof(proof)
            .set_amount_commitment(amount_commitment)
            .set_balance_commitment(balance_commitment)
    }
    pub fn set_mp_token_issuance_id(mut self, value: crate::MPTID) -> Self {
        self.base
            .object_mut()
            .set_field_h192(field("sfMPTokenIssuanceID"), value);
        self
    }
    pub fn set_destination(mut self, value: crate::AccountID) -> Self {
        self.base
            .object_mut()
            .set_account_id(field("sfDestination"), value);
        self
    }
    pub fn set_destination_tag(mut self, value: u32) -> Self {
        self.base
            .object_mut()
            .set_field_u32(field("sfDestinationTag"), value);
        self
    }
    pub fn set_sender_encrypted_amount(mut self, value: Vec<u8>) -> Self {
        self.base
            .object_mut()
            .set_field_vl(field("sfSenderEncryptedAmount"), &value);
        self
    }
    pub fn set_destination_encrypted_amount(mut self, value: Vec<u8>) -> Self {
        self.base
            .object_mut()
            .set_field_vl(field("sfDestinationEncryptedAmount"), &value);
        self
    }
    pub fn set_issuer_encrypted_amount(mut self, value: Vec<u8>) -> Self {
        self.base
            .object_mut()
            .set_field_vl(field("sfIssuerEncryptedAmount"), &value);
        self
    }
    pub fn set_auditor_encrypted_amount(mut self, value: Vec<u8>) -> Self {
        self.base
            .object_mut()
            .set_field_vl(field("sfAuditorEncryptedAmount"), &value);
        self
    }
    pub fn set_zk_proof(mut self, value: Vec<u8>) -> Self {
        self.base
            .object_mut()
            .set_field_vl(field("sfZKProof"), &value);
        self
    }
    pub fn set_amount_commitment(mut self, value: Vec<u8>) -> Self {
        self.base
            .object_mut()
            .set_field_vl(field("sfAmountCommitment"), &value);
        self
    }
    pub fn set_balance_commitment(mut self, value: Vec<u8>) -> Self {
        self.base
            .object_mut()
            .set_field_vl(field("sfBalanceCommitment"), &value);
        self
    }
}

impl ConfidentialMPTClawback {
    pub fn get_holder(&self) -> crate::AccountID {
        self.base.as_sttx().get_account_id(field("sfHolder"))
    }
    pub fn get_mpt_amount(&self) -> u64 {
        self.base.as_sttx().get_field_u64(field("sfMPTAmount"))
    }
    pub fn get_zk_proof(&self) -> Vec<u8> {
        self.base.as_sttx().get_field_vl(field("sfZKProof"))
    }
}

impl ConfidentialMPTClawbackBuilder {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        account: crate::AccountID,
        issuance: crate::MPTID,
        holder: crate::AccountID,
        amount: u64,
        proof: Vec<u8>,
        sequence: Option<u32>,
        fee: Option<crate::STAmount>,
    ) -> Self {
        Self::base(account, sequence, fee)
            .set_mp_token_issuance_id(issuance)
            .set_holder(holder)
            .set_mpt_amount(amount)
            .set_zk_proof(proof)
    }
    pub fn set_mp_token_issuance_id(mut self, value: crate::MPTID) -> Self {
        self.base
            .object_mut()
            .set_field_h192(field("sfMPTokenIssuanceID"), value);
        self
    }
    pub fn set_holder(mut self, value: crate::AccountID) -> Self {
        self.base
            .object_mut()
            .set_account_id(field("sfHolder"), value);
        self
    }
    pub fn set_mpt_amount(mut self, value: u64) -> Self {
        self.base
            .object_mut()
            .set_field_u64(field("sfMPTAmount"), value);
        self
    }
    pub fn set_zk_proof(mut self, value: Vec<u8>) -> Self {
        self.base
            .object_mut()
            .set_field_vl(field("sfZKProof"), &value);
        self
    }
}
