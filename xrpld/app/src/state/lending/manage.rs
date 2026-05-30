use std::sync::Arc;

use basics::number::{NumberRoundModeGuard, RoundingMode};
use ledger::{has_expired, views::apply_view::ApplyView};
use protocol::{STLedgerEntry, STTx, Ter, feature_id, tfLoanDefault, tfLoanImpair, tfLoanUnimpair};

use super::common::*;
use super::helpers::*;

pub fn apply_loan_manage<V: ApplyView>(view: &mut V, sttx: &STTx) -> Ter {
    if !view.rules().enabled(&feature_id("LendingProtocol")) {
        return Ter::TEM_DISABLED;
    }

    let account = sttx.get_account_id(sf("sfAccount"));
    let loan_id = sttx.get_field_h256(sf("sfLoanID"));
    let tx_requests_default = sttx.is_flag(tfLoanDefault);
    let tx_requests_impair = sttx.is_flag(tfLoanImpair);
    let tx_requests_unimpair = sttx.is_flag(tfLoanUnimpair);

    let Ok(Some(loan_sle)) = view.peek(protocol::loan_keylet_from_key(loan_id)) else {
        return Ter::TEF_BAD_LEDGER;
    };
    let broker_id = loan_sle.get_field_h256(sf("sfLoanBrokerID"));
    let Ok(Some(broker_sle)) = view.peek(protocol::loan_broker_keylet_from_key(broker_id)) else {
        return Ter::TEF_BAD_LEDGER;
    };
    let vault_id = broker_sle.get_field_h256(sf("sfVaultID"));
    let Ok(Some(vault_sle)) = view.peek(protocol::vault_keylet_from_key(vault_id)) else {
        return Ter::TEF_BAD_LEDGER;
    };
    let vault_asset = vault_sle.get_field_issue(sf("sfAsset")).asset();

    let next_due = loan_sle
        .is_field_present(sf("sfNextPaymentDueDate"))
        .then(|| loan_sle.get_field_u32(sf("sfNextPaymentDueDate")));
    let grace_period = if loan_sle.is_field_present(sf("sfGracePeriod")) {
        loan_sle.get_field_u32(sf("sfGracePeriod"))
    } else {
        0
    };
    let default_expiration = next_due.map(|due| due.saturating_add(grace_period));
    let preclaim = tx::run_loan_manage_preclaim(tx::LoanManagePreclaimFacts {
        loan_exists: true,
        loan_is_defaulted: loan_sle.is_flag(protocol::lsfLoanDefault),
        loan_is_impaired: loan_sle.is_flag(protocol::lsfLoanImpaired),
        tx_requests_impair,
        tx_requests_unimpair,
        tx_requests_default,
        payment_remaining_is_zero: loan_sle.get_field_u32(sf("sfPaymentRemaining")) == 0,
        default_is_too_soon: tx_requests_default && !has_expired(view, default_expiration),
        broker_exists: true,
        submitter_is_broker_owner: broker_sle.get_account_id(sf("sfOwner")) == account,
    });
    if preclaim != Ter::TES_SUCCESS {
        return preclaim;
    }

    let result = if tx_requests_default {
        let loan_scale = loan_sle.get_field_i32(sf("sfLoanScale"));
        let scale = vault_scale(&vault_sle, vault_asset);

        let total_value_outstanding = loan_sle
            .get_field_number(sf("sfTotalValueOutstanding"))
            .value();
        let management_fee_outstanding = loan_sle
            .get_field_number(sf("sfManagementFeeOutstanding"))
            .value();
        let total_default_amount = total_value_outstanding - management_fee_outstanding;

        let broker_debt_total = broker_sle.get_field_number(sf("sfDebtTotal")).value();
        let minimum_cover = tenth_bips_of_runtime_number(
            broker_debt_total,
            broker_sle.get_field_u32(sf("sfCoverRateMinimum")),
        );
        let liquidation_cover = tenth_bips_of_runtime_number(
            minimum_cover,
            broker_sle.get_field_u32(sf("sfCoverRateLiquidation")),
        );
        let liquidation_cover_capped = liquidation_cover.min(total_default_amount);
        let _upward = NumberRoundModeGuard::new(RoundingMode::Upward);
        let covered_before_cover_available = round_number_to_asset_with_scale(
            vault_asset,
            liquidation_cover_capped,
            loan_scale,
            RoundingMode::Upward,
        );
        drop(_upward);
        let cover_available = broker_sle.get_field_number(sf("sfCoverAvailable")).value();
        let default_covered = covered_before_cover_available.min(cover_available);
        let vault_default_amount = total_default_amount - default_covered;

        let vault_total_assets = vault_sle.get_field_number(sf("sfAssetsTotal")).value();
        if vault_total_assets < vault_default_amount {
            Ter::TEF_BAD_LEDGER
        } else {
            let vault_default_rounded = round_number_to_asset_with_scale(
                vault_asset,
                vault_default_amount,
                scale,
                RoundingMode::Downward,
            );
            let mut vault_total_after = vault_total_assets - vault_default_rounded;
            let vault_available_after =
                vault_sle.get_field_number(sf("sfAssetsAvailable")).value() + default_covered;
            if vault_available_after > vault_total_after && !vault_asset.integral() {
                let difference = vault_available_after - vault_total_after;
                let available_exponent = vault_available_after
                    .external_parts()
                    .map(|(_, exponent)| exponent)
                    .unwrap_or(0);
                let difference_exponent = difference
                    .external_parts()
                    .map(|(_, exponent)| exponent)
                    .unwrap_or(0);
                if available_exponent - difference_exponent > 13 {
                    vault_total_after = vault_available_after;
                }
            }
            if vault_available_after > vault_total_after {
                Ter::TEC_INTERNAL
            } else {
                let mut vault_obj = vault_sle.clone_as_object();
                vault_obj.set_field_number(
                    sf("sfAssetsTotal"),
                    with_asset_number(vault_total_after, vault_asset),
                );
                vault_obj.set_field_number(
                    sf("sfAssetsAvailable"),
                    with_asset_number(vault_available_after, vault_asset),
                );
                if loan_sle.is_flag(protocol::lsfLoanImpaired) {
                    let loss_unrealized =
                        vault_sle.get_field_number(sf("sfLossUnrealized")).value();
                    if loss_unrealized < total_default_amount {
                        return Ter::TEF_BAD_LEDGER;
                    }
                    let loss_after = adjust_imprecise_number(
                        loss_unrealized,
                        -total_default_amount,
                        vault_asset,
                        scale,
                    );
                    vault_obj.set_field_number(
                        sf("sfLossUnrealized"),
                        with_asset_number(loss_after, vault_asset),
                    );
                }
                let vault_update = persist_entry(
                    view,
                    STLedgerEntry::from_stobject(vault_obj, *vault_sle.key()),
                );
                if vault_update != Ter::TES_SUCCESS {
                    vault_update
                } else {
                    if cover_available < default_covered {
                        return Ter::TEF_BAD_LEDGER;
                    }
                    let mut broker_obj = broker_sle.clone_as_object();
                    let broker_debt_after = adjust_imprecise_number(
                        broker_debt_total,
                        -total_default_amount,
                        vault_asset,
                        scale,
                    );
                    broker_obj.set_field_number(
                        sf("sfDebtTotal"),
                        with_asset_number(broker_debt_after, vault_asset),
                    );
                    broker_obj.set_field_number(
                        sf("sfCoverAvailable"),
                        with_asset_number(cover_available - default_covered, vault_asset),
                    );
                    let broker_update = persist_entry(
                        view,
                        STLedgerEntry::from_stobject(broker_obj, *broker_sle.key()),
                    );
                    if broker_update != Ter::TES_SUCCESS {
                        broker_update
                    } else {
                        let mut loan_obj = loan_sle.clone_as_object();
                        loan_obj.set_flag(protocol::lsfLoanDefault);
                        loan_obj.set_field_number(
                            sf("sfTotalValueOutstanding"),
                            zero_asset_number(vault_asset),
                        );
                        loan_obj.set_field_u32(sf("sfPaymentRemaining"), 0);
                        loan_obj.set_field_number(
                            sf("sfPrincipalOutstanding"),
                            zero_asset_number(vault_asset),
                        );
                        loan_obj.set_field_number(
                            sf("sfManagementFeeOutstanding"),
                            zero_asset_number(vault_asset),
                        );
                        loan_obj.set_field_u32(sf("sfNextPaymentDueDate"), 0);
                        let loan_update = persist_entry(
                            view,
                            STLedgerEntry::from_stobject(loan_obj, *loan_sle.key()),
                        );
                        if loan_update != Ter::TES_SUCCESS {
                            loan_update
                        } else {
                            let Ok(default_covered_amount) = vault_asset.amount(default_covered)
                            else {
                                return Ter::TEF_BAD_LEDGER;
                            };
                            account_send(
                                view,
                                &broker_sle.get_account_id(sf("sfAccount")),
                                &vault_sle.get_account_id(sf("sfAccount")),
                                &default_covered_amount,
                            )
                        }
                    }
                }
            }
        }
    } else if tx_requests_impair {
        let scale = vault_scale(&vault_sle, vault_asset);
        let loss_unrealized = loan_sle
            .get_field_number(sf("sfTotalValueOutstanding"))
            .value()
            - loan_sle
                .get_field_number(sf("sfManagementFeeOutstanding"))
                .value();
        let current_loss = vault_sle.get_field_number(sf("sfLossUnrealized")).value();
        let updated_loss =
            adjust_imprecise_number(current_loss, loss_unrealized, vault_asset, scale);
        let unavailable_assets = vault_sle.get_field_number(sf("sfAssetsTotal")).value()
            - vault_sle.get_field_number(sf("sfAssetsAvailable")).value();
        if updated_loss > unavailable_assets {
            Ter::TEC_LIMIT_EXCEEDED
        } else {
            let mut vault_obj = vault_sle.clone_as_object();
            vault_obj.set_field_number(
                sf("sfLossUnrealized"),
                with_asset_number(updated_loss, vault_asset),
            );
            let vault_update = persist_entry(
                view,
                STLedgerEntry::from_stobject(vault_obj, *vault_sle.key()),
            );
            if vault_update != Ter::TES_SUCCESS {
                vault_update
            } else {
                let mut loan_obj = loan_sle.clone_as_object();
                loan_obj.set_flag(protocol::lsfLoanImpaired);
                let current_due = next_due.unwrap_or(0);
                let next_payment_due = if has_expired(view, Some(current_due)) {
                    current_due
                } else {
                    view.parent_close_time().as_seconds()
                };
                loan_obj.set_field_u32(sf("sfNextPaymentDueDate"), next_payment_due);
                persist_entry(
                    view,
                    STLedgerEntry::from_stobject(loan_obj, *loan_sle.key()),
                )
            }
        }
    } else if tx_requests_unimpair {
        let scale = vault_scale(&vault_sle, vault_asset);
        let loss_reversed = loan_sle
            .get_field_number(sf("sfTotalValueOutstanding"))
            .value()
            - loan_sle
                .get_field_number(sf("sfManagementFeeOutstanding"))
                .value();
        let current_loss = vault_sle.get_field_number(sf("sfLossUnrealized")).value();
        if current_loss < loss_reversed {
            Ter::TEF_BAD_LEDGER
        } else {
            let updated_loss =
                adjust_imprecise_number(current_loss, -loss_reversed, vault_asset, scale);
            let mut vault_obj = vault_sle.clone_as_object();
            vault_obj.set_field_number(
                sf("sfLossUnrealized"),
                with_asset_number(updated_loss, vault_asset),
            );
            let vault_update = persist_entry(
                view,
                STLedgerEntry::from_stobject(vault_obj, *vault_sle.key()),
            );
            if vault_update != Ter::TES_SUCCESS {
                vault_update
            } else {
                let previous_due = if loan_sle.is_field_present(sf("sfPreviousPaymentDueDate")) {
                    loan_sle.get_field_u32(sf("sfPreviousPaymentDueDate"))
                } else {
                    0
                };
                let start_date = loan_sle.get_field_u32(sf("sfStartDate"));
                let payment_interval = loan_sle.get_field_u32(sf("sfPaymentInterval"));
                let normal_payment_due_date = previous_due
                    .max(start_date)
                    .saturating_add(payment_interval);
                let next_payment_due = if has_expired(view, Some(normal_payment_due_date)) {
                    view.parent_close_time()
                        .as_seconds()
                        .saturating_add(payment_interval)
                } else {
                    normal_payment_due_date
                };
                let mut loan_obj = loan_sle.clone_as_object();
                loan_obj.clear_flag(protocol::lsfLoanImpaired);
                loan_obj.set_field_u32(sf("sfNextPaymentDueDate"), next_payment_due);
                persist_entry(
                    view,
                    STLedgerEntry::from_stobject(loan_obj, *loan_sle.key()),
                )
            }
        }
    } else {
        Ter::TES_SUCCESS
    };

    if result == Ter::TES_SUCCESS && view.rules().enabled(&feature_id("fixCleanup3_1_3")) {
        let Ok(Some(loan_entry)) = view.peek(protocol::loan_keylet_from_key(loan_id)) else {
            return Ter::TEF_BAD_LEDGER;
        };
        let Ok(Some(broker_entry)) = view.peek(protocol::loan_broker_keylet_from_key(broker_id))
        else {
            return Ter::TEF_BAD_LEDGER;
        };
        let Ok(Some(vault_entry)) = view.peek(protocol::vault_keylet_from_key(vault_id)) else {
            return Ter::TEF_BAD_LEDGER;
        };
        let loan_obj = loan_entry.clone_as_object();
        let mut loan_update = STLedgerEntry::from_stobject(loan_obj, *loan_entry.key());
        associate_asset_entry(&mut loan_update, vault_asset);
        let _ = view.update(Arc::new(loan_update));
        let broker_obj = broker_entry.clone_as_object();
        let mut broker_update = STLedgerEntry::from_stobject(broker_obj, *broker_entry.key());
        associate_asset_entry(&mut broker_update, vault_asset);
        let _ = view.update(Arc::new(broker_update));
        let vault_obj = vault_entry.clone_as_object();
        let mut vault_update = STLedgerEntry::from_stobject(vault_obj, *vault_entry.key());
        associate_asset_entry(&mut vault_update, vault_asset);
        let _ = view.update(Arc::new(vault_update));
    }

    result
}
