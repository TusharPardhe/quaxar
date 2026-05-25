//! the reference implementation compatibility surface.
//!
//! This ports the exact deterministic behavior around:
//!
//! - accepting only the current `nfTokenPageLink` fix type,
//! - requiring `sfOwner` for that fix type in preflight,
//! - mapping unknown fix types to `tefINVALID_LEDGER_FIX_TYPE`,
//! - requiring the owner account to exist in preclaim,
//! - and mapping the repair callback result to `tesSUCCESS` versus
//!   `tecFAILED_PROCESSING` in `doApply()`.

use protocol::{NotTec, Ter};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LedgerStateFixType {
    NfTokenPageLink,
    Unknown(u16),
}

impl From<u16> for LedgerStateFixType {
    fn from(value: u16) -> Self {
        match value {
            1 => Self::NfTokenPageLink,
            other => Self::Unknown(other),
        }
    }
}

pub fn run_ledger_state_fix_preflight(fix_type: LedgerStateFixType, owner_present: bool) -> NotTec {
    match fix_type {
        LedgerStateFixType::NfTokenPageLink => {
            if !owner_present {
                return Ter::TEM_INVALID;
            }
            Ter::TES_SUCCESS
        }
        LedgerStateFixType::Unknown(_) => Ter::TEF_INVALID_LEDGER_FIX_TYPE,
    }
}

pub fn run_ledger_state_fix_preclaim(fix_type: LedgerStateFixType, owner_exists: bool) -> Ter {
    match fix_type {
        LedgerStateFixType::NfTokenPageLink => {
            if !owner_exists {
                return Ter::TEC_OBJECT_NOT_FOUND;
            }
            Ter::TES_SUCCESS
        }
        LedgerStateFixType::Unknown(_) => Ter::TEC_INTERNAL,
    }
}

pub fn run_ledger_state_fix_do_apply(
    fix_type: LedgerStateFixType,
    repair_nft_page_links: impl FnOnce() -> bool,
) -> Ter {
    match fix_type {
        LedgerStateFixType::NfTokenPageLink => {
            if !repair_nft_page_links() {
                return Ter::TEC_FAILED_PROCESSING;
            }
            Ter::TES_SUCCESS
        }
        LedgerStateFixType::Unknown(_) => Ter::TEC_INTERNAL,
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use protocol::trans_token;

    use super::{
        LedgerStateFixType, run_ledger_state_fix_do_apply, run_ledger_state_fix_preclaim,
        run_ledger_state_fix_preflight,
    };

    #[test]
    fn ledger_state_fix_type_matches_current_cpp_header_values() {
        assert_eq!(
            LedgerStateFixType::from(1),
            LedgerStateFixType::NfTokenPageLink
        );
        assert_eq!(LedgerStateFixType::from(0), LedgerStateFixType::Unknown(0));
        assert_eq!(
            LedgerStateFixType::from(200),
            LedgerStateFixType::Unknown(200)
        );
    }

    #[test]
    fn ledger_state_fix_preflight_requires_owner_for_nft_page_link() {
        assert_eq!(
            run_ledger_state_fix_preflight(LedgerStateFixType::NfTokenPageLink, false),
            protocol::Ter::TEM_INVALID
        );
        assert_eq!(
            run_ledger_state_fix_preflight(LedgerStateFixType::NfTokenPageLink, true),
            protocol::Ter::TES_SUCCESS
        );
    }

    #[test]
    fn ledger_state_fix_preflight_rejects_unknown_fix_type() {
        let result = run_ledger_state_fix_preflight(LedgerStateFixType::Unknown(7), true);

        assert_eq!(result, protocol::Ter::TEF_INVALID_LEDGER_FIX_TYPE);
        assert_eq!(trans_token(result), "tefINVALID_LEDGER_FIX_TYPE");
    }

    #[test]
    fn ledger_state_fix_preclaim_requires_owner_object_for_nft_page_link() {
        assert_eq!(
            run_ledger_state_fix_preclaim(LedgerStateFixType::NfTokenPageLink, false),
            protocol::Ter::TEC_OBJECT_NOT_FOUND
        );
        assert_eq!(
            run_ledger_state_fix_preclaim(LedgerStateFixType::NfTokenPageLink, true),
            protocol::Ter::TES_SUCCESS
        );
    }

    #[test]
    fn ledger_state_fix_preclaim_keeps_internal_fallback_for_unknown_type() {
        assert_eq!(
            run_ledger_state_fix_preclaim(LedgerStateFixType::Unknown(9), true),
            protocol::Ter::TEC_INTERNAL
        );
    }

    #[test]
    fn ledger_state_fix_do_apply_maps_repair_result() {
        let called = Cell::new(false);
        let success = run_ledger_state_fix_do_apply(LedgerStateFixType::NfTokenPageLink, || {
            called.set(true);
            true
        });

        assert!(called.get());
        assert_eq!(success, protocol::Ter::TES_SUCCESS);

        let failure = run_ledger_state_fix_do_apply(LedgerStateFixType::NfTokenPageLink, || false);

        assert_eq!(failure, protocol::Ter::TEC_FAILED_PROCESSING);
        assert_eq!(trans_token(failure), "tecFAILED_PROCESSING");
    }

    #[test]
    fn ledger_state_fix_do_apply_keeps_internal_fallback_for_unknown_type() {
        let callback_called = Cell::new(false);
        let result = run_ledger_state_fix_do_apply(LedgerStateFixType::Unknown(11), || {
            callback_called.set(true);
            true
        });

        assert!(!callback_called.get());
        assert_eq!(result, protocol::Ter::TEC_INTERNAL);
    }
}
