//! Integration tests that pin the narrowed Rust `assetsToClawback(...)` helper
//! to the current C++ behavior.

use std::{cell::Cell, rc::Rc};

use protocol::{Ter, trans_token};
use tx::{
    VaultClawbackAssetsToClawback, VaultClawbackAssetsToClawbackVault,
    compute_vault_clawback_assets_to_clawback,
};

#[derive(Debug, Clone, PartialEq, Eq)]
struct TestVault {
    assets_available: i64,
}

impl VaultClawbackAssetsToClawbackVault for TestVault {
    type Amount = i64;

    fn assets_available(&self) -> &Self::Amount {
        &self.assets_available
    }
}

#[test]
fn vault_clawback_assets_to_clawback_rejects_amount_asset_mismatch() {
    let holds_called = Cell::new(false);

    let result = compute_vault_clawback_assets_to_clawback(
        &TestVault {
            assets_available: 5,
        },
        &10_i64,
        |_, _| false,
        |_| false,
        || {
            holds_called.set(true);
            9_i64
        },
        |_| Some(1_i64),
        |_| Ok::<_, ()>(Some(1_i64)),
        |_| Ok::<_, ()>(Some(1_i64)),
        |_| Ok::<_, ()>(Some(1_i64)),
    );

    assert_eq!(result, Err(Ter::TEC_INTERNAL));
    assert_eq!(trans_token(result.unwrap_err()), "tecINTERNAL");
    assert!(!holds_called.get());
}

#[test]
fn vault_clawback_assets_to_clawback_uses_holder_shares_directly_for_zero_amount() {
    let steps = Rc::new(std::cell::RefCell::new(Vec::new()));

    let result = compute_vault_clawback_assets_to_clawback(
        &TestVault {
            assets_available: 5,
        },
        &0_i64,
        |_, _| true,
        |amount| *amount == 0,
        {
            let steps = Rc::clone(&steps);
            move || {
                steps.borrow_mut().push("account_holds".to_string());
                9_i64
            }
        },
        {
            let steps = Rc::clone(&steps);
            move |shares| {
                steps
                    .borrow_mut()
                    .push(format!("shares_to_assets_from_holds:{shares}"));
                Some(4_i64)
            }
        },
        |_| Ok::<_, ()>(Some(1_i64)),
        |_| Ok::<_, ()>(Some(1_i64)),
        |_| Ok::<_, ()>(Some(1_i64)),
    );

    assert_eq!(
        result,
        Ok(VaultClawbackAssetsToClawback {
            assets_recovered: 4_i64,
            shares_destroyed: 9_i64,
        })
    );
    assert_eq!(
        steps.borrow().as_slice(),
        ["account_holds", "shares_to_assets_from_holds:9"]
    );
}

#[test]
fn vault_clawback_assets_to_clawback_clamps_and_retries_with_truncated_shares() {
    let steps = Rc::new(std::cell::RefCell::new(Vec::new()));

    let result = compute_vault_clawback_assets_to_clawback(
        &TestVault {
            assets_available: 5,
        },
        &9_i64,
        |_, _| true,
        |_| false,
        || 9_i64,
        |_| Some(1_i64),
        {
            let steps = Rc::clone(&steps);
            move |amount| {
                steps
                    .borrow_mut()
                    .push(format!("assets_to_shares:{amount}"));
                Ok::<_, ()>(Some(8_i64))
            }
        },
        {
            let steps = Rc::clone(&steps);
            move |amount| {
                steps
                    .borrow_mut()
                    .push(format!("assets_to_shares_truncated:{amount}"));
                Ok::<_, ()>(Some(5_i64))
            }
        },
        {
            let steps = Rc::clone(&steps);
            let call = Cell::new(0_u32);
            move |shares| {
                let next = call.get() + 1;
                call.set(next);
                steps
                    .borrow_mut()
                    .push(format!("shares_to_assets:{shares}"));
                if next == 1 {
                    Ok::<_, ()>(Some(7_i64))
                } else {
                    Ok::<_, ()>(Some(5_i64))
                }
            }
        },
    );

    assert_eq!(
        result,
        Ok(VaultClawbackAssetsToClawback {
            assets_recovered: 5_i64,
            shares_destroyed: 5_i64,
        })
    );
    assert_eq!(
        steps.borrow().as_slice(),
        [
            "assets_to_shares:9",
            "shares_to_assets:8",
            "assets_to_shares_truncated:5",
            "shares_to_assets:5",
        ]
    );
}

#[test]
fn vault_clawback_assets_to_clawback_rejects_invalid_post_clamp_rounding() {
    let result = compute_vault_clawback_assets_to_clawback(
        &TestVault {
            assets_available: 5,
        },
        &9_i64,
        |_, _| true,
        |_| false,
        || 9_i64,
        |_| Some(1_i64),
        |_| Ok::<_, ()>(Some(8_i64)),
        |_| Ok::<_, ()>(Some(5_i64)),
        {
            let call = Cell::new(0_u32);
            move |_| {
                let next = call.get() + 1;
                call.set(next);
                if next == 1 {
                    Ok::<_, ()>(Some(7_i64))
                } else {
                    Ok::<_, ()>(Some(6_i64))
                }
            }
        },
    );

    assert_eq!(result, Err(Ter::TEC_INTERNAL));
    assert_eq!(trans_token(result.unwrap_err()), "tecINTERNAL");
}

#[test]
fn vault_clawback_assets_to_clawback_maps_nonzero_overflow_to_path_dry() {
    let shares_to_assets_called = Cell::new(false);

    let result = compute_vault_clawback_assets_to_clawback(
        &TestVault {
            assets_available: 5,
        },
        &9_i64,
        |_, _| true,
        |_| false,
        || 9_i64,
        |_| Some(1_i64),
        |_| Err::<Option<i64>, &'static str>("overflow"),
        |_| Ok::<_, &'static str>(Some(5_i64)),
        |_| {
            shares_to_assets_called.set(true);
            Ok::<_, &'static str>(Some(4_i64))
        },
    );

    assert_eq!(result, Err(Ter::TEC_PATH_DRY));
    assert_eq!(trans_token(result.unwrap_err()), "tecPATH_DRY");
    assert!(!shares_to_assets_called.get());
}
