//! `SponsorshipSet` and `SponsorshipTransfer`, ported from pinned rippled.
use basics::base_uint::Uint160;
use ledger::{ApplyView, ReadView};
use protocol::{AccountID, LedgerEntryType, STLedgerEntry, STTx, Ter, TxType, get_field_by_symbol};
use std::sync::Arc;

fn sf(name: &str) -> &'static protocol::SField {
    get_field_by_symbol(name)
}
fn account_key(account: AccountID) -> protocol::Keylet {
    protocol::account_keylet(Uint160::from_void(account.data()))
}
fn sponsorship_key(sponsor: AccountID, sponsee: AccountID) -> protocol::Keylet {
    protocol::sponsorship_keylet(
        Uint160::from_void(sponsor.data()),
        Uint160::from_void(sponsee.data()),
    )
}
fn read<V: ReadView>(
    view: &V,
    key: protocol::Keylet,
) -> Result<Option<Arc<STLedgerEntry>>, ledger::ViewError> {
    view.read(key)
}
fn peek<V: ApplyView>(
    view: &mut V,
    key: protocol::Keylet,
) -> Result<Option<Arc<STLedgerEntry>>, Ter> {
    view.peek(key).map_err(|_| Ter::TEF_BAD_LEDGER)
}

fn parties(tx: &STTx) -> (AccountID, AccountID) {
    let account = tx.get_account_id(sf("sfAccount"));
    let sponsor = if tx.is_field_present(sf("sfCounterpartySponsor")) {
        tx.get_account_id(sf("sfCounterpartySponsor"))
    } else {
        account
    };
    let sponsee = if tx.is_field_present(sf("sfSponsee")) {
        tx.get_account_id(sf("sfSponsee"))
    } else {
        account
    };
    (sponsor, sponsee)
}

fn supported_object(sle: &STLedgerEntry) -> bool {
    matches!(
        sle.get_type(),
        LedgerEntryType::Check
            | LedgerEntryType::Escrow
            | LedgerEntryType::PayChannel
            | LedgerEntryType::MPToken
            | LedgerEntryType::Delegate
            | LedgerEntryType::DepositPreauth
            | LedgerEntryType::MPTokenIssuance
            | LedgerEntryType::SignerList
            | LedgerEntryType::Credential
            | LedgerEntryType::RippleState
    )
}
fn object_owner<V: ReadView>(view: &V, sle: &STLedgerEntry, owner: AccountID) -> Result<bool, Ter> {
    Ok(match sle.get_type() {
        LedgerEntryType::Check
        | LedgerEntryType::Escrow
        | LedgerEntryType::PayChannel
        | LedgerEntryType::MPToken
        | LedgerEntryType::Delegate
        | LedgerEntryType::DepositPreauth => sle.get_account_id(sf("sfAccount")) == owner,
        LedgerEntryType::MPTokenIssuance => sle.get_account_id(sf("sfIssuer")) == owner,
        LedgerEntryType::SignerList => read(
            view,
            protocol::signers_keylet(Uint160::from_void(owner.data())),
        )
        .map_err(|_| Ter::TEF_BAD_LEDGER)?
        .is_some_and(|s| s.key() == sle.key()),
        LedgerEntryType::Credential => {
            let f = if sle.get_field_u32(sf("sfFlags")) & protocol::lsfAccepted != 0 {
                sf("sfSubject")
            } else {
                sf("sfIssuer")
            };
            sle.get_account_id(f) == owner
        }
        LedgerEntryType::RippleState => {
            (sle.get_field_u32(sf("sfFlags")) & protocol::lsfHighReserve != 0
                && sle.get_field_amount(sf("sfHighLimit")).issue().account == owner)
                || (sle.get_field_u32(sf("sfFlags")) & protocol::lsfLowReserve != 0
                    && sle.get_field_amount(sf("sfLowLimit")).issue().account == owner)
        }
        _ => false,
    })
}
fn sponsor_field(sle: &STLedgerEntry, owner: AccountID) -> &'static protocol::SField {
    if sle.get_type() == LedgerEntryType::RippleState {
        if sle.get_field_u32(sf("sfFlags")) & protocol::lsfHighReserve != 0
            && sle.get_field_amount(sf("sfHighLimit")).issue().account == owner
        {
            sf("sfHighSponsor")
        } else {
            sf("sfLowSponsor")
        }
    } else {
        sf("sfSponsor")
    }
}
fn object_count(sle: &STLedgerEntry) -> u32 {
    match sle.get_type() {
        LedgerEntryType::Oracle => {
            if sle.get_field_array(sf("sfPriceDataSeries")).len() > 5 {
                2
            } else {
                1
            }
        }
        LedgerEntryType::Vault => 2,
        LedgerEntryType::SignerList => {
            if sle.get_field_u32(sf("sfFlags")) & protocol::lsfOneOwnerCount != 0 {
                1
            } else {
                2 + sle.get_field_array(sf("sfSignerEntries")).len() as u32
            }
        }
        _ => 1,
    }
}

pub fn preclaim<V: ReadView>(view: &V, tx: &STTx) -> Ter {
    if tx.get_txn_type() == TxType::SPONSORSHIP_SET {
        let (sponsor, sponsee) = parties(tx);
        let sa = match read(view, account_key(sponsor)) {
            Ok(Some(sle)) => sle,
            Ok(None) => return Ter::TEC_NO_DST,
            Err(_) => return Ter::TEF_BAD_LEDGER,
        };
        let se = match read(view, account_key(sponsee)) {
            Ok(Some(sle)) => sle,
            Ok(None) => return Ter::TEC_NO_DST,
            Err(_) => return Ter::TEF_BAD_LEDGER,
        };
        if ledger::is_pseudo_account(&sa) || ledger::is_pseudo_account(&se) {
            return Ter::TEC_PSEUDO_ACCOUNT;
        }
        let existing = match read(view, sponsorship_key(sponsor, sponsee)) {
            Ok(sle) => sle,
            Err(_) => return Ter::TEF_BAD_LEDGER,
        };
        if tx.get_flags() & protocol::DELETE_OBJECT_FLAG != 0 {
            return if existing.is_some() {
                Ter::TES_SUCCESS
            } else {
                Ter::TEC_NO_ENTRY
            };
        }
        // Pinned hasSponsorshipBudget requires each budget delta supplied
        // while creating a new Sponsorship to be strictly positive. An
        // otherwise-positive owner budget must not mask a negative fee delta,
        // nor may a positive fee budget mask a negative owner-count delta.
        if existing.is_none() {
            if tx.is_field_present(sf("sfFeeAmountDelta"))
                && tx.get_field_amount(sf("sfFeeAmountDelta")).signum() <= 0
            {
                return Ter::TEC_NO_PERMISSION;
            }
            if tx.is_field_present(sf("sfRemainingOwnerCountDelta"))
                && tx.get_field_i32(sf("sfRemainingOwnerCountDelta")) <= 0
            {
                return Ter::TEC_NO_PERMISSION;
            }
        }
        let old_count = existing
            .as_ref()
            .map(|s| s.get_field_u32(sf("sfRemainingOwnerCount")))
            .unwrap_or(0) as i64;
        let delta = if tx.is_field_present(sf("sfRemainingOwnerCountDelta")) {
            tx.get_field_i32(sf("sfRemainingOwnerCountDelta")) as i64
        } else {
            0
        };
        let new_count = old_count + delta;
        if new_count > u32::MAX as i64 {
            return Ter::TEC_LIMIT_EXCEEDED;
        }
        let old_fee = existing
            .as_ref()
            .filter(|s| s.is_field_present(sf("sfFeeAmount")))
            .map(|s| s.get_field_amount(sf("sfFeeAmount")).xrp().drops())
            .unwrap_or(0);
        let fee_delta = if tx.is_field_present(sf("sfFeeAmountDelta")) {
            tx.get_field_amount(sf("sfFeeAmountDelta")).xrp().drops()
        } else {
            0
        };
        let Some(new_fee) = old_fee.checked_add(fee_delta) else {
            return Ter::TEC_INTERNAL;
        };
        if new_fee <= 0 && new_count <= 0 {
            return Ter::TEC_NO_PERMISSION;
        }
        Ter::TES_SUCCESS
    } else {
        let account = tx.get_account_id(sf("sfAccount"));
        let sponsee = if tx.is_field_present(sf("sfSponsee")) {
            tx.get_account_id(sf("sfSponsee"))
        } else {
            account
        };
        let sponsee_root = match read(view, account_key(sponsee)) {
            Ok(Some(sle)) => sle,
            Ok(None) => {
                return if tx.is_field_present(sf("sfSponsee")) {
                    Ter::TER_NO_ACCOUNT
                } else {
                    Ter::TEC_INTERNAL
                };
            }
            Err(_) => return Ter::TEF_BAD_LEDGER,
        };
        let target = if tx.is_field_present(sf("sfObjectID")) {
            let key =
                protocol::Keylet::new(LedgerEntryType::Any, tx.get_field_h256(sf("sfObjectID")));
            let s = match read(view, key) {
                Ok(Some(sle)) => sle,
                Ok(None) => return Ter::TEC_NO_ENTRY,
                Err(_) => return Ter::TEF_BAD_LEDGER,
            };
            if !supported_object(&s) {
                return Ter::TEC_NO_PERMISSION;
            }
            match object_owner(view, &s, sponsee) {
                Ok(true) => {}
                Ok(false) => return Ter::TEC_NO_PERMISSION,
                Err(ter) => return ter,
            }
            s
        } else {
            sponsee_root
        };
        let field = sponsor_field(&target, sponsee);
        let sponsored = target.is_field_present(field);
        let create = tx.get_flags() & protocol::SPONSORSHIP_CREATE_FLAG != 0;
        let reassign = tx.get_flags() & protocol::SPONSORSHIP_REASSIGN_FLAG != 0;
        if create && sponsored {
            return Ter::TEC_NO_PERMISSION;
        }
        if reassign
            && (!sponsored || target.get_account_id(field) == tx.get_account_id(sf("sfSponsor")))
        {
            return Ter::TEC_NO_PERMISSION;
        }
        if !create && !reassign {
            if !sponsored {
                return Ter::TEC_NO_PERMISSION;
            }
            let old = target.get_account_id(field);
            if account != old && account != sponsee {
                return Ter::TEC_NO_PERMISSION;
            }
        }
        Ter::TES_SUCCESS
    }
}

fn update_count<V: ApplyView>(
    view: &mut V,
    sle: &Arc<STLedgerEntry>,
    field: &'static protocol::SField,
    delta: i64,
) -> Result<(), Ter> {
    let current = sle.get_field_u32(field) as i64;
    let next = current
        .checked_add(delta)
        .filter(|v| (0..=u32::MAX as i64).contains(v))
        .ok_or(Ter::TEC_INTERNAL)?;
    let mut o = sle.clone_as_object();
    o.set_field_u32(field, next as u32);
    view.update(Arc::new(STLedgerEntry::from_stobject(o, *sle.key())))
        .map_err(|_| Ter::TEF_BAD_LEDGER)
}

pub fn apply<V: ApplyView>(view: &mut V, tx: &STTx, pre_fee: Option<i64>) -> Ter {
    if tx.get_txn_type() == TxType::SPONSORSHIP_SET {
        apply_set(view, tx)
    } else {
        apply_transfer(view, tx, pre_fee)
    }
}

fn apply_set<V: ApplyView>(view: &mut V, tx: &STTx) -> Ter {
    let (sponsor, sponsee) = parties(tx);
    let key = sponsorship_key(sponsor, sponsee);
    let sponsor_root = match peek(view, account_key(sponsor)) {
        Ok(Some(sle)) => sle,
        Ok(None) => return Ter::TEC_INTERNAL,
        Err(ter) => return ter,
    };
    let existing = match peek(view, key) {
        Ok(sle) => sle,
        Err(ter) => return ter,
    };
    if tx.get_flags() & protocol::DELETE_OBJECT_FLAG != 0 {
        let Some(sle) = existing else {
            return Ter::TEC_INTERNAL;
        };
        if !ledger::dir_remove(
            view,
            &protocol::owner_dir_keylet(Uint160::from_void(sponsor.data())),
            sle.get_field_u64(sf("sfOwnerNode")),
            *sle.key(),
            false,
        )
        .unwrap_or(false)
        {
            return Ter::TEF_BAD_LEDGER;
        }
        if !ledger::dir_remove(
            view,
            &protocol::owner_dir_keylet(Uint160::from_void(sponsee.data())),
            sle.get_field_u64(sf("sfSponseeNode")),
            *sle.key(),
            false,
        )
        .unwrap_or(false)
        {
            return Ter::TEF_BAD_LEDGER;
        }
        if ledger::decrease_owner_count_for_object(view, &sponsor_root, &sle, 1).is_err() {
            return Ter::TEF_BAD_LEDGER;
        }
        if sle.is_field_present(sf("sfFeeAmount")) {
            // decreaseOwnerCountForObject may have replaced the AccountRoot;
            // reload it so the fee refund cannot overwrite its count changes.
            let refreshed_root = match peek(view, account_key(sponsor)) {
                Ok(Some(root)) => root,
                Ok(None) => return Ter::TEC_INTERNAL,
                Err(ter) => return ter,
            };
            let mut root = refreshed_root.clone_as_object();
            let b = refreshed_root
                .get_field_amount(sf("sfBalance"))
                .xrp()
                .drops()
                + sle.get_field_amount(sf("sfFeeAmount")).xrp().drops();
            root.set_field_amount(
                sf("sfBalance"),
                protocol::STAmount::new_native(b as u64, false),
            );
            if view
                .update(Arc::new(STLedgerEntry::from_stobject(
                    root,
                    *refreshed_root.key(),
                )))
                .is_err()
            {
                return Ter::TEF_BAD_LEDGER;
            }
        }
        return if view.erase(sle).is_ok() {
            Ter::TES_SUCCESS
        } else {
            Ter::TEF_BAD_LEDGER
        };
    }
    if existing.is_none() {
        let fee = if tx.is_field_present(sf("sfFeeAmountDelta")) {
            tx.get_field_amount(sf("sfFeeAmountDelta")).xrp().drops()
        } else {
            0
        };
        let balance = sponsor_root.get_field_amount(sf("sfBalance")).xrp().drops();
        let reserve = ledger::effective_account_reserve(view.fees(), &sponsor_root, 1, 0) as i64;
        if fee > balance || balance - fee < reserve {
            return Ter::TEC_UNFUNDED;
        }
        let sp_dir = protocol::owner_dir_keylet(Uint160::from_void(sponsor.data()));
        let se_dir = protocol::owner_dir_keylet(Uint160::from_void(sponsee.data()));
        let sp_node = match ledger::dir_insert(view, &sp_dir, key.key, &|o| {
            o.set_account_id(sf("sfOwner"), sponsor)
        }) {
            Ok(Some(node)) => node,
            Ok(None) => return Ter::TEC_DIR_FULL,
            Err(_) => return Ter::TEF_BAD_LEDGER,
        };
        let se_node = match ledger::dir_insert(view, &se_dir, key.key, &|o| {
            o.set_account_id(sf("sfOwner"), sponsee)
        }) {
            Ok(Some(node)) => node,
            Ok(None) => return Ter::TEC_DIR_FULL,
            Err(_) => return Ter::TEF_BAD_LEDGER,
        };
        let mut sle = STLedgerEntry::from_type_and_key(LedgerEntryType::Sponsorship, key.key);
        sle.set_account_id(sf("sfOwner"), sponsor);
        sle.set_account_id(sf("sfSponsee"), sponsee);
        sle.set_field_u64(sf("sfOwnerNode"), sp_node);
        sle.set_field_u64(sf("sfSponseeNode"), se_node);
        if fee > 0 {
            sle.set_field_amount(
                sf("sfFeeAmount"),
                protocol::STAmount::new_native(fee as u64, false),
            );
        }
        if tx.is_field_present(sf("sfMaxFee")) && tx.get_field_amount(sf("sfMaxFee")).signum() > 0 {
            sle.set_field_amount(sf("sfMaxFee"), tx.get_field_amount(sf("sfMaxFee")));
        }
        if tx.is_field_present(sf("sfRemainingOwnerCountDelta"))
            && tx.get_field_i32(sf("sfRemainingOwnerCountDelta")) > 0
        {
            sle.set_field_u32(
                sf("sfRemainingOwnerCount"),
                tx.get_field_i32(sf("sfRemainingOwnerCountDelta")) as u32,
            );
        }
        let mut flags = 0;
        if tx.get_flags() & protocol::SPONSORSHIP_SET_REQUIRE_SIGN_FOR_FEE_FLAG != 0 {
            flags |= protocol::LSF_SPONSORSHIP_REQUIRE_SIGN_FOR_FEE;
        }
        if tx.get_flags() & protocol::SPONSORSHIP_SET_REQUIRE_SIGN_FOR_RESERVE_FLAG != 0 {
            flags |= protocol::LSF_SPONSORSHIP_REQUIRE_SIGN_FOR_RESERVE;
        }
        sle.set_field_u32(sf("sfFlags"), flags);
        let mut root = sponsor_root.clone_as_object();
        let Some(owner_count) = sponsor_root
            .get_field_u32(sf("sfOwnerCount"))
            .checked_add(1)
        else {
            return Ter::TEF_BAD_LEDGER;
        };
        root.set_field_u32(sf("sfOwnerCount"), owner_count);
        root.set_field_amount(
            sf("sfBalance"),
            protocol::STAmount::new_native((balance - fee) as u64, false),
        );
        if view
            .update(Arc::new(STLedgerEntry::from_stobject(
                root,
                *sponsor_root.key(),
            )))
            .is_err()
            || view.insert(Arc::new(sle)).is_err()
        {
            return Ter::TEF_BAD_LEDGER;
        }
        return Ter::TES_SUCCESS;
    }
    let sle = existing.unwrap();
    let mut o = sle.clone_as_object();
    let mut root = sponsor_root.clone_as_object();
    let mut root_changed = false;
    if tx.is_field_present(sf("sfFeeAmountDelta")) {
        let old = if sle.is_field_present(sf("sfFeeAmount")) {
            sle.get_field_amount(sf("sfFeeAmount")).xrp().drops()
        } else {
            0
        };
        let balance = sponsor_root.get_field_amount(sf("sfBalance")).xrp().drops();
        let delta = tx
            .get_field_amount(sf("sfFeeAmountDelta"))
            .xrp()
            .drops()
            .max(-old);
        if delta > balance {
            return Ter::TEC_UNFUNDED;
        }
        let post_balance = balance - delta;
        let reserve = ledger::effective_account_reserve(view.fees(), &sponsor_root, 0, 0) as i64;
        if post_balance < reserve {
            return Ter::TEC_UNFUNDED;
        }
        let new = old + delta;
        if new == 0 {
            o.make_field_absent(sf("sfFeeAmount"));
        } else {
            o.set_field_amount(
                sf("sfFeeAmount"),
                protocol::STAmount::new_native(new as u64, false),
            );
        }
        root.set_field_amount(
            sf("sfBalance"),
            protocol::STAmount::new_native(post_balance as u64, false),
        );
        root_changed = true;
    }
    if tx.is_field_present(sf("sfMaxFee")) {
        let a = tx.get_field_amount(sf("sfMaxFee"));
        if a.signum() == 0 {
            o.make_field_absent(sf("sfMaxFee"));
        } else {
            o.set_field_amount(sf("sfMaxFee"), a);
        }
    }
    if tx.is_field_present(sf("sfRemainingOwnerCountDelta")) {
        let n = (sle.get_field_u32(sf("sfRemainingOwnerCount")) as i64
            + tx.get_field_i32(sf("sfRemainingOwnerCountDelta")) as i64)
            .max(0);
        if n == 0 {
            o.make_field_absent(sf("sfRemainingOwnerCount"));
        } else {
            o.set_field_u32(sf("sfRemainingOwnerCount"), n as u32);
        }
    }
    let mut f = sle.get_field_u32(sf("sfFlags"));
    for (set, clear, ledger_flag) in [
        (
            protocol::SPONSORSHIP_SET_REQUIRE_SIGN_FOR_FEE_FLAG,
            protocol::SPONSORSHIP_CLEAR_REQUIRE_SIGN_FOR_FEE_FLAG,
            protocol::LSF_SPONSORSHIP_REQUIRE_SIGN_FOR_FEE,
        ),
        (
            protocol::SPONSORSHIP_SET_REQUIRE_SIGN_FOR_RESERVE_FLAG,
            protocol::SPONSORSHIP_CLEAR_REQUIRE_SIGN_FOR_RESERVE_FLAG,
            protocol::LSF_SPONSORSHIP_REQUIRE_SIGN_FOR_RESERVE,
        ),
    ] {
        if tx.get_flags() & set != 0 {
            f |= ledger_flag;
        }
        if tx.get_flags() & clear != 0 {
            f &= !ledger_flag;
        }
    }
    o.set_field_u32(sf("sfFlags"), f);
    if (root_changed
        && view
            .update(Arc::new(STLedgerEntry::from_stobject(
                root,
                *sponsor_root.key(),
            )))
            .is_err())
        || view
            .update(Arc::new(STLedgerEntry::from_stobject(o, *sle.key())))
            .is_err()
    {
        Ter::TEF_BAD_LEDGER
    } else {
        Ter::TES_SUCCESS
    }
}

fn apply_transfer<V: ApplyView>(view: &mut V, tx: &STTx, pre_fee: Option<i64>) -> Ter {
    let account = tx.get_account_id(sf("sfAccount"));
    let sponsee = if tx.is_field_present(sf("sfSponsee")) {
        tx.get_account_id(sf("sfSponsee"))
    } else {
        account
    };
    let sponsee_root = match peek(view, account_key(sponsee)) {
        Ok(Some(sle)) => sle,
        Ok(None) => return Ter::TEF_INTERNAL,
        Err(ter) => return ter,
    };
    let object_id = tx
        .is_field_present(sf("sfObjectID"))
        .then(|| tx.get_field_h256(sf("sfObjectID")));
    let target = if let Some(id) = object_id {
        let s = match peek(view, protocol::Keylet::new(LedgerEntryType::Any, id)) {
            Ok(Some(sle)) => sle,
            Ok(None) => return Ter::TEF_INTERNAL,
            Err(ter) => return ter,
        };
        // Pinned doApply defensively revalidates the preclaim-proven owner
        // before mutating object sponsorship. A changed/corrupt target is an
        // internal invariant failure, not a fresh user-facing permission TER.
        match object_owner(view, &s, sponsee) {
            Ok(true) => {}
            Ok(false) => return Ter::TEF_INTERNAL,
            Err(ter) => return ter,
        }
        s
    } else {
        sponsee_root.clone()
    };
    let field = sponsor_field(&target, sponsee);
    let count = if object_id.is_some() {
        object_count(&target)
    } else {
        1
    };
    let create = tx.get_flags() & protocol::SPONSORSHIP_CREATE_FLAG != 0;
    let reassign = tx.get_flags() & protocol::SPONSORSHIP_REASSIGN_FLAG != 0;
    if create || reassign {
        let new_id = tx.get_account_id(sf("sfSponsor"));
        let new_root = match peek(view, account_key(new_id)) {
            Ok(Some(sle)) => sle,
            Ok(None) => return Ter::TEF_INTERNAL,
            Err(ter) => return ter,
        };
        let owner_delta = if object_id.is_some() { count as i32 } else { 0 };
        let account_delta = if object_id.is_none() { 1 } else { 0 };
        let prefunded = if object_id.is_some() {
            match peek(view, sponsorship_key(new_id, sponsee)) {
                Ok(sle) => sle,
                Err(ter) => return ter,
            }
        } else {
            None
        };
        if prefunded
            .as_ref()
            .is_some_and(|budget| budget.get_field_u32(sf("sfRemainingOwnerCount")) < count)
        {
            return Ter::TEC_INSUFFICIENT_RESERVE;
        }
        let required =
            ledger::effective_account_reserve(view.fees(), &new_root, owner_delta, account_delta)
                as i64;
        if new_root.get_field_amount(sf("sfBalance")).xrp().drops() < required {
            return Ter::TEC_INSUFFICIENT_RESERVE;
        }
        if reassign {
            let old_id = target.get_account_id(field);
            let old_root = match peek(view, account_key(old_id)) {
                Ok(Some(sle)) => sle,
                Ok(None) => return Ter::TEF_INTERNAL,
                Err(ter) => return ter,
            };
            if let Err(ter) = update_count(
                view,
                &old_root,
                if object_id.is_some() {
                    sf("sfSponsoringOwnerCount")
                } else {
                    sf("sfSponsoringAccountCount")
                },
                -(count as i64),
            ) {
                return ter;
            }
        }
        if object_id.is_some() {
            if create {
                if let Err(ter) = update_count(
                    view,
                    &sponsee_root,
                    sf("sfSponsoredOwnerCount"),
                    count as i64,
                ) {
                    return ter;
                }
            }
            if let Err(ter) =
                update_count(view, &new_root, sf("sfSponsoringOwnerCount"), count as i64)
            {
                return ter;
            }
            if let Some(budget) = prefunded {
                let mut b = budget.clone_as_object();
                let Some(remaining) = budget
                    .get_field_u32(sf("sfRemainingOwnerCount"))
                    .checked_sub(count)
                else {
                    return Ter::TEF_BAD_LEDGER;
                };
                b.set_field_u32(sf("sfRemainingOwnerCount"), remaining);
                if view
                    .update(Arc::new(STLedgerEntry::from_stobject(b, *budget.key())))
                    .is_err()
                {
                    return Ter::TEF_BAD_LEDGER;
                }
            }
        } else if let Err(ter) = update_count(view, &new_root, sf("sfSponsoringAccountCount"), 1) {
            return ter;
        }
        let mut o = target.clone_as_object();
        o.set_account_id(field, new_id);
        return if view
            .update(Arc::new(STLedgerEntry::from_stobject(o, *target.key())))
            .is_ok()
        {
            Ter::TES_SUCCESS
        } else {
            Ter::TEF_BAD_LEDGER
        };
    }
    let old_id = target.get_account_id(field);
    let old_root = match peek(view, account_key(old_id)) {
        Ok(Some(sle)) => sle,
        Ok(None) => return Ter::TEF_INTERNAL,
        Err(ter) => return ter,
    };
    if object_id.is_none() {
        let balance = if sponsee == account {
            let Some(pre_fee) = pre_fee else {
                return Ter::TEF_BAD_LEDGER;
            };
            pre_fee
        } else {
            sponsee_root.get_field_amount(sf("sfBalance")).xrp().drops()
        };
        let required = ledger::effective_account_reserve(view.fees(), &sponsee_root, 0, 1) as i64;
        if balance < required {
            return Ter::TEC_INSUFFICIENT_RESERVE;
        }
    }
    if object_id.is_some() {
        if let Err(ter) = update_count(
            view,
            &sponsee_root,
            sf("sfSponsoredOwnerCount"),
            -(count as i64),
        ) {
            return ter;
        }
    }
    if let Err(ter) = update_count(
        view,
        &old_root,
        if object_id.is_some() {
            sf("sfSponsoringOwnerCount")
        } else {
            sf("sfSponsoringAccountCount")
        },
        -(count as i64),
    ) {
        return ter;
    }
    let mut o = target.clone_as_object();
    o.make_field_absent(field);
    if view
        .update(Arc::new(STLedgerEntry::from_stobject(o, *target.key())))
        .is_ok()
    {
        Ter::TES_SUCCESS
    } else {
        Ter::TEF_BAD_LEDGER
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use basics::base_uint::Uint256;
    use ledger::{ApplyView, ApplyViewImpl, Ledger, ReadViewTx, Sandbox, ViewError};
    use protocol::{ApplyFlags, Issue, STAmount, STArray, currency_from_string};

    #[derive(Debug)]
    struct FaultReadView {
        base: Arc<Ledger>,
    }

    impl ReadView for FaultReadView {
        fn open(&self) -> bool {
            ReadView::open(self.base.as_ref())
        }
        fn header(&self) -> ledger::LedgerHeader {
            ReadView::header(self.base.as_ref())
        }
        fn fees(&self) -> ledger::Fees {
            ReadView::fees(self.base.as_ref())
        }
        fn rules(&self) -> protocol::Rules {
            ReadView::rules(self.base.as_ref())
        }
        fn exists(&self, key: protocol::Keylet) -> Result<bool, ViewError> {
            ReadView::exists(self.base.as_ref(), key)
        }
        fn succ(&self, key: Uint256, last: Option<Uint256>) -> Result<Option<Uint256>, ViewError> {
            ReadView::succ(self.base.as_ref(), key, last)
        }
        fn read(&self, _key: protocol::Keylet) -> Result<Option<Arc<STLedgerEntry>>, ViewError> {
            Err(ViewError::Conversion("injected read failure".into()))
        }
        fn sles(&self) -> Result<Vec<Arc<STLedgerEntry>>, ViewError> {
            ReadView::sles(self.base.as_ref())
        }
        fn tx_exists(&self, key: Uint256) -> Result<bool, ViewError> {
            ReadView::tx_exists(self.base.as_ref(), key)
        }
        fn tx_read(&self, key: Uint256) -> Result<Option<ReadViewTx>, ViewError> {
            ReadView::tx_read(self.base.as_ref(), key)
        }
        fn txs(&self) -> Result<Vec<ReadViewTx>, ViewError> {
            ReadView::txs(self.base.as_ref())
        }
    }

    fn insert_account(view: &mut Sandbox<Ledger>, id: AccountID, balance: u64) {
        let mut sle =
            STLedgerEntry::from_type_and_key(LedgerEntryType::AccountRoot, account_key(id).key);
        sle.set_account_id(sf("sfAccount"), id);
        sle.set_field_u32(sf("sfSequence"), 1);
        sle.set_field_u32(sf("sfOwnerCount"), 0);
        sle.set_field_amount(sf("sfBalance"), STAmount::new_native(balance, false));
        view.insert(Arc::new(sle)).unwrap();
    }

    #[test]
    fn sponsorship_read_failures_are_never_treated_as_missing_entries() {
        let sponsor = AccountID::from_array([0x61; 20]);
        let sponsee = AccountID::from_array([0x62; 20]);
        let tx = set_tx(sponsor, sponsee, 100, 1);
        let faulty = Arc::new(FaultReadView {
            base: Arc::new(Ledger::from_ledger_seq_and_close_time(1, 0, false)),
        });
        assert_eq!(preclaim(faulty.as_ref(), &tx), Ter::TEF_BAD_LEDGER);
        let mut apply = ApplyViewImpl::new(faulty, ApplyFlags::NONE);
        assert_eq!(super::apply(&mut apply, &tx, None), Ter::TEF_BAD_LEDGER);
    }

    #[test]
    fn sponsorship_set_new_budget_deltas_are_individually_positive() {
        let sponsor = AccountID::from_array([0x63; 20]);
        let sponsee = AccountID::from_array([0x64; 20]);
        let mut view = Sandbox::new(
            Arc::new(Ledger::from_ledger_seq_and_close_time(1, 0, false)),
            ApplyFlags::NONE,
        );
        insert_account(&mut view, sponsor, 1_000_000_000);
        insert_account(&mut view, sponsee, 1_000_000_000);

        // Pinned hasSponsorshipBudget rejects either non-positive creation
        // delta even when the other field would leave aggregate budget.
        assert_eq!(
            preclaim(&view, &set_tx(sponsor, sponsee, -1, 2)),
            Ter::TEC_NO_PERMISSION
        );
        assert_eq!(
            preclaim(&view, &set_tx(sponsor, sponsee, 100, -1)),
            Ter::TEC_NO_PERMISSION
        );
        assert_eq!(
            preclaim(&view, &set_tx(sponsor, sponsee, 100, 2)),
            Ter::TES_SUCCESS
        );
    }

    #[test]
    fn sponsorship_transfer_do_apply_fails_closed_if_object_owner_changed() {
        let owner = AccountID::from_array([0x65; 20]);
        let other = AccountID::from_array([0x66; 20]);
        let sponsor = AccountID::from_array([0x67; 20]);
        let object_key = Uint256::from_u64(0x6566);
        let mut view = Sandbox::new(
            Arc::new(Ledger::from_ledger_seq_and_close_time(1, 0, false)),
            ApplyFlags::NONE,
        );
        insert_account(&mut view, owner, 1_000_000_000);
        insert_account(&mut view, other, 1_000_000_000);
        insert_account(&mut view, sponsor, 1_000_000_000);
        view.insert(Arc::new(object_fixture(
            LedgerEntryType::Check,
            object_key,
            other,
        )))
        .unwrap();

        let tx = transfer_tx(
            owner,
            owner,
            object_key,
            protocol::SPONSORSHIP_CREATE_FLAG,
            Some(sponsor),
        );
        // A production immutable preclaim would reject this as tecNO_PERMISSION.
        // The direct apply harness models the target changing after preclaim;
        // pinned doApply classifies that invariant break as tefINTERNAL.
        assert_eq!(apply(&mut view, &tx, None), Ter::TEF_INTERNAL);
    }
    fn set_tx(sponsor: AccountID, sponsee: AccountID, fee: i64, count: i32) -> STTx {
        STTx::new(TxType::SPONSORSHIP_SET, |o| {
            o.set_account_id(sf("sfAccount"), sponsor);
            o.set_account_id(sf("sfSponsee"), sponsee);
            o.set_field_amount(sf("sfFee"), STAmount::new_native(10, false));
            o.set_field_amount(
                sf("sfFeeAmountDelta"),
                STAmount::from_xrp_amount(protocol::XRPAmount::from_drops(fee)),
            );
            o.set_field_i32(sf("sfRemainingOwnerCountDelta"), count);
        })
    }

    fn insert_budget(
        view: &mut Sandbox<Ledger>,
        sponsor: AccountID,
        sponsee: AccountID,
        count: u32,
    ) {
        let key = sponsorship_key(sponsor, sponsee);
        let mut sle = STLedgerEntry::from_type_and_key(LedgerEntryType::Sponsorship, key.key);
        sle.set_account_id(sf("sfOwner"), sponsor);
        sle.set_account_id(sf("sfSponsee"), sponsee);
        sle.set_field_u32(sf("sfRemainingOwnerCount"), count);
        view.insert(Arc::new(sle)).unwrap();
    }

    fn transfer_tx(
        account: AccountID,
        sponsee: AccountID,
        object: Uint256,
        flag: u32,
        sponsor: Option<AccountID>,
    ) -> STTx {
        STTx::new(TxType::SPONSORSHIP_TRANSFER, |o| {
            o.set_account_id(sf("sfAccount"), account);
            o.set_account_id(sf("sfSponsee"), sponsee);
            o.set_field_h256(sf("sfObjectID"), object);
            o.set_field_u32(sf("sfFlags"), flag);
            o.set_field_amount(sf("sfFee"), STAmount::new_native(10, false));
            if let Some(sponsor) = sponsor {
                o.set_account_id(sf("sfSponsor"), sponsor);
                o.set_field_u32(sf("sfSponsorFlags"), ledger::SPF_SPONSOR_RESERVE);
            }
        })
    }

    fn object_fixture(kind: LedgerEntryType, key: Uint256, owner: AccountID) -> STLedgerEntry {
        let mut sle = STLedgerEntry::from_type_and_key(kind, key);
        match kind {
            LedgerEntryType::MPTokenIssuance => sle.set_account_id(sf("sfIssuer"), owner),
            LedgerEntryType::Credential => {
                sle.set_account_id(sf("sfIssuer"), owner);
                sle.set_account_id(sf("sfSubject"), AccountID::from_array([0x77; 20]));
                sle.set_field_u32(sf("sfFlags"), 0);
            }
            LedgerEntryType::RippleState => {
                sle.set_field_u32(sf("sfFlags"), protocol::lsfHighReserve);
                sle.set_field_amount(
                    sf("sfHighLimit"),
                    STAmount::new_with_asset(
                        sf("sfHighLimit"),
                        Issue::new(currency_from_string("USD"), owner),
                        1,
                        0,
                        false,
                    ),
                );
            }
            _ => sle.set_account_id(sf("sfAccount"), owner),
        }
        sle
    }

    fn assert_transfer_cycle(kind: LedgerEntryType, object_count: u32) {
        let owner = AccountID::from_array([0x41; 20]);
        let sponsor1 = AccountID::from_array([0x42; 20]);
        let sponsor2 = AccountID::from_array([0x43; 20]);
        let mut view = Sandbox::new(
            Arc::new(Ledger::from_ledger_seq_and_close_time(1, 0, false)),
            ApplyFlags::NONE,
        );
        insert_account(&mut view, owner, 1_000_000_000);
        insert_account(&mut view, sponsor1, 1_000_000_000);
        insert_account(&mut view, sponsor2, 1_000_000_000);
        let key = if kind == LedgerEntryType::SignerList {
            protocol::signers_keylet(Uint160::from_void(owner.data())).key
        } else {
            Uint256::from_u64(10_000 + kind as u64)
        };
        let mut object = object_fixture(kind, key, owner);
        if kind == LedgerEntryType::SignerList {
            object.set_field_u32(sf("sfFlags"), protocol::lsfOneOwnerCount);
        }
        view.insert(Arc::new(object)).unwrap();
        insert_budget(&mut view, sponsor1, owner, object_count + 2);
        insert_budget(&mut view, sponsor2, owner, object_count + 2);

        let create = transfer_tx(
            owner,
            owner,
            key,
            protocol::SPONSORSHIP_CREATE_FLAG,
            Some(sponsor1),
        );
        assert_eq!(
            preclaim(&view, &create),
            Ter::TES_SUCCESS,
            "{kind:?} create preclaim"
        );
        assert_eq!(
            apply(&mut view, &create, None),
            Ter::TES_SUCCESS,
            "{kind:?} create"
        );
        let object = view
            .peek(protocol::Keylet::new(LedgerEntryType::Any, key))
            .unwrap()
            .unwrap();
        let sponsor_field = sponsor_field(&object, owner);
        assert_eq!(object.get_account_id(sponsor_field), sponsor1);
        assert_eq!(
            view.peek(account_key(owner))
                .unwrap()
                .unwrap()
                .get_field_u32(sf("sfSponsoredOwnerCount")),
            object_count
        );
        assert_eq!(
            view.peek(account_key(sponsor1))
                .unwrap()
                .unwrap()
                .get_field_u32(sf("sfSponsoringOwnerCount")),
            object_count
        );
        assert_eq!(
            view.peek(sponsorship_key(sponsor1, owner))
                .unwrap()
                .unwrap()
                .get_field_u32(sf("sfRemainingOwnerCount")),
            2
        );

        let reassign = transfer_tx(
            owner,
            owner,
            key,
            protocol::SPONSORSHIP_REASSIGN_FLAG,
            Some(sponsor2),
        );
        assert_eq!(
            preclaim(&view, &reassign),
            Ter::TES_SUCCESS,
            "{kind:?} reassign preclaim"
        );
        assert_eq!(
            apply(&mut view, &reassign, None),
            Ter::TES_SUCCESS,
            "{kind:?} reassign"
        );
        assert_eq!(
            view.peek(account_key(sponsor1))
                .unwrap()
                .unwrap()
                .get_field_u32(sf("sfSponsoringOwnerCount")),
            0
        );
        assert_eq!(
            view.peek(account_key(sponsor2))
                .unwrap()
                .unwrap()
                .get_field_u32(sf("sfSponsoringOwnerCount")),
            object_count
        );

        let end = transfer_tx(owner, owner, key, protocol::SPONSORSHIP_END_FLAG, None);
        assert_eq!(
            preclaim(&view, &end),
            Ter::TES_SUCCESS,
            "{kind:?} end preclaim"
        );
        assert_eq!(
            apply(&mut view, &end, None),
            Ter::TES_SUCCESS,
            "{kind:?} end"
        );
        let object = view
            .peek(protocol::Keylet::new(LedgerEntryType::Any, key))
            .unwrap()
            .unwrap();
        assert!(!object.is_field_present(sponsor_field));
        assert_eq!(
            view.peek(account_key(owner))
                .unwrap()
                .unwrap()
                .get_field_u32(sf("sfSponsoredOwnerCount")),
            0
        );
        assert_eq!(
            view.peek(account_key(sponsor2))
                .unwrap()
                .unwrap()
                .get_field_u32(sf("sfSponsoringOwnerCount")),
            0
        );
    }

    #[test]
    fn sponsorship_transfer_all_pinned_supported_object_types_create_reassign_and_end() {
        for kind in [
            LedgerEntryType::Check,
            LedgerEntryType::Escrow,
            LedgerEntryType::PayChannel,
            LedgerEntryType::MPToken,
            LedgerEntryType::Delegate,
            LedgerEntryType::DepositPreauth,
            LedgerEntryType::MPTokenIssuance,
            LedgerEntryType::Credential,
            LedgerEntryType::SignerList,
            LedgerEntryType::RippleState,
        ] {
            assert_transfer_cycle(kind, 1);
        }
    }

    #[test]
    fn sponsorship_object_catalog_and_reserve_weights_match_pinned_helpers() {
        let owner = AccountID::from_array([0x49; 20]);
        for kind in [LedgerEntryType::Oracle, LedgerEntryType::Vault] {
            let mut unsupported = STLedgerEntry::from_type_and_key(kind, Uint256::from_u64(90));
            unsupported.set_account_id(sf("sfAccount"), owner);
            if kind == LedgerEntryType::Oracle {
                unsupported.set_field_array(
                    sf("sfPriceDataSeries"),
                    STArray::new(sf("sfPriceDataSeries")),
                );
                assert_eq!(object_count(&unsupported), 1);
            } else {
                assert_eq!(object_count(&unsupported), 2);
            }
            assert!(!supported_object(&unsupported));
        }

        let mut low_line =
            STLedgerEntry::from_type_and_key(LedgerEntryType::RippleState, Uint256::from_u64(91));
        low_line.set_field_u32(sf("sfFlags"), protocol::lsfLowReserve);
        low_line.set_field_amount(
            sf("sfLowLimit"),
            STAmount::new_with_asset(
                sf("sfLowLimit"),
                Issue::new(currency_from_string("USD"), owner),
                1,
                0,
                false,
            ),
        );
        assert_eq!(sponsor_field(&low_line, owner), sf("sfLowSponsor"));
    }

    #[test]
    fn sponsorship_transfer_account_create_reassign_and_end_tracks_account_reserves() {
        let sponsee = AccountID::from_array([0x51; 20]);
        let sponsor1 = AccountID::from_array([0x52; 20]);
        let sponsor2 = AccountID::from_array([0x53; 20]);
        let mut view = Sandbox::new(
            Arc::new(Ledger::from_ledger_seq_and_close_time(1, 0, false)),
            ApplyFlags::NONE,
        );
        for account in [sponsee, sponsor1, sponsor2] {
            insert_account(&mut view, account, 1_000_000_000);
        }
        let account_tx = |flag: u32, sponsor: Option<AccountID>| {
            STTx::new(TxType::SPONSORSHIP_TRANSFER, |o| {
                o.set_account_id(sf("sfAccount"), sponsee);
                o.set_field_u32(sf("sfFlags"), flag);
                o.set_field_amount(sf("sfFee"), STAmount::new_native(10, false));
                if let Some(sponsor) = sponsor {
                    o.set_account_id(sf("sfSponsor"), sponsor);
                    o.set_field_u32(sf("sfSponsorFlags"), ledger::SPF_SPONSOR_RESERVE);
                }
            })
        };
        let create = account_tx(protocol::SPONSORSHIP_CREATE_FLAG, Some(sponsor1));
        assert_eq!(preclaim(&view, &create), Ter::TES_SUCCESS);
        assert_eq!(apply(&mut view, &create, None), Ter::TES_SUCCESS);
        assert_eq!(
            view.peek(account_key(sponsee))
                .unwrap()
                .unwrap()
                .get_account_id(sf("sfSponsor")),
            sponsor1
        );
        assert_eq!(
            view.peek(account_key(sponsor1))
                .unwrap()
                .unwrap()
                .get_field_u32(sf("sfSponsoringAccountCount")),
            1
        );

        let reassign = account_tx(protocol::SPONSORSHIP_REASSIGN_FLAG, Some(sponsor2));
        assert_eq!(preclaim(&view, &reassign), Ter::TES_SUCCESS);
        assert_eq!(apply(&mut view, &reassign, None), Ter::TES_SUCCESS);
        assert_eq!(
            view.peek(account_key(sponsor1))
                .unwrap()
                .unwrap()
                .get_field_u32(sf("sfSponsoringAccountCount")),
            0
        );
        assert_eq!(
            view.peek(account_key(sponsor2))
                .unwrap()
                .unwrap()
                .get_field_u32(sf("sfSponsoringAccountCount")),
            1
        );

        let end = account_tx(protocol::SPONSORSHIP_END_FLAG, None);
        assert_eq!(preclaim(&view, &end), Ter::TES_SUCCESS);
        assert_eq!(
            apply(&mut view, &end, Some(1_000_000_000)),
            Ter::TES_SUCCESS
        );
        assert!(
            !view
                .peek(account_key(sponsee))
                .unwrap()
                .unwrap()
                .is_field_present(sf("sfSponsor"))
        );
        assert_eq!(
            view.peek(account_key(sponsor2))
                .unwrap()
                .unwrap()
                .get_field_u32(sf("sfSponsoringAccountCount")),
            0
        );
    }
    #[test]
    fn sponsorship_set_create_update_delete_preserves_budget_counts_and_directories() {
        let sponsor = AccountID::from_array([0x31; 20]);
        let sponsee = AccountID::from_array([0x32; 20]);
        let mut view = Sandbox::new(
            Arc::new(Ledger::from_ledger_seq_and_close_time(1, 0, false)),
            ApplyFlags::NONE,
        );
        insert_account(&mut view, sponsor, 1_000_000_000);
        insert_account(&mut view, sponsee, 1_000_000_000);
        let mut create = set_tx(sponsor, sponsee, 100, 3);
        create.set_field_u32(
            sf("sfFlags"),
            protocol::SPONSORSHIP_SET_REQUIRE_SIGN_FOR_FEE_FLAG
                | protocol::SPONSORSHIP_SET_REQUIRE_SIGN_FOR_RESERVE_FLAG,
        );
        assert_eq!(preclaim(&view, &create), Ter::TES_SUCCESS);
        assert_eq!(apply(&mut view, &create, None), Ter::TES_SUCCESS);
        let key = sponsorship_key(sponsor, sponsee);
        let sle = view.peek(key).unwrap().unwrap();
        assert_eq!(sle.get_field_amount(sf("sfFeeAmount")).xrp().drops(), 100);
        assert_eq!(sle.get_field_u32(sf("sfRemainingOwnerCount")), 3);
        assert_eq!(
            sle.get_field_u32(sf("sfFlags")),
            protocol::LSF_SPONSORSHIP_REQUIRE_SIGN_FOR_FEE
                | protocol::LSF_SPONSORSHIP_REQUIRE_SIGN_FOR_RESERVE
        );
        assert!(sle.is_field_present(sf("sfOwnerNode")));
        assert!(sle.is_field_present(sf("sfSponseeNode")));
        assert_eq!(
            view.peek(account_key(sponsor))
                .unwrap()
                .unwrap()
                .get_field_u32(sf("sfOwnerCount")),
            1
        );
        let update = set_tx(sponsor, sponsee, -40, -1);
        assert_eq!(apply(&mut view, &update, None), Ter::TES_SUCCESS);
        let sle = view.peek(key).unwrap().unwrap();
        assert_eq!(sle.get_field_amount(sf("sfFeeAmount")).xrp().drops(), 60);
        assert_eq!(sle.get_field_u32(sf("sfRemainingOwnerCount")), 2);
        let delete = STTx::new(TxType::SPONSORSHIP_SET, |o| {
            o.set_account_id(sf("sfAccount"), sponsor);
            o.set_account_id(sf("sfSponsee"), sponsee);
            o.set_field_amount(sf("sfFee"), STAmount::new_native(10, false));
            o.set_field_u32(sf("sfFlags"), protocol::DELETE_OBJECT_FLAG);
        });
        assert_eq!(apply(&mut view, &delete, None), Ter::TES_SUCCESS);
        assert!(view.peek(key).unwrap().is_none());
        assert_eq!(
            view.peek(account_key(sponsor))
                .unwrap()
                .unwrap()
                .get_field_u32(sf("sfOwnerCount")),
            0
        );
    }
}
