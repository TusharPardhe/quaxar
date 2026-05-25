//! `MPTIssue` helpers ported from `xrpl/protocol/MPTIssue.*`.

use std::collections::BTreeMap;

use crate::{AccountID, JsonValue, MPTID};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct MPTIssue {
    mpt_id: MPTID,
}

impl MPTIssue {
    pub fn new(mpt_id: MPTID) -> Self {
        Self { mpt_id }
    }

    pub fn mpt_id(&self) -> MPTID {
        self.mpt_id
    }

    pub fn issuer(&self) -> AccountID {
        AccountID::from_slice(&self.mpt_id.data()[4..]).expect("MPTID issuer width should match")
    }

    pub fn text(&self) -> String {
        self.mpt_id.to_string()
    }

    pub fn set_json(&self, json: &mut BTreeMap<String, JsonValue>) {
        json.insert(
            "mpt_issuance_id".to_string(),
            JsonValue::String(self.text()),
        );
    }
}

pub fn mpt_issue_to_string(issue: MPTIssue) -> String {
    issue.text()
}

pub fn mpt_issue_from_json(value: &JsonValue) -> Result<MPTIssue, String> {
    let JsonValue::Object(object) = value else {
        return Err("mptIssueFromJson can only be specified with an 'object' Json value".into());
    };

    if object.contains_key("currency") || object.contains_key("issuer") {
        return Err("mptIssueFromJson, MPTIssue should not have currency or issuer".into());
    }

    let Some(JsonValue::String(mpt_id)) = object.get("mpt_issuance_id") else {
        return Err("mptIssueFromJson MPTID must be a string Json value".into());
    };

    let mpt_id = MPTID::from_hex(mpt_id).map_err(|_| "mptIssueFromJson MPTID is invalid")?;
    Ok(MPTIssue::new(mpt_id))
}
