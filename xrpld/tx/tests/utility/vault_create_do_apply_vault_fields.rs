//! Integration tests that pin the narrowed Rust
//! `VaultCreate.cpp::doApply()` vault-field population shell to the current
//! C++ behavior.

use tx::{
    VAULT_PRIVATE_FLAG, VAULT_SHARE_NON_TRANSFERABLE_FLAG, VAULT_STRATEGY_FIRST_COME_FIRST_SERVE,
    VaultCreateDoApplyVaultFieldSink, VaultCreateDoApplyVaultFields,
    run_vault_create_do_apply_vault_fields,
};

#[derive(Default)]
struct RecordingSink {
    steps: Vec<String>,
}

impl VaultCreateDoApplyVaultFieldSink for RecordingSink {
    type Asset = &'static str;
    type AccountId = &'static str;
    type Amount = i64;
    type AssetsMaximum = &'static str;
    type ShareId = &'static str;
    type Data = &'static str;

    fn set_asset(&mut self, value: Self::Asset) {
        self.steps.push(format!("asset={value}"));
    }

    fn set_flags(&mut self, value: u32) {
        self.steps.push(format!("flags={value:#x}"));
    }

    fn set_sequence(&mut self, value: u32) {
        self.steps.push(format!("sequence={value}"));
    }

    fn set_owner(&mut self, value: Self::AccountId) {
        self.steps.push(format!("owner={value}"));
    }

    fn set_account(&mut self, value: Self::AccountId) {
        self.steps.push(format!("account={value}"));
    }

    fn set_assets_total(&mut self, value: Self::Amount) {
        self.steps.push(format!("assets_total={value}"));
    }

    fn set_assets_available(&mut self, value: Self::Amount) {
        self.steps.push(format!("assets_available={value}"));
    }

    fn set_loss_unrealized(&mut self, value: Self::Amount) {
        self.steps.push(format!("loss_unrealized={value}"));
    }

    fn set_assets_maximum(&mut self, value: Self::AssetsMaximum) {
        self.steps.push(format!("assets_maximum={value}"));
    }

    fn set_share_mpt_id(&mut self, value: Self::ShareId) {
        self.steps.push(format!("share_mpt_id={value}"));
    }

    fn set_data(&mut self, value: Self::Data) {
        self.steps.push(format!("data={value}"));
    }

    fn set_withdrawal_policy(&mut self, value: u8) {
        self.steps.push(format!("withdrawal_policy={value}"));
    }

    fn set_scale(&mut self, value: u8) {
        self.steps.push(format!("scale={value}"));
    }

    fn insert_vault(&mut self) {
        self.steps.push("insert_vault".to_string());
    }
}

fn sample_fields() -> VaultCreateDoApplyVaultFields<
    &'static str,
    &'static str,
    i64,
    &'static str,
    &'static str,
    &'static str,
> {
    VaultCreateDoApplyVaultFields {
        asset: "USD",
        tx_flags: VAULT_PRIVATE_FLAG | VAULT_SHARE_NON_TRANSFERABLE_FLAG,
        sequence: 9,
        owner: "owner",
        pseudo_id: "pseudo",
        zero_amount: 0,
        assets_maximum: Some("1000"),
        share_mpt_id: "share-id",
        data: Some("abcd"),
        withdrawal_policy: Some(7),
        scale: 6,
    }
}

#[test]
fn vault_create_do_apply_vault_fields_uses_current_cpp_write_order() {
    let mut sink = RecordingSink::default();

    run_vault_create_do_apply_vault_fields(&mut sink, sample_fields());

    assert_eq!(
        sink.steps,
        vec![
            "asset=USD",
            "flags=0x10000",
            "sequence=9",
            "owner=owner",
            "account=pseudo",
            "assets_total=0",
            "assets_available=0",
            "loss_unrealized=0",
            "assets_maximum=1000",
            "share_mpt_id=share-id",
            "data=abcd",
            "withdrawal_policy=7",
            "scale=6",
            "insert_vault",
        ]
    );
}

#[test]
fn vault_create_do_apply_vault_fields_defaults_withdrawal_policy() {
    let mut sink = RecordingSink::default();

    run_vault_create_do_apply_vault_fields(
        &mut sink,
        VaultCreateDoApplyVaultFields {
            withdrawal_policy: None,
            scale: 0,
            assets_maximum: None,
            data: None,
            ..sample_fields()
        },
    );

    assert!(sink.steps.contains(&format!(
        "withdrawal_policy={VAULT_STRATEGY_FIRST_COME_FIRST_SERVE}"
    )));
    assert!(!sink.steps.iter().any(|step| step.starts_with("scale=")));
}

#[test]
fn vault_create_do_apply_vault_fields_masks_flags_to_private_bit() {
    let mut sink = RecordingSink::default();

    run_vault_create_do_apply_vault_fields(
        &mut sink,
        VaultCreateDoApplyVaultFields {
            tx_flags: VAULT_PRIVATE_FLAG | VAULT_SHARE_NON_TRANSFERABLE_FLAG | 0x0040_0000,
            ..sample_fields()
        },
    );

    assert!(sink.steps.contains(&"flags=0x10000".to_string()));
}

#[test]
fn vault_create_do_apply_vault_fields_inserts_after_all_field_writes() {
    let mut sink = RecordingSink::default();

    run_vault_create_do_apply_vault_fields(&mut sink, sample_fields());

    assert_eq!(sink.steps.last(), Some(&"insert_vault".to_string()));
}
