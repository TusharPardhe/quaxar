//! Full reference flow engine parity.

pub mod steps;
pub mod strand_builder;
pub mod strand_flow;

use std::{
    cell::RefCell,
    collections::{BTreeSet, HashMap},
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
}

impl SelfCrossCancellation {
    pub fn record(&self, offer_key: Uint256) {
        self.offer_keys.borrow_mut().insert(offer_key);
    }

    /// Apply the accumulated offer deletions to the transaction view. Missing
    /// offers are already gone through ordinary flow processing and therefore
    /// require no further action.
    pub fn apply_to<V: crate::ApplyView>(&self, view: &mut V) -> Ter {
        let offer_keys: Vec<_> = self.offer_keys.borrow().iter().copied().collect();

        for offer_key in offer_keys {
            let keylet = Keylet::new(LedgerEntryType::Offer, offer_key);
            let offer = match view.peek(keylet) {
                Ok(Some(offer)) => Some(offer),
                Ok(None) => match view.read(keylet) {
                    Ok(offer) => offer,
                    Err(_) => return Ter::TEF_BAD_LEDGER,
                },
                Err(_) => return Ter::TEF_BAD_LEDGER,
            };

            if let Some(offer) = offer {
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
#[derive(Debug, Clone)]
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
