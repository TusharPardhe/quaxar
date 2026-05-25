//! `xrpld/app/misc/FeeVote.*` compatibility surface.
//!
//! This ports the fee-voting helper that:
//! - publishes local fee preferences into validations,
//! - aggregates trusted parent validations on flag ledgers,
//! - and injects a `ttFEE` pseudo-transaction into the consensus tx-set when
//!   the vote should move the network away from the current fee schedule.

use std::collections::BTreeMap;

use ledger::{Fees, INITIAL_XRP_DROPS, Ledger};
use protocol::{
    REFERENCE_FEE_UNITS_DEPRECATED, Rules, STAmount, STTx, STValidation, TxType, XRPAmount,
    feature_xrp_fees, get_field_by_symbol,
};

use crate::tx_queue::vote_tx_set::VoteTxSet;

pub trait FeeVoteJournal {
    fn info(&self, message: &str);
    fn warn(&self, message: &str);
}

#[derive(Debug, Default, Clone, Copy)]
pub struct NullFeeVoteJournal;

impl FeeVoteJournal for NullFeeVoteJournal {
    fn info(&self, _message: &str) {}

    fn warn(&self, _message: &str) {}
}

pub trait FeeVoteLedgerView {
    fn fees(&self) -> Fees;
    fn rules(&self) -> &Rules;
    fn seq(&self) -> u32;
    fn is_flag_ledger(&self) -> bool;
}

impl FeeVoteLedgerView for Ledger {
    fn fees(&self) -> Fees {
        Ledger::fees(self)
    }

    fn rules(&self) -> &Rules {
        Ledger::rules(self)
    }

    fn seq(&self) -> u32 {
        self.header().seq
    }

    fn is_flag_ledger(&self) -> bool {
        self.is_flag_ledger()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FeeSetup {
    pub reference_fee: XRPAmount,
    pub account_reserve: XRPAmount,
    pub owner_reserve: XRPAmount,
}

impl Default for FeeSetup {
    fn default() -> Self {
        Self {
            reference_fee: XRPAmount::from_drops(10),
            account_reserve: XRPAmount::from_drops(1_000_000), // 1 XRP — default fee setup
            owner_reserve: XRPAmount::from_drops(200_000),     // 0.2 XRP — default fee setup
        }
    }
}

impl FeeSetup {
    pub fn to_fees(self) -> Fees {
        Fees {
            base: self
                .reference_fee
                .drops_as()
                .expect("reference fee must fit u64"),
            reserve: self
                .account_reserve
                .drops_as()
                .expect("account reserve must fit u64"),
            increment: self
                .owner_reserve
                .drops_as()
                .expect("owner reserve must fit u64"),
        }
    }
}

#[derive(Debug, Clone)]
struct VotableValue {
    current: XRPAmount,
    target: XRPAmount,
    vote_map: BTreeMap<XRPAmount, usize>,
}

impl VotableValue {
    fn new(current: XRPAmount, target: XRPAmount) -> Self {
        let mut vote_map = BTreeMap::new();
        *vote_map.entry(target).or_default() += 1;
        Self {
            current,
            target,
            vote_map,
        }
    }

    fn add_vote(&mut self, vote: XRPAmount) {
        *self.vote_map.entry(vote).or_default() += 1;
    }

    fn no_vote(&mut self) {
        self.add_vote(self.current);
    }

    fn current(&self) -> XRPAmount {
        self.current
    }

    fn get_votes(&self) -> (XRPAmount, bool) {
        let low = self.current.min(self.target);
        let high = self.current.max(self.target);
        let mut our_vote = self.current;
        let mut weight = 0usize;

        for (&value, &count) in &self.vote_map {
            if value >= low && value <= high && count > weight {
                our_vote = value;
                weight = count;
            }
        }

        (our_vote, our_vote != self.current)
    }
}

#[derive(Debug, Clone)]
pub struct FeeVote<J = NullFeeVoteJournal> {
    target: FeeSetup,
    journal: J,
}

impl<J> FeeVote<J>
where
    J: FeeVoteJournal,
{
    pub fn new(target: FeeSetup, journal: J) -> Self {
        Self { target, journal }
    }

    pub fn target(&self) -> FeeSetup {
        self.target
    }

    pub fn do_validation(&self, last_fees: Fees, rules: &Rules, validation: &mut STValidation) {
        let (current_base, current_reserve, current_increment) = current_fee_schedule(last_fees);

        if rules.enabled(&feature_xrp_fees()) {
            self.vote_xrp_field(
                validation,
                current_base,
                self.target.reference_fee,
                "base fee",
                get_field_by_symbol("sfBaseFeeDrops"),
            );
            self.vote_xrp_field(
                validation,
                current_reserve,
                self.target.account_reserve,
                "base reserve",
                get_field_by_symbol("sfReserveBaseDrops"),
            );
            self.vote_xrp_field(
                validation,
                current_increment,
                self.target.owner_reserve,
                "reserve increment",
                get_field_by_symbol("sfReserveIncrementDrops"),
            );
            return;
        }

        self.vote_legacy_u64_field(
            validation,
            current_base,
            self.target.reference_fee,
            "base fee",
            get_field_by_symbol("sfBaseFee"),
        );
        self.vote_legacy_u32_field(
            validation,
            current_reserve,
            self.target.account_reserve,
            "base reserve",
            get_field_by_symbol("sfReserveBase"),
        );
        self.vote_legacy_u32_field(
            validation,
            current_increment,
            self.target.owner_reserve,
            "reserve increment",
            get_field_by_symbol("sfReserveIncrement"),
        );
    }

    pub fn do_voting<L, S>(
        &self,
        last_closed_ledger: &L,
        parent_validations: &[STValidation],
        initial_position: &mut S,
    ) where
        L: FeeVoteLedgerView,
        S: VoteTxSet,
    {
        assert!(
            last_closed_ledger.is_flag_ledger(),
            "xrpl::FeeVote::doVoting : has a flag ledger"
        );

        let (current_base, current_reserve, current_increment) =
            current_fee_schedule(last_closed_ledger.fees());
        let mut base_fee_vote = VotableValue::new(current_base, self.target.reference_fee);
        let mut base_reserve_vote = VotableValue::new(current_reserve, self.target.account_reserve);
        let mut inc_reserve_vote = VotableValue::new(current_increment, self.target.owner_reserve);

        let rules = last_closed_ledger.rules();
        if rules.enabled(&feature_xrp_fees()) {
            for validation in parent_validations
                .iter()
                .filter(|validation| validation.is_trusted())
            {
                apply_xrp_vote(
                    validation,
                    &mut base_fee_vote,
                    get_field_by_symbol("sfBaseFeeDrops"),
                );
                apply_xrp_vote(
                    validation,
                    &mut base_reserve_vote,
                    get_field_by_symbol("sfReserveBaseDrops"),
                );
                apply_xrp_vote(
                    validation,
                    &mut inc_reserve_vote,
                    get_field_by_symbol("sfReserveIncrementDrops"),
                );
            }
        } else {
            for validation in parent_validations
                .iter()
                .filter(|validation| validation.is_trusted())
            {
                apply_legacy_u64_vote(
                    validation,
                    &mut base_fee_vote,
                    get_field_by_symbol("sfBaseFee"),
                );
                apply_legacy_u32_vote(
                    validation,
                    &mut base_reserve_vote,
                    get_field_by_symbol("sfReserveBase"),
                );
                apply_legacy_u32_vote(
                    validation,
                    &mut inc_reserve_vote,
                    get_field_by_symbol("sfReserveIncrement"),
                );
            }
        }

        let base_fee = base_fee_vote.get_votes();
        let base_reserve = base_reserve_vote.get_votes();
        let inc_reserve = inc_reserve_vote.get_votes();
        let seq = last_closed_ledger.seq().wrapping_add(1);

        if !(base_fee.1 || base_reserve.1 || inc_reserve.1) {
            return;
        }

        self.journal.warn(&format!(
            "We are voting for a fee change: {}/{}/{}",
            base_fee.0.drops(),
            base_reserve.0.drops(),
            inc_reserve.0.drops()
        ));

        let fee_tx = STTx::new(TxType::FEE, |tx| {
            tx.set_account_id(
                get_field_by_symbol("sfAccount"),
                protocol::AccountID::default(),
            );
            tx.set_field_u32(get_field_by_symbol("sfLedgerSequence"), seq);
            if rules.enabled(&feature_xrp_fees()) {
                tx.set_field_amount(
                    get_field_by_symbol("sfBaseFeeDrops"),
                    STAmount::from_xrp_amount(base_fee.0),
                );
                tx.set_field_amount(
                    get_field_by_symbol("sfReserveBaseDrops"),
                    STAmount::from_xrp_amount(base_reserve.0),
                );
                tx.set_field_amount(
                    get_field_by_symbol("sfReserveIncrementDrops"),
                    STAmount::from_xrp_amount(inc_reserve.0),
                );
            } else {
                tx.set_field_u64(
                    get_field_by_symbol("sfBaseFee"),
                    base_fee
                        .0
                        .drops_as()
                        .unwrap_or_else(|| base_fee_vote.current().drops_as().unwrap_or_default()),
                );
                tx.set_field_u32(
                    get_field_by_symbol("sfReserveBase"),
                    base_reserve.0.drops_as().unwrap_or_else(|| {
                        base_reserve_vote.current().drops_as().unwrap_or_default()
                    }),
                );
                tx.set_field_u32(
                    get_field_by_symbol("sfReserveIncrement"),
                    inc_reserve.0.drops_as().unwrap_or_else(|| {
                        inc_reserve_vote.current().drops_as().unwrap_or_default()
                    }),
                );
                tx.set_field_u32(
                    get_field_by_symbol("sfReferenceFeeUnits"),
                    REFERENCE_FEE_UNITS_DEPRECATED,
                );
            }
        });

        self.journal
            .warn(&format!("Vote: {}", fee_tx.get_transaction_id()));
        if !initial_position.add_transaction(&fee_tx) {
            self.journal.warn("Ledger already had fee change");
        }
    }

    fn vote_xrp_field(
        &self,
        validation: &mut STValidation,
        current: XRPAmount,
        target: XRPAmount,
        name: &str,
        field: &'static protocol::SField,
    ) {
        if current == target {
            return;
        }

        self.journal
            .info(&format!("Voting for {name} of {}", target.drops()));
        validation.set_field_amount(field, STAmount::from_xrp_amount(target));
    }

    fn vote_legacy_u64_field(
        &self,
        validation: &mut STValidation,
        current: XRPAmount,
        target: XRPAmount,
        name: &str,
        field: &'static protocol::SField,
    ) {
        if current == target {
            return;
        }

        self.journal
            .info(&format!("Voting for {name} of {}", target.drops()));
        if let Some(value) = target.drops_as() {
            validation.set_field_u64(field, value);
        }
    }

    fn vote_legacy_u32_field(
        &self,
        validation: &mut STValidation,
        current: XRPAmount,
        target: XRPAmount,
        name: &str,
        field: &'static protocol::SField,
    ) {
        if current == target {
            return;
        }

        self.journal
            .info(&format!("Voting for {name} of {}", target.drops()));
        if let Some(value) = target.drops_as() {
            validation.set_field_u32(field, value);
        }
    }
}

fn current_fee_schedule(last_fees: Fees) -> (XRPAmount, XRPAmount, XRPAmount) {
    (
        XRPAmount::from_drops(i64::try_from(last_fees.base).expect("base fee fits i64")),
        XRPAmount::from_drops(i64::try_from(last_fees.reserve).expect("reserve fits i64")),
        XRPAmount::from_drops(i64::try_from(last_fees.increment).expect("increment fits i64")),
    )
}

fn apply_xrp_vote(
    validation: &STValidation,
    value: &mut VotableValue,
    field: &'static protocol::SField,
) {
    if !validation.is_field_present(field) {
        value.no_vote();
        return;
    }

    let amount = validation.get_field_amount(field);
    if !amount.native() {
        value.no_vote();
        return;
    }

    let vote = amount.xrp();
    if is_legal_amount_signed(vote) {
        value.add_vote(vote);
    } else {
        value.no_vote();
    }
}

fn apply_legacy_u64_vote(
    validation: &STValidation,
    value: &mut VotableValue,
    field: &'static protocol::SField,
) {
    if !validation.is_field_present(field) {
        value.no_vote();
        return;
    }

    let raw_vote = validation.get_field_u64(field);
    match i64::try_from(raw_vote)
        .ok()
        .map(XRPAmount::from_drops)
        .filter(|vote| is_legal_amount_signed(*vote))
    {
        Some(vote) => value.add_vote(vote),
        None => value.no_vote(),
    }
}

fn apply_legacy_u32_vote(
    validation: &STValidation,
    value: &mut VotableValue,
    field: &'static protocol::SField,
) {
    if !validation.is_field_present(field) {
        value.no_vote();
        return;
    }

    let vote = XRPAmount::from_drops(i64::from(validation.get_field_u32(field)));
    if is_legal_amount_signed(vote) {
        value.add_vote(vote);
    } else {
        value.no_vote();
    }
}

fn is_legal_amount_signed(amount: XRPAmount) -> bool {
    let drops = i128::from(amount.drops());
    let limit = i128::from(INITIAL_XRP_DROPS);
    drops >= -limit && drops <= limit
}

#[cfg(test)]
mod tests {
    use super::{FeeSetup, VotableValue};
    use protocol::XRPAmount;

    #[test]
    fn fee_setup_defaults_match_current_cpp_recommendations() {
        let setup = FeeSetup::default();

        assert_eq!(setup.reference_fee, XRPAmount::from_drops(10));
        assert_eq!(setup.account_reserve, XRPAmount::from_drops(1_000_000));
        assert_eq!(setup.owner_reserve, XRPAmount::from_drops(200_000));
    }

    #[test]
    fn votable_value_chooses_the_most_voted_value_between_current_and_target() {
        let mut value = VotableValue::new(XRPAmount::from_drops(10), XRPAmount::from_drops(30));
        value.add_vote(XRPAmount::from_drops(50));
        value.add_vote(XRPAmount::from_drops(20));
        value.add_vote(XRPAmount::from_drops(20));

        assert_eq!(value.get_votes(), (XRPAmount::from_drops(20), true));
    }

    #[test]
    fn votable_value_keeps_target_when_only_out_of_range_values_are_added() {
        let mut value = VotableValue::new(XRPAmount::from_drops(10), XRPAmount::from_drops(30));
        value.add_vote(XRPAmount::from_drops(50));
        value.add_vote(XRPAmount::from_drops(50));

        assert_eq!(value.get_votes(), (XRPAmount::from_drops(30), true));
    }
}
