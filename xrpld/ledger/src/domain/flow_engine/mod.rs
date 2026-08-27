//! Full reference flow engine parity.

pub mod steps;
pub mod strand_builder;
pub mod strand_flow;

use std::{
    cell::RefCell,
    collections::{BTreeMap, BTreeSet, HashMap},
    rc::Rc,
};

use basics::base_uint::Uint256;
use protocol::{AccountID, Asset, Keylet, LedgerEntryType, MPTIssue, STAmount, Ter};

const MAX_AMM_ITERATIONS: u16 = 30;

#[derive(Debug, Clone)]
pub struct AmmContext {
    inner: Rc<RefCell<AmmContextState>>,
}

#[derive(Debug)]
struct AmmContextState {
    account: AccountID,
    multi_path: bool,
    amm_used: bool,
    iterations: u16,
    initial_balances: HashMap<(Asset, Asset), (STAmount, STAmount)>,
}

impl AmmContext {
    pub fn new(account: AccountID, multi_path: bool) -> Self {
        Self {
            inner: Rc::new(RefCell::new(AmmContextState {
                account,
                multi_path,
                amm_used: false,
                iterations: 0,
                initial_balances: HashMap::new(),
            })),
        }
    }

    pub fn account(&self) -> AccountID {
        self.inner.borrow().account
    }
    pub fn multi_path(&self) -> bool {
        self.inner.borrow().multi_path
    }
    pub fn set_multi_path(&self, value: bool) {
        self.inner.borrow_mut().multi_path = value;
    }
    pub fn set_amm_used(&self) {
        self.inner.borrow_mut().amm_used = true;
    }
    pub fn clear(&self) {
        self.inner.borrow_mut().amm_used = false;
    }
    pub fn update(&self) {
        let mut state = self.inner.borrow_mut();
        if state.amm_used {
            state.iterations += 1;
        }
        state.amm_used = false;
    }
    pub fn iterations(&self) -> u16 {
        self.inner.borrow().iterations
    }
    pub fn max_iterations_reached(&self) -> bool {
        self.iterations() >= MAX_AMM_ITERATIONS
    }
    pub fn initial_balances(
        &self,
        book: (Asset, Asset),
        current: &(STAmount, STAmount),
    ) -> (STAmount, STAmount) {
        let mut state = self.inner.borrow_mut();
        if let Some(initial) = state.initial_balances.get(&book) {
            return initial.clone();
        }
        state.initial_balances.insert(book, current.clone());
        state
            .initial_balances
            .insert((book.1, book.0), (current.1.clone(), current.0.clone()));
        current.clone()
    }
}

/// A Strand is an ordered sequence of StepKind from source to destination.
pub type Strand = Vec<StepKind>;

/// Cancellation-only counterpart to rippled's `psbCancel`.
///
/// A direct OfferCreate crossing records offers that must be removed even when
/// the tentative flow sandbox is discarded (self-crossed, malformed, or unfunded).
/// Flow sandboxes remain disposable and therefore never own these deletions:
/// after the flow outcome is known, the caller applies this accumulator to the
/// transaction view with the canonical offer deletion helper. This deliberately
/// cannot carry transfers or partial trade state from a dry strand.
#[derive(Debug, Clone, Default)]
pub struct SelfCrossCancellation {
    offer_keys: Rc<RefCell<BTreeSet<Uint256>>>,
    /// FlowOfferStream only puts an unfunded or tiny reduced offer in its
    /// permanent-removal set when the pristine cancellation view has exactly
    /// the same funding as the working payment view. Recheck these candidates
    /// against the parent transaction view after tentative flow finishes so
    /// an offer changed by an earlier liquidity pass is not deleted by a
    /// failed FillOrKill crossing.
    funding_sensitive_offer_keys: Rc<RefCell<BTreeMap<Uint256, STAmount>>>,
}

impl SelfCrossCancellation {
    pub fn record(&self, offer_key: Uint256) {
        self.offer_keys.borrow_mut().insert(offer_key);
    }

    pub fn record_if_funding_unchanged(&self, offer_key: Uint256, observed_funds: STAmount) {
        self.funding_sensitive_offer_keys
            .borrow_mut()
            .insert(offer_key, observed_funds);
    }

    /// Apply the accumulated offer deletions to the transaction view. Missing
    /// offers are already gone through ordinary flow processing and therefore
    /// require no further action.
    pub fn apply_to<V: crate::ApplyView>(&self, view: &mut V) -> Ter {
        let permanent = self.offer_keys.borrow().clone();
        let mut offer_keys = permanent.clone();
        offer_keys.extend(self.funding_sensitive_offer_keys.borrow().keys().copied());

        for offer_key in offer_keys {
            let keylet = Keylet::new(LedgerEntryType::Offer, offer_key);
            let offer = match view.peek(keylet) {
                Ok(Some(offer)) => Some(offer),
                // ApplyStateTable::peek already resolves the effective base
                // entry and honors a staged erase. Falling back to read here
                // can resurrect an offer which the successful flow just
                // deleted and attempt its directory removal a second time.
                Ok(None) => None,
                Err(_) => return Ter::TEF_BAD_LEDGER,
            };

            if let Some(offer) = offer {
                if !permanent.contains(&offer_key) {
                    let owner = offer.get_account_id(protocol::get_field_by_symbol("sfAccount"));
                    let owner_gives =
                        offer.get_field_amount(protocol::get_field_by_symbol("sfTakerGets"));
                    let observed = self
                        .funding_sensitive_offer_keys
                        .borrow()
                        .get(&offer_key)
                        .cloned();
                    match crate::domain::ripple_calc::book_step::get_owner_funds(
                        view,
                        &owner,
                        &owner_gives,
                    ) {
                        Ok(funds) if observed.as_ref() != Some(&funds) => continue,
                        Ok(_) => {}
                        Err(_) => return Ter::TEF_BAD_LEDGER,
                    }
                }
                match crate::offer_helpers::offer_delete(view, offer) {
                    Ok(Ter::TES_SUCCESS) => {}
                    Ok(ter) => return ter,
                    Err(_) => return Ter::TEF_BAD_LEDGER,
                }
            }
        }

        Ter::TES_SUCCESS
    }
}

/// Step types that can appear in a strand.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StepKind {
    Direct {
        src: AccountID,
        dst: AccountID,
        currency: protocol::Currency,
    },
    XrpEndpoint {
        account: AccountID,
        is_last: bool,
    },
    /// An MPT may only ripple at a strand endpoint.  The issuer is therefore
    /// always one side of this step (holder->issuer or issuer->holder).
    MptEndpoint {
        src: AccountID,
        dst: AccountID,
        issue: MPTIssue,
        is_first: bool,
        is_last: bool,
        offer_crossing: bool,
    },
    Book {
        book_in: Asset,
        book_out: Asset,
        domain: Option<Uint256>,
        /// Offer crossing charges the offer owner; payments charge the path
        /// sender according to the surrounding step rules.
        owner_pays_transfer_fee: bool,
        /// rippled only removes a taker's own tip offer for direct/default
        /// offer crossing.  Explicit paths skip self offers instead.
        remove_self_crossing: bool,
    },
}

/// Context for strand building.
#[derive(Debug, Clone)]
pub struct StrandContext {
    pub src: AccountID,
    pub dst: AccountID,
    pub deliver: Asset,
    pub is_default_path: bool,
    pub owner_pays_transfer_fee: bool,
    pub offer_crossing: bool,
}

/// Result of flow execution.
#[derive(Debug, Clone)]
pub struct FlowResult {
    pub ter: Ter,
    pub actual_in: STAmount,
    pub actual_out: STAmount,
}

#[cfg(test)]
mod cancellation_tests {
    use std::sync::Arc;

    use basics::base_uint::{Uint160, Uint256};
    use protocol::{ApplyFlags, XRPAmount, get_field_by_symbol};

    use super::SelfCrossCancellation;
    use crate::{ApplyView, ApplyViewImpl, Ledger, RawView};

    fn sf(name: &str) -> &'static protocol::SField {
        get_field_by_symbol(name)
    }

    #[test]
    fn cancellation_does_not_resurrect_a_staged_offer_erase() {
        let offer_key = Uint256::from_array([0x71; 32]);
        let offer_keylet = protocol::Keylet::new(protocol::LedgerEntryType::Offer, offer_key);
        let offer = Arc::new(protocol::STLedgerEntry::new(offer_keylet));
        let mut base = Ledger::from_ledger_seq_and_close_time(1, 1, false);
        base.raw_insert(offer).expect("seed offer");
        let mut view = ApplyViewImpl::new(Arc::new(base), ApplyFlags::NONE);

        let staged = view
            .peek(offer_keylet)
            .expect("effective offer read")
            .expect("seeded offer");
        view.raw_erase(staged).expect("stage erase");

        let cancellations = SelfCrossCancellation::default();
        cancellations.record(offer_key);
        assert_eq!(
            cancellations.apply_to(&mut view),
            protocol::Ter::TES_SUCCESS
        );
        assert!(
            view.peek(offer_keylet)
                .expect("effective post-cancel read")
                .is_none(),
            "the cancellation view must honor the staged erase instead of reading the base offer"
        );
    }

    #[test]
    fn transiently_unfunded_candidate_is_rechecked_in_parent_view() {
        let owner = protocol::AccountID::from_array([0x22; 20]);
        let account_keylet = protocol::account_keylet(Uint160::from_void(owner.data()));
        let mut account = protocol::STLedgerEntry::new(account_keylet);
        account.set_field_amount(
            sf("sfBalance"),
            protocol::STAmount::from_xrp_amount(XRPAmount::from_drops(100_000_000)),
        );
        account.set_field_u32(sf("sfOwnerCount"), 0);

        let offer_key = Uint256::from_array([0x72; 32]);
        let offer_keylet = protocol::Keylet::new(protocol::LedgerEntryType::Offer, offer_key);
        let mut offer = protocol::STLedgerEntry::new(offer_keylet);
        offer.set_account_id(sf("sfAccount"), owner);
        offer.set_field_amount(
            sf("sfTakerGets"),
            protocol::STAmount::from_xrp_amount(XRPAmount::from_drops(1)),
        );

        let mut base = Ledger::from_ledger_seq_and_close_time(1, 1, false);
        base.raw_insert(Arc::new(account)).expect("seed owner");
        base.raw_insert(Arc::new(offer)).expect("seed offer");
        let mut view = ApplyViewImpl::new(Arc::new(base), ApplyFlags::NONE);

        let cancellations = SelfCrossCancellation::default();
        cancellations.record_if_funding_unchanged(
            offer_key,
            protocol::STAmount::from_xrp_amount(XRPAmount::from_drops(0)),
        );
        assert_eq!(
            cancellations.apply_to(&mut view),
            protocol::Ter::TES_SUCCESS
        );
        assert!(
            view.peek(offer_keylet)
                .expect("effective offer read")
                .is_some(),
            "an offer funded in the pristine parent is not a permanent FlowOfferStream removal"
        );
    }
}
