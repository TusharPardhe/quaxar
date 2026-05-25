//! Stateful path request helper aligned with the reference implementation.

use std::time::Instant;

use protocol::JsonValue;

use super::pathfinder::{
    PathFindTuning, PathFinderSource, make_path_find_status, parse_path_finder_request,
};
use xrpld_core::Status;

#[derive(Debug, Clone)]
pub struct PathRequest {
    id: u64,
    params: JsonValue,
    status: JsonValue,
    last_ledger_index: u32,
    search_level: u32,
    tuning: PathFindTuning,
    last_success: bool,
    legacy: bool,
    created_at: Instant,
}

impl PathRequest {
    pub fn new(id: u64) -> Self {
        Self {
            id,
            params: JsonValue::Null,
            status: JsonValue::Null,
            last_ledger_index: 0,
            search_level: 0,
            tuning: PathFindTuning::default(),
            last_success: false,
            legacy: false,
            created_at: Instant::now(),
        }
    }

    pub fn id(&self) -> u64 {
        self.id
    }

    pub fn age_ms(&self) -> u128 {
        self.created_at.elapsed().as_millis()
    }

    pub fn create<S: PathFinderSource + ?Sized>(
        &mut self,
        source: &S,
        params: &JsonValue,
        ledger_index: u32,
        full_reply: bool,
        legacy: bool,
    ) -> Result<JsonValue, Status> {
        let parsed = parse_path_finder_request(params)?;
        self.tuning = source.path_find_tuning();
        self.search_level = if legacy {
            self.tuning.old.max(1)
        } else if full_reply {
            self.tuning.search.max(1)
        } else {
            self.tuning.fast.max(1)
        };
        let result = source.find_paths(&parsed, params, self.search_level, legacy)?;
        self.params = params.clone();
        self.status = make_path_find_status(self.id, &parsed, result.clone(), full_reply, legacy);
        self.last_ledger_index = ledger_index;
        self.last_success = !matches!(result, JsonValue::Array(ref values) if values.is_empty());
        self.legacy = legacy;
        Ok(self.status.clone())
    }

    pub fn update<S: PathFinderSource + ?Sized>(
        &mut self,
        source: &S,
        ledger_index: u32,
        full_reply: bool,
        legacy: bool,
    ) -> Result<JsonValue, Status> {
        if self.last_success {
            let floor = if self.legacy {
                self.tuning.old.max(1)
            } else {
                self.tuning.search.max(1)
            };
            if self.search_level > floor {
                self.search_level -= 1;
            }
        } else if self.search_level < self.tuning.max.max(1) {
            self.search_level += 1;
        }
        let parsed = parse_path_finder_request(&self.params)?;
        let result = source.find_paths(&parsed, &self.params, self.search_level, legacy)?;
        self.status = make_path_find_status(self.id, &parsed, result.clone(), full_reply, legacy);
        self.last_ledger_index = ledger_index;
        self.last_success = !matches!(result, JsonValue::Array(ref values) if values.is_empty());
        self.legacy = legacy;
        Ok(self.status.clone())
    }

    pub fn close(&mut self) -> JsonValue {
        match &mut self.status {
            JsonValue::Object(status) => {
                status.insert("closed".to_owned(), JsonValue::Bool(true));
            }
            _ => {
                self.status = JsonValue::Object(std::collections::BTreeMap::from([(
                    "closed".to_owned(),
                    JsonValue::Bool(true),
                )]));
            }
        }
        self.status.clone()
    }

    pub fn status(&self) -> JsonValue {
        self.status.clone()
    }

    pub fn last_ledger_index(&self) -> u32 {
        self.last_ledger_index
    }
}
