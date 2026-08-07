//! Shared stateless semantic-preflight dispatcher for canonical `STTx` values.
//!
//! This is intentionally independent of ledger reads and signature validation.
//! It composes the common transactor checks with transaction-specific semantic
//! helpers, so callers such as Batch can validate canonical inner transactions
//! without depending on the higher-level RPC crate.

use protocol::{
    INNER_BATCH_TRANSACTION_FLAG, NotTec, Permission, Rules, STAmount, STTx, Ter, TxType,
    equal_tokens, feature_lending_protocol, get_field_by_symbol, is_bad_asset, is_tes_success,
};

use crate::{
    ChangePreflightFacts, TransactorPreflight0Facts, run_change_invoke_preflight_for_txn_type,
    run_change_preflight, run_change_preflight_flag_mask, run_transactor_preflight0,
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
        TxType::PAYMENT => validate_payment_preflight(tx),
        TxType::DEPOSIT_PREAUTH => validate_deposit_preauth_preflight(tx, rules),
        TxType::ACCOUNT_DELETE => validate_account_delete_preflight(tx, rules),
        TxType::TICKET_CREATE => crate::run_ticket_create_preflight(
            tx.get_field_u32(get_field_by_symbol("sfTicketCount")),
        ),
        TxType::OFFER_CANCEL => crate::run_offer_cancel_preflight(
            tx.get_field_u32(get_field_by_symbol("sfOfferSequence")),
        ),
        TxType::PAYCHAN_CREATE => validate_payment_channel_create_preflight(tx),
        TxType::PAYCHAN_FUND => validate_payment_channel_fund_preflight(tx),
        TxType::CHECK_CREATE => validate_check_create_preflight(tx),
        TxType::CHECK_CASH => validate_check_cash_preflight(tx),
        TxType::REGULAR_KEY_SET => validate_set_regular_key_preflight(tx),
        TxType::ESCROW_CREATE => validate_escrow_create_preflight(tx, rules),
        TxType::ACCOUNT_SET => validate_account_set_preflight(tx),
        // Types with no additional stateless rule still traverse the standalone
        // dispatcher explicitly; unknown types never silently succeed.
        _ if txn_type.is_dispatchable() => validate_sttx_noop_preflight(txn_type),
        _ => Ter::TEM_UNKNOWN,
    }
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

fn validate_payment_preflight(tx: &STTx) -> NotTec {
    let account = get_field_by_symbol("sfAccount");
    let amount_field = get_field_by_symbol("sfAmount");
    let destination = get_field_by_symbol("sfDestination");
    let deliver_min = get_field_by_symbol("sfDeliverMin");
    if !tx.is_field_present(amount_field) || !tx.is_field_present(destination) {
        return Ter::TEM_MALFORMED;
    }

    let amount = tx.get_field_amount(amount_field);
    let source_account = tx.get_account_id(account);
    let send_max = get_field_by_symbol("sfSendMax");
    let max_source_amount = payment_max_source_amount(
        source_account,
        &amount,
        tx.is_field_present(send_max)
            .then(|| tx.get_field_amount(send_max))
            .as_ref(),
    );
    // Preserve rippled's preflight ordering: bad/zero source or destination
    // amounts and bad assets are rejected before the self-payment predicate.
    if amount.signum() <= 0
        || !amount.is_legal_net()
        || !max_source_amount.is_legal_net()
        || (tx.is_field_present(send_max) && max_source_amount.signum() <= 0)
    {
        return Ter::TEM_BAD_AMOUNT;
    }
    if is_bad_asset(max_source_amount.asset()) || is_bad_asset(amount.asset()) {
        return Ter::TEM_BAD_CURRENCY;
    }

    if let Some(deliver_min) = tx
        .is_field_present(deliver_min)
        .then(|| tx.get_field_amount(deliver_min))
        && (deliver_min.negative()
            || !deliver_min.is_legal_net()
            || deliver_min.asset() != amount.asset())
    {
        return Ter::TEM_BAD_AMOUNT;
    }

    let destination_account = tx.get_account_id(destination);
    if destination_account.is_zero() {
        return Ter::TEM_DST_IS_SRC;
    }

    // Match Payment::preflight's getMaxSourceAmount + equalTokens gate. IOUs
    // compare by currency regardless of issuer, MPTs by issuance ID, and an
    // explicit path is allowed because it may perform arbitrage.
    let has_paths = tx.is_field_present(get_field_by_symbol("sfPaths"));
    if payment_is_redundant_to_self(
        source_account,
        destination_account,
        &max_source_amount,
        &amount,
        has_paths,
    ) {
        return Ter::TEM_REDUNDANT;
    }

    Ter::TES_SUCCESS
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

fn validate_payment_channel_fund_preflight(tx: &STTx) -> NotTec {
    let amount_field = get_field_by_symbol("sfAmount");
    if !tx.is_field_present(amount_field) {
        return Ter::TEM_MALFORMED;
    }

    let amount = tx.get_field_amount(amount_field);
    crate::run_payment_channel_fund_preflight(amount.native(), amount.signum() > 0)
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

    let amount = tx.get_field_amount(amount_field);
    let cancel_after = get_field_by_symbol("sfCancelAfter");
    let finish_after = get_field_by_symbol("sfFinishAfter");
    let condition = get_field_by_symbol("sfCondition");
    let cancel_after_value = tx
        .is_field_present(cancel_after)
        .then(|| tx.get_field_u32(cancel_after));
    let finish_after_value = tx
        .is_field_present(finish_after)
        .then(|| tx.get_field_u32(finish_after));

    crate::run_escrow_create_preflight(crate::EscrowCreatePreflightFacts {
        amount_kind: if amount.native() {
            crate::EscrowCreateAmountKind::Xrp
        } else if amount.holds_mpt_issue() {
            crate::EscrowCreateAmountKind::Mpt
        } else {
            crate::EscrowCreateAmountKind::Issue
        },
        amount_positive: amount.signum() > 0 && amount.is_legal_net(),
        feature_token_escrow_enabled: rules.enabled(&protocol::feature_id("TokenEscrow")),
        feature_mptokens_enabled: rules.enabled(&protocol::feature_id("MPTokensV1")),
        issue_has_bad_currency: amount.holds_issue()
            && amount.issue().currency == protocol::bad_currency(),
        mpt_amount_within_limit: !amount.holds_mpt_issue()
            || amount.mantissa() <= crate::ESCROW_CREATE_MAX_MPTOKEN_AMOUNT,
        cancel_after_present: cancel_after_value.is_some(),
        finish_after_present: finish_after_value.is_some(),
        cancel_after_strictly_after_finish_after: match (cancel_after_value, finish_after_value) {
            (Some(cancel), Some(finish)) => cancel > finish,
            _ => true,
        },
        condition_present: tx.is_field_present(condition),
        condition_valid: !tx.is_field_present(condition) || !tx.get_field_vl(condition).is_empty(),
    })
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
        AccountID, Currency, IOUAmount, Issue, Rules, STAmount, STPathSet, STTx, Ter, TxType,
        XRPAmount, get_field_by_symbol,
    };

    use super::{validate_payment_preflight, validate_sttx_transaction_preflight_with_rules};

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
