//! Shared stateless semantic-preflight dispatcher for canonical `STTx` values.
//!
//! This is intentionally independent of ledger reads and signature validation.
//! It composes the common transactor checks with transaction-specific semantic
//! helpers, so callers such as Batch can validate canonical inner transactions
//! without depending on the higher-level RPC crate.

use protocol::{
    Asset, INNER_BATCH_TRANSACTION_FLAG, NF_TOKEN_BURNABLE_FLAG, NF_TOKEN_MUTABLE_FLAG,
    NF_TOKEN_ONLY_XRP_FLAG, NF_TOKEN_TRANSFERABLE_FLAG, NF_TOKEN_TRUST_LINE_FLAG, NotTec,
    Permission, Rules, STAmount, STTx, Ter, TxType, UNIVERSAL_TRANSACTION_FLAGS, equal_tokens,
    feature_deep_freeze, feature_lending_protocol, get_field_by_symbol, is_bad_asset,
    is_tes_success,
};

use crate::{
    ChangePreflightFacts, PaymentPreflightEvalFacts, TransactorPreflight0Facts,
    run_change_invoke_preflight_for_txn_type, run_change_preflight, run_change_preflight_flag_mask,
    run_payment_preflight_eval, run_transactor_preflight0,
};

/// Runs the common, amendment-aware, stateless preflight portion for a
/// canonical transaction. Signature validation and ledger-dependent preclaim
/// remain the responsibility of the caller's normal transaction pipeline.
pub fn validate_sttx_transaction_preflight_with_rules(tx: &STTx, rules: &Rules) -> NotTec {
    // Change does not inherit Transactor::preflight1 or the normal account
    // validation path: its own typed preflight validates the canonical zero
    // account, zero fee, signature-free pseudo form.
    if is_change_transaction(tx.get_txn_type()) {
        return validate_sttx_change_preflight(tx, rules);
    }

    if protocol::passes_local_checks(tx).is_err() {
        return Ter::TEM_INVALID;
    }

    if let Some(feature) = Permission::get_instance().get_tx_feature(tx.get_txn_type())
        && !rules.enabled(&feature)
    {
        return Ter::TEM_DISABLED;
    }

    // Transactor::invokePreflight calls checkExtraFeatures before preflight1
    // and the universal/common checks. Preserve that TER precedence.
    let extra_features = validate_sttx_extra_features(tx, rules);
    if !is_tes_success(extra_features) {
        return extra_features;
    }

    // Transactor::preflight1 invokes preflight0 (including the concrete
    // transactor's flag mask) before validating Account and Fee.
    let flags = validate_sttx_flag_mask(tx, rules);
    if !is_tes_success(flags) {
        return flags;
    }

    let common = validate_sttx_common_transactor_preflight(tx);
    if !is_tes_success(common) {
        return common;
    }

    let sponsor = validate_sttx_sponsor_preflight(tx, rules);
    if !is_tes_success(sponsor) {
        return sponsor;
    }

    let typed_preflight = crate::run_with_txn_type_key(rules, tx.get_txn_type(), |txn_type| {
        validate_sttx_typed_semantic_preflight(tx, rules, txn_type)
    });
    let Ok(typed_preflight) = typed_preflight else {
        return Ter::TEM_UNKNOWN;
    };
    if !is_tes_success(typed_preflight) {
        return typed_preflight;
    }

    let delegate = get_field_by_symbol("sfDelegate");
    if !tx.is_field_present(delegate) {
        return Ter::TES_SUCCESS;
    }

    if !rules.enabled(&protocol::feature_id("PermissionDelegationV1_1")) {
        return Ter::TEM_DISABLED;
    }

    if tx.get_account_id(delegate) == tx.get_account_id(get_field_by_symbol("sfAccount")) {
        return Ter::TEM_BAD_SIGNER;
    }

    Ter::TES_SUCCESS
}

fn validate_sttx_flag_mask(tx: &STTx, rules: &Rules) -> NotTec {
    let flags = tx.get_flags();
    let invalid = match tx.get_txn_type() {
        TxType::PAYMENT => {
            let amount = tx.get_field_amount(get_field_by_symbol("sfAmount"));
            flags
                & crate::run_payment_get_flags_mask(
                    amount.holds_mpt_issue(),
                    rules.enabled(&protocol::feature_id("MPTokensV2")),
                )
        }
        TxType::OFFER_CREATE => {
            let mut mask = protocol::OFFER_CREATE_FLAGS_MASK;
            if !rules.enabled(&protocol::feature_id("PermissionedDEX")) {
                mask |= protocol::tfHybrid;
            }
            flags & mask
        }
        TxType::ACCOUNT_SET => flags & crate::get_account_set_flags_mask(),
        TxType::PAYCHAN_CLAIM => flags & crate::get_payment_channel_claim_flags_mask(),
        TxType::MPTOKEN_ISSUANCE_CREATE => flags & protocol::MPT_ISSUANCE_CREATE_FLAGS_MASK,
        TxType::MPTOKEN_ISSUANCE_SET => flags & protocol::MPT_ISSUANCE_SET_FLAGS_MASK,
        TxType::MPTOKEN_AUTHORIZE => flags & protocol::MPT_UNAUTHORIZE_FLAGS_MASK,
        TxType::XCHAIN_MODIFY_BRIDGE => flags & protocol::XCHAIN_MODIFY_BRIDGE_FLAGS_MASK,
        TxType::VAULT_CREATE => flags & protocol::VAULT_CREATE_FLAGS_MASK,
        TxType::BATCH => flags & protocol::BATCH_FLAGS_MASK,
        TxType::LOAN_SET => flags & protocol::LOAN_SET_FLAGS_MASK,
        TxType::LOAN_MANAGE => flags & protocol::LOAN_MANAGE_FLAGS_MASK,
        TxType::LOAN_PAY => flags & protocol::LOAN_PAY_FLAGS_MASK,
        TxType::TRUST_SET => flags & crate::run_trust_set_get_flags_mask(),
        TxType::SIGNER_LIST_SET if rules.enabled(&protocol::feature_id("fixInvalidTxFlags")) => {
            flags & crate::get_signer_list_set_flags_mask(true)
        }
        TxType::SIGNER_LIST_SET => 0,
        TxType::NFTOKEN_CREATE_OFFER => {
            flags & !(UNIVERSAL_TRANSACTION_FLAGS | crate::TF_SELL_NFTOKEN)
        }
        TxType::NFTOKEN_MINT => {
            let mut allowed = UNIVERSAL_TRANSACTION_FLAGS
                | NF_TOKEN_BURNABLE_FLAG
                | NF_TOKEN_ONLY_XRP_FLAG
                | NF_TOKEN_TRANSFERABLE_FLAG;
            if !rules.enabled(&protocol::feature_id("fixRemoveNFTokenAutoTrustLine")) {
                allowed |= NF_TOKEN_TRUST_LINE_FLAG;
            }
            if rules.enabled(&protocol::feature_id("DynamicNFT")) {
                allowed |= NF_TOKEN_MUTABLE_FLAG;
            }
            flags & !allowed
        }
        TxType::AMM_WITHDRAW => {
            flags & !(UNIVERSAL_TRANSACTION_FLAGS | protocol::WITHDRAW_SUB_TX_FLAGS)
        }
        TxType::AMM_DEPOSIT => {
            flags & !(UNIVERSAL_TRANSACTION_FLAGS | protocol::DEPOSIT_SUB_TX_FLAGS)
        }
        TxType::AMM_CLAWBACK => {
            flags & !(UNIVERSAL_TRANSACTION_FLAGS | protocol::AMM_CLAWBACK_TWO_ASSETS_FLAG)
        }
        TxType::DID_SET
        | TxType::DID_DELETE
        | TxType::NFTOKEN_CANCEL_OFFER
        | TxType::NFTOKEN_MODIFY
        | TxType::NFTOKEN_ACCEPT_OFFER
        | TxType::AMM_CREATE
        | TxType::AMM_VOTE
        | TxType::AMM_BID
        | TxType::AMM_DELETE
        | TxType::PERMISSIONED_DOMAIN_SET
        | TxType::PERMISSIONED_DOMAIN_DELETE
        | TxType::ESCROW_FINISH
        | TxType::ESCROW_CREATE
        | TxType::ACCOUNT_DELETE
        | TxType::DEPOSIT_PREAUTH
        | TxType::PAYCHAN_CREATE
        | TxType::PAYCHAN_FUND
        | TxType::CHECK_CREATE
        | TxType::CHECK_CASH
        | TxType::REGULAR_KEY_SET
        | TxType::OFFER_CANCEL
        | TxType::TICKET_CREATE => flags & !UNIVERSAL_TRANSACTION_FLAGS,
        TxType::CREDENTIAL_CREATE | TxType::CREDENTIAL_ACCEPT | TxType::CREDENTIAL_DELETE
            if rules.enabled(&protocol::feature_id("fixInvalidTxFlags")) =>
        {
            flags & !UNIVERSAL_TRANSACTION_FLAGS
        }
        TxType::CREDENTIAL_CREATE | TxType::CREDENTIAL_ACCEPT | TxType::CREDENTIAL_DELETE => 0,
        TxType::ESCROW_CANCEL
        | TxType::CHECK_CANCEL
        | TxType::NFTOKEN_BURN
        | TxType::CLAWBACK
        | TxType::XCHAIN_CREATE_CLAIM_ID
        | TxType::XCHAIN_COMMIT
        | TxType::XCHAIN_CLAIM
        | TxType::XCHAIN_ACCOUNT_CREATE_COMMIT
        | TxType::XCHAIN_ADD_CLAIM_ATTESTATION
        | TxType::XCHAIN_ADD_ACCOUNT_CREATE_ATTESTATION
        | TxType::XCHAIN_CREATE_BRIDGE
        | TxType::ORACLE_SET
        | TxType::ORACLE_DELETE
        | TxType::LEDGER_STATE_FIX
        | TxType::MPTOKEN_ISSUANCE_DESTROY
        | TxType::DELEGATE_SET
        | TxType::VAULT_SET
        | TxType::VAULT_DELETE
        | TxType::VAULT_DEPOSIT
        | TxType::VAULT_WITHDRAW
        | TxType::VAULT_CLAWBACK
        | TxType::LOAN_BROKER_SET
        | TxType::LOAN_BROKER_DELETE
        | TxType::LOAN_BROKER_COVER_DEPOSIT
        | TxType::LOAN_BROKER_COVER_WITHDRAW
        | TxType::LOAN_BROKER_COVER_CLAWBACK
        | TxType::LOAN_DELETE
        | TxType::AMENDMENT
        | TxType::FEE
        | TxType::UNL_MODIFY => flags & !UNIVERSAL_TRANSACTION_FLAGS,
        _ => 0,
    };
    if invalid != 0 {
        Ter::TEM_INVALID_FLAG
    } else {
        Ter::TES_SUCCESS
    }
}

fn validate_sttx_extra_features(tx: &STTx, rules: &Rules) -> NotTec {
    match tx.get_txn_type() {
        TxType::PAYMENT => {
            if tx.is_field_present(get_field_by_symbol("sfDomainID"))
                && !rules.enabled(&protocol::feature_id("PermissionedDEX"))
            {
                return Ter::TEM_DISABLED;
            }
            if tx.is_field_present(get_field_by_symbol("sfCredentialIDs"))
                && !rules.enabled(&protocol::feature_id("Credentials"))
            {
                return Ter::TEM_DISABLED;
            }
        }
        TxType::NFTOKEN_MINT => {
            let has_offer_fields = ["sfAmount", "sfDestination", "sfExpiration"]
                .into_iter()
                .any(|symbol| tx.is_field_present(get_field_by_symbol(symbol)));
            if has_offer_fields && !rules.enabled(&protocol::feature_id("NFTokenMintOffer")) {
                return Ter::TEM_DISABLED;
            }
        }
        TxType::OFFER_CREATE => {
            if tx.is_field_present(get_field_by_symbol("sfDomainID"))
                && !rules.enabled(&protocol::feature_id("PermissionedDEX"))
            {
                return Ter::TEM_DISABLED;
            }
            let pays = get_field_by_symbol("sfTakerPays");
            let gets = get_field_by_symbol("sfTakerGets");
            if tx.is_field_present(pays)
                && tx.is_field_present(gets)
                && !rules.enabled(&protocol::feature_id("MPTokensV2"))
                && (tx.get_field_amount(pays).holds_mpt_issue()
                    || tx.get_field_amount(gets).holds_mpt_issue())
            {
                return Ter::TEM_DISABLED;
            }
        }
        TxType::PAYCHAN_CLAIM => {
            if tx.is_field_present(get_field_by_symbol("sfCredentialIDs"))
                && !rules.enabled(&protocol::feature_id("Credentials"))
            {
                return Ter::TEM_DISABLED;
            }
        }
        TxType::AMM_CREATE => {
            if !protocol::amm_enabled(rules) {
                return Ter::TEM_DISABLED;
            }
            let amount = get_field_by_symbol("sfAmount");
            let amount2 = get_field_by_symbol("sfAmount2");
            if tx.is_field_present(amount)
                && tx.is_field_present(amount2)
                && !rules.enabled(&protocol::feature_id("MPTokensV2"))
                && (tx.get_field_amount(amount).holds_mpt_issue()
                    || tx.get_field_amount(amount2).holds_mpt_issue())
            {
                return Ter::TEM_DISABLED;
            }
        }
        TxType::AMM_WITHDRAW => {
            let (asset, asset2) = amm_pair(tx);
            let amount_is_mpt = |symbol| {
                let field = get_field_by_symbol(symbol);
                tx.is_field_present(field) && tx.get_field_amount(field).holds_mpt_issue()
            };
            if !crate::amm_withdraw_check_extra_features(
                protocol::amm_enabled(rules),
                rules.enabled(&protocol::feature_id("MPTokensV2")),
                matches!(asset, Asset::MPTIssue(_)),
                matches!(asset2, Asset::MPTIssue(_)),
                amount_is_mpt("sfAmount"),
                amount_is_mpt("sfAmount2"),
            ) {
                return Ter::TEM_DISABLED;
            }
        }
        TxType::AMM_VOTE => {
            let (asset, asset2) = amm_pair(tx);
            if !crate::amm_vote_check_extra_features(
                protocol::amm_enabled(rules),
                rules.enabled(&protocol::feature_id("MPTokensV2")),
                matches!(asset, Asset::MPTIssue(_)),
                matches!(asset2, Asset::MPTIssue(_)),
            ) {
                return Ter::TEM_DISABLED;
            }
        }
        TxType::AMM_DEPOSIT => {
            let (asset, asset2) = amm_pair(tx);
            let amount_is_mpt = |symbol| {
                let field = get_field_by_symbol(symbol);
                tx.is_field_present(field) && tx.get_field_amount(field).holds_mpt_issue()
            };
            if !crate::amm_deposit_check_extra_features(
                protocol::amm_enabled(rules),
                rules.enabled(&protocol::feature_id("MPTokensV2")),
                matches!(asset, Asset::MPTIssue(_)),
                matches!(asset2, Asset::MPTIssue(_)),
                amount_is_mpt("sfAmount"),
                amount_is_mpt("sfAmount2"),
            ) {
                return Ter::TEM_DISABLED;
            }
        }
        TxType::AMM_BID => {
            let (asset, asset2) = amm_pair(tx);
            if !crate::amm_bid_check_extra_features(
                protocol::amm_enabled(rules),
                rules.enabled(&protocol::feature_id("MPTokensV2")),
                matches!(asset, Asset::MPTIssue(_)) || matches!(asset2, Asset::MPTIssue(_)),
            ) {
                return Ter::TEM_DISABLED;
            }
        }
        TxType::AMM_DELETE => {
            let (asset, asset2) = amm_pair(tx);
            if !crate::amm_delete_check_extra_features(
                protocol::amm_enabled(rules),
                rules.enabled(&protocol::feature_id("MPTokensV2")),
                matches!(asset, Asset::MPTIssue(_)) || matches!(asset2, Asset::MPTIssue(_)),
            ) {
                return Ter::TEM_DISABLED;
            }
        }
        TxType::AMM_CLAWBACK => {
            let (asset, asset2) = amm_pair(tx);
            let amount = get_field_by_symbol("sfAmount");
            if !crate::run_amm_clawback_check_extra_features(
                rules.enabled(&protocol::feature_id("AMMClawback")),
                rules.enabled(&protocol::feature_id("MPTokensV2")),
                tx.is_field_present(amount) && tx.get_field_amount(amount).holds_mpt_issue(),
                matches!(asset, Asset::MPTIssue(_)),
                matches!(asset2, Asset::MPTIssue(_)),
            ) {
                return Ter::TEM_DISABLED;
            }
        }
        TxType::PERMISSIONED_DOMAIN_SET => {
            if !crate::permissioned_domain_set_check_extra_features(
                rules.enabled(&protocol::feature_id("Credentials")),
            ) {
                return Ter::TEM_DISABLED;
            }
        }
        TxType::ESCROW_FINISH => {
            if tx.is_field_present(get_field_by_symbol("sfCredentialIDs"))
                && !rules.enabled(&protocol::feature_id("Credentials"))
            {
                return Ter::TEM_DISABLED;
            }
        }
        _ => {}
    }
    Ter::TES_SUCCESS
}

/// Backward-compatible name for the standalone transaction semantic dispatcher.
pub fn validate_sttx_semantic_preflight_with_rules(tx: &STTx, rules: &Rules) -> NotTec {
    validate_sttx_transaction_preflight_with_rules(tx, rules)
}

fn validate_sttx_sponsor_preflight(tx: &STTx, rules: &Rules) -> NotTec {
    let sponsor = get_field_by_symbol("sfSponsor");
    let sponsor_flags = get_field_by_symbol("sfSponsorFlags");
    let sponsor_signature = get_field_by_symbol("sfSponsorSignature");
    let has_sponsor = tx.is_field_present(sponsor);
    let has_sponsor_flags = tx.is_field_present(sponsor_flags);
    let has_sponsor_signature = tx.is_field_present(sponsor_signature);

    if (has_sponsor || has_sponsor_flags || has_sponsor_signature)
        && !rules.enabled(&protocol::feature_sponsor())
    {
        return Ter::TEM_DISABLED;
    }
    if has_sponsor != has_sponsor_flags {
        return Ter::TEM_INVALID_FLAG;
    }
    if has_sponsor_signature && (!has_sponsor || !has_sponsor_flags) {
        return Ter::TEM_MALFORMED;
    }
    if has_sponsor_flags {
        let flags = tx.get_field_u32(sponsor_flags);
        if flags == 0 || (flags & ledger::SPF_SPONSOR_FLAG_MASK) != 0 {
            return Ter::TEM_INVALID_FLAG;
        }
        if ledger::is_reserve_sponsored(flags)
            && !ledger::is_reserve_sponsor_allowed(tx.get_txn_type())
        {
            return Ter::TEM_INVALID_FLAG;
        }
    }
    if has_sponsor
        && tx.get_account_id(sponsor) == tx.get_account_id(get_field_by_symbol("sfAccount"))
    {
        return Ter::TEM_MALFORMED;
    }
    Ter::TES_SUCCESS
}

fn is_change_transaction(txn_type: TxType) -> bool {
    matches!(
        txn_type,
        TxType::AMENDMENT | TxType::FEE | TxType::UNL_MODIFY
    )
}

/// `Change::invokePreflight` is deliberately outside the normal
/// `Transactor::preflight1` account/fee/signature path: Change requires the
/// all-zero source account, zero native fee, no signature material, and zero
/// sequence.  Keep the reference's `preflight0` then Change ordering here so
/// every canonical STTx admission path shares it.
fn validate_sttx_change_preflight(tx: &STTx, rules: &Rules) -> NotTec {
    let field = get_field_by_symbol;
    let account = tx.get_account_id(field("sfAccount"));
    let fee = tx.get_field_amount(field("sfFee"));
    let result = run_change_invoke_preflight_for_txn_type(
        tx.get_txn_type(),
        rules.enabled(&feature_lending_protocol()),
        || run_change_preflight_flag_mask(true),
        |flag_mask| {
            run_transactor_preflight0(
                TransactorPreflight0Facts {
                    is_pseudo_tx: true,
                    inner_batch_flag_set: tx.get_flags() & INNER_BATCH_TRANSACTION_FLAG != 0,
                    // The application-level canonical preflight has no node
                    // network-id service. Match its existing local-check
                    // boundary and preserve Change-specific preflight0 checks.
                    tx_id_is_zero: tx.get_transaction_id().is_zero(),
                    tx_flags: tx.get_flags(),
                    ..TransactorPreflight0Facts::default()
                },
                flag_mask,
            )
        },
        || {
            run_change_preflight(
                Ter::TES_SUCCESS,
                ChangePreflightFacts {
                    account_is_zero: account.is_zero(),
                    fee_is_native_and_zero: fee.native() && fee.xrp().drops() == 0,
                    signing_pub_key_empty: tx.get_field_vl(field("sfSigningPubKey")).is_empty(),
                    signature_empty: STTx::get_signature(tx).is_empty(),
                    signers_present: tx.is_field_present(field("sfSigners")),
                    sequence_is_zero: tx.get_field_u32(field("sfSequence")) == 0,
                    previous_txn_id_present: tx.is_field_present(field("sfPreviousTxnID")),
                },
            )
        },
    );

    result.unwrap_or(Ter::TEM_UNKNOWN)
}

fn validate_sttx_common_transactor_preflight(tx: &STTx) -> NotTec {
    let account = get_field_by_symbol("sfAccount");
    if tx.get_account_id(account).is_zero() {
        return Ter::TEM_BAD_SRC_ACCOUNT;
    }

    let fee_field = get_field_by_symbol("sfFee");
    if !tx.is_field_present(fee_field) {
        return Ter::TEM_MALFORMED;
    }
    let fee = tx.get_field_amount(fee_field);
    if !fee.native() || fee.negative() || !fee.is_legal_net() {
        return Ter::TEM_BAD_FEE;
    }

    Ter::TES_SUCCESS
}

fn validate_sttx_typed_semantic_preflight(tx: &STTx, rules: &Rules, txn_type: TxType) -> NotTec {
    match txn_type {
        TxType::PAYMENT => validate_payment_preflight_with_rules(tx, rules),
        TxType::OFFER_CREATE => validate_offer_create_preflight(tx, rules),
        TxType::DEPOSIT_PREAUTH => validate_deposit_preauth_preflight(tx, rules),
        TxType::ACCOUNT_DELETE => {
            validate_default_flags_then(tx, || validate_account_delete_preflight(tx, rules))
        }
        TxType::TICKET_CREATE => validate_default_flags_then(tx, || {
            crate::run_ticket_create_preflight(
                tx.get_field_u32(get_field_by_symbol("sfTicketCount")),
            )
        }),
        TxType::OFFER_CANCEL => validate_default_flags_then(tx, || {
            crate::run_offer_cancel_preflight(
                tx.get_field_u32(get_field_by_symbol("sfOfferSequence")),
            )
        }),
        TxType::PAYCHAN_CREATE => validate_payment_channel_create_preflight(tx),
        TxType::PAYCHAN_FUND => validate_payment_channel_fund_preflight(tx, rules),
        TxType::PAYCHAN_CLAIM => validate_payment_channel_claim_preflight(tx, rules),
        TxType::CHECK_CREATE => validate_check_create_preflight(tx),
        TxType::CHECK_CASH => validate_check_cash_preflight(tx),
        TxType::REGULAR_KEY_SET => {
            validate_default_flags_then(tx, || validate_set_regular_key_preflight(tx))
        }
        TxType::ESCROW_CREATE => validate_escrow_create_preflight(tx, rules),
        TxType::ESCROW_FINISH => validate_escrow_finish_preflight(tx, rules),
        TxType::ACCOUNT_SET => validate_account_set_preflight(tx),
        TxType::TRUST_SET => validate_trust_set_preflight(tx, rules),
        TxType::SIGNER_LIST_SET => validate_signer_list_set_preflight(tx, rules),
        TxType::NFTOKEN_MINT => validate_nftoken_mint_preflight(tx, rules),
        TxType::NFTOKEN_CREATE_OFFER => validate_nftoken_create_offer_preflight(tx),
        TxType::NFTOKEN_ACCEPT_OFFER => validate_nftoken_accept_offer_preflight(tx),
        TxType::NFTOKEN_CANCEL_OFFER => validate_nftoken_cancel_offer_preflight(tx, rules),
        TxType::NFTOKEN_MODIFY => validate_nftoken_modify_preflight(tx),
        TxType::AMM_CREATE => validate_amm_create_preflight(tx, rules),
        TxType::AMM_DEPOSIT => validate_amm_deposit_preflight(tx, rules),
        TxType::AMM_WITHDRAW => validate_amm_withdraw_preflight(tx, rules),
        TxType::AMM_VOTE => validate_amm_vote_preflight(tx, rules),
        TxType::AMM_BID => validate_amm_bid_preflight(tx, rules),
        TxType::AMM_DELETE => Ter::TES_SUCCESS,
        TxType::AMM_CLAWBACK => validate_amm_clawback_preflight(tx),
        TxType::DID_SET => validate_default_flags_then(tx, || validate_did_set_preflight(tx)),
        TxType::CREDENTIAL_CREATE => validate_credential_create_preflight(tx, rules),
        TxType::CREDENTIAL_ACCEPT => validate_credential_accept_preflight(tx, rules),
        TxType::CREDENTIAL_DELETE => validate_credential_delete_preflight(tx, rules),
        TxType::DID_DELETE => validate_default_flags(tx),
        TxType::PERMISSIONED_DOMAIN_SET => validate_permissioned_domain_set_preflight(tx),
        TxType::PERMISSIONED_DOMAIN_DELETE => validate_permissioned_domain_delete_preflight(tx),
        // Types with no additional stateless rule still traverse the standalone
        // dispatcher explicitly; unknown types never silently succeed.
        _ if txn_type.is_dispatchable() => validate_sttx_noop_preflight(txn_type),
        _ => Ter::TEM_UNKNOWN,
    }
}

fn validate_default_flags(tx: &STTx) -> NotTec {
    if tx.get_flags() & !UNIVERSAL_TRANSACTION_FLAGS != 0 {
        Ter::TEM_INVALID_FLAG
    } else {
        Ter::TES_SUCCESS
    }
}

fn validate_default_flags_then(tx: &STTx, next: impl FnOnce() -> NotTec) -> NotTec {
    let flags = validate_default_flags(tx);
    if flags != Ter::TES_SUCCESS {
        flags
    } else {
        next()
    }
}

fn validate_trust_set_preflight(tx: &STTx, rules: &Rules) -> NotTec {
    let limit = tx.get_field_amount(get_field_by_symbol("sfLimitAmount"));
    crate::run_trust_set_preflight_eval(
        crate::TrustSetPreflightEvalFacts {
            tx_flags: tx.get_flags(),
            deep_freeze_enabled: rules.enabled(&feature_deep_freeze()),
            limit_is_legal_net: limit.is_legal_net(),
            limit_is_native: limit.native(),
            limit_currency_is_bad: limit.holds_issue()
                && limit.issue().currency == protocol::bad_currency(),
            limit_is_negative: limit.signum() < 0,
            issuer_present: limit.holds_issue() && !limit.issue().account.is_zero(),
        },
        || {
            if tx.get_flags() & crate::run_trust_set_get_flags_mask() != 0 {
                Ter::TEM_INVALID_FLAG
            } else {
                Ter::TES_SUCCESS
            }
        },
        || Ter::TES_SUCCESS,
    )
}

fn validate_signer_list_set_preflight(tx: &STTx, rules: &Rules) -> NotTec {
    if rules.enabled(&protocol::feature_id("fixInvalidTxFlags"))
        && tx.get_flags() & crate::get_signer_list_set_flags_mask(true) != 0
    {
        return Ter::TEM_INVALID_FLAG;
    }
    let entries_field = get_field_by_symbol("sfSignerEntries");
    let entries = if !tx.is_field_present(entries_field) {
        Ok(Vec::new())
    } else {
        let wrapper = get_field_by_symbol("sfSignerEntry");
        let account = get_field_by_symbol("sfAccount");
        let weight = get_field_by_symbol("sfSignerWeight");
        tx.get_field_array(entries_field)
            .iter()
            .map(|entry| {
                if protocol::StBase::fname(entry) != wrapper {
                    return Err(Ter::TEM_MALFORMED);
                }
                let object = entry.clone();
                if !object.is_field_present(account) || !object.is_field_present(weight) {
                    return Err(Ter::TEM_MALFORMED);
                }
                Ok(crate::SignerListSetEntry {
                    account: object.get_account_id(account),
                    weight: object.get_field_u16(weight),
                })
            })
            .collect()
    };
    crate::run_signer_list_set_preflight(crate::SignerListSetPreflightFacts {
        quorum: tx.get_field_u32(get_field_by_symbol("sfSignerQuorum")),
        has_signer_entries: tx.is_field_present(entries_field),
        signer_entries: entries,
        account: tx.get_account_id(get_field_by_symbol("sfAccount")),
    })
}

fn validate_did_set_preflight(tx: &STTx) -> NotTec {
    let length = |symbol| {
        let field = get_field_by_symbol(symbol);
        tx.is_field_present(field)
            .then(|| tx.get_field_vl(field).len())
    };
    crate::run_did_set_preflight(crate::DidSetPreflightFacts {
        uri_len: length("sfURI"),
        did_document_len: length("sfDIDDocument"),
        data_len: length("sfData"),
    })
}

fn amm_pair(tx: &STTx) -> (Asset, Asset) {
    (
        tx.get_field_issue(get_field_by_symbol("sfAsset")).asset(),
        tx.get_field_issue(get_field_by_symbol("sfAsset2")).asset(),
    )
}

fn invalid_amm_asset_for_pair(asset: Asset, pair: Option<(Asset, Asset)>) -> NotTec {
    let result = match asset {
        Asset::MPTIssue(issue) if issue.issuer().is_zero() => Ter::TEM_BAD_MPT,
        Asset::MPTIssue(_) => Ter::TES_SUCCESS,
        Asset::Issue(issue) => protocol::invalid_amm_asset(issue, None),
    };
    if result != Ter::TES_SUCCESS {
        return result;
    }
    if pair.is_some_and(|(first, second)| asset != first && asset != second) {
        return Ter::TEM_BAD_AMM_TOKENS;
    }
    Ter::TES_SUCCESS
}

fn invalid_classic_amm_pair(asset: Asset, asset2: Asset) -> Option<NotTec> {
    if asset == asset2 {
        return Some(Ter::TEM_BAD_AMM_TOKENS);
    }
    for asset in [asset, asset2] {
        let result = invalid_amm_asset_for_pair(asset, None);
        if result != Ter::TES_SUCCESS {
            return Some(result);
        }
    }
    None
}

fn invalid_amm_transaction_amount(
    amount: &STAmount,
    pool_pair: Option<(Asset, Asset)>,
    valid_zero: bool,
) -> Option<NotTec> {
    let result = invalid_amm_asset_for_pair(amount.asset(), pool_pair);
    if result != Ter::TES_SUCCESS {
        return Some(result);
    }
    let result = if amount.signum() < 0 || (!valid_zero && amount.signum() == 0) {
        Ter::TEM_BAD_AMOUNT
    } else {
        Ter::TES_SUCCESS
    };
    (result != Ter::TES_SUCCESS).then_some(result)
}

fn validate_amm_deposit_preflight(tx: &STTx, rules: &Rules) -> NotTec {
    let (asset, asset2) = amm_pair(tx);
    let optional_amount = |symbol| {
        let field = get_field_by_symbol(symbol);
        tx.is_field_present(field)
            .then(|| tx.get_field_amount(field))
    };
    let amount = optional_amount("sfAmount");
    let amount2 = optional_amount("sfAmount2");
    let e_price = optional_amount("sfEPrice");
    let lp_token_out = optional_amount("sfLPTokenOut");
    let trading_fee_field = get_field_by_symbol("sfTradingFee");

    let e_price_invalid = e_price.as_ref().and_then(|price| {
        if !rules.enabled(&protocol::feature_id("MPTokensV2"))
            && amount
                .as_ref()
                .is_some_and(|value| price.asset() != value.asset())
        {
            Some(Ter::TEM_BAD_AMM_TOKENS)
        } else {
            invalid_amm_transaction_amount(price, None, false)
        }
    });

    crate::run_amm_deposit_preflight_facts(crate::AMMDepositPreflightFacts {
        flags: tx.get_flags(),
        asset_pair_invalid: invalid_classic_amm_pair(asset, asset2),
        amount: amount.as_ref().map(STAmount::asset),
        amount_invalid: amount.as_ref().and_then(|value| {
            invalid_amm_transaction_amount(value, Some((asset, asset2)), e_price.is_some())
        }),
        amount2: amount2.as_ref().map(STAmount::asset),
        amount2_invalid: amount2
            .as_ref()
            .and_then(|value| invalid_amm_transaction_amount(value, Some((asset, asset2)), false)),
        e_price: e_price.as_ref().map(STAmount::asset),
        e_price_invalid,
        lp_token_out_signum: lp_token_out.as_ref().map(STAmount::signum),
        trading_fee: tx
            .is_field_present(trading_fee_field)
            .then(|| tx.get_field_u16(trading_fee_field)),
    })
}

fn validate_amm_bid_preflight(tx: &STTx, rules: &Rules) -> NotTec {
    let (asset, asset2) = amm_pair(tx);
    let optional_amount = |symbol| {
        let field = get_field_by_symbol(symbol);
        tx.is_field_present(field)
            .then(|| tx.get_field_amount(field))
    };
    let bid_min = optional_amount("sfBidMin");
    let bid_max = optional_amount("sfBidMax");
    let auth_accounts_field = get_field_by_symbol("sfAuthAccounts");
    let auth_accounts = if tx.is_field_present(auth_accounts_field) {
        tx.get_field_array(auth_accounts_field)
            .iter()
            .map(|entry| entry.get_account_id(get_field_by_symbol("sfAccount")))
            .collect()
    } else {
        Vec::new()
    };

    crate::run_amm_bid_preflight(crate::AmmBidPreflightFacts {
        invalid_asset_pair: invalid_classic_amm_pair(asset, asset2),
        bid_min_invalid: bid_min
            .as_ref()
            .and_then(|value| invalid_amm_transaction_amount(value, None, false)),
        bid_max_invalid: bid_max
            .as_ref()
            .and_then(|value| invalid_amm_transaction_amount(value, None, false)),
        auth_accounts,
        account: tx.get_account_id(get_field_by_symbol("sfAccount")),
        fix_amm_v1_3_enabled: rules.enabled(&protocol::fix_ammv1_3()),
    })
}

fn validate_amm_clawback_preflight(tx: &STTx) -> NotTec {
    let account = tx.get_account_id(get_field_by_symbol("sfAccount"));
    let holder = tx.get_account_id(get_field_by_symbol("sfHolder"));
    let (asset, asset2) = amm_pair(tx);
    let amount_field = get_field_by_symbol("sfAmount");
    let amount = tx
        .is_field_present(amount_field)
        .then(|| tx.get_field_amount(amount_field));

    crate::run_amm_clawback_preflight_facts(crate::AMMClawbackPreflightFacts {
        issuer_equals_holder: account == holder,
        asset_is_xrp: asset.native(),
        claw_two_assets: tx.get_flags() & protocol::AMM_CLAWBACK_TWO_ASSETS_FLAG != 0,
        asset_issuer_matches_asset2_issuer: asset.issuer() == asset2.issuer(),
        asset_issuer_matches_account: asset.issuer() == account,
        claw_amount_asset_matches_asset: amount.as_ref().map(|value| value.asset() == asset),
        claw_amount_signum: amount.as_ref().map(STAmount::signum),
    })
}

fn validate_amm_vote_preflight(tx: &STTx, rules: &Rules) -> NotTec {
    if tx.get_flags() & !UNIVERSAL_TRANSACTION_FLAGS != 0 {
        return Ter::TEM_INVALID_FLAG;
    }
    let (asset, asset2) = amm_pair(tx);
    if !crate::amm_vote_check_extra_features(
        protocol::amm_enabled(rules),
        rules.enabled(&protocol::feature_id("MPTokensV2")),
        matches!(asset, Asset::MPTIssue(_)),
        matches!(asset2, Asset::MPTIssue(_)),
    ) {
        return Ter::TEM_DISABLED;
    }
    crate::run_amm_vote_preflight_facts(crate::AMMVotePreflightFacts {
        asset_pair_invalid: invalid_classic_amm_pair(asset, asset2),
        trading_fee: tx.get_field_u16(get_field_by_symbol("sfTradingFee")),
    })
}

fn validate_amm_withdraw_preflight(tx: &STTx, rules: &Rules) -> NotTec {
    let allowed = UNIVERSAL_TRANSACTION_FLAGS | protocol::WITHDRAW_SUB_TX_FLAGS;
    if tx.get_flags() & !allowed != 0 {
        return Ter::TEM_INVALID_FLAG;
    }
    let (asset, asset2) = amm_pair(tx);
    let optional_amount = |symbol| {
        let field = get_field_by_symbol(symbol);
        tx.is_field_present(field)
            .then(|| tx.get_field_amount(field))
    };
    let amount = optional_amount("sfAmount");
    let amount2 = optional_amount("sfAmount2");
    let e_price = optional_amount("sfEPrice");
    let lp_token_in = optional_amount("sfLPTokenIn");
    if !crate::amm_withdraw_check_extra_features(
        protocol::amm_enabled(rules),
        rules.enabled(&protocol::feature_id("MPTokensV2")),
        matches!(asset, Asset::MPTIssue(_)),
        matches!(asset2, Asset::MPTIssue(_)),
        amount.as_ref().is_some_and(STAmount::holds_mpt_issue),
        amount2.as_ref().is_some_and(STAmount::holds_mpt_issue),
    ) {
        return Ter::TEM_DISABLED;
    }
    let invalid_amount = |value: &Option<STAmount>, check_pair: bool, valid_zero: bool| {
        value.as_ref().and_then(|amount| {
            invalid_amm_transaction_amount(
                amount,
                check_pair.then_some((asset, asset2)),
                valid_zero,
            )
        })
    };
    let amount_zero_valid = tx.get_flags()
        & (protocol::AMM_ONE_ASSET_WITHDRAW_ALL_FLAG | protocol::AMM_ONE_ASSET_LP_TOKEN_FLAG)
        != 0
        || e_price.is_some();
    crate::run_amm_withdraw_preflight_facts(crate::AMMWithdrawPreflightFacts {
        flags: tx.get_flags(),
        asset_pair_invalid: invalid_classic_amm_pair(asset, asset2),
        amount: amount.as_ref().map(STAmount::asset),
        amount_invalid: invalid_amount(&amount, true, amount_zero_valid),
        amount2: amount2.as_ref().map(STAmount::asset),
        amount2_invalid: invalid_amount(&amount2, true, false),
        e_price: e_price.as_ref().map(STAmount::asset),
        e_price_invalid: invalid_amount(&e_price, false, false),
        lp_token_in_signum: lp_token_in.as_ref().map(STAmount::signum),
    })
}

fn credential_flags_valid(tx: &STTx, rules: &Rules) -> bool {
    !rules.enabled(&protocol::feature_id("fixInvalidTxFlags"))
        || tx.get_flags() & !UNIVERSAL_TRANSACTION_FLAGS == 0
}

fn validate_credential_create_preflight(tx: &STTx, rules: &Rules) -> NotTec {
    if !credential_flags_valid(tx, rules) {
        return Ter::TEM_INVALID_FLAG;
    }
    let subject = get_field_by_symbol("sfSubject");
    let uri = get_field_by_symbol("sfURI");
    let credential_type = get_field_by_symbol("sfCredentialType");
    crate::run_credential_create_preflight(crate::CredentialCreatePreflightFacts {
        subject_present: tx.is_field_present(subject) && !tx.get_account_id(subject).is_zero(),
        uri_len: tx.is_field_present(uri).then(|| tx.get_field_vl(uri).len()),
        credential_type_len: tx.get_field_vl(credential_type).len(),
    })
}

fn validate_credential_accept_preflight(tx: &STTx, rules: &Rules) -> NotTec {
    if !credential_flags_valid(tx, rules) {
        return Ter::TEM_INVALID_FLAG;
    }
    let issuer = get_field_by_symbol("sfIssuer");
    crate::run_credential_accept_preflight(crate::CredentialAcceptPreflightFacts {
        issuer_present: tx.is_field_present(issuer) && !tx.get_account_id(issuer).is_zero(),
        credential_type_len: tx
            .get_field_vl(get_field_by_symbol("sfCredentialType"))
            .len(),
    })
}

fn validate_credential_delete_preflight(tx: &STTx, rules: &Rules) -> NotTec {
    if !credential_flags_valid(tx, rules) {
        return Ter::TEM_INVALID_FLAG;
    }
    let account_field = |symbol| {
        let field = get_field_by_symbol(symbol);
        if !tx.is_field_present(field) {
            crate::CredentialOptionalAccountField::Missing
        } else if tx.get_account_id(field).is_zero() {
            crate::CredentialOptionalAccountField::Zero
        } else {
            crate::CredentialOptionalAccountField::Present
        }
    };
    crate::run_credential_delete_preflight(crate::CredentialDeletePreflightFacts {
        subject: account_field("sfSubject"),
        issuer: account_field("sfIssuer"),
        credential_type_len: tx
            .get_field_vl(get_field_by_symbol("sfCredentialType"))
            .len(),
    })
}

fn validate_permissioned_domain_set_preflight(tx: &STTx) -> NotTec {
    let domain_id = get_field_by_symbol("sfDomainID");
    let accepted_credentials = get_field_by_symbol("sfAcceptedCredentials");
    crate::run_permissioned_domain_set_preflight(
        tx.is_field_present(domain_id),
        tx.is_field_present(domain_id) && tx.get_field_h256(domain_id).is_zero(),
        || {
            ledger::credential_helpers::check_array(
                &tx.get_field_array(accepted_credentials),
                crate::MAX_PERMISSIONED_DOMAIN_CREDENTIALS_ARRAY_SIZE,
            )
        },
    )
}

fn validate_permissioned_domain_delete_preflight(tx: &STTx) -> NotTec {
    crate::run_permissioned_domain_delete_preflight(
        tx.get_field_h256(get_field_by_symbol("sfDomainID"))
            .is_zero(),
    )
}

fn validate_escrow_finish_preflight(tx: &STTx, rules: &Rules) -> NotTec {
    let condition = tx.is_field_present(get_field_by_symbol("sfCondition"));
    let fulfillment = tx.is_field_present(get_field_by_symbol("sfFulfillment"));
    if condition != fulfillment {
        return Ter::TEM_MALFORMED;
    }

    let credential_ids = get_field_by_symbol("sfCredentialIDs");
    if !tx.is_field_present(credential_ids) {
        return Ter::TES_SUCCESS;
    }
    let credentials = tx.get_field_v256(credential_ids);
    if rules.enabled(&protocol::feature_id("fixCleanup3_4_0"))
        && credentials.value().iter().any(|id| id.is_zero())
    {
        return Ter::TEM_MALFORMED;
    }
    ledger::credential_helpers::check_fields(tx)
}

fn validate_nftoken_cancel_offer_preflight(tx: &STTx, rules: &Rules) -> NotTec {
    if tx.get_flags() & !UNIVERSAL_TRANSACTION_FLAGS != 0 {
        return Ter::TEM_INVALID_FLAG;
    }
    let mut ids = tx
        .get_field_v256(get_field_by_symbol("sfNFTokenOffers"))
        .value()
        .to_vec();
    if ids.is_empty() || ids.len() > protocol::MAX_TOKEN_OFFER_CANCEL_COUNT {
        return Ter::TEM_MALFORMED;
    }
    if rules.enabled(&protocol::feature_id("fixCleanup3_2_0")) && ids.iter().any(|id| id.is_zero())
    {
        return Ter::TEM_MALFORMED;
    }
    ids.sort_unstable();
    if ids.windows(2).any(|pair| pair[0] == pair[1]) {
        return Ter::TEM_MALFORMED;
    }
    Ter::TES_SUCCESS
}

fn validate_nftoken_modify_preflight(tx: &STTx) -> NotTec {
    if tx.get_flags() & !UNIVERSAL_TRANSACTION_FLAGS != 0 {
        return Ter::TEM_INVALID_FLAG;
    }
    let owner = get_field_by_symbol("sfOwner");
    if tx.is_field_present(owner)
        && tx.get_account_id(owner) == tx.get_account_id(get_field_by_symbol("sfAccount"))
    {
        return Ter::TEM_MALFORMED;
    }
    let uri = get_field_by_symbol("sfURI");
    if tx.is_field_present(uri) {
        let len = tx.get_field_vl(uri).len();
        if len == 0 || len > protocol::MAX_TOKEN_URI_LENGTH {
            return Ter::TEM_MALFORMED;
        }
    }
    Ter::TES_SUCCESS
}

fn validate_nftoken_create_offer_preflight(tx: &STTx) -> NotTec {
    let allowed = UNIVERSAL_TRANSACTION_FLAGS | crate::TF_SELL_NFTOKEN;
    if tx.get_flags() & !allowed != 0 {
        return Ter::TEM_INVALID_FLAG;
    }
    let amount = tx.get_field_amount(get_field_by_symbol("sfAmount"));
    let optional_account = |symbol| {
        let field = get_field_by_symbol(symbol);
        tx.is_field_present(field).then(|| tx.get_account_id(field))
    };
    let expiration = get_field_by_symbol("sfExpiration");
    let nftoken_id = tx.get_field_h256(get_field_by_symbol("sfNFTokenID"));
    ledger::nftoken_helpers::token_offer_create_preflight(
        &tx.get_account_id(get_field_by_symbol("sfAccount")),
        &amount,
        optional_account("sfDestination").as_ref(),
        tx.is_field_present(expiration)
            .then(|| tx.get_field_u32(expiration)),
        protocol::nft::get_flags(nftoken_id),
        &Rules::default(),
        optional_account("sfOwner").as_ref(),
        tx.get_flags(),
    )
}

fn validate_amm_create_preflight(tx: &STTx, rules: &Rules) -> NotTec {
    let amount_field = get_field_by_symbol("sfAmount");
    let amount2_field = get_field_by_symbol("sfAmount2");
    if !tx.is_field_present(amount_field) || !tx.is_field_present(amount2_field) {
        return Ter::TEM_MALFORMED;
    }
    if tx.get_flags() & !UNIVERSAL_TRANSACTION_FLAGS != 0 {
        return Ter::TEM_INVALID_FLAG;
    }

    let amount = tx.get_field_amount(amount_field);
    let amount2 = tx.get_field_amount(amount2_field);
    if !crate::amm_create_check_extra_features(
        protocol::amm_enabled(rules),
        rules.enabled(&protocol::feature_id("MPTokensV2")),
        amount.holds_mpt_issue(),
        amount2.holds_mpt_issue(),
    ) {
        return Ter::TEM_DISABLED;
    }

    let invalid_amount = invalid_amm_transaction_amount(&amount, None, false);
    let invalid_amount2 = invalid_amm_transaction_amount(&amount2, None, false);

    crate::run_amm_create_preflight_facts(crate::AMMCreatePreflightFacts {
        amount_asset: amount.asset(),
        amount_invalid: invalid_amount,
        amount2_asset: amount2.asset(),
        amount2_invalid: invalid_amount2,
        trading_fee: tx.get_field_u16(get_field_by_symbol("sfTradingFee")),
    })
}

fn validate_nftoken_accept_offer_preflight(tx: &STTx) -> NotTec {
    if tx.get_flags() & !UNIVERSAL_TRANSACTION_FLAGS != 0 {
        return Ter::TEM_INVALID_FLAG;
    }
    let buy_field = get_field_by_symbol("sfNFTokenBuyOffer");
    let sell_field = get_field_by_symbol("sfNFTokenSellOffer");
    let broker_fee_field = get_field_by_symbol("sfNFTokenBrokerFee");
    let buy_present = tx.is_field_present(buy_field);
    let sell_present = tx.is_field_present(sell_field);
    if !buy_present && !sell_present {
        return Ter::TEM_MALFORMED;
    }
    if tx.is_field_present(broker_fee_field) {
        if !buy_present || !sell_present {
            return Ter::TEM_MALFORMED;
        }
        let broker_fee = tx.get_field_amount(broker_fee_field);
        // NFTokenAcceptOffer.cpp checks only the strict-positive condition.
        // Unlike OfferCreate, it does not apply isLegalNet to BrokerFee.
        if broker_fee.signum() <= 0 {
            return Ter::TEM_MALFORMED;
        }
    }
    Ter::TES_SUCCESS
}

fn validate_offer_create_preflight(tx: &STTx, rules: &Rules) -> NotTec {
    let taker_pays_field = get_field_by_symbol("sfTakerPays");
    let taker_gets_field = get_field_by_symbol("sfTakerGets");
    if !tx.is_field_present(taker_pays_field) || !tx.is_field_present(taker_gets_field) {
        return Ter::TEM_MALFORMED;
    }

    let permissioned_dex = rules.enabled(&protocol::feature_id("PermissionedDEX"));
    let domain_field = get_field_by_symbol("sfDomainID");
    let domain_present = tx.is_field_present(domain_field);
    if domain_present && !permissioned_dex {
        return Ter::TEM_DISABLED;
    }

    let taker_pays = tx.get_field_amount(taker_pays_field);
    let taker_gets = tx.get_field_amount(taker_gets_field);
    if (!rules.enabled(&protocol::feature_id("MPTokensV2")))
        && (taker_pays.holds_mpt_issue() || taker_gets.holds_mpt_issue())
    {
        return Ter::TEM_DISABLED;
    }

    let flags = tx.get_flags();
    let mut allowed_flags = UNIVERSAL_TRANSACTION_FLAGS
        | protocol::tfPassive
        | protocol::tfImmediateOrCancel
        | protocol::tfFillOrKill
        | protocol::tfSell;
    if permissioned_dex {
        allowed_flags |= protocol::tfHybrid;
    }
    if flags & !allowed_flags != 0 {
        return Ter::TEM_INVALID_FLAG;
    }
    if flags & protocol::tfHybrid != 0 && !domain_present {
        return Ter::TEM_INVALID_FLAG;
    }
    if rules.enabled(&protocol::feature_id("fixCleanup3_2_0"))
        && domain_present
        && tx.get_field_h256(domain_field).is_zero()
    {
        return Ter::TEM_MALFORMED;
    }
    if flags & protocol::tfImmediateOrCancel != 0 && flags & protocol::tfFillOrKill != 0 {
        return Ter::TEM_INVALID_FLAG;
    }

    let expiration_field = get_field_by_symbol("sfExpiration");
    if tx.is_field_present(expiration_field) && tx.get_field_u32(expiration_field) == 0 {
        return Ter::TEM_BAD_EXPIRATION;
    }
    let offer_sequence_field = get_field_by_symbol("sfOfferSequence");
    if tx.is_field_present(offer_sequence_field) && tx.get_field_u32(offer_sequence_field) == 0 {
        return Ter::TEM_BAD_SEQUENCE;
    }

    if !taker_pays.is_legal_net() || !taker_gets.is_legal_net() {
        return Ter::TEM_BAD_AMOUNT;
    }
    if taker_pays.native() && taker_gets.native() {
        return Ter::TEM_BAD_OFFER;
    }
    if taker_pays.signum() <= 0 || taker_gets.signum() <= 0 {
        return Ter::TEM_BAD_OFFER;
    }
    if taker_pays.asset() == taker_gets.asset() {
        return Ter::TEM_REDUNDANT;
    }
    if is_bad_asset(taker_pays.asset()) || is_bad_asset(taker_gets.asset()) {
        return Ter::TEM_BAD_CURRENCY;
    }

    for amount in [&taker_pays, &taker_gets] {
        if let protocol::Asset::Issue(issue) = amount.asset()
            && amount.native() != issue.account.is_zero()
        {
            return Ter::TEM_BAD_ISSUER;
        }
    }
    Ter::TES_SUCCESS
}

fn validate_sttx_noop_preflight(_txn_type: TxType) -> NotTec {
    Ter::TES_SUCCESS
}

/// Mirrors `rippled` `getMaxSourceAmount` in
/// `src/libxrpl/tx/transactors/payment/Payment.cpp:62-83`.
pub fn payment_max_source_amount(
    account: protocol::AccountID,
    destination_amount: &STAmount,
    send_max: Option<&STAmount>,
) -> STAmount {
    if let Some(send_max) = send_max {
        return send_max.clone();
    }
    if destination_amount.native() || destination_amount.holds_mpt_issue() {
        return destination_amount.clone();
    }

    let mut source_amount = destination_amount.clone();
    source_amount.set_issuer(account);
    source_amount
}

/// Mirrors `Payment::preflight` at
/// `../rippled/src/libxrpl/tx/transactors/payment/Payment.cpp:193-201`.
pub fn payment_is_redundant_to_self(
    account: protocol::AccountID,
    destination: protocol::AccountID,
    source_amount: &STAmount,
    destination_amount: &STAmount,
    has_paths: bool,
) -> bool {
    account == destination
        && equal_tokens(source_amount.asset(), destination_amount.asset())
        && !has_paths
}

#[cfg(test)]
fn validate_payment_preflight(tx: &STTx) -> NotTec {
    validate_payment_preflight_with_rules(tx, &Rules::default())
}

fn validate_payment_preflight_with_rules(tx: &STTx, rules: &Rules) -> NotTec {
    let account_field = get_field_by_symbol("sfAccount");
    let amount_field = get_field_by_symbol("sfAmount");
    let destination_field = get_field_by_symbol("sfDestination");
    let deliver_min_field = get_field_by_symbol("sfDeliverMin");
    let send_max_field = get_field_by_symbol("sfSendMax");
    let paths_field = get_field_by_symbol("sfPaths");
    if !tx.is_field_present(amount_field) || !tx.is_field_present(destination_field) {
        return Ter::TEM_MALFORMED;
    }
    let domain = get_field_by_symbol("sfDomainID");
    if rules.enabled(&protocol::feature_id("fixCleanup3_2_0"))
        && tx.is_field_present(domain)
        && tx.get_field_h256(domain).is_zero()
    {
        return Ter::TEM_MALFORMED;
    }

    // This is the same Payment::preflight decision table used by the concrete
    // Payment transactor. Keeping semantic preflight on this shared evaluator
    // prevents RPC admission from accepting flag combinations that rippled
    // rejects before preclaim or Flow execution.
    let amount = tx.get_field_amount(amount_field);
    let source_account = tx.get_account_id(account_field);
    let send_max = tx
        .is_field_present(send_max_field)
        .then(|| tx.get_field_amount(send_max_field));
    let max_source_amount = payment_max_source_amount(source_account, &amount, send_max.as_ref());
    let deliver_min = tx
        .is_field_present(deliver_min_field)
        .then(|| tx.get_field_amount(deliver_min_field));
    let destination_account = tx.get_account_id(destination_field);
    let has_paths = tx.is_field_present(paths_field);

    run_payment_preflight_eval(
        PaymentPreflightEvalFacts {
            tx_flags: tx.get_flags(),
            mptokens_v1_enabled: rules.enabled(&protocol::feature_id("MPTokensV1")),
            mptokens_v2_enabled: rules.enabled(&protocol::feature_id("MPTokensV2")),
            amount_is_mpt: amount.holds_mpt_issue(),
            paths_present: has_paths,
            send_max_present: send_max.is_some(),
            send_max_asset_matches_amount: send_max
                .as_ref()
                .is_none_or(|value| value.asset() == amount.asset()),
            send_max_is_mpt: send_max.as_ref().is_some_and(STAmount::holds_mpt_issue),
            amount_is_legal_net: amount.is_legal_net(),
            max_source_is_legal_net: max_source_amount.is_legal_net(),
            destination_present: !destination_account.is_zero(),
            max_source_positive: max_source_amount.signum() > 0,
            amount_positive: amount.signum() > 0,
            src_asset_bad: is_bad_asset(max_source_amount.asset()),
            dst_asset_bad: is_bad_asset(amount.asset()),
            src_asset_is_xrp: max_source_amount.native(),
            dst_asset_is_xrp: amount.native(),
            account_equals_destination: source_account == destination_account,
            src_dst_tokens_equal: equal_tokens(max_source_amount.asset(), amount.asset()),
            deliver_min_present: deliver_min.is_some(),
            deliver_min_is_legal_net: deliver_min.as_ref().is_none_or(STAmount::is_legal_net),
            deliver_min_is_positive: deliver_min.as_ref().is_none_or(|value| value.signum() > 0),
            deliver_min_asset_matches_amount: deliver_min
                .as_ref()
                .is_none_or(|value| value.asset() == amount.asset()),
            deliver_min_not_greater_than_amount: deliver_min
                .as_ref()
                .is_none_or(|value| value <= &amount),
        },
        || Ter::TES_SUCCESS,
        || Ter::TES_SUCCESS,
        || Ter::TES_SUCCESS,
    )
}

fn validate_deposit_preauth_preflight(tx: &STTx, rules: &Rules) -> NotTec {
    let account = tx.get_account_id(get_field_by_symbol("sfAccount"));
    let authorize_field = get_field_by_symbol("sfAuthorize");
    let unauthorize_field = get_field_by_symbol("sfUnauthorize");
    let authorize_credentials_field = get_field_by_symbol("sfAuthorizeCredentials");
    let unauthorize_credentials_field = get_field_by_symbol("sfUnauthorizeCredentials");
    let authorize = tx
        .is_field_present(authorize_field)
        .then(|| tx.get_account_id(authorize_field));
    let unauthorize = tx
        .is_field_present(unauthorize_field)
        .then(|| tx.get_account_id(unauthorize_field));
    let authorize_credentials_present = tx.is_field_present(authorize_credentials_field);
    let unauthorize_credentials_present = tx.is_field_present(unauthorize_credentials_field);

    if !crate::deposit_preauth_check_extra_features(
        authorize_credentials_present,
        unauthorize_credentials_present,
        rules.enabled(&protocol::feature_id("Credentials")),
    ) {
        return Ter::TEM_DISABLED;
    }

    crate::run_deposit_preauth_preflight(
        crate::DepositPreauthPreflightFacts {
            account,
            authorize,
            unauthorize,
            authorize_is_zero: authorize.is_some_and(|account| account.is_zero()),
            unauthorize_is_zero: unauthorize.is_some_and(|account| account.is_zero()),
            authorize_credentials_present,
            unauthorize_credentials_present,
        },
        || {
            let credentials = tx.get_field_array(if authorize_credentials_present {
                authorize_credentials_field
            } else {
                unauthorize_credentials_field
            });
            ledger::credential_helpers::check_array(
                &credentials,
                ledger::credential_helpers::MAX_CREDENTIALS_ARRAY_SIZE,
            )
        },
    )
}

fn validate_account_delete_preflight(tx: &STTx, rules: &Rules) -> NotTec {
    let account = tx.get_account_id(get_field_by_symbol("sfAccount"));
    let destination_field = get_field_by_symbol("sfDestination");
    if !tx.is_field_present(destination_field) {
        return Ter::TEM_MALFORMED;
    }

    let credential_ids_present = tx.is_field_present(get_field_by_symbol("sfCredentialIDs"));
    if !crate::account_delete_check_extra_features(
        credential_ids_present,
        rules.enabled(&protocol::feature_id("Credentials")),
    ) {
        return Ter::TEM_DISABLED;
    }

    crate::run_account_delete_preflight(
        crate::AccountDeletePreflightFacts {
            account,
            destination: tx.get_account_id(destination_field),
        },
        || ledger::credential_helpers::check_fields(tx),
    )
}

fn validate_payment_channel_create_preflight(tx: &STTx) -> NotTec {
    let account = get_field_by_symbol("sfAccount");
    let amount_field = get_field_by_symbol("sfAmount");
    let destination = get_field_by_symbol("sfDestination");
    let public_key = get_field_by_symbol("sfPublicKey");
    if !tx.is_field_present(amount_field)
        || !tx.is_field_present(destination)
        || !tx.is_field_present(public_key)
    {
        return Ter::TEM_MALFORMED;
    }

    let amount = tx.get_field_amount(amount_field);
    crate::run_payment_channel_create_preflight(crate::PaymentChannelCreatePreflightFacts {
        amount_is_xrp: amount.native(),
        amount_positive: amount.signum() > 0,
        tx_account_is_destination: tx.get_account_id(account) == tx.get_account_id(destination),
        public_key_valid: protocol::PublicKey::from_slice(&tx.get_field_vl(public_key)).is_ok(),
    })
}

fn validate_payment_channel_fund_preflight(tx: &STTx, rules: &Rules) -> NotTec {
    let amount_field = get_field_by_symbol("sfAmount");
    let channel_field = get_field_by_symbol("sfChannel");
    if !tx.is_field_present(amount_field) || !tx.is_field_present(channel_field) {
        return Ter::TEM_MALFORMED;
    }
    if tx.get_flags() & !UNIVERSAL_TRANSACTION_FLAGS != 0 {
        return Ter::TEM_INVALID_FLAG;
    }
    if rules.enabled(&protocol::feature_id("fixCleanup3_2_0"))
        && tx.get_field_h256(channel_field).is_zero()
    {
        return Ter::TEM_MALFORMED;
    }

    let amount = tx.get_field_amount(amount_field);
    crate::run_payment_channel_fund_preflight(amount.native(), amount.signum() > 0)
}

fn validate_payment_channel_claim_preflight(tx: &STTx, rules: &Rules) -> NotTec {
    let channel_field = get_field_by_symbol("sfChannel");
    if !tx.is_field_present(channel_field) {
        return Ter::TEM_MALFORMED;
    }
    if rules.enabled(&protocol::feature_id("fixCleanup3_2_0"))
        && tx.get_field_h256(channel_field).is_zero()
    {
        return Ter::TEM_MALFORMED;
    }
    if !crate::run_payment_channel_claim_check_extra_features(
        tx.is_field_present(get_field_by_symbol("sfCredentialIDs")),
        rules.enabled(&protocol::feature_id("Credentials")),
    ) {
        return Ter::TEM_DISABLED;
    }
    if tx.get_flags() & crate::get_payment_channel_claim_flags_mask() != 0 {
        return Ter::TEM_INVALID_FLAG;
    }

    let balance_field = get_field_by_symbol("sfBalance");
    let amount_field = get_field_by_symbol("sfAmount");
    let signature_field = get_field_by_symbol("sfSignature");
    let public_key_field = get_field_by_symbol("sfPublicKey");
    let balance = tx
        .is_field_present(balance_field)
        .then(|| tx.get_field_amount(balance_field));
    let amount = tx
        .is_field_present(amount_field)
        .then(|| tx.get_field_amount(amount_field));
    let public_key = tx
        .is_field_present(public_key_field)
        .then(|| protocol::PublicKey::from_slice(&tx.get_field_vl(public_key_field)))
        .transpose()
        .ok()
        .flatten();
    let signature = tx
        .is_field_present(signature_field)
        .then(|| tx.get_field_vl(signature_field));
    let requested_balance = balance
        .as_ref()
        .filter(|value| value.native())
        .map_or(0, |value| value.xrp().drops().max(0) as u64);
    let authorized_amount = amount
        .as_ref()
        .filter(|value| value.native())
        .map_or(requested_balance, |value| value.xrp().drops().max(0) as u64);
    let signature_facts =
        signature
            .as_ref()
            .map(|_| crate::PaymentChannelClaimSignaturePreflightFacts {
                public_key_present: tx.is_field_present(public_key_field),
                requested_balance_drops: requested_balance,
                authorization_message: crate::PaymentChannelClaimAuthorizationMessageFacts {
                    channel_key: tx.get_field_h256(channel_field),
                    authorized_amount_drops: authorized_amount,
                },
                public_key_type_valid: public_key.is_some(),
            });

    crate::run_payment_channel_claim_preflight(
        crate::PaymentChannelClaimPreflightFacts {
            balance_present: balance.is_some(),
            balance_is_xrp: balance.as_ref().is_none_or(STAmount::native),
            balance_positive: balance.as_ref().is_none_or(|value| value.signum() > 0),
            amount_present: amount.is_some(),
            amount_is_xrp: amount.as_ref().is_none_or(STAmount::native),
            amount_positive: amount.as_ref().is_none_or(|value| value.signum() > 0),
            balance_exceeds_amount: balance
                .as_ref()
                .zip(amount.as_ref())
                .is_some_and(|(balance, amount)| balance > amount),
            tx_flags: tx.get_flags(),
            signature: signature_facts,
        },
        |message| {
            public_key
                .as_ref()
                .zip(signature.as_ref())
                .is_some_and(|(public_key, signature)| {
                    protocol::sign::verify(public_key, message, signature)
                })
        },
        || ledger::credential_helpers::check_fields(tx),
    )
}

fn validate_check_create_preflight(tx: &STTx) -> NotTec {
    let account = get_field_by_symbol("sfAccount");
    let destination = get_field_by_symbol("sfDestination");
    let send_max_field = get_field_by_symbol("sfSendMax");
    if !tx.is_field_present(destination) || !tx.is_field_present(send_max_field) {
        return Ter::TEM_MALFORMED;
    }

    let send_max = tx.get_field_amount(send_max_field);
    let expiration = get_field_by_symbol("sfExpiration");
    crate::run_check_create_preflight(crate::CheckCreatePreflightFacts {
        tx_account_is_destination: tx.get_account_id(account) == tx.get_account_id(destination),
        send_max_is_legal: send_max.is_legal_net(),
        send_max_signum_positive: send_max.signum() > 0,
        send_max_currency_is_bad: send_max.holds_issue()
            && send_max.issue().currency == protocol::bad_currency(),
        expiration: tx
            .is_field_present(expiration)
            .then(|| tx.get_field_u32(expiration)),
    })
}

fn validate_check_cash_preflight(tx: &STTx) -> NotTec {
    let amount_field = get_field_by_symbol("sfAmount");
    let deliver_min_field = get_field_by_symbol("sfDeliverMin");
    let amount_present = tx.is_field_present(amount_field);
    let deliver_min_present = tx.is_field_present(deliver_min_field);
    let value = amount_present
        .then(|| tx.get_field_amount(amount_field))
        .or_else(|| deliver_min_present.then(|| tx.get_field_amount(deliver_min_field)));
    let (value_signum_positive, value_currency_is_bad) = value
        .map(|value| {
            (
                value.signum() > 0,
                !value.native() && value.issue().currency.is_zero(),
            )
        })
        .unwrap_or((true, false));

    crate::run_check_cash_preflight(crate::CheckCashPreflightFacts {
        amount_present,
        deliver_min_present,
        value_is_legal: true,
        value_signum_positive,
        value_currency_is_bad,
    })
}

/// Mirrors `NFTokenMint::getFlagsMask` and `NFTokenMint::preflight` in
/// `rippled/src/libxrpl/tx/transactors/nft/NFTokenMint.cpp`.
fn validate_nftoken_mint_preflight(tx: &STTx, rules: &Rules) -> NotTec {
    let mut valid_flags = UNIVERSAL_TRANSACTION_FLAGS
        | NF_TOKEN_BURNABLE_FLAG
        | NF_TOKEN_ONLY_XRP_FLAG
        | NF_TOKEN_TRANSFERABLE_FLAG;
    if !rules.enabled(&protocol::feature_id("fixRemoveNFTokenAutoTrustLine")) {
        valid_flags |= NF_TOKEN_TRUST_LINE_FLAG;
    }
    if rules.enabled(&protocol::feature_id("DynamicNFT")) {
        valid_flags |= NF_TOKEN_MUTABLE_FLAG;
    }
    if tx.get_flags() & !valid_flags != 0 {
        return Ter::TEM_INVALID_FLAG;
    }

    if tx.is_field_present(get_field_by_symbol("sfTransferFee")) {
        let transfer_fee = tx.get_field_u16(get_field_by_symbol("sfTransferFee"));
        if transfer_fee > 50_000 {
            return Ter::TEM_BAD_NFTOKEN_TRANSFER_FEE;
        }
        if transfer_fee > 0 && tx.get_flags() & NF_TOKEN_TRANSFERABLE_FLAG == 0 {
            return Ter::TEM_MALFORMED;
        }
    }

    if tx.is_field_present(get_field_by_symbol("sfIssuer"))
        && tx.get_account_id(get_field_by_symbol("sfIssuer"))
            == tx.get_account_id(get_field_by_symbol("sfAccount"))
    {
        return Ter::TEM_MALFORMED;
    }

    if tx.is_field_present(get_field_by_symbol("sfURI")) {
        let uri = tx.get_field_vl(get_field_by_symbol("sfURI"));
        if uri.is_empty() || uri.len() > 256 {
            return Ter::TEM_MALFORMED;
        }
    }

    Ter::TES_SUCCESS
}

fn validate_set_regular_key_preflight(tx: &STTx) -> NotTec {
    let account = tx.get_account_id(get_field_by_symbol("sfAccount"));
    let regular_key_field = get_field_by_symbol("sfRegularKey");
    if tx.is_field_present(regular_key_field) && tx.get_account_id(regular_key_field) == account {
        return Ter::TEM_BAD_REGKEY;
    }
    Ter::TES_SUCCESS
}

fn validate_escrow_create_preflight(tx: &STTx, rules: &Rules) -> NotTec {
    let amount_field = get_field_by_symbol("sfAmount");
    let destination = get_field_by_symbol("sfDestination");
    if !tx.is_field_present(amount_field) || !tx.is_field_present(destination) {
        return Ter::TEM_MALFORMED;
    }

    crate::run_escrow_create_sttx_preflight(tx, rules)
}

fn validate_account_set_preflight(tx: &STTx) -> NotTec {
    crate::run_account_set_preflight(crate::AccountSetPreflightFacts {
        tx_flags: tx.get_flags(),
        set_flag: tx.get_field_u32(get_field_by_symbol("sfSetFlag")),
        clear_flag: tx.get_field_u32(get_field_by_symbol("sfClearFlag")),
        transfer_rate: tx
            .is_field_present(get_field_by_symbol("sfTransferRate"))
            .then(|| tx.get_field_u32(get_field_by_symbol("sfTransferRate"))),
        tick_size: tx
            .is_field_present(get_field_by_symbol("sfTickSize"))
            .then(|| tx.get_field_u8(get_field_by_symbol("sfTickSize"))),
        message_key_present: tx.is_field_present(get_field_by_symbol("sfMessageKey")),
        message_key_is_valid: !tx.is_field_present(get_field_by_symbol("sfMessageKey"))
            || tx
                .get_field_vl(get_field_by_symbol("sfMessageKey"))
                .is_empty()
            || protocol::PublicKey::from_slice(
                &tx.get_field_vl(get_field_by_symbol("sfMessageKey")),
            )
            .is_ok(),
        domain_len: tx
            .is_field_present(get_field_by_symbol("sfDomain"))
            .then(|| tx.get_field_vl(get_field_by_symbol("sfDomain")).len()),
        nftoken_minter_present: tx.is_field_present(get_field_by_symbol("sfNFTokenMinter")),
        ..crate::AccountSetPreflightFacts::default()
    })
}

#[cfg(test)]
mod tests {
    use basics::base_uint::Uint256;
    use protocol::{
        AccountID, Asset, Currency, IOUAmount, Issue, KeyType, MPTAmount, MPTID, MPTIssue, Rules,
        STAmount, STArray, STIssue, STObject, STPathSet, STTx, SecretKey, Ter, TxType, XRPAmount,
        derive_public_key, get_field_by_symbol,
    };

    use super::{
        validate_amm_bid_preflight, validate_amm_clawback_preflight, validate_amm_create_preflight,
        validate_amm_deposit_preflight, validate_amm_vote_preflight,
        validate_amm_withdraw_preflight, validate_escrow_finish_preflight,
        validate_payment_preflight, validate_permissioned_domain_delete_preflight,
        validate_permissioned_domain_set_preflight, validate_sttx_transaction_preflight_with_rules,
    };

    fn sf(name: &str) -> &'static protocol::SField {
        get_field_by_symbol(name)
    }

    fn change_amendment() -> STTx {
        STTx::new(TxType::AMENDMENT, |tx| {
            tx.set_field_amount(sf("sfFee"), STAmount::from_xrp_amount(XRPAmount::new()));
            tx.set_field_h256(sf("sfAmendment"), Uint256::from_u64(1));
        })
    }

    fn payment_self_tx(amount: STAmount, send_max: Option<STAmount>, has_paths: bool) -> STTx {
        let account = AccountID::from_array([0xA1; 20]);
        STTx::new(TxType::PAYMENT, |tx| {
            tx.set_account_id(sf("sfAccount"), account);
            tx.set_account_id(sf("sfDestination"), account);
            tx.set_field_amount(sf("sfAmount"), amount);
            tx.set_field_amount(
                sf("sfFee"),
                STAmount::from_xrp_amount(XRPAmount::from_drops(10)),
            );
            tx.set_field_u32(sf("sfSequence"), 1);
            if let Some(send_max) = send_max {
                tx.set_field_amount(sf("sfSendMax"), send_max);
            }
            if has_paths {
                tx.set_field_path_set(sf("sfPaths"), STPathSet::new(sf("sfPaths")));
            }
        })
    }

    fn nftoken_mint(flags: u32, transfer_fee: Option<u16>) -> STTx {
        STTx::new(TxType::NFTOKEN_MINT, |tx| {
            tx.set_account_id(sf("sfAccount"), AccountID::from_array([0xC3; 20]));
            tx.set_field_u32(sf("sfNFTokenTaxon"), 0);
            tx.set_field_u32(sf("sfFlags"), flags);
            tx.set_field_amount(
                sf("sfFee"),
                STAmount::from_xrp_amount(XRPAmount::from_drops(10)),
            );
            tx.set_field_u32(sf("sfSequence"), 1);
            if let Some(transfer_fee) = transfer_fee {
                tx.set_field_u16(sf("sfTransferFee"), transfer_fee);
            }
        })
    }

    fn offer_create(flags: u32) -> STTx {
        let account = AccountID::from_array([0xD1; 20]);
        let issuer = AccountID::from_array([0xD2; 20]);
        STTx::new(TxType::OFFER_CREATE, |tx| {
            tx.set_account_id(sf("sfAccount"), account);
            tx.set_field_amount(
                sf("sfTakerPays"),
                STAmount::from_xrp_amount(XRPAmount::from_drops(100)),
            );
            tx.set_field_amount(sf("sfTakerGets"), iou(sf("sfTakerGets"), issuer, 0x44));
            tx.set_field_u32(sf("sfFlags"), flags);
            tx.set_field_amount(
                sf("sfFee"),
                STAmount::from_xrp_amount(XRPAmount::from_drops(10)),
            );
            tx.set_field_u32(sf("sfSequence"), 1);
        })
    }

    fn amm_create(trading_fee: u16) -> STTx {
        let issuer = AccountID::from_array([0xF2; 20]);
        STTx::new(TxType::AMM_CREATE, |tx| {
            tx.set_account_id(sf("sfAccount"), AccountID::from_array([0xF1; 20]));
            tx.set_field_amount(
                sf("sfAmount"),
                STAmount::from_xrp_amount(XRPAmount::from_drops(100)),
            );
            tx.set_field_amount(sf("sfAmount2"), iou(sf("sfAmount2"), issuer, 0x66));
            tx.set_field_u16(sf("sfTradingFee"), trading_fee);
            tx.set_field_amount(
                sf("sfFee"),
                STAmount::from_xrp_amount(XRPAmount::from_drops(10)),
            );
            tx.set_field_u32(sf("sfSequence"), 1);
        })
    }

    fn paychan_claim(flags: u32, with_signature: bool) -> STTx {
        STTx::new(TxType::PAYCHAN_CLAIM, |tx| {
            tx.set_account_id(sf("sfAccount"), AccountID::from_array([0xA7; 20]));
            tx.set_field_h256(sf("sfChannel"), Uint256::from_u64(7));
            tx.set_field_amount(
                sf("sfBalance"),
                STAmount::from_xrp_amount(XRPAmount::from_drops(100)),
            );
            tx.set_field_u32(sf("sfFlags"), flags);
            tx.set_field_amount(
                sf("sfFee"),
                STAmount::from_xrp_amount(XRPAmount::from_drops(10)),
            );
            tx.set_field_u32(sf("sfSequence"), 1);
            if with_signature {
                let secret = SecretKey::from_bytes([0x71; 32]);
                let public = derive_public_key(KeyType::Secp256k1, &secret).expect("public key");
                tx.set_field_vl(sf("sfPublicKey"), public.as_bytes());
                tx.set_field_vl(sf("sfSignature"), &[1]);
            }
        })
    }

    fn nftoken_accept_offer(
        include_buy: bool,
        include_sell: bool,
        broker_fee: Option<i64>,
    ) -> STTx {
        STTx::new(TxType::NFTOKEN_ACCEPT_OFFER, |tx| {
            tx.set_account_id(sf("sfAccount"), AccountID::from_array([0xE1; 20]));
            tx.set_field_amount(
                sf("sfFee"),
                STAmount::from_xrp_amount(XRPAmount::from_drops(10)),
            );
            tx.set_field_u32(sf("sfSequence"), 1);
            if include_buy {
                tx.set_field_h256(sf("sfNFTokenBuyOffer"), Uint256::from_u64(1));
            }
            if include_sell {
                tx.set_field_h256(sf("sfNFTokenSellOffer"), Uint256::from_u64(2));
            }
            if let Some(broker_fee) = broker_fee {
                tx.set_field_amount(
                    sf("sfNFTokenBrokerFee"),
                    STAmount::from_xrp_amount(XRPAmount::from_drops(broker_fee)),
                );
            }
        })
    }

    fn current_nft_rules() -> Rules {
        Rules::new([
            protocol::feature_id("fixRemoveNFTokenAutoTrustLine"),
            protocol::feature_id("DynamicNFT"),
        ])
    }

    fn iou(field: &'static protocol::SField, issuer: AccountID, currency_byte: u8) -> STAmount {
        STAmount::from_iou_amount(
            field,
            IOUAmount::from_parts(1, 0).expect("positive IOU amount"),
            Issue {
                currency: Currency::from_array([currency_byte; 20]),
                account: issuer,
            },
        )
    }

    fn amm_shell(tx_type: TxType, flags: u32) -> STTx {
        let issuer = AccountID::from_array([0xA2; 20]);
        STTx::new(tx_type, |tx| {
            tx.set_account_id(sf("sfAccount"), AccountID::from_array([0xA1; 20]));
            tx.set_field_issue(
                sf("sfAsset"),
                STIssue::new_with_asset(
                    sf("sfAsset"),
                    Asset::Issue(Issue::new(
                        protocol::xrp_currency(),
                        protocol::xrp_account(),
                    )),
                ),
            );
            tx.set_field_issue(
                sf("sfAsset2"),
                STIssue::new_with_asset(
                    sf("sfAsset2"),
                    Asset::Issue(Issue {
                        currency: Currency::from_array([0x44; 20]),
                        account: issuer,
                    }),
                ),
            );
            tx.set_field_u32(sf("sfFlags"), flags);
            tx.set_field_amount(
                sf("sfFee"),
                STAmount::from_xrp_amount(XRPAmount::from_drops(10)),
            );
            tx.set_field_u32(sf("sfSequence"), 1);
        })
    }

    #[test]
    fn advanced_transactor_preflights_match_enabled_amendment_edges() {
        let amm_rules = Rules::new([
            protocol::feature_amm(),
            protocol::feature_universal_number(),
            protocol::fix_ammv1_3(),
        ]);

        let mut deposit = amm_shell(TxType::AMM_DEPOSIT, protocol::AMM_SINGLE_ASSET_FLAG);
        deposit.set_field_amount(
            sf("sfAmount"),
            STAmount::from_xrp_amount(XRPAmount::from_drops(1)),
        );
        assert_eq!(
            validate_amm_deposit_preflight(&deposit, &amm_rules),
            Ter::TES_SUCCESS
        );
        deposit.set_field_u32(sf("sfFlags"), 0);
        assert_eq!(
            validate_amm_deposit_preflight(&deposit, &amm_rules),
            Ter::TEM_MALFORMED
        );

        let mut bid = amm_shell(TxType::AMM_BID, 0);
        let duplicate = AccountID::from_array([0xB1; 20]);
        let mut auth_accounts = STArray::new(sf("sfAuthAccounts"));
        for _ in 0..2 {
            let mut entry = STObject::make_inner_object(sf("sfAuthAccount"));
            entry.set_account_id(sf("sfAccount"), duplicate);
            auth_accounts.push_back(entry);
        }
        bid.set_field_array(sf("sfAuthAccounts"), auth_accounts);
        assert_eq!(
            validate_amm_bid_preflight(&bid, &amm_rules),
            Ter::TEM_MALFORMED
        );

        let mut clawback = amm_shell(TxType::AMM_CLAWBACK, 0);
        clawback.set_account_id(sf("sfHolder"), AccountID::from_array([0xA1; 20]));
        assert_eq!(
            validate_amm_clawback_preflight(&clawback),
            Ter::TEM_MALFORMED
        );

        let mut domain_delete = STTx::new(TxType::PERMISSIONED_DOMAIN_DELETE, |_| {});
        domain_delete.set_field_h256(sf("sfDomainID"), Uint256::default());
        assert_eq!(
            validate_permissioned_domain_delete_preflight(&domain_delete),
            Ter::TEM_MALFORMED
        );

        let mut domain_set = STTx::new(TxType::PERMISSIONED_DOMAIN_SET, |_| {});
        domain_set.set_field_array(
            sf("sfAcceptedCredentials"),
            STArray::new(sf("sfAcceptedCredentials")),
        );
        assert_eq!(
            validate_permissioned_domain_set_preflight(&domain_set),
            Ter::TEM_ARRAY_EMPTY
        );

        let mut finish = STTx::new(TxType::ESCROW_FINISH, |_| {});
        finish.set_field_vl(sf("sfCondition"), &[1]);
        assert_eq!(
            validate_escrow_finish_preflight(&finish, &Rules::new(std::iter::empty())),
            Ter::TEM_MALFORMED
        );
    }

    #[test]
    fn amm_preflights_reject_zero_issuer_mpt_as_bad_mpt() {
        let rules = Rules::new([
            protocol::feature_amm(),
            protocol::feature_universal_number(),
            protocol::feature_id("MPTokensV2"),
        ]);
        let bad_issue = MPTIssue::new(MPTID::default());
        let bad_asset = Asset::MPTIssue(bad_issue);
        let set_bad_pool_asset = |tx: &mut STTx| {
            tx.set_field_issue(
                sf("sfAsset"),
                STIssue::new_with_asset(sf("sfAsset"), bad_asset),
            );
        };

        let mut create = amm_create(0);
        create.set_field_amount(
            sf("sfAmount"),
            STAmount::from_mpt_amount(sf("sfAmount"), MPTAmount::from_value(1), bad_issue),
        );
        assert_eq!(
            validate_amm_create_preflight(&create, &rules),
            Ter::TEM_BAD_MPT
        );

        let mut deposit = amm_shell(TxType::AMM_DEPOSIT, protocol::AMM_SINGLE_ASSET_FLAG);
        set_bad_pool_asset(&mut deposit);
        deposit.set_field_amount(
            sf("sfAmount"),
            STAmount::from_mpt_amount(sf("sfAmount"), MPTAmount::from_value(1), bad_issue),
        );
        assert_eq!(
            validate_amm_deposit_preflight(&deposit, &rules),
            Ter::TEM_BAD_MPT
        );

        let mut withdraw = amm_shell(TxType::AMM_WITHDRAW, protocol::AMM_WITHDRAW_ALL_FLAG);
        set_bad_pool_asset(&mut withdraw);
        assert_eq!(
            validate_amm_withdraw_preflight(&withdraw, &rules),
            Ter::TEM_BAD_MPT
        );

        let mut vote = amm_shell(TxType::AMM_VOTE, 0);
        set_bad_pool_asset(&mut vote);
        vote.set_field_u16(sf("sfTradingFee"), 0);
        assert_eq!(validate_amm_vote_preflight(&vote, &rules), Ter::TEM_BAD_MPT);

        let mut bid = amm_shell(TxType::AMM_BID, 0);
        set_bad_pool_asset(&mut bid);
        assert_eq!(validate_amm_bid_preflight(&bid, &rules), Ter::TEM_BAD_MPT);
    }

    #[test]
    fn nftoken_mint_preflight_matches_current_rippled_flag_and_fee_rules() {
        let rules = current_nft_rules();
        assert_eq!(
            validate_sttx_transaction_preflight_with_rules(&nftoken_mint(1_048_576, None), &rules),
            Ter::TEM_INVALID_FLAG,
            "tfTransferFee is not an NFTokenMint transaction flag",
        );
        assert_eq!(
            validate_sttx_transaction_preflight_with_rules(
                &nftoken_mint(1 | 2 | 4 | 1_048_576, None),
                &rules
            ),
            Ter::TEM_INVALID_FLAG,
            "deprecated tfTrustLine and unknown bits must be rejected",
        );
        assert_eq!(
            validate_sttx_transaction_preflight_with_rules(&nftoken_mint(4, None), &rules),
            Ter::TEM_INVALID_FLAG,
            "fixRemoveNFTokenAutoTrustLine rejects tfTrustLine",
        );
        assert_eq!(
            validate_sttx_transaction_preflight_with_rules(&nftoken_mint(8, Some(60_000)), &rules),
            Ter::TEM_BAD_NFTOKEN_TRANSFER_FEE,
        );
    }

    #[test]
    fn offer_create_preflight_is_not_a_success_noop() {
        let rules = Rules::new(std::iter::empty());
        assert_eq!(
            validate_sttx_transaction_preflight_with_rules(
                &offer_create(protocol::tfImmediateOrCancel | protocol::tfFillOrKill),
                &rules,
            ),
            Ter::TEM_INVALID_FLAG,
        );

        let mut zero_expiration = offer_create(0);
        zero_expiration.set_field_u32(sf("sfExpiration"), 0);
        assert_eq!(
            validate_sttx_transaction_preflight_with_rules(&zero_expiration, &rules),
            Ter::TEM_BAD_EXPIRATION,
        );

        let mut zero_cancel_sequence = offer_create(0);
        zero_cancel_sequence.set_field_u32(sf("sfOfferSequence"), 0);
        assert_eq!(
            validate_sttx_transaction_preflight_with_rules(&zero_cancel_sequence, &rules),
            Ter::TEM_BAD_SEQUENCE,
        );
    }

    #[test]
    fn typed_flag_masks_precede_bad_fee_like_rippled_preflight1() {
        let rules = Rules::new(std::iter::empty());
        let fee_issuer = AccountID::from_array([0xF9; 20]);

        let mut payment = payment_self_tx(
            STAmount::from_xrp_amount(XRPAmount::from_drops(1)),
            None,
            false,
        );
        payment.set_field_u32(sf("sfFlags"), protocol::tfSell);
        payment.set_field_amount(sf("sfFee"), iou(sf("sfFee"), fee_issuer, 0x31));
        assert_eq!(
            validate_sttx_transaction_preflight_with_rules(&payment, &rules),
            Ter::TEM_INVALID_FLAG,
            "Payment::getFlagsMask runs before the common Fee check",
        );

        let mut offer = offer_create(protocol::tfHybrid);
        offer.set_field_amount(sf("sfFee"), iou(sf("sfFee"), fee_issuer, 0x32));
        assert_eq!(
            validate_sttx_transaction_preflight_with_rules(&offer, &rules),
            Ter::TEM_INVALID_FLAG,
            "tfHybrid is masked before PermissionedDEX and before the Fee check",
        );

        let mut claim = paychan_claim(protocol::tfFillOrKill, false);
        claim.set_field_amount(sf("sfFee"), iou(sf("sfFee"), fee_issuer, 0x33));
        assert_eq!(
            validate_sttx_transaction_preflight_with_rules(&claim, &rules),
            Ter::TEM_INVALID_FLAG,
            "PaymentChannelClaim uses its typed mask before the Fee check",
        );

        let account_set = STTx::new(TxType::ACCOUNT_SET, |tx| {
            tx.set_account_id(sf("sfAccount"), AccountID::from_array([0xA9; 20]));
            tx.set_field_u32(sf("sfFlags"), 0x0040_0000);
            tx.set_field_amount(sf("sfFee"), iou(sf("sfFee"), fee_issuer, 0x34));
            tx.set_field_u32(sf("sfSequence"), 1);
        });
        assert_eq!(
            validate_sttx_transaction_preflight_with_rules(&account_set, &rules),
            Ter::TEM_INVALID_FLAG,
            "AccountSet uses tfAccountSetMask before the Fee check",
        );
    }

    #[test]
    fn typed_extra_feature_gates_precede_common_preflight() {
        let mut permissioned_offer = offer_create(0);
        permissioned_offer.set_account_id(sf("sfAccount"), AccountID::default());
        permissioned_offer.set_field_h256(sf("sfDomainID"), Uint256::from_u64(1));
        assert_eq!(
            validate_sttx_transaction_preflight_with_rules(
                &permissioned_offer,
                &Rules::new(std::iter::empty()),
            ),
            Ter::TEM_DISABLED,
        );

        let mut disabled_amm = amm_create(0);
        disabled_amm.set_account_id(sf("sfAccount"), AccountID::default());
        assert_eq!(
            validate_sttx_transaction_preflight_with_rules(
                &disabled_amm,
                &Rules::new([protocol::feature_amm()]),
            ),
            Ter::TEM_DISABLED,
        );
    }

    #[test]
    fn nftoken_accept_offer_preflight_is_not_a_success_noop() {
        let rules = Rules::new(std::iter::empty());
        assert_eq!(
            validate_sttx_transaction_preflight_with_rules(
                &nftoken_accept_offer(false, false, None),
                &rules,
            ),
            Ter::TEM_MALFORMED,
        );
        assert_eq!(
            validate_sttx_transaction_preflight_with_rules(
                &nftoken_accept_offer(true, false, Some(1)),
                &rules,
            ),
            Ter::TEM_MALFORMED,
        );
        assert_eq!(
            validate_sttx_transaction_preflight_with_rules(
                &nftoken_accept_offer(true, true, Some(0)),
                &rules,
            ),
            Ter::TEM_MALFORMED,
        );
        assert_eq!(
            validate_sttx_transaction_preflight_with_rules(
                &nftoken_accept_offer(true, true, Some(1)),
                &rules,
            ),
            Ter::TES_SUCCESS,
        );
    }

    #[test]
    fn amm_create_preflight_is_not_a_success_noop() {
        let rules = Rules::new([
            protocol::feature_amm(),
            protocol::feature_universal_number(),
        ]);
        assert_eq!(
            validate_sttx_transaction_preflight_with_rules(&amm_create(1_001), &rules),
            Ter::TEM_BAD_FEE,
        );

        let mut invalid_flags = amm_create(0);
        invalid_flags.set_field_u32(sf("sfFlags"), protocol::tfSell);
        assert_eq!(
            validate_sttx_transaction_preflight_with_rules(&invalid_flags, &rules),
            Ter::TEM_INVALID_FLAG,
        );

        let mut duplicate_assets = amm_create(0);
        duplicate_assets.set_field_amount(
            sf("sfAmount2"),
            STAmount::from_xrp_amount(XRPAmount::from_drops(100)),
        );
        assert_eq!(
            validate_sttx_transaction_preflight_with_rules(&duplicate_assets, &rules),
            Ter::TEM_BAD_AMM_TOKENS,
        );
    }

    #[test]
    fn payment_channel_claim_preflight_is_not_a_success_noop() {
        let rules = Rules::new([protocol::feature_id("fixCleanup3_2_0")]);
        assert_eq!(
            validate_sttx_transaction_preflight_with_rules(
                &paychan_claim(
                    crate::PAYMENT_CHANNEL_CLAIM_CLOSE_FLAG
                        | crate::PAYMENT_CHANNEL_CLAIM_RENEW_FLAG,
                    false,
                ),
                &rules,
            ),
            Ter::TEM_MALFORMED,
        );
        assert_eq!(
            validate_sttx_transaction_preflight_with_rules(
                &paychan_claim(protocol::tfSell, false),
                &rules,
            ),
            Ter::TEM_INVALID_FLAG,
        );
        assert_eq!(
            validate_sttx_transaction_preflight_with_rules(&paychan_claim(0, true), &rules),
            Ter::TEM_BAD_SIGNATURE,
        );
    }

    #[test]
    fn payment_self_preflight_matches_rippled_asset_and_path_rules() {
        let issuer = AccountID::from_array([0xB2; 20]);
        let amount_xrp = STAmount::from_xrp_amount(XRPAmount::from_drops(1));
        let amount_iou = iou(sf("sfAmount"), issuer, 0x55);
        let send_max_iou = iou(sf("sfSendMax"), issuer, 0x55);
        let send_max_xrp = STAmount::from_xrp_amount(XRPAmount::from_drops(1));

        // Cross-currency self payments are Flow candidates, not temREDUNDANT.
        assert_eq!(
            validate_payment_preflight(&payment_self_tx(
                amount_xrp.clone(),
                Some(send_max_iou),
                false
            )),
            Ter::TES_SUCCESS,
            "XRP Amount plus issued SendMax must not be treated as redundant"
        );
        assert_eq!(
            validate_payment_preflight(&payment_self_tx(
                amount_iou.clone(),
                Some(send_max_xrp),
                false
            )),
            Ter::TES_SUCCESS,
            "issued Amount plus XRP SendMax must not be treated as redundant"
        );

        // getMaxSourceAmount changes an IOU's issuer to Account, but
        // equalTokens intentionally compares IOUs by currency only.
        assert_eq!(
            validate_payment_preflight(&payment_self_tx(amount_iou.clone(), None, false)),
            Ter::TEM_REDUNDANT,
            "same-token direct self payment must remain redundant"
        );
        assert_eq!(
            validate_payment_preflight(&payment_self_tx(amount_iou, None, true)),
            Ter::TES_SUCCESS,
            "an explicit path may perform arbitrage and must be admitted"
        );
    }

    #[test]
    fn payment_self_preflight_is_reached_through_the_canonical_dispatcher() {
        let issuer = AccountID::from_array([0xB2; 20]);
        let transaction = payment_self_tx(
            STAmount::from_xrp_amount(XRPAmount::from_drops(1)),
            Some(iou(sf("sfSendMax"), issuer, 0x55)),
            false,
        );

        assert_eq!(
            validate_sttx_transaction_preflight_with_rules(
                &transaction,
                &Rules::new(std::iter::empty()),
            ),
            Ter::TES_SUCCESS,
        );
    }

    #[test]
    fn change_pseudo_preflight_accepts_zero_account_without_signature() {
        assert_eq!(
            validate_sttx_transaction_preflight_with_rules(
                &change_amendment(),
                &Rules::new(std::iter::empty()),
            ),
            Ter::TES_SUCCESS,
        );
    }

    #[test]
    fn change_pseudo_preflight_rejects_signature_material_before_admission() {
        let mut tx = change_amendment();
        tx.set_field_vl(sf("sfTxnSignature"), &[1]);

        assert_eq!(
            validate_sttx_transaction_preflight_with_rules(&tx, &Rules::new(std::iter::empty())),
            Ter::TEM_BAD_SIGNATURE,
        );
    }
}
