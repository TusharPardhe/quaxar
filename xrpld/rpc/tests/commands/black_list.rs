//! Tests for black list.

use protocol::JsonValue;
use rpc::{BlackListSource, do_black_list};
use std::cell::RefCell;
use std::collections::BTreeMap;

#[derive(Default)]
struct RecordingSource {
    calls: RefCell<Vec<Option<i64>>>,
}

impl BlackListSource for RecordingSource {
    fn black_list_json(&self) -> JsonValue {
        self.calls.borrow_mut().push(None);
        JsonValue::Object(BTreeMap::from([(
            "mode".to_owned(),
            JsonValue::String("default".to_owned()),
        )]))
    }

    fn black_list_json_with_threshold(&self, threshold: i64) -> JsonValue {
        self.calls.borrow_mut().push(Some(threshold));
        JsonValue::Object(BTreeMap::from([(
            "threshold".to_owned(),
            JsonValue::Signed(threshold),
        )]))
    }
}

fn object(entries: impl IntoIterator<Item = (&'static str, JsonValue)>) -> JsonValue {
    JsonValue::Object(
        entries
            .into_iter()
            .map(|(key, value)| (key.to_owned(), value))
            .collect::<BTreeMap<_, _>>(),
    )
}

#[test]
fn blacklist_uses_default_json_when_threshold_is_absent() {
    let source = RecordingSource::default();

    let result = do_black_list(&object([]), &source);

    assert_eq!(
        result,
        JsonValue::Object(BTreeMap::from([(
            "mode".to_owned(),
            JsonValue::String("default".to_owned()),
        )]))
    );
    assert_eq!(source.calls.into_inner(), vec![None]);
}

#[test]
fn blacklist_forwards_threshold_json_when_present() {
    let source = RecordingSource::default();

    let result = do_black_list(&object([("threshold", JsonValue::Signed(17))]), &source);

    assert_eq!(
        result,
        JsonValue::Object(BTreeMap::from([(
            "threshold".to_owned(),
            JsonValue::Signed(17),
        )]))
    );
    assert_eq!(source.calls.into_inner(), vec![Some(17)]);
}
