//! App-owned `NetworkOPs` validation ingress and publication runtime.
//!
//! This ports the narrow `NetworkOPsImp::recvValidation(...)` /
//! `pubValidation(...)` ownership that now has enough landed Rust seams:
//! - dedupe through the current `pendingValidations_` rule keyed by ledger hash,
//! - trust / listing / validation-store updates through the shared app validations owner,
//! - current `validationReceived` JSON shaping for downstream subscribers,
//! - and the relay gate for trusted versus optionally-untrusted validations.

use crate::consensus::rcl_validations::{
    RclValidationAcceptanceSink, RclValidationJournal, SharedAppValidations,
    handle_new_validation_with_store,
};
use crate::state::app_registry::AppJournal;
use crate::state::application_root::ApplicationRoot;
use crate::validator::validator_list::{SystemValidatorListClock, ValidatorList};
use basics::base_uint::Uint256;
use basics::str_hex::str_hex;
use protocol::{JsonValue, PublicKey, STValidation, get_field_by_symbol};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use xrpl_core::{NetworkIDService, ServiceRegistry};

pub trait NetworkOpsValidationPublisher: Send + Sync + 'static {
    fn publish_validation(&self, message: JsonValue);
}

#[derive(Debug, Default)]
pub struct NullNetworkOpsValidationPublisher;

impl NetworkOpsValidationPublisher for NullNetworkOpsValidationPublisher {
    fn publish_validation(&self, _message: JsonValue) {}
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AppNetworkOpsValidationRuntimeSnapshot {
    pub pending_validations: usize,
    pub has_publisher: bool,
    pub network_id: u32,
    pub relay_untrusted_validations: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppNetworkOpsValidationReceiveReport {
    pub ledger_hash: Uint256,
    pub source: String,
    pub bypass_accept: bool,
    pub relay: bool,
    pub published: bool,
    pub trusted: bool,
    pub full: bool,
    pub current: bool,
}

pub struct AppNetworkOpsValidationRuntime {
    validations: SharedAppValidations<crate::state::time_keeper::SystemTimeKeeperClock>,
    validators: Arc<ValidatorList<SystemValidatorListClock>>,
    pending_validations: Mutex<BTreeSet<Uint256>>,
    publisher: Mutex<Option<Arc<dyn NetworkOpsValidationPublisher>>>,
    network_id: AtomicU32,
    relay_untrusted_validations: AtomicBool,
    journal: Arc<AppJournal>,
}

impl std::fmt::Debug for AppNetworkOpsValidationRuntime {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("AppNetworkOpsValidationRuntime")
            .field("snapshot", &self.snapshot())
            .finish()
    }
}

impl AppNetworkOpsValidationRuntime {
    pub fn new(
        validations: SharedAppValidations<crate::state::time_keeper::SystemTimeKeeperClock>,
        validators: Arc<ValidatorList<SystemValidatorListClock>>,
        network_id: u32,
        relay_untrusted_validations: bool,
        journal: Arc<AppJournal>,
    ) -> Self {
        Self {
            validations,
            validators,
            pending_validations: Mutex::new(BTreeSet::new()),
            publisher: Mutex::new(None),
            network_id: AtomicU32::new(network_id),
            relay_untrusted_validations: AtomicBool::new(relay_untrusted_validations),
            journal,
        }
    }

    pub fn from_application_root(root: &ApplicationRoot) -> Self {
        Self::new(
            root.validations().clone(),
            root.validators(),
            root.get_network_id_service().get_network_id(),
            root.relay_untrusted_validations(),
            root.logs().journal("NetworkOPs"),
        )
    }

    pub fn validations(
        &self,
    ) -> &SharedAppValidations<crate::state::time_keeper::SystemTimeKeeperClock> {
        &self.validations
    }

    pub fn validators(&self) -> Arc<ValidatorList<SystemValidatorListClock>> {
        Arc::clone(&self.validators)
    }

    pub fn snapshot(&self) -> AppNetworkOpsValidationRuntimeSnapshot {
        AppNetworkOpsValidationRuntimeSnapshot {
            pending_validations: self.pending_validation_count(),
            has_publisher: self.publisher(),
            network_id: self.network_id(),
            relay_untrusted_validations: self.relay_untrusted_validations(),
        }
    }

    pub fn pending_validation_count(&self) -> usize {
        self.pending_validations
            .lock()
            .expect("network ops validation pending set mutex must not be poisoned")
            .len()
    }

    pub fn insert_pending_validation(&self, ledger_hash: Uint256) -> bool {
        self.pending_validations
            .lock()
            .expect("network ops validation pending set mutex must not be poisoned")
            .insert(ledger_hash)
    }

    pub fn remove_pending_validation(&self, ledger_hash: &Uint256) -> bool {
        self.pending_validations
            .lock()
            .expect("network ops validation pending set mutex must not be poisoned")
            .remove(ledger_hash)
    }

    pub fn publisher(&self) -> bool {
        self.publisher
            .lock()
            .expect("network ops validation publisher mutex must not be poisoned")
            .is_some()
    }

    pub fn set_publisher(
        &self,
        publisher: Option<Arc<dyn NetworkOpsValidationPublisher>>,
    ) -> Option<Arc<dyn NetworkOpsValidationPublisher>> {
        let mut current = self
            .publisher
            .lock()
            .expect("network ops validation publisher mutex must not be poisoned");
        std::mem::replace(&mut *current, publisher)
    }

    pub fn network_id(&self) -> u32 {
        self.network_id.load(Ordering::Acquire)
    }

    pub fn set_network_id(&self, network_id: u32) -> u32 {
        self.network_id.swap(network_id, Ordering::AcqRel)
    }

    pub fn relay_untrusted_validations(&self) -> bool {
        self.relay_untrusted_validations.load(Ordering::Acquire)
    }

    pub fn set_relay_untrusted_validations(&self, relay_untrusted_validations: bool) -> bool {
        self.relay_untrusted_validations
            .swap(relay_untrusted_validations, Ordering::AcqRel)
    }

    pub fn publish_validation(&self, validation: &STValidation) -> bool {
        let publisher = self
            .publisher
            .lock()
            .expect("network ops validation publisher mutex must not be poisoned")
            .clone();
        let Some(publisher) = publisher else {
            return false;
        };

        publisher.publish_validation(validation_received_json(
            validation,
            self.network_id(),
            validation_master_key(self.validators.as_ref(), validation.get_signer_public()),
        ));
        true
    }

    pub fn receive_validation(
        &self,
        validation: &mut STValidation,
        source: &str,
    ) -> AppNetworkOpsValidationReceiveReport {
        self.receive_validation_with_accept(validation, source, None)
    }

    pub fn receive_validation_with_accept(
        &self,
        validation: &mut STValidation,
        source: &str,
        accept_sink: Option<&dyn RclValidationAcceptanceSink>,
    ) -> AppNetworkOpsValidationReceiveReport {
        let ledger_hash = validation.get_ledger_hash();
        self.journal
            .trace(&format!("recvValidation {ledger_hash} from {source}"));

        let bypass_accept = {
            let mut pending = self
                .pending_validations
                .lock()
                .expect("network ops validation pending set mutex must not be poisoned");
            if pending.contains(&ledger_hash) {
                true
            } else {
                pending.insert(ledger_hash);
                false
            }
        };

        let current = {
            let mut validations = self
                .validations
                .validations()
                .lock()
                .expect("shared app validations mutex must not be poisoned");
            handle_new_validation_with_store(
                self.validators.as_ref(),
                &mut validations,
                validation,
                bypass_accept,
                accept_sink,
                Some(self.validations.persistence().as_ref()),
                Some(self.journal.as_ref()),
            ) == consensus::ValidationStatus::Current
        };

        if !bypass_accept {
            self.pending_validations
                .lock()
                .expect("network ops validation pending set mutex must not be poisoned")
                .remove(&ledger_hash);
        }

        let published = self.publish_validation(validation);
        let relay = self.relay_untrusted_validations() || validation.is_trusted();

        AppNetworkOpsValidationReceiveReport {
            ledger_hash,
            source: source.to_owned(),
            bypass_accept,
            relay,
            published,
            trusted: validation.is_trusted(),
            full: validation.is_full(),
            current,
        }
    }
}

pub fn validation_master_key(
    validators: &ValidatorList<SystemValidatorListClock>,
    signing_key: &PublicKey,
) -> Option<PublicKey> {
    let master_key = validators.master_key(*signing_key);
    (master_key != *signing_key).then_some(master_key)
}

pub fn validation_received_json(
    validation: &STValidation,
    network_id: u32,
    master_key: Option<PublicKey>,
) -> JsonValue {
    let mut object = BTreeMap::from([
        (
            "type".to_owned(),
            JsonValue::String("validationReceived".to_owned()),
        ),
        (
            "validation_public_key".to_owned(),
            JsonValue::String(validation.get_signer_public().to_node_public_base58()),
        ),
        (
            "ledger_hash".to_owned(),
            JsonValue::String(validation.get_ledger_hash().to_string()),
        ),
        (
            "signature".to_owned(),
            JsonValue::String(str_hex(validation.get_signature())),
        ),
        ("full".to_owned(), JsonValue::Bool(validation.is_full())),
        (
            "flags".to_owned(),
            JsonValue::Unsigned(u64::from(validation.get_flags())),
        ),
        (
            "signing_time".to_owned(),
            JsonValue::Unsigned(u64::from(
                validation.get_field_u32(get_field_by_symbol("sfSigningTime")),
            )),
        ),
        (
            "data".to_owned(),
            JsonValue::String(str_hex(validation.get_serializer().data())),
        ),
        (
            "network_id".to_owned(),
            JsonValue::Unsigned(u64::from(network_id)),
        ),
    ]);

    if validation.is_field_present(get_field_by_symbol("sfServerVersion")) {
        object.insert(
            "server_version".to_owned(),
            JsonValue::String(
                validation
                    .get_field_u64(get_field_by_symbol("sfServerVersion"))
                    .to_string(),
            ),
        );
    }

    if validation.is_field_present(get_field_by_symbol("sfCookie")) {
        object.insert(
            "cookie".to_owned(),
            JsonValue::String(
                validation
                    .get_field_u64(get_field_by_symbol("sfCookie"))
                    .to_string(),
            ),
        );
    }

    if validation.is_field_present(get_field_by_symbol("sfValidatedHash")) {
        object.insert(
            "validated_hash".to_owned(),
            JsonValue::String(
                validation
                    .get_field_h256(get_field_by_symbol("sfValidatedHash"))
                    .to_string(),
            ),
        );
    }

    if let Some(master_key) = master_key {
        object.insert(
            "master_key".to_owned(),
            JsonValue::String(master_key.to_node_public_base58()),
        );
    }

    if validation.is_field_present(get_field_by_symbol("sfLedgerSequence")) {
        object.insert(
            "ledger_index".to_owned(),
            JsonValue::Unsigned(u64::from(
                validation.get_field_u32(get_field_by_symbol("sfLedgerSequence")),
            )),
        );
    }

    if validation.is_field_present(get_field_by_symbol("sfAmendments")) {
        object.insert(
            "amendments".to_owned(),
            JsonValue::Array(
                validation
                    .get_field_v256(get_field_by_symbol("sfAmendments"))
                    .value()
                    .iter()
                    .map(|amendment| JsonValue::String(amendment.to_string()))
                    .collect(),
            ),
        );
    }

    if validation.is_field_present(get_field_by_symbol("sfCloseTime")) {
        object.insert(
            "close_time".to_owned(),
            JsonValue::Unsigned(u64::from(
                validation.get_field_u32(get_field_by_symbol("sfCloseTime")),
            )),
        );
    }

    if validation.is_field_present(get_field_by_symbol("sfLoadFee")) {
        object.insert(
            "load_fee".to_owned(),
            JsonValue::Unsigned(u64::from(
                validation.get_field_u32(get_field_by_symbol("sfLoadFee")),
            )),
        );
    }

    if validation.is_field_present(get_field_by_symbol("sfBaseFee")) {
        object.insert(
            "base_fee".to_owned(),
            JsonValue::Unsigned(validation.get_field_u64(get_field_by_symbol("sfBaseFee"))),
        );
    }

    if validation.is_field_present(get_field_by_symbol("sfReserveBase")) {
        object.insert(
            "reserve_base".to_owned(),
            JsonValue::Unsigned(u64::from(
                validation.get_field_u32(get_field_by_symbol("sfReserveBase")),
            )),
        );
    }

    if validation.is_field_present(get_field_by_symbol("sfReserveIncrement")) {
        object.insert(
            "reserve_inc".to_owned(),
            JsonValue::Unsigned(u64::from(
                validation.get_field_u32(get_field_by_symbol("sfReserveIncrement")),
            )),
        );
    }

    if validation.is_field_present(get_field_by_symbol("sfBaseFeeDrops")) {
        let amount = validation.get_field_amount(get_field_by_symbol("sfBaseFeeDrops"));
        if amount.native() {
            object.insert("base_fee".to_owned(), amount.xrp().json_clipped());
        }
    }

    if validation.is_field_present(get_field_by_symbol("sfReserveBaseDrops")) {
        let amount = validation.get_field_amount(get_field_by_symbol("sfReserveBaseDrops"));
        if amount.native() {
            object.insert("reserve_base".to_owned(), amount.xrp().json_clipped());
        }
    }

    if validation.is_field_present(get_field_by_symbol("sfReserveIncrementDrops")) {
        let amount = validation.get_field_amount(get_field_by_symbol("sfReserveIncrementDrops"));
        if amount.native() {
            object.insert("reserve_inc".to_owned(), amount.xrp().json_clipped());
        }
    }

    JsonValue::Object(object)
}
