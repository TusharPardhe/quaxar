//! Deterministic
//! the reference implementation metadata, `preflight(...)`, `preclaim(...)`,
//! and `doApply()` shell.
//!
//! This ports the current top-level control flow around:
//!
//! - malformed empty input rejection,
//! - malformed broker-id and amount validation,
//! - malformed native, negative, illegal-net, and issuer-shape checks,
//! - broker-id resolution failures when the id is omitted,
//! - missing-broker and missing-vault rejection,
//! - native-vault-asset and wrong-issuer permission checks,
//! - optional transaction-asset mismatch rejection,
//! - minimum-cover clawback computation failure,
//! - the explicit pseudo-account balance invariant check,
//! - the impossible missing-issuer fallback to `tefBAD_LEDGER`,
//! - the MPT-vs-IOU issuer permission split,
//! - and the `doApply()` resolve, load, decrement, update, asset-association,
//!   and delegated transfer ordering.

use protocol::{NotTec, Ter};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoanBrokerCoverClawbackAmountKind {
    Issue,
    Mpt,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct LoanBrokerCoverClawbackPreflightFacts {
    pub broker_id_is_present: bool,
    pub broker_id_is_zero: bool,
    pub amount_is_present: bool,
    pub amount_is_native: bool,
    pub amount_is_negative: bool,
    pub amount_is_legal_net: bool,
    pub broker_id_missing_amount_is_mpt: bool,
    pub broker_id_missing_amount_holder_is_account: bool,
    pub broker_id_missing_amount_holder_is_zero: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct LoanBrokerCoverClawbackPreclaimFacts {
    pub broker_id_resolution_result: Ter,
    pub broker_exists: bool,
    pub vault_exists: bool,
    pub vault_asset_is_native: bool,
    pub submitter_is_vault_asset_issuer: bool,
    pub amount_is_present: bool,
    pub amount_asset_matches_vault_asset: bool,
    pub claw_amount_can_be_determined: bool,
    pub pseudo_balance_at_least_claw_amount: bool,
    pub issuer_account_exists: bool,
    pub amount_kind: LoanBrokerCoverClawbackAmountKind,
    pub mpt_issuance_exists: bool,
    pub mpt_can_clawback: bool,
    pub mpt_issuer_matches_submitter: bool,
    pub issuer_allows_trustline_clawback: bool,
    pub issuer_has_no_freeze: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoanBrokerCoverClawbackResolveBrokerIdFacts<BrokerId> {
    pub broker_id_from_tx: Option<BrokerId>,
    pub amount_is_present: bool,
    pub amount_holds_issue: bool,
    pub holder_account_exists: bool,
    pub broker_id_from_holder_account: Option<BrokerId>,
}

pub trait LoanBrokerCoverClawbackDoApplyBroker {
    type AccountId;
    type Amount;
    type Asset;
    type VaultId;

    fn vault_id(&self) -> &Self::VaultId;
    fn pseudo_account_id(&self) -> &Self::AccountId;
    fn subtract_cover_available(&mut self, amount: &Self::Amount);
}

pub trait LoanBrokerCoverClawbackDoApplyVault {
    type Asset;

    fn asset(&self) -> &Self::Asset;
}

pub trait LoanBrokerCoverClawbackDoApplyAmount {
    fn is_native(&self) -> bool;
}

pub trait LoanBrokerCoverClawbackDeterminableAmount:
    LoanBrokerCoverClawbackDoApplyAmount + Clone + PartialOrd
{
    fn is_zero(&self) -> bool;
    fn is_positive(&self) -> bool;
}

pub trait LoanBrokerCoverClawbackDoApplySink {
    type Broker: LoanBrokerCoverClawbackDoApplyBroker<
            AccountId = Self::AccountId,
            Amount = Self::Amount,
            Asset = Self::Asset,
            VaultId = Self::VaultId,
        >;
    type Vault: LoanBrokerCoverClawbackDoApplyVault<Asset = Self::Asset>;
    type BrokerId;
    type AccountId;
    type Amount: LoanBrokerCoverClawbackDoApplyAmount;
    type Asset;
    type VaultId;

    fn read_broker(&mut self, broker_id: &Self::BrokerId) -> Option<Self::Broker>;
    fn read_vault(&mut self, vault_id: &Self::VaultId) -> Option<Self::Vault>;
    fn update_broker(&mut self, broker: &Self::Broker);
    fn associate_asset(&mut self, broker: &Self::Broker, asset: &Self::Asset);
    fn send_asset(
        &mut self,
        pseudo_account_id: &Self::AccountId,
        destination: &Self::AccountId,
        amount: &Self::Amount,
    ) -> Ter;
}

impl Default for LoanBrokerCoverClawbackAmountKind {
    fn default() -> Self {
        Self::Issue
    }
}

impl LoanBrokerCoverClawbackDoApplyAmount for i64 {
    fn is_native(&self) -> bool {
        false
    }
}

impl LoanBrokerCoverClawbackDeterminableAmount for i64 {
    fn is_zero(&self) -> bool {
        *self == 0
    }

    fn is_positive(&self) -> bool {
        *self > 0
    }
}

pub fn run_loan_broker_cover_clawback_preflight(
    facts: LoanBrokerCoverClawbackPreflightFacts,
) -> NotTec {
    if !facts.broker_id_is_present && !facts.amount_is_present {
        return Ter::TEM_INVALID;
    }

    if facts.broker_id_is_present && facts.broker_id_is_zero {
        return Ter::TEM_INVALID;
    }

    if facts.amount_is_present {
        if facts.amount_is_native {
            return Ter::TEM_BAD_AMOUNT;
        }

        if facts.amount_is_negative {
            return Ter::TEM_BAD_AMOUNT;
        }

        if !facts.amount_is_legal_net {
            return Ter::TEM_BAD_AMOUNT;
        }

        if !facts.broker_id_is_present {
            if facts.broker_id_missing_amount_is_mpt {
                return Ter::TEM_INVALID;
            }

            if facts.broker_id_missing_amount_holder_is_account
                || facts.broker_id_missing_amount_holder_is_zero
            {
                return Ter::TEM_INVALID;
            }
        }
    }

    Ter::TES_SUCCESS
}

pub fn run_loan_broker_cover_clawback_resolve_broker_id<BrokerId: Clone>(
    facts: LoanBrokerCoverClawbackResolveBrokerIdFacts<BrokerId>,
) -> Result<BrokerId, Ter> {
    if let Some(broker_id) = facts.broker_id_from_tx {
        return Ok(broker_id);
    }

    if !facts.amount_is_present || !facts.amount_holds_issue {
        return Err(Ter::TEC_INTERNAL);
    }

    if !facts.holder_account_exists {
        return Err(Ter::TEC_NO_ENTRY);
    }

    match facts.broker_id_from_holder_account {
        Some(broker_id) => Ok(broker_id),
        None => Err(Ter::TEC_OBJECT_NOT_FOUND),
    }
}

pub fn run_loan_broker_cover_clawback_determine_amount<Amount>(
    max_claw_amount: Amount,
    requested_amount: Option<Amount>,
) -> Result<Amount, Ter>
where
    Amount: LoanBrokerCoverClawbackDeterminableAmount,
{
    if !max_claw_amount.is_positive() {
        return Err(Ter::TEC_INSUFFICIENT_FUNDS);
    }

    match requested_amount {
        None => Ok(max_claw_amount),
        Some(requested_amount) if requested_amount.is_zero() => Ok(max_claw_amount),
        Some(requested_amount) if requested_amount > max_claw_amount => Ok(max_claw_amount),
        Some(requested_amount) => Ok(requested_amount),
    }
}

pub fn run_loan_broker_cover_clawback_preclaim(facts: LoanBrokerCoverClawbackPreclaimFacts) -> Ter {
    if facts.broker_id_resolution_result != Ter::TES_SUCCESS {
        return facts.broker_id_resolution_result;
    }

    if !facts.broker_exists {
        return Ter::TEC_NO_ENTRY;
    }

    if !facts.vault_exists {
        return Ter::TEF_BAD_LEDGER;
    }

    if facts.vault_asset_is_native {
        return Ter::TEC_NO_PERMISSION;
    }

    if !facts.submitter_is_vault_asset_issuer {
        return Ter::TEC_NO_PERMISSION;
    }

    if facts.amount_is_present && !facts.amount_asset_matches_vault_asset {
        return Ter::TEC_WRONG_ASSET;
    }

    if !facts.claw_amount_can_be_determined {
        return Ter::TEC_INSUFFICIENT_FUNDS;
    }

    if !facts.pseudo_balance_at_least_claw_amount {
        return Ter::TEC_INTERNAL;
    }

    if !facts.issuer_account_exists {
        return Ter::TEF_BAD_LEDGER;
    }

    match facts.amount_kind {
        LoanBrokerCoverClawbackAmountKind::Issue => {
            if !facts.issuer_allows_trustline_clawback || facts.issuer_has_no_freeze {
                return Ter::TEC_NO_PERMISSION;
            }
        }
        LoanBrokerCoverClawbackAmountKind::Mpt => {
            if !facts.mpt_issuance_exists {
                return Ter::TEC_OBJECT_NOT_FOUND;
            }

            if !facts.mpt_can_clawback {
                return Ter::TEC_NO_PERMISSION;
            }

            if !facts.mpt_issuer_matches_submitter {
                return Ter::TEC_INTERNAL;
            }
        }
    }

    Ter::TES_SUCCESS
}

pub fn run_loan_broker_cover_clawback_do_apply<Sink, DetermineBrokerId, DetermineClawAmount>(
    sink: &mut Sink,
    account: &Sink::AccountId,
    determine_broker_id: DetermineBrokerId,
    determine_claw_amount: DetermineClawAmount,
) -> Ter
where
    Sink: LoanBrokerCoverClawbackDoApplySink,
    Sink::AccountId: Clone,
    Sink::Asset: Clone,
    Sink::BrokerId: Clone,
    Sink::VaultId: Clone,
    DetermineBrokerId: FnOnce() -> Result<Sink::BrokerId, Ter>,
    DetermineClawAmount: FnOnce(&Sink::Broker, &Sink::Asset) -> Result<Sink::Amount, Ter>,
{
    let broker_id = match determine_broker_id() {
        Ok(broker_id) => broker_id,
        Err(_) => return Ter::TEC_INTERNAL,
    };

    let mut broker = match sink.read_broker(&broker_id) {
        Some(broker) => broker,
        None => return Ter::TEC_INTERNAL,
    };

    let vault_id = broker.vault_id().clone();
    let vault = match sink.read_vault(&vault_id) {
        Some(vault) => vault,
        None => return Ter::TEC_INTERNAL,
    };

    let vault_asset = vault.asset().clone();
    let claw_amount = match determine_claw_amount(&broker, &vault_asset) {
        Ok(claw_amount) => claw_amount,
        Err(_) => return Ter::TEC_INTERNAL,
    };
    if claw_amount.is_native() {
        return Ter::TEC_INTERNAL;
    }

    let pseudo_account_id = broker.pseudo_account_id().clone();
    broker.subtract_cover_available(&claw_amount);
    sink.update_broker(&broker);
    sink.associate_asset(&broker, &vault_asset);

    sink.send_asset(&pseudo_account_id, account, &claw_amount)
}

#[cfg(test)]
mod tests {
    use std::{cell::RefCell, rc::Rc};

    use protocol::{Ter, trans_token};

    use super::{
        LoanBrokerCoverClawbackAmountKind, LoanBrokerCoverClawbackDeterminableAmount,
        LoanBrokerCoverClawbackDoApplyAmount, LoanBrokerCoverClawbackDoApplyBroker,
        LoanBrokerCoverClawbackDoApplySink, LoanBrokerCoverClawbackDoApplyVault,
        LoanBrokerCoverClawbackPreclaimFacts, LoanBrokerCoverClawbackPreflightFacts,
        LoanBrokerCoverClawbackResolveBrokerIdFacts,
        run_loan_broker_cover_clawback_determine_amount, run_loan_broker_cover_clawback_do_apply,
        run_loan_broker_cover_clawback_preclaim, run_loan_broker_cover_clawback_preflight,
        run_loan_broker_cover_clawback_resolve_broker_id,
    };

    fn base() -> LoanBrokerCoverClawbackPreclaimFacts {
        LoanBrokerCoverClawbackPreclaimFacts {
            broker_id_resolution_result: Ter::TES_SUCCESS,
            broker_exists: true,
            vault_exists: true,
            vault_asset_is_native: false,
            submitter_is_vault_asset_issuer: true,
            amount_is_present: false,
            amount_asset_matches_vault_asset: true,
            claw_amount_can_be_determined: true,
            pseudo_balance_at_least_claw_amount: true,
            issuer_account_exists: true,
            amount_kind: LoanBrokerCoverClawbackAmountKind::Issue,
            mpt_issuance_exists: true,
            mpt_can_clawback: true,
            mpt_issuer_matches_submitter: true,
            issuer_allows_trustline_clawback: true,
            issuer_has_no_freeze: false,
        }
    }

    #[test]
    fn loan_broker_cover_clawback_preflight_rejects_missing_id_and_amount() {
        let result = run_loan_broker_cover_clawback_preflight(
            LoanBrokerCoverClawbackPreflightFacts::default(),
        );

        assert_eq!(result, Ter::TEM_INVALID);
    }

    #[test]
    fn loan_broker_cover_clawback_preflight_rejects_bad_broker_id_and_amounts() {
        let broker =
            run_loan_broker_cover_clawback_preflight(LoanBrokerCoverClawbackPreflightFacts {
                broker_id_is_present: true,
                broker_id_is_zero: true,
                ..LoanBrokerCoverClawbackPreflightFacts::default()
            });
        let native =
            run_loan_broker_cover_clawback_preflight(LoanBrokerCoverClawbackPreflightFacts {
                amount_is_present: true,
                amount_is_native: true,
                amount_is_legal_net: true,
                ..LoanBrokerCoverClawbackPreflightFacts::default()
            });
        let negative =
            run_loan_broker_cover_clawback_preflight(LoanBrokerCoverClawbackPreflightFacts {
                amount_is_present: true,
                amount_is_negative: true,
                amount_is_legal_net: true,
                ..LoanBrokerCoverClawbackPreflightFacts::default()
            });
        let illegal =
            run_loan_broker_cover_clawback_preflight(LoanBrokerCoverClawbackPreflightFacts {
                amount_is_present: true,
                amount_is_legal_net: false,
                ..LoanBrokerCoverClawbackPreflightFacts::default()
            });

        assert_eq!(broker, Ter::TEM_INVALID);
        assert_eq!(native, Ter::TEM_BAD_AMOUNT);
        assert_eq!(negative, Ter::TEM_BAD_AMOUNT);
        assert_eq!(illegal, Ter::TEM_BAD_AMOUNT);
    }

    #[test]
    fn loan_broker_cover_clawback_preflight_rejects_missing_id_mpt_and_bad_holders() {
        let mpt = run_loan_broker_cover_clawback_preflight(LoanBrokerCoverClawbackPreflightFacts {
            amount_is_present: true,
            amount_is_legal_net: true,
            broker_id_missing_amount_is_mpt: true,
            ..LoanBrokerCoverClawbackPreflightFacts::default()
        });
        let account_holder =
            run_loan_broker_cover_clawback_preflight(LoanBrokerCoverClawbackPreflightFacts {
                amount_is_present: true,
                amount_is_legal_net: true,
                broker_id_missing_amount_holder_is_account: true,
                ..LoanBrokerCoverClawbackPreflightFacts::default()
            });
        let zero_holder =
            run_loan_broker_cover_clawback_preflight(LoanBrokerCoverClawbackPreflightFacts {
                amount_is_present: true,
                amount_is_legal_net: true,
                broker_id_missing_amount_holder_is_zero: true,
                ..LoanBrokerCoverClawbackPreflightFacts::default()
            });

        assert_eq!(mpt, Ter::TEM_INVALID);
        assert_eq!(account_holder, Ter::TEM_INVALID);
        assert_eq!(zero_holder, Ter::TEM_INVALID);
    }

    #[test]
    fn loan_broker_cover_clawback_resolve_broker_id_matches_current_cpp_routes() {
        assert_eq!(
            run_loan_broker_cover_clawback_resolve_broker_id(
                LoanBrokerCoverClawbackResolveBrokerIdFacts {
                    broker_id_from_tx: Some("broker-1"),
                    amount_is_present: false,
                    amount_holds_issue: false,
                    holder_account_exists: false,
                    broker_id_from_holder_account: None,
                }
            ),
            Ok("broker-1")
        );
        assert_eq!(
            run_loan_broker_cover_clawback_resolve_broker_id(
                LoanBrokerCoverClawbackResolveBrokerIdFacts::<&str> {
                    broker_id_from_tx: None,
                    amount_is_present: false,
                    amount_holds_issue: false,
                    holder_account_exists: false,
                    broker_id_from_holder_account: None,
                }
            ),
            Err(Ter::TEC_INTERNAL)
        );
        assert_eq!(
            run_loan_broker_cover_clawback_resolve_broker_id(
                LoanBrokerCoverClawbackResolveBrokerIdFacts::<&str> {
                    broker_id_from_tx: None,
                    amount_is_present: true,
                    amount_holds_issue: false,
                    holder_account_exists: false,
                    broker_id_from_holder_account: None,
                }
            ),
            Err(Ter::TEC_INTERNAL)
        );
        assert_eq!(
            run_loan_broker_cover_clawback_resolve_broker_id(
                LoanBrokerCoverClawbackResolveBrokerIdFacts::<&str> {
                    broker_id_from_tx: None,
                    amount_is_present: true,
                    amount_holds_issue: true,
                    holder_account_exists: false,
                    broker_id_from_holder_account: None,
                }
            ),
            Err(Ter::TEC_NO_ENTRY)
        );
        assert_eq!(
            run_loan_broker_cover_clawback_resolve_broker_id(
                LoanBrokerCoverClawbackResolveBrokerIdFacts::<&str> {
                    broker_id_from_tx: None,
                    amount_is_present: true,
                    amount_holds_issue: true,
                    holder_account_exists: true,
                    broker_id_from_holder_account: None,
                }
            ),
            Err(Ter::TEC_OBJECT_NOT_FOUND)
        );
        assert_eq!(
            run_loan_broker_cover_clawback_resolve_broker_id(
                LoanBrokerCoverClawbackResolveBrokerIdFacts {
                    broker_id_from_tx: None,
                    amount_is_present: true,
                    amount_holds_issue: true,
                    holder_account_exists: true,
                    broker_id_from_holder_account: Some("broker-2"),
                }
            ),
            Ok("broker-2")
        );
    }

    #[test]
    fn loan_broker_cover_clawback_preclaim_returns_broker_id_resolution_failure() {
        let result =
            run_loan_broker_cover_clawback_preclaim(LoanBrokerCoverClawbackPreclaimFacts {
                broker_id_resolution_result: Ter::TEC_OBJECT_NOT_FOUND,
                ..base()
            });

        assert_eq!(result, Ter::TEC_OBJECT_NOT_FOUND);
        assert_eq!(trans_token(result), "tecOBJECT_NOT_FOUND");
    }

    #[test]
    fn loan_broker_cover_clawback_preclaim_rejects_missing_broker_and_vault() {
        let broker =
            run_loan_broker_cover_clawback_preclaim(LoanBrokerCoverClawbackPreclaimFacts {
                broker_exists: false,
                ..base()
            });
        let vault = run_loan_broker_cover_clawback_preclaim(LoanBrokerCoverClawbackPreclaimFacts {
            vault_exists: false,
            ..base()
        });

        assert_eq!(broker, Ter::TEC_NO_ENTRY);
        assert_eq!(vault, Ter::TEF_BAD_LEDGER);
    }

    #[test]
    fn loan_broker_cover_clawback_preclaim_enforces_native_and_issuer_permissions() {
        let native =
            run_loan_broker_cover_clawback_preclaim(LoanBrokerCoverClawbackPreclaimFacts {
                vault_asset_is_native: true,
                ..base()
            });
        let issuer =
            run_loan_broker_cover_clawback_preclaim(LoanBrokerCoverClawbackPreclaimFacts {
                submitter_is_vault_asset_issuer: false,
                ..base()
            });

        assert_eq!(native, Ter::TEC_NO_PERMISSION);
        assert_eq!(issuer, Ter::TEC_NO_PERMISSION);
    }

    #[test]
    fn loan_broker_cover_clawback_preclaim_rejects_wrong_asset_and_minimum_cover() {
        let wrong_asset =
            run_loan_broker_cover_clawback_preclaim(LoanBrokerCoverClawbackPreclaimFacts {
                amount_is_present: true,
                amount_asset_matches_vault_asset: false,
                ..base()
            });
        let minimum =
            run_loan_broker_cover_clawback_preclaim(LoanBrokerCoverClawbackPreclaimFacts {
                claw_amount_can_be_determined: false,
                ..base()
            });

        assert_eq!(wrong_asset, Ter::TEC_WRONG_ASSET);
        assert_eq!(minimum, Ter::TEC_INSUFFICIENT_FUNDS);
    }

    #[test]
    fn loan_broker_cover_clawback_preclaim_rejects_balance_and_missing_issuer() {
        let balance =
            run_loan_broker_cover_clawback_preclaim(LoanBrokerCoverClawbackPreclaimFacts {
                pseudo_balance_at_least_claw_amount: false,
                ..base()
            });
        let issuer =
            run_loan_broker_cover_clawback_preclaim(LoanBrokerCoverClawbackPreclaimFacts {
                issuer_account_exists: false,
                ..base()
            });

        assert_eq!(balance, Ter::TEC_INTERNAL);
        assert_eq!(issuer, Ter::TEF_BAD_LEDGER);
    }

    #[test]
    fn loan_broker_cover_clawback_preclaim_enforces_iou_issuer_flags() {
        let no_clawback =
            run_loan_broker_cover_clawback_preclaim(LoanBrokerCoverClawbackPreclaimFacts {
                issuer_allows_trustline_clawback: false,
                ..base()
            });
        let no_freeze =
            run_loan_broker_cover_clawback_preclaim(LoanBrokerCoverClawbackPreclaimFacts {
                issuer_has_no_freeze: true,
                ..base()
            });

        assert_eq!(no_clawback, Ter::TEC_NO_PERMISSION);
        assert_eq!(no_freeze, Ter::TEC_NO_PERMISSION);
    }

    #[test]
    fn loan_broker_cover_clawback_preclaim_enforces_mpt_permission_split() {
        let missing =
            run_loan_broker_cover_clawback_preclaim(LoanBrokerCoverClawbackPreclaimFacts {
                amount_kind: LoanBrokerCoverClawbackAmountKind::Mpt,
                mpt_issuance_exists: false,
                ..base()
            });
        let no_clawback =
            run_loan_broker_cover_clawback_preclaim(LoanBrokerCoverClawbackPreclaimFacts {
                amount_kind: LoanBrokerCoverClawbackAmountKind::Mpt,
                mpt_can_clawback: false,
                ..base()
            });
        let issuer_mismatch =
            run_loan_broker_cover_clawback_preclaim(LoanBrokerCoverClawbackPreclaimFacts {
                amount_kind: LoanBrokerCoverClawbackAmountKind::Mpt,
                mpt_issuer_matches_submitter: false,
                ..base()
            });

        assert_eq!(missing, Ter::TEC_OBJECT_NOT_FOUND);
        assert_eq!(no_clawback, Ter::TEC_NO_PERMISSION);
        assert_eq!(issuer_mismatch, Ter::TEC_INTERNAL);
    }

    #[test]
    fn loan_broker_cover_clawback_determine_amount_matches_current_cpp_clamp_rules() {
        assert_eq!(
            run_loan_broker_cover_clawback_determine_amount(
                TestAmount {
                    value: 0,
                    native: false,
                },
                None,
            ),
            Err(Ter::TEC_INSUFFICIENT_FUNDS)
        );
        assert_eq!(
            run_loan_broker_cover_clawback_determine_amount(
                TestAmount {
                    value: 50,
                    native: false,
                },
                None,
            ),
            Ok(TestAmount {
                value: 50,
                native: false,
            })
        );
        assert_eq!(
            run_loan_broker_cover_clawback_determine_amount(
                TestAmount {
                    value: 50,
                    native: false,
                },
                Some(TestAmount {
                    value: 0,
                    native: false,
                }),
            ),
            Ok(TestAmount {
                value: 50,
                native: false,
            })
        );
        assert_eq!(
            run_loan_broker_cover_clawback_determine_amount(
                TestAmount {
                    value: 50,
                    native: false,
                },
                Some(TestAmount {
                    value: 75,
                    native: false,
                }),
            ),
            Ok(TestAmount {
                value: 50,
                native: false,
            })
        );
        assert_eq!(
            run_loan_broker_cover_clawback_determine_amount(
                TestAmount {
                    value: 50,
                    native: false,
                },
                Some(TestAmount {
                    value: 25,
                    native: false,
                }),
            ),
            Ok(TestAmount {
                value: 25,
                native: false,
            })
        );
    }

    #[derive(Clone, Debug, PartialEq, Eq)]
    struct TestBroker {
        vault_id: &'static str,
        pseudo_account_id: &'static str,
        cover_available: i64,
        steps: Rc<RefCell<Vec<String>>>,
    }

    impl LoanBrokerCoverClawbackDoApplyBroker for TestBroker {
        type AccountId = &'static str;
        type Amount = TestAmount;
        type Asset = &'static str;
        type VaultId = &'static str;

        fn vault_id(&self) -> &Self::VaultId {
            &self.vault_id
        }

        fn pseudo_account_id(&self) -> &Self::AccountId {
            &self.pseudo_account_id
        }

        fn subtract_cover_available(&mut self, amount: &Self::Amount) {
            self.cover_available -= amount.value;
            self.steps
                .borrow_mut()
                .push(format!("cover-={}", amount.value));
        }
    }

    #[derive(Clone, Debug, PartialEq, Eq)]
    struct TestVault {
        asset: &'static str,
    }

    impl LoanBrokerCoverClawbackDoApplyVault for TestVault {
        type Asset = &'static str;

        fn asset(&self) -> &Self::Asset {
            &self.asset
        }
    }

    #[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
    struct TestAmount {
        value: i64,
        native: bool,
    }

    impl LoanBrokerCoverClawbackDoApplyAmount for TestAmount {
        fn is_native(&self) -> bool {
            self.native
        }
    }

    impl LoanBrokerCoverClawbackDeterminableAmount for TestAmount {
        fn is_zero(&self) -> bool {
            self.value == 0
        }

        fn is_positive(&self) -> bool {
            self.value > 0
        }
    }

    struct TestSink {
        steps: Rc<RefCell<Vec<String>>>,
        broker: Option<TestBroker>,
        vault: Option<TestVault>,
        send_result: Ter,
        observed_pseudo_account: Option<&'static str>,
        observed_destination: Option<&'static str>,
        observed_amount: Option<TestAmount>,
    }

    impl LoanBrokerCoverClawbackDoApplySink for TestSink {
        type Broker = TestBroker;
        type Vault = TestVault;
        type BrokerId = &'static str;
        type AccountId = &'static str;
        type Amount = TestAmount;
        type Asset = &'static str;
        type VaultId = &'static str;

        fn read_broker(&mut self, broker_id: &Self::BrokerId) -> Option<Self::Broker> {
            self.steps
                .borrow_mut()
                .push(format!("read_broker={broker_id}"));
            self.broker.take()
        }

        fn read_vault(&mut self, vault_id: &Self::VaultId) -> Option<Self::Vault> {
            self.steps
                .borrow_mut()
                .push(format!("read_vault={vault_id}"));
            self.vault.take()
        }

        fn update_broker(&mut self, _broker: &Self::Broker) {
            self.steps.borrow_mut().push("update_broker".to_string());
        }

        fn associate_asset(&mut self, _broker: &Self::Broker, asset: &Self::Asset) {
            self.steps
                .borrow_mut()
                .push(format!("associate_asset={asset}"));
        }

        fn send_asset(
            &mut self,
            pseudo_account_id: &Self::AccountId,
            destination: &Self::AccountId,
            amount: &Self::Amount,
        ) -> Ter {
            self.steps.borrow_mut().push("send_asset".to_string());
            self.observed_pseudo_account = Some(*pseudo_account_id);
            self.observed_destination = Some(*destination);
            self.observed_amount = Some(amount.clone());
            self.send_result
        }
    }

    fn build_sink(steps: Rc<RefCell<Vec<String>>>) -> TestSink {
        TestSink {
            steps: Rc::clone(&steps),
            broker: Some(TestBroker {
                vault_id: "vault-1",
                pseudo_account_id: "pseudo-1",
                cover_available: 90,
                steps,
            }),
            vault: Some(TestVault { asset: "USD" }),
            send_result: Ter::TES_SUCCESS,
            observed_pseudo_account: None,
            observed_destination: None,
            observed_amount: None,
        }
    }

    #[test]
    fn loan_broker_cover_clawback_do_apply_maps_broker_resolution_failure_to_internal() {
        let steps = Rc::new(RefCell::new(Vec::new()));
        let mut sink = build_sink(Rc::clone(&steps));

        let result = run_loan_broker_cover_clawback_do_apply(
            &mut sink,
            &"issuer",
            || Err(Ter::TEC_OBJECT_NOT_FOUND),
            |_, _| {
                Ok(TestAmount {
                    value: 15,
                    native: false,
                })
            },
        );

        assert_eq!(result, Ter::TEC_INTERNAL);
        assert!(steps.borrow().is_empty());
    }

    #[test]
    fn loan_broker_cover_clawback_do_apply_maps_missing_broker_and_vault_to_internal() {
        let steps = Rc::new(RefCell::new(Vec::new()));
        let mut missing_broker = TestSink {
            steps: Rc::clone(&steps),
            broker: None,
            vault: Some(TestVault { asset: "USD" }),
            send_result: Ter::TES_SUCCESS,
            observed_pseudo_account: None,
            observed_destination: None,
            observed_amount: None,
        };

        let missing_broker_result = run_loan_broker_cover_clawback_do_apply(
            &mut missing_broker,
            &"issuer",
            || Ok("broker-1"),
            |_, _| {
                Ok(TestAmount {
                    value: 15,
                    native: false,
                })
            },
        );
        assert_eq!(missing_broker_result, Ter::TEC_INTERNAL);
        assert_eq!(steps.borrow().as_slice(), ["read_broker=broker-1"]);

        steps.borrow_mut().clear();
        let mut missing_vault = TestSink {
            steps: Rc::clone(&steps),
            broker: Some(TestBroker {
                vault_id: "vault-1",
                pseudo_account_id: "pseudo-1",
                cover_available: 90,
                steps: Rc::clone(&steps),
            }),
            vault: None,
            send_result: Ter::TES_SUCCESS,
            observed_pseudo_account: None,
            observed_destination: None,
            observed_amount: None,
        };

        let missing_vault_result = run_loan_broker_cover_clawback_do_apply(
            &mut missing_vault,
            &"issuer",
            || Ok("broker-1"),
            |_, _| {
                Ok(TestAmount {
                    value: 15,
                    native: false,
                })
            },
        );
        assert_eq!(missing_vault_result, Ter::TEC_INTERNAL);
        assert_eq!(
            steps.borrow().as_slice(),
            ["read_broker=broker-1", "read_vault=vault-1"]
        );
    }

    #[test]
    fn loan_broker_cover_clawback_do_apply_maps_claw_amount_failures_to_internal() {
        let steps = Rc::new(RefCell::new(Vec::new()));
        let mut sink = build_sink(Rc::clone(&steps));

        let amount_error = run_loan_broker_cover_clawback_do_apply(
            &mut sink,
            &"issuer",
            || Ok("broker-1"),
            |_, _| Err(Ter::TEC_INSUFFICIENT_FUNDS),
        );
        assert_eq!(amount_error, Ter::TEC_INTERNAL);
        assert_eq!(
            steps.borrow().as_slice(),
            ["read_broker=broker-1", "read_vault=vault-1"]
        );

        steps.borrow_mut().clear();
        let mut sink = build_sink(Rc::clone(&steps));
        let native = run_loan_broker_cover_clawback_do_apply(
            &mut sink,
            &"issuer",
            || Ok("broker-1"),
            |_, _| {
                Ok(TestAmount {
                    value: 15,
                    native: true,
                })
            },
        );
        assert_eq!(native, Ter::TEC_INTERNAL);
        assert_eq!(
            steps.borrow().as_slice(),
            ["read_broker=broker-1", "read_vault=vault-1"]
        );
    }

    #[test]
    fn loan_broker_cover_clawback_do_apply_returns_send_failure_after_updates() {
        let steps = Rc::new(RefCell::new(Vec::new()));
        let mut sink = build_sink(Rc::clone(&steps));
        sink.send_result = Ter::TER_NO_AUTH;

        let result = run_loan_broker_cover_clawback_do_apply(
            &mut sink,
            &"issuer",
            || {
                steps.borrow_mut().push("determine_broker_id".to_string());
                Ok("broker-1")
            },
            |_, _| {
                steps.borrow_mut().push("determine_claw_amount".to_string());
                Ok(TestAmount {
                    value: 15,
                    native: false,
                })
            },
        );

        assert_eq!(result, Ter::TER_NO_AUTH);
        assert_eq!(
            steps.borrow().as_slice(),
            [
                "determine_broker_id",
                "read_broker=broker-1",
                "read_vault=vault-1",
                "determine_claw_amount",
                "cover-=15",
                "update_broker",
                "associate_asset=USD",
                "send_asset",
            ]
        );
        assert_eq!(sink.observed_pseudo_account, Some("pseudo-1"));
        assert_eq!(sink.observed_destination, Some("issuer"));
        assert_eq!(
            sink.observed_amount,
            Some(TestAmount {
                value: 15,
                native: false
            })
        );
    }

    #[test]
    fn loan_broker_cover_clawback_do_apply_runs_current_on_success() {
        let steps = Rc::new(RefCell::new(Vec::new()));
        let mut sink = build_sink(Rc::clone(&steps));

        let result = run_loan_broker_cover_clawback_do_apply(
            &mut sink,
            &"issuer",
            || {
                steps.borrow_mut().push("determine_broker_id".to_string());
                Ok("broker-1")
            },
            |_, _| {
                steps.borrow_mut().push("determine_claw_amount".to_string());
                Ok(TestAmount {
                    value: 15,
                    native: false,
                })
            },
        );

        assert_eq!(result, Ter::TES_SUCCESS);
        assert_eq!(
            steps.borrow().as_slice(),
            [
                "determine_broker_id",
                "read_broker=broker-1",
                "read_vault=vault-1",
                "determine_claw_amount",
                "cover-=15",
                "update_broker",
                "associate_asset=USD",
                "send_asset",
            ]
        );
        assert_eq!(sink.observed_pseudo_account, Some("pseudo-1"));
        assert_eq!(sink.observed_destination, Some("issuer"));
        assert_eq!(
            sink.observed_amount,
            Some(TestAmount {
                value: 15,
                native: false
            })
        );
    }
}
