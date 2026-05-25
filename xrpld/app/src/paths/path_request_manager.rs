//! Path request owner aligned with the reference implementation.

use std::collections::BTreeMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

use protocol::JsonValue;

use super::path_request::PathRequest;
use super::pathfinder::PathFinderSource;
use xrpld_core::{RpcErrorCode, Status};

pub trait PathFindSession {
    fn session_id(&self) -> u64;
    fn api_version(&self) -> u32;
    fn set_api_version(&self, api_version: u32);
    fn current_path_request_id(&self) -> Option<u64>;
    fn set_current_path_request_id(&self, request_id: Option<u64>);
}

#[derive(Debug, Default)]
pub struct PathRequestManager {
    next_request_id: AtomicU64,
    requests: Mutex<BTreeMap<u64, PathRequest>>,
}

impl PathRequestManager {
    pub fn new() -> Self {
        Self::default()
    }

    fn next_request_id(&self) -> u64 {
        self.next_request_id.fetch_add(1, Ordering::Relaxed) + 1
    }

    pub fn make_path_request<S: PathFinderSource + ?Sized, Session: PathFindSession + ?Sized>(
        &self,
        session: &Session,
        source: &S,
        ledger_index: u32,
        params: &JsonValue,
    ) -> Result<JsonValue, Status> {
        let request_id = self.next_request_id();
        let mut request = PathRequest::new(request_id);
        let status = request.create(source, params, ledger_index, false, false)?;
        self.requests
            .lock()
            .expect("path request manager mutex poisoned")
            .insert(request_id, request);
        session.set_current_path_request_id(Some(request_id));
        Ok(status)
    }

    pub fn make_legacy_path_request<
        S: PathFinderSource + ?Sized,
        Session: PathFindSession + ?Sized,
    >(
        &self,
        session: &Session,
        source: &S,
        ledger_index: u32,
        params: &JsonValue,
    ) -> Result<JsonValue, Status> {
        let request_id = self.next_request_id();
        let mut request = PathRequest::new(request_id);
        let status = request.create(source, params, ledger_index, true, true)?;
        self.requests
            .lock()
            .expect("path request manager mutex poisoned")
            .insert(request_id, request);
        session.set_current_path_request_id(Some(request_id));
        Ok(status)
    }

    pub fn direct_legacy_path_request<S: PathFinderSource + ?Sized>(
        &self,
        source: &S,
        ledger_index: u32,
        params: &JsonValue,
    ) -> Result<JsonValue, Status> {
        let mut request = PathRequest::new(self.next_request_id());
        request.create(source, params, ledger_index, true, true)
    }

    pub fn close_request<Session: PathFindSession + ?Sized>(
        &self,
        session: &Session,
    ) -> Result<JsonValue, Status> {
        let Some(request_id) = session.current_path_request_id() else {
            return Err(Status::new(RpcErrorCode::NoPathRequest));
        };
        let response = self
            .requests
            .lock()
            .expect("path request manager mutex poisoned")
            .remove(&request_id)
            .map(|mut request| request.close())
            .ok_or_else(|| Status::new(RpcErrorCode::NoPathRequest))?;
        session.set_current_path_request_id(None);
        Ok(response)
    }

    pub fn status_request<Session: PathFindSession + ?Sized>(
        &self,
        session: &Session,
    ) -> Result<JsonValue, Status> {
        let Some(request_id) = session.current_path_request_id() else {
            return Err(Status::new(RpcErrorCode::NoPathRequest));
        };
        self.requests
            .lock()
            .expect("path request manager mutex poisoned")
            .get(&request_id)
            .map(PathRequest::status)
            .ok_or_else(|| Status::new(RpcErrorCode::NoPathRequest))
    }

    pub fn update_all<S: PathFinderSource + ?Sized>(
        &self,
        source: &S,
        ledger_index: u32,
    ) -> Result<Vec<(u64, JsonValue)>, Status> {
        let mut updates = Vec::new();
        let mut requests = self
            .requests
            .lock()
            .expect("path request manager mutex poisoned");
        for (request_id, request) in requests.iter_mut() {
            if request.last_ledger_index() < ledger_index {
                updates.push((
                    *request_id,
                    request.update(source, ledger_index, false, false)?,
                ));
            }
        }
        Ok(updates)
    }

    pub fn request_count(&self) -> usize {
        self.requests
            .lock()
            .expect("path request manager mutex poisoned")
            .len()
    }
}
