//! Protocol-stable transaction type identifiers from
//! `xrpl/protocol/TxFormats.h`.
//!
//! This module ports the transaction-type key space together with the
//! dispatchable `transactions.macro` catalog.

use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
pub struct TxType(u16);

impl TxType {
    pub const PAYMENT: Self = Self(0);
    pub const ESCROW_CREATE: Self = Self(1);
    pub const ESCROW_FINISH: Self = Self(2);
    pub const ACCOUNT_SET: Self = Self(3);
    pub const ESCROW_CANCEL: Self = Self(4);
    pub const REGULAR_KEY_SET: Self = Self(5);
    pub const NICKNAME_SET: Self = Self(6);
    pub const OFFER_CREATE: Self = Self(7);
    pub const OFFER_CANCEL: Self = Self(8);
    pub const CONTRACT: Self = Self(9);
    pub const TICKET_CREATE: Self = Self(10);
    pub const SPINAL_TAP: Self = Self(11);
    pub const SIGNER_LIST_SET: Self = Self(12);
    pub const PAYCHAN_CREATE: Self = Self(13);
    pub const PAYCHAN_FUND: Self = Self(14);
    pub const PAYCHAN_CLAIM: Self = Self(15);
    pub const CHECK_CREATE: Self = Self(16);
    pub const CHECK_CASH: Self = Self(17);
    pub const CHECK_CANCEL: Self = Self(18);
    pub const DEPOSIT_PREAUTH: Self = Self(19);
    pub const TRUST_SET: Self = Self(20);
    pub const ACCOUNT_DELETE: Self = Self(21);
    pub const HOOK_SET: Self = Self(22);
    pub const NFTOKEN_MINT: Self = Self(25);
    pub const NFTOKEN_BURN: Self = Self(26);
    pub const NFTOKEN_CREATE_OFFER: Self = Self(27);
    pub const NFTOKEN_CANCEL_OFFER: Self = Self(28);
    pub const NFTOKEN_ACCEPT_OFFER: Self = Self(29);
    pub const CLAWBACK: Self = Self(30);
    pub const AMM_CLAWBACK: Self = Self(31);
    pub const AMM_CREATE: Self = Self(35);
    pub const AMM_DEPOSIT: Self = Self(36);
    pub const AMM_WITHDRAW: Self = Self(37);
    pub const AMM_VOTE: Self = Self(38);
    pub const AMM_BID: Self = Self(39);
    pub const AMM_DELETE: Self = Self(40);
    pub const XCHAIN_CREATE_CLAIM_ID: Self = Self(41);
    pub const XCHAIN_COMMIT: Self = Self(42);
    pub const XCHAIN_CLAIM: Self = Self(43);
    pub const XCHAIN_ACCOUNT_CREATE_COMMIT: Self = Self(44);
    pub const XCHAIN_ADD_CLAIM_ATTESTATION: Self = Self(45);
    pub const XCHAIN_ADD_ACCOUNT_CREATE_ATTESTATION: Self = Self(46);
    pub const XCHAIN_MODIFY_BRIDGE: Self = Self(47);
    pub const XCHAIN_CREATE_BRIDGE: Self = Self(48);
    pub const DID_SET: Self = Self(49);
    pub const DID_DELETE: Self = Self(50);
    pub const ORACLE_SET: Self = Self(51);
    pub const ORACLE_DELETE: Self = Self(52);
    pub const LEDGER_STATE_FIX: Self = Self(53);
    pub const MPTOKEN_ISSUANCE_CREATE: Self = Self(54);
    pub const MPTOKEN_ISSUANCE_DESTROY: Self = Self(55);
    pub const MPTOKEN_ISSUANCE_SET: Self = Self(56);
    pub const MPTOKEN_AUTHORIZE: Self = Self(57);
    pub const CREDENTIAL_CREATE: Self = Self(58);
    pub const CREDENTIAL_ACCEPT: Self = Self(59);
    pub const CREDENTIAL_DELETE: Self = Self(60);
    pub const NFTOKEN_MODIFY: Self = Self(61);
    pub const PERMISSIONED_DOMAIN_SET: Self = Self(62);
    pub const PERMISSIONED_DOMAIN_DELETE: Self = Self(63);
    pub const DELEGATE_SET: Self = Self(64);
    pub const VAULT_CREATE: Self = Self(65);
    pub const VAULT_SET: Self = Self(66);
    pub const VAULT_DELETE: Self = Self(67);
    pub const VAULT_DEPOSIT: Self = Self(68);
    pub const VAULT_WITHDRAW: Self = Self(69);
    pub const VAULT_CLAWBACK: Self = Self(70);
    pub const BATCH: Self = Self(71);
    pub const LOAN_BROKER_SET: Self = Self(74);
    pub const LOAN_BROKER_DELETE: Self = Self(75);
    pub const LOAN_BROKER_COVER_DEPOSIT: Self = Self(76);
    pub const LOAN_BROKER_COVER_WITHDRAW: Self = Self(77);
    pub const LOAN_BROKER_COVER_CLAWBACK: Self = Self(78);
    pub const LOAN_SET: Self = Self(80);
    pub const LOAN_DELETE: Self = Self(81);
    pub const LOAN_MANAGE: Self = Self(82);
    pub const LOAN_PAY: Self = Self(84);
    pub const AMENDMENT: Self = Self(100);
    pub const FEE: Self = Self(101);
    pub const UNL_MODIFY: Self = Self(102);

    pub const fn from_u16(value: u16) -> Self {
        Self(value)
    }

    pub const fn to_u16(self) -> u16 {
        self.0
    }

    pub fn tag_name(self) -> Option<&'static str> {
        TX_TYPE_TAGS
            .iter()
            .find_map(|(value, tag, _)| (*value == self.0).then_some(*tag))
    }

    pub fn format_name(self) -> Option<&'static str> {
        DISPATCHABLE_TX_TYPES
            .iter()
            .find_map(|(value, _, format)| (*value == self.0).then_some(*format))
    }

    pub fn from_tag_name(name: &str) -> Option<Self> {
        TX_TYPE_TAGS
            .iter()
            .find_map(|(value, tag, _)| (*tag == name).then_some(Self(*value)))
    }

    pub fn from_format_name(name: &str) -> Option<Self> {
        DISPATCHABLE_TX_TYPES
            .iter()
            .find_map(|(value, _, format)| (*format == name).then_some(Self(*value)))
    }

    pub fn is_known(self) -> bool {
        self.tag_name().is_some()
    }

    pub fn is_dispatchable(self) -> bool {
        self.format_name().is_some()
    }

    pub const fn is_deprecated(self) -> bool {
        matches!(self.0, 6 | 9 | 11)
    }

    pub fn is_protocol_only(self) -> bool {
        self.is_known() && !self.is_dispatchable()
    }
}

impl From<u16> for TxType {
    fn from(value: u16) -> Self {
        Self::from_u16(value)
    }
}

impl From<TxType> for u16 {
    fn from(value: TxType) -> Self {
        value.to_u16()
    }
}

impl fmt::Display for TxType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.tag_name() {
            Some(tag) => write!(f, "{tag}"),
            None => write!(f, "txType({})", self.0),
        }
    }
}

const DISPATCHABLE_TX_TYPES: &[(u16, &str, &str)] = &[
    (0, "ttPAYMENT", "Payment"),
    (1, "ttESCROW_CREATE", "EscrowCreate"),
    (2, "ttESCROW_FINISH", "EscrowFinish"),
    (3, "ttACCOUNT_SET", "AccountSet"),
    (4, "ttESCROW_CANCEL", "EscrowCancel"),
    (5, "ttREGULAR_KEY_SET", "SetRegularKey"),
    (7, "ttOFFER_CREATE", "OfferCreate"),
    (8, "ttOFFER_CANCEL", "OfferCancel"),
    (10, "ttTICKET_CREATE", "TicketCreate"),
    (12, "ttSIGNER_LIST_SET", "SignerListSet"),
    (13, "ttPAYCHAN_CREATE", "PaymentChannelCreate"),
    (14, "ttPAYCHAN_FUND", "PaymentChannelFund"),
    (15, "ttPAYCHAN_CLAIM", "PaymentChannelClaim"),
    (16, "ttCHECK_CREATE", "CheckCreate"),
    (17, "ttCHECK_CASH", "CheckCash"),
    (18, "ttCHECK_CANCEL", "CheckCancel"),
    (19, "ttDEPOSIT_PREAUTH", "DepositPreauth"),
    (20, "ttTRUST_SET", "TrustSet"),
    (21, "ttACCOUNT_DELETE", "AccountDelete"),
    (25, "ttNFTOKEN_MINT", "NFTokenMint"),
    (26, "ttNFTOKEN_BURN", "NFTokenBurn"),
    (27, "ttNFTOKEN_CREATE_OFFER", "NFTokenCreateOffer"),
    (28, "ttNFTOKEN_CANCEL_OFFER", "NFTokenCancelOffer"),
    (29, "ttNFTOKEN_ACCEPT_OFFER", "NFTokenAcceptOffer"),
    (30, "ttCLAWBACK", "Clawback"),
    (31, "ttAMM_CLAWBACK", "AMMClawback"),
    (35, "ttAMM_CREATE", "AMMCreate"),
    (36, "ttAMM_DEPOSIT", "AMMDeposit"),
    (37, "ttAMM_WITHDRAW", "AMMWithdraw"),
    (38, "ttAMM_VOTE", "AMMVote"),
    (39, "ttAMM_BID", "AMMBid"),
    (40, "ttAMM_DELETE", "AMMDelete"),
    (41, "ttXCHAIN_CREATE_CLAIM_ID", "XChainCreateClaimID"),
    (42, "ttXCHAIN_COMMIT", "XChainCommit"),
    (43, "ttXCHAIN_CLAIM", "XChainClaim"),
    (
        44,
        "ttXCHAIN_ACCOUNT_CREATE_COMMIT",
        "XChainAccountCreateCommit",
    ),
    (
        45,
        "ttXCHAIN_ADD_CLAIM_ATTESTATION",
        "XChainAddClaimAttestation",
    ),
    (
        46,
        "ttXCHAIN_ADD_ACCOUNT_CREATE_ATTESTATION",
        "XChainAddAccountCreateAttestation",
    ),
    (47, "ttXCHAIN_MODIFY_BRIDGE", "XChainModifyBridge"),
    (48, "ttXCHAIN_CREATE_BRIDGE", "XChainCreateBridge"),
    (49, "ttDID_SET", "DIDSet"),
    (50, "ttDID_DELETE", "DIDDelete"),
    (51, "ttORACLE_SET", "OracleSet"),
    (52, "ttORACLE_DELETE", "OracleDelete"),
    (53, "ttLEDGER_STATE_FIX", "LedgerStateFix"),
    (54, "ttMPTOKEN_ISSUANCE_CREATE", "MPTokenIssuanceCreate"),
    (55, "ttMPTOKEN_ISSUANCE_DESTROY", "MPTokenIssuanceDestroy"),
    (56, "ttMPTOKEN_ISSUANCE_SET", "MPTokenIssuanceSet"),
    (57, "ttMPTOKEN_AUTHORIZE", "MPTokenAuthorize"),
    (58, "ttCREDENTIAL_CREATE", "CredentialCreate"),
    (59, "ttCREDENTIAL_ACCEPT", "CredentialAccept"),
    (60, "ttCREDENTIAL_DELETE", "CredentialDelete"),
    (61, "ttNFTOKEN_MODIFY", "NFTokenModify"),
    (62, "ttPERMISSIONED_DOMAIN_SET", "PermissionedDomainSet"),
    (
        63,
        "ttPERMISSIONED_DOMAIN_DELETE",
        "PermissionedDomainDelete",
    ),
    (64, "ttDELEGATE_SET", "DelegateSet"),
    (65, "ttVAULT_CREATE", "VaultCreate"),
    (66, "ttVAULT_SET", "VaultSet"),
    (67, "ttVAULT_DELETE", "VaultDelete"),
    (68, "ttVAULT_DEPOSIT", "VaultDeposit"),
    (69, "ttVAULT_WITHDRAW", "VaultWithdraw"),
    (70, "ttVAULT_CLAWBACK", "VaultClawback"),
    (71, "ttBATCH", "Batch"),
    (74, "ttLOAN_BROKER_SET", "LoanBrokerSet"),
    (75, "ttLOAN_BROKER_DELETE", "LoanBrokerDelete"),
    (76, "ttLOAN_BROKER_COVER_DEPOSIT", "LoanBrokerCoverDeposit"),
    (
        77,
        "ttLOAN_BROKER_COVER_WITHDRAW",
        "LoanBrokerCoverWithdraw",
    ),
    (
        78,
        "ttLOAN_BROKER_COVER_CLAWBACK",
        "LoanBrokerCoverClawback",
    ),
    (80, "ttLOAN_SET", "LoanSet"),
    (81, "ttLOAN_DELETE", "LoanDelete"),
    (82, "ttLOAN_MANAGE", "LoanManage"),
    (84, "ttLOAN_PAY", "LoanPay"),
    (100, "ttAMENDMENT", "EnableAmendment"),
    (101, "ttFEE", "SetFee"),
    (102, "ttUNL_MODIFY", "UNLModify"),
];

#[cfg(test)]
const EXTRA_KNOWN_TX_TYPES: &[(u16, &str)] = &[
    (6, "ttNICKNAME_SET"),
    (9, "ttCONTRACT"),
    (11, "ttSPINAL_TAP"),
    (22, "ttHOOK_SET"),
];

const TX_TYPE_TAGS: &[(u16, &str, Option<&str>)] = &[
    (0, "ttPAYMENT", Some("Payment")),
    (1, "ttESCROW_CREATE", Some("EscrowCreate")),
    (2, "ttESCROW_FINISH", Some("EscrowFinish")),
    (3, "ttACCOUNT_SET", Some("AccountSet")),
    (4, "ttESCROW_CANCEL", Some("EscrowCancel")),
    (5, "ttREGULAR_KEY_SET", Some("SetRegularKey")),
    (6, "ttNICKNAME_SET", None),
    (7, "ttOFFER_CREATE", Some("OfferCreate")),
    (8, "ttOFFER_CANCEL", Some("OfferCancel")),
    (9, "ttCONTRACT", None),
    (10, "ttTICKET_CREATE", Some("TicketCreate")),
    (11, "ttSPINAL_TAP", None),
    (12, "ttSIGNER_LIST_SET", Some("SignerListSet")),
    (13, "ttPAYCHAN_CREATE", Some("PaymentChannelCreate")),
    (14, "ttPAYCHAN_FUND", Some("PaymentChannelFund")),
    (15, "ttPAYCHAN_CLAIM", Some("PaymentChannelClaim")),
    (16, "ttCHECK_CREATE", Some("CheckCreate")),
    (17, "ttCHECK_CASH", Some("CheckCash")),
    (18, "ttCHECK_CANCEL", Some("CheckCancel")),
    (19, "ttDEPOSIT_PREAUTH", Some("DepositPreauth")),
    (20, "ttTRUST_SET", Some("TrustSet")),
    (21, "ttACCOUNT_DELETE", Some("AccountDelete")),
    (22, "ttHOOK_SET", None),
    (25, "ttNFTOKEN_MINT", Some("NFTokenMint")),
    (26, "ttNFTOKEN_BURN", Some("NFTokenBurn")),
    (27, "ttNFTOKEN_CREATE_OFFER", Some("NFTokenCreateOffer")),
    (28, "ttNFTOKEN_CANCEL_OFFER", Some("NFTokenCancelOffer")),
    (29, "ttNFTOKEN_ACCEPT_OFFER", Some("NFTokenAcceptOffer")),
    (30, "ttCLAWBACK", Some("Clawback")),
    (31, "ttAMM_CLAWBACK", Some("AMMClawback")),
    (35, "ttAMM_CREATE", Some("AMMCreate")),
    (36, "ttAMM_DEPOSIT", Some("AMMDeposit")),
    (37, "ttAMM_WITHDRAW", Some("AMMWithdraw")),
    (38, "ttAMM_VOTE", Some("AMMVote")),
    (39, "ttAMM_BID", Some("AMMBid")),
    (40, "ttAMM_DELETE", Some("AMMDelete")),
    (41, "ttXCHAIN_CREATE_CLAIM_ID", Some("XChainCreateClaimID")),
    (42, "ttXCHAIN_COMMIT", Some("XChainCommit")),
    (43, "ttXCHAIN_CLAIM", Some("XChainClaim")),
    (
        44,
        "ttXCHAIN_ACCOUNT_CREATE_COMMIT",
        Some("XChainAccountCreateCommit"),
    ),
    (
        45,
        "ttXCHAIN_ADD_CLAIM_ATTESTATION",
        Some("XChainAddClaimAttestation"),
    ),
    (
        46,
        "ttXCHAIN_ADD_ACCOUNT_CREATE_ATTESTATION",
        Some("XChainAddAccountCreateAttestation"),
    ),
    (47, "ttXCHAIN_MODIFY_BRIDGE", Some("XChainModifyBridge")),
    (48, "ttXCHAIN_CREATE_BRIDGE", Some("XChainCreateBridge")),
    (49, "ttDID_SET", Some("DIDSet")),
    (50, "ttDID_DELETE", Some("DIDDelete")),
    (51, "ttORACLE_SET", Some("OracleSet")),
    (52, "ttORACLE_DELETE", Some("OracleDelete")),
    (53, "ttLEDGER_STATE_FIX", Some("LedgerStateFix")),
    (
        54,
        "ttMPTOKEN_ISSUANCE_CREATE",
        Some("MPTokenIssuanceCreate"),
    ),
    (
        55,
        "ttMPTOKEN_ISSUANCE_DESTROY",
        Some("MPTokenIssuanceDestroy"),
    ),
    (56, "ttMPTOKEN_ISSUANCE_SET", Some("MPTokenIssuanceSet")),
    (57, "ttMPTOKEN_AUTHORIZE", Some("MPTokenAuthorize")),
    (58, "ttCREDENTIAL_CREATE", Some("CredentialCreate")),
    (59, "ttCREDENTIAL_ACCEPT", Some("CredentialAccept")),
    (60, "ttCREDENTIAL_DELETE", Some("CredentialDelete")),
    (61, "ttNFTOKEN_MODIFY", Some("NFTokenModify")),
    (
        62,
        "ttPERMISSIONED_DOMAIN_SET",
        Some("PermissionedDomainSet"),
    ),
    (
        63,
        "ttPERMISSIONED_DOMAIN_DELETE",
        Some("PermissionedDomainDelete"),
    ),
    (64, "ttDELEGATE_SET", Some("DelegateSet")),
    (65, "ttVAULT_CREATE", Some("VaultCreate")),
    (66, "ttVAULT_SET", Some("VaultSet")),
    (67, "ttVAULT_DELETE", Some("VaultDelete")),
    (68, "ttVAULT_DEPOSIT", Some("VaultDeposit")),
    (69, "ttVAULT_WITHDRAW", Some("VaultWithdraw")),
    (70, "ttVAULT_CLAWBACK", Some("VaultClawback")),
    (71, "ttBATCH", Some("Batch")),
    (74, "ttLOAN_BROKER_SET", Some("LoanBrokerSet")),
    (75, "ttLOAN_BROKER_DELETE", Some("LoanBrokerDelete")),
    (
        76,
        "ttLOAN_BROKER_COVER_DEPOSIT",
        Some("LoanBrokerCoverDeposit"),
    ),
    (
        77,
        "ttLOAN_BROKER_COVER_WITHDRAW",
        Some("LoanBrokerCoverWithdraw"),
    ),
    (
        78,
        "ttLOAN_BROKER_COVER_CLAWBACK",
        Some("LoanBrokerCoverClawback"),
    ),
    (80, "ttLOAN_SET", Some("LoanSet")),
    (81, "ttLOAN_DELETE", Some("LoanDelete")),
    (82, "ttLOAN_MANAGE", Some("LoanManage")),
    (84, "ttLOAN_PAY", Some("LoanPay")),
    (100, "ttAMENDMENT", Some("EnableAmendment")),
    (101, "ttFEE", Some("SetFee")),
    (102, "ttUNL_MODIFY", Some("UNLModify")),
];

#[cfg(test)]
mod tests {
    use super::{DISPATCHABLE_TX_TYPES, EXTRA_KNOWN_TX_TYPES, TxType};

    #[test]
    fn tx_type_ids_match_current_cpp_protocol_values() {
        assert_eq!(TxType::PAYMENT.to_u16(), 0);
        assert_eq!(TxType::HOOK_SET.to_u16(), 22);
        assert_eq!(TxType::BATCH.to_u16(), 71);
        assert_eq!(TxType::AMENDMENT.to_u16(), 100);
        assert_eq!(TxType::UNL_MODIFY.to_u16(), 102);
    }

    #[test]
    fn tx_type_names_match_current_cpp_tags_and_format_names() {
        assert_eq!(TxType::PAYMENT.tag_name(), Some("ttPAYMENT"));
        assert_eq!(TxType::PAYMENT.format_name(), Some("Payment"));
        assert_eq!(
            TxType::XCHAIN_ADD_ACCOUNT_CREATE_ATTESTATION.format_name(),
            Some("XChainAddAccountCreateAttestation")
        );
        assert_eq!(TxType::HOOK_SET.tag_name(), Some("ttHOOK_SET"));
        assert_eq!(TxType::HOOK_SET.format_name(), None);
    }

    #[test]
    fn tx_type_lookup_helpers_round_trip_current_cpp_names() {
        assert_eq!(TxType::from_tag_name("ttPAYMENT"), Some(TxType::PAYMENT));
        assert_eq!(TxType::from_format_name("Payment"), Some(TxType::PAYMENT));
        assert_eq!(
            TxType::from_format_name("XChainAddAccountCreateAttestation"),
            Some(TxType::XCHAIN_ADD_ACCOUNT_CREATE_ATTESTATION)
        );
        assert_eq!(TxType::from_tag_name("ttHOOK_SET"), Some(TxType::HOOK_SET));
        assert_eq!(TxType::from_format_name("HookSet"), None);
        assert_eq!(TxType::from_tag_name("ttUNKNOWN"), None);
    }

    #[test]
    fn tx_type_distinguishes_dispatchable_from_protocol_only_values() {
        assert!(TxType::PAYMENT.is_known());
        assert!(TxType::PAYMENT.is_dispatchable());
        assert!(!TxType::PAYMENT.is_protocol_only());

        assert!(TxType::HOOK_SET.is_known());
        assert!(!TxType::HOOK_SET.is_dispatchable());
        assert!(TxType::HOOK_SET.is_protocol_only());

        assert!(TxType::NICKNAME_SET.is_known());
        assert!(TxType::NICKNAME_SET.is_deprecated());
        assert!(TxType::NICKNAME_SET.is_protocol_only());

        assert!(!TxType::from_u16(999).is_known());
    }

    #[test]
    fn tx_type_display_prefers_cpp_tag_name() {
        assert_eq!(TxType::PAYMENT.to_string(), "ttPAYMENT");
        assert_eq!(TxType::from_u16(999).to_string(), "txType(999)");
    }

    #[test]
    fn local_tables_cover_protocol_only_entries_and_dispatchable_entries() {
        assert!(
            DISPATCHABLE_TX_TYPES
                .iter()
                .any(|(value, _, _)| *value == 0)
        );
        assert!(EXTRA_KNOWN_TX_TYPES.iter().any(|(value, _)| *value == 22));
    }
}
