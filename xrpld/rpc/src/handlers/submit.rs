use tx::{CheckValidityResult, Validity};

pub const INVALID_TRANSACTION_ERROR: &str = "invalidTransaction";
pub const FAILS_LOCAL_CHECKS_PREFIX: &str = "fails local checks: ";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubmitValidityFailure {
    pub error: &'static str,
    pub error_exception: String,
}

pub fn run_submit_validity_gate(
    check_sigs: bool,
    force_sig_good_only: impl FnOnce(),
    check_validity: impl FnOnce() -> CheckValidityResult,
) -> Result<(), SubmitValidityFailure> {
    if !check_sigs {
        force_sig_good_only();
    }

    let result = check_validity();
    if result.validity != Validity::Valid {
        return Err(SubmitValidityFailure {
            error: INVALID_TRANSACTION_ERROR,
            error_exception: format!("{FAILS_LOCAL_CHECKS_PREFIX}{}", result.reason),
        });
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        FAILS_LOCAL_CHECKS_PREFIX, INVALID_TRANSACTION_ERROR, SubmitValidityFailure,
        run_submit_validity_gate, submit_semantic_preflight,
    };
    use protocol::{
        AccountID, Permission, Rules, STAmount, STArray, STObject, STTx, STXChainBridge, Ter,
        TxType, get_field_by_symbol,
    };
    use std::cell::RefCell;
    use tx::{CheckValidityResult, Validity};
    use xrpl_core::HashRouterFlags;

    fn account(fill: u8) -> AccountID {
        AccountID::from_array([fill; 20])
    }

    fn rules_with_tx_enabled(tx_type: TxType) -> Rules {
        Permission::get_instance()
            .get_tx_feature(tx_type)
            .map(|feature| Rules::new([feature]))
            .unwrap_or_default()
    }

    #[test]
    fn submit_force_sig_good_only_runs_before_validity_check() {
        let calls = RefCell::new(Vec::new());

        let result = run_submit_validity_gate(
            false,
            || calls.borrow_mut().push("force"),
            || {
                calls.borrow_mut().push("check");
                CheckValidityResult {
                    validity: Validity::Valid,
                    reason: String::new(),
                    flags_to_set: HashRouterFlags::UNDEFINED,
                }
            },
        );

        assert_eq!(result, Ok(()));
        assert_eq!(calls.into_inner(), vec!["force", "check"]);
    }

    #[test]
    fn submit_invalidity_is_mapped_to_current_rpc_error_fields() {
        let result = run_submit_validity_gate(
            true,
            || panic!("forceValidity must not run when signatures are enabled"),
            || CheckValidityResult {
                validity: Validity::SigBad,
                reason: "Invalid signature.".to_string(),
                flags_to_set: HashRouterFlags::UNDEFINED,
            },
        );

        assert_eq!(
            result,
            Err(SubmitValidityFailure {
                error: INVALID_TRANSACTION_ERROR,
                error_exception: format!("{FAILS_LOCAL_CHECKS_PREFIX}Invalid signature."),
            })
        );
    }

    #[test]
    fn payment_self_destination_maps_to_tem_redundant_like_reference() {
        let source = account(0x11);
        let payment = STTx::new(TxType::PAYMENT, |object| {
            object.set_account_id(get_field_by_symbol("sfAccount"), source);
            object.set_account_id(get_field_by_symbol("sfDestination"), source);
            object.set_field_amount(
                get_field_by_symbol("sfAmount"),
                STAmount::new_native(1_000_000, false),
            );
            object.set_field_amount(
                get_field_by_symbol("sfFee"),
                STAmount::new_native(10, false),
            );
        });

        assert_eq!(
            submit_semantic_preflight(&payment, &Rules::default()),
            Ter::TEM_REDUNDANT
        );
    }

    #[test]
    fn signer_list_set_duplicate_signers_map_to_tem_bad_signer_like_reference() {
        let source = account(0x22);
        let duplicate_signer = account(0x33);
        let mut signer_entries = STArray::new(get_field_by_symbol("sfSignerEntries"));
        for _ in 0..2 {
            let mut signer_entry =
                STObject::make_inner_object(get_field_by_symbol("sfSignerEntry"));
            signer_entry.set_account_id(get_field_by_symbol("sfAccount"), duplicate_signer);
            signer_entry.set_field_u16(get_field_by_symbol("sfSignerWeight"), 1);
            signer_entries.push_back(signer_entry);
        }

        let signer_list_set = STTx::new(TxType::SIGNER_LIST_SET, move |object| {
            object.set_account_id(get_field_by_symbol("sfAccount"), source);
            object.set_field_u32(get_field_by_symbol("sfSignerQuorum"), 1);
            object.set_field_array(get_field_by_symbol("sfSignerEntries"), signer_entries);
            object.set_field_amount(
                get_field_by_symbol("sfFee"),
                STAmount::new_native(10, false),
            );
        });

        assert_eq!(
            submit_semantic_preflight(&signer_list_set, &Rules::default()),
            Ter::TEM_BAD_SIGNER
        );
    }

    #[test]
    fn escrow_create_invalid_expiration_maps_to_tem_bad_expiration_like_reference() {
        let source = account(0x44);
        let destination = account(0x45);
        let escrow = STTx::new(TxType::ESCROW_CREATE, |object| {
            object.set_account_id(get_field_by_symbol("sfAccount"), source);
            object.set_account_id(get_field_by_symbol("sfDestination"), destination);
            object.set_field_amount(
                get_field_by_symbol("sfAmount"),
                STAmount::new_native(1_000_000, false),
            );
            object.set_field_u32(get_field_by_symbol("sfFinishAfter"), 100);
            object.set_field_u32(get_field_by_symbol("sfCancelAfter"), 100);
            object.set_field_amount(
                get_field_by_symbol("sfFee"),
                STAmount::new_native(10, false),
            );
        });

        assert_eq!(
            submit_semantic_preflight(&escrow, &Rules::default()),
            Ter::TEM_BAD_EXPIRATION
        );
    }

    #[test]
    fn check_create_negative_send_max_maps_to_tem_bad_amount_like_reference() {
        let source = account(0x46);
        let destination = account(0x47);
        let check = STTx::new(TxType::CHECK_CREATE, |object| {
            object.set_account_id(get_field_by_symbol("sfAccount"), source);
            object.set_account_id(get_field_by_symbol("sfDestination"), destination);
            object.set_field_amount(
                get_field_by_symbol("sfSendMax"),
                STAmount::new_native(10, true),
            );
            object.set_field_amount(
                get_field_by_symbol("sfFee"),
                STAmount::new_native(10, false),
            );
        });

        assert_eq!(
            submit_semantic_preflight(&check, &Rules::default()),
            Ter::TEM_BAD_AMOUNT
        );
    }

    #[test]
    fn paychan_create_negative_amount_maps_to_tem_bad_amount_like_reference() {
        let source = account(0x48);
        let destination = account(0x49);
        let channel = STTx::new(TxType::PAYCHAN_CREATE, |object| {
            object.set_account_id(get_field_by_symbol("sfAccount"), source);
            object.set_account_id(get_field_by_symbol("sfDestination"), destination);
            object.set_field_amount(
                get_field_by_symbol("sfAmount"),
                STAmount::new_native(10, true),
            );
            object.set_field_u32(get_field_by_symbol("sfSettleDelay"), 86_400);
            object.set_field_vl(
                get_field_by_symbol("sfPublicKey"),
                &protocol::genesis_public_key(),
            );
            object.set_field_amount(
                get_field_by_symbol("sfFee"),
                STAmount::new_native(10, false),
            );
        });

        assert_eq!(
            submit_semantic_preflight(&channel, &Rules::default()),
            Ter::TEM_BAD_AMOUNT
        );
    }

    #[test]
    fn paychan_fund_negative_amount_maps_to_tem_bad_amount_like_reference() {
        let source = account(0x4A);
        let channel = STTx::new(TxType::PAYCHAN_FUND, |object| {
            object.set_account_id(get_field_by_symbol("sfAccount"), source);
            object.set_field_h256(
                get_field_by_symbol("sfChannel"),
                basics::base_uint::Uint256::from_u64(1),
            );
            object.set_field_amount(
                get_field_by_symbol("sfAmount"),
                STAmount::new_native(10, true),
            );
            object.set_field_amount(
                get_field_by_symbol("sfFee"),
                STAmount::new_native(10, false),
            );
        });

        assert_eq!(
            submit_semantic_preflight(&channel, &Rules::default()),
            Ter::TEM_BAD_AMOUNT
        );
    }

    #[test]
    fn offer_create_same_native_asset_prefers_tem_bad_offer_like_reference() {
        let source = account(0x4B);
        let offer = STTx::new(TxType::OFFER_CREATE, |object| {
            object.set_account_id(get_field_by_symbol("sfAccount"), source);
            object.set_field_amount(
                get_field_by_symbol("sfTakerPays"),
                STAmount::new_native(10, true),
            );
            object.set_field_amount(
                get_field_by_symbol("sfTakerGets"),
                STAmount::new_native(100, false),
            );
            object.set_field_amount(
                get_field_by_symbol("sfFee"),
                STAmount::new_native(10, false),
            );
        });

        assert_eq!(
            submit_semantic_preflight(&offer, &Rules::default()),
            Ter::TEM_BAD_OFFER
        );
    }

    #[test]
    fn offer_create_same_iou_asset_maps_to_tem_redundant_like_reference() {
        let source = account(0x4C);
        let issuer = account(0x4D);
        let issue = protocol::Issue::new(protocol::currency_from_string("USD"), issuer);
        let offer = STTx::new(TxType::OFFER_CREATE, |object| {
            object.set_account_id(get_field_by_symbol("sfAccount"), source);
            object.set_field_amount(
                get_field_by_symbol("sfTakerPays"),
                STAmount::new_with_asset(get_field_by_symbol("sfTakerPays"), issue, 6, 0, false),
            );
            object.set_field_amount(
                get_field_by_symbol("sfTakerGets"),
                STAmount::new_with_asset(get_field_by_symbol("sfTakerGets"), issue, 5, 0, false),
            );
            object.set_field_amount(
                get_field_by_symbol("sfFee"),
                STAmount::new_native(10, false),
            );
        });

        assert_eq!(
            submit_semantic_preflight(&offer, &Rules::default()),
            Ter::TEM_REDUNDANT
        );
    }

    #[test]
    fn clawback_issue_base_maps_to_tem_bad_amount_like_reference() {
        let issuer = account(0x4E);
        let issue = protocol::Issue::new(protocol::currency_from_string("USD"), issuer);
        let clawback = STTx::new(TxType::CLAWBACK, |object| {
            object.set_account_id(get_field_by_symbol("sfAccount"), issuer);
            object.set_field_amount(
                get_field_by_symbol("sfAmount"),
                STAmount::new_with_asset(get_field_by_symbol("sfAmount"), issue, 10, 0, false),
            );
            object.set_field_amount(
                get_field_by_symbol("sfFee"),
                STAmount::new_native(10, false),
            );
        });

        assert_eq!(
            submit_semantic_preflight(&clawback, &rules_with_tx_enabled(TxType::CLAWBACK)),
            Ter::TEM_BAD_AMOUNT
        );
    }

    #[test]
    fn vault_deposit_negative_amount_maps_to_tem_bad_amount_like_reference() {
        let source = account(0x4F);
        let vault_deposit = STTx::new(TxType::VAULT_DEPOSIT, |object| {
            object.set_account_id(get_field_by_symbol("sfAccount"), source);
            object.set_field_h256(
                get_field_by_symbol("sfVaultID"),
                basics::base_uint::Uint256::from_u64(1),
            );
            object.set_field_amount(
                get_field_by_symbol("sfAmount"),
                STAmount::new_native(10, true),
            );
            object.set_field_amount(
                get_field_by_symbol("sfFee"),
                STAmount::new_native(10, false),
            );
        });

        assert_eq!(
            submit_semantic_preflight(
                &vault_deposit,
                &rules_with_tx_enabled(TxType::VAULT_DEPOSIT)
            ),
            Ter::TEM_BAD_AMOUNT
        );
    }

    #[test]
    fn vault_withdraw_negative_amount_maps_to_tem_bad_amount_like_reference() {
        let source = account(0x50);
        let vault_withdraw = STTx::new(TxType::VAULT_WITHDRAW, |object| {
            object.set_account_id(get_field_by_symbol("sfAccount"), source);
            object.set_field_h256(
                get_field_by_symbol("sfVaultID"),
                basics::base_uint::Uint256::from_u64(1),
            );
            object.set_field_amount(
                get_field_by_symbol("sfAmount"),
                STAmount::new_native(10, true),
            );
            object.set_field_amount(
                get_field_by_symbol("sfFee"),
                STAmount::new_native(10, false),
            );
        });

        assert_eq!(
            submit_semantic_preflight(
                &vault_withdraw,
                &rules_with_tx_enabled(TxType::VAULT_WITHDRAW)
            ),
            Ter::TEM_BAD_AMOUNT
        );
    }

    #[test]
    fn did_set_empty_fields_map_to_tem_empty_did_like_reference() {
        let source = account(0x51);
        let did_set = STTx::new(TxType::DID_SET, |object| {
            object.set_account_id(get_field_by_symbol("sfAccount"), source);
            object.set_field_amount(
                get_field_by_symbol("sfFee"),
                STAmount::new_native(10, false),
            );
        });

        assert_eq!(
            submit_semantic_preflight(&did_set, &rules_with_tx_enabled(TxType::DID_SET)),
            Ter::TEM_EMPTY_DID
        );
    }

    #[test]
    fn credential_delete_without_subject_or_issuer_maps_to_tem_malformed_like_reference() {
        let source = account(0x52);
        let credential_delete = STTx::new(TxType::CREDENTIAL_DELETE, |object| {
            object.set_account_id(get_field_by_symbol("sfAccount"), source);
            object.set_field_vl(get_field_by_symbol("sfCredentialType"), &[0xAB, 0xCD]);
            object.set_field_amount(
                get_field_by_symbol("sfFee"),
                STAmount::new_native(10, false),
            );
        });

        assert_eq!(
            submit_semantic_preflight(
                &credential_delete,
                &rules_with_tx_enabled(TxType::CREDENTIAL_DELETE)
            ),
            Ter::TEM_MALFORMED
        );
    }

    #[test]
    fn vault_set_no_requested_updates_maps_to_tem_malformed_like_reference() {
        let source = account(0x53);
        let vault_set = STTx::new(TxType::VAULT_SET, |object| {
            object.set_account_id(get_field_by_symbol("sfAccount"), source);
            object.set_field_h256(
                get_field_by_symbol("sfVaultID"),
                basics::base_uint::Uint256::from_u64(1),
            );
            object.set_field_amount(
                get_field_by_symbol("sfFee"),
                STAmount::new_native(10, false),
            );
        });

        assert_eq!(
            submit_semantic_preflight(&vault_set, &rules_with_tx_enabled(TxType::VAULT_SET)),
            Ter::TEM_MALFORMED
        );
    }

    #[test]
    fn xchain_create_bridge_nondoor_owner_maps_to_tem_xchain_bridge_nondoor_owner() {
        let source = account(0x54);
        let locking_door = account(0x55);
        let issuing_door = account(0x56);
        let bridge = STXChainBridge::from_parts(
            locking_door,
            protocol::xrp_issue(),
            issuing_door,
            protocol::xrp_issue(),
        );
        let tx = STTx::new(TxType::XCHAIN_CREATE_BRIDGE, |object| {
            object.set_account_id(get_field_by_symbol("sfAccount"), source);
            object.set_field_xchain_bridge(get_field_by_symbol("sfXChainBridge"), bridge);
            object.set_field_amount(
                get_field_by_symbol("sfSignatureReward"),
                STAmount::new_native(10, false),
            );
            object.set_field_amount(
                get_field_by_symbol("sfFee"),
                STAmount::new_native(10, false),
            );
        });

        assert_eq!(
            submit_semantic_preflight(&tx, &rules_with_tx_enabled(TxType::XCHAIN_CREATE_BRIDGE)),
            Ter::TEM_XCHAIN_BRIDGE_NONDOOR_OWNER
        );
    }

    #[test]
    fn xchain_modify_bridge_without_updates_maps_to_tem_malformed_like_reference() {
        let source = account(0x57);
        let bridge = STXChainBridge::from_parts(
            source,
            protocol::xrp_issue(),
            account(0x58),
            protocol::xrp_issue(),
        );
        let tx = STTx::new(TxType::XCHAIN_MODIFY_BRIDGE, |object| {
            object.set_account_id(get_field_by_symbol("sfAccount"), source);
            object.set_field_xchain_bridge(get_field_by_symbol("sfXChainBridge"), bridge);
            object.set_field_amount(
                get_field_by_symbol("sfFee"),
                STAmount::new_native(10, false),
            );
        });

        assert_eq!(
            submit_semantic_preflight(&tx, &rules_with_tx_enabled(TxType::XCHAIN_MODIFY_BRIDGE)),
            Ter::TEM_MALFORMED
        );
    }

    #[test]
    fn nftoken_create_offer_without_sell_flag_or_owner_maps_to_tem_malformed() {
        let source = account(0x59);
        let tx = STTx::new(TxType::NFTOKEN_CREATE_OFFER, |object| {
            object.set_account_id(get_field_by_symbol("sfAccount"), source);
            object.set_field_h256(
                get_field_by_symbol("sfNFTokenID"),
                basics::base_uint::Uint256::from_u64(1),
            );
            object.set_field_amount(
                get_field_by_symbol("sfAmount"),
                STAmount::new_native(100, false),
            );
            object.set_field_amount(
                get_field_by_symbol("sfFee"),
                STAmount::new_native(10, false),
            );
        });

        assert_eq!(
            submit_semantic_preflight(&tx, &rules_with_tx_enabled(TxType::XCHAIN_CREATE_CLAIM_ID)),
            Ter::TEM_MALFORMED
        );
    }

    #[test]
    fn nftoken_create_offer_negative_amount_maps_to_tem_bad_amount() {
        let source = account(0x5A);
        let tx = STTx::new(TxType::NFTOKEN_CREATE_OFFER, |object| {
            object.set_account_id(get_field_by_symbol("sfAccount"), source);
            object.set_field_h256(
                get_field_by_symbol("sfNFTokenID"),
                basics::base_uint::Uint256::from_u64(2),
            );
            object.set_field_amount(
                get_field_by_symbol("sfAmount"),
                STAmount::new_native(1, true),
            );
            object.set_field_amount(
                get_field_by_symbol("sfFee"),
                STAmount::new_native(10, false),
            );
        });

        assert_eq!(
            submit_semantic_preflight(&tx, &rules_with_tx_enabled(TxType::XCHAIN_COMMIT)),
            Ter::TEM_BAD_AMOUNT
        );
    }

    #[test]
    fn xchain_create_claim_id_negative_reward_maps_to_tem_xchain_bridge_bad_reward_amount() {
        let source = account(0x5B);
        let bridge = STXChainBridge::from_parts(
            source,
            protocol::xrp_issue(),
            account(0x5C),
            protocol::xrp_issue(),
        );
        let tx = STTx::new(TxType::XCHAIN_CREATE_CLAIM_ID, |object| {
            object.set_account_id(get_field_by_symbol("sfAccount"), source);
            object.set_field_xchain_bridge(get_field_by_symbol("sfXChainBridge"), bridge);
            object.set_field_amount(
                get_field_by_symbol("sfSignatureReward"),
                STAmount::new_native(1, true),
            );
            object.set_field_amount(
                get_field_by_symbol("sfFee"),
                STAmount::new_native(10, false),
            );
        });

        assert_eq!(
            submit_semantic_preflight(
                &tx,
                &rules_with_tx_enabled(TxType::XCHAIN_ADD_CLAIM_ATTESTATION)
            ),
            Ter::TEM_XCHAIN_BRIDGE_BAD_REWARD_AMOUNT
        );
    }

    #[test]
    fn xchain_commit_negative_amount_maps_to_tem_bad_amount() {
        let source = account(0x5D);
        let bridge = STXChainBridge::from_parts(
            source,
            protocol::xrp_issue(),
            account(0x5E),
            protocol::xrp_issue(),
        );
        let tx = STTx::new(TxType::XCHAIN_COMMIT, |object| {
            object.set_account_id(get_field_by_symbol("sfAccount"), source);
            object.set_field_xchain_bridge(get_field_by_symbol("sfXChainBridge"), bridge);
            object.set_field_amount(
                get_field_by_symbol("sfAmount"),
                STAmount::new_native(1, true),
            );
            object.set_field_amount(
                get_field_by_symbol("sfFee"),
                STAmount::new_native(10, false),
            );
        });

        assert_eq!(
            submit_semantic_preflight(&tx, &rules_with_tx_enabled(TxType::XCHAIN_COMMIT)),
            Ter::TEM_BAD_AMOUNT
        );
    }

    #[test]
    fn xchain_add_claim_attestation_invalid_public_key_maps_to_tem_xchain_bad_proof() {
        let source = account(0x5F);
        let bridge = STXChainBridge::from_parts(
            account(0x60),
            protocol::xrp_issue(),
            account(0x61),
            protocol::xrp_issue(),
        );
        let tx = STTx::new(TxType::XCHAIN_ADD_CLAIM_ATTESTATION, |object| {
            object.set_account_id(get_field_by_symbol("sfAccount"), source);
            object.set_field_xchain_bridge(get_field_by_symbol("sfXChainBridge"), bridge);
            object.set_account_id(
                get_field_by_symbol("sfAttestationSignerAccount"),
                account(0x62),
            );
            object.set_field_vl(get_field_by_symbol("sfPublicKey"), &[]);
            object.set_field_vl(get_field_by_symbol("sfSignature"), &[0x01, 0x02]);
            object.set_account_id(get_field_by_symbol("sfOtherChainSource"), account(0x63));
            object.set_field_amount(
                get_field_by_symbol("sfAmount"),
                STAmount::new_native(10, false),
            );
            object.set_account_id(
                get_field_by_symbol("sfAttestationRewardAccount"),
                account(0x64),
            );
            object.set_field_u8(get_field_by_symbol("sfWasLockingChainSend"), 1);
            object.set_field_u64(get_field_by_symbol("sfXChainClaimID"), 1);
            object.set_field_amount(
                get_field_by_symbol("sfFee"),
                STAmount::new_native(10, false),
            );
        });

        assert_eq!(
            submit_semantic_preflight(
                &tx,
                &rules_with_tx_enabled(TxType::XCHAIN_ADD_CLAIM_ATTESTATION)
            ),
            Ter::TEM_XCHAIN_BAD_PROOF
        );
    }

    #[test]
    fn mptoken_issuance_set_without_mutation_maps_to_tem_malformed() {
        let source = account(0x65);
        let tx = STTx::new(TxType::MPTOKEN_ISSUANCE_SET, |object| {
            object.set_account_id(get_field_by_symbol("sfAccount"), source);
            object.set_field_h192(
                get_field_by_symbol("sfMPTokenIssuanceID"),
                basics::base_uint::Uint192::from_u64(1),
            );
            object.set_field_amount(
                get_field_by_symbol("sfFee"),
                STAmount::new_native(10, false),
            );
        });

        assert_eq!(
            submit_semantic_preflight(&tx, &rules_with_tx_enabled(TxType::MPTOKEN_ISSUANCE_SET)),
            Ter::TEM_MALFORMED
        );
    }

    #[test]
    fn batch_without_mode_flag_maps_to_tem_invalid_flag() {
        let source = account(0x66);
        let tx = STTx::new(TxType::BATCH, |object| {
            object.set_account_id(get_field_by_symbol("sfAccount"), source);
            object.set_field_amount(
                get_field_by_symbol("sfFee"),
                STAmount::new_native(10, false),
            );
        });

        assert_eq!(
            submit_semantic_preflight(&tx, &rules_with_tx_enabled(TxType::BATCH)),
            Ter::TEM_INVALID_FLAG
        );
    }

    #[test]
    fn amm_clawback_equal_assets_map_to_tem_malformed() {
        let source = account(0x67);
        let tx = STTx::new(TxType::AMM_CLAWBACK, |object| {
            object.set_account_id(get_field_by_symbol("sfAccount"), source);
            object.set_account_id(get_field_by_symbol("sfHolder"), account(0x68));
            object.set_field_issue(
                get_field_by_symbol("sfAsset"),
                protocol::STIssue::new_with_asset(
                    get_field_by_symbol("sfAsset"),
                    protocol::xrp_issue(),
                ),
            );
            object.set_field_issue(
                get_field_by_symbol("sfAsset2"),
                protocol::STIssue::new_with_asset(
                    get_field_by_symbol("sfAsset2"),
                    protocol::xrp_issue(),
                ),
            );
            object.set_field_amount(
                get_field_by_symbol("sfFee"),
                STAmount::new_native(10, false),
            );
        });

        assert_eq!(
            submit_semantic_preflight(&tx, &rules_with_tx_enabled(TxType::AMM_CLAWBACK)),
            Ter::TEM_MALFORMED
        );
    }
}

use crate::state::context::{RpcRequestContext, RpcRuntime};
use crate::status::{RpcErrorCode, Status};
use app::{NetworkOpsCurrentLedgerState, SharedTransaction, Transaction};
use ledger::Ledger;
use protocol::ter::trans_human;
use protocol::{
    AccountID, Asset, CurrentTransactionRulesGuard, JsonOptions, JsonValue, Keylet,
    LedgerEntryType, Permission, Rules, STLedgerEntry, STTx, STXChainBridge, Ter, TxType,
    XRPAmount, account_keylet, bridge_keylet_from_door_issue, check_keylet_from_key,
    credential_keylet, deposit_preauth_keylet, did_keylet, escrow_keylet, feature_amm, feature_id,
    feature_single_asset_vault, get_field_by_symbol, is_tec_claim, is_tes_success, jss, line,
    mpt_issuance_keylet_from_mptid, mptoken_keylet_from_mptid, nft_offer_keylet_from_key,
    nft_page_keylet, nft_page_max_keylet, nft_page_min_keylet, oracle_keylet, pay_channel_keylet_from_key,
    permissioned_domain_keylet_from_id, trans_token, vault_keylet_from_key,
    xchain_owned_claim_id_keylet_from_bridge,
};
use std::any::Any;
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

pub struct SubmitSource;

fn panic_payload_message(payload: Box<dyn Any + Send>) -> String {
    match payload.downcast::<String>() {
        Ok(message) => *message,
        Err(payload) => match payload.downcast::<&'static str>() {
            Ok(message) => (*message).to_string(),
            Err(_) => "invalid transaction blob".to_owned(),
        },
    }
}

fn submit_ledger(runtime: &app::AppNetworkOpsRuntime) -> Option<Arc<Ledger>> {
    let ledger_state = runtime.ledger_master_state();
    ledger_state
        .closed_ledger()
        .or_else(|| ledger_state.validated_ledger())
}

fn ledger_keylet_exists(ledger: &Ledger, keylet: Keylet) -> bool {
    matches!(ledger.exists_keylet(keylet), Ok(true))
}

fn ledger_read_keylet(ledger: &Ledger, keylet: Keylet) -> Option<protocol::STLedgerEntry> {
    ledger.read(keylet).ok().flatten()
}

fn ledger_account_exists(ledger: &Ledger, account: AccountID) -> bool {
    ledger_keylet_exists(ledger, account_keylet(account_to_uint160(account)))
}

fn account_to_uint160(account: AccountID) -> basics::base_uint::Uint160 {
    basics::base_uint::Uint160::from_slice(account.data())
        .expect("AccountID byte width should match Uint160")
}

fn account_keylet_for(account: AccountID) -> Keylet {
    account_keylet(account_to_uint160(account))
}

fn escrow_keylet_for(source: AccountID, sequence: u32) -> Keylet {
    escrow_keylet(account_to_uint160(source), sequence)
}

fn ledger_bridge_exists(ledger: &Ledger, bridge: &tx::XChainBridgeSpec) -> bool {
    ledger_keylet_exists(
        ledger,
        bridge_keylet_from_door_issue(
            account_to_uint160(bridge.locking_chain_door),
            bridge.locking_chain_issue,
        ),
    ) || ledger_keylet_exists(
        ledger,
        bridge_keylet_from_door_issue(
            account_to_uint160(bridge.issuing_chain_door),
            bridge.issuing_chain_issue,
        ),
    )
}

fn ledger_claim_id_exists(ledger: &Ledger, bridge: &tx::XChainBridgeSpec, claim_id: u64) -> bool {
    ledger_keylet_exists(
        ledger,
        xchain_owned_claim_id_keylet_from_bridge(
            account_to_uint160(bridge.locking_chain_door),
            bridge.locking_chain_issue,
            account_to_uint160(bridge.issuing_chain_door),
            bridge.issuing_chain_issue,
            claim_id,
        ),
    )
}

fn ledger_bridge_entry(ledger: &Ledger, bridge: &tx::XChainBridgeSpec) -> Option<STLedgerEntry> {
    ledger_read_keylet(
        ledger,
        bridge_keylet_from_door_issue(
            account_to_uint160(bridge.locking_chain_door),
            bridge.locking_chain_issue,
        ),
    )
    .or_else(|| {
        ledger_read_keylet(
            ledger,
            bridge_keylet_from_door_issue(
                account_to_uint160(bridge.issuing_chain_door),
                bridge.issuing_chain_issue,
            ),
        )
    })
}

fn tx_required_feature(tx_type: TxType) -> Option<basics::base_uint::Uint256> {
    Permission::get_instance().get_tx_feature(tx_type)
}

fn ledger_nft_present_for_owner(
    ledger: &Ledger,
    owner: AccountID,
    nft_id: basics::base_uint::Uint256,
) -> bool {
    // NFT pages are stored at the max key for the owner, not at the
    // token-derived key. Use succ to find the correct page.
    let first = nft_page_keylet(nft_page_min_keylet(account_to_uint160(owner)), nft_id);
    let last = nft_page_max_keylet(account_to_uint160(owner));
    let page_key = match ledger.succ(first.key, Some(last.key.next())) {
        Ok(Some(k)) => k,
        _ => last.key,
    };
    let page_kl = protocol::Keylet::new(protocol::LedgerEntryType::NFTokenPage, page_key);
    let Some(page) = ledger_read_keylet(ledger, page_kl) else {
        return false;
    };
    let nftokens_field = get_field_by_symbol("sfNFTokens");
    if !page.is_field_present(nftokens_field) {
        return false;
    }

    let nftoken_id_field = get_field_by_symbol("sfNFTokenID");
    page.get_field_array(nftokens_field).iter().any(|entry| {
        entry.is_field_present(nftoken_id_field) && entry.get_field_h256(nftoken_id_field) == nft_id
    })
}

fn vault_create_can_add_holding(ledger: &Ledger, account: AccountID, asset: Asset) -> Ter {
    match asset {
        Asset::Issue(issue) => {
            if issue.native()
                || issue.account == account
                || ledger_keylet_exists(ledger, line(account, issue.account, issue.currency))
            {
                Ter::TES_SUCCESS
            } else {
                Ter::TER_NO_RIPPLE
            }
        }
        Asset::MPTIssue(_) => Ter::TES_SUCCESS,
    }
}

pub(crate) fn submit_error_result(
    error: &'static str,
    error_exception: impl Into<String>,
) -> JsonValue {
    JsonValue::Object(BTreeMap::from([
        (jss::error.to_string(), JsonValue::String(error.to_owned())),
        (
            jss::error_exception.to_string(),
            JsonValue::String(error_exception.into()),
        ),
    ]))
}

pub(crate) fn parse_sttx_from_bytes(bytes: &[u8]) -> Result<STTx, String> {
    let mut iter = protocol::SerialIter::new(bytes);
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        STTx::from_serial_iter(&mut iter)
    }))
    .map_err(panic_payload_message)
}

pub(crate) fn parse_fail_hard(params: &JsonValue) -> bool {
    match params {
        JsonValue::Object(obj) => obj
            .get(jss::fail_hard)
            .and_then(|value| match value {
                JsonValue::Bool(value) => Some(*value),
                _ => None,
            })
            .unwrap_or(false),
        _ => false,
    }
}

fn parse_signer_list_set_entries(
    st_tx: &STTx,
) -> Result<Vec<tx::SignerListSetEntry<protocol::AccountID>>, Ter> {
    let signer_entries_field = get_field_by_symbol("sfSignerEntries");
    if !st_tx.is_field_present(signer_entries_field) {
        return Ok(Vec::new());
    }

    let signer_entry_field = get_field_by_symbol("sfSignerEntry");
    let account_field = get_field_by_symbol("sfAccount");
    let signer_weight_field = get_field_by_symbol("sfSignerWeight");

    let signer_entries = st_tx.get_field_array(signer_entries_field);
    let mut parsed_entries = Vec::with_capacity(signer_entries.len());
    for entry in signer_entries.iter() {
        let signer_entry = if entry.is_field_present(signer_entry_field) {
            entry.get_field_object(signer_entry_field)
        } else {
            entry.clone()
        };

        if !signer_entry.is_field_present(account_field)
            || !signer_entry.is_field_present(signer_weight_field)
        {
            return Err(Ter::TEM_MALFORMED);
        }

        parsed_entries.push(tx::SignerListSetEntry {
            account: signer_entry.get_account_id(account_field),
            weight: signer_entry.get_field_u16(signer_weight_field),
        });
    }

    Ok(parsed_entries)
}

fn parse_xchain_bridge_spec(st_tx: &STTx) -> Result<tx::XChainBridgeSpec, Ter> {
    let bridge_field = get_field_by_symbol("sfXChainBridge");
    if !st_tx.is_field_present(bridge_field) {
        return Err(Ter::TEM_MALFORMED);
    }

    let bridge = st_tx.get_field_xchain_bridge(bridge_field);
    let locking_issue = match bridge.locking_chain_issue() {
        protocol::Asset::Issue(issue) => issue,
        _ => return Err(Ter::TEM_MALFORMED),
    };
    let issuing_issue = match bridge.issuing_chain_issue() {
        protocol::Asset::Issue(issue) => issue,
        _ => return Err(Ter::TEM_MALFORMED),
    };

    Ok(tx::XChainBridgeSpec {
        locking_chain_door: bridge.locking_chain_door(),
        locking_chain_issue: locking_issue,
        issuing_chain_door: bridge.issuing_chain_door(),
        issuing_chain_issue: issuing_issue,
    })
}

fn xchain_add_claim_attestation_preflight(st_tx: &STTx) -> Ter {
    let bridge_field = get_field_by_symbol("sfXChainBridge");
    if !st_tx.is_field_present(bridge_field) {
        return Ter::TEM_MALFORMED;
    }

    let public_key_field = get_field_by_symbol("sfPublicKey");
    if protocol::PublicKey::from_slice(&st_tx.get_field_vl(public_key_field)).is_err() {
        return Ter::TEM_XCHAIN_BAD_PROOF;
    }

    let bridge = st_tx.get_field_xchain_bridge(bridge_field);
    let attestation = protocol::attestations::AttestationClaim::from_st_object(st_tx);
    if !attestation.valid_amounts() || !attestation.verify(&bridge) {
        return Ter::TEM_XCHAIN_BAD_PROOF;
    }
    if attestation.base.sending_amount.signum() <= 0 {
        return Ter::TEM_XCHAIN_BAD_PROOF;
    }
    let expected_issue = match bridge.issue(STXChainBridge::src_chain(
        attestation.base.was_locking_chain_send,
    )) {
        Asset::Issue(issue) => issue,
        Asset::MPTIssue(_) => return Ter::TEM_XCHAIN_BAD_PROOF,
    };
    if attestation.base.sending_amount.issue() != expected_issue {
        return Ter::TEM_XCHAIN_BAD_PROOF;
    }

    Ter::TES_SUCCESS
}

fn xchain_add_account_create_attestation_preflight(st_tx: &STTx) -> Ter {
    let bridge_field = get_field_by_symbol("sfXChainBridge");
    if !st_tx.is_field_present(bridge_field) {
        return Ter::TEM_MALFORMED;
    }

    let public_key_field = get_field_by_symbol("sfPublicKey");
    if protocol::PublicKey::from_slice(&st_tx.get_field_vl(public_key_field)).is_err() {
        return Ter::TEM_XCHAIN_BAD_PROOF;
    }

    let bridge = st_tx.get_field_xchain_bridge(bridge_field);
    let attestation = protocol::attestations::AttestationCreateAccount::from_st_object(st_tx);
    if !attestation.valid_amounts() || !attestation.verify(&bridge) {
        return Ter::TEM_XCHAIN_BAD_PROOF;
    }
    if attestation.base.sending_amount.signum() <= 0 {
        return Ter::TEM_XCHAIN_BAD_PROOF;
    }
    let expected_issue = match bridge.issue(STXChainBridge::src_chain(
        attestation.base.was_locking_chain_send,
    )) {
        Asset::Issue(issue) => issue,
        Asset::MPTIssue(_) => return Ter::TEM_XCHAIN_BAD_PROOF,
    };
    if attestation.base.sending_amount.issue() != expected_issue {
        return Ter::TEM_XCHAIN_BAD_PROOF;
    }

    Ter::TES_SUCCESS
}

#[cfg(test)]
fn submit_semantic_preflight(st_tx: &STTx, rules: &Rules) -> Ter {
    submit_semantic_preflight_with_ledger(st_tx, rules, None)
}

fn submit_semantic_preflight_with_ledger(
    st_tx: &STTx,
    rules: &Rules,
    ledger: Option<&Ledger>,
) -> Ter {
    let _rules_guard = CurrentTransactionRulesGuard::new(rules.clone());

    if st_tx.get_txn_type().to_u16() == 103 {
        return Ter::TEM_DISABLED;
    }

    if let Some(feature) = tx_required_feature(st_tx.get_txn_type())
        && !rules.enabled(&feature)
    {
        return Ter::TEM_DISABLED;
    }

    let fee_field = get_field_by_symbol("sfFee");
    if !st_tx.is_field_present(fee_field) {
        return Ter::TEM_MALFORMED;
    }
    let fee = st_tx.get_field_amount(fee_field);
    if !fee.native() || fee.negative() || !fee.is_legal_net() {
        return Ter::TEM_BAD_FEE;
    }

    match st_tx.get_txn_type() {
        TxType::PAYMENT => {
            let account_field = get_field_by_symbol("sfAccount");
            let amount_field = get_field_by_symbol("sfAmount");
            let destination_field = get_field_by_symbol("sfDestination");
            let deliver_min_field = get_field_by_symbol("sfDeliverMin");

            if !st_tx.is_field_present(amount_field) || !st_tx.is_field_present(destination_field) {
                return Ter::TEM_MALFORMED;
            }

            let amount = st_tx.get_field_amount(amount_field);
            if amount.negative() || !amount.is_legal_net() {
                return Ter::TEM_BAD_AMOUNT;
            }

            if let Some(deliver_min) = st_tx
                .is_field_present(deliver_min_field)
                .then(|| st_tx.get_field_amount(deliver_min_field))
            {
                if deliver_min.negative()
                    || !deliver_min.is_legal_net()
                    || deliver_min.asset() != amount.asset()
                {
                    return Ter::TEM_BAD_AMOUNT;
                }
            }

            if st_tx.get_account_id(destination_field).is_zero() {
                return Ter::TEM_DST_IS_SRC;
            }

            if st_tx.get_account_id(destination_field) == st_tx.get_account_id(account_field) {
                return Ter::TEM_REDUNDANT;
            }

            Ter::TES_SUCCESS
        }
        TxType::SIGNER_LIST_SET => {
            let signer_entries_field = get_field_by_symbol("sfSignerEntries");
            tx::run_signer_list_set_preflight(tx::SignerListSetPreflightFacts {
                quorum: st_tx.get_field_u32(get_field_by_symbol("sfSignerQuorum")),
                has_signer_entries: st_tx.is_field_present(signer_entries_field),
                signer_entries: parse_signer_list_set_entries(st_tx),
                account: st_tx.get_account_id(get_field_by_symbol("sfAccount")),
            })
        }
        TxType::ESCROW_CREATE => {
            let amount_field = get_field_by_symbol("sfAmount");
            let destination_field = get_field_by_symbol("sfDestination");
            if !st_tx.is_field_present(amount_field) || !st_tx.is_field_present(destination_field) {
                return Ter::TEM_MALFORMED;
            }

            let amount = st_tx.get_field_amount(amount_field);
            let cancel_after_field = get_field_by_symbol("sfCancelAfter");
            let finish_after_field = get_field_by_symbol("sfFinishAfter");
            let condition_field = get_field_by_symbol("sfCondition");
            let cancel_after = st_tx
                .is_field_present(cancel_after_field)
                .then(|| st_tx.get_field_u32(cancel_after_field));
            let finish_after = st_tx
                .is_field_present(finish_after_field)
                .then(|| st_tx.get_field_u32(finish_after_field));
            let condition_present = st_tx.is_field_present(condition_field);

            tx::run_escrow_create_preflight(tx::EscrowCreatePreflightFacts {
                amount_kind: if amount.native() {
                    tx::EscrowCreateAmountKind::Xrp
                } else if amount.holds_mpt_issue() {
                    tx::EscrowCreateAmountKind::Mpt
                } else {
                    tx::EscrowCreateAmountKind::Issue
                },
                amount_positive: amount.signum() > 0 && amount.is_legal_net(),
                feature_token_escrow_enabled: true,
                feature_mptokens_enabled: true,
                issue_has_bad_currency: amount.holds_issue()
                    && amount.issue().currency == protocol::bad_currency(),
                mpt_amount_within_limit: !amount.holds_mpt_issue()
                    || amount.mantissa() <= tx::ESCROW_CREATE_MAX_MPTOKEN_AMOUNT,
                cancel_after_present: cancel_after.is_some(),
                finish_after_present: finish_after.is_some(),
                cancel_after_strictly_after_finish_after: match (cancel_after, finish_after) {
                    (Some(cancel), Some(finish)) => cancel > finish,
                    _ => true,
                },
                condition_present,
                condition_valid: !condition_present
                    || !st_tx.get_field_vl(condition_field).is_empty(),
            })
        }
        TxType::ESCROW_FINISH => {
            if let Some(ledger) = ledger {
                let owner_field = get_field_by_symbol("sfOwner");
                let offer_sequence_field = get_field_by_symbol("sfOfferSequence");
                if !ledger_keylet_exists(
                    ledger,
                    escrow_keylet_for(
                        st_tx.get_account_id(owner_field),
                        st_tx.get_field_u32(offer_sequence_field),
                    ),
                ) {
                    return Ter::TEC_NO_TARGET;
                }
            }

            Ter::TES_SUCCESS
        }
        TxType::ESCROW_CANCEL => {
            let offer_sequence_field = get_field_by_symbol("sfOfferSequence");
            let preflight = tx::run_escrow_cancel_preflight();
            if preflight != Ter::TES_SUCCESS {
                return preflight;
            }

            if let Some(ledger) = ledger {
                let owner_field = get_field_by_symbol("sfOwner");
                let escrow_key = escrow_keylet_for(
                    st_tx.get_account_id(owner_field),
                    st_tx.get_field_u32(offer_sequence_field),
                );
                if ledger_read_keylet(ledger, escrow_key).is_none() {
                    return Ter::TEC_NO_TARGET;
                }
            }

            Ter::TES_SUCCESS
        }
        TxType::PAYCHAN_CREATE => {
            let account_field = get_field_by_symbol("sfAccount");
            let amount_field = get_field_by_symbol("sfAmount");
            let destination_field = get_field_by_symbol("sfDestination");
            let public_key_field = get_field_by_symbol("sfPublicKey");
            if !st_tx.is_field_present(amount_field)
                || !st_tx.is_field_present(destination_field)
                || !st_tx.is_field_present(public_key_field)
            {
                return Ter::TEM_MALFORMED;
            }

            let amount = st_tx.get_field_amount(amount_field);
            tx::run_payment_channel_create_preflight(tx::PaymentChannelCreatePreflightFacts {
                amount_is_xrp: amount.native(),
                amount_positive: amount.signum() > 0,
                tx_account_is_destination: st_tx.get_account_id(account_field)
                    == st_tx.get_account_id(destination_field),
                public_key_valid: protocol::PublicKey::from_slice(
                    &st_tx.get_field_vl(public_key_field),
                )
                .is_ok(),
            })
        }
        TxType::PAYCHAN_FUND => {
            let amount_field = get_field_by_symbol("sfAmount");
            if !st_tx.is_field_present(amount_field) {
                return Ter::TEM_MALFORMED;
            }

            let amount = st_tx.get_field_amount(amount_field);
            let preflight =
                tx::run_payment_channel_fund_preflight(amount.native(), amount.signum() > 0);
            if preflight != Ter::TES_SUCCESS {
                return preflight;
            }

            if let Some(ledger) = ledger {
                let pay_channel_field = get_field_by_symbol("sfChannel");
                if !st_tx.is_field_present(pay_channel_field) {
                    return Ter::TEM_MALFORMED;
                }
            }

            Ter::TES_SUCCESS
        }
        TxType::PAYCHAN_CLAIM => {
            let amount_field = get_field_by_symbol("sfAmount");
            let balance_field = get_field_by_symbol("sfBalance");
            let amount = st_tx.get_field_amount(amount_field);
            let balance = st_tx.get_field_amount(balance_field);
            let preflight = tx::run_payment_channel_claim_preflight(
                tx::PaymentChannelClaimPreflightFacts {
                    balance_present: st_tx.is_field_present(balance_field),
                    balance_is_xrp: balance.native(),
                    balance_positive: balance.signum() > 0,
                    amount_present: st_tx.is_field_present(amount_field),
                    amount_is_xrp: amount.native(),
                    amount_positive: amount.signum() > 0,
                    balance_exceeds_amount: st_tx.is_field_present(balance_field)
                        && st_tx.is_field_present(amount_field)
                        && balance > amount,
                    tx_flags: st_tx.get_flags(),
                    signature: None,
                },
                |_| true,
                || Ter::TES_SUCCESS,
            );
            if preflight != Ter::TES_SUCCESS {
                return preflight;
            }

            if let Some(ledger) = ledger {
                let pay_channel_field = get_field_by_symbol("sfChannel");
                if !st_tx.is_field_present(pay_channel_field) {
                    return Ter::TEM_MALFORMED;
                }
            }

            Ter::TES_SUCCESS
        }
        TxType::CHECK_CREATE => {
            let account_field = get_field_by_symbol("sfAccount");
            let destination_field = get_field_by_symbol("sfDestination");
            let send_max_field = get_field_by_symbol("sfSendMax");
            if !st_tx.is_field_present(destination_field) || !st_tx.is_field_present(send_max_field)
            {
                return Ter::TEM_MALFORMED;
            }

            let send_max = st_tx.get_field_amount(send_max_field);
            let expiration_field = get_field_by_symbol("sfExpiration");
            tx::run_check_create_preflight(tx::CheckCreatePreflightFacts {
                tx_account_is_destination: st_tx.get_account_id(account_field)
                    == st_tx.get_account_id(destination_field),
                send_max_is_legal: send_max.is_legal_net(),
                send_max_signum_positive: send_max.signum() > 0,
                send_max_currency_is_bad: send_max.holds_issue()
                    && send_max.issue().currency == protocol::bad_currency(),
                expiration: st_tx
                    .is_field_present(expiration_field)
                    .then(|| st_tx.get_field_u32(expiration_field)),
            })
        }
        TxType::CHECK_CASH => {
            let amount_field = get_field_by_symbol("sfAmount");
            let deliver_min_field = get_field_by_symbol("sfDeliverMin");
            let amount_present = st_tx.is_field_present(amount_field);
            let deliver_min_present = st_tx.is_field_present(deliver_min_field);
            let value = if amount_present {
                st_tx.get_field_amount(amount_field)
            } else {
                st_tx.get_field_amount(deliver_min_field)
            };
            let preflight = tx::run_check_cash_preflight(tx::CheckCashPreflightFacts {
                amount_present,
                deliver_min_present,
                value_is_legal: value.is_legal_net(),
                value_signum_positive: value.signum() > 0,
                value_currency_is_bad: value.holds_issue()
                    && value.issue().currency == protocol::bad_currency(),
            });
            if preflight != Ter::TES_SUCCESS {
                return preflight;
            }

            if let Some(ledger) = ledger {
                let check_id_field = get_field_by_symbol("sfCheckID");
                if !st_tx.is_field_present(check_id_field) {
                    return Ter::TEM_MALFORMED;
                }
                let check_id = st_tx.get_field_h256(check_id_field);
                if !ledger_keylet_exists(ledger, check_keylet_from_key(check_id)) {
                    return Ter::TEC_NO_ENTRY;
                }
            }

            Ter::TES_SUCCESS
        }
        TxType::CHECK_CANCEL => {
            let preflight = tx::run_check_cancel_preflight();
            if preflight != Ter::TES_SUCCESS {
                return preflight;
            }

            if let Some(ledger) = ledger {
                let check_id_field = get_field_by_symbol("sfCheckID");
                if !st_tx.is_field_present(check_id_field) {
                    return Ter::TEM_MALFORMED;
                }
                let check_id = st_tx.get_field_h256(check_id_field);
                if !ledger_keylet_exists(ledger, check_keylet_from_key(check_id)) {
                    return Ter::TEC_NO_ENTRY;
                }
            }

            Ter::TES_SUCCESS
        }
        TxType::DEPOSIT_PREAUTH => {
            let account_field = get_field_by_symbol("sfAccount");
            let authorize_field = get_field_by_symbol("sfAuthorize");
            let unauthorize_field = get_field_by_symbol("sfUnauthorize");
            let authorize_credentials_field = get_field_by_symbol("sfAuthorizeCredentials");
            let unauthorize_credentials_field = get_field_by_symbol("sfUnauthorizeCredentials");
            let account = st_tx.get_account_id(account_field);
            let authorize = st_tx
                .is_field_present(authorize_field)
                .then(|| st_tx.get_account_id(authorize_field));
            let unauthorize = st_tx
                .is_field_present(unauthorize_field)
                .then(|| st_tx.get_account_id(unauthorize_field));
            let preflight = tx::run_deposit_preauth_preflight(
                tx::DepositPreauthPreflightFacts {
                    account,
                    authorize,
                    unauthorize,
                    authorize_is_zero: authorize.is_some_and(|id| id.is_zero()),
                    unauthorize_is_zero: unauthorize.is_some_and(|id| id.is_zero()),
                    authorize_credentials_present: st_tx
                        .is_field_present(authorize_credentials_field),
                    unauthorize_credentials_present: st_tx
                        .is_field_present(unauthorize_credentials_field),
                },
                || Ter::TES_SUCCESS,
            );
            if preflight != Ter::TES_SUCCESS {
                return preflight;
            }

            if let Some(ledger) = ledger {
                let preclaim = tx::run_deposit_preauth_preclaim(tx::DepositPreauthPreclaimFacts {
                    authorize,
                    unauthorize,
                    authorize_target_exists: authorize
                        .is_some_and(|target| ledger_account_exists(ledger, target)),
                    authorize_preauth_exists: authorize.is_some_and(|target| {
                        ledger_keylet_exists(
                            ledger,
                            deposit_preauth_keylet(
                                account_to_uint160(account),
                                account_to_uint160(target),
                            ),
                        )
                    }),
                    unauthorize_preauth_exists: unauthorize.is_some_and(|target| {
                        ledger_keylet_exists(
                            ledger,
                            deposit_preauth_keylet(
                                account_to_uint160(account),
                                account_to_uint160(target),
                            ),
                        )
                    }),
                    authorize_credentials_present: false,
                    authorize_credentials: Vec::<
                        tx::DepositPreauthCredentialPreclaimFact<AccountID, Vec<u8>>,
                    >::new(),
                    authorize_credentials_preauth_exists: false,
                    unauthorize_credentials_present: false,
                    unauthorize_credentials_preauth_exists: false,
                });
                if preclaim != Ter::TES_SUCCESS {
                    return preclaim;
                }
            }

            Ter::TES_SUCCESS
        }
        TxType::ACCOUNT_DELETE => {
            let account_field = get_field_by_symbol("sfAccount");
            let destination_field = get_field_by_symbol("sfDestination");
            if !st_tx.is_field_present(destination_field) {
                return Ter::TEM_MALFORMED;
            }
            let account = st_tx.get_account_id(account_field);
            let destination = st_tx.get_account_id(destination_field);
            let preflight = tx::run_account_delete_preflight(
                tx::AccountDeletePreflightFacts {
                    account,
                    destination,
                },
                || Ter::TES_SUCCESS,
            );
            if preflight != Ter::TES_SUCCESS {
                return preflight;
            }

            if let Some(ledger) = ledger {
                let destination_sle = ledger_read_keylet(ledger, account_keylet_for(destination));
                let destination_flags_field = get_field_by_symbol("sfFlags");
                let destination_flags = destination_sle
                    .as_ref()
                    .map(|sle| {
                        if sle.is_field_present(destination_flags_field) {
                            sle.get_field_u32(destination_flags_field)
                        } else {
                            0
                        }
                    })
                    .unwrap_or(0);
                let front = tx::run_account_delete_preclaim_front(
                    tx::AccountDeletePreclaimFrontFacts {
                        destination_exists: destination_sle.is_some(),
                        destination_flags,
                        destination_tag_present: st_tx
                            .is_field_present(get_field_by_symbol("sfDestinationTag")),
                        credential_ids_present: st_tx
                            .is_field_present(get_field_by_symbol("sfCredentialIDs")),
                        source_account_exists: ledger_account_exists(ledger, account),
                    },
                    || Ter::TES_SUCCESS,
                    || {
                        ledger_keylet_exists(
                            ledger,
                            deposit_preauth_keylet(
                                account_to_uint160(destination),
                                account_to_uint160(account),
                            ),
                        )
                    },
                );
                if front != Ter::TES_SUCCESS {
                    return front;
                }

                // Only reject if account has non-deletable objects.
                // Tickets, credentials, signer lists, etc. are deletable.
                // If OwnerCount > 0, check if there are any NON-deletable items.
                // For the simple case: trust lines with balance, escrows with
                // pending funds, etc. are obligations.
                // 
                // NOTE: We don't do a full directory scan in the preflight
                // (that happens in the transactor). Here we only do a quick
                // reject for the common case of trust lines (RippleState).
                // The transactor will handle the full check.
                //
                // Skip this check — let the transactor handle it properly.
            }

            Ter::TES_SUCCESS
        }
        TxType::DID_SET => {
            let uri_field = get_field_by_symbol("sfURI");
            let did_document_field = get_field_by_symbol("sfDIDDocument");
            let data_field = get_field_by_symbol("sfData");
            tx::run_did_set_preflight(tx::DidSetPreflightFacts {
                uri_len: st_tx
                    .is_field_present(uri_field)
                    .then(|| st_tx.get_field_vl(uri_field).len()),
                did_document_len: st_tx
                    .is_field_present(did_document_field)
                    .then(|| st_tx.get_field_vl(did_document_field).len()),
                data_len: st_tx
                    .is_field_present(data_field)
                    .then(|| st_tx.get_field_vl(data_field).len()),
            })
        }
        TxType::DID_DELETE => {
            if let Some(ledger) = ledger {
                let account = st_tx.get_account_id(get_field_by_symbol("sfAccount"));
                if !ledger_keylet_exists(ledger, did_keylet(account_to_uint160(account))) {
                    return Ter::TEC_NO_ENTRY;
                }
            }

            Ter::TES_SUCCESS
        }
        TxType::CREDENTIAL_CREATE => {
            let subject_field = get_field_by_symbol("sfSubject");
            let credential_type_field = get_field_by_symbol("sfCredentialType");
            let uri_field = get_field_by_symbol("sfURI");
            let preflight =
                tx::run_credential_create_preflight(tx::CredentialCreatePreflightFacts {
                    subject_present: st_tx.is_field_present(subject_field),
                    uri_len: st_tx
                        .is_field_present(uri_field)
                        .then(|| st_tx.get_field_vl(uri_field).len()),
                    credential_type_len: st_tx.get_field_vl(credential_type_field).len(),
                });
            if preflight != Ter::TES_SUCCESS {
                return preflight;
            }

            if let Some(ledger) = ledger {
                let subject = st_tx.get_account_id(subject_field);
                let issuer = st_tx.get_account_id(get_field_by_symbol("sfAccount"));
                let subject_u160 = account_to_uint160(subject);
                let issuer_u160 = account_to_uint160(issuer);
                let credential_type = st_tx.get_field_vl(credential_type_field);
                let preclaim =
                    tx::run_credential_create_preclaim(tx::CredentialCreatePreclaimFacts {
                        subject_exists: ledger_account_exists(ledger, subject),
                        credential_exists: ledger_keylet_exists(
                            ledger,
                            credential_keylet(subject_u160, issuer_u160, &credential_type),
                        ),
                    });
                if preclaim != Ter::TES_SUCCESS {
                    return preclaim;
                }
            }

            Ter::TES_SUCCESS
        }
        TxType::CREDENTIAL_ACCEPT => {
            let issuer_field = get_field_by_symbol("sfIssuer");
            let credential_type_field = get_field_by_symbol("sfCredentialType");
            let preflight =
                tx::run_credential_accept_preflight(tx::CredentialAcceptPreflightFacts {
                    issuer_present: st_tx.is_field_present(issuer_field),
                    credential_type_len: st_tx.get_field_vl(credential_type_field).len(),
                });
            if preflight != Ter::TES_SUCCESS {
                return preflight;
            }

            if let Some(ledger) = ledger {
                let subject = st_tx.get_account_id(get_field_by_symbol("sfAccount"));
                let issuer = st_tx.get_account_id(issuer_field);
                let subject_u160 = account_to_uint160(subject);
                let issuer_u160 = account_to_uint160(issuer);
                let credential_type = st_tx.get_field_vl(credential_type_field);
                let credential = ledger_read_keylet(
                    ledger,
                    credential_keylet(subject_u160, issuer_u160, &credential_type),
                );
                let accepted_flag_field = get_field_by_symbol("sfFlags");
                let preclaim =
                    tx::run_credential_accept_preclaim(tx::CredentialAcceptPreclaimFacts {
                        issuer_exists: ledger_account_exists(ledger, issuer),
                        credential_exists: credential.is_some(),
                        credential_accepted: credential
                            .as_ref()
                            .map(|sle| {
                                sle.is_field_present(accepted_flag_field)
                                    && (sle.get_field_u32(accepted_flag_field)
                                        & tx::CREDENTIAL_ACCEPTED_FLAG)
                                        != 0
                            })
                            .unwrap_or(false),
                    });
                if preclaim != Ter::TES_SUCCESS {
                    return preclaim;
                }
            }

            Ter::TES_SUCCESS
        }
        TxType::CREDENTIAL_DELETE => {
            let subject_field = get_field_by_symbol("sfSubject");
            let issuer_field = get_field_by_symbol("sfIssuer");
            let credential_type_field = get_field_by_symbol("sfCredentialType");
            tx::run_credential_delete_preflight(tx::CredentialDeletePreflightFacts {
                subject: if !st_tx.is_field_present(subject_field) {
                    tx::CredentialOptionalAccountField::Missing
                } else if st_tx.get_account_id(subject_field).is_zero() {
                    tx::CredentialOptionalAccountField::Zero
                } else {
                    tx::CredentialOptionalAccountField::Present
                },
                issuer: if !st_tx.is_field_present(issuer_field) {
                    tx::CredentialOptionalAccountField::Missing
                } else if st_tx.get_account_id(issuer_field).is_zero() {
                    tx::CredentialOptionalAccountField::Zero
                } else {
                    tx::CredentialOptionalAccountField::Present
                },
                credential_type_len: st_tx.get_field_vl(credential_type_field).len(),
            })
        }
        TxType::NFTOKEN_CREATE_OFFER => {
            let amount_field = get_field_by_symbol("sfAmount");
            if !st_tx.is_field_present(amount_field) {
                return Ter::TEM_MALFORMED;
            }

            let amount = st_tx.get_field_amount(amount_field);
            if amount.negative() || !amount.is_legal_net() {
                return Ter::TEM_BAD_AMOUNT;
            }

            // (tokenOfferCreatePreflight): IOU zero is always bad
            if !amount.native() && amount.mantissa() == 0 {
                return Ter::TEM_BAD_AMOUNT;
            }

            let owner_field = get_field_by_symbol("sfOwner");
            let has_sell_flag = (st_tx.get_flags() & tx::TF_SELL_NFTOKEN) != 0;

            // Buy offers must have non-zero amount (any currency)
            if !has_sell_flag && amount.mantissa() == 0 {
                return Ter::TEM_BAD_AMOUNT;
            }

            if !has_sell_flag && !st_tx.is_field_present(owner_field) {
                return Ter::TEM_MALFORMED;
            }

            Ter::TES_SUCCESS
        }
        TxType::NFTOKEN_BURN => {
            let nft_id_field = get_field_by_symbol("sfNFTokenID");
            if !st_tx.is_field_present(nft_id_field) {
                return Ter::TEM_MALFORMED;
            }
            if let Some(ledger) = ledger {
                let account = st_tx.get_account_id(get_field_by_symbol("sfAccount"));
                let nft_id = st_tx.get_field_h256(nft_id_field);
                // Use sfOwner if present (issuer burning someone else's NFT)
                let owner = if st_tx.is_field_present(get_field_by_symbol("sfOwner")) {
                    st_tx.get_account_id(get_field_by_symbol("sfOwner"))
                } else {
                    account
                };
                if !ledger_nft_present_for_owner(ledger, owner, nft_id) {
                    return Ter::TEC_NO_ENTRY;
                }
                // If account != owner, check burnable flag (bit 0x0001 in NFTokenID)
                if account != owner {
                    let id_bytes = nft_id.data();
                    let nft_flags = ((id_bytes[0] as u16) << 8) | (id_bytes[1] as u16);
                    if (nft_flags & 0x0001) == 0 {
                        return Ter::TEC_NO_PERMISSION;
                    }
                }
            }

            Ter::TES_SUCCESS
        }
        TxType::NFTOKEN_ACCEPT_OFFER => {
            if let Some(ledger) = ledger {
                let sell_offer_field = get_field_by_symbol("sfNFTokenSellOffer");
                if st_tx.is_field_present(sell_offer_field)
                    && !ledger_keylet_exists(
                        ledger,
                        nft_offer_keylet_from_key(st_tx.get_field_h256(sell_offer_field)),
                    )
                {
                    return Ter::TEC_OBJECT_NOT_FOUND;
                }
                let buy_offer_field = get_field_by_symbol("sfNFTokenBuyOffer");
                if st_tx.is_field_present(buy_offer_field)
                    && !ledger_keylet_exists(
                        ledger,
                        nft_offer_keylet_from_key(st_tx.get_field_h256(buy_offer_field)),
                    )
                {
                    return Ter::TEC_OBJECT_NOT_FOUND;
                }
            }

            Ter::TES_SUCCESS
        }
        TxType::NFTOKEN_MODIFY => {
            let nft_id_field = get_field_by_symbol("sfNFTokenID");
            if !st_tx.is_field_present(nft_id_field) {
                return Ter::TEM_MALFORMED;
            }
            if let Some(ledger) = ledger {
                let account = st_tx.get_account_id(get_field_by_symbol("sfAccount"));
                let nft_id = st_tx.get_field_h256(nft_id_field);
                if !ledger_nft_present_for_owner(ledger, account, nft_id) {
                    return Ter::TEC_NO_ENTRY;
                }
            }

            Ter::TES_SUCCESS
        }
        TxType::AMM_CLAWBACK => {
            let asset_field = get_field_by_symbol("sfAsset");
            let asset2_field = get_field_by_symbol("sfAsset2");
            if !st_tx.is_field_present(asset_field) || !st_tx.is_field_present(asset2_field) {
                return Ter::TEM_MALFORMED;
            }

            if st_tx.get_field_issue(asset_field).asset()
                == st_tx.get_field_issue(asset2_field).asset()
            {
                return Ter::TEM_MALFORMED;
            }

            Ter::TES_SUCCESS
        }
        TxType::CLAWBACK => {
            let account_field = get_field_by_symbol("sfAccount");
            let amount_field = get_field_by_symbol("sfAmount");
            if !st_tx.is_field_present(amount_field) {
                return Ter::TEM_MALFORMED;
            }

            let amount = st_tx.get_field_amount(amount_field);
            let holder_field = get_field_by_symbol("sfHolder");
            let holder_present = st_tx.is_field_present(holder_field);
            tx::run_clawback_preflight(tx::ClawbackPreflightFacts {
                asset_kind: if amount.holds_mpt_issue() {
                    tx::ClawbackAssetKind::Mpt
                } else {
                    tx::ClawbackAssetKind::Issue
                },
                holder_field_present: holder_present,
                mptokens_v1_enabled: true,
                issuer_equals_holder: if amount.holds_mpt_issue() {
                    holder_present
                        && st_tx.get_account_id(holder_field) == st_tx.get_account_id(account_field)
                } else {
                    amount.holds_issue()
                        && amount.issue().account == st_tx.get_account_id(account_field)
                },
                amount_is_xrp: amount.native(),
                amount_positive: amount.signum() > 0,
                mpt_amount_exceeds_max: false,
            })
        }
        TxType::XCHAIN_CREATE_BRIDGE => {
            let signature_reward_field = get_field_by_symbol("sfSignatureReward");
            if !st_tx.is_field_present(signature_reward_field) {
                return Ter::TEM_MALFORMED;
            }

            let Ok(bridge) = parse_xchain_bridge_spec(st_tx) else {
                return Ter::TEM_MALFORMED;
            };
            let min_account_create_field = get_field_by_symbol("sfMinAccountCreateAmount");
            tx::run_xchain_create_bridge_preflight(tx::XChainCreateBridgePreflightFacts {
                account: st_tx.get_account_id(get_field_by_symbol("sfAccount")),
                reward: st_tx.get_field_amount(signature_reward_field),
                min_account_create: st_tx
                    .is_field_present(min_account_create_field)
                    .then(|| st_tx.get_field_amount(min_account_create_field)),
                bridge,
            })
        }
        TxType::XCHAIN_MODIFY_BRIDGE => {
            let Ok(bridge) = parse_xchain_bridge_spec(st_tx) else {
                return Ter::TEM_MALFORMED;
            };
            let signature_reward_field = get_field_by_symbol("sfSignatureReward");
            let min_account_create_field = get_field_by_symbol("sfMinAccountCreateAmount");
            tx::run_xchain_modify_bridge_preflight(tx::XChainModifyBridgePreflightFacts {
                account: st_tx.get_account_id(get_field_by_symbol("sfAccount")),
                reward: st_tx
                    .is_field_present(signature_reward_field)
                    .then(|| st_tx.get_field_amount(signature_reward_field)),
                min_account_create: st_tx
                    .is_field_present(min_account_create_field)
                    .then(|| st_tx.get_field_amount(min_account_create_field)),
                clear_account_create: st_tx
                    .is_flag(protocol::XCHAIN_MODIFY_BRIDGE_CLEAR_ACCOUNT_CREATE_AMOUNT_FLAG),
                bridge,
            })
        }
        TxType::XCHAIN_CREATE_CLAIM_ID => {
            let reward_field = get_field_by_symbol("sfSignatureReward");
            if !st_tx.is_field_present(reward_field) {
                return Ter::TEM_MALFORMED;
            }

            let reward = st_tx.get_field_amount(reward_field);
            if !reward.native() || reward.signum() <= 0 || !reward.is_legal_net() {
                return Ter::TEM_XCHAIN_BRIDGE_BAD_REWARD_AMOUNT;
            }

            if let Some(ledger) = ledger {
                let Ok(bridge) = parse_xchain_bridge_spec(st_tx) else {
                    return Ter::TEM_MALFORMED;
                };
                if !ledger_bridge_exists(ledger, &bridge) {
                    return Ter::TEC_NO_ENTRY;
                }
            }

            Ter::TES_SUCCESS
        }
        TxType::XCHAIN_COMMIT => {
            let amount_field = get_field_by_symbol("sfAmount");
            if !st_tx.is_field_present(amount_field) {
                return Ter::TEM_MALFORMED;
            }

            let amount = st_tx.get_field_amount(amount_field);
            if amount.signum() <= 0 || !amount.is_legal_net() {
                return Ter::TEM_BAD_AMOUNT;
            }
            let Ok(bridge) = parse_xchain_bridge_spec(st_tx) else {
                return Ter::TEM_MALFORMED;
            };
            if amount.asset() != Asset::Issue(bridge.locking_chain_issue)
                && amount.asset() != Asset::Issue(bridge.issuing_chain_issue)
            {
                return Ter::TEM_BAD_ISSUER;
            }

            if let Some(ledger) = ledger {
                let Some(sle_bridge) = ledger_bridge_entry(ledger, &bridge) else {
                    return Ter::TEC_NO_ENTRY;
                };
                let this_door = sle_bridge.get_account_id(get_field_by_symbol("sfAccount"));
                let account = st_tx.get_account_id(get_field_by_symbol("sfAccount"));
                if this_door == account {
                    return Ter::TEC_XCHAIN_SELF_COMMIT;
                }

                let expected_issue = if this_door == bridge.locking_chain_door {
                    bridge.locking_chain_issue
                } else if this_door == bridge.issuing_chain_door {
                    bridge.issuing_chain_issue
                } else {
                    return Ter::TEC_INTERNAL;
                };
                if amount.issue() != expected_issue {
                    return Ter::TEC_XCHAIN_BAD_TRANSFER_ISSUE;
                }
            }

            Ter::TES_SUCCESS
        }
        TxType::XCHAIN_CLAIM => {
            let amount_field = get_field_by_symbol("sfAmount");
            if !st_tx.is_field_present(amount_field) {
                return Ter::TEM_MALFORMED;
            }

            let amount = st_tx.get_field_amount(amount_field);
            if amount.signum() <= 0 || !amount.is_legal_net() {
                return Ter::TEM_BAD_AMOUNT;
            }

            if let Some(ledger) = ledger {
                let Ok(bridge) = parse_xchain_bridge_spec(st_tx) else {
                    return Ter::TEM_MALFORMED;
                };
                if !ledger_bridge_exists(ledger, &bridge) {
                    return Ter::TEC_NO_ENTRY;
                }
                let claim_id_field = get_field_by_symbol("sfXChainClaimID");
                if !st_tx.is_field_present(claim_id_field) {
                    return Ter::TEM_MALFORMED;
                }
                if !ledger_claim_id_exists(ledger, &bridge, st_tx.get_field_u64(claim_id_field)) {
                    return Ter::TEC_NO_ENTRY;
                }
            }

            Ter::TES_SUCCESS
        }
        TxType::XCHAIN_ACCOUNT_CREATE_COMMIT => {
            let amount_field = get_field_by_symbol("sfAmount");
            let reward_field = get_field_by_symbol("sfSignatureReward");
            if !st_tx.is_field_present(amount_field) || !st_tx.is_field_present(reward_field) {
                return Ter::TEM_MALFORMED;
            }

            let amount = st_tx.get_field_amount(amount_field);
            if amount.signum() <= 0 || !amount.is_legal_net() {
                return Ter::TEM_BAD_AMOUNT;
            }

            let reward = st_tx.get_field_amount(reward_field);
            if reward.signum() < 0 || !reward.is_legal_net() {
                return Ter::TEM_BAD_AMOUNT;
            }
            if reward.asset() != amount.asset() {
                return Ter::TEM_BAD_AMOUNT;
            }
            let Ok(bridge) = parse_xchain_bridge_spec(st_tx) else {
                return Ter::TEM_MALFORMED;
            };

            if let Some(ledger) = ledger {
                let Some(sle_bridge) = ledger_bridge_entry(ledger, &bridge) else {
                    return Ter::TEC_NO_ENTRY;
                };
                if reward != sle_bridge.get_field_amount(get_field_by_symbol("sfSignatureReward")) {
                    return Ter::TEC_XCHAIN_REWARD_MISMATCH;
                }

                let min_account_create_field = get_field_by_symbol("sfMinAccountCreateAmount");
                if !sle_bridge.is_field_present(min_account_create_field) {
                    return Ter::TEC_XCHAIN_CREATE_ACCOUNT_DISABLED;
                }
                let min_account_create = sle_bridge.get_field_amount(min_account_create_field);
                if amount < min_account_create {
                    return Ter::TEC_XCHAIN_INSUFF_CREATE_AMOUNT;
                }
                if min_account_create.asset() != amount.asset() {
                    return Ter::TEC_XCHAIN_BAD_TRANSFER_ISSUE;
                }

                let this_door = sle_bridge.get_account_id(get_field_by_symbol("sfAccount"));
                let account = st_tx.get_account_id(get_field_by_symbol("sfAccount"));
                if this_door == account {
                    return Ter::TEC_XCHAIN_SELF_COMMIT;
                }

                let expected_issue = if this_door == bridge.locking_chain_door {
                    bridge.locking_chain_issue
                } else if this_door == bridge.issuing_chain_door {
                    bridge.issuing_chain_issue
                } else {
                    return Ter::TEC_INTERNAL;
                };
                if amount.issue() != expected_issue {
                    return Ter::TEC_XCHAIN_BAD_TRANSFER_ISSUE;
                }

                let dst_issue = if this_door == bridge.locking_chain_door {
                    bridge.issuing_chain_issue
                } else if this_door == bridge.issuing_chain_door {
                    bridge.locking_chain_issue
                } else {
                    return Ter::TEC_INTERNAL;
                };
                if !dst_issue.native() {
                    return Ter::TEC_XCHAIN_CREATE_ACCOUNT_NONXRP_ISSUE;
                }
            }

            Ter::TES_SUCCESS
        }
        TxType::XCHAIN_ADD_CLAIM_ATTESTATION => xchain_add_claim_attestation_preflight(st_tx),
        TxType::XCHAIN_ADD_ACCOUNT_CREATE_ATTESTATION => {
            xchain_add_account_create_attestation_preflight(st_tx)
        }
        TxType::ORACLE_SET => {
            let price_data_series_field = get_field_by_symbol("sfPriceDataSeries");
            let provider_field = get_field_by_symbol("sfProvider");
            let uri_field = get_field_by_symbol("sfURI");
            let asset_class_field = get_field_by_symbol("sfAssetClass");
            if !st_tx.is_field_present(price_data_series_field) {
                return Ter::TEM_MALFORMED;
            }
            let preflight = tx::run_oracle_set_preflight(tx::OracleSetPreflightFacts {
                price_data_series_len: st_tx.get_field_array(price_data_series_field).len(),
                provider_len: st_tx
                    .is_field_present(provider_field)
                    .then(|| st_tx.get_field_vl(provider_field).len()),
                uri_len: st_tx
                    .is_field_present(uri_field)
                    .then(|| st_tx.get_field_vl(uri_field).len()),
                asset_class_len: st_tx
                    .is_field_present(asset_class_field)
                    .then(|| st_tx.get_field_vl(asset_class_field).len()),
            });
            if preflight != Ter::TES_SUCCESS {
                return preflight;
            }

            if let Some(ledger) = ledger {
                let account = st_tx.get_account_id(get_field_by_symbol("sfAccount"));
                let last_update_time_field = get_field_by_symbol("sfLastUpdateTime");
                let front = tx::run_oracle_set_preclaim_front(tx::OracleSetPreclaimFrontFacts {
                    account_exists: ledger_account_exists(ledger, account),
                    close_time_secs: u64::from(ledger.header().close_time),
                    last_update_time_secs: if st_tx.is_field_present(last_update_time_field) {
                        u64::from(st_tx.get_field_u32(last_update_time_field))
                    } else {
                        Default::default()
                    },
                });
                if front != Ter::TES_SUCCESS {
                    return front;
                }
            }

            Ter::TES_SUCCESS
        }
        TxType::ORACLE_DELETE => {
            if let Some(ledger) = ledger {
                let account = st_tx.get_account_id(get_field_by_symbol("sfAccount"));
                let document_id_field = get_field_by_symbol("sfOracleDocumentID");
                if !st_tx.is_field_present(document_id_field) {
                    return Ter::TEM_MALFORMED;
                }
                let oracle = ledger_read_keylet(
                    ledger,
                    oracle_keylet(
                        account_to_uint160(account),
                        st_tx.get_field_u32(document_id_field),
                    ),
                );
                let owner_field = get_field_by_symbol("sfOwner");
                let preclaim = tx::run_oracle_delete_preclaim(tx::OracleDeletePreclaimFacts {
                    account_exists: ledger_account_exists(ledger, account),
                    oracle_exists: oracle.is_some(),
                    tx_account_matches_owner: oracle
                        .as_ref()
                        .map(|sle| {
                            sle.is_field_present(owner_field)
                                && sle.get_account_id(owner_field) == account
                        })
                        .unwrap_or(false),
                });
                if preclaim != Ter::TES_SUCCESS {
                    return preclaim;
                }
            }

            Ter::TES_SUCCESS
        }
        TxType::MPTOKEN_ISSUANCE_DESTROY => {
            if let Some(ledger) = ledger {
                let issuance_id_field = get_field_by_symbol("sfMPTokenIssuanceID");
                if !st_tx.is_field_present(issuance_id_field) {
                    return Ter::TEM_MALFORMED;
                }
                let issuance = ledger_read_keylet(
                    ledger,
                    mpt_issuance_keylet_from_mptid(st_tx.get_field_h192(issuance_id_field)),
                );
                let issuer_field = get_field_by_symbol("sfIssuer");
                let outstanding_amount_field = get_field_by_symbol("sfOutstandingAmount");
                let locked_amount_field = get_field_by_symbol("sfLockedAmount");
                let account = st_tx.get_account_id(get_field_by_symbol("sfAccount"));
                let preclaim = tx::run_mp_token_issuance_destroy_preclaim(
                    tx::MPTokenIssuanceDestroyPreclaimFacts {
                        issuance_exists: issuance.is_some(),
                        issuer_matches: issuance
                            .as_ref()
                            .map(|sle| {
                                sle.is_field_present(issuer_field)
                                    && sle.get_account_id(issuer_field) == account
                            })
                            .unwrap_or(false),
                        outstanding_amount_is_zero: issuance
                            .as_ref()
                            .map(|sle| {
                                !sle.is_field_present(outstanding_amount_field)
                                    || sle.get_field_amount(outstanding_amount_field).signum() == 0
                            })
                            .unwrap_or(true),
                        locked_amount_is_zero: issuance
                            .as_ref()
                            .map(|sle| {
                                !sle.is_field_present(locked_amount_field)
                                    || sle.get_field_amount(locked_amount_field).signum() == 0
                            })
                            .unwrap_or(true),
                    },
                );
                if preclaim != Ter::TES_SUCCESS {
                    return preclaim;
                }
            }

            Ter::TES_SUCCESS
        }
        TxType::MPTOKEN_AUTHORIZE => {
            let holder_field = get_field_by_symbol("sfHolder");
            let account = st_tx.get_account_id(get_field_by_symbol("sfAccount"));
            let holder = st_tx
                .is_field_present(holder_field)
                .then(|| st_tx.get_account_id(holder_field));
            let preflight =
                tx::run_mp_token_authorize_preflight(tx::MPTokenAuthorizePreflightFacts {
                    account_equals_holder: holder.is_some_and(|h| h == account),
                });
            if preflight != Ter::TES_SUCCESS {
                return preflight;
            }

            if let Some(ledger) = ledger {
                let issuance_id_field = get_field_by_symbol("sfMPTokenIssuanceID");
                if !st_tx.is_field_present(issuance_id_field) {
                    return Ter::TEM_MALFORMED;
                }
                let issuance_id = st_tx.get_field_h192(issuance_id_field);
                let issuance =
                    ledger_read_keylet(ledger, mpt_issuance_keylet_from_mptid(issuance_id));
                let issuer_field = get_field_by_symbol("sfIssuer");
                let flags_field = get_field_by_symbol("sfFlags");
                let holder_token_exists = holder.is_some_and(|h| {
                    ledger_keylet_exists(
                        ledger,
                        mptoken_keylet_from_mptid(issuance_id, account_to_uint160(h)),
                    )
                });
                let preclaim =
                    tx::run_mp_token_authorize_preclaim(tx::MPTokenAuthorizePreclaimFacts {
                        holder_present: holder.is_some(),
                        account_token_exists: ledger_keylet_exists(
                            ledger,
                            mptoken_keylet_from_mptid(issuance_id, account_to_uint160(account)),
                        ),
                        tx_flags: st_tx.get_flags(),
                        token_balance_is_zero: true,
                        token_locked_amount_is_zero: true,
                        issuance_exists: issuance.is_some(),
                        single_asset_vault_enabled: rules.enabled(&feature_single_asset_vault()),
                        token_locked: false,
                        account_is_issuer: issuance
                            .as_ref()
                            .map(|sle| {
                                sle.is_field_present(issuer_field)
                                    && sle.get_account_id(issuer_field) == account
                            })
                            .unwrap_or(false),
                        holder_account_exists: holder
                            .is_some_and(|h| ledger_account_exists(ledger, h)),
                        issuance_requires_auth: issuance
                            .as_ref()
                            .map(|sle| {
                                sle.is_field_present(flags_field)
                                    && (sle.get_field_u32(flags_field)
                                        & protocol::lsfMPTRequireAuth)
                                        != 0
                            })
                            .unwrap_or(false),
                        holder_token_exists,
                        holder_is_pseudo_account: holder.is_some_and(|h| h.is_zero()),
                    });
                if preclaim != Ter::TES_SUCCESS {
                    return preclaim;
                }
            }

            Ter::TES_SUCCESS
        }
        TxType::PERMISSIONED_DOMAIN_DELETE => {
            let domain_id_field = get_field_by_symbol("sfDomainID");
            if !st_tx.is_field_present(domain_id_field) {
                return Ter::TEM_MALFORMED;
            }
            let domain_id = st_tx.get_field_h256(domain_id_field);
            let preflight = tx::run_permissioned_domain_delete_preflight(domain_id.is_zero());
            if preflight != Ter::TES_SUCCESS {
                return preflight;
            }

            if let Some(ledger) = ledger {
                let domain =
                    ledger_read_keylet(ledger, permissioned_domain_keylet_from_id(domain_id));
                let owner_field = get_field_by_symbol("sfOwner");
                let account = st_tx.get_account_id(get_field_by_symbol("sfAccount"));
                let preclaim = tx::run_permissioned_domain_delete_preclaim(
                    tx::PermissionedDomainDeletePreclaimFacts {
                        domain_exists: domain.is_some(),
                        tx_account_matches_owner: domain
                            .as_ref()
                            .map(|sle| {
                                sle.is_field_present(owner_field)
                                    && sle.get_account_id(owner_field) == account
                            })
                            .unwrap_or(false),
                    },
                );
                if preclaim != Ter::TES_SUCCESS {
                    return preclaim;
                }
            }

            Ter::TES_SUCCESS
        }
        TxType::MPTOKEN_ISSUANCE_SET => {
            let holder_field = get_field_by_symbol("sfHolder");
            let domain_id_field = get_field_by_symbol("sfDomainID");
            let metadata_field = get_field_by_symbol("sfMPTokenMetadata");
            let transfer_fee_field = get_field_by_symbol("sfTransferFee");
            let tx_flags = st_tx.get_flags();
            let mutable_flags = tx_flags & protocol::tmfMPTokenIssuanceSetMutableMask;
            tx::run_mp_token_issuance_set_preflight(tx::MPTokenIssuanceSetPreflightFacts {
                dynamic_mpt_enabled: true,
                single_asset_vault_enabled: rules.enabled(&feature_single_asset_vault()),
                domain_id_present: st_tx.is_field_present(domain_id_field),
                holder_present: st_tx.is_field_present(holder_field),
                account_equals_holder: st_tx.is_field_present(holder_field)
                    && st_tx.get_account_id(holder_field)
                        == st_tx.get_account_id(get_field_by_symbol("sfAccount")),
                tx_flags,
                mutable_flags: (mutable_flags != 0).then_some(mutable_flags),
                metadata_len: st_tx
                    .is_field_present(metadata_field)
                    .then(|| st_tx.get_field_vl(metadata_field).len()),
                transfer_fee: st_tx
                    .is_field_present(transfer_fee_field)
                    .then(|| st_tx.get_field_u16(transfer_fee_field)),
            })
        }
        TxType::VAULT_CREATE => {
            let asset_field = get_field_by_symbol("sfAsset");
            let data_field = get_field_by_symbol("sfData");
            let withdrawal_policy_field = get_field_by_symbol("sfWithdrawalPolicy");
            let domain_id_field = get_field_by_symbol("sfDomainID");
            let assets_maximum_field = get_field_by_symbol("sfAssetsMaximum");
            let mptoken_metadata_field = get_field_by_symbol("sfMPTokenMetadata");
            let scale_field = get_field_by_symbol("sfScale");
            if !st_tx.is_field_present(asset_field) {
                return Ter::TEM_MALFORMED;
            }
            let asset = st_tx.get_field_issue(asset_field).asset();
            let preflight = tx::run_vault_create_preflight(tx::VaultCreatePreflightFacts {
                data_len: st_tx
                    .is_field_present(data_field)
                    .then(|| st_tx.get_field_vl(data_field).len()),
                withdrawal_policy: st_tx
                    .is_field_present(withdrawal_policy_field)
                    .then(|| st_tx.get_field_u8(withdrawal_policy_field)),
                domain_id_present: st_tx.is_field_present(domain_id_field),
                domain_id_is_zero: st_tx.is_field_present(domain_id_field)
                    && st_tx.get_field_h256(domain_id_field).is_zero(),
                is_private: st_tx.is_flag(protocol::tfVaultPrivate),
                assets_maximum_is_negative: st_tx.is_field_present(assets_maximum_field)
                    && st_tx.get_field_amount(assets_maximum_field).negative(),
                mptoken_metadata_len: st_tx
                    .is_field_present(mptoken_metadata_field)
                    .then(|| st_tx.get_field_vl(mptoken_metadata_field).len()),
                scale: st_tx
                    .is_field_present(scale_field)
                    .then(|| st_tx.get_field_u8(scale_field)),
                asset_is_mpt: matches!(asset, Asset::MPTIssue(_)),
                asset_is_native: asset.native(),
            });
            if preflight != Ter::TES_SUCCESS {
                return preflight;
            }

            if let Some(ledger) = ledger {
                let account = st_tx.get_account_id(get_field_by_symbol("sfAccount"));
                let domain_id = st_tx
                    .is_field_present(domain_id_field)
                    .then(|| st_tx.get_field_h256(domain_id_field));
                let preclaim = tx::run_vault_create_preclaim(
                    tx::VaultCreatePreclaimFacts {
                        asset_is_native: asset.native(),
                        asset_is_issue: matches!(asset, Asset::Issue(_)),
                        domain_id_present: domain_id.is_some(),
                    },
                    || vault_create_can_add_holding(ledger, account, asset),
                    || false,
                    || false,
                    || {
                        domain_id.is_some_and(|id| {
                            ledger_keylet_exists(ledger, permissioned_domain_keylet_from_id(id))
                        })
                    },
                    || false,
                );
                if preclaim != Ter::TES_SUCCESS {
                    return preclaim;
                }
            }

            Ter::TES_SUCCESS
        }
        TxType::VAULT_DEPOSIT => {
            let vault_id_field = get_field_by_symbol("sfVaultID");
            let amount_field = get_field_by_symbol("sfAmount");
            if !st_tx.is_field_present(vault_id_field) || !st_tx.is_field_present(amount_field) {
                return Ter::TEM_MALFORMED;
            }

            let vault_id = st_tx.get_field_h256(vault_id_field);
            let amount = st_tx.get_field_amount(amount_field);
            let preflight = tx::run_vault_deposit_preflight(tx::VaultDepositPreflightFacts {
                vault_id_is_zero: vault_id.is_zero(),
                amount_is_positive: amount.signum() > 0,
            });
            if preflight != Ter::TES_SUCCESS {
                return preflight;
            }

            if let Some(ledger) = ledger
                && !ledger_keylet_exists(ledger, vault_keylet_from_key(vault_id))
            {
                return Ter::TEC_NO_ENTRY;
            }

            Ter::TES_SUCCESS
        }
        TxType::VAULT_DELETE => {
            let vault_id_field = get_field_by_symbol("sfVaultID");
            if !st_tx.is_field_present(vault_id_field) {
                return Ter::TEM_MALFORMED;
            }
            let vault_id = st_tx.get_field_h256(vault_id_field);
            let preflight = tx::run_vault_delete_preflight(tx::VaultDeletePreflightFacts {
                vault_id_is_zero: vault_id.is_zero(),
                has_memo_data: false,
                lending_protocol_v1_1_enabled: false,
                memo_data_length_valid: true,
            });
            if preflight != Ter::TES_SUCCESS {
                return preflight;
            }

            if let Some(ledger) = ledger
                && !ledger_keylet_exists(ledger, vault_keylet_from_key(vault_id))
            {
                return Ter::TEC_NO_ENTRY;
            }

            Ter::TES_SUCCESS
        }
        TxType::VAULT_CLAWBACK => {
            let vault_id_field = get_field_by_symbol("sfVaultID");
            let amount_field = get_field_by_symbol("sfAmount");
            if !st_tx.is_field_present(vault_id_field) {
                return Ter::TEM_MALFORMED;
            }
            let amount_present = st_tx.is_field_present(amount_field);
            let amount = st_tx.get_field_amount(amount_field);
            let preflight = tx::run_vault_clawback_preflight(tx::VaultClawbackPreflightFacts {
                vault_id_is_zero: st_tx.get_field_h256(vault_id_field).is_zero(),
                amount_present,
                amount_is_negative: amount_present && amount.negative(),
                amount_asset_is_xrp: amount_present && amount.native(),
            });
            if preflight != Ter::TES_SUCCESS {
                return preflight;
            }

            if let Some(ledger) = ledger
                && !ledger_keylet_exists(
                    ledger,
                    vault_keylet_from_key(st_tx.get_field_h256(vault_id_field)),
                )
            {
                return Ter::TEC_NO_ENTRY;
            }

            Ter::TES_SUCCESS
        }
        TxType::VAULT_SET => {
            let vault_id_field = get_field_by_symbol("sfVaultID");
            if !st_tx.is_field_present(vault_id_field) {
                return Ter::TEM_MALFORMED;
            }

            let data_field = get_field_by_symbol("sfData");
            let assets_maximum_field = get_field_by_symbol("sfAssetsMaximum");
            let domain_id_field = get_field_by_symbol("sfDomainID");
            let vault_id = st_tx.get_field_h256(vault_id_field);
            tx::run_vault_set_preflight(tx::VaultSetPreflightFacts {
                vault_id_is_zero: vault_id.is_zero(),
                data_len: st_tx
                    .is_field_present(data_field)
                    .then(|| st_tx.get_field_vl(data_field).len()),
                assets_maximum_is_negative: st_tx.is_field_present(assets_maximum_field)
                    && st_tx.get_field_amount(assets_maximum_field).negative(),
                domain_id_present: st_tx.is_field_present(domain_id_field),
                assets_maximum_present: st_tx.is_field_present(assets_maximum_field),
                data_present: st_tx.is_field_present(data_field),
            })
        }
        TxType::VAULT_WITHDRAW => {
            let vault_id_field = get_field_by_symbol("sfVaultID");
            let amount_field = get_field_by_symbol("sfAmount");
            let destination_field = get_field_by_symbol("sfDestination");
            if !st_tx.is_field_present(vault_id_field) || !st_tx.is_field_present(amount_field) {
                return Ter::TEM_MALFORMED;
            }

            let vault_id = st_tx.get_field_h256(vault_id_field);
            let amount = st_tx.get_field_amount(amount_field);
            let preflight = tx::run_vault_withdraw_preflight(tx::VaultWithdrawPreflightFacts {
                vault_id_is_zero: vault_id.is_zero(),
                amount_is_positive: amount.signum() > 0,
                destination_present: st_tx.is_field_present(destination_field),
                destination_is_zero: st_tx.get_account_id(destination_field).is_zero(),
            });
            if preflight != Ter::TES_SUCCESS {
                return preflight;
            }

            if let Some(ledger) = ledger
                && !ledger_keylet_exists(ledger, vault_keylet_from_key(vault_id))
            {
                return Ter::TEC_NO_ENTRY;
            }

            Ter::TES_SUCCESS
        }
        TxType::OFFER_CREATE => {
            let taker_pays_field = get_field_by_symbol("sfTakerPays");
            let taker_gets_field = get_field_by_symbol("sfTakerGets");

            if !st_tx.is_field_present(taker_pays_field)
                || !st_tx.is_field_present(taker_gets_field)
            {
                return Ter::TEM_MALFORMED;
            }

            let taker_pays = st_tx.get_field_amount(taker_pays_field);
            let taker_gets = st_tx.get_field_amount(taker_gets_field);
            if taker_pays.asset() == taker_gets.asset() {
                if taker_pays.native() && taker_gets.native() {
                    return Ter::TEM_BAD_OFFER;
                }

                if !taker_pays.is_legal_net() || !taker_gets.is_legal_net() {
                    return Ter::TEM_BAD_AMOUNT;
                }

                if taker_pays.negative() || taker_gets.negative() {
                    return Ter::TEM_BAD_AMOUNT;
                }

                return Ter::TEM_REDUNDANT;
            }

            if !taker_pays.is_legal_net() || !taker_gets.is_legal_net() {
                return Ter::TEM_BAD_AMOUNT;
            }

            if taker_pays.negative() || taker_gets.negative() {
                return Ter::TEM_BAD_AMOUNT;
            }

            Ter::TES_SUCCESS
        }
        TxType::BATCH => {
            let mode_flags = protocol::BatchTransactionFlags::from_bits(st_tx.get_flags());
            if mode_flags.bits().count_ones() != 1 {
                return Ter::TEM_INVALID_FLAG;
            }

            Ter::TES_SUCCESS
        }
        _ => Ter::TES_SUCCESS,
    }
}

fn build_submit_result(transaction: &SharedTransaction, tx_blob_hex: &str) -> JsonValue {
    let mut ret = BTreeMap::new();
    let (tx_json, result, submit_result, current_ledger_state) = {
        let tx = transaction
            .lock()
            .expect("transaction mutex must not be poisoned");
        (
            tx.get_json(JsonOptions::NONE, false),
            tx.get_result(),
            tx.get_submit_result(),
            tx.get_current_ledger_state(),
        )
    };

    ret.insert(jss::tx_json.to_string(), tx_json);
    ret.insert(
        jss::tx_blob.to_string(),
        JsonValue::String(tx_blob_hex.to_owned()),
    );

    if result != Ter::TEM_UNCERTAIN {
        ret.insert(
            jss::engine_result.to_string(),
            JsonValue::String(trans_token(result).to_owned()),
        );
        ret.insert(
            jss::engine_result_code.to_string(),
            JsonValue::Signed(i64::from(result.to_int())),
        );
        ret.insert(
            jss::engine_result_message.to_string(),
            JsonValue::String(trans_human(result).to_owned()),
        );
        ret.insert(
            jss::accepted.to_string(),
            JsonValue::Bool(submit_result.any()),
        );
        ret.insert(
            jss::applied.to_string(),
            JsonValue::Bool(submit_result.applied),
        );
        ret.insert(
            jss::broadcast.to_string(),
            JsonValue::Bool(submit_result.broadcast),
        );
        ret.insert(
            jss::queued.to_string(),
            JsonValue::Bool(submit_result.queued),
        );
        ret.insert(jss::kept.to_string(), JsonValue::Bool(submit_result.kept));

        if let Some(current_ledger_state) = current_ledger_state {
            ret.insert(
                jss::account_sequence_next.to_string(),
                JsonValue::Unsigned(u64::from(current_ledger_state.account_seq_next)),
            );
            ret.insert(
                jss::account_sequence_available.to_string(),
                JsonValue::Unsigned(u64::from(current_ledger_state.account_seq_avail)),
            );
            ret.insert(
                jss::open_ledger_cost.to_string(),
                JsonValue::String(current_ledger_state.min_fee_required.drops().to_string()),
            );
            ret.insert(
                jss::validated_ledger_index.to_string(),
                JsonValue::Unsigned(u64::from(current_ledger_state.validated_ledger)),
            );
        }
    }

    JsonValue::Object(ret)
}

pub(crate) fn submit_sttx<Env, Runtime: RpcRuntime>(
    ctx: &RpcRequestContext<'_, Env, Runtime>,
    st_tx: STTx,
    tx_blob_hex: &str,
    fail_hard: bool,
) -> JsonValue {
    let Some(runtime) = ctx.runtime.network_ops_runtime() else {
        return submit_error_result("internalSubmit", "network ops runtime unavailable");
    };

    let semantic_ledger = submit_ledger(&runtime);
    let rules = semantic_ledger
        .as_ref()
        .map(|ledger| ledger.rules().clone())
        .unwrap_or_default();
    if let Err(failure) =
        run_submit_validity_gate(true, || {}, || runtime.check_validity(&st_tx, &rules))
    {
        return submit_error_result(failure.error, failure.error_exception);
    }

    let st_tx = Arc::new(st_tx);
    let mut transaction: SharedTransaction =
        Arc::new(Mutex::new(Transaction::new(Arc::clone(&st_tx))));
    let semantic_preflight =
        submit_semantic_preflight_with_ledger(st_tx.as_ref(), &rules, semantic_ledger.as_deref());
    if !is_tes_success(semantic_preflight) {
        transaction
            .lock()
            .expect("transaction mutex must not be poisoned")
            .set_result(semantic_preflight);
        return build_submit_result(&transaction, tx_blob_hex);
    }

    let _ = runtime.process_transaction(
        &mut transaction,
        ctx.unlimited,
        true,
        fail_hard,
        || false,
        || {
            if let Some(app) = ctx.runtime.app() {
                let _ = app.apply_network_ops_pending_to_open_ledger();
            }
        },
    );

    let result = transaction
        .lock()
        .expect("transaction mutex must not be poisoned")
        .get_result();
    if result == Ter::TEM_UNCERTAIN {
        return submit_error_result(
            "internalSubmit",
            "submit did not reach a concrete open-ledger apply result",
        );
    }

    tracing::info!(target: "rpc", tx_hash = %st_tx.get_transaction_id(), "Transaction submitted via RPC");

    // In standalone mode, submit immediately closes the ledger
    // so that subsequent RPC calls (account_info, etc.) see the state changes.
    if ctx.runtime.standalone() {
        let _ = ctx.runtime.ledger_accept();
    }

    build_submit_result(&transaction, tx_blob_hex)
}

pub fn do_submit<Runtime: RpcRuntime>(
    ctx: &RpcRequestContext<'_, SubmitSource, Runtime>,
) -> Result<JsonValue, Status> {
    tracing::debug!(target: "rpc", method = "submit", "RPC request received");
    let tx_blob = match ctx.params {
        JsonValue::Object(obj) => obj.get(jss::tx_blob).and_then(JsonValue::as_str),
        _ => None,
    };

    // If tx_blob is not present, fall back to deprecated sign-and-submit mode
    // (signing with secret + tx_json for legacy compatibility).
    if tx_blob.is_none() {
        let has_secret = matches!(ctx.params, JsonValue::Object(obj) if obj.contains_key(jss::secret) || obj.contains_key(jss::key_type));
        if has_secret {
            return submit_with_sign(ctx);
        }
    }

    let tx_blob_hex = tx_blob.ok_or_else(|| Status::new(RpcErrorCode::InvalidParams))?;
    let bytes = hex::decode(tx_blob_hex).map_err(|_| Status::new(RpcErrorCode::InvalidParams))?;
    let st_tx = match parse_sttx_from_bytes(&bytes) {
        Ok(st_tx) => st_tx,
        Err(error_exception) => {
            return Ok(submit_error_result(
                INVALID_TRANSACTION_ERROR,
                error_exception,
            ));
        }
    };

    Ok(submit_sttx(
        ctx,
        st_tx,
        tx_blob_hex,
        parse_fail_hard(ctx.params),
    ))
}

/// Deprecated sign-and-submit mode: signs the transaction server-side using the
/// provided secret/seed, then submits it. Legacy behavior
/// for backward compatibility. Users should migrate to client-side signing.
fn submit_with_sign<Runtime: RpcRuntime>(
    ctx: &RpcRequestContext<'_, SubmitSource, Runtime>,
) -> Result<JsonValue, Status> {
    use crate::commands::rpc_helpers::transaction_sign;

    // Reinterpret the context with a temporary SignSource for the sign call
    let sign_ctx = RpcRequestContext {
        params: ctx.params,
        env: &crate::signing::sign::SignSource,
        runtime: ctx.runtime,
        role: ctx.role,
        api_version: ctx.api_version,
        headers: ctx.headers.clone(),
        request_headers: ctx.request_headers.clone(),
        unlimited: ctx.unlimited,
        remote_ip: ctx.remote_ip,
        load_type: ctx.load_type,
    };

    let sign_result = transaction_sign(&sign_ctx)?;

    // Extract tx_blob from sign result
    let tx_blob_hex = match &sign_result {
        JsonValue::Object(obj) => obj
            .get(jss::tx_blob)
            .and_then(JsonValue::as_str)
            .ok_or_else(|| Status::new(RpcErrorCode::Internal))?,
        _ => return Err(Status::new(RpcErrorCode::Internal)),
    };

    let bytes = hex::decode(tx_blob_hex).map_err(|_| Status::new(RpcErrorCode::InvalidParams))?;
    let st_tx = parse_sttx_from_bytes(&bytes)
        .map_err(|e| Status::with_message(RpcErrorCode::Internal, e))?;

    let fail_hard = parse_fail_hard(ctx.params);
    let mut result = submit_sttx(ctx, st_tx, tx_blob_hex, fail_hard);

    // Mark as deprecated — users should migrate to client-side signing
    if let JsonValue::Object(ref mut obj) = result {
        obj.insert(
            jss::deprecated.to_string(),
            JsonValue::String(
                "Signing support in the 'submit' command has been deprecated and will be \
                 removed in a future version of the server. Please migrate to a standalone \
                 signing tool."
                    .to_owned(),
            ),
        );
    }

    Ok(result)
}
