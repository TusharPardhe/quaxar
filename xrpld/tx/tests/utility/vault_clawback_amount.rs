//! Integration tests that pin the narrowed Rust `clawbackAmount(...)` helper
//! to the current C++ behavior.

use std::cell::Cell;

use tx::{VaultClawbackAmountVault, select_vault_clawback_amount};

#[derive(Debug, Clone, PartialEq, Eq)]
struct TestVault {
    asset: &'static str,
    owner: &'static str,
    share_mpt_id: &'static str,
}

impl VaultClawbackAmountVault for TestVault {
    type Asset = &'static str;
    type AccountId = &'static str;
    type ShareId = &'static str;

    fn asset(&self) -> &Self::Asset {
        &self.asset
    }

    fn owner(&self) -> &Self::AccountId {
        &self.owner
    }

    fn share_mpt_id(&self) -> &Self::ShareId {
        &self.share_mpt_id
    }
}

fn test_vault() -> TestVault {
    TestVault {
        asset: "USD",
        owner: "vault-owner",
        share_mpt_id: "share-id",
    }
}

#[test]
fn vault_clawback_amount_returns_explicit_amount_unchanged() {
    let share_called = Cell::new(false);
    let asset_called = Cell::new(false);

    let result = select_vault_clawback_amount(
        &test_vault(),
        Some("explicit-amount"),
        &"vault-owner",
        |_| {
            share_called.set(true);
            "share-amount"
        },
        |_| {
            asset_called.set(true);
            "asset-amount"
        },
    );

    assert_eq!(result, "explicit-amount");
    assert!(!share_called.get());
    assert!(!asset_called.get());
}

#[test]
fn vault_clawback_amount_selects_zero_share_amount_for_owner() {
    let asset_called = Cell::new(false);

    let result = select_vault_clawback_amount(
        &test_vault(),
        None::<&'static str>,
        &"vault-owner",
        |share_id| {
            assert_eq!(share_id, "share-id");
            "share-amount"
        },
        |_| {
            asset_called.set(true);
            "asset-amount"
        },
    );

    assert_eq!(result, "share-amount");
    assert!(!asset_called.get());
}

#[test]
fn vault_clawback_amount_selects_zero_vault_asset_amount_for_non_owner() {
    let share_called = Cell::new(false);

    let result = select_vault_clawback_amount(
        &test_vault(),
        None::<&'static str>,
        &"issuer",
        |_| {
            share_called.set(true);
            "share-amount"
        },
        |asset| {
            assert_eq!(asset, "USD");
            "asset-amount"
        },
    );

    assert_eq!(result, "asset-amount");
    assert!(!share_called.get());
}

#[test]
fn vault_clawback_amount_uses_owner_comparison_before_asset_fallback() {
    let result = select_vault_clawback_amount(
        &test_vault(),
        None::<String>,
        &"vault-owner",
        |share_id| format!("share:{share_id}"),
        |asset| format!("asset:{asset}"),
    );

    assert_eq!(result, "share:share-id");
}
