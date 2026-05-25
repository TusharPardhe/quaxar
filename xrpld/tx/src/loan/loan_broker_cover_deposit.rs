//! Deterministic
//! the reference implementation control flow.
//!
//! This ports the current top-level branch ordering around:
//!
//! - the shared lending dependency gate,
//! - zero broker-id rejection in `preflight(...)`,
//! - positive-amount and legal-net rejection in `preflight(...)`,
//! - missing-broker and wrong-owner rejection in `preclaim(...)`,
//! - the impossible missing-vault fallback to `tefBAD_LEDGER`,
//! - asset mismatch rejection,
//! - transfer, frozen, deep-frozen, and auth checks,
//! - the final available-balance gate,
//! - and the current `doApply()` mutation order over broker load, vault load,
//!   transfer, cover update, persistence, and asset association.

use protocol::{NotTec, Ter, is_tes_success};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct LoanBrokerCoverDepositPreflightFacts {
    pub broker_id_is_zero: bool,
    pub amount_is_positive: bool,
    pub amount_is_legal_net: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct LoanBrokerCoverDepositPreclaimFacts {
    pub broker_exists: bool,
    pub submitter_is_broker_owner: bool,
    pub vault_exists: bool,
    pub amount_matches_vault_asset: bool,
    pub can_transfer_result: Ter,
    pub frozen_result: Ter,
    pub deep_frozen_result: Ter,
    pub require_auth_result: Ter,
    pub balance_is_less_than_amount: bool,
}

pub trait LoanBrokerCoverDepositDoApplyBroker {
    type AccountId;
    type Amount;
    type Asset;
    type VaultId;

    fn vault_id(&self) -> &Self::VaultId;
    fn pseudo_account_id(&self) -> &Self::AccountId;
    fn add_cover_available(&mut self, amount: Self::Amount);
}

pub trait LoanBrokerCoverDepositDoApplyVault {
    type Asset;

    fn asset(&self) -> &Self::Asset;
}

pub fn run_loan_broker_cover_deposit_check_extra_features(
    single_asset_vault_enabled: bool,
    check_lending_protocol_dependencies: impl FnOnce() -> bool,
) -> bool {
    single_asset_vault_enabled && check_lending_protocol_dependencies()
}

pub fn run_loan_broker_cover_deposit_preflight(
    facts: LoanBrokerCoverDepositPreflightFacts,
) -> NotTec {
    if facts.broker_id_is_zero {
        return Ter::TEM_INVALID;
    }

    if !facts.amount_is_positive {
        return Ter::TEM_BAD_AMOUNT;
    }

    if !facts.amount_is_legal_net {
        return Ter::TEM_BAD_AMOUNT;
    }

    Ter::TES_SUCCESS
}

pub fn run_loan_broker_cover_deposit_preclaim(facts: LoanBrokerCoverDepositPreclaimFacts) -> Ter {
    if !facts.broker_exists {
        return Ter::TEC_NO_ENTRY;
    }

    if !facts.submitter_is_broker_owner {
        return Ter::TEC_NO_PERMISSION;
    }

    if !facts.vault_exists {
        return Ter::TEF_BAD_LEDGER;
    }

    if !facts.amount_matches_vault_asset {
        return Ter::TEC_WRONG_ASSET;
    }

    if !is_tes_success(facts.can_transfer_result) {
        return facts.can_transfer_result;
    }

    if !is_tes_success(facts.frozen_result) {
        return facts.frozen_result;
    }

    if !is_tes_success(facts.deep_frozen_result) {
        return facts.deep_frozen_result;
    }

    if !is_tes_success(facts.require_auth_result) {
        return facts.require_auth_result;
    }

    if facts.balance_is_less_than_amount {
        return Ter::TEC_INSUFFICIENT_FUNDS;
    }

    Ter::TES_SUCCESS
}

#[allow(clippy::too_many_arguments)]
pub fn run_loan_broker_cover_deposit_do_apply<
    Broker,
    Vault,
    ReadBroker,
    ReadVault,
    SendAssets,
    UpdateBroker,
    AssociateAsset,
>(
    submitter_account: &Broker::AccountId,
    amount: &Broker::Amount,
    read_broker: ReadBroker,
    read_vault: ReadVault,
    send_assets: SendAssets,
    update_broker: UpdateBroker,
    associate_asset: AssociateAsset,
) -> Ter
where
    Broker: LoanBrokerCoverDepositDoApplyBroker,
    Vault: LoanBrokerCoverDepositDoApplyVault<Asset = Broker::Asset>,
    Broker::Amount: Clone,
    ReadBroker: FnOnce() -> Option<Broker>,
    ReadVault: FnOnce(&Broker::VaultId) -> Option<Vault>,
    SendAssets: FnOnce(&Broker::AccountId, &Broker::AccountId, &Broker::Amount) -> Ter,
    UpdateBroker: FnOnce(&mut Broker),
    AssociateAsset: FnOnce(&Broker, &Broker::Asset),
{
    let mut broker = match read_broker() {
        Some(broker) => broker,
        None => return Ter::TEF_INTERNAL,
    };

    let vault = match read_vault(broker.vault_id()) {
        Some(vault) => vault,
        None => return Ter::TEF_INTERNAL,
    };

    let broker_pseudo_id = broker.pseudo_account_id();
    let transfer_result = send_assets(submitter_account, broker_pseudo_id, amount);
    if !is_tes_success(transfer_result) {
        return transfer_result;
    }

    broker.add_cover_available(amount.clone());
    update_broker(&mut broker);
    associate_asset(&broker, vault.asset());

    Ter::TES_SUCCESS
}
