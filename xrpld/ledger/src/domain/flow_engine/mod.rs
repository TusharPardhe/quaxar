//! Full reference flow engine parity.

pub mod steps;
pub mod strand_builder;
pub mod strand_flow;

use std::{cell::RefCell, collections::BTreeSet, rc::Rc};

use basics::base_uint::Uint256;
use protocol::{AccountID, Asset, Issue, Keylet, LedgerEntryType, STAmount, Ter};

/// A Strand is an ordered sequence of StepKind from source to destination.
pub type Strand = Vec<StepKind>;

/// Cancellation-only counterpart to rippled's `psbCancel`.
///
/// A direct OfferCreate crossing records only eligible self-offer keys here.
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
    Book {
        book_in: Issue,
        book_out: Issue,
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
