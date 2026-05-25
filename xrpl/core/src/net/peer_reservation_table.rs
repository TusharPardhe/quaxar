use basics::blob::Blob;
use protocol::{PublicKey, parse_base58_node_public};
use serde_json::{Map, Value};
use std::any::TypeId;
use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::hash::{Hash, Hasher};
use std::sync::{Arc, Mutex};
use xrpld_core::DatabaseCon;

use crate::service_registry::ServiceRegistry;

pub type PeerReservationNodeId = Blob;

pub trait PeerReservationStore<NodeId>: Send + Sync
where
    NodeId: Clone + Ord + 'static,
{
    fn load(&self) -> Result<Vec<PeerReservation<NodeId>>, String>;
    fn insert_or_assign(&self, reservation: &PeerReservation<NodeId>) -> Result<(), String>;
    fn erase(&self, node_id: &NodeId) -> Result<(), String>;
}

pub trait PeerReservationJournal: Send + Sync {
    fn warn(&self, _message: &str) {}
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct NullPeerReservationJournal;

impl PeerReservationJournal for NullPeerReservationJournal {}

impl<T> PeerReservationJournal for Arc<T>
where
    T: PeerReservationJournal + ?Sized,
{
    fn warn(&self, message: &str) {
        (**self).warn(message);
    }
}

#[derive(Clone)]
pub struct SqlitePeerReservationStore {
    connection: Arc<DatabaseCon>,
    journal: Arc<dyn PeerReservationJournal>,
}

impl SqlitePeerReservationStore {
    pub fn new(connection: Arc<DatabaseCon>) -> Self {
        Self::with_journal(connection, Arc::new(NullPeerReservationJournal))
    }

    pub fn with_journal(
        connection: Arc<DatabaseCon>,
        journal: Arc<dyn PeerReservationJournal>,
    ) -> Self {
        Self {
            connection,
            journal,
        }
    }
}

#[derive(Debug, Clone)]
pub struct PeerReservation<NodeId = PeerReservationNodeId> {
    pub node_id: NodeId,
    pub description: String,
}

impl<NodeId> PeerReservation<NodeId> {
    pub fn new(node_id: NodeId, description: impl Into<String>) -> Self {
        Self {
            node_id,
            description: description.into(),
        }
    }
}

impl PeerReservation<PublicKey> {
    pub fn to_json(&self) -> Value {
        let mut result = Map::new();
        result.insert(
            "node".to_string(),
            Value::String(self.node_id.to_node_public_base58()),
        );
        if !self.description.is_empty() {
            result.insert(
                "description".to_string(),
                Value::String(self.description.clone()),
            );
        }
        Value::Object(result)
    }
}

impl<NodeId> PartialEq for PeerReservation<NodeId>
where
    NodeId: PartialEq,
{
    fn eq(&self, other: &Self) -> bool {
        self.node_id == other.node_id
    }
}

impl<NodeId> Eq for PeerReservation<NodeId> where NodeId: Eq {}

impl<NodeId> PartialOrd for PeerReservation<NodeId>
where
    NodeId: Ord,
{
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl<NodeId> Ord for PeerReservation<NodeId>
where
    NodeId: Ord,
{
    fn cmp(&self, other: &Self) -> Ordering {
        self.node_id.cmp(&other.node_id)
    }
}

impl<NodeId> Hash for PeerReservation<NodeId>
where
    NodeId: Hash,
{
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.node_id.hash(state);
    }
}

struct PeerReservationTableState<NodeId>
where
    NodeId: Clone + Ord + 'static,
{
    table: BTreeMap<NodeId, PeerReservation<NodeId>>,
    store: Option<Arc<dyn PeerReservationStore<NodeId>>>,
    journal: Arc<dyn PeerReservationJournal>,
}

pub struct PeerReservationTable<NodeId = PeerReservationNodeId>
where
    NodeId: Clone + Ord + 'static,
{
    state: Mutex<PeerReservationTableState<NodeId>>,
}

impl<NodeId> Default for PeerReservationTable<NodeId>
where
    NodeId: Clone + Ord + 'static,
{
    fn default() -> Self {
        Self::new()
    }
}

impl<NodeId> PeerReservationTable<NodeId>
where
    NodeId: Clone + Ord + 'static,
{
    pub fn new() -> Self {
        Self::with_journal(Arc::new(NullPeerReservationJournal))
    }

    pub fn with_journal(journal: Arc<dyn PeerReservationJournal>) -> Self {
        Self {
            state: Mutex::new(PeerReservationTableState {
                table: BTreeMap::new(),
                store: None,
                journal,
            }),
        }
    }

    pub fn load(&self, store: Arc<dyn PeerReservationStore<NodeId>>) -> Result<bool, String> {
        let mut state = self
            .state
            .lock()
            .expect("peer reservation table mutex poisoned");
        let previous_store = state.store.clone();

        state.store = Some(Arc::clone(&store));
        let loaded = match store.load() {
            Ok(loaded) => loaded,
            Err(error) => {
                state.store = previous_store;
                return Err(error);
            }
        };
        for reservation in loaded {
            state
                .table
                .entry(reservation.node_id.clone())
                .or_insert(reservation);
        }

        Ok(true)
    }

    pub fn list(&self) -> Vec<PeerReservation<NodeId>> {
        self.state
            .lock()
            .expect("peer reservation table mutex poisoned")
            .table
            .values()
            .cloned()
            .collect()
    }

    pub fn contains(&self, node_id: &NodeId) -> bool {
        self.state
            .lock()
            .expect("peer reservation table mutex poisoned")
            .table
            .contains_key(node_id)
    }

    pub fn insert_or_assign(
        &self,
        reservation: PeerReservation<NodeId>,
    ) -> Option<PeerReservation<NodeId>> {
        self.try_insert_or_assign(reservation)
            .expect("peer reservation persistence must succeed")
    }

    pub fn erase(&self, node_id: &NodeId) -> Option<PeerReservation<NodeId>> {
        self.try_erase(node_id)
            .expect("peer reservation persistence must succeed")
    }

    pub fn try_insert_or_assign(
        &self,
        reservation: PeerReservation<NodeId>,
    ) -> Result<Option<PeerReservation<NodeId>>, String> {
        let mut state = self
            .state
            .lock()
            .expect("peer reservation table mutex poisoned");
        self.require_store_before_mutation(&state)?;
        let previous = state
            .table
            .insert(reservation.node_id.clone(), reservation.clone());

        if let Some(store) = state.store.as_ref() {
            store.insert_or_assign(&reservation)?;
        }

        Ok(previous)
    }

    pub fn try_erase(&self, node_id: &NodeId) -> Result<Option<PeerReservation<NodeId>>, String> {
        let mut state = self
            .state
            .lock()
            .expect("peer reservation table mutex poisoned");
        self.require_store_before_mutation(&state)?;
        let previous = state.table.remove(node_id);

        if previous.is_some()
            && let Some(store) = state.store.as_ref()
        {
            store.erase(node_id)?;
        }

        Ok(previous)
    }

    fn require_store_before_mutation(
        &self,
        state: &PeerReservationTableState<NodeId>,
    ) -> Result<(), String> {
        if TypeId::of::<NodeId>() == TypeId::of::<PublicKey>() && state.store.is_none() {
            return Err(
                "peer reservation table must be loaded before wallet-backed mutation".to_owned(),
            );
        }
        Ok(())
    }
}

impl PeerReservationTable<PublicKey> {
    pub fn new_with_journal(journal: Arc<dyn PeerReservationJournal>) -> Self {
        Self::with_journal(journal)
    }

    pub fn load_from_database(&self, connection: Arc<DatabaseCon>) -> Result<bool, String> {
        let journal = {
            let state = self
                .state
                .lock()
                .expect("peer reservation table mutex poisoned");
            Arc::clone(&state.journal)
        };
        self.load(Arc::new(SqlitePeerReservationStore::with_journal(
            connection, journal,
        )))
    }

    pub fn load_from_database_with_journal(
        &self,
        connection: Arc<DatabaseCon>,
        journal: Arc<dyn PeerReservationJournal>,
    ) -> Result<bool, String> {
        let previous_journal = {
            let mut state = self
                .state
                .lock()
                .expect("peer reservation table mutex poisoned");
            let previous_journal = Arc::clone(&state.journal);
            state.journal = Arc::clone(&journal);
            previous_journal
        };

        let result = self.load(Arc::new(SqlitePeerReservationStore::with_journal(
            connection, journal,
        )));

        if result.is_err() {
            let mut state = self
                .state
                .lock()
                .expect("peer reservation table mutex poisoned");
            state.journal = previous_journal;
        }

        result
    }

    #[cfg(test)]
    fn load_with_store_and_journal(
        &self,
        store: Arc<dyn PeerReservationStore<PublicKey>>,
        journal: Arc<dyn PeerReservationJournal>,
    ) -> Result<bool, String> {
        let previous_journal = {
            let mut state = self
                .state
                .lock()
                .expect("peer reservation table mutex poisoned");
            let previous_journal = Arc::clone(&state.journal);
            state.journal = Arc::clone(&journal);
            previous_journal
        };

        let result = self.load(store);
        if result.is_err() {
            let mut state = self
                .state
                .lock()
                .expect("peer reservation table mutex poisoned");
            state.journal = previous_journal;
        }

        result
    }
}

impl PeerReservationStore<PublicKey> for SqlitePeerReservationStore {
    fn load(&self) -> Result<Vec<PeerReservation<PublicKey>>, String> {
        let connection = self.connection.checkout_db();
        let mut statement = connection
            .prepare("SELECT PublicKey, Description FROM PeerReservations;")
            .map_err(|error| error.to_string())?;
        let rows = statement
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(|error| error.to_string())?;

        let mut reservations = Vec::new();
        for row in rows {
            let (node_public, description) = row.map_err(|error| error.to_string())?;
            let Some(bytes) = parse_base58_node_public(&node_public) else {
                self.journal
                    .warn(&format!("load: not a public key: {node_public}"));
                continue;
            };
            reservations.push(PeerReservation::new(
                PublicKey::from_bytes(bytes),
                description,
            ));
        }

        Ok(reservations)
    }

    fn insert_or_assign(&self, reservation: &PeerReservation<PublicKey>) -> Result<(), String> {
        let connection = self.connection.checkout_db();
        connection
            .execute(
                "INSERT INTO PeerReservations (PublicKey, Description) \
                 VALUES (?1, ?2) \
                 ON CONFLICT (PublicKey) DO UPDATE SET Description=excluded.Description",
                [
                    reservation.node_id.to_node_public_base58(),
                    reservation.description.clone(),
                ],
            )
            .map_err(|error| error.to_string())?;
        Ok(())
    }

    fn erase(&self, node_id: &PublicKey) -> Result<(), String> {
        let connection = self.connection.checkout_db();
        connection
            .execute(
                "DELETE FROM PeerReservations WHERE PublicKey = ?1",
                [node_id.to_node_public_base58()],
            )
            .map_err(|error| error.to_string())?;
        Ok(())
    }
}

pub fn load_peer_reservations_from_registry<R>(registry: &R) -> Result<bool, String>
where
    R: ServiceRegistry<
            PeerReservationTable = PeerReservationTable<PublicKey>,
            WalletDb = Arc<DatabaseCon>,
        >,
{
    registry
        .get_peer_reservations()
        .load_from_database(Arc::clone(registry.get_wallet_db()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct RecordingJournal {
        warnings: Mutex<Vec<String>>,
    }

    impl PeerReservationJournal for RecordingJournal {
        fn warn(&self, message: &str) {
            self.warnings
                .lock()
                .expect("recording journal poisoned")
                .push(message.to_owned());
        }
    }

    struct FailingStore;

    impl PeerReservationStore<PublicKey> for FailingStore {
        fn load(&self) -> Result<Vec<PeerReservation<PublicKey>>, String> {
            Err("load failed".to_owned())
        }

        fn insert_or_assign(
            &self,
            _reservation: &PeerReservation<PublicKey>,
        ) -> Result<(), String> {
            Ok(())
        }

        fn erase(&self, _node_id: &PublicKey) -> Result<(), String> {
            Ok(())
        }
    }

    #[test]
    fn load_with_store_and_journal_restores_previous_journal_on_failure() {
        let initial_journal: Arc<dyn PeerReservationJournal> =
            Arc::new(RecordingJournal::default());
        let replacement_journal: Arc<dyn PeerReservationJournal> =
            Arc::new(RecordingJournal::default());
        let table = PeerReservationTable::new_with_journal(initial_journal.clone());

        let result =
            table.load_with_store_and_journal(Arc::new(FailingStore), replacement_journal.clone());

        assert_eq!(result.expect_err("load should fail"), "load failed");

        let state = table
            .state
            .lock()
            .expect("peer reservation table mutex poisoned");
        assert!(std::ptr::eq(
            Arc::as_ptr(&state.journal),
            Arc::as_ptr(&initial_journal),
        ));
        assert!(!std::ptr::eq(
            Arc::as_ptr(&state.journal),
            Arc::as_ptr(&replacement_journal),
        ));
    }
}
