//! Deterministic the reference implementation helpers.
//!
//! This ports the current compatibility-safe behavior for:
//!
//! - `checkTxPermission(...)`, and
//! - `loadGranularPermission(...)`.

use std::collections::BTreeSet;

use protocol::{NotTec, Ter};

pub const fn tx_to_permission_type(tx_type: u16) -> u32 {
    tx_type as u32 + 1
}

pub const fn permission_to_tx_type(permission_value: u32) -> u16 {
    permission_value.saturating_sub(1) as u16
}

pub fn run_check_tx_permission(delegate_permissions: Option<&[u32]>, tx_type: u16) -> NotTec {
    let Some(permission_array) = delegate_permissions else {
        return Ter::TER_NO_DELEGATE_PERMISSION;
    };

    let tx_permission = tx_to_permission_type(tx_type);
    for &permission in permission_array {
        if permission == tx_permission {
            return Ter::TES_SUCCESS;
        }
    }

    Ter::TER_NO_DELEGATE_PERMISSION
}

pub fn load_granular_permissions<TxType, MapGranularTxType>(
    delegate_permissions: Option<&[u32]>,
    tx_type: TxType,
    mut map_granular_tx_type: MapGranularTxType,
) -> BTreeSet<u32>
where
    TxType: Copy + Eq,
    MapGranularTxType: FnMut(u32) -> Option<TxType>,
{
    let Some(permission_array) = delegate_permissions else {
        return BTreeSet::new();
    };

    let mut granular_permissions = BTreeSet::new();
    for &permission in permission_array {
        if map_granular_tx_type(permission).is_some_and(|mapped| mapped == tx_type) {
            granular_permissions.insert(permission);
        }
    }

    granular_permissions
}

#[cfg(test)]
mod tests {
    use super::{
        load_granular_permissions, permission_to_tx_type, run_check_tx_permission,
        tx_to_permission_type,
    };
    use protocol::Ter;

    #[test]
    fn permission_type_helpers_match_cpp_translation() {
        assert_eq!(tx_to_permission_type(0), 1);
        assert_eq!(tx_to_permission_type(42), 43);
        assert_eq!(permission_to_tx_type(1), 0);
        assert_eq!(permission_to_tx_type(43), 42);
        assert_eq!(permission_to_tx_type(0), 0);
    }

    #[test]
    fn check_tx_permission_delegate_scan() {
        assert_eq!(
            run_check_tx_permission(None, 7),
            Ter::TER_NO_DELEGATE_PERMISSION
        );
        assert_eq!(
            run_check_tx_permission(Some(&[4, tx_to_permission_type(7), 9]), 7),
            Ter::TES_SUCCESS
        );
        assert_eq!(
            run_check_tx_permission(Some(&[4, 9, 10]), 7),
            Ter::TER_NO_DELEGATE_PERMISSION
        );
    }

    #[test]
    fn load_granular_permissions_filters_only_matching_transaction_type() {
        let loaded = load_granular_permissions(Some(&[65_540, 65_541, 99]), 5_u16, |permission| {
            match permission {
                65_540 => Some(5),
                65_541 => Some(6),
                _ => None,
            }
        });

        assert_eq!(loaded.into_iter().collect::<Vec<_>>(), vec![65_540]);
        assert!(load_granular_permissions::<u16, _>(None, 5, |_| Some(5)).is_empty());
    }
}
