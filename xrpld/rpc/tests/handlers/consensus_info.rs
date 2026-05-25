//! Tests for the consensus info RPC handler.

use std::{cell::RefCell, collections::BTreeMap};

use protocol::JsonValue;
use rpc::{ConsensusInfoSource, JsonContext, JsonContextHeaders, RpcRole, do_consensus_info};

#[derive(Debug, Default)]
struct FakeConsensusInfoSource {
    calls: RefCell<u32>,
}

impl ConsensusInfoSource for FakeConsensusInfoSource {
    fn get_consensus_info(&self) -> JsonValue {
        *self.calls.borrow_mut() += 1;
        JsonValue::Object(BTreeMap::from([
            ("mode".to_owned(), JsonValue::String("consensus".to_owned())),
            ("seq".to_owned(), JsonValue::Unsigned(7)),
        ]))
    }
}

fn context<'a, Env>(params: &'a JsonValue, env: &'a Env, role: RpcRole) -> JsonContext<'a, Env> {
    JsonContext {
        params,
        env,
        role,
        api_version: 2,
        headers: JsonContextHeaders::default(),
        unlimited: false,
    }
}

#[test]
fn consensus_info_wraps_source_json() {
    let params = JsonValue::Object(Default::default());
    let source = FakeConsensusInfoSource::default();
    let result = do_consensus_info(&context(&params, &source, RpcRole::Admin));
    let JsonValue::Object(result) = result else {
        panic!("result must be an object");
    };

    let JsonValue::Object(info) = result.get("info").expect("info") else {
        panic!("info must be an object");
    };
    assert_eq!(
        info.get("mode"),
        Some(&JsonValue::String("consensus".to_owned()))
    );
    assert_eq!(info.get("seq"), Some(&JsonValue::Unsigned(7)));
    assert_eq!(*source.calls.borrow(), 1);
}
