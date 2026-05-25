//! Narrow `MultiApiJson` port from `xrpl/protocol/MultiApiJson.h`.

use crate::JsonValue;

pub const DEFAULT_API_MINIMUM_SUPPORTED_VERSION: u32 = 1;
pub const DEFAULT_API_MAXIMUM_SUPPORTED_VERSION: u32 = 2;
pub const DEFAULT_API_MAXIMUM_VALID_VERSION: u32 = 3;
pub const DEFAULT_API_VERSION_IF_UNSPECIFIED: u32 = 1;
pub const DEFAULT_API_BETA_ENABLED: bool = true;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IsMemberResult {
    None = 0,
    Some,
    All,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MultiApiJson<
    const MIN_VER: u32 = DEFAULT_API_MINIMUM_SUPPORTED_VERSION,
    const MAX_VER: u32 = DEFAULT_API_MAXIMUM_VALID_VERSION,
> {
    values: Vec<JsonValue>,
}

impl<const MIN_VER: u32, const MAX_VER: u32> Default for MultiApiJson<MIN_VER, MAX_VER> {
    fn default() -> Self {
        Self::new(JsonValue::Null)
    }
}

impl<const MIN_VER: u32, const MAX_VER: u32> MultiApiJson<MIN_VER, MAX_VER> {
    pub const SIZE: usize = (MAX_VER - MIN_VER + 1) as usize;

    pub const fn valid(version: u32) -> bool {
        version >= MIN_VER && version <= MAX_VER
    }

    pub const fn index(version: u32) -> usize {
        if version < MIN_VER {
            0
        } else {
            (version - MIN_VER) as usize
        }
    }

    pub fn new(init: JsonValue) -> Self {
        let values = if matches!(init, JsonValue::Null) {
            vec![JsonValue::Null; Self::SIZE]
        } else {
            vec![init; Self::SIZE]
        };
        Self { values }
    }

    pub fn set(&mut self, key: &str, value: impl Into<JsonValue>) {
        let value = value.into();
        for json in &mut self.values {
            let JsonValue::Object(object) = json else {
                *json = JsonValue::Object(std::collections::BTreeMap::new());
                let JsonValue::Object(object) = json else {
                    unreachable!("json value must become an object");
                };
                object.insert(key.to_owned(), value.clone());
                continue;
            };
            object.insert(key.to_owned(), value.clone());
        }
    }

    pub fn is_member(&self, key: &str) -> IsMemberResult {
        let count = self
            .values
            .iter()
            .filter(|json| matches!(json, JsonValue::Object(object) if object.contains_key(key)))
            .count();

        if count == 0 {
            IsMemberResult::None
        } else if count < self.values.len() {
            IsMemberResult::Some
        } else {
            IsMemberResult::All
        }
    }

    pub fn visit<R>(&self, version: u32, f: impl FnOnce(&JsonValue) -> R) -> R {
        assert!(
            Self::valid(version),
            "MultiApiJson::visit requires a valid API version"
        );
        f(&self.values[Self::index(version)])
    }

    pub fn visit_mut<R>(&mut self, version: u32, f: impl FnOnce(&mut JsonValue) -> R) -> R {
        assert!(
            Self::valid(version),
            "MultiApiJson::visit_mut requires a valid API version"
        );
        f(&mut self.values[Self::index(version)])
    }

    pub fn values(&self) -> &[JsonValue] {
        &self.values
    }
}

impl From<bool> for JsonValue {
    fn from(value: bool) -> Self {
        JsonValue::Bool(value)
    }
}

impl From<u64> for JsonValue {
    fn from(value: u64) -> Self {
        JsonValue::Unsigned(value)
    }
}

impl From<String> for JsonValue {
    fn from(value: String) -> Self {
        JsonValue::String(value)
    }
}

impl From<&str> for JsonValue {
    fn from(value: &str) -> Self {
        JsonValue::String(value.to_owned())
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::{IsMemberResult, MultiApiJson};
    use crate::JsonValue;

    #[test]
    fn valid_and_index_match_current_cpp_version_range() {
        type TestJson = MultiApiJson<1, 3>;

        assert!(TestJson::valid(1));
        assert!(TestJson::valid(3));
        assert!(!TestJson::valid(0));
        assert!(!TestJson::valid(4));
        assert_eq!(TestJson::index(0), 0);
        assert_eq!(TestJson::index(1), 0);
        assert_eq!(TestJson::index(2), 1);
        assert_eq!(TestJson::index(3), 2);
    }

    #[test]
    fn set_and_is_member_touch_all_versions() {
        let mut json = MultiApiJson::<1, 3>::new(JsonValue::Object(BTreeMap::new()));

        assert_eq!(json.is_member("status"), IsMemberResult::None);
        json.set("status", "ok");
        assert_eq!(json.is_member("status"), IsMemberResult::All);
        for value in json.values() {
            let JsonValue::Object(object) = value else {
                panic!("all values should stay objects");
            };
            assert_eq!(
                object.get("status"),
                Some(&JsonValue::String("ok".to_owned()))
            );
        }
    }

    #[test]
    fn visit_selects_versioned_slot() {
        let mut json = MultiApiJson::<1, 3>::new(JsonValue::Object(BTreeMap::new()));
        json.visit_mut(2, |value| {
            let JsonValue::Object(object) = value else {
                panic!("slot must be object");
            };
            object.insert("ledger_index".to_owned(), JsonValue::Unsigned(9));
        });

        assert_eq!(
            json.visit(2, |value| value.clone()),
            JsonValue::Object(BTreeMap::from([(
                "ledger_index".to_owned(),
                JsonValue::Unsigned(9)
            )]))
        );
        assert_eq!(
            json.visit(1, |value| value.clone()),
            JsonValue::Object(BTreeMap::new())
        );
    }
}
