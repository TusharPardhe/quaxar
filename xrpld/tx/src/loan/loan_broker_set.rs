//! Current the reference implementation control flow.
//!
//! This ports the current deterministic branch ordering around:
//!
//! - the shared lending dependency gate,
//! - data-length and numeric-range validation in `preflight(...)`,
//! - the fixed-field rejection rules for updates,
//! - vault existence and ownership checks in `preclaim(...)`,
//! - the update-versus-create branch split,
//! - the existing-broker ownership and vault checks,
//! - the current debt-maximum floor guard,
//! - the create-path holding and freeze checks,
//! - the representability check for `sfDebtMaximum`,
//! - and the current top-level `doApply()` create-versus-update ordering while
//!   keeping construction and storage callbacks explicit.
//!
//! The apply-time object creation and mutation stack is still outside this
//! slice.

use protocol::{NotTec, Ter, is_tes_success};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct LoanBrokerSetPreflightFacts {
    pub data_is_present: bool,
    pub data_is_empty: bool,
    pub data_length_is_valid: bool,
    pub management_fee_rate_is_valid: bool,
    pub cover_rate_minimum_is_valid: bool,
    pub cover_rate_liquidation_is_valid: bool,
    pub debt_maximum_is_valid: bool,
    pub loan_broker_id_is_present: bool,
    pub management_fee_rate_is_present: bool,
    pub cover_rate_minimum_is_present: bool,
    pub cover_rate_liquidation_is_present: bool,
    pub loan_broker_id_is_zero: bool,
    pub vault_id_is_present: bool,
    pub vault_id_is_zero: bool,
    pub cover_rate_minimum_value: Option<u32>,
    pub cover_rate_liquidation_value: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct LoanBrokerSetPreclaimFacts {
    pub vault_exists: bool,
    pub submitter_is_vault_owner: bool,
    pub broker_id_is_present: bool,
    pub broker_exists: bool,
    pub vault_id_matches_existing_broker: bool,
    pub submitter_is_broker_owner: bool,
    pub debt_maximum_is_zero_or_not_below_current_debt: bool,
    pub debt_maximum_is_present: bool,
    pub debt_maximum_is_representable: bool,
    pub can_add_holding_result: Ter,
    pub check_frozen_result: Ter,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoanBrokerSetDoApplyFacts<AccountId, BrokerId, VaultId, Sequence, Amount, Data> {
    pub account: AccountId,
    pub broker_id: Option<BrokerId>,
    pub vault_id: VaultId,
    pub sequence: Sequence,
    pub pre_fee_balance: Amount,
    pub data: Option<Data>,
    pub management_fee_rate: Option<u32>,
    pub debt_maximum: Option<Amount>,
    pub cover_rate_minimum: Option<u32>,
    pub cover_rate_liquidation: Option<u32>,
}

pub trait LoanBrokerSetDoApplyBroker {
    type AccountId;
    type Amount;
    type Asset;
    type VaultId;
    type Sequence;
    type Data;

    fn vault_id(&self) -> &Self::VaultId;
    fn set_data(&mut self, value: Self::Data);
    fn set_debt_maximum(&mut self, value: Self::Amount);
    fn set_sequence(&mut self, value: Self::Sequence);
    fn set_vault_id(&mut self, value: Self::VaultId);
    fn set_owner(&mut self, value: Self::AccountId);
    fn set_account(&mut self, value: Self::AccountId);
    fn set_loan_sequence(&mut self, value: Self::Sequence);
    fn set_management_fee_rate(&mut self, value: u32);
    fn set_cover_rate_minimum(&mut self, value: u32);
    fn set_cover_rate_liquidation(&mut self, value: u32);
}

pub trait LoanBrokerSetDoApplyVault {
    type AccountId;
    type Asset;

    fn account_id(&self) -> &Self::AccountId;
    fn asset(&self) -> &Self::Asset;
}

pub trait LoanBrokerSetDoApplyPseudoAccount {
    type AccountId;

    fn account_id(&self) -> &Self::AccountId;
}

pub trait LoanBrokerSetDoApplySink {
    type Broker: LoanBrokerSetDoApplyBroker<
            AccountId = Self::AccountId,
            Amount = Self::Amount,
            Asset = Self::Asset,
            VaultId = Self::VaultId,
            Sequence = Self::Sequence,
            Data = Self::Data,
        >;
    type Vault: LoanBrokerSetDoApplyVault<AccountId = Self::AccountId, Asset = Self::Asset>;
    type Owner;
    type PseudoAccount: LoanBrokerSetDoApplyPseudoAccount<AccountId = Self::AccountId>;
    type AccountId;
    type BrokerId;
    type VaultId;
    type Amount;
    type Asset;
    type Sequence: From<u32>;
    type Data;
    type OwnerCount;

    fn read_broker(&mut self, broker_id: &Self::BrokerId) -> Option<Self::Broker>;
    fn read_vault(&mut self, vault_id: &Self::VaultId) -> Option<Self::Vault>;
    fn read_owner(&mut self, account: &Self::AccountId) -> Option<Self::Owner>;
    fn make_broker(&mut self, account: &Self::AccountId, sequence: &Self::Sequence)
    -> Self::Broker;
    fn dir_link_broker(&mut self, broker: &mut Self::Broker) -> Ter;
    fn dir_link_vault(
        &mut self,
        broker: &mut Self::Broker,
        vault_pseudo_id: &Self::AccountId,
    ) -> Ter;
    fn adjust_owner_count(&mut self, owner: &mut Self::Owner, delta: u32) -> Self::OwnerCount;
    fn account_reserve(&mut self, owner_count: &Self::OwnerCount) -> Self::Amount;
    fn create_pseudo_account(&mut self, broker: &Self::Broker) -> Result<Self::PseudoAccount, Ter>;
    fn add_empty_holding(
        &mut self,
        pseudo_account_id: &Self::AccountId,
        pre_fee_balance: &Self::Amount,
        asset: &Self::Asset,
    ) -> Ter;
    fn update_broker(&mut self, broker: &Self::Broker);
    fn insert_broker(&mut self, broker: &Self::Broker);
    fn associate_asset(&mut self, broker: &Self::Broker, asset: &Self::Asset);
}

pub fn run_loan_broker_set_check_extra_features(
    single_asset_vault_enabled: bool,
    check_lending_protocol_dependencies: impl FnOnce() -> bool,
) -> bool {
    single_asset_vault_enabled && check_lending_protocol_dependencies()
}

pub fn run_loan_broker_set_preflight(facts: LoanBrokerSetPreflightFacts) -> NotTec {
    if facts.data_is_present && !facts.data_is_empty && !facts.data_length_is_valid {
        return Ter::TEM_INVALID;
    }

    if !facts.management_fee_rate_is_valid {
        return Ter::TEM_INVALID;
    }

    if !facts.cover_rate_minimum_is_valid {
        return Ter::TEM_INVALID;
    }

    if !facts.cover_rate_liquidation_is_valid {
        return Ter::TEM_INVALID;
    }

    if !facts.debt_maximum_is_valid {
        return Ter::TEM_INVALID;
    }

    if facts.loan_broker_id_is_present {
        if facts.management_fee_rate_is_present
            || facts.cover_rate_minimum_is_present
            || facts.cover_rate_liquidation_is_present
        {
            return Ter::TEM_INVALID;
        }

        if facts.loan_broker_id_is_zero {
            return Ter::TEM_INVALID;
        }
    }

    if facts.vault_id_is_present && facts.vault_id_is_zero {
        return Ter::TEM_INVALID;
    }

    let minimum_zero = facts.cover_rate_minimum_value.unwrap_or(0) == 0;
    let liquidation_zero = facts.cover_rate_liquidation_value.unwrap_or(0) == 0;
    if minimum_zero != liquidation_zero {
        return Ter::TEM_INVALID;
    }

    Ter::TES_SUCCESS
}

pub fn run_loan_broker_set_preclaim(facts: LoanBrokerSetPreclaimFacts) -> Ter {
    if !facts.vault_exists {
        return Ter::TEC_NO_ENTRY;
    }

    if !facts.submitter_is_vault_owner {
        return Ter::TEC_NO_PERMISSION;
    }

    if facts.broker_id_is_present {
        if !facts.broker_exists {
            return Ter::TEC_NO_ENTRY;
        }

        if !facts.vault_id_matches_existing_broker {
            return Ter::TEC_NO_PERMISSION;
        }

        if !facts.submitter_is_broker_owner {
            return Ter::TEC_NO_PERMISSION;
        }

        if !facts.debt_maximum_is_zero_or_not_below_current_debt {
            return Ter::TEC_LIMIT_EXCEEDED;
        }
    } else {
        if !is_tes_success(facts.can_add_holding_result) {
            return facts.can_add_holding_result;
        }

        if !is_tes_success(facts.check_frozen_result) {
            return facts.check_frozen_result;
        }
    }

    if facts.debt_maximum_is_present && !facts.debt_maximum_is_representable {
        return Ter::TEC_PRECISION_LOSS;
    }

    Ter::TES_SUCCESS
}

#[allow(clippy::too_many_arguments)]
pub fn run_loan_broker_set_do_apply<Sink>(
    sink: &mut Sink,
    facts: LoanBrokerSetDoApplyFacts<
        Sink::AccountId,
        Sink::BrokerId,
        Sink::VaultId,
        Sink::Sequence,
        Sink::Amount,
        Sink::Data,
    >,
) -> Ter
where
    Sink: LoanBrokerSetDoApplySink,
    Sink::AccountId: Clone,
    Sink::Amount: PartialOrd + Clone,
    Sink::Sequence: Clone,
{
    if let Some(broker_id) = facts.broker_id.as_ref() {
        let mut broker = match sink.read_broker(broker_id) {
            Some(broker) => broker,
            None => return Ter::TEF_BAD_LEDGER,
        };

        let vault = match sink.read_vault(broker.vault_id()) {
            Some(vault) => vault,
            None => return Ter::TEC_INTERNAL,
        };

        if let Some(data) = facts.data {
            broker.set_data(data);
        }
        if let Some(debt_maximum) = facts.debt_maximum {
            broker.set_debt_maximum(debt_maximum);
        }

        sink.update_broker(&broker);
        sink.associate_asset(&broker, vault.asset());
        return Ter::TES_SUCCESS;
    }

    let vault = match sink.read_vault(&facts.vault_id) {
        Some(vault) => vault,
        None => return Ter::TEF_BAD_LEDGER,
    };

    let mut owner = match sink.read_owner(&facts.account) {
        Some(owner) => owner,
        None => return Ter::TEF_BAD_LEDGER,
    };

    let mut broker = sink.make_broker(&facts.account, &facts.sequence);

    let ter = sink.dir_link_broker(&mut broker);
    if !is_tes_success(ter) {
        return ter;
    }

    let ter = sink.dir_link_vault(&mut broker, vault.account_id());
    if !is_tes_success(ter) {
        return ter;
    }

    let owner_count = sink.adjust_owner_count(&mut owner, 2);
    if facts.pre_fee_balance < sink.account_reserve(&owner_count) {
        return Ter::TEC_INSUFFICIENT_RESERVE;
    }

    let pseudo_account = match sink.create_pseudo_account(&broker) {
        Ok(pseudo_account) => pseudo_account,
        Err(ter) => return ter,
    };

    let ter = sink.add_empty_holding(
        pseudo_account.account_id(),
        &facts.pre_fee_balance,
        vault.asset(),
    );
    if !is_tes_success(ter) {
        return ter;
    }

    broker.set_sequence(facts.sequence.clone());
    broker.set_vault_id(facts.vault_id);
    broker.set_owner(facts.account.clone());
    broker.set_account(pseudo_account.account_id().clone());
    broker.set_loan_sequence(Sink::Sequence::from(1_u32));
    if let Some(data) = facts.data {
        broker.set_data(data);
    }
    if let Some(rate) = facts.management_fee_rate {
        broker.set_management_fee_rate(rate);
    }
    if let Some(debt_maximum) = facts.debt_maximum {
        broker.set_debt_maximum(debt_maximum);
    }
    if let Some(cover_rate_minimum) = facts.cover_rate_minimum {
        broker.set_cover_rate_minimum(cover_rate_minimum);
    }
    if let Some(cover_rate_liquidation) = facts.cover_rate_liquidation {
        broker.set_cover_rate_liquidation(cover_rate_liquidation);
    }

    sink.insert_broker(&broker);
    sink.associate_asset(&broker, vault.asset());
    Ter::TES_SUCCESS
}
