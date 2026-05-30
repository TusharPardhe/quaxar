//! `OfferCreate` transactor port from `xrpld/src/libxrpl/tx/transactors/dex/the reference source`.

use crate::{
    ApplyContext, ApplyResult, PreclaimContext, PreflightContext, TransactorPreflight0Facts,
    TransactorPreflight1Facts, TransactorPreflight2Facts,
    dex::{BookStepImpl, flow_cross},
    run_transactor_preflight0, run_transactor_preflight1, run_transactor_preflight2,
};
use basics::base_uint::Uint160;
use ledger::views::apply_view::ApplyView;
use ledger::views::bridge::{adjust_owner_count, dir_append, dir_insert};
use protocol::{
    Book, Keylet, LedgerEntryType, NotTec, SField, STAmount, STLedgerEntry, STObject, Ter,
    feature_batch, get_field_by_symbol, is_tes_success,
};
use std::sync::Arc;

pub const TF_PASSIVE: u32 = 0x0001_0000;
pub const TF_IMMEDIATE_OR_CANCEL: u32 = 0x0002_0000;
pub const TF_FILL_OR_KILL: u32 = 0x0004_0000;
pub const TF_SELL: u32 = 0x0008_0000;

pub const OFFER_CREATE_FLAGS_MASK: u32 =
    !(TF_PASSIVE | TF_IMMEDIATE_OR_CANCEL | TF_FILL_OR_KILL | TF_SELL);

fn sf(name: &str) -> &'static SField {
    get_field_by_symbol(name)
}

pub struct OfferCreatePreflightFacts {
    pub taker_pays: STAmount,
    pub taker_gets: STAmount,
    pub expiration: Option<u32>,
    pub flags: u32,
}

pub fn run_offer_create_preflight<Registry, Tx, Journal, ParentBatchId>(
    ctx: &PreflightContext<Registry, Tx, Journal, ParentBatchId>,
    facts: OfferCreatePreflightFacts,
) -> NotTec {
    let ret = run_transactor_preflight1(
        TransactorPreflight1Facts {
            inner_batch_flag_set: (ctx.flags.bits() & crate::ApplyFlags::BATCH.bits()) != 0,
            batch_enabled: ctx.rules.enabled(&feature_batch()),
            ..Default::default()
        },
        || {
            run_transactor_preflight0(
                TransactorPreflight0Facts {
                    tx_flags: facts.flags,
                    ..Default::default()
                },
                OFFER_CREATE_FLAGS_MASK,
            )
        },
        || Ter::TES_SUCCESS,
    );

    if !is_tes_success(ret) {
        return ret;
    }

    if !facts.taker_pays.is_legal_net() || !facts.taker_gets.is_legal_net() {
        return Ter::TEM_BAD_AMOUNT;
    }

    if facts.taker_pays.negative() || facts.taker_gets.negative() {
        return Ter::TEM_BAD_AMOUNT;
    }

    if facts.taker_pays.asset() == facts.taker_gets.asset() {
        return Ter::TEM_BAD_OFFER;
    }

    run_transactor_preflight2(
        TransactorPreflight2Facts {
            ..Default::default()
        },
        || None,
        || crate::Validity::Valid,
    )
}

pub struct OfferCreatePreclaimFacts {
    pub account: protocol::AccountID,
    pub taker_pays: STAmount,
    pub taker_gets: STAmount,
    pub cancel_sequence: Option<u32>,
    pub account_sequence: u32,
    pub expiration: Option<u32>,
    pub has_domain_id: bool,
    pub domain_id: Option<basics::base_uint::Uint256>,
    pub is_global_frozen_pays: bool,
    pub is_global_frozen_gets: bool,
    pub account_funds_zero: bool,
    pub is_taker_gets_mpt_issuer: bool,
}

pub fn run_offer_create_preclaim<Registry, View, Tx, Journal, ParentBatchId>(
    _ctx: &PreclaimContext<Registry, View, Tx, Journal, ParentBatchId>,
    facts: OfferCreatePreclaimFacts,
) -> Ter {
    if facts.is_global_frozen_pays || facts.is_global_frozen_gets {
        return Ter::TEC_FROZEN;
    }

    if !facts.is_taker_gets_mpt_issuer && facts.account_funds_zero {
        return Ter::TEC_UNFUNDED_OFFER;
    }

    if let Some(cancel_seq) = facts.cancel_sequence {
        if facts.account_sequence <= cancel_seq {
            return Ter::TEM_BAD_SEQUENCE;
        }
    }

    Ter::TES_SUCCESS
}

pub fn run_offer_create_do_apply<Registry, BaseView, View, Fee, Journal, ParentBatchId>(
    ctx: &mut ApplyContext<Registry, BaseView, View, STObject, Fee, Journal, ParentBatchId>,
) -> ApplyResult
where
    View: ApplyView,
{
    let tx = ctx.tx.clone();
    let account = tx.get_account_id(sf("sfAccount"));
    let taker_pays = tx.get_field_amount(sf("sfTakerPays"));
    let taker_gets = tx.get_field_amount(sf("sfTakerGets"));
    let flags = tx.get_field_u32(sf("sfFlags"));
    let sequence = tx.get_field_u32(sf("sfSequence"));
    let is_passive = (flags & TF_PASSIVE) != 0;

    // Cancel existing offer if OfferSequence present
    if tx.has_field(sf("sfOfferSequence")) {
        let cancel_seq = tx.get_field_u32(sf("sfOfferSequence"));
        let cancel_keylet = protocol::offer_keylet(Uint160::from_void(account.data()), cancel_seq);
        if let Ok(Some(old_offer)) = ctx.view_mut().peek(cancel_keylet) {
            let _ = ctx.view_mut().erase(old_offer);
        }
    }

    let book = Book {
        r#in: taker_pays.asset(),
        out: taker_gets.asset(),
        domain: None,
    };
    let reverse_book = Book {
        r#in: taker_gets.asset(),
        out: taker_pays.asset(),
        domain: None,
    };

    // Cross offers (skip for passive offers — they don't cross)
    let (remaining_pays, remaining_gets) = if is_passive {
        (taker_pays.clone(), taker_gets.clone())
    } else {
        let mut book_step = BookStepImpl::new(reverse_book);
        let cross_result = match flow_cross(
            ctx.view_mut(),
            &mut book_step,
            taker_pays.clone(),
            taker_gets.clone(),
        ) {
            Ok(res) => res,
            Err(_) => return ApplyResult::new(Ter::TEF_FAILURE, false, false),
        };
        (
            taker_pays.clone() - cross_result.taker_pays.clone(),
            taker_gets.clone() - cross_result.taker_gets.clone(),
        )
    };

    // Handle FOK/IOC flags
    if (flags & TF_FILL_OR_KILL) != 0 && remaining_pays.signum() > 0 {
        return ApplyResult::new(Ter::TEC_KILLED, false, false);
    }

    if (flags & TF_IMMEDIATE_OR_CANCEL) != 0 || remaining_pays.signum() <= 0 {
        return ApplyResult::new(Ter::TES_SUCCESS, true, false);
    }

    // Create residual offer
    let offer_keylet = protocol::offer_keylet(Uint160::from_void(account.data()), sequence);
    let offer_index = offer_keylet.key;
    let mut offer_sle = STLedgerEntry::new(Keylet {
        entry_type: LedgerEntryType::Offer,
        key: offer_index,
    });

    offer_sle.set_field_h160(sf("sfAccount"), Uint160::from_void(account.data()));
    offer_sle.set_field_u32(sf("sfSequence"), sequence);
    offer_sle.set_field_amount(sf("sfTakerPays"), remaining_pays);
    offer_sle.set_field_amount(sf("sfTakerGets"), remaining_gets);
    if is_passive {
        offer_sle.set_field_u32(sf("sfFlags"), 0x0001_0000); // lsfPassive
    }

    // Insert into owner directory and record the page
    let owner_dir = protocol::owner_dir_keylet(Uint160::from_void(account.data()));
    match dir_append(ctx.view_mut(), &owner_dir, offer_index, &|_| {}) {
        Ok(Some(owner_page)) => {
            offer_sle.set_field_u64(sf("sfOwnerNode"), owner_page);
        }
        _ => return ApplyResult::new(Ter::TEF_FAILURE, false, false),
    }

    // Insert into book directory and record the page
    let book_keylet = protocol::book_keylet(book);
    let book_dir = Keylet {
        entry_type: LedgerEntryType::DirectoryNode,
        key: book_keylet.key,
    };
    match dir_insert(ctx.view_mut(), &book_dir, offer_index, &|_| {}) {
        Ok(Some(book_page)) => {
            offer_sle.set_field_u64(sf("sfBookNode"), book_page);
        }
        _ => return ApplyResult::new(Ter::TEF_FAILURE, false, false),
    }

    // Adjust owner count
    if let Ok(Some(acct_sle)) = ctx
        .view_mut()
        .peek(protocol::account_keylet(Uint160::from_void(account.data())))
    {
        let _ = adjust_owner_count(ctx.view_mut(), &acct_sle, 1);
    }

    if ctx.view_mut().insert(Arc::new(offer_sle)).is_err() {
        return ApplyResult::new(Ter::TEF_FAILURE, false, false);
    }

    ApplyResult::new(Ter::TES_SUCCESS, true, false)
}
