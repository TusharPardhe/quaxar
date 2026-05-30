//! the reference implementation compatibility surface.
//!
//! This ports the exact deterministic behavior around:
//!
//! - accepting the current `nfTokenPageLink` and `bookExchangeRate` fix types,
//! - requiring exactly the fix-specific field in preflight,
//! - mapping unknown fix types to `tefINVALID_LEDGER_FIX_TYPE`,
//! - requiring the relevant object to exist in preclaim,
//! - and mapping the repair callback result to `tesSUCCESS` versus
//!   `tecFAILED_PROCESSING` in `doApply()`.

use protocol::{NotTec, Ter};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LedgerStateFixType {
    NfTokenPageLink,
    BookExchangeRate,
    Unknown(u16),
}

impl From<u16> for LedgerStateFixType {
    fn from(value: u16) -> Self {
        match value {
            1 => Self::NfTokenPageLink,
            2 => Self::BookExchangeRate,
            other => Self::Unknown(other),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LedgerStateFixPreflightFacts {
    pub fix_type: LedgerStateFixType,
    pub owner_present: bool,
    pub book_directory_present: bool,
    pub fix_cleanup_3_2_0_enabled: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LedgerStateFixPreclaimFacts {
    pub fix_type: LedgerStateFixType,
    pub owner_exists: bool,
    pub book_directory_exists: bool,
    pub book_directory_has_exchange_rate: bool,
    pub book_directory_exchange_rate_matches_key: bool,
}

pub fn run_ledger_state_fix_preflight_facts(facts: LedgerStateFixPreflightFacts) -> NotTec {
    match facts.fix_type {
        LedgerStateFixType::NfTokenPageLink => {
            if !facts.owner_present || facts.book_directory_present {
                return Ter::TEM_INVALID;
            }
            Ter::TES_SUCCESS
        }
        LedgerStateFixType::BookExchangeRate => {
            if !facts.fix_cleanup_3_2_0_enabled {
                return Ter::TEM_DISABLED;
            }
            if !facts.book_directory_present || facts.owner_present {
                return Ter::TEM_INVALID;
            }
            Ter::TES_SUCCESS
        }
        LedgerStateFixType::Unknown(_) => Ter::TEF_INVALID_LEDGER_FIX_TYPE,
    }
}

pub fn run_ledger_state_fix_preflight(fix_type: LedgerStateFixType, owner_present: bool) -> NotTec {
    run_ledger_state_fix_preflight_facts(LedgerStateFixPreflightFacts {
        fix_type,
        owner_present,
        book_directory_present: false,
        fix_cleanup_3_2_0_enabled: false,
    })
}

pub fn run_ledger_state_fix_preclaim_facts(facts: LedgerStateFixPreclaimFacts) -> Ter {
    match facts.fix_type {
        LedgerStateFixType::NfTokenPageLink => {
            if !facts.owner_exists {
                return Ter::TEC_OBJECT_NOT_FOUND;
            }
            Ter::TES_SUCCESS
        }
        LedgerStateFixType::BookExchangeRate => {
            if !facts.book_directory_exists {
                return Ter::TEC_OBJECT_NOT_FOUND;
            }
            if !facts.book_directory_has_exchange_rate
                || facts.book_directory_exchange_rate_matches_key
            {
                return Ter::TEC_NO_PERMISSION;
            }
            Ter::TES_SUCCESS
        }
        LedgerStateFixType::Unknown(_) => Ter::TEC_INTERNAL,
    }
}

pub fn run_ledger_state_fix_preclaim(fix_type: LedgerStateFixType, owner_exists: bool) -> Ter {
    run_ledger_state_fix_preclaim_facts(LedgerStateFixPreclaimFacts {
        fix_type,
        owner_exists,
        book_directory_exists: false,
        book_directory_has_exchange_rate: false,
        book_directory_exchange_rate_matches_key: false,
    })
}

pub fn run_ledger_state_fix_do_apply_with_book(
    fix_type: LedgerStateFixType,
    repair_nft_page_links: impl FnOnce() -> bool,
    repair_book_exchange_rate: impl FnOnce() -> bool,
) -> Ter {
    match fix_type {
        LedgerStateFixType::NfTokenPageLink => {
            if !repair_nft_page_links() {
                return Ter::TEC_FAILED_PROCESSING;
            }
            Ter::TES_SUCCESS
        }
        LedgerStateFixType::BookExchangeRate => {
            if !repair_book_exchange_rate() {
                return Ter::TEC_INTERNAL;
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
    run_ledger_state_fix_do_apply_with_book(fix_type, repair_nft_page_links, || false)
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use protocol::trans_token;

    use super::{
        LedgerStateFixPreclaimFacts, LedgerStateFixPreflightFacts, LedgerStateFixType,
        run_ledger_state_fix_do_apply, run_ledger_state_fix_do_apply_with_book,
        run_ledger_state_fix_preclaim, run_ledger_state_fix_preclaim_facts,
        run_ledger_state_fix_preflight, run_ledger_state_fix_preflight_facts,
    };

    #[test]
    fn ledger_state_fix_type_matches_current_cpp_header_values() {
        assert_eq!(
            LedgerStateFixType::from(1),
            LedgerStateFixType::NfTokenPageLink
        );
        assert_eq!(
            LedgerStateFixType::from(2),
            LedgerStateFixType::BookExchangeRate
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
    fn ledger_state_fix_preflight_requires_book_directory_for_exchange_rate() {
        assert_eq!(
            run_ledger_state_fix_preflight_facts(LedgerStateFixPreflightFacts {
                fix_type: LedgerStateFixType::BookExchangeRate,
                owner_present: false,
                book_directory_present: true,
                fix_cleanup_3_2_0_enabled: false,
            }),
            protocol::Ter::TEM_DISABLED
        );
        assert_eq!(
            run_ledger_state_fix_preflight_facts(LedgerStateFixPreflightFacts {
                fix_type: LedgerStateFixType::BookExchangeRate,
                owner_present: false,
                book_directory_present: false,
                fix_cleanup_3_2_0_enabled: true,
            }),
            protocol::Ter::TEM_INVALID
        );
        assert_eq!(
            run_ledger_state_fix_preflight_facts(LedgerStateFixPreflightFacts {
                fix_type: LedgerStateFixType::BookExchangeRate,
                owner_present: true,
                book_directory_present: true,
                fix_cleanup_3_2_0_enabled: true,
            }),
            protocol::Ter::TEM_INVALID
        );
        assert_eq!(
            run_ledger_state_fix_preflight_facts(LedgerStateFixPreflightFacts {
                fix_type: LedgerStateFixType::BookExchangeRate,
                owner_present: false,
                book_directory_present: true,
                fix_cleanup_3_2_0_enabled: true,
            }),
            protocol::Ter::TES_SUCCESS
        );
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
    fn ledger_state_fix_preclaim_checks_book_directory_exchange_rate_state() {
        let base = LedgerStateFixPreclaimFacts {
            fix_type: LedgerStateFixType::BookExchangeRate,
            owner_exists: false,
            book_directory_exists: true,
            book_directory_has_exchange_rate: true,
            book_directory_exchange_rate_matches_key: false,
        };
        assert_eq!(
            run_ledger_state_fix_preclaim_facts(LedgerStateFixPreclaimFacts {
                book_directory_exists: false,
                ..base
            }),
            protocol::Ter::TEC_OBJECT_NOT_FOUND
        );
        assert_eq!(
            run_ledger_state_fix_preclaim_facts(LedgerStateFixPreclaimFacts {
                book_directory_has_exchange_rate: false,
                ..base
            }),
            protocol::Ter::TEC_NO_PERMISSION
        );
        assert_eq!(
            run_ledger_state_fix_preclaim_facts(LedgerStateFixPreclaimFacts {
                book_directory_exchange_rate_matches_key: true,
                ..base
            }),
            protocol::Ter::TEC_NO_PERMISSION
        );
        assert_eq!(
            run_ledger_state_fix_preclaim_facts(base),
            protocol::Ter::TES_SUCCESS
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

    #[test]
    fn ledger_state_fix_do_apply_dispatches_book_exchange_rate_repair() {
        let nft_called = Cell::new(false);
        let book_called = Cell::new(false);
        let result = run_ledger_state_fix_do_apply_with_book(
            LedgerStateFixType::BookExchangeRate,
            || {
                nft_called.set(true);
                true
            },
            || {
                book_called.set(true);
                true
            },
        );

        assert!(!nft_called.get());
        assert!(book_called.get());
        assert_eq!(result, protocol::Ter::TES_SUCCESS);

        let failure = run_ledger_state_fix_do_apply_with_book(
            LedgerStateFixType::BookExchangeRate,
            || true,
            || false,
        );
        assert_eq!(failure, protocol::Ter::TEC_INTERNAL);
    }
}
