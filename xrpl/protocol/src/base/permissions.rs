//! Permission helpers from `xrpl/protocol/Permissions.*`.

use std::sync::OnceLock;

use basics::base_uint::Uint256;

use crate::{Rules, TxFormats, TxType, feature_id};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[repr(u32)]
pub enum GranularPermissionType {
    TrustlineAuthorize = 65_537,
    TrustlineFreeze = 65_538,
    TrustlineUnfreeze = 65_539,
    AccountDomainSet = 65_540,
    AccountEmailHashSet = 65_541,
    AccountMessageKeySet = 65_542,
    AccountTransferRateSet = 65_543,
    AccountTickSizeSet = 65_544,
    PaymentMint = 65_545,
    PaymentBurn = 65_546,
    MPTokenIssuanceLock = 65_547,
    MPTokenIssuanceUnlock = 65_548,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Delegation {
    Delegable,
    NotDelegable,
}

const GRANULAR_PERMISSIONS: &[(GranularPermissionType, &str, TxType)] = &[
    (
        GranularPermissionType::TrustlineAuthorize,
        "TrustlineAuthorize",
        TxType::TRUST_SET,
    ),
    (
        GranularPermissionType::TrustlineFreeze,
        "TrustlineFreeze",
        TxType::TRUST_SET,
    ),
    (
        GranularPermissionType::TrustlineUnfreeze,
        "TrustlineUnfreeze",
        TxType::TRUST_SET,
    ),
    (
        GranularPermissionType::AccountDomainSet,
        "AccountDomainSet",
        TxType::ACCOUNT_SET,
    ),
    (
        GranularPermissionType::AccountEmailHashSet,
        "AccountEmailHashSet",
        TxType::ACCOUNT_SET,
    ),
    (
        GranularPermissionType::AccountMessageKeySet,
        "AccountMessageKeySet",
        TxType::ACCOUNT_SET,
    ),
    (
        GranularPermissionType::AccountTransferRateSet,
        "AccountTransferRateSet",
        TxType::ACCOUNT_SET,
    ),
    (
        GranularPermissionType::AccountTickSizeSet,
        "AccountTickSizeSet",
        TxType::ACCOUNT_SET,
    ),
    (
        GranularPermissionType::PaymentMint,
        "PaymentMint",
        TxType::PAYMENT,
    ),
    (
        GranularPermissionType::PaymentBurn,
        "PaymentBurn",
        TxType::PAYMENT,
    ),
    (
        GranularPermissionType::MPTokenIssuanceLock,
        "MPTokenIssuanceLock",
        TxType::MPTOKEN_ISSUANCE_SET,
    ),
    (
        GranularPermissionType::MPTokenIssuanceUnlock,
        "MPTokenIssuanceUnlock",
        TxType::MPTOKEN_ISSUANCE_SET,
    ),
];

#[derive(Debug, Default)]
pub struct Permission;

impl Permission {
    pub fn get_instance() -> &'static Self {
        static INSTANCE: OnceLock<Permission> = OnceLock::new();
        INSTANCE.get_or_init(Self::default)
    }

    pub fn get_permission_name(&self, value: u32) -> Option<String> {
        if let Some(name) = Self::granular_permission_from_value(value)
            .and_then(|permission| self.get_granular_name(permission))
        {
            return Some(name.to_owned());
        }

        let tx_type = Self::permission_to_tx_type(value);
        TxFormats::get_instance()
            .find_by_type(tx_type)
            .map(|item| item.name().to_owned())
    }

    pub fn get_granular_value(&self, name: &str) -> Option<u32> {
        GRANULAR_PERMISSIONS
            .iter()
            .find_map(|(permission, permission_name, _)| {
                (*permission_name == name).then_some(*permission as u32)
            })
    }

    pub fn get_granular_name(&self, value: GranularPermissionType) -> Option<&'static str> {
        GRANULAR_PERMISSIONS
            .iter()
            .find_map(|(permission, name, _)| (*permission == value).then_some(*name))
    }

    pub fn get_granular_tx_type(&self, value: GranularPermissionType) -> Option<TxType> {
        GRANULAR_PERMISSIONS
            .iter()
            .find_map(|(permission, _, tx_type)| (*permission == value).then_some(*tx_type))
    }

    pub fn get_tx_feature(&self, tx_type: TxType) -> Option<Uint256> {
        let metadata = TxFormats::get_instance().find_by_type(tx_type)?.metadata();
        amendment_string_to_feature(metadata.amendment)
    }

    pub fn is_delegable(&self, permission_value: u32, rules: &Rules) -> bool {
        if Self::granular_permission_from_value(permission_value).is_some() {
            return true;
        }

        let tx_type = Self::permission_to_tx_type(permission_value);
        let Some(metadata) = TxFormats::get_instance()
            .find_by_type(tx_type)
            .map(|item| item.metadata())
        else {
            return false;
        };

        if metadata.delegable != "Delegation::delegable" {
            return false;
        }

        match amendment_string_to_feature(metadata.amendment) {
            Some(feature) => rules.enabled(&feature),
            None => true,
        }
    }

    pub fn get_permission_value(&self, name: &str) -> Option<u32> {
        self.get_granular_value(name).or_else(|| {
            TxFormats::get_instance()
                .find_by_name(name)
                .map(|item| Self::tx_to_permission_type(item.format_type()))
        })
    }

    pub const fn tx_to_permission_type(type_: TxType) -> u32 {
        type_.to_u16() as u32 + 1
    }

    pub const fn permission_to_tx_type(value: u32) -> TxType {
        TxType::from_u16(value.saturating_sub(1) as u16)
    }

    fn granular_permission_from_value(value: u32) -> Option<GranularPermissionType> {
        GRANULAR_PERMISSIONS
            .iter()
            .find_map(|(permission, _, _)| (*permission as u32 == value).then_some(*permission))
    }
}

fn amendment_string_to_feature(raw: &str) -> Option<Uint256> {
    let feature_name = match raw {
        "uint256{}" => return None,
        "featureAMM" => "AMM",
        "featureAMMClawback" => "AMMClawback",
        "featureBatch" => "Batch",
        "featureClawback" => "Clawback",
        "featureCredentials" => "Credentials",
        "featureDID" => "DID",
        "featureDynamicNFT" => "DynamicNFT",
        "featureLendingProtocol" => "LendingProtocol",
        "featureMPTokensV1" => "MPTokensV1",
        "featurePermissionDelegationV1_1" => "PermissionDelegationV1_1",
        "featurePermissionedDomains" => "PermissionedDomains",
        "featurePriceOracle" => "PriceOracle",
        "featureSingleAssetVault" => "SingleAssetVault",
        "featureXChainBridge" => "XChainBridge",
        "fixNFTokenPageLinks" => "fixNFTokenPageLinks",
        other => other.strip_prefix("feature").unwrap_or(other),
    };

    Some(feature_id(feature_name))
}

#[cfg(test)]
mod tests {
    use super::{GranularPermissionType, Permission};
    use crate::{Rules, TxType, feature_xchain_bridge};

    #[test]
    fn permissions_map_granular_and_tx_level_names() {
        let permission = Permission::get_instance();

        assert_eq!(
            permission.get_granular_value("PaymentMint"),
            Some(GranularPermissionType::PaymentMint as u32)
        );
        assert_eq!(
            permission.get_permission_name(Permission::tx_to_permission_type(TxType::PAYMENT)),
            Some("Payment".to_owned())
        );
    }

    #[test]
    fn delegability_honors_required_amendments() {
        let permission = Permission::get_instance();
        let value = Permission::tx_to_permission_type(TxType::XCHAIN_COMMIT);

        assert!(!permission.is_delegable(value, &Rules::new(std::iter::empty())));
        assert!(permission.is_delegable(value, &Rules::new([feature_xchain_bridge()])));
    }
}
