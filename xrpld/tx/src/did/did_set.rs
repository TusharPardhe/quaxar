//! Deterministic the reference implementation shells.
//!
//! This ports the current narrow compatibility-safe behavior for:
//!
//! - the `preflight(...)` empty-field and length guards,
//! - the update-path field mutation planning plus final empty-object guard,
//! - and the create-path owner / reserve / owner-dir ordering above the
//!   existing `addSLE(...)` seam.

use protocol::{AccountID, NotTec, Ter};

pub const MAX_DID_URI_LENGTH: usize = 256;
pub const MAX_DID_DOCUMENT_LENGTH: usize = 256;
pub const MAX_DID_DATA_LENGTH: usize = 256;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct DidSetPreflightFacts {
    pub uri_len: Option<usize>,
    pub did_document_len: Option<usize>,
    pub data_len: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DidSetApplyFacts<AccountId> {
    pub account: AccountId,
    pub uri: Option<Vec<u8>>,
    pub did_document: Option<Vec<u8>>,
    pub data: Option<Vec<u8>>,
    pub fix_empty_did_enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DidSetLoadedEntry {
    pub uri: Option<Vec<u8>>,
    pub did_document: Option<Vec<u8>>,
    pub data: Option<Vec<u8>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DidSetFieldUpdate {
    NoChange,
    Remove,
    Set(Vec<u8>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DidSetUpdateMutation {
    pub uri: DidSetFieldUpdate,
    pub did_document: DidSetFieldUpdate,
    pub data: DidSetFieldUpdate,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DidSetCreateMutation {
    pub account: AccountID,
    pub uri: Option<Vec<u8>>,
    pub did_document: Option<Vec<u8>>,
    pub data: Option<Vec<u8>>,
    pub owner_node: u64,
}

pub trait DidSetApplySink {
    fn existing_did(&mut self) -> Option<DidSetLoadedEntry>;
    fn owner_account_exists(&mut self) -> bool;
    fn reserve_sufficient(&mut self) -> bool;
    fn insert_owner_dir(&mut self) -> Option<u64>;
    fn update_did(&mut self, mutation: DidSetUpdateMutation);
    fn create_did(&mut self, mutation: DidSetCreateMutation);
    fn adjust_owner_count(&mut self, delta: i32);
}

fn is_too_long(length: Option<usize>, limit: usize) -> bool {
    length.is_some_and(|length| length > limit)
}

fn classify_update(field: Option<Vec<u8>>) -> DidSetFieldUpdate {
    match field {
        None => DidSetFieldUpdate::NoChange,
        Some(bytes) if bytes.is_empty() => DidSetFieldUpdate::Remove,
        Some(bytes) => DidSetFieldUpdate::Set(bytes),
    }
}

fn final_field_present(current: bool, update: &DidSetFieldUpdate) -> bool {
    match update {
        DidSetFieldUpdate::NoChange => current,
        DidSetFieldUpdate::Remove => false,
        DidSetFieldUpdate::Set(_) => true,
    }
}

fn create_field(field: Option<Vec<u8>>) -> Option<Vec<u8>> {
    match field {
        Some(bytes) if !bytes.is_empty() => Some(bytes),
        _ => None,
    }
}

pub fn run_did_set_preflight(facts: DidSetPreflightFacts) -> NotTec {
    if facts.uri_len.is_none() && facts.did_document_len.is_none() && facts.data_len.is_none() {
        return Ter::TEM_EMPTY_DID;
    }

    if facts.uri_len == Some(0) && facts.did_document_len == Some(0) && facts.data_len == Some(0) {
        return Ter::TEM_EMPTY_DID;
    }

    if is_too_long(facts.uri_len, MAX_DID_URI_LENGTH)
        || is_too_long(facts.did_document_len, MAX_DID_DOCUMENT_LENGTH)
        || is_too_long(facts.data_len, MAX_DID_DATA_LENGTH)
    {
        return Ter::TEM_MALFORMED;
    }

    Ter::TES_SUCCESS
}

pub fn run_did_set_do_apply<S>(facts: DidSetApplyFacts<AccountID>, sink: &mut S) -> Ter
where
    S: DidSetApplySink,
{
    if let Some(existing) = sink.existing_did() {
        let mutation = DidSetUpdateMutation {
            uri: classify_update(facts.uri),
            did_document: classify_update(facts.did_document),
            data: classify_update(facts.data),
        };

        let uri_present = final_field_present(existing.uri.is_some(), &mutation.uri);
        let did_document_present =
            final_field_present(existing.did_document.is_some(), &mutation.did_document);
        let data_present = final_field_present(existing.data.is_some(), &mutation.data);

        if !uri_present && !did_document_present && !data_present {
            return Ter::TEC_EMPTY_DID;
        }

        sink.update_did(mutation);
        return Ter::TES_SUCCESS;
    }

    if !sink.owner_account_exists() {
        return Ter::TEF_INTERNAL;
    }

    if !sink.reserve_sufficient() {
        return Ter::TEC_INSUFFICIENT_RESERVE;
    }

    let uri = create_field(facts.uri);
    let did_document = create_field(facts.did_document);
    let data = create_field(facts.data);

    if facts.fix_empty_did_enabled && uri.is_none() && did_document.is_none() && data.is_none() {
        return Ter::TEC_EMPTY_DID;
    }

    let Some(owner_node) = sink.insert_owner_dir() else {
        return Ter::TEC_DIR_FULL;
    };

    sink.create_did(DidSetCreateMutation {
        account: facts.account,
        uri,
        did_document,
        data,
        owner_node,
    });
    sink.adjust_owner_count(1);
    Ter::TES_SUCCESS
}
