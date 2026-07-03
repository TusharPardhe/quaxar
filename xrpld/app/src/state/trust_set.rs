//! Full TrustSet transactor — reference the reference implementation parity.
//!
//! Handles:
//! - Existing trust line modification (limits, quality, flags)
//! - NoRipple / Freeze / DeepFreeze / Auth flag management
//! - Reserve tracking (lsfLowReserve / lsfHighReserve)
//! - Owner count adjustment on reserve changes
//! - Trust line deletion when both sides at defaults
//! - Trust line creation with owner directory insertion
//! - Reserve check before creation

use basics::math::base_uint::Uint160;
use protocol::{AccountID, STAmount, STLedgerEntry, STObject, STTx, Ter, get_field_by_symbol};
use std::sync::Arc;

fn sf(name: &str) -> &'static protocol::SField {
    get_field_by_symbol(name)
}

// TrustSet flags
const TF_SET_AUTH: u32 = 0x0001_0000;
const TF_SET_NO_RIPPLE: u32 = 0x0002_0000;
const TF_CLEAR_NO_RIPPLE: u32 = 0x0004_0000;
const TF_SET_FREEZE: u32 = 0x0010_0000;
const TF_CLEAR_FREEZE: u32 = 0x0020_0000;
const TF_SET_DEEP_FREEZE: u32 = 0x0040_0000;
const TF_CLEAR_DEEP_FREEZE: u32 = 0x0080_0000;

// Ledger entry flags for RippleState
const LSF_LOW_RESERVE: u32 = 0x0001_0000;
const LSF_HIGH_RESERVE: u32 = 0x0002_0000;
const LSF_LOW_AUTH: u32 = 0x0004_0000;
const LSF_HIGH_AUTH: u32 = 0x0008_0000;
const LSF_LOW_NO_RIPPLE: u32 = 0x0010_0000;
const LSF_HIGH_NO_RIPPLE: u32 = 0x0020_0000;
const LSF_LOW_FREEZE: u32 = 0x0040_0000;
const LSF_HIGH_FREEZE: u32 = 0x0080_0000;
const LSF_LOW_DEEP_FREEZE: u32 = 0x0100_0000;
const LSF_HIGH_DEEP_FREEZE: u32 = 0x0200_0000;

// Account flags
const LSF_DEFAULT_RIPPLE: u32 = 0x0080_0000;
const LSF_NO_FREEZE: u32 = 0x0020_0000;

const QUALITY_ONE: u32 = 1_000_000_000;

/// Full reference TrustSet::doApply parity.
pub fn do_trust_set<V: ledger::ApplyView>(
    view: &mut V,
    sttx: &STTx,
    pre_fee_balance_drops: Option<i64>,
) -> Ter {
    let account = sttx.get_account_id(sf("sfAccount"));
    let limit_amount = sttx.get_field_amount(sf("sfLimitAmount"));
    let tx_flags = sttx.get_field_u32(sf("sfFlags"));
    let b_quality_in = sttx.is_field_present(sf("sfQualityIn"));
    let b_quality_out = sttx.is_field_present(sf("sfQualityOut"));

    let issue = limit_amount.issue();
    let currency = issue.currency;
    let dst_account_id = issue.account;

    // C++ preflight: cannot create trust line to self
    if account == dst_account_id {
        return Ter::TEM_DST_IS_SRC;
    }

    // bHigh: true if current account is the "high" side
    let b_high = account > dst_account_id;

    // Get account SLE
    let acct_keylet = protocol::account_keylet(Uint160::from_void(account.data()));
    let Some(sle) = view.peek(acct_keylet).ok().flatten() else {
        return Ter::TEF_INTERNAL;
    };
    let owner_count = sle.get_field_u32(sf("sfOwnerCount"));

    // Reserve: only enforce if owner_count >= 2
    let reserve_create = if owner_count < 2 {
        0i64
    } else {
        view.fees().account_reserve(owner_count as usize + 1) as i64
    };

    let quality_in = if b_quality_in {
        sttx.get_field_u32(sf("sfQualityIn"))
    } else {
        0
    };
    let mut quality_out = if b_quality_out {
        sttx.get_field_u32(sf("sfQualityOut"))
    } else {
        0
    };
    if b_quality_out && quality_out == QUALITY_ONE {
        quality_out = 0;
    }

    let b_set_auth = (tx_flags & TF_SET_AUTH) != 0;
    let b_set_no_ripple = (tx_flags & TF_SET_NO_RIPPLE) != 0;
    let b_clear_no_ripple = (tx_flags & TF_CLEAR_NO_RIPPLE) != 0;
    let b_set_freeze = (tx_flags & TF_SET_FREEZE) != 0;
    let b_clear_freeze = (tx_flags & TF_CLEAR_FREEZE) != 0;
    let b_set_deep_freeze = (tx_flags & TF_SET_DEEP_FREEZE) != 0;
    let b_clear_deep_freeze = (tx_flags & TF_CLEAR_DEEP_FREEZE) != 0;

    // Check destination exists
    let dst_keylet = protocol::account_keylet(Uint160::from_void(dst_account_id.data()));
    let sle_dst = view.peek(dst_keylet).ok().flatten();
    if sle_dst.is_none() {
        return Ter::TEC_NO_DST;
    }
    let sle_dst = sle_dst.unwrap();

    // Prepare limit with account's own issuer
    let mut limit_allow = limit_amount.clone();
    limit_allow.set_issuer(account);

    // Check if trust line exists
    let line_keylet = protocol::line(account, dst_account_id, currency);
    let existing_line = view.peek(line_keylet).ok().flatten();

    if let Some(state_sle) = existing_line {
        // --- MODIFY EXISTING TRUST LINE ---
        let mut obj = state_sle.clone_as_object();

        // Balances
        let sa_low_balance = obj.get_field_amount(sf("sfBalance"));

        // Set limit
        let limit_field = if !b_high {
            sf("sfLowLimit")
        } else {
            sf("sfHighLimit")
        };
        obj.set_field_amount(limit_field, limit_allow.clone());

        let sa_low_limit = if !b_high {
            limit_allow.clone()
        } else {
            obj.get_field_amount(sf("sfLowLimit"))
        };
        let sa_high_limit = if b_high {
            limit_allow.clone()
        } else {
            obj.get_field_amount(sf("sfHighLimit"))
        };

        // Quality In
        let mut u_low_quality_in;
        let mut u_high_quality_in;
        if !b_quality_in {
            u_low_quality_in = obj.get_field_u32(sf("sfLowQualityIn"));
            u_high_quality_in = obj.get_field_u32(sf("sfHighQualityIn"));
        } else if quality_in != 0 {
            let qi_field = if !b_high {
                sf("sfLowQualityIn")
            } else {
                sf("sfHighQualityIn")
            };
            obj.set_field_u32(qi_field, quality_in);
            u_low_quality_in = if !b_high {
                quality_in
            } else {
                obj.get_field_u32(sf("sfLowQualityIn"))
            };
            u_high_quality_in = if b_high {
                quality_in
            } else {
                obj.get_field_u32(sf("sfHighQualityIn"))
            };
        } else {
            // Clearing
            let qi_field = if !b_high {
                sf("sfLowQualityIn")
            } else {
                sf("sfHighQualityIn")
            };
            obj.set_field_u32(qi_field, 0);
            u_low_quality_in = if !b_high {
                0
            } else {
                obj.get_field_u32(sf("sfLowQualityIn"))
            };
            u_high_quality_in = if b_high {
                0
            } else {
                obj.get_field_u32(sf("sfHighQualityIn"))
            };
        }
        if u_low_quality_in == QUALITY_ONE {
            u_low_quality_in = 0;
        }
        if u_high_quality_in == QUALITY_ONE {
            u_high_quality_in = 0;
        }

        // Quality Out
        let mut u_low_quality_out;
        let mut u_high_quality_out;
        if !b_quality_out {
            u_low_quality_out = obj.get_field_u32(sf("sfLowQualityOut"));
            u_high_quality_out = obj.get_field_u32(sf("sfHighQualityOut"));
        } else if quality_out != 0 {
            let qo_field = if !b_high {
                sf("sfLowQualityOut")
            } else {
                sf("sfHighQualityOut")
            };
            obj.set_field_u32(qo_field, quality_out);
            u_low_quality_out = if !b_high {
                quality_out
            } else {
                obj.get_field_u32(sf("sfLowQualityOut"))
            };
            u_high_quality_out = if b_high {
                quality_out
            } else {
                obj.get_field_u32(sf("sfHighQualityOut"))
            };
        } else {
            let qo_field = if !b_high {
                sf("sfLowQualityOut")
            } else {
                sf("sfHighQualityOut")
            };
            obj.set_field_u32(qo_field, 0);
            u_low_quality_out = if !b_high {
                0
            } else {
                obj.get_field_u32(sf("sfLowQualityOut"))
            };
            u_high_quality_out = if b_high {
                0
            } else {
                obj.get_field_u32(sf("sfHighQualityOut"))
            };
        }
        if u_low_quality_out == QUALITY_ONE {
            u_low_quality_out = 0;
        }
        if u_high_quality_out == QUALITY_ONE {
            u_high_quality_out = 0;
        }

        // Flags
        let flags_in = obj.get_field_u32(sf("sfFlags"));
        let mut flags_out = flags_in;

        // NoRipple
        if b_set_no_ripple && !b_clear_no_ripple {
            // Can only set NoRipple if balance >= 0 on our side
            // Trust line balance is IOU — use signum, not .xrp()
            let our_balance_positive = if b_high {
                // high balance is positive when low balance is negative
                sa_low_balance.signum() <= 0
            } else {
                sa_low_balance.signum() >= 0
            };
            if our_balance_positive {
                flags_out |= if b_high {
                    LSF_HIGH_NO_RIPPLE
                } else {
                    LSF_LOW_NO_RIPPLE
                };
            } else {
                return Ter::TEC_NO_PERMISSION;
            }
        } else if b_clear_no_ripple && !b_set_no_ripple {
            flags_out &= !(if b_high {
                LSF_HIGH_NO_RIPPLE
            } else {
                LSF_LOW_NO_RIPPLE
            });
        }

        // Freeze flags
        let b_no_freeze = (sle.get_field_u32(sf("sfFlags")) & LSF_NO_FREEZE) != 0;
        flags_out = compute_freeze_flags(
            flags_out,
            b_high,
            b_no_freeze,
            b_set_freeze,
            b_clear_freeze,
            b_set_deep_freeze,
            b_clear_deep_freeze,
        );

        // Auth
        if b_set_auth {
            flags_out |= if b_high { LSF_HIGH_AUTH } else { LSF_LOW_AUTH };
        }

        // Reserve logic
        let low_acct_flags = if !b_high {
            sle.get_field_u32(sf("sfFlags"))
        } else {
            sle_dst.get_field_u32(sf("sfFlags"))
        };
        let high_acct_flags = if b_high {
            sle.get_field_u32(sf("sfFlags"))
        } else {
            sle_dst.get_field_u32(sf("sfFlags"))
        };
        let b_low_def_ripple = (low_acct_flags & LSF_DEFAULT_RIPPLE) != 0;
        let b_high_def_ripple = (high_acct_flags & LSF_DEFAULT_RIPPLE) != 0;

        let sa_high_balance_positive = sa_low_balance.signum() < 0; // high has positive balance when low is negative
        let sa_low_balance_positive = sa_low_balance.signum() > 0;

        let b_low_reserve_set = u_low_quality_in != 0
            || u_low_quality_out != 0
            || ((flags_out & LSF_LOW_NO_RIPPLE) == 0) != b_low_def_ripple
            || (flags_out & LSF_LOW_FREEZE) != 0
            || sa_low_limit.signum() > 0
            || sa_low_balance_positive;
        let b_high_reserve_set = u_high_quality_in != 0
            || u_high_quality_out != 0
            || ((flags_out & LSF_HIGH_NO_RIPPLE) == 0) != b_high_def_ripple
            || (flags_out & LSF_HIGH_FREEZE) != 0
            || sa_high_limit.signum() > 0
            || sa_high_balance_positive;

        let b_default = !b_low_reserve_set && !b_high_reserve_set;
        let b_low_reserved = (flags_in & LSF_LOW_RESERVE) != 0;
        let b_high_reserved = (flags_in & LSF_HIGH_RESERVE) != 0;
        let mut b_reserve_increase = false;

        if b_low_reserve_set && !b_low_reserved {
            // Set reserve for low account
            let low_sle = if !b_high { &sle } else { &sle_dst };
            let _ = ledger::adjust_owner_count(view, low_sle, 1);
            flags_out |= LSF_LOW_RESERVE;
            if !b_high {
                b_reserve_increase = true;
            }
        }
        if !b_low_reserve_set && b_low_reserved {
            let low_sle = if !b_high { &sle } else { &sle_dst };
            let _ = ledger::adjust_owner_count(view, low_sle, -1);
            flags_out &= !LSF_LOW_RESERVE;
        }
        if b_high_reserve_set && !b_high_reserved {
            let high_sle = if b_high { &sle } else { &sle_dst };
            let _ = ledger::adjust_owner_count(view, high_sle, 1);
            flags_out |= LSF_HIGH_RESERVE;
            if b_high {
                b_reserve_increase = true;
            }
        }
        if !b_high_reserve_set && b_high_reserved {
            let high_sle = if b_high { &sle } else { &sle_dst };
            let _ = ledger::adjust_owner_count(view, high_sle, -1);
            flags_out &= !LSF_HIGH_RESERVE;
        }

        if flags_in != flags_out {
            obj.set_field_u32(sf("sfFlags"), flags_out);
        }

        if b_default {
            // Delete the trust line
            trust_delete(view, &state_sle, &account, &dst_account_id)
        } else if b_reserve_increase {
            let balance = pre_fee_balance_drops
                .unwrap_or_else(|| sle.get_field_amount(sf("sfBalance")).xrp().drops());
            if balance < reserve_create {
                Ter::TEC_INSUF_RESERVE_LINE
            } else {
                let _ = view.update(Arc::new(STLedgerEntry::from_stobject(
                    obj,
                    *state_sle.key(),
                )));
                Ter::TES_SUCCESS
            }
        } else {
            let _ = view.update(Arc::new(STLedgerEntry::from_stobject(
                obj,
                *state_sle.key(),
            )));
            Ter::TES_SUCCESS
        }
    } else {
        // --- CREATE NEW TRUST LINE ---

        // Redundancy check
        if limit_amount.signum() <= 0
            && (!b_quality_in || quality_in == 0)
            && (!b_quality_out || quality_out == 0)
            && !b_set_auth
        {
            return Ter::TEC_NO_LINE_REDUNDANT;
        }

        // Reserve check
        let balance = pre_fee_balance_drops
            .unwrap_or_else(|| sle.get_field_amount(sf("sfBalance")).xrp().drops());
        if balance < reserve_create {
            return Ter::TEC_NO_LINE_INSUF_RESERVE;
        }

        // Create the RippleState SLE
        trust_create(
            view,
            b_high,
            &account,
            &dst_account_id,
            line_keylet.key,
            &sle,
            b_set_auth,
            b_set_no_ripple && !b_clear_no_ripple,
            b_set_freeze && !b_clear_freeze,
            b_set_deep_freeze,
            &limit_allow,
            quality_in,
            quality_out,
        )
    }
}

fn compute_freeze_flags(
    mut flags: u32,
    b_high: bool,
    b_no_freeze: bool,
    b_set_freeze: bool,
    b_clear_freeze: bool,
    b_set_deep_freeze: bool,
    b_clear_deep_freeze: bool,
) -> u32 {
    if b_set_freeze && !b_clear_freeze && !b_no_freeze {
        flags |= if b_high {
            LSF_HIGH_FREEZE
        } else {
            LSF_LOW_FREEZE
        };
    } else if b_clear_freeze && !b_set_freeze {
        flags &= !(if b_high {
            LSF_HIGH_FREEZE
        } else {
            LSF_LOW_FREEZE
        });
    }
    if b_set_deep_freeze && !b_clear_deep_freeze && !b_no_freeze {
        flags |= if b_high {
            LSF_HIGH_DEEP_FREEZE
        } else {
            LSF_LOW_DEEP_FREEZE
        };
    } else if b_clear_deep_freeze && !b_set_deep_freeze {
        flags &= !(if b_high {
            LSF_HIGH_DEEP_FREEZE
        } else {
            LSF_LOW_DEEP_FREEZE
        });
    }
    flags
}

fn trust_create<V: ledger::ApplyView>(
    view: &mut V,
    b_high: bool,
    account: &AccountID,
    dst: &AccountID,
    key: basics::math::base_uint::Uint256,
    account_sle: &Arc<STLedgerEntry>,
    b_auth: bool,
    b_no_ripple: bool,
    b_freeze: bool,
    b_deep_freeze: bool,
    limit_allow: &STAmount,
    quality_in: u32,
    quality_out: u32,
) -> Ter {
    let mut obj = STObject::new(sf("sfLedgerEntry"));
    obj.set_field_u16(sf("sfLedgerEntryType"), 0x0072); // ltRIPPLE_STATE

    // Zero balance in the currency
    let zero_balance = limit_allow.zeroed();
    obj.set_field_amount(sf("sfBalance"), zero_balance);

    // LowLimit issuer = low account, HighLimit issuer = high account
    // limit_allow has issuer = account (the trust line creator)
    // The OTHER side's limit gets a zero amount with the OTHER account as issuer
    let mut peer_limit = limit_allow.zeroed();
    peer_limit.set_issuer(*dst);

    if !b_high {
        // account is low, dst is high
        obj.set_field_amount(sf("sfLowLimit"), limit_allow.clone());
        obj.set_field_amount(sf("sfHighLimit"), peer_limit);
    } else {
        // account is high, dst is low
        obj.set_field_amount(sf("sfLowLimit"), peer_limit);
        obj.set_field_amount(sf("sfHighLimit"), limit_allow.clone());
    }

    if quality_in != 0 {
        let qi_field = if !b_high {
            sf("sfLowQualityIn")
        } else {
            sf("sfHighQualityIn")
        };
        obj.set_field_u32(qi_field, quality_in);
    }
    if quality_out != 0 {
        let qo_field = if !b_high {
            sf("sfLowQualityOut")
        } else {
            sf("sfHighQualityOut")
        };
        obj.set_field_u32(qo_field, quality_out);
    }

    // Flags
    let mut flags = 0u32;
    if b_auth {
        flags |= if b_high { LSF_HIGH_AUTH } else { LSF_LOW_AUTH };
    }
    if b_no_ripple {
        flags |= if b_high {
            LSF_HIGH_NO_RIPPLE
        } else {
            LSF_LOW_NO_RIPPLE
        };
    }
    if b_freeze {
        flags |= if b_high {
            LSF_HIGH_FREEZE
        } else {
            LSF_LOW_FREEZE
        };
    }
    if b_deep_freeze {
        flags |= if b_high {
            LSF_HIGH_DEEP_FREEZE
        } else {
            LSF_LOW_DEEP_FREEZE
        };
    }

    // Reserve: the creator always gets reserve
    flags |= if b_high {
        LSF_HIGH_RESERVE
    } else {
        LSF_LOW_RESERVE
    };

    // set NoRipple on their side of the trust line.
    let dst_keylet = protocol::account_keylet(Uint160::from_void(dst.data()));
    if let Ok(Some(dst_sle)) = view.read(dst_keylet) {
        let dst_flags = dst_sle.get_field_u32(sf("sfFlags"));
        if (dst_flags & protocol::lsfDefaultRipple) == 0 {
            flags |= if b_high {
                LSF_LOW_NO_RIPPLE
            } else {
                LSF_HIGH_NO_RIPPLE
            };
        }
    }

    let _ = ledger::adjust_owner_count(view, account_sle, 1);

    if flags != 0 {
        obj.set_field_u32(sf("sfFlags"), flags);
    }

    let new_sle = Arc::new(STLedgerEntry::from_stobject(obj, key));

    // Insert into both owner directories
    let low_account = if !b_high { account } else { dst };
    let high_account = if b_high { account } else { dst };

    let low_dir = protocol::owner_dir_keylet(Uint160::from_void(low_account.data()));
    let _ = ledger::dir_append(view, &low_dir, key, &|_| {});

    let high_dir = protocol::owner_dir_keylet(Uint160::from_void(high_account.data()));
    let _ = ledger::dir_append(view, &high_dir, key, &|_| {});

    let _ = view.insert(new_sle);

    Ter::TES_SUCCESS
}

pub(crate) fn trust_delete<V: ledger::ApplyView>(
    view: &mut V,
    state_sle: &Arc<STLedgerEntry>,
    account: &AccountID,
    dst: &AccountID,
) -> Ter {
    let key = *state_sle.key();
    let flags = state_sle.get_field_u32(sf("sfFlags"));

    // Remove from low owner directory
    let low_node = state_sle.get_field_u64(sf("sfLowNode"));
    let low_dir = protocol::owner_dir_keylet(Uint160::from_void(
        if *account < *dst { account } else { dst }.data(),
    ));
    let _ = ledger::dir_remove(view, &low_dir, low_node, key, false);

    // Remove from high owner directory
    let high_node = state_sle.get_field_u64(sf("sfHighNode"));
    let high_dir = protocol::owner_dir_keylet(Uint160::from_void(
        if *account > *dst { account } else { dst }.data(),
    ));
    let _ = ledger::dir_remove(view, &high_dir, high_node, key, false);

    // Adjust owner counts
    let low_account = if *account < *dst { account } else { dst };
    let high_account = if *account > *dst { account } else { dst };

    if (flags & LSF_LOW_RESERVE) != 0 {
        let low_keylet = protocol::account_keylet(Uint160::from_void(low_account.data()));
        if let Ok(Some(low_sle)) = view.peek(low_keylet) {
            let _ = ledger::adjust_owner_count(view, &low_sle, -1);
        }
    }
    if (flags & LSF_HIGH_RESERVE) != 0 {
        let high_keylet = protocol::account_keylet(Uint160::from_void(high_account.data()));
        if let Ok(Some(high_sle)) = view.peek(high_keylet) {
            let _ = ledger::adjust_owner_count(view, &high_sle, -1);
        }
    }

    let _ = view.erase(state_sle.clone());
    Ter::TES_SUCCESS
}
