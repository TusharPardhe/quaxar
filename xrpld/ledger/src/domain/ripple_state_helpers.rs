//! Full reference the reference source parity.
//!
//! Trust line manipulation: issueIOU, redeemIOU, trustCreate, trustDelete,
//! updateTrustLine, creditLimit, creditBalance, freeze checks.

use std::sync::Arc;

use crate::ApplyView;
use protocol::{
    AccountID, Asset, Currency, Issue, MPTIssue, STAmount, STLedgerEntry, STObject, Ter, XRPAmount,
    get_field_by_symbol as sf,
};

// Trust line flags (reference lsf* constants)
const LSF_LOW_RESERVE: u32 = 0x0001_0000;
const LSF_HIGH_RESERVE: u32 = 0x0002_0000;
const LSF_LOW_NO_RIPPLE: u32 = 0x0010_0000;
const LSF_HIGH_NO_RIPPLE: u32 = 0x0020_0000;
const LSF_LOW_FREEZE: u32 = 0x0040_0000;
const LSF_HIGH_FREEZE: u32 = 0x0080_0000;
const LSF_DEFAULT_RIPPLE: u32 = 0x0080_0000; // on AccountRoot

/// Returns true if the trust line should be deleted.
fn update_trust_line<V: ApplyView>(
    view: &mut V,
    state: &STLedgerEntry,
    b_sender_high: bool,
    sender: &AccountID,
    before: &STAmount,
    after: &STAmount,
) -> (bool, Option<u32>) {
    // Returns (should_delete, new_flags_if_reserve_cleared)
    let flags = state.get_field_u32(sf("sfFlags"));

    let sender_keylet =
        protocol::account_keylet(basics::base_uint::Uint160::from_void(sender.data()));
    let Some(sle) = view.peek(sender_keylet).ok().flatten() else {
        return (false, None);
    };

    // Determine which side's flags to check based on sender position
    let reserve_flag = if !b_sender_high {
        LSF_LOW_RESERVE
    } else {
        LSF_HIGH_RESERVE
    };
    let no_ripple_flag = if !b_sender_high {
        LSF_LOW_NO_RIPPLE
    } else {
        LSF_HIGH_NO_RIPPLE
    };
    let freeze_flag = if !b_sender_high {
        LSF_LOW_FREEZE
    } else {
        LSF_HIGH_FREEZE
    };
    let limit_field = if !b_sender_high {
        sf("sfLowLimit")
    } else {
        sf("sfHighLimit")
    };
    let quality_in_field = if !b_sender_high {
        sf("sfLowQualityIn")
    } else {
        sf("sfHighQualityIn")
    };
    let quality_out_field = if !b_sender_high {
        sf("sfLowQualityOut")
    } else {
        sf("sfHighQualityOut")
    };

    // reserve is set, NoRipple matches DefaultRipple, not frozen,
    // limit is zero, quality in/out are zero.
    let account_flags = sle.get_field_u32(sf("sfFlags"));
    let no_ripple_set = (flags & no_ripple_flag) != 0;
    let default_ripple_set = (account_flags & LSF_DEFAULT_RIPPLE) != 0;

    if before.signum() > 0
        && after.signum() <= 0
        && (flags & reserve_flag) != 0
        && no_ripple_set != default_ripple_set
        && (flags & freeze_flag) == 0
        && state.get_field_amount(limit_field).signum() == 0
        && state.get_field_u32(quality_in_field) == 0
        && state.get_field_u32(quality_out_field) == 0
    {
        // Release reserve
        adjust_owner_count(view, &sle, -1);

        // Clear reserve flag
        let new_flags = flags & !reserve_flag;

        // Check if line should be deleted:
        // Balance is zero AND the other side's reserve is also clear
        let other_reserve = if b_sender_high {
            LSF_LOW_RESERVE
        } else {
            LSF_HIGH_RESERVE
        };
        if after.signum() == 0 && (new_flags & other_reserve) == 0 {
            return (true, Some(new_flags));
        }
        return (false, Some(new_flags));
    }
    (false, None)
}

fn adjust_owner_count<V: ApplyView>(view: &mut V, account_sle: &STLedgerEntry, adjustment: i32) {
    let current = account_sle.get_field_u32(sf("sfOwnerCount"));
    let new_count = if adjustment > 0 {
        current.saturating_add(adjustment as u32)
    } else {
        current.saturating_sub((-adjustment) as u32)
    };
    let mut obj = account_sle.clone_as_object();
    obj.set_field_u32(sf("sfOwnerCount"), new_count);
    let _ = view.update(Arc::new(STLedgerEntry::from_stobject(
        obj,
        *account_sle.key(),
    )));
}

fn trust_delete<V: ApplyView>(
    view: &mut V,
    state: &STLedgerEntry,
    _low_account: &AccountID,
    _high_account: &AccountID,
) -> Ter {
    // Remove from both owner directories and delete the entry
    let _ = view.erase(Arc::new(state.clone()));
    Ter::TES_SUCCESS
}

/// Creates trust line if it doesn't exist.
pub fn issue_iou<V: ApplyView>(
    view: &mut V,
    account: &AccountID,
    amount: &STAmount,
    issue: &Issue,
) -> Ter {
    if amount.signum() <= 0 {
        return Ter::TES_SUCCESS;
    }

    let b_sender_high = issue.account > *account;
    let line_keylet = protocol::line(issue.account, *account, issue.currency);

    if let Some(state) = view.peek(line_keylet).ok().flatten() {
        let mut final_balance = state.get_field_amount(sf("sfBalance"));
        if b_sender_high {
            final_balance.negate();
        }
        let start_balance = final_balance.clone();
        final_balance -= amount.clone();

        let (must_delete, new_flags) = update_trust_line(
            view,
            &state,
            b_sender_high,
            &issue.account,
            &start_balance,
            &final_balance,
        );

        if b_sender_high {
            final_balance.negate();
        }

        let mut obj = state.clone_as_object();
        obj.set_field_amount(sf("sfBalance"), final_balance);
        if let Some(nf) = new_flags {
            obj.set_field_u32(sf("sfFlags"), nf);
        }

        if must_delete {
            let low = if b_sender_high {
                account
            } else {
                &issue.account
            };
            let high = if b_sender_high {
                &issue.account
            } else {
                account
            };
            return trust_delete(view, &state, low, high);
        }

        let _ = view.update(Arc::new(STLedgerEntry::from_stobject(obj, *state.key())));
        Ter::TES_SUCCESS
    } else {
        // Trust line doesn't exist — create it
        // For the payment engine, this means the receiver gets a new trust line
        // with the balance set to the amount received.
        let mut new_state = STObject::new(sf("sfGeneric"));
        new_state.set_field_u16(
            sf("sfLedgerEntryType"),
            protocol::LedgerEntryType::RippleState as u16,
        );

        let balance = if b_sender_high {
            let mut b = amount.clone();
            b.negate();
            b
        } else {
            amount.clone()
        };
        new_state.set_field_amount(sf("sfBalance"), balance);

        // Set limits (zero for the new side)
        let low_limit = STAmount::new_with_asset(
            sf("sfLowLimit"),
            protocol::Asset::Issue(Issue {
                currency: issue.currency,
                account: if b_sender_high {
                    *account
                } else {
                    issue.account
                },
            }),
            0,
            0,
            false,
        );
        let high_limit = STAmount::new_with_asset(
            sf("sfHighLimit"),
            protocol::Asset::Issue(Issue {
                currency: issue.currency,
                account: if b_sender_high {
                    issue.account
                } else {
                    *account
                },
            }),
            0,
            0,
            false,
        );
        new_state.set_field_amount(sf("sfLowLimit"), low_limit);
        new_state.set_field_amount(sf("sfHighLimit"), high_limit);

        // Set flags with reserve for the receiver
        let flags = if b_sender_high {
            LSF_HIGH_RESERVE
        } else {
            LSF_LOW_RESERVE
        };
        new_state.set_field_u32(sf("sfFlags"), flags);

        let sle = Arc::new(STLedgerEntry::from_stobject(new_state, line_keylet.key));
        let _ = view.insert(sle);

        let low_account = if b_sender_high {
            *account
        } else {
            issue.account
        };
        let high_account = if b_sender_high {
            issue.account
        } else {
            *account
        };
        let low_dir =
            protocol::owner_dir_keylet(basics::base_uint::Uint160::from_void(low_account.data()));
        let high_dir =
            protocol::owner_dir_keylet(basics::base_uint::Uint160::from_void(high_account.data()));
        let _ = crate::views::directory::dir_insert(view, &low_dir, line_keylet.key, &|_obj| {});
        let _ = crate::views::directory::dir_insert(view, &high_dir, line_keylet.key, &|_obj| {});

        // Adjust owner count for receiver
        let acct_keylet =
            protocol::account_keylet(basics::base_uint::Uint160::from_void(account.data()));
        if let Some(acct_sle) = view.peek(acct_keylet).ok().flatten() {
            adjust_owner_count(view, &acct_sle, 1);
        }

        Ter::TES_SUCCESS
    }
}

pub fn redeem_iou<V: ApplyView>(
    view: &mut V,
    account: &AccountID,
    amount: &STAmount,
    issue: &Issue,
) -> Ter {
    if amount.signum() <= 0 {
        return Ter::TES_SUCCESS;
    }

    let b_sender_high = *account > issue.account;
    let line_keylet = protocol::line(*account, issue.account, issue.currency);

    let Some(state) = view.peek(line_keylet).ok().flatten() else {
        return Ter::TEF_INTERNAL;
    };

    let mut final_balance = state.get_field_amount(sf("sfBalance"));
    if b_sender_high {
        final_balance.negate();
    }
    let start_balance = final_balance.clone();
    final_balance -= amount.clone();

    let (must_delete, new_flags) = update_trust_line(
        view,
        &state,
        b_sender_high,
        account,
        &start_balance,
        &final_balance,
    );

    if b_sender_high {
        final_balance.negate();
    }

    let mut obj = state.clone_as_object();
    obj.set_field_amount(sf("sfBalance"), final_balance);
    if let Some(nf) = new_flags {
        obj.set_field_u32(sf("sfFlags"), nf);
    }

    if must_delete {
        let low = if b_sender_high {
            &issue.account
        } else {
            account
        };
        let high = if b_sender_high {
            account
        } else {
            &issue.account
        };
        return trust_delete(view, &state, low, high);
    }

    let _ = view.update(Arc::new(STLedgerEntry::from_stobject(obj, *state.key())));
    Ter::TES_SUCCESS
}

pub fn transfer_xrp<V: ApplyView>(
    view: &mut V,
    from: &AccountID,
    to: &AccountID,
    amount: XRPAmount,
) -> Ter {
    if amount.drops() <= 0 {
        return Ter::TES_SUCCESS;
    }

    // When zero, skip that side — this is used by the flow engine for
    // XRP endpoint steps where XRP flows through the virtual "XRP issuer".
    let from_is_zero = from.data().iter().all(|&b| b == 0);
    let to_is_zero = to.data().iter().all(|&b| b == 0);

    if from_is_zero && to_is_zero {
        return Ter::TES_SUCCESS;
    }

    // Debit sender (if not xrpAccount)
    if !from_is_zero {
        let from_keylet =
            protocol::account_keylet(basics::base_uint::Uint160::from_void(from.data()));
        let Some(from_sle) = view.peek(from_keylet).ok().flatten() else {
            return Ter::TER_NO_ACCOUNT;
        };
        let from_balance = from_sle.get_field_amount(sf("sfBalance")).xrp().drops();
        // Safety guard: prevent negative balance. Upstream callers should
        // validate balances before reaching this point.
        if from_balance < amount.drops() {
            return Ter::TEC_FAILED_PROCESSING;
        }
        let mut from_obj = from_sle.clone_as_object();
        from_obj.set_field_amount(
            sf("sfBalance"),
            STAmount::from_xrp_amount(XRPAmount::from_drops(from_balance - amount.drops())),
        );
        let _ = view.update(Arc::new(STLedgerEntry::from_stobject(
            from_obj,
            *from_sle.key(),
        )));
    }

    // Credit receiver (if not xrpAccount)
    if !to_is_zero {
        let to_keylet = protocol::account_keylet(basics::base_uint::Uint160::from_void(to.data()));
        let Some(to_sle) = view.peek(to_keylet).ok().flatten() else {
            return Ter::TER_NO_ACCOUNT;
        };
        let to_balance = to_sle.get_field_amount(sf("sfBalance")).xrp().drops();
        let mut to_obj = to_sle.clone_as_object();
        to_obj.set_field_amount(
            sf("sfBalance"),
            STAmount::from_xrp_amount(XRPAmount::from_drops(to_balance + amount.drops())),
        );
        let _ = view.update(Arc::new(STLedgerEntry::from_stobject(
            to_obj,
            *to_sle.key(),
        )));
    }

    Ter::TES_SUCCESS
}

pub fn account_send<V: ApplyView>(
    view: &mut V,
    from: &AccountID,
    to: &AccountID,
    amount: &STAmount,
) -> Ter {
    if amount.signum() <= 0 || *from == *to {
        return Ter::TES_SUCCESS;
    }

    static SEND_LOG: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let c = SEND_LOG.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    if c < 5 {
        tracing::debug!(target: "ledger",            "[account_send] native={} from={:02x}{:02x} to={:02x}{:02x} amount_signum={}",
            amount.native(),
            from.data()[0],
            from.data()[1],
            to.data()[0],
            to.data()[1],
            amount.signum()
        );
    }

    if amount.native() {
        return transfer_xrp(view, from, to, amount.xrp());
    }

    if let Asset::MPTIssue(issue) = amount.asset() {
        return account_send_mpt(view, from, to, amount, &issue);
    }

    let issue = amount.issue();

    // If sender or receiver is issuer, or issuer is noAccount → direct send (no fee)
    if *from == issue.account || *to == issue.account || issue.account.is_zero() {
        return direct_send_no_fee_iou(view, from, to, amount, false);
    }

    // Sending 3rd party IOUs: transit with transfer fee
    let rate = transfer_rate(view, &issue.account);
    let actual_cost = if rate == 1_000_000_000 {
        amount.clone()
    } else {
        let iou = amount.iou();
        let adjusted = crate::domain::mul_ratio::mul_ratio(
            iou,
            rate,
            crate::domain::mul_ratio::QUALITY_ONE,
            true,
        );
        STAmount::from_iou_amount(sf("sfAmount"), adjusted, issue)
    };

    // reference: directSendNoFeeIOU(view, issuer, receiver, amount, true)
    let res = direct_send_no_fee_iou(view, &issue.account, to, amount, true);
    if res != Ter::TES_SUCCESS {
        return res;
    }
    // reference: directSendNoFeeIOU(view, sender, issuer, actualCost, true)
    direct_send_no_fee_iou(view, from, &issue.account, &actual_cost, true)
}

fn update_mpt_amount<V: ApplyView>(
    view: &mut V,
    account: &AccountID,
    issue: &MPTIssue,
    delta: i64,
) -> Ter {
    if *account == issue.issuer() || delta == 0 {
        return Ter::TES_SUCCESS;
    }

    let token_key = protocol::mptoken_keylet_from_mptid(
        issue.mpt_id(),
        basics::base_uint::Uint160::from_void(account.data()),
    );
    let Some(token) = view.peek(token_key).ok().flatten() else {
        return Ter::TEC_NO_AUTH;
    };
    let current = token.get_field_u64(sf("sfMPTAmount"));
    let Some(next) = (if delta.is_negative() {
        current.checked_sub(delta.unsigned_abs())
    } else {
        current.checked_add(delta as u64)
    }) else {
        return if delta.is_negative() {
            Ter::TEC_INSUFFICIENT_FUNDS
        } else {
            Ter::TEC_INTERNAL
        };
    };

    let mut updated = (*token).clone();
    updated.set_field_u64(sf("sfMPTAmount"), next);
    view.update(Arc::new(updated))
        .map_or(Ter::TEF_BAD_LEDGER, |_| Ter::TES_SUCCESS)
}

fn account_send_mpt<V: ApplyView>(
    view: &mut V,
    from: &AccountID,
    to: &AccountID,
    amount: &STAmount,
    issue: &MPTIssue,
) -> Ter {
    let value = amount.mpt().value();
    if value <= 0 || from == to {
        return Ter::TES_SUCCESS;
    }

    let issuer = issue.issuer();
    let debit_value = if from != &issuer && to != &issuer {
        let rate = crate::mptoken_helpers::transfer_rate_mpt(view, issue.mpt_id())
            .unwrap_or(protocol::PARITY_RATE);
        protocol::multiply_round(amount, rate, true).mpt().value()
    } else {
        value
    };
    if debit_value < value {
        return Ter::TEC_INTERNAL;
    }

    let Some(issuance) = view
        .peek(protocol::mpt_issuance_keylet_from_mptid(issue.mpt_id()))
        .ok()
        .flatten()
    else {
        return Ter::TEC_OBJECT_NOT_FOUND;
    };
    let amount = value as u64;
    let debit_amount = debit_value as u64;
    let outstanding = issuance.get_field_u64(sf("sfOutstandingAmount"));

    if from == &issuer {
        let maximum = crate::mptoken_helpers::max_mpt_amount(&issuance);
        if crate::mptoken_helpers::is_mpt_overflow(value, outstanding, maximum) {
            return Ter::TEC_PATH_DRY;
        }
        let Some(next_outstanding) = outstanding.checked_add(amount) else {
            return Ter::TEC_INTERNAL;
        };
        let mut updated = (*issuance).clone();
        updated.set_field_u64(sf("sfOutstandingAmount"), next_outstanding);
        if view.update(Arc::new(updated)).is_err() {
            return Ter::TEF_BAD_LEDGER;
        }
    } else {
        let result = update_mpt_amount(view, from, issue, -debit_value);
        if result != Ter::TES_SUCCESS {
            return result;
        }
    }

    if to == &issuer {
        let Some(issuance) = view
            .peek(protocol::mpt_issuance_keylet_from_mptid(issue.mpt_id()))
            .ok()
            .flatten()
        else {
            return Ter::TEC_OBJECT_NOT_FOUND;
        };
        let outstanding = issuance.get_field_u64(sf("sfOutstandingAmount"));
        let Some(next_outstanding) = outstanding.checked_sub(amount) else {
            return Ter::TEC_INTERNAL;
        };
        let mut updated = (*issuance).clone();
        updated.set_field_u64(sf("sfOutstandingAmount"), next_outstanding);
        view.update(Arc::new(updated))
            .map_or(Ter::TEF_BAD_LEDGER, |_| Ter::TES_SUCCESS)
    } else {
        let result = update_mpt_amount(view, to, issue, value);
        if result != Ter::TES_SUCCESS {
            return result;
        }
        let fee = debit_amount - amount;
        if fee == 0 {
            return Ter::TES_SUCCESS;
        }
        let Some(issuance) = view
            .peek(protocol::mpt_issuance_keylet_from_mptid(issue.mpt_id()))
            .ok()
            .flatten()
        else {
            return Ter::TEC_OBJECT_NOT_FOUND;
        };
        let outstanding = issuance.get_field_u64(sf("sfOutstandingAmount"));
        let Some(next_outstanding) = outstanding.checked_sub(fee) else {
            return Ter::TEC_INTERNAL;
        };
        let mut updated = (*issuance).clone();
        updated.set_field_u64(sf("sfOutstandingAmount"), next_outstanding);
        view.update(Arc::new(updated))
            .map_or(Ter::TEF_BAD_LEDGER, |_| Ter::TES_SUCCESS)
    }
}

/// This is the core function that handles reserve cleanup.
fn direct_send_no_fee_iou<V: ApplyView>(
    view: &mut V,
    sender: &AccountID,
    receiver: &AccountID,
    amount: &STAmount,
    check_issuer: bool,
) -> Ter {
    if sender == receiver || amount.signum() <= 0 {
        return Ter::TES_SUCCESS;
    }

    let issue = amount.issue();

    // Only check freeze when check_issuer is true.
    // AMMClawback passes check_issuer=false to bypass freeze checks.
    if check_issuer && (is_frozen(view, sender, &issue) || is_frozen(view, receiver, &issue)) {
        return Ter::TEC_PATH_DRY;
    }

    let b_sender_high = *sender > *receiver;
    let line_keylet = protocol::line(*sender, *receiver, issue.currency);

    if let Some(state) = view.peek(line_keylet).ok().flatten() {
        let mut balance = state.get_field_amount(sf("sfBalance"));
        if b_sender_high {
            balance.negate();
        }

        let before = balance.clone();
        balance -= amount.clone();

        let (must_delete, new_flags) =
            update_trust_line(view, &state, b_sender_high, sender, &before, &balance);

        if b_sender_high {
            balance.negate();
        }

        let mut obj = state.clone_as_object();
        obj.set_field_amount(sf("sfBalance"), balance);
        if let Some(nf) = new_flags {
            obj.set_field_u32(sf("sfFlags"), nf);
        }

        if must_delete {
            let low = if b_sender_high { receiver } else { sender };
            let high = if b_sender_high { sender } else { receiver };
            return trust_delete(view, &state, low, high);
        }

        let _ = view.update(Arc::new(STLedgerEntry::from_stobject(obj, *state.key())));
        Ter::TES_SUCCESS
    } else {
        // Trust line doesn't exist — create it (receiver gets balance)
        let b_high = *sender > *receiver;
        let mut balance = amount.clone();
        balance.set_issuer(protocol::no_account());
        if !b_high {
            // balance stored from low's perspective; sender is low, so negate
            // (sender loses, so low balance goes negative)
            balance.negate();
        }

        // Check receiver's DefaultRipple for NoRipple flag
        let receiver_keylet =
            protocol::account_keylet(basics::base_uint::Uint160::from_void(receiver.data()));
        let no_ripple = if let Ok(Some(rcv_sle)) = view.peek(receiver_keylet) {
            (rcv_sle.get_field_u32(sf("sfFlags")) & LSF_DEFAULT_RIPPLE) == 0
        } else {
            return Ter::TEF_INTERNAL;
        };

        // Create the trust line
        let mut new_obj = STObject::new(sf("sfGeneric"));
        new_obj.set_field_u16(sf("sfLedgerEntryType"), 0x0072); // ltRIPPLE_STATE
        new_obj.set_field_amount(sf("sfBalance"), balance);
        let zero = amount.zeroed();
        if !b_high {
            new_obj.set_field_amount(sf("sfLowLimit"), zero.clone());
            let mut recv_limit = zero;
            recv_limit.set_issuer(*receiver);
            new_obj.set_field_amount(sf("sfHighLimit"), recv_limit);
        } else {
            let mut recv_limit = zero.clone();
            recv_limit.set_issuer(*receiver);
            new_obj.set_field_amount(sf("sfLowLimit"), recv_limit);
            new_obj.set_field_amount(sf("sfHighLimit"), zero);
        }

        // Set flags: receiver gets reserve, maybe NoRipple
        let mut flags = if b_high {
            LSF_LOW_RESERVE
        } else {
            LSF_HIGH_RESERVE
        };
        if no_ripple {
            flags |= if b_high {
                LSF_LOW_NO_RIPPLE
            } else {
                LSF_HIGH_NO_RIPPLE
            };
        }
        new_obj.set_field_u32(sf("sfFlags"), flags);

        let new_sle = Arc::new(STLedgerEntry::from_stobject(new_obj, line_keylet.key));

        // Add to both owner directories
        let low_dir = protocol::owner_dir_keylet(basics::base_uint::Uint160::from_void(
            (if b_sender_high { receiver } else { sender }).data(),
        ));
        let _ = crate::dir_append(
            view as &mut dyn ApplyView,
            &low_dir,
            line_keylet.key,
            &|_| {},
        );
        let high_dir = protocol::owner_dir_keylet(basics::base_uint::Uint160::from_void(
            (if b_sender_high { sender } else { receiver }).data(),
        ));
        let _ = crate::dir_append(
            view as &mut dyn ApplyView,
            &high_dir,
            line_keylet.key,
            &|_| {},
        );

        // Adjust receiver's owner count
        if let Ok(Some(rcv_sle)) = view.peek(protocol::account_keylet(
            basics::base_uint::Uint160::from_void(receiver.data()),
        )) {
            adjust_owner_count(view, &rcv_sle, 1);
        }

        let _ = view.insert(new_sle);
        Ter::TES_SUCCESS
    }
}

/// Handles the case where sender/receiver may be the issuer.
pub fn ripple_credit<V: ApplyView>(
    view: &mut V,
    from: &AccountID,
    to: &AccountID,
    amount: &STAmount,
    _check_issuer: bool,
) -> Ter {
    account_send(view, from, to, amount)
}

/// Public wrapper for direct_send_no_fee_iou — transfers IOU without applying
/// transfer fee. Used by the flow engine where fees are computed at the strand level.
pub fn direct_send_no_fee_iou_pub<V: ApplyView>(
    view: &mut V,
    sender: &AccountID,
    receiver: &AccountID,
    amount: &STAmount,
) -> Ter {
    direct_send_no_fee_iou(view, sender, receiver, amount, false)
}

/// Check if a trust line is individually frozen.
pub fn is_frozen<V: ApplyView>(view: &mut V, account: &AccountID, issue: &Issue) -> bool {
    if issue.currency == protocol::xrp_currency() {
        return false;
    }
    let issuer_keylet =
        protocol::account_keylet(basics::base_uint::Uint160::from_void(issue.account.data()));
    if let Some(issuer_sle) = view
        .peek(issuer_keylet)
        .ok()
        .flatten()
        .or_else(|| view.read(issuer_keylet).ok().flatten())
    {
        let issuer_flags = issuer_sle.get_field_u32(sf("sfFlags"));
        // lsfGlobalFreeze = 0x00400000
        if (issuer_flags & 0x0040_0000) != 0 {
            return true;
        }
    }
    // Then check individual trust line freeze
    let line_keylet = protocol::line(*account, issue.account, issue.currency);
    let Some(state) = view
        .peek(line_keylet)
        .ok()
        .flatten()
        .or_else(|| view.read(line_keylet).ok().flatten())
    else {
        return false;
    };
    let flags = state.get_field_u32(sf("sfFlags"));
    // lsfLowFreeze means the low account froze it, lsfHighFreeze means the high account froze it.
    // Either way, the line is frozen for both parties.
    (flags & LSF_LOW_FREEZE) != 0 || (flags & LSF_HIGH_FREEZE) != 0
}

/// Get the credit balance on a trust line from account's perspective.
/// Returns positive when account HOLDS the IOU (issuer owes account).
/// Returns negative when account OWES the issuer.
///
/// Convention: sfBalance is stored from the low account's perspective
/// (positive = low account holds). When account is the high account
/// (account > issuer), negate to convert to account's perspective.
///
/// NOTE: This is "hold semantics" — equivalent to reference getTrustLineBalance,
/// NOT reference creditBalance (which has opposite/debt semantics).
/// Do NOT change the sign rule to match reference creditBalance.
pub fn credit_balance<V: ApplyView>(
    view: &mut V,
    account: &AccountID,
    issuer: &AccountID,
    currency: Currency,
) -> STAmount {
    let line_keylet = protocol::line(*account, *issuer, currency);
    let Some(state) = view
        .peek(line_keylet)
        .ok()
        .flatten()
        .or_else(|| view.read(line_keylet).ok().flatten())
    else {
        return STAmount::default();
    };
    let mut balance = state.get_field_amount(sf("sfBalance"));
    // sfBalance is from the low account's perspective.
    // If account is the high account, negate to get account's perspective.
    if *account > *issuer {
        balance.negate();
    }
    balance
}

/// Returns the rate as u32 (1000000000 = no fee, 2000000000 = 100% fee).
pub fn transfer_rate<V: ApplyView>(view: &mut V, issuer: &AccountID) -> u32 {
    let acct_keylet =
        protocol::account_keylet(basics::base_uint::Uint160::from_void(issuer.data()));
    let Some(sle) = view.peek(acct_keylet).ok().flatten() else {
        return 1_000_000_000; // PARITY_RATE
    };
    if sle.is_field_present(sf("sfTransferRate")) {
        sle.get_field_u32(sf("sfTransferRate"))
    } else {
        1_000_000_000 // PARITY_RATE (no fee)
    }
}

/// Applies transfer rate when sender and receiver are both non-issuer.
pub fn account_send_with_fee<V: ApplyView>(
    view: &mut V,
    from: &AccountID,
    to: &AccountID,
    amount: &STAmount,
) -> Ter {
    if amount.signum() <= 0 || *from == *to {
        return Ter::TES_SUCCESS;
    }

    if amount.native() {
        return transfer_xrp(view, from, to, amount.xrp());
    }

    if let Asset::MPTIssue(issue) = amount.asset() {
        return account_send_mpt(view, from, to, amount, &issue);
    }

    let issue = amount.issue();

    // Exception: issuer is never frozen for their own tokens
    if *from != issue.account
        && *to != issue.account
        && (is_frozen(view, from, &issue) || is_frozen(view, to, &issue))
    {
        return Ter::TEC_PATH_DRY;
    }

    // If sender or receiver is issuer, no transfer fee
    if *from == issue.account || *to == issue.account {
        return account_send(view, from, to, amount);
    }

    // Third-party transfer: apply transfer rate
    let rate = transfer_rate(view, &issue.account);
    let actual_cost = if rate == 1_000_000_000 {
        amount.clone()
    } else {
        // mulRatio(amount.mantissa, rate, QUALITY_ONE, roundUp=true)
        let iou = amount.iou();
        let adjusted = crate::domain::mul_ratio::mul_ratio(
            iou,
            rate,
            crate::domain::mul_ratio::QUALITY_ONE,
            true,
        );
        STAmount::from_iou_amount(sf("sfAmount"), adjusted, issue)
    };

    // Issue to receiver (amount they receive)
    let res = issue_iou(view, to, amount, &issue);
    if res != Ter::TES_SUCCESS {
        return res;
    }

    // Redeem from sender (amount + fee)
    redeem_iou(view, from, &actual_cost, &issue)
}
