//! Current Rust helpers mirroring the shared the reference implementation
//! `preflight0(...)`, `preflight1(...)`, and `preflight2(...)` shells.
//!
//! This module preserves the exact deterministic control flow around:
//!
//! - pseudo-transaction inner-batch rejection,
//! - legacy versus modern `NetworkID` handling,
//! - zero transaction-id and invalid-flag rejection,
//! - the delegate feature gate and self-delegate rejection,
//! - malformed account and fee rejection,
//! - the `AccountTxnID` plus ticket incompatibility rule,
//! - inner-batch feature assertions and invalid-flag rejection,
//! - dry-run simulate-key short-circuiting,
//! - batch-inner signature-check bypass,
//! - and the final `Validity::SigBad` mapping to `temINVALID`.

use protocol::{NotTec, Ter, is_tes_success};

use crate::Validity;

pub const LEGACY_NETWORK_ID_MAX: u32 = 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TransactorPreflight0Facts {
    pub is_pseudo_tx: bool,
    pub inner_batch_flag_set: bool,
    pub network_id_present: bool,
    pub node_network_id: u32,
    pub tx_network_id: Option<u32>,
    pub tx_id_is_zero: bool,
    pub tx_flags: u32,
}

pub const fn run_transactor_preflight0(facts: TransactorPreflight0Facts, flag_mask: u32) -> NotTec {
    if facts.is_pseudo_tx && facts.inner_batch_flag_set {
        return Ter::TEM_INVALID_FLAG;
    }

    if !facts.is_pseudo_tx || facts.network_id_present {
        if facts.node_network_id <= LEGACY_NETWORK_ID_MAX {
            if facts.tx_network_id.is_some() {
                return Ter::TEL_NETWORK_ID_MAKES_TX_NON_CANONICAL;
            }
        } else {
            let Some(tx_network_id) = facts.tx_network_id else {
                return Ter::TEL_REQUIRES_NETWORK_ID;
            };

            if tx_network_id != facts.node_network_id {
                return Ter::TEL_WRONG_NETWORK;
            }
        }
    }

    if facts.tx_id_is_zero {
        return Ter::TEM_INVALID;
    }

    if (facts.tx_flags & flag_mask) != 0 {
        return Ter::TEM_INVALID_FLAG;
    }

    Ter::TES_SUCCESS
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TransactorPreflight1Facts {
    pub delegate_present: bool,
    pub permission_delegation_enabled: bool,
    pub delegate_equals_account: bool,
    pub account_is_zero: bool,
    pub fee_is_native: bool,
    pub fee_is_negative: bool,
    pub fee_is_legal: bool,
    pub seq_proxy_is_ticket: bool,
    pub account_txn_id_present: bool,
    pub inner_batch_flag_set: bool,
    pub batch_enabled: bool,
    pub parent_batch_id_present: bool,
}

pub fn run_transactor_preflight1(
    facts: TransactorPreflight1Facts,
    run_preflight0: impl FnOnce() -> NotTec,
    run_preflight_check_signing_key: impl FnOnce() -> NotTec,
) -> NotTec {
    if facts.delegate_present {
        if !facts.permission_delegation_enabled {
            return Ter::TEM_DISABLED;
        }

        if facts.delegate_equals_account {
            return Ter::TEM_BAD_SIGNER;
        }
    }

    let ret = run_preflight0();
    if !is_tes_success(ret) {
        return ret;
    }

    if facts.account_is_zero {
        return Ter::TEM_BAD_SRC_ACCOUNT;
    }

    if !facts.fee_is_native || facts.fee_is_negative || !facts.fee_is_legal {
        return Ter::TEM_BAD_FEE;
    }

    let ret = run_preflight_check_signing_key();
    if !is_tes_success(ret) {
        return ret;
    }

    if facts.seq_proxy_is_ticket && facts.account_txn_id_present {
        return Ter::TEM_INVALID;
    }

    if facts.inner_batch_flag_set && !facts.batch_enabled {
        return Ter::TEM_INVALID_FLAG;
    }

    assert!(
        facts.inner_batch_flag_set == facts.parent_batch_id_present || !facts.batch_enabled,
        "Inner batch transaction must have a parent batch ID."
    );

    Ter::TES_SUCCESS
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TransactorPreflight2Facts {
    pub inner_batch_flag_set: bool,
    pub batch_enabled: bool,
}

pub fn run_transactor_preflight2(
    facts: TransactorPreflight2Facts,
    run_preflight_check_simulate_keys: impl FnOnce() -> Option<NotTec>,
    check_validity: impl FnOnce() -> Validity,
) -> NotTec {
    if let Some(ret) = run_preflight_check_simulate_keys() {
        return ret;
    }

    assert!(
        !facts.inner_batch_flag_set || facts.batch_enabled,
        "InnerBatch flag only set if feature enabled"
    );

    if facts.inner_batch_flag_set && facts.batch_enabled {
        return Ter::TES_SUCCESS;
    }

    if check_validity() == Validity::SigBad {
        return Ter::TEM_INVALID;
    }

    Ter::TES_SUCCESS
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use protocol::{Ter, trans_token};

    use super::{
        LEGACY_NETWORK_ID_MAX, TransactorPreflight0Facts, TransactorPreflight1Facts,
        TransactorPreflight2Facts, run_transactor_preflight0, run_transactor_preflight1,
        run_transactor_preflight2,
    };
    use crate::Validity;

    #[test]
    fn transactor_preflight0_matches_current_network_and_flag_ordering() {
        let pseudo = run_transactor_preflight0(
            TransactorPreflight0Facts {
                is_pseudo_tx: true,
                inner_batch_flag_set: true,
                ..TransactorPreflight0Facts::default()
            },
            0,
        );
        let legacy_network = run_transactor_preflight0(
            TransactorPreflight0Facts {
                node_network_id: LEGACY_NETWORK_ID_MAX,
                network_id_present: true,
                tx_network_id: Some(13),
                ..TransactorPreflight0Facts::default()
            },
            0,
        );
        let modern_missing = run_transactor_preflight0(
            TransactorPreflight0Facts {
                node_network_id: LEGACY_NETWORK_ID_MAX + 1,
                ..TransactorPreflight0Facts::default()
            },
            0,
        );

        assert_eq!(pseudo, Ter::TEM_INVALID_FLAG);
        assert_eq!(legacy_network, Ter::TEL_NETWORK_ID_MAKES_TX_NON_CANONICAL);
        assert_eq!(modern_missing, Ter::TEL_REQUIRES_NETWORK_ID);
        assert_eq!(trans_token(modern_missing), "telREQUIRES_NETWORK_ID");
    }

    #[test]
    fn transactor_preflight1_short_circuits_in_current() {
        let trace = RefCell::new(Vec::new());

        let result = run_transactor_preflight1(
            TransactorPreflight1Facts {
                delegate_present: true,
                permission_delegation_enabled: true,
                delegate_equals_account: false,
                ..TransactorPreflight1Facts::default()
            },
            || {
                trace.borrow_mut().push("preflight0");
                Ter::TEM_INVALID_FLAG
            },
            || {
                trace.borrow_mut().push("signing-key");
                Ter::TES_SUCCESS
            },
        );

        assert_eq!(result, Ter::TEM_INVALID_FLAG);
        assert_eq!(trace.into_inner(), vec!["preflight0"]);
    }

    #[test]
    fn transactor_preflight1_rejects_disabled_delegate_before_preflight0() {
        let result = run_transactor_preflight1(
            TransactorPreflight1Facts {
                delegate_present: true,
                permission_delegation_enabled: false,
                ..TransactorPreflight1Facts::default()
            },
            || panic!("delegate-disabled path should skip preflight0"),
            || panic!("delegate-disabled path should skip signing-key helper"),
        );

        assert_eq!(result, Ter::TEM_DISABLED);
    }

    #[test]
    fn transactor_preflight1_rejects_bad_account_fee_and_ticket_shapes() {
        let bad_account = run_transactor_preflight1(
            TransactorPreflight1Facts {
                account_is_zero: true,
                fee_is_native: true,
                fee_is_legal: true,
                ..TransactorPreflight1Facts::default()
            },
            || Ter::TES_SUCCESS,
            || Ter::TES_SUCCESS,
        );
        let bad_fee = run_transactor_preflight1(
            TransactorPreflight1Facts {
                fee_is_native: false,
                ..TransactorPreflight1Facts::default()
            },
            || Ter::TES_SUCCESS,
            || Ter::TES_SUCCESS,
        );
        let ticket_and_account_txn_id = run_transactor_preflight1(
            TransactorPreflight1Facts {
                fee_is_native: true,
                fee_is_legal: true,
                seq_proxy_is_ticket: true,
                account_txn_id_present: true,
                ..TransactorPreflight1Facts::default()
            },
            || Ter::TES_SUCCESS,
            || Ter::TES_SUCCESS,
        );

        assert_eq!(bad_account, Ter::TEM_BAD_SRC_ACCOUNT);
        assert_eq!(bad_fee, Ter::TEM_BAD_FEE);
        assert_eq!(ticket_and_account_txn_id, Ter::TEM_INVALID);
    }

    #[test]
    fn transactor_preflight2_preserves_simulate_and_inner_batch_short_circuits() {
        let simulate = run_transactor_preflight2(
            TransactorPreflight2Facts::default(),
            || Some(Ter::TEM_INVALID),
            || panic!("simulate short-circuit should skip validity"),
        );
        let inner_batch = run_transactor_preflight2(
            TransactorPreflight2Facts {
                inner_batch_flag_set: true,
                batch_enabled: true,
            },
            || None,
            || panic!("batch-inner bypass should skip validity"),
        );

        assert_eq!(simulate, Ter::TEM_INVALID);
        assert_eq!(inner_batch, Ter::TES_SUCCESS);
    }

    #[test]
    fn transactor_preflight2_maps_sigbad_only() {
        let sig_bad = run_transactor_preflight2(
            TransactorPreflight2Facts::default(),
            || None,
            || Validity::SigBad,
        );
        let valid = run_transactor_preflight2(
            TransactorPreflight2Facts::default(),
            || None,
            || Validity::Valid,
        );

        assert_eq!(sig_bad, Ter::TEM_INVALID);
        assert_eq!(valid, Ter::TES_SUCCESS);
    }
}
