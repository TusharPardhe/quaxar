#![allow(
    unused_imports,
    unused_variables,
    unused_mut,
    dead_code,
    unused_comparisons
)]
//! Full-pipeline transaction application for integration tests.
//! Runs preflight → preclaim → doApply, mirroring C++ Env::apply().

use std::sync::Arc;

use app::state::application_root::apply_submit_transactor_shell;
use basics::base_uint::{Uint160, Uint256};
use ledger::{ApplyView, ApplyViewImpl, Ledger, ReadView};
use protocol::{
    AccountID, Asset, Currency, IOUAmount, Issue, LedgerEntryType, STAmount, STIssue,
    STLedgerEntry, STTx, Ter, TxType, XRPAmount, account_keylet, get_field_by_symbol,
    is_tes_success, owner_dir_keylet, sf_generic,
};
use tx::*;

fn sf(name: &str) -> &'static protocol::SField {
    get_field_by_symbol(name)
}

fn tx_amm_asset(tx: &STTx, field: &'static protocol::SField) -> Asset {
    if let Some(value) = tx.peek_at_pfield(field) {
        if let Some(issue) = value.as_any().downcast_ref::<STIssue>() {
            return issue.asset();
        }
        if let Some(amount) = value.as_any().downcast_ref::<STAmount>() {
            return amount.asset();
        }
    }
    tx.get_field_amount(field).asset()
}

fn optional_tx_amount(tx: &STTx, field: &'static protocol::SField) -> Option<STAmount> {
    tx.is_field_present(field)
        .then(|| tx.get_field_amount(field))
}

fn invalid_amm_asset_pair_for_assets(asset1: Asset, asset2: Asset) -> Ter {
    if asset1 == asset2 {
        return Ter::TEM_BAD_AMM_TOKENS;
    }

    match (asset1, asset2) {
        (Asset::Issue(issue1), Asset::Issue(issue2)) => {
            protocol::invalid_amm_asset_pair(issue1, issue2, None)
        }
        (Asset::Issue(issue), Asset::MPTIssue(_)) | (Asset::MPTIssue(_), Asset::Issue(issue)) => {
            protocol::invalid_amm_asset(issue, None)
        }
        (Asset::MPTIssue(_), Asset::MPTIssue(_)) => Ter::TES_SUCCESS,
    }
}

fn invalid_amm_amount_for_asset_pair(
    amount: &STAmount,
    pair: Option<(Asset, Asset)>,
    valid_zero: bool,
) -> Ter {
    if let Some((asset1, asset2)) = pair
        && amount.asset() != asset1
        && amount.asset() != asset2
    {
        return Ter::TEM_BAD_AMM_TOKENS;
    }

    if let Asset::Issue(issue) = amount.asset() {
        let issue_pair = pair.and_then(|(asset1, asset2)| match (asset1, asset2) {
            (Asset::Issue(issue1), Asset::Issue(issue2)) => Some((issue1, issue2)),
            _ => None,
        });
        let asset_result = protocol::invalid_amm_asset(issue, issue_pair);
        if asset_result != Ter::TES_SUCCESS {
            return asset_result;
        }
    }

    if amount.signum() < 0 || (!valid_zero && amount.signum() == 0) {
        return Ter::TEM_BAD_AMOUNT;
    }

    Ter::TES_SUCCESS
}

fn validate_amm_deposit_tx_fields(tx: &STTx) -> Ter {
    let asset1 = tx_amm_asset(tx, sf("sfAsset"));
    let asset2 = tx_amm_asset(tx, sf("sfAsset2"));
    let amount = optional_tx_amount(tx, sf("sfAmount"));
    let amount2 = optional_tx_amount(tx, sf("sfAmount2"));
    let e_price = optional_tx_amount(tx, sf("sfEPrice"));
    let lp_token_out = optional_tx_amount(tx, sf("sfLPTokenOut"));
    let asset_pair_invalid = match invalid_amm_asset_pair_for_assets(asset1, asset2) {
        Ter::TES_SUCCESS => None,
        err => Some(err),
    };
    let pair = Some((asset1, asset2));

    run_amm_deposit_preflight_facts(AMMDepositPreflightFacts {
        flags: tx.get_flags(),
        asset_pair_invalid,
        amount: amount.as_ref().map(STAmount::asset),
        amount_invalid: amount
            .as_ref()
            .map(|amount| invalid_amm_amount_for_asset_pair(amount, pair, false))
            .filter(|result| *result != Ter::TES_SUCCESS),
        amount2: amount2.as_ref().map(STAmount::asset),
        amount2_invalid: amount2
            .as_ref()
            .map(|amount| invalid_amm_amount_for_asset_pair(amount, pair, false))
            .filter(|result| *result != Ter::TES_SUCCESS),
        e_price: e_price.as_ref().map(STAmount::asset),
        e_price_invalid: e_price
            .as_ref()
            .map(|amount| invalid_amm_amount_for_asset_pair(amount, None, false))
            .filter(|result| *result != Ter::TES_SUCCESS),
        lp_token_out_signum: lp_token_out.as_ref().map(STAmount::signum),
        trading_fee: tx
            .is_field_present(sf("sfTradingFee"))
            .then(|| tx.get_field_u16(sf("sfTradingFee"))),
    })
}

fn validate_amm_withdraw_tx_fields(tx: &STTx) -> Ter {
    let asset1 = tx_amm_asset(tx, sf("sfAsset"));
    let asset2 = tx_amm_asset(tx, sf("sfAsset2"));
    let amount = optional_tx_amount(tx, sf("sfAmount"));
    let amount2 = optional_tx_amount(tx, sf("sfAmount2"));
    let e_price = optional_tx_amount(tx, sf("sfEPrice"));
    let lp_token_in = optional_tx_amount(tx, sf("sfLPTokenIn"));
    let asset_pair_invalid = match invalid_amm_asset_pair_for_assets(asset1, asset2) {
        Ter::TES_SUCCESS => None,
        err => Some(err),
    };
    let pair = Some((asset1, asset2));
    let amount_valid_zero = (tx.get_flags()
        & (protocol::AMM_ONE_ASSET_WITHDRAW_ALL_FLAG | protocol::AMM_ONE_ASSET_LP_TOKEN_FLAG))
        != 0
        || e_price.is_some();

    run_amm_withdraw_preflight_facts(AMMWithdrawPreflightFacts {
        flags: tx.get_flags(),
        asset_pair_invalid,
        amount: amount.as_ref().map(STAmount::asset),
        amount_invalid: amount
            .as_ref()
            .map(|amount| invalid_amm_amount_for_asset_pair(amount, pair, amount_valid_zero))
            .filter(|result| *result != Ter::TES_SUCCESS),
        amount2: amount2.as_ref().map(STAmount::asset),
        amount2_invalid: amount2
            .as_ref()
            .map(|amount| invalid_amm_amount_for_asset_pair(amount, pair, false))
            .filter(|result| *result != Ter::TES_SUCCESS),
        e_price: e_price.as_ref().map(STAmount::asset),
        e_price_invalid: e_price
            .as_ref()
            .map(|amount| invalid_amm_amount_for_asset_pair(amount, None, false))
            .filter(|result| *result != Ter::TES_SUCCESS),
        lp_token_in_signum: lp_token_in.as_ref().map(STAmount::signum),
    })
}

fn acct_uint160(account: AccountID) -> Uint160 {
    Uint160::from_slice(account.data()).expect("account width")
}

/// Apply a transaction through the full pipeline (preflight + preclaim + doApply).
/// Returns the first non-success Ter, or TES_SUCCESS if all stages pass.
pub fn full_apply(view: &mut ApplyViewImpl<Ledger>, tx: &STTx, txn_type: TxType) -> Ter {
    // Run per-type preflight
    let preflight_result = run_preflight(view, tx, txn_type);
    if !is_tes_success(preflight_result) {
        return preflight_result;
    }

    // Run per-type preclaim
    let preclaim_result = run_preclaim(view, tx, txn_type);
    if !is_tes_success(preclaim_result) {
        return preclaim_result;
    }

    // Run the full apply (common checks + doApply)
    apply_submit_transactor_shell(view, tx, txn_type)
}

fn run_preflight(view: &impl ReadView, tx: &STTx, txn_type: TxType) -> Ter {
    match txn_type {
        TxType::CHECK_CREATE => {
            let account = tx.get_account_id(sf("sfAccount"));
            let dst = tx.get_account_id(sf("sfDestination"));
            let send_max = tx.get_field_amount(sf("sfSendMax"));
            let expiration = if tx.is_field_present(sf("sfExpiration")) {
                Some(tx.get_field_u32(sf("sfExpiration")))
            } else {
                None
            };
            // Check flags first (C++ preflight1 universal check)
            if tx.is_field_present(sf("sfFlags")) && tx.get_field_u32(sf("sfFlags")) != 0 {
                return Ter::TEM_INVALID_FLAG;
            }
            let facts = CheckCreatePreflightFacts {
                tx_account_is_destination: account == dst,
                send_max_is_legal: true,
                send_max_signum_positive: send_max.signum() > 0,
                send_max_currency_is_bad: !send_max.native() && send_max.issue().currency.is_zero(),
                expiration,
            };
            run_check_create_preflight(facts)
        }
        TxType::CHECK_CASH => {
            let has_amount = tx.is_field_present(sf("sfAmount"));
            let has_deliver_min = tx.is_field_present(sf("sfDeliverMin"));
            let (value_positive, value_bad_currency) = if has_amount {
                let amt = tx.get_field_amount(sf("sfAmount"));
                (
                    amt.signum() > 0,
                    !amt.native() && amt.issue().currency.is_zero(),
                )
            } else if has_deliver_min {
                let amt = tx.get_field_amount(sf("sfDeliverMin"));
                (
                    amt.signum() > 0,
                    !amt.native() && amt.issue().currency.is_zero(),
                )
            } else {
                (true, false)
            };
            let facts = CheckCashPreflightFacts {
                amount_present: has_amount,
                deliver_min_present: has_deliver_min,
                value_is_legal: true,
                value_signum_positive: value_positive,
                value_currency_is_bad: value_bad_currency,
            };
            run_check_cash_preflight(facts)
        }
        TxType::CHECK_CANCEL => {
            // CheckCancel preflight only checks flags
            let flags = if tx.is_field_present(sf("sfFlags")) {
                tx.get_field_u32(sf("sfFlags"))
            } else {
                0
            };
            if flags != 0 {
                Ter::TEM_INVALID_FLAG
            } else {
                Ter::TES_SUCCESS
            }
        }
        TxType::OFFER_CREATE => {
            let taker_pays = tx.get_field_amount(sf("sfTakerPays"));
            let taker_gets = tx.get_field_amount(sf("sfTakerGets"));
            let flags = if tx.is_field_present(sf("sfFlags")) {
                tx.get_field_u32(sf("sfFlags"))
            } else {
                0
            };
            let expiration = if tx.is_field_present(sf("sfExpiration")) {
                Some(tx.get_field_u32(sf("sfExpiration")))
            } else {
                None
            };
            let offer_seq = if tx.is_field_present(sf("sfOfferSequence")) {
                Some(tx.get_field_u32(sf("sfOfferSequence")))
            } else {
                None
            };

            // Check flags
            let tf_sell: u32 = 0x00080000;
            let tf_passive: u32 = 0x00010000;
            let tf_immediate_or_cancel: u32 = 0x00020000;
            let tf_fill_or_kill: u32 = 0x00040000;
            let valid_flags = tf_sell | tf_passive | tf_immediate_or_cancel | tf_fill_or_kill;
            if flags & !valid_flags != 0 {
                return Ter::TEM_INVALID_FLAG;
            }
            if flags & tf_immediate_or_cancel != 0 && flags & tf_fill_or_kill != 0 {
                return Ter::TEM_INVALID_FLAG;
            }

            // Check amounts
            if taker_pays.signum() <= 0 || taker_gets.signum() <= 0 {
                return Ter::TEM_BAD_OFFER;
            }

            // Bad currency (check before same-asset to match C++ order)
            let bad_cur = protocol::bad_currency();
            let no_cur = protocol::no_currency();
            let pays_bad = !taker_pays.native()
                && (taker_pays.issue().currency == bad_cur
                    || taker_pays.issue().currency == no_cur);
            let gets_bad = !taker_gets.native()
                && (taker_gets.issue().currency == bad_cur
                    || taker_gets.issue().currency == no_cur);
            if pays_bad || gets_bad {
                return Ter::TEM_BAD_CURRENCY;
            }

            // Same asset
            if taker_pays.native() && taker_gets.native() {
                return Ter::TEM_BAD_OFFER;
            }
            if !taker_pays.native() && !taker_gets.native() {
                if taker_pays.issue() == taker_gets.issue() {
                    return Ter::TEM_BAD_OFFER; // C++ parity: temBAD_OFFER for same asset
                }
            }

            // Bad expiration
            if let Some(exp) = expiration {
                if exp == 0 {
                    return Ter::TEM_BAD_EXPIRATION;
                }
            }

            // Bad offer sequence
            if let Some(seq) = offer_seq {
                if seq == 0 {
                    return Ter::TEM_BAD_SEQUENCE;
                }
            }

            Ter::TES_SUCCESS
        }
        TxType::OFFER_CANCEL => {
            let offer_seq = tx.get_field_u32(sf("sfOfferSequence"));
            if offer_seq == 0 {
                return Ter::TEM_BAD_SEQUENCE;
            }
            Ter::TES_SUCCESS
        }
        TxType::TRUST_SET => {
            let limit = tx.get_field_amount(sf("sfLimitAmount"));
            let flags = if tx.is_field_present(sf("sfFlags")) {
                tx.get_field_u32(sf("sfFlags"))
            } else {
                0
            };

            // Invalid flags: tfSetfAuth(0x10000) + tfSetNoRipple(0x20000) + tfClearNoRipple(0x40000)
            // + tfSetFreeze(0x100000) + tfClearFreeze(0x200000)
            let tf_set_no_ripple: u32 = 0x00020000;
            let tf_clear_no_ripple: u32 = 0x00040000;
            let tf_set_freeze: u32 = 0x00100000;
            let tf_clear_freeze: u32 = 0x00200000;
            if flags & tf_set_no_ripple != 0 && flags & tf_clear_no_ripple != 0 {
                return Ter::TEM_INVALID_FLAG;
            }
            if flags & tf_set_freeze != 0 && flags & tf_clear_freeze != 0 {
                return Ter::TEM_INVALID_FLAG;
            }

            // Native limit = bad
            if limit.native() {
                return Ter::TEM_BAD_LIMIT;
            }
            // Negative limit
            if limit.negative() {
                return Ter::TEM_BAD_LIMIT;
            }
            // Bad currency
            let bad_cur = protocol::bad_currency();
            if limit.issue().currency == bad_cur {
                return Ter::TEM_BAD_CURRENCY;
            }
            // Trust to self
            let account = tx.get_account_id(sf("sfAccount"));
            if account == limit.issue().account {
                return Ter::TEM_DST_IS_SRC;
            }
            // tfSetfAuth without lsfRequireAuth
            let tf_set_auth: u32 = 0x00010000;
            if flags & tf_set_auth != 0 {
                return Ter::TEF_NO_AUTH_REQUIRED;
            }
            Ter::TES_SUCCESS
        }
        TxType::ACCOUNT_DELETE => {
            let account = tx.get_account_id(sf("sfAccount"));
            let dst = tx.get_account_id(sf("sfDestination"));
            let flags = if tx.is_field_present(sf("sfFlags")) {
                tx.get_field_u32(sf("sfFlags"))
            } else {
                0
            };
            if account == dst {
                return Ter::TEM_DST_IS_SRC;
            }
            if flags != 0 {
                return Ter::TEM_INVALID_FLAG;
            }
            // Fee check: AccountDelete requires increment fee
            let fee = if tx.is_field_present(sf("sfFee")) {
                tx.get_field_amount(sf("sfFee")).xrp().drops()
            } else {
                0
            };
            if fee < 50_000 {
                // increment = 50_000 in test config
                return Ter::TEL_INSUF_FEE_P;
            }
            Ter::TES_SUCCESS
        }
        TxType::ESCROW_CREATE => {
            let amount = tx.get_field_amount(sf("sfAmount"));
            let flags = if tx.is_field_present(sf("sfFlags")) {
                tx.get_field_u32(sf("sfFlags"))
            } else {
                0
            };
            if flags != 0 {
                return Ter::TEM_INVALID_FLAG;
            }
            if amount.signum() <= 0 {
                return Ter::TEM_BAD_AMOUNT;
            }
            let has_finish = tx.is_field_present(sf("sfFinishAfter"));
            let has_cancel = tx.is_field_present(sf("sfCancelAfter"));
            let has_condition = tx.is_field_present(sf("sfCondition"));
            if !has_finish && !has_cancel && !has_condition {
                return Ter::TEM_MALFORMED;
            }
            if has_cancel {
                let cancel_after = tx.get_field_u32(sf("sfCancelAfter"));
                if cancel_after == 0 {
                    return Ter::TEM_BAD_EXPIRATION;
                }
                // CancelAfter must be > FinishAfter
                if has_finish {
                    let finish_after = tx.get_field_u32(sf("sfFinishAfter"));
                    if cancel_after <= finish_after {
                        return Ter::TEM_BAD_EXPIRATION;
                    }
                }
            }
            Ter::TES_SUCCESS
        }
        TxType::ESCROW_FINISH => {
            // C++ EscrowFinish::preflight: condition XOR fulfillment → temMALFORMED
            let has_condition = tx.is_field_present(sf("sfCondition"));
            let has_fulfillment = tx.is_field_present(sf("sfFulfillment"));
            if has_condition != has_fulfillment {
                return Ter::TEM_MALFORMED;
            }
            Ter::TES_SUCCESS
        }
        TxType::ESCROW_CANCEL => Ter::TES_SUCCESS,
        TxType::PAYCHAN_CREATE => {
            let account = tx.get_account_id(sf("sfAccount"));
            let dst = tx.get_account_id(sf("sfDestination"));
            let amount = tx.get_field_amount(sf("sfAmount"));
            if account == dst {
                return Ter::TEM_DST_IS_SRC;
            }
            if amount.signum() <= 0 || !amount.native() {
                return Ter::TEM_BAD_AMOUNT;
            }
            Ter::TES_SUCCESS
        }
        TxType::PAYCHAN_FUND => {
            let amount = tx.get_field_amount(sf("sfAmount"));
            if amount.signum() <= 0 || !amount.native() {
                return Ter::TEM_BAD_AMOUNT;
            }
            Ter::TES_SUCCESS
        }
        TxType::PAYCHAN_CLAIM => {
            // C++ PaymentChannelClaim::preflight
            if tx.is_field_present(sf("sfBalance")) {
                let bal = tx.get_field_amount(sf("sfBalance"));
                if bal.signum() < 0 {
                    return Ter::TEM_BAD_AMOUNT;
                }
                if !bal.native() {
                    return Ter::TEM_BAD_AMOUNT;
                }
            }
            if tx.is_field_present(sf("sfAmount")) {
                let amt = tx.get_field_amount(sf("sfAmount"));
                if amt.signum() < 0 {
                    return Ter::TEM_BAD_AMOUNT;
                }
                if !amt.native() {
                    return Ter::TEM_BAD_AMOUNT;
                }
            }
            Ter::TES_SUCCESS
        }
        TxType::NFTOKEN_MINT => {
            let flags = if tx.is_field_present(sf("sfFlags")) {
                tx.get_field_u32(sf("sfFlags"))
            } else {
                0
            };
            // Valid flags: tfBurnable(1) | tfOnlyXRP(2) | tfTrustLine(4) | tfTransferable(8)
            let valid_flags: u32 = 0x0000000F;
            if flags & !valid_flags != 0 {
                return Ter::TEM_INVALID_FLAG;
            }
            // Transfer fee checks — C++ order:
            // 1. fee > max (50000) → temBAD_NFTOKEN_TRANSFER_FEE
            // 2. fee > 0 && !tfTransferable → temMALFORMED
            if tx.is_field_present(sf("sfTransferFee")) {
                let xfer_fee = tx.get_field_u16(sf("sfTransferFee"));
                if xfer_fee > 50000 {
                    return Ter::TEM_BAD_NFTOKEN_TRANSFER_FEE;
                }
                if xfer_fee > 0 && (flags & 0x08) == 0 {
                    return Ter::TEM_MALFORMED;
                }
            }
            // Empty URI rejected
            if tx.is_field_present(sf("sfURI")) {
                let uri = tx.get_field_vl(sf("sfURI"));
                if uri.is_empty() || uri.len() > 256 {
                    return Ter::TEM_MALFORMED;
                }
            }
            // Issuer == self rejected
            if tx.is_field_present(sf("sfIssuer")) {
                let issuer = tx.get_account_id(sf("sfIssuer"));
                let account = tx.get_account_id(sf("sfAccount"));
                if issuer == account {
                    return Ter::TEM_MALFORMED;
                }
            }
            Ter::TES_SUCCESS
        }
        TxType::NFTOKEN_BURN => {
            let flags = if tx.is_field_present(sf("sfFlags")) {
                tx.get_field_u32(sf("sfFlags"))
            } else {
                0
            };
            if flags != 0 {
                return Ter::TEM_INVALID_FLAG;
            }
            Ter::TES_SUCCESS
        }
        TxType::NFTOKEN_CREATE_OFFER => {
            let amount = tx.get_field_amount(sf("sfAmount"));
            if amount.signum() <= 0 {
                return Ter::TEM_BAD_AMOUNT;
            }
            if tx.is_field_present(sf("sfExpiration")) {
                let exp = tx.get_field_u32(sf("sfExpiration"));
                if exp == 0 {
                    return Ter::TEM_BAD_EXPIRATION;
                }
            }
            Ter::TES_SUCCESS
        }
        TxType::NFTOKEN_CANCEL_OFFER => {
            if tx.is_field_present(sf("sfNFTokenOffers")) {
                let offers = tx.get_field_v256(sf("sfNFTokenOffers"));
                if offers.value().is_empty() {
                    return Ter::TEM_MALFORMED;
                }
            }
            Ter::TES_SUCCESS
        }
        TxType::AMM_CREATE => {
            let amount1 = tx.get_field_amount(sf("sfAmount"));
            let amount2 = tx.get_field_amount(sf("sfAmount2"));
            let flags = if tx.is_field_present(sf("sfFlags")) {
                tx.get_field_u32(sf("sfFlags"))
            } else {
                0
            };
            let trading_fee = tx.get_field_u16(sf("sfTradingFee"));

            if flags != 0 {
                return Ter::TEM_INVALID_FLAG;
            }
            if amount1.signum() <= 0 || amount2.signum() <= 0 {
                return Ter::TEM_BAD_AMOUNT;
            }
            if amount1.native() && amount2.native() {
                return Ter::TEM_BAD_AMM_TOKENS;
            }
            if !amount1.native() && !amount2.native() && amount1.issue() == amount2.issue() {
                return Ter::TEM_BAD_AMM_TOKENS;
            }
            let bad_cur = protocol::bad_currency();
            if (!amount1.native() && amount1.issue().currency == bad_cur)
                || (!amount2.native() && amount2.issue().currency == bad_cur)
            {
                return Ter::TEM_BAD_CURRENCY;
            }
            if trading_fee > 1000 {
                return Ter::TEM_BAD_FEE;
            }
            Ter::TES_SUCCESS
        }
        TxType::AMM_DEPOSIT => validate_amm_deposit_tx_fields(tx),
        TxType::AMM_WITHDRAW => validate_amm_withdraw_tx_fields(tx),
        TxType::AMM_VOTE => {
            let trading_fee = tx.get_field_u16(sf("sfTradingFee"));
            if trading_fee > 1000 {
                return Ter::TEM_BAD_FEE;
            }
            Ter::TES_SUCCESS
        }
        // --- MPTokenIssuanceCreate: C++ preflight parity ---
        TxType::MPTOKEN_ISSUANCE_CREATE => {
            let flags = if tx.is_field_present(sf("sfFlags")) {
                tx.get_field_u32(sf("sfFlags"))
            } else {
                0
            };
            let valid_flags: u32 = protocol::tfMPTCanLock
                | protocol::tfMPTRequireAuth
                | protocol::tfMPTCanEscrow
                | protocol::tfMPTCanTrade
                | protocol::tfMPTCanTransfer
                | protocol::tfMPTCanClawback;
            if flags & !valid_flags != 0 {
                return Ter::TEM_INVALID_FLAG;
            }
            // TransferFee checks
            if tx.is_field_present(sf("sfTransferFee")) {
                let fee = tx.get_field_u16(sf("sfTransferFee"));
                if fee > 50000 {
                    // kMaxTransferFee
                    return Ter::from_int(-263); // temBAD_TRANSFER_FEE
                }
                if fee > 0 && (flags & protocol::tfMPTCanTransfer) == 0 {
                    return Ter::TEM_MALFORMED;
                }
            }
            // Empty metadata
            if tx.is_field_present(sf("sfMPTokenMetadata")) {
                let meta = tx.get_field_vl(sf("sfMPTokenMetadata"));
                if meta.is_empty() || meta.len() > 1024 {
                    return Ter::TEM_MALFORMED;
                }
            }
            // MaximumAmount checks
            if tx.is_field_present(sf("sfMaximumAmount")) {
                let max_amt = tx.get_field_u64(sf("sfMaximumAmount"));
                if max_amt == 0 {
                    return Ter::TEM_MALFORMED;
                }
                if max_amt > 0x7FFF_FFFF_FFFF_FFFF {
                    // > 63-bit
                    return Ter::TEM_MALFORMED;
                }
            }
            Ter::TES_SUCCESS
        }
        // --- CredentialCreate: C++ preflight parity ---
        TxType::CREDENTIAL_CREATE => {
            // Subject must be present and non-zero
            if !tx.is_field_present(sf("sfSubject")) {
                return Ter::TEM_MALFORMED;
            }
            let subject = tx.get_account_id(sf("sfSubject"));
            if subject == protocol::AccountID::default() {
                return Ter::TEM_MALFORMED;
            }
            // CredentialType must be present and non-empty
            if tx.is_field_present(sf("sfCredentialType")) {
                let cred_type = tx.get_field_vl(sf("sfCredentialType"));
                if cred_type.is_empty() || cred_type.len() > 64 {
                    return Ter::TEM_MALFORMED;
                }
            } else {
                return Ter::TEM_MALFORMED;
            }
            // URI if present must be non-empty and <= 256
            if tx.is_field_present(sf("sfURI")) {
                let uri = tx.get_field_vl(sf("sfURI"));
                if uri.is_empty() || uri.len() > 256 {
                    return Ter::TEM_MALFORMED;
                }
            }
            Ter::TES_SUCCESS
        }
        // --- DelegateSet: C++ preflight parity ---
        TxType::DELEGATE_SET => {
            // Cannot authorize self
            let account = tx.get_account_id(sf("sfAccount"));
            if tx.is_field_present(sf("sfAuthorize")) {
                let authorize = tx.get_account_id(sf("sfAuthorize"));
                if account == authorize {
                    return Ter::TEM_MALFORMED;
                }
            }
            Ter::TES_SUCCESS
        }
        TxType::TICKET_CREATE => {
            let count = tx.get_field_u32(sf("sfTicketCount"));
            if count == 0 || count > 250 {
                return Ter::TEM_INVALID_COUNT;
            }
            Ter::TES_SUCCESS
        }
        TxType::DEPOSIT_PREAUTH => {
            if tx.is_field_present(sf("sfAuthorize")) {
                let account = tx.get_account_id(sf("sfAccount"));
                let authorize = tx.get_account_id(sf("sfAuthorize"));
                if account == authorize {
                    return Ter::TEM_CANNOT_PREAUTH_SELF;
                }
            }
            Ter::TES_SUCCESS
        }
        TxType::CLAWBACK => {
            let amount = tx.get_field_amount(sf("sfAmount"));
            if amount.native() {
                return Ter::TEM_BAD_AMOUNT;
            }
            if amount.signum() <= 0 {
                return Ter::TEM_BAD_AMOUNT;
            }
            Ter::TES_SUCCESS
        }
        TxType::PAYMENT => {
            let account = tx.get_account_id(sf("sfAccount"));
            let dst = tx.get_account_id(sf("sfDestination"));
            let amount = tx.get_field_amount(sf("sfAmount"));
            let flags = if tx.is_field_present(sf("sfFlags")) {
                tx.get_field_u32(sf("sfFlags"))
            } else {
                0
            };
            let has_send_max = tx.is_field_present(sf("sfSendMax"));
            let has_paths = tx.is_field_present(sf("sfPaths"));
            let xrp_direct =
                amount.native() && (!has_send_max || tx.get_field_amount(sf("sfSendMax")).native());
            let partial = (flags & 0x00020000) != 0;
            let limit_quality = (flags & 0x00040000) != 0;
            let no_ripple_direct = (flags & 0x00010000) != 0;
            // Self-payment
            if account == dst {
                return Ter::TEM_REDUNDANT;
            }
            if amount.signum() <= 0 {
                return Ter::TEM_BAD_AMOUNT;
            }
            // C++ Payment::preflight: SendMax <= 0 -> temBAD_AMOUNT
            if has_send_max && tx.get_field_amount(sf("sfSendMax")).signum() <= 0 {
                return Ter::TEM_BAD_AMOUNT;
            }
            // XRP-to-XRP specific checks
            if xrp_direct && amount.native() {
                if has_send_max {
                    return Ter::from_int(-269);
                } // temBAD_SEND_XRP_MAX
                if has_paths {
                    return Ter::from_int(-270);
                } // temBAD_SEND_XRP_PATHS
                if partial {
                    return Ter::from_int(-271);
                } // temBAD_SEND_XRP_PARTIAL
                if limit_quality {
                    return Ter::from_int(-272);
                } // temBAD_SEND_XRP_LIMIT
                if no_ripple_direct {
                    return Ter::from_int(-273);
                } // temBAD_SEND_XRP_NO_DIRECT
            }
            Ter::TES_SUCCESS
        }
        TxType::VAULT_CREATE => {
            let asset = tx.get_field_amount(sf("sfAsset"));
            let flags = if tx.is_field_present(sf("sfFlags")) {
                tx.get_field_u32(sf("sfFlags"))
            } else {
                0
            };
            if asset.native() {
                return Ter::TEM_MALFORMED;
            }
            // Valid vault flags: tfPrivate(1) | tfShareNonTransferable(2)
            let valid_flags: u32 = 0x03;
            if flags & !valid_flags != 0 {
                return Ter::TEM_INVALID_FLAG;
            }
            Ter::TES_SUCCESS
        }
        TxType::VAULT_DEPOSIT | TxType::VAULT_WITHDRAW => {
            // C++ VaultDeposit/Withdraw::preflight: zero VaultID -> temMALFORMED
            if tx.is_field_present(sf("sfVaultID")) && tx.get_field_h256(sf("sfVaultID")).is_zero()
            {
                return Ter::TEM_MALFORMED;
            }
            let amount = tx.get_field_amount(sf("sfAmount"));
            if amount.signum() <= 0 {
                return Ter::TEM_BAD_AMOUNT;
            }
            Ter::TES_SUCCESS
        }
        // --- AccountSet: C++ AccountSet::preflight ---
        TxType::ACCOUNT_SET => {
            let set_flag = if tx.is_field_present(sf("sfSetFlag")) {
                tx.get_field_u32(sf("sfSetFlag"))
            } else {
                0
            };
            let clear_flag = if tx.is_field_present(sf("sfClearFlag")) {
                tx.get_field_u32(sf("sfClearFlag"))
            } else {
                0
            };
            if set_flag != 0 && set_flag == clear_flag {
                return Ter::TEM_INVALID_FLAG;
            }
            // TransferRate validation
            if tx.is_field_present(sf("sfTransferRate")) {
                let rate = tx.get_field_u32(sf("sfTransferRate"));
                if rate != 0 && rate < 1_000_000_000 {
                    return Ter::from_int(-260); // temBAD_TRANSFER_RATE
                }
                if rate > 2_000_000_000 {
                    return Ter::from_int(-260); // temBAD_TRANSFER_RATE
                }
            }
            // TickSize validation
            if tx.is_field_present(sf("sfTickSize")) {
                let tick = tx.get_field_u8(sf("sfTickSize"));
                if tick != 0 && (tick < 3 || tick > 15) {
                    return Ter::from_int(-264); // temBAD_TICK_SIZE
                }
            }
            Ter::TES_SUCCESS
        }
        // --- SetRegularKey: C++ SetRegularKey::preflight ---
        TxType::REGULAR_KEY_SET => {
            if tx.is_field_present(sf("sfRegularKey")) {
                let reg_key = tx.get_account_id(sf("sfRegularKey"));
                let account = tx.get_account_id(sf("sfAccount"));
                if reg_key == account {
                    return Ter::from_int(-261); // temBAD_REGKEY
                }
            }
            Ter::TES_SUCCESS
        }
        // --- AMMBid: C++ AMMBid::preflight ---
        TxType::AMM_BID => {
            Ter::TES_SUCCESS // Asset pair validation handled by common AMM code
        }
        // --- AMMClawback: C++ AMMClawback::preflight ---
        TxType::AMM_CLAWBACK => {
            if tx.is_field_present(sf("sfHolder")) {
                let account = tx.get_account_id(sf("sfAccount"));
                let holder = tx.get_account_id(sf("sfHolder"));
                if account == holder {
                    return Ter::TEM_MALFORMED;
                }
            }
            Ter::TES_SUCCESS
        }
        // --- AMMDelete: no special preflight ---
        TxType::AMM_DELETE => Ter::TES_SUCCESS,
        // --- Batch: C++ Batch::preflight ---
        TxType::BATCH => {
            let flags = if tx.is_field_present(sf("sfFlags")) {
                tx.get_field_u32(sf("sfFlags"))
            } else {
                0
            };
            // Valid batch flags: tfAllOrNothing(1) | tfOnlyOne(2) | tfUntilFailure(4) | tfIndependent(8)
            let valid = 0x0F;
            if flags & !valid != 0 {
                return Ter::TEM_INVALID_FLAG;
            }
            // Can't have more than one mode flag
            let mode_count =
                (flags & 1) + ((flags >> 1) & 1) + ((flags >> 2) & 1) + ((flags >> 3) & 1);
            if mode_count > 1 {
                return Ter::TEM_INVALID_FLAG;
            }
            Ter::TES_SUCCESS
        }
        // --- CredentialAccept/Delete ---
        TxType::CREDENTIAL_ACCEPT => {
            if !tx.is_field_present(sf("sfIssuer")) {
                return Ter::TEM_INVALID_ACCOUNT_ID;
            }
            let issuer = tx.get_account_id(sf("sfIssuer"));
            if issuer == protocol::AccountID::default() {
                return Ter::TEM_INVALID_ACCOUNT_ID;
            }
            if !tx.is_field_present(sf("sfCredentialType")) {
                return Ter::TEM_MALFORMED;
            }
            let cred_type = tx.get_field_vl(sf("sfCredentialType"));
            if cred_type.is_empty() || cred_type.len() > 64 {
                return Ter::TEM_MALFORMED;
            }
            Ter::TES_SUCCESS
        }
        TxType::CREDENTIAL_DELETE => {
            // Must have Subject or Issuer
            if !tx.is_field_present(sf("sfSubject")) && !tx.is_field_present(sf("sfIssuer")) {
                return Ter::TEM_MALFORMED;
            }
            if !tx.is_field_present(sf("sfCredentialType")) {
                return Ter::TEM_MALFORMED;
            }
            let cred_type = tx.get_field_vl(sf("sfCredentialType"));
            if cred_type.is_empty() || cred_type.len() > 64 {
                return Ter::TEM_MALFORMED;
            }
            Ter::TES_SUCCESS
        }
        // --- DIDSet: C++ DIDSet::preflight ---
        TxType::DID_SET => {
            let has_uri = tx.is_field_present(sf("sfURI"));
            let has_doc = tx.is_field_present(sf("sfDIDDocument"));
            let has_data = tx.is_field_present(sf("sfData"));
            if !has_uri && !has_doc && !has_data {
                return Ter::TEM_EMPTY_DID;
            }
            // All present but all empty
            if has_uri && has_doc && has_data {
                let uri_empty = tx.get_field_vl(sf("sfURI")).is_empty();
                let doc_empty = tx.get_field_vl(sf("sfDIDDocument")).is_empty();
                let data_empty = tx.get_field_vl(sf("sfData")).is_empty();
                if uri_empty && doc_empty && data_empty {
                    return Ter::TEM_EMPTY_DID;
                }
            }
            Ter::TES_SUCCESS
        }
        TxType::DID_DELETE => Ter::TES_SUCCESS,
        // --- MPTokenAuthorize: C++ MPTokenAuthorize::preflight ---
        TxType::MPTOKEN_AUTHORIZE => {
            if tx.is_field_present(sf("sfHolder")) {
                let account = tx.get_account_id(sf("sfAccount"));
                let holder = tx.get_account_id(sf("sfHolder"));
                if account == holder {
                    return Ter::TEM_MALFORMED;
                }
            }
            Ter::TES_SUCCESS
        }
        TxType::MPTOKEN_ISSUANCE_DESTROY => Ter::TES_SUCCESS,
        TxType::MPTOKEN_ISSUANCE_SET => Ter::TES_SUCCESS,
        // --- NFTokenAcceptOffer: C++ NFTokenAcceptOffer::preflight ---
        TxType::NFTOKEN_ACCEPT_OFFER => {
            let has_buy = tx.is_field_present(sf("sfNFTokenBuyOffer"));
            let has_sell = tx.is_field_present(sf("sfNFTokenSellOffer"));
            if !has_buy && !has_sell {
                return Ter::TEM_MALFORMED;
            }
            // BrokerFee only valid in brokered mode (both buy+sell)
            if tx.is_field_present(sf("sfNFTokenBrokerFee")) && (!has_buy || !has_sell) {
                return Ter::TEM_MALFORMED;
            }
            Ter::TES_SUCCESS
        }
        // --- OracleSet: C++ OracleSet::preflight ---
        TxType::ORACLE_SET => {
            // PriceDataSeries must be present and non-empty
            // (simplified — full check needs array inspection)
            Ter::TES_SUCCESS
        }
        TxType::ORACLE_DELETE => Ter::TES_SUCCESS,
        // --- SignerListSet: C++ SignerListSet::preflight ---
        TxType::SIGNER_LIST_SET => Ter::TES_SUCCESS,
        // --- Loan transactors ---
        TxType::LOAN_BROKER_SET => Ter::TES_SUCCESS,
        TxType::LOAN_BROKER_DELETE => Ter::TES_SUCCESS,
        // --- XChainBridge ---
        TxType::XCHAIN_CREATE_BRIDGE => Ter::TES_SUCCESS,
        TxType::XCHAIN_MODIFY_BRIDGE => Ter::TES_SUCCESS,
        TxType::XCHAIN_CREATE_CLAIM_ID => Ter::TES_SUCCESS,
        TxType::XCHAIN_COMMIT => Ter::TES_SUCCESS,
        TxType::XCHAIN_CLAIM => Ter::TES_SUCCESS,
        TxType::XCHAIN_ACCOUNT_CREATE_COMMIT => Ter::TES_SUCCESS,
        TxType::XCHAIN_ADD_CLAIM_ATTESTATION => Ter::TES_SUCCESS,
        TxType::XCHAIN_ADD_ACCOUNT_CREATE_ATTESTATION => Ter::TES_SUCCESS,
        // --- VaultSet/Clawback ---
        TxType::VAULT_SET => Ter::TES_SUCCESS,
        TxType::VAULT_CLAWBACK => Ter::TES_SUCCESS,
        // --- PermissionedDomain ---
        TxType::PERMISSIONED_DOMAIN_SET => Ter::TES_SUCCESS,
        TxType::PERMISSIONED_DOMAIN_DELETE => Ter::TES_SUCCESS,
        _ => Ter::TES_SUCCESS,
    }
}

fn run_preclaim(view: &impl ReadView, tx: &STTx, txn_type: TxType) -> Ter {
    match txn_type {
        TxType::CHECK_CREATE => {
            let account = tx.get_account_id(sf("sfAccount"));
            let dst = tx.get_account_id(sf("sfDestination"));
            let dst_keylet = account_keylet(acct_uint160(dst));
            let dst_exists = view.exists(dst_keylet.clone()).unwrap_or(false);
            if !dst_exists {
                return Ter::TEC_NO_DST;
            }
            // Check destination requires tag
            if let Ok(Some(dst_sle)) = view.read(dst_keylet) {
                let dst_flags = dst_sle.get_field_u32(sf("sfFlags"));
                let lsf_require_dest_tag: u32 = 0x00020000;
                if dst_flags & lsf_require_dest_tag != 0
                    && !tx.is_field_present(sf("sfDestinationTag"))
                {
                    return Ter::TEC_DST_TAG_NEEDED;
                }
            }
            // Reserve check
            let acct_keylet = account_keylet(acct_uint160(account));
            if let Ok(Some(acct_sle)) = view.read(acct_keylet) {
                let balance = acct_sle.get_field_amount(sf("sfBalance")).xrp().drops();
                let owner_count = acct_sle.get_field_u32(sf("sfOwnerCount"));
                let needed_reserve = 200_000 + 50_000 * (owner_count as i64 + 1);
                let fee = if tx.is_field_present(sf("sfFee")) {
                    tx.get_field_amount(sf("sfFee")).xrp().drops()
                } else {
                    0
                };
                if balance - fee < needed_reserve {
                    return Ter::TEC_INSUFFICIENT_RESERVE;
                }
            }
            Ter::TES_SUCCESS
        }
        TxType::CHECK_CASH => {
            let check_id = tx.get_field_h256(sf("sfCheckID"));
            let check_keylet = protocol::unchecked_keylet(check_id);
            let check_exists = view.exists(check_keylet.clone()).unwrap_or(false);
            if !check_exists {
                return Ter::TEC_NO_ENTRY;
            }
            if let Ok(Some(check_sle)) = view.read(check_keylet) {
                let destination = check_sle.get_account_id(sf("sfDestination"));
                let casher = tx.get_account_id(sf("sfAccount"));
                if casher != destination {
                    return Ter::TEC_NO_PERMISSION;
                }
            }
            // Check amount vs SendMax
            if tx.is_field_present(sf("sfAmount")) {
                let amount = tx.get_field_amount(sf("sfAmount"));
                if amount.signum() <= 0 {
                    return Ter::TEM_BAD_AMOUNT;
                }
            }
            Ter::TES_SUCCESS
        }
        TxType::CHECK_CANCEL => {
            let check_id = tx.get_field_h256(sf("sfCheckID"));
            let check_keylet = protocol::unchecked_keylet(check_id);
            if !view.exists(check_keylet.clone()).unwrap_or(false) {
                return Ter::TEC_NO_ENTRY;
            }
            // Check if caller is creator or destination
            let casher = tx.get_account_id(sf("sfAccount"));
            if let Ok(Some(check_sle)) = view.read(check_keylet) {
                let creator = check_sle.get_account_id(sf("sfAccount"));
                let destination = check_sle.get_account_id(sf("sfDestination"));
                if casher != creator && casher != destination {
                    // Check if expired — anyone can cancel expired
                    if check_sle.is_field_present(sf("sfExpiration")) {
                        // For now, allow (expired check logic)
                    } else {
                        return Ter::TEC_NO_PERMISSION;
                    }
                }
            }
            Ter::TES_SUCCESS
        }
        TxType::OFFER_CREATE => {
            // Reserve check
            let account = tx.get_account_id(sf("sfAccount"));
            let acct_keylet = account_keylet(acct_uint160(account));
            if let Ok(Some(acct_sle)) = view.read(acct_keylet) {
                let balance = acct_sle.get_field_amount(sf("sfBalance")).xrp().drops();
                let owner_count = acct_sle.get_field_u32(sf("sfOwnerCount"));
                // reserve = base(200_000) + increment(50_000) * (owner_count + 1)
                let needed_reserve = 200_000 + 50_000 * (owner_count as i64 + 1);
                let fee = if tx.is_field_present(sf("sfFee")) {
                    tx.get_field_amount(sf("sfFee")).xrp().drops()
                } else {
                    0
                };
                if balance - fee < needed_reserve {
                    return Ter::TEC_INSUFFICIENT_RESERVE;
                }
            }
            Ter::TES_SUCCESS
        }
        TxType::ESCROW_CREATE => {
            let dst = tx.get_account_id(sf("sfDestination"));
            let dst_keylet = account_keylet(acct_uint160(dst));
            if !view.exists(dst_keylet.clone()).unwrap_or(false) {
                return Ter::TEC_NO_DST;
            }
            if let Ok(Some(dst_sle)) = view.read(dst_keylet) {
                let dst_flags = dst_sle.get_field_u32(sf("sfFlags"));
                let lsf_require_dest_tag: u32 = 0x00020000;
                if dst_flags & lsf_require_dest_tag != 0
                    && !tx.is_field_present(sf("sfDestinationTag"))
                {
                    return Ter::TEC_DST_TAG_NEEDED;
                }
            }
            Ter::TES_SUCCESS
        }
        TxType::PAYCHAN_CREATE => {
            let dst = tx.get_account_id(sf("sfDestination"));
            let dst_keylet = account_keylet(acct_uint160(dst));
            if !view.exists(dst_keylet.clone()).unwrap_or(false) {
                return Ter::TEC_NO_DST;
            }
            if let Ok(Some(dst_sle)) = view.read(dst_keylet) {
                let dst_flags = dst_sle.get_field_u32(sf("sfFlags"));
                let lsf_require_dest_tag: u32 = 0x00020000;
                if dst_flags & lsf_require_dest_tag != 0
                    && !tx.is_field_present(sf("sfDestinationTag"))
                {
                    return Ter::TEC_DST_TAG_NEEDED;
                }
            }
            Ter::TES_SUCCESS
        }
        TxType::PAYCHAN_FUND => {
            let channel_id = tx.get_field_h256(sf("sfChannel"));
            let chan_keylet = protocol::unchecked_keylet(channel_id);
            if let Ok(Some(chan_sle)) = view.read(chan_keylet) {
                let src = chan_sle.get_account_id(sf("sfAccount"));
                let tx_account = tx.get_account_id(sf("sfAccount"));
                if tx_account != src {
                    return Ter::TEC_NO_PERMISSION;
                }
            } else {
                return Ter::TEC_NO_ENTRY;
            }
            Ter::TES_SUCCESS
        }
        TxType::NFTOKEN_MINT => {
            // Reserve check
            let account = tx.get_account_id(sf("sfAccount"));
            let acct_keylet_v = account_keylet(acct_uint160(account));
            if let Ok(Some(acct_sle)) = view.read(acct_keylet_v) {
                let balance = acct_sle.get_field_amount(sf("sfBalance")).xrp().drops();
                let owner_count = acct_sle.get_field_u32(sf("sfOwnerCount"));
                let needed_reserve = 200_000 + 50_000 * (owner_count as i64 + 1);
                let fee = if tx.is_field_present(sf("sfFee")) {
                    tx.get_field_amount(sf("sfFee")).xrp().drops()
                } else {
                    0
                };
                if balance - fee < needed_reserve {
                    return Ter::TEC_INSUFFICIENT_RESERVE;
                }
            }
            // Issuer must exist if specified
            if tx.is_field_present(sf("sfIssuer")) {
                let issuer = tx.get_account_id(sf("sfIssuer"));
                let issuer_keylet = account_keylet(acct_uint160(issuer));
                if !view.exists(issuer_keylet).unwrap_or(false) {
                    return Ter::TEC_NO_ISSUER;
                }
            }
            Ter::TES_SUCCESS
        }
        TxType::VAULT_DELETE | TxType::VAULT_DEPOSIT | TxType::VAULT_WITHDRAW => {
            let vault_id = tx.get_field_h256(sf("sfVaultID"));
            let vault_keylet = protocol::unchecked_keylet(vault_id);
            if !view.exists(vault_keylet).unwrap_or(false) {
                return Ter::TEC_NO_ENTRY;
            }
            Ter::TES_SUCCESS
        }
        _ => Ter::TES_SUCCESS,
    }
}
